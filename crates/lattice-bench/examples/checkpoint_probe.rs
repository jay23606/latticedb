//! Host-only, fresh-process checkpoint recovery probe. Dataset construction and verification are
//! separate commands so their allocations/IO are excluded from recover-only RSS/time measurements.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
use lattice_core::{ChangeSet, ChangeSetId, Config, Kernel, Receipt, TurnRef};
use lattice_engines::{index::IndexDef, mvcc::Snapshot, store::RowStore};
use lattice_seams::{Io, IoError, OsRng, StdIo, SystemClock};
use serde_json::{Value, json};
use std::{cell::RefCell, path::Path, rc::Rc, time::Instant};

type Result<T> = std::result::Result<T, String>;
#[derive(Default)]
struct Stats {
    read_bytes: u64,
    prefix_read_bytes: u64,
    suffix_read_bytes: u64,
    snapshot_read_bytes: u64,
    largest_read: usize,
    len_calls: u64,
    directory_entries: u64,
    wal_directory_entries: u64,
    checkpoint_directory_entries: u64,
    collecting_list_calls: u64,
}
struct Measured {
    inner: StdIo,
    stats: Rc<RefCell<Stats>>,
    anchor: u64,
    full: bool,
}
impl Io for Measured {
    fn exists(&self, p: &str) -> bool {
        self.inner.exists(p)
    }
    fn len(&self, p: &str) -> std::result::Result<u64, IoError> {
        self.stats.borrow_mut().len_calls += 1;
        self.inner.len(p)
    }
    fn read(&self, p: &str) -> std::result::Result<Vec<u8>, IoError> {
        let bytes = self.inner.read(p)?;
        let mut s = self.stats.borrow_mut();
        let n = bytes.len() as u64;
        s.read_bytes += n;
        s.largest_read = s.largest_read.max(bytes.len());
        if p.contains("/.checkpoint/") {
            s.snapshot_read_bytes += n;
        } else if let Some(first) = p
            .rsplit('/')
            .next()
            .and_then(|n| n.strip_suffix(".seg"))
            .and_then(|n| n.parse::<u64>().ok())
        {
            if first < self.anchor {
                s.prefix_read_bytes += n;
            } else {
                s.suffix_read_bytes += n;
            }
        }
        Ok(bytes)
    }
    fn append(&mut self, p: &str, b: &[u8]) -> std::result::Result<(), IoError> {
        self.inner.append(p, b)
    }
    fn truncate(&mut self, p: &str, n: u64) -> std::result::Result<(), IoError> {
        self.inner.truncate(p, n)
    }
    fn sync(&mut self, p: &str) -> std::result::Result<(), IoError> {
        self.inner.sync(p)
    }
    fn sync_dir(&mut self, p: &str) -> std::result::Result<(), IoError> {
        self.inner.sync_dir(p)
    }
    fn list(&self, p: &str) -> std::result::Result<Vec<String>, IoError> {
        self.stats.borrow_mut().collecting_list_calls += 1;
        self.inner.list(p)
    }
    fn visit_dir(&self, p: &str, visit: &mut dyn FnMut(&str)) -> std::result::Result<(), IoError> {
        // The controlled full-replay baseline hides only the optional cache, never WAL segments.
        if self.full && p == "wal/.checkpoint" {
            return Err(IoError::NotFound(p.into()));
        }
        self.inner.visit_dir(p, &mut |name| {
            let mut s = self.stats.borrow_mut();
            s.directory_entries += 1;
            if p == "wal" {
                s.wal_directory_entries += 1;
            } else if p == "wal/.checkpoint" {
                s.checkpoint_directory_entries += 1;
            }
            drop(s);
            visit(name);
        })
    }
    fn remove(&mut self, p: &str) -> std::result::Result<(), IoError> {
        self.inner.remove(p)
    }
    fn create_dir_all(&mut self, p: &str) -> std::result::Result<(), IoError> {
        self.inner.create_dir_all(p)
    }
    fn lock(&mut self, p: &str) -> std::result::Result<(), IoError> {
        self.inner.lock(p)
    }
    fn unlock(&mut self, p: &str) {
        self.inner.unlock(p);
    }
}
fn error(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
fn number(args: &[String], name: &str, default: u64) -> Result<u64> {
    arg(args, name)
        .map(|s| s.parse().map_err(error))
        .unwrap_or(Ok(default))
}
fn get(v: &Value, key: &str) -> Result<u64> {
    v.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing numeric {key}"))
}
fn set(meta: &mut Value, key: &str, value: Value) -> Result<()> {
    meta.as_object_mut()
        .ok_or("dataset metadata must be an object")?
        .insert(key.to_string(), value);
    Ok(())
}
fn config(meta: &Value) -> Result<Config> {
    Ok(Config {
        segment_limit: get(meta, "segment_bytes")?,
        ..Config::default()
    })
}
fn defs() -> Vec<IndexDef> {
    vec![IndexDef {
        id: 1,
        table: 1,
        extract: |v| v.get(..8).map(|b| b.to_vec()),
    }]
}
fn row_value(key: u64, version: u64, bytes: usize) -> Vec<u8> {
    let mut out = vec![0x55; bytes.max(16)];
    for (dst, src) in out
        .iter_mut()
        .zip(version.to_le_bytes().into_iter().chain(key.to_le_bytes()))
    {
        *dst = src;
    }
    out
}
fn metadata(dir: &Path) -> Result<Value> {
    serde_json::from_slice(&std::fs::read(dir.join("dataset.json")).map_err(error)?).map_err(error)
}
fn prepare(args: &[String], dir: &Path) -> Result<Value> {
    if dir.exists() {
        return Err("dataset directory must be new; refusing to overwrite".into());
    }
    std::fs::create_dir_all(dir).map_err(error)?;
    let rows = number(args, "--rows", 128)?;
    let versions = number(args, "--versions", 2)?;
    let value_bytes = number(args, "--value-bytes", 256)?;
    let opaque_records = number(args, "--opaque-records", 64)?;
    let opaque_bytes = number(args, "--opaque-bytes", 4096)?;
    let segment_bytes = number(args, "--segment-bytes", 2 * 1024 * 1024)?;
    let tail_rows = number(args, "--tail-rows", 32)?;
    let tail_value_bytes = number(args, "--tail-value-bytes", 1024)?;
    if rows > 100_000
        || versions > 16
        || value_bytes > 8 * 1024 * 1024
        || opaque_records > 20_000
        || opaque_bytes > 8 * 1024 * 1024
        || tail_rows > 100_000
        || tail_value_bytes > 8 * 1024 * 1024
        || segment_bytes < 16
    {
        return Err("dataset parameters exceed bounded experiment limits".into());
    }
    let mut meta = json!({"rows":rows,"versions":versions,"value_bytes":value_bytes,"opaque_records":opaque_records,
        "opaque_bytes":opaque_bytes,"segment_bytes":segment_bytes,"tail_rows":tail_rows,"tail_value_bytes":tail_value_bytes});
    let cfg = config(&meta)?;
    let (mut kernel, _) = Kernel::open(StdIo::new(dir), SystemClock, OsRng, &cfg).map_err(error)?;
    kernel
        .begin_turn(TurnRef {
            session: 9,
            turn: 2,
        })
        .map_err(error)?;
    let mut first_id = None;
    for _ in 0..opaque_records {
        let id = kernel.new_id();
        if first_id.is_none() {
            first_id = Some(id);
        }
        if !matches!(
            kernel
                .commit(ChangeSet::new(id, vec![0xa5; opaque_bytes as usize]))
                .map_err(error)?,
            Receipt::Recorded { .. }
        ) {
            return Err("opaque ID was unexpectedly a duplicate".into());
        }
    }
    drop(kernel);
    let mut store = RowStore::open_with_indexes(StdIo::new(dir), SystemClock, OsRng, &cfg, defs())
        .map_err(error)?;
    let mut snapshots = Vec::new();
    for version in 0..versions {
        for key in 0..rows {
            let mut tx = store.begin();
            if version + 1 == versions && version > 0 && key % 7 == 0 {
                tx.delete((1, key.to_le_bytes().to_vec()));
            } else {
                tx.put(
                    (1, key.to_le_bytes().to_vec()),
                    row_value(key, version, value_bytes as usize),
                );
            }
            store.commit(tx).map_err(error)?;
        }
        snapshots.push(store.snapshot().as_of);
    }
    let applied = store.snapshot().as_of;
    let anchor = store.checkpoint().map_err(error)?;
    for key in 0..tail_rows {
        let mut tx = store.begin();
        tx.put(
            (2, key.to_le_bytes().to_vec()),
            row_value(key, 0, tail_value_bytes as usize),
        );
        store.commit(tx).map_err(error)?;
    }
    let final_applied = store.snapshot().as_of;
    drop(store);
    let io = StdIo::new(dir);
    let mut prefix_bytes = 0;
    let mut prefix_files = 0;
    let mut suffix_bytes = 0;
    let mut suffix_files = 0;
    let mut largest_segment = 0;
    for name in io.list("wal").map_err(error)? {
        if let Some(lsn) = name
            .strip_suffix(".seg")
            .and_then(|s| s.parse::<u64>().ok())
        {
            let len = io.len(&format!("wal/{name}")).map_err(error)?;
            largest_segment = largest_segment.max(len);
            if lsn < anchor {
                prefix_bytes += len;
                prefix_files += 1;
            } else {
                suffix_bytes += len;
                suffix_files += 1;
            }
        }
    }
    let mut checkpoint_state_bytes = 0;
    let mut snapshot_bytes = 0;
    let mut snapshot_files = 0;
    for name in io.list("wal/.checkpoint").map_err(error)? {
        let len = io.len(&format!("wal/.checkpoint/{name}")).map_err(error)?;
        snapshot_bytes += len;
        if name.ends_with(".chunk") {
            checkpoint_state_bytes += len;
        }
        snapshot_files += 1;
    }
    set(&mut meta, "anchor", json!(anchor))?;
    set(&mut meta, "source_applied", json!(applied))?;
    set(&mut meta, "final_applied", json!(final_applied))?;
    set(&mut meta, "historical_snapshots", json!(snapshots))?;
    set(&mut meta, "prefix_bytes", json!(prefix_bytes))?;
    set(&mut meta, "prefix_files", json!(prefix_files))?;
    set(&mut meta, "suffix_bytes", json!(suffix_bytes))?;
    set(&mut meta, "suffix_files", json!(suffix_files))?;
    set(&mut meta, "largest_segment", json!(largest_segment))?;
    set(
        &mut meta,
        "checkpoint_state_bytes",
        json!(checkpoint_state_bytes),
    )?;
    set(
        &mut meta,
        "row_tombstone_versions",
        json!(if versions > 1 { rows.div_ceil(7) } else { 0 }),
    )?;
    set(
        &mut meta,
        "retained_index_versions",
        json!(if versions > 0 {
            rows * (versions * 2 - 1) - if versions > 1 { rows.div_ceil(7) } else { 0 }
        } else {
            0
        }),
    )?;
    set(&mut meta, "snapshot_file_bytes", json!(snapshot_bytes))?;
    set(&mut meta, "snapshot_files", json!(snapshot_files))?;
    set(
        &mut meta,
        "retained_row_versions",
        json!(rows * versions + tail_rows),
    )?;
    set(
        &mut meta,
        "kernel_committed_ids",
        json!(opaque_records + rows * versions + tail_rows),
    )?;
    set(
        &mut meta,
        "first_opaque_id",
        json!(first_id.map(|id| {
            id.to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        })),
    )?;
    std::fs::write(
        dir.join("dataset.json"),
        serde_json::to_vec_pretty(&meta).map_err(error)?,
    )
    .map_err(error)?;
    Ok(meta)
}
fn sample<I: Io>(store: &RowStore<I, SystemClock, OsRng>, meta: &Value) -> Result<()> {
    let rows = get(meta, "rows")?;
    let versions = get(meta, "versions")?;
    let snapshots = meta
        .get("historical_snapshots")
        .and_then(Value::as_array)
        .ok_or("snapshots missing")?;
    for (version, snapshot) in snapshots.iter().enumerate() {
        for key in [0, rows / 2, rows.saturating_sub(1)] {
            if rows == 0 {
                continue;
            }
            let value = store.get_at(
                Snapshot {
                    as_of: snapshot.as_u64().ok_or("snapshot LSN")?,
                },
                1,
                &key.to_le_bytes(),
            );
            let deleted = version as u64 + 1 == versions && version > 0 && key % 7 == 0;
            if deleted {
                if value.is_some() {
                    return Err("lost tombstone".into());
                }
            } else if value.map(Vec::as_slice)
                != Some(
                    row_value(key, version as u64, get(meta, "value_bytes")? as usize).as_slice(),
                )
            {
                return Err("historical sample mismatch".into());
            }
        }
    }
    let expected_applied = get(meta, "final_applied")?;
    if store.snapshot().as_of != expected_applied {
        return Err(format!(
            "applied LSN mismatch: {} != {expected_applied}",
            store.snapshot().as_of
        ));
    }
    Ok(())
}
fn recover(args: &[String], dir: &Path) -> Result<Value> {
    let meta = metadata(dir)?;
    let full = match arg(args, "--mode").as_deref() {
        Some("full") => true,
        Some("checkpoint") => false,
        _ => return Err("--mode full|checkpoint required".into()),
    };
    let stats = Rc::new(RefCell::new(Stats::default()));
    let io = Measured {
        inner: StdIo::new(dir),
        stats: stats.clone(),
        anchor: get(&meta, "anchor")?,
        full,
    };
    let started = Instant::now();
    let store = RowStore::open_with_indexes(io, SystemClock, OsRng, &config(&meta)?, defs())
        .map_err(error)?;
    let open_ns = started.elapsed().as_nanos();
    sample(&store, &meta)?;
    let (replay, checkpoint) = store.recovery_report();
    let s = stats.borrow();
    if !full
        && (checkpoint.through_lsn + 1 != get(&meta, "anchor")?
            || s.prefix_read_bytes != 0
            || s.collecting_list_calls != 0)
    {
        return Err("checkpoint recovery did not use expected bounded path".into());
    }
    if full && checkpoint.state_bytes != 0 {
        return Err("full baseline unexpectedly used checkpoint".into());
    }
    Ok(
        json!({"mode":if full {"full"} else {"checkpoint"},"open_ns":open_ns,"snapshot_state_bytes":checkpoint.state_bytes,
        "through_lsn":checkpoint.through_lsn,"replayed_records":replay.records,"scanned_segments":replay.segments,
        "read_bytes":s.read_bytes,"prefix_read_bytes":s.prefix_read_bytes,"suffix_read_bytes":s.suffix_read_bytes,"snapshot_read_bytes":s.snapshot_read_bytes,
        "largest_read":s.largest_read,"len_calls":s.len_calls,"directory_entries":s.directory_entries,
        "wal_directory_entries":s.wal_directory_entries,"checkpoint_directory_entries":s.checkpoint_directory_entries,"collecting_list_calls":s.collecting_list_calls}),
    )
}
// Complete byte-level row model; deliberately outside the measured recovery process. The index
// keys cover only the first eight value bytes, so index consistency alone cannot check the body.
fn verify_all_rows<I: Io>(store: &RowStore<I, SystemClock, OsRng>, meta: &Value) -> Result<()> {
    let rows = get(meta, "rows")?;
    let versions = get(meta, "versions")?;
    let snapshots = meta
        .get("historical_snapshots")
        .and_then(Value::as_array)
        .ok_or("snapshots missing")?;
    for (version, snapshot) in snapshots.iter().enumerate() {
        let snap = Snapshot {
            as_of: snapshot.as_u64().ok_or("snapshot LSN")?,
        };
        let final_version = version as u64 + 1 == versions && version > 0;
        let expected_live = rows - if final_version { rows.div_ceil(7) } else { 0 };
        if store.scan_at(snap, 1, ..).len() as u64 != expected_live {
            return Err(format!("extra/missing source rows at version {version}"));
        }
        for key in 0..rows {
            let value = store.get_at(snap, 1, &key.to_le_bytes());
            if final_version && key % 7 == 0 {
                if value.is_some() {
                    return Err(format!("lost tombstone at version {version}, key {key}"));
                }
            } else if value.map(Vec::as_slice)
                != Some(
                    row_value(key, version as u64, get(meta, "value_bytes")? as usize).as_slice(),
                )
            {
                return Err(format!(
                    "source bytes differ at version {version}, key {key}"
                ));
            }
        }
    }
    let current = store.snapshot();
    let expected_source_live = if versions == 0 {
        0
    } else {
        rows - if versions > 1 { rows.div_ceil(7) } else { 0 }
    };
    if store.scan_at(current, 1, ..).len() as u64 != expected_source_live {
        return Err("source count changed during suffix replay".into());
    }
    for key in 0..rows {
        let value = store.get_at(current, 1, &key.to_le_bytes());
        if versions == 0 || (versions > 1 && key % 7 == 0) {
            if value.is_some() {
                return Err(format!("current source tombstone differs at key {key}"));
            }
        } else if value.map(Vec::as_slice)
            != Some(row_value(key, versions - 1, get(meta, "value_bytes")? as usize).as_slice())
        {
            return Err(format!("current source bytes differ at key {key}"));
        }
    }
    let tail_rows = get(meta, "tail_rows")?;
    if store.scan_at(current, 2, ..).len() as u64 != tail_rows {
        return Err("extra/missing suffix rows".into());
    }
    for key in 0..tail_rows {
        if store
            .get_at(current, 2, &key.to_le_bytes())
            .map(Vec::as_slice)
            != Some(row_value(key, 0, get(meta, "tail_value_bytes")? as usize).as_slice())
        {
            return Err(format!("suffix bytes differ at key {key}"));
        }
    }
    Ok(())
}

fn verify(dir: &Path) -> Result<Value> {
    let meta = metadata(dir)?;
    let store =
        RowStore::open_with_indexes(StdIo::new(dir), SystemClock, OsRng, &config(&meta)?, defs())
            .map_err(error)?;
    sample(&store, &meta)?;
    verify_all_rows(&store, &meta)?;
    for snap in meta
        .get("historical_snapshots")
        .and_then(Value::as_array)
        .ok_or("snapshots")?
    {
        store
            .check_indexes_at(Snapshot {
                as_of: snap.as_u64().ok_or("snapshot")?,
            })
            .map_err(error)?;
    }
    store.check_indexes().map_err(error)?;
    drop(store);
    let (mut k, _) =
        Kernel::open(StdIo::new(dir), SystemClock, OsRng, &config(&meta)?).map_err(error)?;
    if k.begin_turn(TurnRef {
        session: 9,
        turn: 1,
    })
    .is_ok()
    {
        return Err("lost turn fence".into());
    }
    if let Some(id) = meta.get("first_opaque_id").and_then(Value::as_str) {
        if id.len() != 32 {
            return Err("opaque id length".into());
        }
        let mut bytes = [0; 16];
        for (out, pair) in bytes
            .iter_mut()
            .zip(id.as_bytes().as_chunks::<2>().0.iter())
        {
            *out =
                u8::from_str_radix(std::str::from_utf8(pair).map_err(error)?, 16).map_err(error)?;
        }
        if k.commit(ChangeSet::new(ChangeSetId::from_bytes(bytes), vec![]))
            .map_err(error)?
            != Receipt::Duplicate
        {
            return Err("lost dedupe id".into());
        }
    }
    let scrub = k.scrub().map_err(error)?;
    if !scrub.issues.is_empty() {
        return Err(format!("scrub: {:?}", scrub.issues));
    }
    Ok(
        json!({"verified":true,"verification_is_separate_from_measured_process":true,"full_row_bytes_and_counts_verified":true}),
    )
}
fn run() -> Result<Value> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = arg(&args, "--dir").ok_or("--dir required")?;
    match args.first().map(String::as_str) {
        Some("prepare") => prepare(&args, Path::new(&dir)),
        Some("recover") => recover(&args, Path::new(&dir)),
        Some("verify") => verify(Path::new(&dir)),
        _ => Err("prepare|recover|verify required".into()),
    }
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(value) => {
            println!("{value}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
