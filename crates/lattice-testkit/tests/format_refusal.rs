//! Recovery must refuse what it cannot understand, loudly and without touching it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_core::record::{SEGMENT_HEADER_LEN, encode_record, segment_header};
use lattice_core::{Config, Kernel, LogError, TxnError};
use lattice_seams::Io;
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use lattice_testkit::scenario::{SharedClock, SharedIo};
use std::rc::Rc;

fn open(disk: &SharedIo) -> Result<(), TxnError> {
    let config = Config {
        dir: "wal".into(),
        segment_limit: 1 << 20,
        node: 1,
    };
    Kernel::open(
        disk.clone(),
        SharedClock(Rc::new(SimClock::new(1))),
        SeededRng::new(1),
        &config,
    )
    .map(|_| ())
}

fn disk_with(files: &[(&str, Vec<u8>)]) -> SharedIo {
    let mut io = SimIo::new(1, FaultProfile::clean());
    io.create_dir_all("wal").unwrap();
    for (name, bytes) in files {
        io.append(&format!("wal/{name}"), bytes).unwrap();
    }
    SharedIo::new(io)
}

fn corrupt_message(r: Result<(), TxnError>) -> String {
    match r {
        Err(TxnError::Log(LogError::Corrupt(m))) => m,
        other => panic!("expected a corruption refusal, got {other:?}"),
    }
}

#[test]
fn a_newer_format_version_is_refused_not_guessed() {
    let mut h = segment_header();
    h[8] = 9;
    let disk = disk_with(&[("00000000000000000001.seg", h)]);
    assert!(corrupt_message(open(&disk)).contains("UnsupportedVersion(9)"));
}

#[test]
fn a_short_header_is_only_repaired_in_the_newest_segment() {
    let mut first = segment_header();
    first.extend_from_slice(&encode_record(1, 1, b"x"));
    // Middle segment with a torn header: refuse. (The first record is not a valid change set but
    // the header problem is reported first.)
    let disk = disk_with(&[
        ("00000000000000000001.seg", segment_header()[..5].to_vec()),
        ("00000000000000000002.seg", segment_header()),
    ]);
    assert!(corrupt_message(open(&disk)).contains("header"));
    // The same torn header in the newest segment is a torn creation and is repaired.
    let disk = disk_with(&[("00000000000000000001.seg", segment_header()[..5].to_vec())]);
    assert!(open(&disk).is_ok());
}

#[test]
fn a_missing_segment_is_corruption() {
    let disk = disk_with(&[
        ("00000000000000000001.seg", segment_header()),
        ("00000000000000000009.seg", segment_header()),
    ]);
    assert!(corrupt_message(open(&disk)).contains("missing segment"));
}

#[test]
fn a_checksummed_but_malformed_known_record_is_corruption_not_ignored() {
    let mut seg = segment_header();
    // TurnBegin (type 2) with a 3 byte payload: valid framing, invalid content.
    seg.extend_from_slice(&encode_record(1, 2, &[1, 2, 3]));
    let disk = disk_with(&[("00000000000000000001.seg", seg)]);
    assert!(corrupt_message(open(&disk)).contains("turn begin"));
    // A ChangeSet record with an unknown envelope version likewise.
    let mut seg = segment_header();
    seg.extend_from_slice(&encode_record(1, 1, &[7, 0, 0]));
    let disk = disk_with(&[("00000000000000000001.seg", seg)]);
    assert!(corrupt_message(open(&disk)).contains("change set"));
}

#[test]
fn unknown_record_types_are_kept_opaque() {
    let mut seg = segment_header();
    seg.extend_from_slice(&encode_record(1, 200, b"future"));
    let disk = disk_with(&[("00000000000000000001.seg", seg)]);
    assert!(open(&disk).is_ok());
}

#[test]
fn a_second_writer_on_the_same_log_is_refused() {
    let disk = SharedIo::new(SimIo::new(1, FaultProfile::clean()));
    let config = Config {
        dir: "wal".into(),
        segment_limit: 1 << 20,
        node: 1,
    };
    let first = Kernel::open(
        disk.clone(),
        SharedClock(Rc::new(SimClock::new(1))),
        SeededRng::new(1),
        &config,
    );
    assert!(first.is_ok());
    assert!(open(&disk).is_err(), "two writers would interleave LSNs");
    drop(first);
    assert!(
        open(&disk).is_ok(),
        "the lock is released when the first writer goes away"
    );
    let _ = SEGMENT_HEADER_LEN;
}
