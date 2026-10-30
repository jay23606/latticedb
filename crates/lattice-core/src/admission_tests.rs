//! Independent byte, admission and durability witnesses for the WAL guard.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use crate::admission::{self, MANAGED_COMPOUND};
use crate::record;
use crate::{ChangeSet, ChangeSetId, Config, Kernel, Log, LogError};
use lattice_seams::{Io, IoError};
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const HEADER: &[u8] = include_bytes!("../../../formats/corpus/wal-admission-v2-header.golden");
const COMPOUND: &[u8] = include_bytes!("../../../formats/corpus/wal-admission-v2-compound.golden");
const UNKNOWN_CAP: &[u8] =
    include_bytes!("../../../formats/corpus/wal-admission-v2-unknown-capability.golden");
const UNKNOWN_VERSION: &[u8] =
    include_bytes!("../../../formats/corpus/wal-admission-v2-unknown-version.golden");
const LEGACY: &[u8] = include_bytes!("../../../formats/corpus/wal-admission-v1-legacy.golden");
const FIRST: &str = "wal/00000000000000000001.seg";

#[derive(Clone)]
struct Disk(Rc<RefCell<SimIo>>, Rc<Cell<usize>>);
impl Disk {
    fn new() -> Self {
        Self(
            Rc::new(RefCell::new(SimIo::new(3, FaultProfile::clean()))),
            Rc::new(Cell::new(0)),
        )
    }
    fn manifest(&self) -> Vec<(String, Vec<u8>)> {
        self.list("wal")
            .unwrap()
            .into_iter()
            .filter(|p| p.ends_with(".seg"))
            .map(|p| {
                let b = self.read(&format!("wal/{p}")).unwrap();
                (p, b)
            })
            .collect()
    }
}
impl Io for Disk {
    fn exists(&self, p: &str) -> bool {
        self.0.borrow().exists(p)
    }
    fn len(&self, p: &str) -> Result<u64, IoError> {
        self.0.borrow().len(p)
    }
    fn read(&self, p: &str) -> Result<Vec<u8>, IoError> {
        self.1.set(self.1.get() + 1);
        self.0.borrow().read(p)
    }
    fn read_prefix(&self, p: &str, n: usize) -> Result<Vec<u8>, IoError> {
        assert!(n <= 32);
        self.0.borrow().read_prefix(p, n)
    }
    fn append(&mut self, p: &str, b: &[u8]) -> Result<(), IoError> {
        self.0.borrow_mut().append(p, b)
    }
    fn truncate(&mut self, p: &str, n: u64) -> Result<(), IoError> {
        self.0.borrow_mut().truncate(p, n)
    }
    fn sync(&mut self, p: &str) -> Result<(), IoError> {
        self.0.borrow_mut().sync(p)
    }
    fn sync_dir(&mut self, p: &str) -> Result<(), IoError> {
        self.0.borrow_mut().sync_dir(p)
    }
    fn list(&self, p: &str) -> Result<Vec<String>, IoError> {
        self.0.borrow().list(p)
    }
    fn remove(&mut self, p: &str) -> Result<(), IoError> {
        self.0.borrow_mut().remove(p)
    }
    fn create_dir_all(&mut self, p: &str) -> Result<(), IoError> {
        self.0.borrow_mut().create_dir_all(p)
    }
    fn lock(&mut self, p: &str) -> Result<(), IoError> {
        self.0.borrow_mut().lock(p)
    }
    fn unlock(&mut self, p: &str) {
        self.0.borrow_mut().unlock(p)
    }
}
fn plant(disk: &mut Disk, path: &str, bytes: &[u8]) {
    disk.create_dir_all("wal").unwrap();
    disk.sync_dir("").unwrap();
    disk.append(path, bytes).unwrap();
    disk.sync(path).unwrap();
    disk.sync_dir("wal").unwrap();
}
fn open(disk: &Disk) -> Log<Disk> {
    Log::open_streaming_with_capabilities(disk.clone(), "wal", 1 << 20, 1, |_, _| {})
        .unwrap()
        .0
}

#[test]
fn independent_headers_and_compound_fixture_pin_exact_bytes_and_context() {
    assert_eq!(admission::guarded_header(1).unwrap(), HEADER);
    assert_eq!(record::segment_header(), LEGACY);
    let parsed = admission::decode_header(HEADER).unwrap();
    assert_eq!((parsed.length, parsed.required_capabilities), (32, 1));
    assert!(matches!(
        admission::decode_header(&UNKNOWN_VERSION[..16]),
        Err(admission::HeaderError::Invalid(_))
    ));
    let mut unknown_length = HEADER[..16].to_vec();
    unknown_length[12] = 16;
    assert!(matches!(
        admission::decode_header(&unknown_length),
        Err(admission::HeaderError::Invalid(_))
    ));
    for bytes in [
        include_bytes!("../../../formats/corpus/wal-admission-v2-wrong-length.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-reserved.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-bad-crc.golden").as_slice(),
    ] {
        assert!(admission::decode_header(bytes).is_err());
    }
    for bytes in [UNKNOWN_CAP, UNKNOWN_VERSION] {
        assert!(admission::decode_header(bytes).is_err());
    }
    for (n, prefix) in [
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-00.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-01.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-02.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-03.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-04.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-05.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-06.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-07.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-08.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-09.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-10.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-11.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-12.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-13.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-14.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-15.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-16.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-17.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-18.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-19.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-20.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-21.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-22.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-23.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-24.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-25.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-26.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-27.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-28.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-29.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-30.golden").as_slice(),
        include_bytes!("../../../formats/corpus/wal-admission-v2-prefix-31.golden").as_slice(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(prefix, &HEADER[..n]);
        assert!(admission::decode_header(prefix).is_err());
    }
    for n in 0..32 {
        let mut b = HEADER.to_vec();
        b[n] ^= 1;
        assert!(admission::decode_header(&b).is_err());
    }
    let dropped =
        include_bytes!("../../../formats/corpus/wal-admission-v2-dropped-capability.golden");
    let mut bad = Disk::new();
    plant(&mut bad, FIRST, &dropped[..201]);
    plant(&mut bad, "wal/00000000000000000002.seg", &dropped[201..]);
    assert!(
        Log::open_streaming_with_capabilities(bad, "wal", 1000, 1, |_, _| panic!(
            "downgrade callback"
        ))
        .is_err()
    );
    let mut disk = Disk::new();
    plant(&mut disk, FIRST, COMPOUND);
    let mut observed = Vec::new();
    let (kernel, report, _) = Kernel::open_streaming_with_capabilities(
        disk,
        SimClock::new(2),
        SeededRng::new(3),
        &Config::default(),
        1,
        |r, c| observed.push((r, c)),
    )
    .unwrap();
    assert_eq!(report.records, 1);
    assert_eq!(observed[0].1.required_capabilities, 1);
    let cs = ChangeSet::decode(&observed[0].0.payload).unwrap();
    assert_eq!(cs.id, ChangeSetId::from_bytes(1u128.to_be_bytes()));
    assert_eq!(&cs.payload[..5], b"LCIX\x01");
    assert_eq!(cs.payload.len(), 96);
    assert!(kernel.scrub().unwrap().is_clean());
    assert_eq!(kernel.read_from_with_context(1, 2).unwrap(), observed);
}

#[test]
fn unsupported_later_header_refuses_before_any_record_read_callback_or_tail_repair() {
    let mut disk = Disk::new();
    let mut first = LEGACY.to_vec();
    first.extend(record::encode_record(1, 42, b"legacy"));
    plant(&mut disk, FIRST, &first);
    plant(&mut disk, "wal/00000000000000000002.seg", HEADER);
    let before = disk.manifest();
    disk.1.set(0);
    let calls = Cell::new(0);
    assert!(matches!(
        Log::open_streaming(disk.clone(), "wal", 100, |_| calls.set(calls.get() + 1)),
        Err(LogError::UnsupportedCapabilities {
            required: 1,
            supported: 0
        })
    ));
    assert_eq!(calls.get(), 0);
    assert_eq!(
        disk.1.get(),
        0,
        "preflight must use bounded prefix reads only"
    );
    assert_eq!(disk.manifest(), before);
    // A caller supporting the known bit still refuses an unknown bit with a valid independent CRC.
    let mut bad = Disk::new();
    plant(&mut bad, FIRST, UNKNOWN_CAP);
    let before = bad.manifest();
    assert!(
        Log::open_streaming_with_capabilities(bad.clone(), "wal", 100, 1, |_, _| panic!(
            "callback before admission"
        ))
        .is_err()
    );
    assert_eq!(bad.manifest(), before);
}

#[test]
fn empty_tail_promotion_rollover_context_and_capability_rollback_are_checked() {
    let disk = Disk::new();
    let mut log = open(&disk);
    assert_eq!(log.next_lsn(), 1);
    assert!(matches!(
        log.append_with_capabilities(42, b"bad", 2),
        Err(LogError::UnsupportedCapabilities { .. })
    ));
    assert_eq!(log.next_lsn(), 1);
    assert_eq!(
        log.append_with_capabilities(42, b"protected", 1).unwrap(),
        1
    );
    assert_eq!(log.sync().unwrap(), 1);
    assert_eq!(disk.read_prefix(FIRST, 32).unwrap(), HEADER);
    assert_eq!(
        log.read_from_with_context(1, 1).unwrap()[0]
            .1
            .required_capabilities,
        1
    );
    drop(log);
    let mut log = Log::open_streaming_with_capabilities(disk.clone(), "wal", 1, 1, |_, _| {})
        .unwrap()
        .0;
    log.append(42, b"ordinary").unwrap();
    log.sync().unwrap();
    drop(log);
    for (_, bytes) in disk.manifest() {
        assert_eq!(
            admission::decode_header(&bytes)
                .unwrap()
                .required_capabilities,
            1
        );
    }
    let mut clone = disk.clone();
    let name = "wal/00000000000000000003.seg";
    clone.truncate(name, 0).unwrap();
    clone.append(name, LEGACY).unwrap();
    assert!(
        Log::open_streaming_with_capabilities(clone, "wal", 100, 1, |_, _| panic!(
            "rollback callback"
        ))
        .is_err()
    );
}

#[test]
fn kernel_guarded_append_preserves_typestate_dedupe_and_checkpoint_records() {
    let disk = Disk::new();
    let (mut k, _, _) = Kernel::open_streaming_with_capabilities(
        disk.clone(),
        SimClock::new(2),
        SeededRng::new(3),
        &Config::default(),
        1,
        |_, _| {},
    )
    .unwrap();
    let cs = ChangeSet::new(
        ChangeSetId::from_bytes(1u128.to_be_bytes()),
        b"opaque".to_vec(),
    );
    k.commit_with_capabilities(cs.clone(), 1).unwrap();
    assert_eq!(
        k.commit_with_capabilities(cs, 1).unwrap(),
        crate::Receipt::Duplicate
    );
    k.checkpoint().unwrap();
    assert!(k.scrub().unwrap().is_clean());
    drop(k);
    assert!(
        Kernel::open(
            disk.clone(),
            SimClock::new(3),
            SeededRng::new(4),
            &Config::default()
        )
        .is_err()
    );
    let (_, report, cp) = Kernel::open_streaming_with_capabilities(
        disk,
        SimClock::new(3),
        SeededRng::new(4),
        &Config::default(),
        1,
        |_, c| assert_eq!(c.required_capabilities, 1),
    )
    .unwrap();
    assert_eq!(report.records, 2);
    assert!(cp.is_some());
}

#[cfg(feature = "planted-bugs")]
#[test]
fn independent_omitted_guard_witness_has_a_clean_control() {
    use crate::bugs::{self, Bug};
    for mutate in [false, true] {
        let disk = Disk::new();
        let mut log = open(&disk);
        if mutate {
            bugs::enable(Bug::RequiredGuardOmitted);
        }
        log.append_with_capabilities(42, b"requires-one", 1)
            .unwrap();
        log.sync().unwrap();
        bugs::clear();
        let rows = log.read_from_with_context(1, 1).unwrap();
        let violated = rows[0].1.required_capabilities != 1;
        assert_eq!(
            violated, mutate,
            "literal required-feature oracle must catch an omitted guard"
        );
    }
}

#[test]
fn guarded_header_refuses_zero_and_unknown_requirements_independently() {
    // Each refusal must hold on its own: zero is never a guard, and a bit this build lacks is never encodable.
    assert!(matches!(
        admission::guarded_header(0),
        Err(admission::HeaderError::Invalid(_))
    ));
    assert!(matches!(
        admission::guarded_header(1 << 40),
        Err(admission::HeaderError::Invalid(_))
    ));
    assert!(matches!(
        admission::guarded_header(1 | (1 << 40)),
        Err(admission::HeaderError::Invalid(_))
    ));
    assert!(admission::guarded_header(1).is_ok());
}

fn bad_header_segments(disk: &Disk, supported: u64) -> Vec<String> {
    crate::scrub::scrub_with_capabilities(disk, "wal", supported)
        .unwrap()
        .issues
        .into_iter()
        .filter_map(|issue| match issue {
            crate::scrub::Issue::BadHeader { segment, .. } => Some(segment),
            _ => None,
        })
        .collect()
}

#[test]
fn scrub_flags_a_header_that_needs_more_than_the_caller_supports() {
    let mut disk = Disk::new();
    plant(
        &mut disk,
        "wal/00000000000000000001.seg",
        &admission::guarded_header(1).unwrap(),
    );
    assert_eq!(
        bad_header_segments(&disk, 0),
        vec!["00000000000000000001.seg".to_string()],
        "a requirement outside the supported set is a finding"
    );
    assert!(
        bad_header_segments(&disk, 1).is_empty(),
        "the same header is fine once its requirement is supported"
    );
}

#[test]
fn scrub_flags_a_later_header_that_drops_a_requirement() {
    let mut disk = Disk::new();
    plant(
        &mut disk,
        "wal/00000000000000000001.seg",
        &admission::guarded_header(1).unwrap(),
    );
    plant(
        &mut disk,
        "wal/00000000000000000002.seg",
        &record::segment_header(),
    );
    assert_eq!(
        bad_header_segments(&disk, 1),
        vec!["00000000000000000002.seg".to_string()],
        "a downgraded header is a finding even when every requirement it keeps is supported"
    );
}

#[path = "admission_matrix_tests.rs"]
mod matrix;

#[path = "admission_rewrite_tests.rs"]
mod guard_rewrite;

#[derive(Clone)]
struct ChangingView {
    disk: Disk,
    mode: Rc<Cell<u8>>,
}
impl Io for ChangingView {
    fn exists(&self, p: &str) -> bool {
        self.disk.exists(p)
    }
    fn len(&self, p: &str) -> Result<u64, IoError> {
        if self.mode.get() == 2 && p == FIRST {
            Ok(7)
        } else {
            self.disk.len(p)
        }
    }
    fn read(&self, p: &str) -> Result<Vec<u8>, IoError> {
        if p == FIRST {
            match self.mode.get() {
                1 => {
                    let mut bytes = HEADER.to_vec();
                    bytes.extend(record::encode_record(1, 42, &[8; 16]));
                    return Ok(bytes);
                }
                2 => return Ok(COMPOUND.to_vec()),
                _ => {}
            }
        }
        self.disk.read(p)
    }
    fn read_prefix(&self, p: &str, n: usize) -> Result<Vec<u8>, IoError> {
        if p == FIRST && matches!(self.mode.get(), 2 | 3) {
            return Ok(LEGACY[..7.min(n)].to_vec());
        }
        self.disk.read_prefix(p, n)
    }
    fn append(&mut self, p: &str, b: &[u8]) -> Result<(), IoError> {
        self.disk.append(p, b)
    }
    fn truncate(&mut self, p: &str, n: u64) -> Result<(), IoError> {
        self.disk.truncate(p, n)
    }
    fn sync(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync(p)
    }
    fn sync_dir(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync_dir(p)
    }
    fn list(&self, p: &str) -> Result<Vec<String>, IoError> {
        self.disk.list(p)
    }
    fn remove(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.remove(p)
    }
    fn create_dir_all(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.create_dir_all(p)
    }
    fn lock(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.lock(p)
    }
    fn unlock(&mut self, p: &str) {
        self.disk.unlock(p)
    }
}

#[test]
fn changing_views_never_reuse_or_repair_unadmitted_segment_bytes() {
    let mut disk = Disk::new();
    let mut bytes = LEGACY.to_vec();
    bytes.extend(record::encode_record(1, 42, &[7; 32]));
    plant(&mut disk, FIRST, &bytes);
    let mode = Rc::new(Cell::new(0));
    let view = ChangingView {
        disk: disk.clone(),
        mode: mode.clone(),
    };
    let (log, _) = Log::open(view.clone(), "wal", 1000).unwrap();
    // Both views contain a CRC-valid record at LSN1 and have exactly65 bytes; only admission differs.
    mode.set(1);
    assert!(matches!(
        log.read_from_with_context(1, 1),
        Err(LogError::Corrupt(_))
    ));
    mode.set(0);
    drop(log);
    let before = disk.manifest();
    let calls = Cell::new(0);
    for mode_value in [2, 3] {
        mode.set(mode_value);
        assert!(matches!(
            Log::open_streaming(view.clone(), "wal", 1000, |_| calls.set(calls.get() + 1)),
            Err(LogError::Corrupt(_))
        ));
        assert_eq!(calls.get(), 0);
        assert_eq!(
            disk.manifest(),
            before,
            "changed/short seam view must never trigger repair"
        );
    }
    // Unknown version is complete at12 bytes, even though the rest of its header is absent.
    for n in 12..16 {
        let mut disk = Disk::new();
        plant(&mut disk, FIRST, &UNKNOWN_VERSION[..n]);
        let before = disk.manifest();
        assert!(matches!(
            Log::open(disk.clone(), "wal", 1000),
            Err(LogError::Corrupt(_))
        ));
        assert_eq!(disk.manifest(), before);
    }
}

/// A view that breaks exactly one half of the exact prefix-read contract, or pads a full read.
#[derive(Clone)]
struct ContractView {
    disk: Disk,
    /// 1: prefix read one byte short; 2: second `len` call differs; 3: full read has a trailing byte.
    fault: u8,
    len_calls: Rc<Cell<u32>>,
    armed: Rc<Cell<bool>>,
}
impl Io for ContractView {
    fn exists(&self, p: &str) -> bool {
        self.disk.exists(p)
    }
    fn len(&self, p: &str) -> Result<u64, IoError> {
        let n = self.disk.len(p)?;
        if self.fault == 2 && p == FIRST {
            let calls = self.len_calls.get();
            self.len_calls.set(calls + 1);
            if calls == 1 {
                return Ok(n + 1);
            }
        }
        Ok(n)
    }
    fn read(&self, p: &str) -> Result<Vec<u8>, IoError> {
        let mut bytes = self.disk.read(p)?;
        if self.fault == 3 && p == FIRST && self.armed.get() {
            bytes.push(0);
        }
        Ok(bytes)
    }
    fn read_prefix(&self, p: &str, n: usize) -> Result<Vec<u8>, IoError> {
        let mut bytes = self.disk.read_prefix(p, n)?;
        if self.fault == 1 && p == FIRST {
            bytes.pop();
        }
        Ok(bytes)
    }
    fn append(&mut self, p: &str, b: &[u8]) -> Result<(), IoError> {
        self.disk.append(p, b)
    }
    fn truncate(&mut self, p: &str, n: u64) -> Result<(), IoError> {
        self.disk.truncate(p, n)
    }
    fn sync(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync(p)
    }
    fn sync_dir(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync_dir(p)
    }
    fn list(&self, p: &str) -> Result<Vec<String>, IoError> {
        self.disk.list(p)
    }
    fn remove(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.remove(p)
    }
    fn create_dir_all(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.create_dir_all(p)
    }
    fn lock(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.lock(p)
    }
    fn unlock(&mut self, p: &str) {
        self.disk.unlock(p)
    }
}

fn contract_view(fault: u8) -> (ContractView, Disk) {
    let mut disk = Disk::new();
    let mut bytes = LEGACY.to_vec();
    bytes.extend(record::encode_record(1, 42, &[7; 32]));
    plant(&mut disk, FIRST, &bytes);
    let view = ContractView {
        disk: disk.clone(),
        fault,
        len_calls: Rc::new(Cell::new(0)),
        armed: Rc::new(Cell::new(false)),
    };
    (view, disk)
}

#[test]
fn each_half_of_the_prefix_read_contract_is_enforced_on_its_own() {
    for fault in [1, 2] {
        let (view, disk) = contract_view(fault);
        let before = disk.manifest();
        assert!(
            matches!(Log::open(view, "wal", 1000), Err(LogError::Corrupt(_))),
            "fault {fault} breaks one half of the contract and must refuse"
        );
        assert_eq!(disk.manifest(), before, "refusal must not repair");
    }
}

#[test]
fn a_full_read_longer_than_the_admitted_length_is_refused_even_with_the_same_prefix() {
    let (view, _disk) = contract_view(3);
    let (log, _) = Log::open(view.clone(), "wal", 1000).unwrap();
    view.armed.set(true);
    assert!(matches!(
        log.read_from_with_context(1, 1),
        Err(LogError::Corrupt(_))
    ));
}

// RFC 0012: required capability bit 3 (value 8, SQL_DEFINITIONS). Nothing requests it yet.
const SQL_DEFINITIONS_HEADER: &[u8] =
    include_bytes!("../../../formats/corpus/wal-admission-v2-sql-definitions.golden");
const MANAGED_SQL_HEADER: &[u8] =
    include_bytes!("../../../formats/corpus/wal-admission-v2-managed-sql-definitions.golden");
const UNASSIGNED_CAP: &[u8] =
    include_bytes!("../../../formats/corpus/wal-admission-v2-unassigned-capability.golden");

fn open_supporting(disk: &Disk, limit: u64, supported: u64) -> Log<Disk> {
    Log::open_streaming_with_capabilities(disk.clone(), "wal", limit, supported, |_, _| {})
        .unwrap()
        .0
}

#[test]
fn bit_3_headers_match_hand_derived_bytes_and_round_trip() {
    // Prefix bytes written out by hand from the layout: magic, version 2, length 32, reserved 0,
    // mask as u64 little endian, reserved 0. The CRC bytes come from the Python generator.
    let prefix = |mask: u8| {
        let mut p = b"LATCAP02".to_vec();
        p.extend_from_slice(&[
            2, 0, 0, 0, 32, 0, 0, 0, mask, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        p
    };
    assert_eq!(&SQL_DEFINITIONS_HEADER[..28], prefix(8).as_slice());
    assert_eq!(&MANAGED_SQL_HEADER[..28], prefix(9).as_slice());
    assert_eq!(SQL_DEFINITIONS_HEADER[28..], [0x32, 0x7b, 0x71, 0x22]);
    assert_eq!(MANAGED_SQL_HEADER[28..], [0x02, 0xaf, 0x00, 0x13]);
    assert_eq!(admission::SQL_DEFINITIONS, 8);
    assert_eq!(admission::KNOWN_CAPABILITIES, 1 | 8);
    for (mask, fixture) in [(8u64, SQL_DEFINITIONS_HEADER), (9, MANAGED_SQL_HEADER)] {
        assert_eq!(admission::guarded_header(mask).unwrap(), fixture);
        let parsed = admission::decode_header(fixture).unwrap();
        assert_eq!((parsed.length, parsed.required_capabilities), (32, mask));
        for n in 0..32 {
            assert!(admission::decode_header(&fixture[..n]).is_err());
            let mut flipped = fixture.to_vec();
            flipped[n] ^= 1;
            assert!(admission::decode_header(&flipped).is_err());
        }
    }
}

#[test]
fn bit_3_neighbours_stay_refused_by_encoder_and_decoder() {
    // Value 2 is unknown and value 4 is unassigned, alone or beside a known bit.
    for mask in [2u64, 4, 3, 5, 10, 12, 13, 15, 16, 1 << 63] {
        assert!(
            matches!(
                admission::guarded_header(mask),
                Err(admission::HeaderError::Invalid(_))
            ),
            "mask {mask}"
        );
    }
    for bytes in [UNKNOWN_CAP, UNASSIGNED_CAP] {
        assert!(admission::decode_header(bytes).is_err());
    }
    assert_eq!(UNASSIGNED_CAP[16], 4);
}

#[test]
fn a_reader_without_bit_3_refuses_before_any_callback_read_or_repair() {
    for (fixture, mask) in [(SQL_DEFINITIONS_HEADER, 8u64), (MANAGED_SQL_HEADER, 9)] {
        let mut disk = Disk::new();
        plant(&mut disk, FIRST, fixture);
        let before = disk.manifest();
        disk.1.set(0);
        // Legacy-only and managed-compound-only readers both refuse.
        for supported in [0u64, 1] {
            let calls = Cell::new(0);
            let result = Log::open_streaming_with_capabilities(
                disk.clone(),
                "wal",
                100,
                supported,
                |_, _| calls.set(calls.get() + 1),
            );
            assert!(
                matches!(
                    result,
                    Err(LogError::UnsupportedCapabilities { required, supported: s })
                        if required == mask && s == supported
                ),
                "mask {mask} supported {supported}"
            );
            assert_eq!(calls.get(), 0);
        }
        assert_eq!(disk.1.get(), 0, "preflight reads bounded prefixes only");
        assert_eq!(disk.manifest(), before);
        // A reader that lists exactly the bits in the header opens it.
        let log = open_supporting(&disk, 100, mask);
        assert_eq!(log.required_capabilities(), mask);
    }
    // Claiming support for an unassigned bit does not make the build understand it.
    let mut disk = Disk::new();
    plant(&mut disk, FIRST, UNASSIGNED_CAP);
    assert!(
        Log::open_streaming_with_capabilities(disk, "wal", 100, 4 | 9, |_, _| panic!(
            "callback before admission"
        ))
        .is_err()
    );
}

#[test]
fn a_later_segment_with_bit_3_refuses_a_reader_without_it_and_keeps_bytes() {
    let mut disk = Disk::new();
    let mut first = LEGACY.to_vec();
    first.extend(record::encode_record(1, 42, b"legacy"));
    plant(&mut disk, FIRST, &first);
    plant(
        &mut disk,
        "wal/00000000000000000002.seg",
        SQL_DEFINITIONS_HEADER,
    );
    let before = disk.manifest();
    assert!(
        Log::open_streaming_with_capabilities(disk.clone(), "wal", 100, 1, |_, _| panic!(
            "callback before admission"
        ))
        .is_err()
    );
    assert_eq!(disk.manifest(), before);
}

#[test]
fn bit_3_is_requested_only_by_a_writer_that_asks_and_survives_rollover() {
    let disk = Disk::new();
    // A build that supports bit 3 but never requests it writes no bit 3.
    let mut log = open_supporting(&disk, 1, 9);
    log.append_with_capabilities(42, b"managed", 1).unwrap();
    log.sync().unwrap();
    log.append(42, b"ordinary").unwrap();
    log.sync().unwrap();
    drop(log);
    for (_, bytes) in disk.manifest() {
        assert_eq!(
            admission::decode_header(&bytes)
                .unwrap()
                .required_capabilities,
            1
        );
    }
    // A writer that does not list bit 3 cannot append it, and consumes no LSN trying.
    let mut narrow = open_supporting(&disk, 1, 1);
    let next = narrow.next_lsn();
    assert!(matches!(
        narrow.append_with_capabilities(42, b"x", 8),
        Err(LogError::UnsupportedCapabilities {
            required: 8,
            supported: 1
        })
    ));
    assert_eq!(narrow.next_lsn(), next);
    drop(narrow);
    // The first append that needs the bit adds it to the cumulative mask; every segment that
    // follows keeps it, including ones opened after a restart.
    let mut log = open_supporting(&disk, 1, 9);
    let lsn = log.next_lsn();
    log.append_with_capabilities(42, b"definition", 8).unwrap();
    log.sync().unwrap();
    assert_eq!(log.required_capabilities(), 9);
    log.append(42, b"after").unwrap();
    log.sync().unwrap();
    drop(log);
    let mut log = open_supporting(&disk, 1, 9);
    log.append(42, b"after restart").unwrap();
    log.sync().unwrap();
    drop(log);
    let masks: Vec<u64> = disk
        .manifest()
        .iter()
        .map(|(_, b)| admission::decode_header(b).unwrap().required_capabilities)
        .collect();
    let first_with_bit = masks.iter().position(|m| m & 8 != 0).unwrap();
    assert!(first_with_bit > 0, "earlier segments keep their older mask");
    assert!(masks[first_with_bit..].iter().all(|m| *m == 9), "{masks:?}");
    assert!(masks[..first_with_bit].iter().all(|m| *m == 1), "{masks:?}");
    let mut seen = Vec::new();
    Log::open_streaming_with_capabilities(disk.clone(), "wal", 1, 9, |r, c| {
        seen.push((r.lsn, c.required_capabilities))
    })
    .unwrap();
    assert_eq!(seen.iter().find(|(l, _)| *l == lsn).unwrap().1, 9);
    assert!(seen.iter().filter(|(l, _)| *l > lsn).all(|(_, c)| *c == 9));
    // A reader without bit 3 now refuses the directory.
    assert!(
        Log::open_streaming_with_capabilities(disk, "wal", 1, 1, |_, _| panic!("callback"))
            .is_err()
    );
}

#[cfg(feature = "planted-bugs")]
#[test]
fn independent_dropped_bit_3_witness_has_a_clean_control() {
    use crate::bugs::{self, Bug};
    for required in [8u64, 9] {
        for mutate in [false, true] {
            let disk = Disk::new();
            let mut log = open_supporting(&disk, 1 << 20, 9);
            if mutate {
                bugs::enable(Bug::SqlDefinitionsBitDropped);
            }
            log.append_with_capabilities(42, b"needs-definitions", required)
                .unwrap();
            log.sync().unwrap();
            bugs::clear();
            // Literal oracle: the context the record replays under carries the exact mask.
            let rows = log.read_from_with_context(1, 1).unwrap();
            let violated = rows[0].1.required_capabilities != required;
            assert_eq!(violated, mutate, "required {required}");
        }
    }
    // A writer that never asks for bit 3 is unaffected by the defect.
    let disk = Disk::new();
    let mut log = open_supporting(&disk, 1 << 20, 9);
    bugs::enable(Bug::SqlDefinitionsBitDropped);
    log.append_with_capabilities(42, b"managed-only", 1)
        .unwrap();
    log.sync().unwrap();
    bugs::clear();
    assert_eq!(
        log.read_from_with_context(1, 1).unwrap()[0]
            .1
            .required_capabilities,
        1
    );
}
