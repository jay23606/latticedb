//! CRC32C (Castagnoli), table driven, no dependencies.

// Const context cannot use `get_mut`; the loop bound `i < 256` makes the index in range.
#[allow(clippy::indexing_slicing)]
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0x82F6_3B78
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static TABLE: [u32; 256] = make_table();

/// Incremental CRC32C.
#[derive(Debug, Clone, Copy)]
pub struct Crc32c(u32);

impl Default for Crc32c {
    fn default() -> Self {
        Self(0xFFFF_FFFF)
    }
}

impl Crc32c {
    /// Feeds bytes.
    pub fn update(&mut self, bytes: &[u8]) {
        for b in bytes {
            let idx = ((self.0 ^ u32::from(*b)) & 0xFF) as usize;
            self.0 = (self.0 >> 8) ^ TABLE.get(idx).copied().unwrap_or(0);
        }
    }

    /// The checksum so far.
    pub fn finish(&self) -> u32 {
        !self.0
    }
}

/// CRC32C of one byte slice.
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut c = Crc32c::default();
    c.update(bytes);
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_vectors() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
    }

    proptest! {
        #[test]
        fn incremental_equals_one_shot(a in proptest::collection::vec(any::<u8>(), 0..64), b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let mut c = Crc32c::default();
            c.update(&a);
            c.update(&b);
            let mut all = a.clone();
            all.extend_from_slice(&b);
            prop_assert_eq!(c.finish(), crc32c(&all));
        }

        #[test]
        fn any_single_bit_flip_changes_the_checksum(data in proptest::collection::vec(any::<u8>(), 1..64), pos in any::<usize>(), bit in 0u8..8) {
            let mut flipped = data.clone();
            let i = pos % data.len();
            if let Some(b) = flipped.get_mut(i) { *b ^= 1 << bit; }
            prop_assert_ne!(crc32c(&data), crc32c(&flipped));
        }
    }
}
