//! Seeded, reproducible randomness for the procedural lane.
//!
//! Each record gets its own PCG stream derived from the batch seed plus the
//! cell and template identity, so a record's content does not depend on how
//! many other records were generated before it. Same seed + config still
//! reproduces the batch byte for byte.

use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;
use sha2::{Digest, Sha256};

/// A small wrapper around the PCG generator used by templates.
pub struct SeededRng {
    inner: Pcg64Mcg,
}

impl SeededRng {
    /// Derive a stream from the batch seed and a set of identity parts.
    pub fn new(seed: u64, parts: &[&str]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(seed.to_le_bytes());
        for part in parts {
            hasher.update((part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self {
            inner: Pcg64Mcg::from_seed(bytes),
        }
    }

    /// Next 64-bit output.
    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Uniform-enough integer in `0..bound` (bound must be positive).
    pub fn below(&mut self, bound: u32) -> u32 {
        assert!(bound > 0, "bound must be positive");
        (self.next_u64() % u64::from(bound)) as u32
    }

    /// Inclusive integer range.
    pub fn range_inclusive(&mut self, lo: u32, hi: u32) -> u32 {
        assert!(lo <= hi, "range must be ordered");
        lo + self.below(hi - lo + 1)
    }

    /// Pick one item.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below((i + 1) as u32) as usize;
            items.swap(i, j);
        }
    }
}
