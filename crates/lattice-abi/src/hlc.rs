//! Hybrid logical clock timestamps.

/// A hybrid logical clock timestamp. Total order is `(wall_ms, counter, node)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hlc {
    /// Physical component, milliseconds.
    pub wall_ms: u64,
    /// Logical counter that breaks ties within one millisecond.
    pub counter: u32,
    /// Node id, final tiebreak.
    pub node: u32,
}

impl Hlc {
    /// Encoded size in bytes.
    pub const ENCODED_LEN: usize = 16;

    /// Encodes as little-endian `wall_ms`, `counter`, `node`.
    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0u8; Self::ENCODED_LEN];
        out[..8].copy_from_slice(&self.wall_ms.to_le_bytes());
        out[8..12].copy_from_slice(&self.counter.to_le_bytes());
        out[12..].copy_from_slice(&self.node.to_le_bytes());
        out
    }

    /// Decodes the form produced by [`Hlc::to_bytes`].
    pub fn from_bytes(b: &[u8; Self::ENCODED_LEN]) -> Self {
        let (w, rest) = b.split_at(8);
        let (c, n) = rest.split_at(4);
        Self {
            wall_ms: u64::from_le_bytes(w.try_into().unwrap_or([0; 8])),
            counter: u32::from_le_bytes(c.try_into().unwrap_or([0; 4])),
            node: u32::from_le_bytes(n.try_into().unwrap_or([0; 4])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn encoding_round_trips(w in any::<u64>(), c in any::<u32>(), n in any::<u32>()) {
            let h = Hlc { wall_ms: w, counter: c, node: n };
            prop_assert_eq!(Hlc::from_bytes(&h.to_bytes()), h);
        }

        #[test]
        fn order_is_lexicographic(a in any::<(u64, u32, u32)>(), b in any::<(u64, u32, u32)>()) {
            let ha = Hlc { wall_ms: a.0, counter: a.1, node: a.2 };
            let hb = Hlc { wall_ms: b.0, counter: b.1, node: b.2 };
            prop_assert_eq!(ha.cmp(&hb), a.cmp(&b));
        }
    }
}
