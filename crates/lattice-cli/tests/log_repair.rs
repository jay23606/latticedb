//! `lattice log repair` through the real binary: exit codes are what operators and scripts depend
//! on, and the output must never show record payloads.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use lattice_seams::{Io, StdIo};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

// A fork can briefly hold another test thread's open lock file. Every spawn and the in-process lock
// used by the live-writer test take this guard, so no child inherits a lock that is about to end.
static PROCESS_IO: Mutex<()> = Mutex::new(());

const OPAQUE: u8 = 0x7F;
/// A 4-byte payload: a record is 17 + payload bytes, so three of them end at byte 16 + 63 = 79.
const SMALL: &[u8] = b"abcd";
/// A payload the output must never repeat.
const MARKER: &[u8] = b"PAYLOAD-MARKER";

/// CRC-32C from formats/wal-v1.md, written out here rather than borrowed from the code under test.
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

/// One record frame: length u32, crc32c u32, lsn u64, type u8, payload.
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

/// A segment header: magic "LATLOG01", format version 1, reserved 0.
fn header() -> Vec<u8> {
    let mut h = b"LATLOG01".to_vec();
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h
}

/// Three good records and then five bytes that never form a record.
fn torn_segment() -> Vec<u8> {
    let mut bytes = header();
    for lsn in 1..=3 {
        bytes.extend(frame(lsn, SMALL));
    }
    bytes.extend_from_slice(&[0xAB; 5]);
    bytes
}

/// A data directory whose `wal` subdirectory holds one segment with `bytes`.
fn data_dir(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lattice-log-repair-cli-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("wal")).unwrap();
    std::fs::write(segment(&dir), bytes).unwrap();
    dir
}

fn segment(dir: &Path) -> PathBuf {
    dir.join("wal").join(format!("{:020}.seg", 1))
}

/// Runs `lattice` with `args`, holding the spawn guard.
fn lattice(args: &[&str]) -> Output {
    let _guard = PROCESS_IO.lock().unwrap();
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .output()
        .unwrap()
}

/// Runs `lattice log repair --dir DIR` followed by `extra`.
fn repair(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["log", "repair", "--dir", dir.to_str().unwrap()];
    args.extend_from_slice(extra);
    lattice(&args)
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn bad_arguments_exit_two_and_help_exits_zero() {
    let cases: [&[&str]; 4] = [
        &["log"],
        &["log", "show"],
        &["log", "repair", "--force"],
        &["log", "repair", "--apply", "--apply"],
    ];
    for args in cases {
        let o = lattice(args);
        assert_eq!(code(&o), 2, "{args:?}: {}", text(&o));
    }
    let o = lattice(&["log", "repair", "--help"]);
    assert_eq!(code(&o), 0);
    assert!(
        String::from_utf8_lossy(&o.stdout).starts_with("usage: lattice log repair"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_torn_tail_is_reported_by_a_dry_run_and_repaired_by_apply() {
    let original = torn_segment();
    let dir = data_dir("torn", &original);

    let dry = repair(&dir, &[]);
    assert_eq!(code(&dry), 0, "{}", text(&dry));
    assert!(
        text(&dry).contains("at offset 79 (5 bytes)"),
        "{}",
        text(&dry)
    );
    assert_eq!(std::fs::read(segment(&dir)).unwrap(), original);

    let apply = repair(&dir, &["--apply"]);
    assert_eq!(code(&apply), 0, "{}", text(&apply));
    assert!(
        text(&apply).contains("result: repaired"),
        "{}",
        text(&apply)
    );
    assert_eq!(std::fs::metadata(segment(&dir)).unwrap().len(), 79);
    let backup = PathBuf::from(format!("{}.repair-backup", segment(&dir).display()));
    assert_eq!(std::fs::read(&backup).unwrap(), original);

    // The repaired log is healthy for the existing doctor, and a second apply changes nothing.
    let doctor = lattice(&["doctor", "--dir", dir.to_str().unwrap()]);
    assert_eq!(code(&doctor), 0, "{}", text(&doctor));
    let again = repair(&dir, &["--apply"]);
    assert_eq!(code(&again), 0, "{}", text(&again));
    assert!(text(&again).contains("result: healthy"), "{}", text(&again));
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interior_damage_exits_one_in_both_modes_and_output_never_shows_payload_bytes() {
    let mut bytes = header();
    for lsn in 1..=4 {
        bytes.extend(frame(lsn, MARKER));
    }
    // Record 2 starts at 16 + 31 (a 14-byte payload makes a 31-byte frame); its payload starts 17
    // bytes into the frame. One flipped byte fails its checksum while records 3 and 4 follow it.
    let payload_of_record_2 = 16 + 31 + 17;
    bytes[payload_of_record_2] ^= 0xFF;
    let dir = data_dir("interior", &bytes);
    for extra in [&[][..], &["--apply"][..]] {
        let o = repair(&dir, extra);
        assert_eq!(code(&o), 1, "{extra:?}: {}", text(&o));
        assert!(text(&o).contains("result: damaged"), "{}", text(&o));
        assert!(
            !text(&o).contains("PAYLOAD"),
            "payload bytes leaked: {}",
            text(&o)
        );
    }
    assert_eq!(std::fs::read(segment(&dir)).unwrap(), bytes);
    let backup = PathBuf::from(format!("{}.repair-backup", segment(&dir).display()));
    assert!(!backup.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_live_writer_makes_the_command_refuse_in_both_modes() {
    let original = torn_segment();
    let dir = data_dir("live", &original);
    {
        let _guard = PROCESS_IO.lock().unwrap();
        let mut writer = StdIo::new(&dir);
        writer.lock("wal/LOCK").unwrap();
        for extra in [&[][..], &["--apply"][..]] {
            let mut args = vec!["log", "repair", "--dir", dir.to_str().unwrap()];
            args.extend_from_slice(extra);
            let o = Command::new(env!("CARGO_BIN_EXE_lattice"))
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(code(&o), 1, "{extra:?}: {}", text(&o));
            assert!(text(&o).contains("in use"), "{}", text(&o));
        }
        writer.unlock("wal/LOCK");
    }
    assert_eq!(std::fs::read(segment(&dir)).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
}
