//! `lattice log repair`: reports damage in a write-ahead log and, with `--apply`, removes a torn
//! tail that recovery would discard anyway (issue 475).
//!
//! The rules, all enforced before any byte of the log changes:
//! - A dry run is the default. It prints what `--apply` would change and why, and it exits non-zero
//!   when the log holds damage this command cannot fix. It creates no file and changes no byte.
//! - The only change ever made is truncating the newest segment at the start of a torn tail: the
//!   bytes the scrubber reports as `TornTail`, which is the test recovery itself uses to discard
//!   them. Interior damage, a checksum-valid frame with the wrong LSN (formats/wal-v1.md, issue
//!   441), a missing segment, an unreadable header and every semantic finding are reported and
//!   refused, and nothing is written.
//! - Before truncating, the whole segment is copied to `<segment>.repair-backup` in the same
//!   directory. The copy is fsynced with its directory and read back, and it must match the original
//!   byte for byte. A backup that already exists, or is a link, is never overwritten.
//! - The log's `LOCK` file is held for the whole scan and repair, so a running writer makes the
//!   command refuse. A dry run takes the lock only when a `LOCK` file already exists.
//! - Symbolic links are never followed: every directory on the `--wal` path, the lock file and every
//!   segment must be a plain file or directory, or the command refuses.
//! - After truncation the log is scanned again, then opened with the same recovery a server uses.
//!
//! Record payloads are never printed; the output names segments, offsets, lengths and LSNs only.

use lattice_core::Config;
use lattice_core::log::Log;
use lattice_core::record::SEGMENT_HEADER_LEN;
use lattice_core::scrub::{Issue, ScrubReport, scrub};
use lattice_seams::{Io, StdIo};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::process::ExitCode;

const USAGE: &str = "\
usage: lattice log repair [--dir DATA_DIR] [--wal NAME] [--apply]

Reports damage in the log NAME inside DATA_DIR (defaults `.` and `wal`). Nothing changes unless
--apply is given. With --apply, a torn tail that recovery would discard is removed after the newest
segment is copied to NAME/<segment>.repair-backup and the copy is verified. Interior damage, a
missing segment and every other finding are refused and left untouched; formats/wal-v1.md describes
the manual procedure for them.
Exit status: 0 healthy, repairable (dry run) or repaired; 1 damage this command cannot fix, a refused
step or a failed write; 2 bad arguments.
";

/// The suffix of the copy kept beside a segment before it is truncated.
const BACKUP_SUFFIX: &str = ".repair-backup";
/// The lock file every writer takes (see `Log::open`).
const LOCK_NAME: &str = "LOCK";

/// What the user asked for, after argument checks.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    dir: String,
    wal: String,
    apply: bool,
}

enum Parsed {
    Help,
    Run(Options),
}

/// The one torn tail the scrubber found: the bytes recovery would discard.
#[derive(Debug, PartialEq, Eq)]
struct Tail {
    segment: String,
    offset: usize,
    bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Healthy,
    TornTail(Tail),
    Damaged,
}

/// What a pass over the log concluded, before the final result line is written.
#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Healthy,
    Damaged,
    Repairable,
    Truncated,
}

#[derive(Debug)]
struct Outcome {
    text: String,
    ok: bool,
}

/// `lattice log ...`: exit 0 for help and for a successful repair, 1 for damage this command cannot
/// fix or a refused step, 2 for bad arguments.
pub(crate) fn command(args: &[String]) -> ExitCode {
    match parse(args) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(opts)) => match repair(&opts) {
            Ok(outcome) => {
                print!("{}", outcome.text);
                if outcome.ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("lattice log repair: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("lattice log repair: {e}");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Parses `repair [--dir DIR] [--wal NAME] [--apply]`. Every flag is strict: an unknown or repeated
/// flag, or a flag without a value, is an error.
fn parse(args: &[String]) -> Result<Parsed, String> {
    let rest = match args.split_first() {
        Some((first, rest)) if first == "repair" => rest,
        Some((first, _)) => return Err(format!("unknown command `{first}`; expected `repair`")),
        None => return Err("expected `repair`".into()),
    };
    let mut dir: Option<String> = None;
    let mut wal: Option<String> = None;
    let mut apply = false;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--apply" => {
                if apply {
                    return Err("--apply given more than once".into());
                }
                apply = true;
            }
            "--dir" | "--wal" => {
                let value = match iter.next() {
                    Some(v) if !v.starts_with("--") => v.clone(),
                    _ => return Err(format!("{arg} needs a value")),
                };
                let slot = if arg == "--dir" { &mut dir } else { &mut wal };
                if slot.replace(value).is_some() {
                    return Err(format!("{arg} given more than once"));
                }
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Parsed::Run(Options {
        dir: dir.unwrap_or_else(|| ".".into()),
        wal: wal.unwrap_or_else(|| "wal".into()),
        apply,
    }))
}

/// The whole command: validate, lock when needed, scan, repair, then verify by opening the log.
fn repair(opts: &Options) -> Result<Outcome, String> {
    check_names(opts)?;
    let root = Path::new(&opts.dir);
    check_directories(root, &opts.wal)?;
    let mut io = StdIo::new(root);
    let names = io
        .list(&opts.wal)
        .map_err(|e| format!("cannot read the log directory `{}`: {e}", opts.wal))?;
    check_plain_files(root, &opts.wal, &names)?;
    let lock = format!("{}/{LOCK_NAME}", opts.wal);
    let lock_exists = names.iter().any(|n| n == LOCK_NAME);
    // A dry run only reads: it takes the lock when one exists, and creates nothing otherwise.
    let (mut text, stage) = if opts.apply || lock_exists {
        with_lock(&mut io, &lock, |io| scan_and_repair(io, opts, root))?
    } else {
        scan_and_repair(&mut io, opts, root)?
    };
    let ok = match stage {
        Stage::Healthy => {
            text.push_str("result: healthy, nothing to repair\n");
            true
        }
        Stage::Damaged => {
            text.push_str(
                "result: damaged; `lattice log repair` cannot fix it and changed nothing (see formats/wal-v1.md)\n",
            );
            false
        }
        Stage::Repairable => {
            text.push_str("result: repairable with --apply; nothing was changed\n");
            true
        }
        Stage::Truncated => {
            let (line, opened) = verify_open(root, &opts.wal);
            text.push_str(&line);
            text.push_str(if opened {
                "result: repaired\n"
            } else {
                "result: truncated, but the log does not open; restore the segment from its .repair-backup copy\n"
            });
            opened
        }
    };
    Ok(Outcome { text, ok })
}

fn check_names(opts: &Options) -> Result<(), String> {
    if opts.dir.is_empty() || opts.dir.contains('\0') {
        return Err("--dir must be a non-empty path without NUL bytes".into());
    }
    let plain = Path::new(&opts.wal)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if opts.wal.is_empty() || opts.wal.contains('\0') || !plain {
        return Err(
            "--wal must name a directory inside --dir: plain names only, no `.`, `..` or leading `/`"
                .into(),
        );
    }
    Ok(())
}

/// Every directory on the path from `root` to the log must exist and must not be a link.
fn check_directories(root: &Path, wal: &str) -> Result<(), String> {
    let mut path = root.to_path_buf();
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("`{}` is not a plain directory", path.display()));
    }
    for part in Path::new(wal).components() {
        if let Component::Normal(name) = part {
            path.push(name);
        }
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "`{}` is a symbolic link; links are never followed",
                path.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!("`{}` is not a directory", path.display()));
        }
    }
    Ok(())
}

/// Segments and the lock file must be plain files: a link could send the repair elsewhere.
fn check_plain_files(root: &Path, wal: &str, names: &[String]) -> Result<(), String> {
    for name in names {
        if !(name.ends_with(".seg") || name == LOCK_NAME) {
            continue;
        }
        let path = root.join(wal).join(name);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
        if !meta.file_type().is_file() {
            return Err(format!(
                "`{}` is not a plain file; links and other entries are never followed",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Runs `body` while holding the log's lock file. A writer that holds it makes this refuse.
fn with_lock<T>(
    io: &mut StdIo,
    lock: &str,
    body: impl FnOnce(&mut StdIo) -> Result<T, String>,
) -> Result<T, String> {
    io.lock(lock).map_err(|e| {
        format!("refusing: the log is in use (cannot take `{lock}`: {e}); stop the server that owns it first")
    })?;
    let result = body(&mut *io);
    io.unlock(lock);
    result
}

fn scan_and_repair(io: &mut StdIo, opts: &Options, root: &Path) -> Result<(String, Stage), String> {
    let wal = opts.wal.as_str();
    let report = scrub(io, wal).map_err(|e| format!("cannot read the log: {e}"))?;
    let mut text = summary(wal, &report);
    match verdict(&report) {
        Verdict::Healthy => Ok((text, Stage::Healthy)),
        Verdict::Damaged => {
            if let Some(segment) = torn_creation(io, wal, &report) {
                text.push_str(&format!(
                    "PROBLEM: {segment} is shorter than its {SEGMENT_HEADER_LEN}-byte header: its creation stopped early. This command does not rewrite it. Start the server: opening the log rewrites a torn header on the newest segment. Do not truncate or delete the segment by hand.\n"
                ));
            } else {
                for issue in &report.issues {
                    text.push_str(&describe(issue));
                    text.push('\n');
                }
            }
            Ok((text, Stage::Damaged))
        }
        Verdict::TornTail(tail) => {
            text.push_str(&describe_tail(&tail));
            text.push('\n');
            if !opts.apply {
                text.push_str(&format!(
                    "dry run: nothing was changed. Run again with --apply to keep a verified copy of {} and truncate it.\n",
                    tail.segment
                ));
                return Ok((text, Stage::Repairable));
            }
            text.push_str(&truncate_torn_tail(io, root, wal, &tail)?);
            Ok((text, Stage::Truncated))
        }
    }
}

/// The newest segment, when its header is shorter than 16 bytes and that is its only finding: a
/// segment whose creation stopped early. Recovery rewrites such a header when the log is next opened.
fn torn_creation(io: &StdIo, wal: &str, report: &ScrubReport) -> Option<String> {
    let [only] = report.issues.as_slice() else {
        return None;
    };
    let Issue::BadHeader { segment, .. } = only else {
        return None;
    };
    let newest = io
        .list(wal)
        .ok()?
        .into_iter()
        .filter(|n| n.ends_with(".seg"))
        .max()?;
    if *segment != newest {
        return None;
    }
    let len = io.len(&format!("{wal}/{segment}")).ok()?;
    (len < SEGMENT_HEADER_LEN as u64).then(|| segment.clone())
}

fn summary(wal: &str, report: &ScrubReport) -> String {
    format!(
        "log `{wal}`: {} segments, {} records, {} change sets\n",
        report.segments, report.records, report.change_sets
    )
}

/// Only a lone torn tail is repairable. Any other finding, or a second tail, is damage.
fn verdict(report: &ScrubReport) -> Verdict {
    let mut tails = Vec::new();
    for issue in &report.issues {
        match issue {
            Issue::TornTail {
                segment,
                offset,
                bytes,
            } => tails.push(Tail {
                segment: segment.clone(),
                offset: *offset,
                bytes: *bytes,
            }),
            _ => return Verdict::Damaged,
        }
    }
    match tails.pop() {
        None => Verdict::Healthy,
        Some(tail) if tails.is_empty() => Verdict::TornTail(tail),
        Some(_) => Verdict::Damaged,
    }
}

fn describe_tail(tail: &Tail) -> String {
    format!(
        "torn tail in {} at offset {} ({} bytes). The repair would truncate {} at offset {}, removing those {} bytes: they form no complete record and no valid record follows them, so recovery discards them too.",
        tail.segment, tail.offset, tail.bytes, tail.segment, tail.offset, tail.bytes
    )
}

/// One line per finding. Only segment names, offsets, LSNs and counts appear, never payload bytes.
fn describe(issue: &Issue) -> String {
    match issue {
        Issue::TornTail {
            segment,
            offset,
            bytes,
        } => format!("note: torn tail in {segment} at offset {offset} ({bytes} bytes)"),
        Issue::MissingSegment {
            segment,
            expected,
            found,
        } => format!(
            "PROBLEM: {segment} starts at lsn {found} but lsn {expected} was expected: a segment is missing"
        ),
        Issue::BadHeader { segment, problem } => {
            format!("PROBLEM: {segment} has a bad header: {problem}")
        }
        Issue::MidLogDamage { segment, offset } => format!(
            "PROBLEM: {segment} is damaged at offset {offset} and recovery refuses to truncate it (damage with log after it, or a checksum-valid record out of LSN order)"
        ),
        Issue::Undecodable { lsn, what } => {
            format!("PROBLEM: the record at lsn {lsn} does not decode: {what}")
        }
        Issue::DuplicateId { lsn, .. } => {
            format!("PROBLEM: the change set at lsn {lsn} repeats an id that is already logged")
        }
        Issue::HlcNotIncreasing { lsn } => {
            format!("PROBLEM: the timestamp at lsn {lsn} is not after the one before it")
        }
        Issue::FencedWriteLogged { lsn, .. } => format!(
            "PROBLEM: the write at lsn {lsn} was logged after a newer turn of its session began"
        ),
        Issue::DerivedStateMismatch { what } => {
            format!("PROBLEM: state disagrees with the log: {what}")
        }
    }
}

/// Copies the segment, verifies the copy, truncates the tail and rescans. Any step that fails
/// stops the repair; the copy is kept whenever the truncation may have started.
fn truncate_torn_tail(
    io: &mut StdIo,
    root: &Path,
    wal: &str,
    tail: &Tail,
) -> Result<String, String> {
    let segment = format!("{wal}/{}", tail.segment);
    let backup = format!("{segment}{BACKUP_SUFFIX}");
    let backup_path = root.join(&backup);
    // `exists` follows links, so ask the file system about the entry itself.
    match std::fs::symlink_metadata(&backup_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => {
            return Err(format!(
                "refusing: `{}` already exists or is a link; move it aside first",
                backup_path.display()
            ));
        }
    }
    let original = io
        .read(&segment)
        .map_err(|e| format!("cannot read `{segment}`: {e}"))?;
    if tail.offset.checked_add(tail.bytes) != Some(original.len()) {
        return Err(format!(
            "refusing: `{segment}` changed since it was scanned"
        ));
    }
    write_backup(io, root, wal, &backup, &original)?;
    io.truncate(&segment, tail.offset as u64).map_err(|e| {
        format!(
            "truncating `{segment}` failed ({e}); the copy is at `{}`",
            backup_path.display()
        )
    })?;
    io.sync(&segment).map_err(|e| {
        format!(
            "syncing `{segment}` failed ({e}); the copy is at `{}`",
            backup_path.display()
        )
    })?;
    let rescan = scrub(io, wal).map_err(|e| format!("cannot rescan `{wal}`: {e}"))?;
    if !rescan.issues.is_empty() {
        return Err(format!(
            "refusing to report success: the rescan still finds {} finding(s); the copy is at `{}`",
            rescan.issues.len(),
            backup_path.display()
        ));
    }
    Ok(format!(
        "backup: {} ({} bytes, read back and identical to the original)\ntruncated: {} at offset {}, removing {} bytes\nrescan: no findings\n",
        backup_path.display(),
        original.len(),
        tail.segment,
        tail.offset,
        tail.bytes
    ))
}

/// Creates the copy exclusively, writes it, makes it durable and reads it back through the same
/// handle. `create_new` fails if anything, a file or a link, already exists at the path, so a path
/// that appears after the check above is never written through. Only a file this call created is
/// removed again, and only when the copy fails.
fn write_backup(
    io: &mut StdIo,
    root: &Path,
    wal: &str,
    backup: &str,
    original: &[u8],
) -> Result<(), String> {
    let path = root.join(backup);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            format!(
                "refusing: the backup `{}` could not be created ({e}); nothing was truncated",
                path.display()
            )
        })?;
    // The file exists because this call just created it, so it is the one file this call may remove.
    match write_and_verify(&mut file, io, wal, original) {
        Ok(()) => Ok(()),
        Err(e) => {
            drop(file);
            let _ = std::fs::remove_file(&path);
            Err(format!(
                "refusing: the backup `{}` could not be written and verified ({e}); nothing was truncated",
                path.display()
            ))
        }
    }
}

fn write_and_verify(
    file: &mut std::fs::File,
    io: &mut StdIo,
    wal: &str,
    original: &[u8],
) -> Result<(), String> {
    file.write_all(original).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    io.sync_dir(wal).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut back = Vec::with_capacity(original.len());
    file.read_to_end(&mut back).map_err(|e| e.to_string())?;
    if !copy_matches(original, &back) {
        return Err("the copy does not match the original".into());
    }
    Ok(())
}

/// Whether a copy read back from the disk is identical to the bytes it was made from.
fn copy_matches(original: &[u8], read_back: &[u8]) -> bool {
    original == read_back
}

/// Opens the repaired log with the recovery a server uses and reports what it found.
fn verify_open(root: &Path, wal: &str) -> (String, bool) {
    match Log::open(StdIo::new(root), wal, Config::default().segment_limit) {
        Ok((_log, report)) => (
            format!(
                "verified: the log opens with {} records; recovery removed {} bytes\n",
                report.records, report.truncated_bytes
            ),
            report.truncated_bytes == 0,
        ),
        Err(e) => (
            format!("PROBLEM: the repaired log does not open: {e}\n"),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// CRC-32C from its definition in formats/wal-v1.md (reflected 0x82F63B78, initial and final
    /// xor 0xFFFFFFFF). It is written out here so the fixtures do not depend on the code under test.
    fn crc32c(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// The record type used by fixtures. It is not one of the kernel's types, so scrub keeps the
    /// payload opaque and checks only the framing.
    const OPAQUE: u8 = 0x7F;
    const PAYLOAD: &[u8] = b"abcd";
    /// A record with a 4-byte payload: 4 + 4 + 8 + 1 + 4 bytes.
    const FRAME_LEN: usize = 21;
    /// The byte offset of the first record after the 16-byte segment header.
    const FIRST_FRAME: usize = 16;

    /// One record frame: length u32, crc32c u32, lsn u64, type u8, payload. The checksum covers the
    /// length, the lsn, the type and the payload.
    fn frame(lsn: u64, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u32;
        let mut covered = Vec::new();
        covered.extend_from_slice(&len.to_le_bytes());
        covered.extend_from_slice(&lsn.to_le_bytes());
        covered.push(OPAQUE);
        covered.extend_from_slice(payload);
        let mut out = Vec::new();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&crc32c(&covered).to_le_bytes());
        out.extend_from_slice(&lsn.to_le_bytes());
        out.push(OPAQUE);
        out.extend_from_slice(payload);
        out
    }

    /// Records `first..first+count`, each with the fixture payload.
    fn frames(first: u64, count: u64) -> Vec<u8> {
        (first..first + count)
            .flat_map(|lsn| frame(lsn, PAYLOAD))
            .collect()
    }

    /// A segment header: magic "LATLOG01", format version 1, reserved 0.
    fn header() -> Vec<u8> {
        let mut h = b"LATLOG01".to_vec();
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h
    }

    /// Three good records and then the start of a record that never completes.
    const TORN_OFFSET: usize = FIRST_FRAME + 3 * FRAME_LEN;

    /// A throwaway data directory with a `wal` subdirectory.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("lattice-log-repair-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("wal")).unwrap();
            Dir(path)
        }

        fn root(&self) -> &str {
            self.0.to_str().unwrap()
        }

        fn seg_path(&self, first: u64) -> PathBuf {
            self.0.join("wal").join(format!("{first:020}.seg"))
        }

        fn backup_path(&self, first: u64) -> PathBuf {
            self.0
                .join("wal")
                .join(format!("{first:020}.seg{BACKUP_SUFFIX}"))
        }

        fn write_seg(&self, first: u64, bytes: &[u8]) {
            std::fs::write(self.seg_path(first), bytes).unwrap();
        }

        fn read_seg(&self, first: u64) -> Vec<u8> {
            std::fs::read(self.seg_path(first)).unwrap()
        }

        /// Every entry of the log directory with its bytes, sorted by name. The lock file is left
        /// out: taking it is not a change to the log, and `--apply` creates it like `Log::open` does.
        fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
            let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(self.0.join("wal"))
                .unwrap()
                .map(|e| {
                    let e = e.unwrap();
                    let name = e.file_name().to_string_lossy().into_owned();
                    (name, std::fs::read(e.path()).unwrap_or_default())
                })
                .collect();
            out.retain(|(name, _)| name != LOCK_NAME);
            out.sort();
            out
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opts(d: &Dir, apply: bool) -> Options {
        Options {
            dir: d.root().into(),
            wal: "wal".into(),
            apply,
        }
    }

    /// A log whose only segment ends in a torn tail. Returns the directory and the original bytes.
    fn torn_log(name: &str) -> (Dir, Vec<u8>) {
        let d = Dir::new(name);
        let mut bytes = [header(), frames(1, 3)].concat();
        assert_eq!(bytes.len(), TORN_OFFSET);
        bytes.extend_from_slice(&[0xAB; 5]);
        d.write_seg(1, &bytes);
        (d, bytes)
    }

    #[test]
    fn a_torn_header_in_the_newest_segment_is_refused_with_guidance_and_untouched() {
        let d = Dir::new("torn-header");
        // Nine bytes of a 16-byte header: a segment whose creation stopped early.
        d.write_seg(1, b"LATLOG01\x01");
        let before = d.snapshot();
        for apply in [false, true] {
            let out = repair(&opts(&d, apply)).unwrap();
            assert!(!out.ok, "{}", out.text);
            assert!(out.text.contains("Start the server"), "{}", out.text);
            assert!(out.text.contains("rewrites a torn header"), "{}", out.text);
        }
        assert_eq!(d.snapshot(), before);
        assert!(!d.backup_path(1).exists());
    }

    #[test]
    fn a_log_that_ends_exactly_at_a_record_boundary_has_no_finding() {
        let d = Dir::new("boundary");
        d.write_seg(1, &[header(), frames(1, 3)].concat());
        let report = scrub(&StdIo::new(d.root()), "wal").unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        let before = d.snapshot();
        let out = repair(&opts(&d, true)).unwrap();
        assert!(
            out.ok && out.text.contains("result: healthy"),
            "{}",
            out.text
        );
        assert_eq!(d.snapshot(), before);
        assert!(!d.backup_path(1).exists());
    }

    #[test]
    fn a_read_back_that_differs_from_the_copy_is_a_mismatch() {
        // The fault-injected path needs real storage faults; the comparison it relies on is
        // checked directly here.
        assert!(copy_matches(b"abc", b"abc"));
        assert!(!copy_matches(b"abc", b"abd"));
        assert!(!copy_matches(b"abc", b"ab"));
        assert!(!copy_matches(b"abc", b"abcd"));
    }

    #[test]
    fn the_fixture_crc_matches_the_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn a_dry_run_of_a_healthy_log_reports_it_and_changes_nothing() {
        let d = Dir::new("healthy");
        d.write_seg(1, &[header(), frames(1, 3)].concat());
        let before = d.snapshot();
        let out = repair(&opts(&d, false)).unwrap();
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("result: healthy"), "{}", out.text);
        // A dry run of a log with no LOCK file creates none.
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn a_dry_run_names_the_torn_tail_offset_and_length_and_changes_nothing() {
        let (d, _) = torn_log("dry");
        let before = d.snapshot();
        let out = repair(&opts(&d, false)).unwrap();
        assert!(out.ok, "{}", out.text);
        assert!(
            out.text
                .contains(&format!("at offset {TORN_OFFSET} (5 bytes)")),
            "{}",
            out.text
        );
        assert!(out.text.contains("result: repairable"), "{}", out.text);
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn apply_keeps_a_verified_copy_truncates_and_the_log_reopens() {
        let (d, original) = torn_log("apply");
        let out = repair(&opts(&d, true)).unwrap();
        assert!(
            out.ok && out.text.contains("result: repaired"),
            "{}",
            out.text
        );
        assert_eq!(d.read_seg(1), original.get(..TORN_OFFSET).unwrap());
        assert_eq!(std::fs::read(d.backup_path(1)).unwrap(), original);
        let (log, report) = Log::open(StdIo::new(d.root()), "wal", 1 << 20).unwrap();
        assert_eq!(report.records, 3);
        assert_eq!(report.truncated_bytes, 0);
        drop(log);
        // A second run finds nothing to repair and leaves the copy alone.
        let again = repair(&opts(&d, true)).unwrap();
        assert!(
            again.ok && again.text.contains("result: healthy"),
            "{}",
            again.text
        );
        assert_eq!(std::fs::read(d.backup_path(1)).unwrap(), original);
    }

    #[test]
    fn an_existing_backup_is_never_overwritten() {
        let (d, _) = torn_log("backup-exists");
        std::fs::write(d.backup_path(1), b"an earlier copy").unwrap();
        let before = d.snapshot();
        let err = repair(&opts(&d, true)).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(d.snapshot(), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_path_that_is_a_link_is_refused_and_its_target_is_untouched() {
        let (d, original) = torn_log("backup-link");
        let victim = d.0.join("victim");
        std::fs::write(&victim, b"victim").unwrap();
        std::os::unix::fs::symlink(&victim, d.backup_path(1)).unwrap();
        let err = repair(&opts(&d, true)).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read(&victim).unwrap(), b"victim");
        assert_eq!(d.read_seg(1), original);
    }

    #[test]
    fn interior_damage_is_reported_refused_and_never_changed() {
        let d = Dir::new("interior");
        let mut bytes = [header(), frames(1, 4)].concat();
        // Record 2 starts at byte 37 and its payload 17 bytes later. One flipped byte fails its
        // checksum while records 3 and 4 still follow it.
        bytes[FIRST_FRAME + FRAME_LEN + 17] ^= 0xFF;
        d.write_seg(1, &bytes);
        let before = d.snapshot();
        for apply in [false, true] {
            let out = repair(&opts(&d, apply)).unwrap();
            assert!(!out.ok, "{}", out.text);
            assert!(out.text.contains("refuses to truncate"), "{}", out.text);
            assert!(out.text.contains("result: damaged"), "{}", out.text);
        }
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn a_checksum_valid_frame_with_the_wrong_lsn_at_the_end_is_refused() {
        // Issue 441: lsn 3 is lost and the frame after lsn 2 carries lsn 4. It is complete and
        // checksum-valid, so it is not a torn write and must not be truncated.
        let d = Dir::new("out-of-order");
        let bytes = [header(), frames(1, 2), frame(4, PAYLOAD)].concat();
        d.write_seg(1, &bytes);
        let before = d.snapshot();
        let out = repair(&opts(&d, true)).unwrap();
        assert!(
            !out.ok && out.text.contains("refuses to truncate"),
            "{}",
            out.text
        );
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn a_missing_segment_is_refused() {
        let d = Dir::new("missing-segment");
        d.write_seg(1, &[header(), frames(1, 2)].concat());
        d.write_seg(5, &[header(), frames(5, 1)].concat());
        let before = d.snapshot();
        let out = repair(&opts(&d, true)).unwrap();
        assert!(!out.ok && out.text.contains("missing"), "{}", out.text);
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn a_torn_tail_in_an_older_segment_is_refused() {
        let d = Dir::new("older-torn");
        let mut first = [header(), frames(1, 2)].concat();
        first.extend_from_slice(&[0xAB; 5]);
        d.write_seg(1, &first);
        d.write_seg(3, &[header(), frames(3, 1)].concat());
        let before = d.snapshot();
        let out = repair(&opts(&d, true)).unwrap();
        assert!(!out.ok, "{}", out.text);
        assert_eq!(d.snapshot(), before);
    }

    #[test]
    fn a_running_writer_makes_both_modes_refuse_and_leaves_the_log_alone() {
        let (d, _) = torn_log("live");
        let mut holder = StdIo::new(d.root());
        holder.lock("wal/LOCK").unwrap();
        let before = d.snapshot();
        for apply in [false, true] {
            let err = repair(&opts(&d, apply)).unwrap_err();
            assert!(err.contains("in use"), "{err}");
        }
        assert_eq!(d.snapshot(), before);
        holder.unlock("wal/LOCK");
        // Once the writer is gone the same log repairs normally.
        assert!(repair(&opts(&d, true)).unwrap().ok);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_segment_is_refused_and_its_target_is_never_touched() {
        let d = Dir::new("symlink-segment");
        let target = d.0.join("elsewhere.bin");
        let bytes = [header(), frames(1, 3), vec![0xAB; 5]].concat();
        std::fs::write(&target, &bytes).unwrap();
        std::os::unix::fs::symlink(&target, d.seg_path(1)).unwrap();
        for apply in [false, true] {
            let err = repair(&opts(&d, apply)).unwrap_err();
            assert!(err.contains("not a plain file"), "{err}");
        }
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_on_the_wal_path_is_refused() {
        let d = Dir::new("symlink-dir");
        let real = d.0.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::rename(d.0.join("wal"), real.join("wal")).unwrap();
        std::os::unix::fs::symlink(&real, d.0.join("link")).unwrap();
        let o = Options {
            dir: d.root().into(),
            wal: "link/wal".into(),
            apply: true,
        };
        let err = repair(&o).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");
    }

    #[test]
    fn a_wal_name_that_leaves_the_data_dir_is_refused_before_anything_is_read() {
        let d = Dir::new("escape");
        for bad in ["", "../wal", "/etc", ".", "a/../b", "a\0b"] {
            let o = Options {
                dir: d.root().into(),
                wal: bad.into(),
                apply: true,
            };
            assert!(repair(&o).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_missing_log_directory_is_reported_and_never_created() {
        let d = Dir::new("missing-dir");
        std::fs::remove_dir_all(d.0.join("wal")).unwrap();
        assert!(repair(&opts(&d, true)).is_err());
        assert!(!d.0.join("wal").exists());
    }

    #[test]
    fn only_a_lone_torn_tail_is_repairable() {
        let tail = |segment: &str| Issue::TornTail {
            segment: segment.into(),
            offset: 16,
            bytes: 3,
        };
        let mut r = ScrubReport::default();
        assert_eq!(verdict(&r), Verdict::Healthy);
        r.issues.push(tail("a.seg"));
        assert_eq!(
            verdict(&r),
            Verdict::TornTail(Tail {
                segment: "a.seg".into(),
                offset: 16,
                bytes: 3
            })
        );
        r.issues.push(tail("b.seg"));
        assert_eq!(verdict(&r), Verdict::Damaged);
        let mut r = ScrubReport::default();
        r.issues.push(Issue::MidLogDamage {
            segment: "a.seg".into(),
            offset: 16,
        });
        assert_eq!(verdict(&r), Verdict::Damaged);
        let mut r = ScrubReport::default();
        r.issues.push(tail("a.seg"));
        r.issues.push(Issue::HlcNotIncreasing { lsn: 4 });
        assert_eq!(verdict(&r), Verdict::Damaged);
    }

    #[test]
    fn arguments_are_strict_and_help_is_not_an_error() {
        let a = |s: &[&str]| parse(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert!(matches!(
            a(&["repair"]),
            Ok(Parsed::Run(o)) if !o.apply && o.dir == "." && o.wal == "wal"
        ));
        assert!(matches!(
            a(&["repair", "--apply", "--dir", "/d", "--wal", "w"]),
            Ok(Parsed::Run(o)) if o.apply && o.dir == "/d" && o.wal == "w"
        ));
        assert!(matches!(a(&["repair", "--help"]), Ok(Parsed::Help)));
        let bad: [&[&str]; 7] = [
            &[],
            &["show"],
            &["repair", "--force"],
            &["repair", "--dir"],
            &["repair", "--dir", "--apply"],
            &["repair", "--apply", "--apply"],
            &["repair", "--wal", "a", "--wal", "b"],
        ];
        for args in bad {
            assert!(a(args).is_err(), "{args:?} was accepted");
        }
    }
}
