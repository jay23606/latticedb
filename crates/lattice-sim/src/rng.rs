//! Seeded random number generator.

use lattice_seams::Rng;

/// SplitMix64: tiny, fast, and fully determined by its seed.
#[derive(Debug, Clone)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// A value in `0..n` (returns 0 when `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    /// True with probability `ppm` per million.
    pub fn chance_ppm(&mut self, ppm: u32) -> bool {
        self.below(1_000_000) < u64::from(ppm)
    }

    /// Derives an independent generator, so components do not perturb each other's streams.
    pub fn fork(&mut self) -> Self {
        Self::new(self.next_u64())
    }
}

impl Rng for SeededRng {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_and_different_seed_differs() {
        let (mut a, mut b, mut c) = (SeededRng::new(7), SeededRng::new(7), SeededRng::new(8));
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let zs: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = SeededRng::new(1);
        assert!((0..1000).all(|_| r.below(10) < 10));
        assert_eq!(r.below(0), 0);
    }
}
