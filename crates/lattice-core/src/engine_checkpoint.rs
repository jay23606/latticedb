//! Draft engine checkpoints (RFC 0002). WAL prefix bytes remain available to full scrub.
//! A sealed snapshot is authoritative for the skipped prefix; opening it does not audit that prefix.
use crate::log::{LogError, first_lsn_of};
use crate::record::{Decoded, HeaderCheck, SEGMENT_HEADER_LEN, check_header, decode_record};
use lattice_seams::{Io, IoError};

/// Maximum transient snapshot chunk size.
pub const CHUNK_BYTES: usize = 256 * 1024;
/// Maximum chunks in one snapshot (one GiB), rejected rather than truncated.
pub const MAX_CHUNKS: usize = 4096;
const MANIFEST_LIMIT: u64 = 256 * 1024;
/// Opaque WAL type; older readers ignore it and replay the retained WAL.
pub const SEAL_TYPE: u8 = 4;
const MAGIC: &[u8; 8] = b"LATCP001";

pub(crate) fn bad(why: &str) -> LogError {
    LogError::Corrupt(format!("engine checkpoint: {why}"))
}

/// Streaming state encoder, with bounded byte fields.
pub trait StateWrite {
    /// Writes bytes, splitting across chunks as necessary.
    fn raw(&mut self, bytes: &[u8]) -> Result<(), LogError>;
    /// Writes a little-endian integer.
    fn u64(&mut self, n: u64) -> Result<(), LogError> {
        self.raw(&n.to_le_bytes())
    }
    /// Writes a length-prefixed field, at most the WAL payload limit.
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), LogError> {
        if bytes.len() > crate::record::MAX_PAYLOAD {
            return Err(bad("field too large"));
        }
        self.u64(bytes.len() as u64)?;
        self.raw(bytes)
    }
}
/// Streaming state decoder. Counts must be checked against `remaining` before allocation.
pub trait StateRead {
    /// Bytes remaining across every chunk.
    fn remaining(&self) -> u64;
    /// Reads exactly the supplied length.
    fn raw(&mut self, bytes: &mut [u8]) -> Result<(), LogError>;
    /// Reads a little-endian integer.
    fn u64(&mut self) -> Result<u64, LogError> {
        let mut a = [0; 8];
        self.raw(&mut a)?;
        Ok(u64::from_le_bytes(a))
    }
    /// Reads a length-prefixed bounded byte field.
    fn bytes(&mut self) -> Result<Vec<u8>, LogError> {
        let n = self.u64()?;
        if n > crate::record::MAX_PAYLOAD as u64 || n > self.remaining() {
            return Err(bad("field length"));
        }
        let mut out = vec![0; n as usize];
        self.raw(&mut out)?;
        Ok(out)
    }
    /// Reads a count bounded by available bytes and a minimum item size.
    fn count(&mut self, minimum: u64) -> Result<u64, LogError> {
        let n = self.u64()?;
        if minimum == 0 || n > self.remaining() / minimum {
            return Err(bad("item count"));
        }
        Ok(n)
    }
}

#[derive(Clone)]
struct Chunk {
    len: u64,
    hash: [u8; 32],
}
#[derive(Clone)]
pub(crate) struct Manifest {
    pub through: u64,
    tag: Vec<u8>,
    prefix_count: u64,
    prefix_bytes: u64,
    prefix_hash: [u8; 32],
    total: u64,
    chunks: Vec<Chunk>,
}

struct Slice<'a>(&'a [u8]);
impl StateRead for Slice<'_> {
    fn remaining(&self) -> u64 {
        self.0.len() as u64
    }
    fn raw(&mut self, out: &mut [u8]) -> Result<(), LogError> {
        let (a, b) = self
            .0
            .split_at_checked(out.len())
            .ok_or_else(|| bad("short manifest"))?;
        out.copy_from_slice(a);
        self.0 = b;
        Ok(())
    }
}
impl StateWrite for Vec<u8> {
    fn raw(&mut self, b: &[u8]) -> Result<(), LogError> {
        self.extend_from_slice(b);
        Ok(())
    }
}
impl Manifest {
    fn encode(&self) -> Result<Vec<u8>, LogError> {
        let mut b = Vec::new();
        b.raw(MAGIC)?;
        b.bytes(&self.tag)?;
        b.u64(self.through)?;
        b.u64(self.prefix_count)?;
        b.u64(self.prefix_bytes)?;
        b.raw(&self.prefix_hash)?;
        b.u64(self.total)?;
        b.u64(self.chunks.len() as u64)?;
        for c in &self.chunks {
            b.u64(c.len)?;
            b.raw(&c.hash)?;
        }
        Ok(b)
    }
    fn decode(b: &[u8]) -> Result<Self, LogError> {
        let mut r = Slice(b);
        let mut magic = [0; 8];
        r.raw(&mut magic)?;
        if &magic != MAGIC {
            return Err(bad("manifest version"));
        }
        let tag = r.bytes()?;
        if tag.is_empty() || tag.len() > 64 {
            return Err(bad("engine tag"));
        }
        let through = r.u64()?;
        through.checked_add(1).ok_or_else(|| bad("LSN overflow"))?;
        let prefix_count = r.u64()?;
        let prefix_bytes = r.u64()?;
        let mut prefix_hash = [0; 32];
        r.raw(&mut prefix_hash)?;
        let total = r.u64()?;
        let n = r.count(40)?;
        if n == 0 || n > MAX_CHUNKS as u64 {
            return Err(bad("chunk count"));
        }
        let mut chunks = Vec::new();
        let mut sum = 0u64;
        for i in 0..n {
            let len = r.u64()?;
            if len == 0 || len > CHUNK_BYTES as u64 || (i + 1 < n && len != CHUNK_BYTES as u64) {
                return Err(bad("chunk length"));
            }
            sum = sum.checked_add(len).ok_or_else(|| bad("total overflow"))?;
            let mut hash = [0; 32];
            r.raw(&mut hash)?;
            chunks.push(Chunk { len, hash });
        }
        if r.remaining() != 0 || sum != total {
            return Err(bad("manifest total/trailing bytes"));
        }
        Ok(Self {
            through,
            tag,
            prefix_count,
            prefix_bytes,
            prefix_hash,
            total,
            chunks,
        })
    }
}

/// Snapshot recovery evidence. Prefix bytes are skipped, not verified against the WAL contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointRecovery {
    /// Snapshot used through this LSN, or zero for full replay.
    pub through_lsn: u64,
    /// WAL prefix bytes avoided by snapshot recovery.
    pub skipped_wal_bytes: u64,
    /// Bytes of retained state read, one chunk at a time.
    pub state_bytes: u64,
}

fn checkpoint_dir(dir: &str) -> String {
    format!("{dir}/.checkpoint")
}
fn chunk_path(base: &str, n: usize) -> String {
    format!("{base}.{n:04}.chunk")
}
// Publication can leave chunks before a manifest. A syntactically valid generation with seal >1
// proves a nonempty source WAL was durably established before snapshot publication began. Preserve
// that evidence even if a cache is damaged; full recovery must not initialize an empty replacement.
pub(crate) fn source_lsn_evidence<I: Io>(io: &I, dir: &str) -> Result<u64, LogError> {
    let mut through = 0;
    let result = io.visit_dir(&checkpoint_dir(dir), &mut |name| {
        let base = name.strip_suffix(".manifest").or_else(|| {
            let stem = name.strip_suffix(".chunk")?;
            let (base, index) = stem.rsplit_once('.')?;
            (index.len() == 4 && index.bytes().all(|b| b.is_ascii_digit())).then_some(base)
        });
        if let Some(base) = base
            && base.len() == 53
            && base.get(20..21) == Some("-")
            && base.get(21..).is_some_and(|s| {
                s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            && let Some(lsn) = base
                .get(..20)
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok())
        {
            through = through.max(lsn.saturating_sub(1));
        }
    });
    match result {
        Ok(()) | Err(IoError::NotFound(_)) => Ok(through),
        Err(e) => Err(e.into()),
    }
}

// Order-independent digest: names are unique filesystem entries. Count and total length bind the
// inventory as well. The sealed manifest authenticates this digest, not skipped WAL byte contents.
fn inventory<I: Io>(io: &I, dir: &str, before: u64) -> Result<(u64, u64, [u8; 32]), LogError> {
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut hash = [0; 32];
    let mut failure = None;
    io.visit_dir(dir, &mut |name| {
        if failure.is_some() || !first_lsn_of(name).is_some_and(|l| l < before) {
            return;
        }
        let result = (|| {
            let len = io.len(&format!("{dir}/{name}"))?;
            count = count.checked_add(1).ok_or_else(|| bad("inventory count"))?;
            bytes = bytes
                .checked_add(len)
                .ok_or_else(|| bad("inventory bytes"))?;
            let mut h = blake3::Hasher::new();
            h.update(name.as_bytes());
            h.update(&len.to_le_bytes());
            for (a, b) in hash.iter_mut().zip(h.finalize().as_bytes()) {
                *a ^= *b;
            }
            Ok::<(), LogError>(())
        })();
        if let Err(e) = result {
            failure = Some(e);
        }
    })?;
    if let Some(e) = failure {
        return Err(e);
    }
    Ok((count, bytes, hash))
}

pub(crate) struct Writer<'a, I: Io> {
    io: &'a mut I,
    base: String,
    buffer: Vec<u8>,
    chunks: Vec<Chunk>,
    total: u64,
}
impl<'a, I: Io> Writer<'a, I> {
    pub(crate) fn new(
        io: &'a mut I,
        dir: &str,
        seal: u64,
        id: &[u8; 16],
    ) -> Result<Self, LogError> {
        let d = checkpoint_dir(dir);
        io.create_dir_all(&d)?;
        io.sync_dir(dir)?;
        let id = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let base = format!("{d}/{seal:020}-{id}");
        if io.exists(&format!("{base}.manifest")) || io.exists(&chunk_path(&base, 0)) {
            return Err(bad("generation collision"));
        }
        Ok(Self {
            io,
            base,
            buffer: Vec::with_capacity(CHUNK_BYTES),
            chunks: Vec::new(),
            total: 0,
        })
    }
    fn flush(&mut self) -> Result<(), LogError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        if self.chunks.len() >= MAX_CHUNKS {
            return Err(bad("snapshot exceeds one GiB"));
        }
        let path = chunk_path(&self.base, self.chunks.len());
        if self.io.exists(&path) {
            return Err(bad("chunk already exists"));
        }
        self.io.append(&path, &self.buffer)?;
        self.io.sync(&path)?;
        self.chunks.push(Chunk {
            len: self.buffer.len() as u64,
            hash: *blake3::hash(&self.buffer).as_bytes(),
        });
        self.buffer.clear();
        Ok(())
    }
    pub(crate) fn finish(
        mut self,
        dir: &str,
        through: u64,
        tag: &[u8],
    ) -> Result<Vec<u8>, LogError> {
        self.flush()?;
        let (prefix_count, prefix_bytes, prefix_hash) = inventory(self.io, dir, through + 1)?;
        let m = Manifest {
            through,
            tag: tag.to_vec(),
            prefix_count,
            prefix_bytes,
            prefix_hash,
            total: self.total,
            chunks: self.chunks,
        };
        let bytes = m.encode()?;
        let path = format!("{}.manifest", self.base);
        self.io.sync_dir(&checkpoint_dir(dir))?;
        self.io.append(&path, &bytes)?;
        self.io.sync(&path)?;
        self.io.sync_dir(&checkpoint_dir(dir))?;
        // The WAL seal binds this exact immutable manifest; the manifest basename is derived from
        // the seal LSN and generation id, so the seal stays bounded even at MAX_CHUNKS.
        let name = self
            .base
            .rsplit('/')
            .next()
            .ok_or_else(|| bad("generation name"))?;
        let mut seal = Vec::new();
        seal.raw(MAGIC)?;
        seal.bytes(name.as_bytes())?;
        seal.raw(blake3::hash(&bytes).as_bytes())?;
        Ok(seal)
    }
}
impl<I: Io> StateWrite for Writer<'_, I> {
    fn raw(&mut self, mut b: &[u8]) -> Result<(), LogError> {
        if self
            .total
            .checked_add(b.len() as u64)
            .is_none_or(|n| n > (CHUNK_BYTES * MAX_CHUNKS) as u64)
        {
            return Err(bad("snapshot size"));
        }
        self.total += b.len() as u64;
        while !b.is_empty() {
            let n = b.len().min(CHUNK_BYTES - self.buffer.len());
            self.buffer
                .extend_from_slice(b.get(..n).ok_or_else(|| bad("write split"))?);
            b = b.get(n..).ok_or_else(|| bad("write split"))?;
            if self.buffer.len() == CHUNK_BYTES {
                self.flush()?;
            }
        }
        Ok(())
    }
}

pub(crate) struct Reader<'a, I: Io> {
    io: &'a I,
    base: String,
    manifest: Manifest,
    index: usize,
    buffer: Vec<u8>,
    offset: usize,
    remaining: u64,
}
impl<I: Io> Reader<'_, I> {
    pub(crate) fn through(&self) -> u64 {
        self.manifest.through
    }
    pub(crate) fn evidence(&self) -> CheckpointRecovery {
        CheckpointRecovery {
            through_lsn: self.manifest.through,
            skipped_wal_bytes: self.manifest.prefix_bytes,
            state_bytes: self.manifest.total,
        }
    }
}
impl<I: Io> StateRead for Reader<'_, I> {
    fn remaining(&self) -> u64 {
        self.remaining
    }
    fn raw(&mut self, mut out: &mut [u8]) -> Result<(), LogError> {
        if out.len() as u64 > self.remaining {
            return Err(bad("short state"));
        }
        self.remaining -= out.len() as u64;
        while !out.is_empty() {
            if self.offset == self.buffer.len() {
                let c = self
                    .manifest
                    .chunks
                    .get(self.index)
                    .ok_or_else(|| bad("missing chunk"))?;
                let path = chunk_path(&self.base, self.index);
                if self.io.len(&path)? != c.len {
                    return Err(bad("chunk size changed"));
                }
                // Release the previous allocation before the next whole-file read.
                self.buffer = Vec::new();
                self.buffer = self.io.read(&path)?;
                if self.buffer.len() as u64 != c.len
                    || blake3::hash(&self.buffer).as_bytes() != &c.hash
                {
                    return Err(bad("chunk checksum"));
                }
                self.index += 1;
                self.offset = 0;
            }
            let n = out.len().min(self.buffer.len() - self.offset);
            let src = self
                .buffer
                .get(self.offset..self.offset + n)
                .ok_or_else(|| bad("chunk bounds"))?;
            let (a, b) = out.split_at_mut(n);
            a.copy_from_slice(src);
            out = b;
            self.offset += n;
        }
        Ok(())
    }
}

/// Attempts the newest generation. An incomplete/invalid cache falls back to full replay; actual
/// device errors propagate. Call only while holding the WAL writer lock.
pub(crate) fn load<'a, I: Io>(
    io: &'a I,
    dir: &str,
    tag: &[u8],
) -> Result<Option<Reader<'a, I>>, LogError> {
    match try_load(io, dir, tag) {
        Err(LogError::Corrupt(_)) | Err(LogError::Io(IoError::NotFound(_))) => Ok(None),
        other => other,
    }
}
fn try_load<'a, I: Io>(
    io: &'a I,
    dir: &str,
    tag: &[u8],
) -> Result<Option<Reader<'a, I>>, LogError> {
    let d = checkpoint_dir(dir);
    let mut newest: Option<String> = None;
    io.visit_dir(&d, &mut |name| {
        // Strict fixed grammar prevents paths escaping the checkpoint directory.
        if name.len() == 62
            && name.ends_with(".manifest")
            && name.get(20..21) == Some("-")
            && name
                .get(..20)
                .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
            && name.get(21..53).is_some_and(|s| {
                s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            && newest.as_ref().is_none_or(|n| name > n.as_str())
        {
            newest = Some(name.into());
        }
    })?;
    let Some(name) = newest else {
        return Ok(None);
    };
    let path = format!("{d}/{name}");
    if io.len(&path)? > MANIFEST_LIMIT {
        return Err(bad("manifest length"));
    }
    let bytes = io.read(&path)?;
    if bytes.len() as u64 > MANIFEST_LIMIT {
        return Err(bad("manifest grew"));
    }
    let m = Manifest::decode(&bytes)?;
    if m.tag != tag {
        return Ok(None);
    }
    let seal = m
        .through
        .checked_add(1)
        .ok_or_else(|| bad("seal overflow"))?;
    if name.get(..20) != Some(format!("{seal:020}").as_str()) {
        return Err(bad("generation LSN"));
    }
    let base = format!(
        "{d}/{}",
        name.strip_suffix(".manifest")
            .ok_or_else(|| bad("suffix"))?
    );
    if inventory(io, dir, seal)? != (m.prefix_count, m.prefix_bytes, m.prefix_hash) {
        return Err(bad("WAL prefix inventory changed"));
    }
    // Anchor segment reads are bounded by the normal WAL segment policy. The seal must be first.
    let anchor = io.read(&format!("{dir}/{seal:020}.seg"))?;
    if check_header(&anchor) != HeaderCheck::Ok {
        return Err(bad("seal segment header"));
    }
    let Decoded::Record(record, _) = decode_record(&anchor, SEGMENT_HEADER_LEN) else {
        return Err(bad("missing seal"));
    };
    let mut expected = Vec::new();
    expected.raw(MAGIC)?;
    expected.bytes(
        base.rsplit('/')
            .next()
            .ok_or_else(|| bad("basename"))?
            .as_bytes(),
    )?;
    expected.raw(blake3::hash(&bytes).as_bytes())?;
    if record.lsn != seal || record.rtype != SEAL_TYPE || record.payload != expected {
        return Err(bad("seal mismatch"));
    }
    drop(anchor);
    // Every chunk is validated before state is exposed; decoding validates again while consuming.
    for (i, c) in m.chunks.iter().enumerate() {
        let p = chunk_path(&base, i);
        if io.len(&p)? != c.len {
            return Err(bad("chunk length"));
        }
        let b = io.read(&p)?;
        if b.len() as u64 != c.len || blake3::hash(&b).as_bytes() != &c.hash {
            return Err(bad("chunk checksum"));
        }
    }
    let remaining = m.total;
    Ok(Some(Reader {
        io,
        base,
        manifest: m,
        index: 0,
        buffer: Vec::new(),
        offset: 0,
        remaining,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use proptest::prelude::*;
    fn example() -> Manifest {
        Manifest {
            through: 7,
            tag: b"test-v1".to_vec(),
            prefix_count: 2,
            prefix_bytes: 123,
            prefix_hash: [9; 32],
            total: 3,
            chunks: vec![Chunk {
                len: 3,
                hash: [8; 32],
            }],
        }
    }
    #[test]
    fn manifest_roundtrips_and_every_truncation_is_rejected() {
        let bytes = example().encode().unwrap();
        assert_eq!(Manifest::decode(&bytes).unwrap().encode().unwrap(), bytes);
        for n in 0..bytes.len() {
            assert!(Manifest::decode(&bytes[..n]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(Manifest::decode(&trailing).is_err());
    }
    #[test]
    fn impossible_chunk_counts_lengths_and_totals_are_refused_before_allocation() {
        let mut m = example();
        m.total = u64::MAX;
        assert!(Manifest::decode(&m.encode().unwrap()).is_err());
        m = example();
        m.chunks[0].len = CHUNK_BYTES as u64 + 1;
        assert!(Manifest::decode(&m.encode().unwrap()).is_err());
        m = example();
        m.chunks[0].len = 0;
        assert!(Manifest::decode(&m.encode().unwrap()).is_err());
        m = example();
        m.through = u64::MAX;
        assert!(Manifest::decode(&m.encode().unwrap()).is_err());
        m = example();
        m.chunks = vec![
            Chunk {
                len: 1,
                hash: [0; 32]
            };
            MAX_CHUNKS + 1
        ];
        assert!(Manifest::decode(&m.encode().unwrap()).is_err());
    }
    proptest! {
        #[test]
        fn arbitrary_manifests_never_panic_and_successes_are_canonical(bytes in prop::collection::vec(any::<u8>(),0..2000)) {
            if let Ok(m)=Manifest::decode(&bytes) { prop_assert_eq!(m.encode().unwrap(),bytes); }
        }
    }
}

#[cfg(test)]
mod independent_golden {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::record::{encode_record, segment_header};
    use lattice_sim::{FaultProfile, SimIo};
    struct Fixture {
        prefix: &'static [u8],
        manifest: &'static [u8],
        seal: &'static [u8],
        chunks: Vec<&'static [u8]>,
        through: u64,
    }
    macro_rules! fixture {
        ($case:literal, $through:expr, $($chunk:literal),+) => { Fixture {
            prefix: include_bytes!(concat!("../../../formats/corpus/checkpoint-v1/",$case,"/prefix.golden")),
            manifest: include_bytes!(concat!("../../../formats/corpus/checkpoint-v1/",$case,"/manifest.golden")),
            seal: include_bytes!(concat!("../../../formats/corpus/checkpoint-v1/",$case,"/seal-segment.golden")),
            chunks: vec![$(include_bytes!(concat!("../../../formats/corpus/checkpoint-v1/",$case,"/chunk-",$chunk,".golden")).as_slice()),+], through: $through,
        } };
    }
    #[test]
    fn python_fixtures_decode_and_production_writer_reproduces_every_byte() {
        for f in [
            fixture!("empty", 0, "0000"),
            fixture!("row", 4, "0000"),
            fixture!("catalog", 4, "0000"),
            fixture!("split", 0, "0000", "0001"),
        ] {
            let m = Manifest::decode(f.manifest).unwrap();
            assert_eq!(m.encode().unwrap(), f.manifest);
            let mut io = SimIo::new(1, FaultProfile::clean());
            io.create_dir_all("wal").unwrap();
            if !f.prefix.is_empty() {
                io.append("wal/00000000000000000001.seg", f.prefix).unwrap();
            }
            let anchor = format!("wal/{:020}.seg", f.through + 1);
            io.append(&anchor, &segment_header()).unwrap();
            let id = [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef,
            ];
            let mut writer = Writer::new(&mut io, "wal", f.through + 1, &id).unwrap();
            for chunk in &f.chunks {
                writer.raw(chunk).unwrap();
            }
            let payload = writer.finish("wal", f.through, &m.tag).unwrap();
            io.append(&anchor, &encode_record(f.through + 1, SEAL_TYPE, &payload))
                .unwrap();
            assert_eq!(io.read(&anchor).unwrap(), f.seal);
            let base = format!(
                "wal/.checkpoint/{:020}-0123456789abcdef0123456789abcdef",
                f.through + 1
            );
            assert_eq!(io.read(&format!("{base}.manifest")).unwrap(), f.manifest);
            for (i, chunk) in f.chunks.iter().enumerate() {
                assert_eq!(io.read(&chunk_path(&base, i)).unwrap(), *chunk);
            }
            let mut reader = load(&io, "wal", &m.tag).unwrap().unwrap();
            let mut actual = vec![0; reader.remaining() as usize];
            reader.raw(&mut actual).unwrap();
            assert_eq!(actual, f.chunks.concat());
            assert_eq!(reader.remaining(), 0);
        }
    }
}
