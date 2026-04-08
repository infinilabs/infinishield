//! Singular managed buffer for all algorithmic inference workloads.
//!
//! All intermediate matrices, tensor data, and PN sequences during feature detection,
//! patch normalization, and transform operations (DWT, DFT, SVD) must be routed
//! exclusively through this buffer. No secondary temporary allocations or unmanaged
//! in-memory buffers may be instantiated for inference workloads.
//!
//! Thread safety: each thread gets its own `TempInputForInference` instance.
//! The struct is `Send` but not `Sync` — pass by `&mut` reference only.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};

/// Block size used across all engines for spread spectrum embedding.
pub const BLOCK_SIZE: usize = 16;

/// Number of coefficients per block (BLOCK_SIZE × BLOCK_SIZE).
pub const COEFFS_PER_BLOCK: usize = BLOCK_SIZE * BLOCK_SIZE;

/// The singular temporary buffer for all inference workloads.
///
/// Pre-allocates scratch space for patch pixels, DWT coefficients, and PN chip
/// sequences. Buffers are reused across patches to avoid per-patch allocation.
///
/// # Usage
///
/// ```rust,no_run
/// use infinishield::common::TempInputForInference;
///
/// let mut ctx = TempInputForInference::new(64); // 64×64 patches
/// ctx.set_seed([0u8; 32]);
/// let pn = ctx.generate_pn_chip(0);
/// let data = ctx.patch_buffer();
/// ```
pub struct TempInputForInference {
    /// Scratch buffer for patch pixel data, sized for max patch dimensions.
    patch_buf: Vec<f64>,
    /// Maximum patch side length this context was allocated for.
    max_patch_size: usize,
    /// Pre-allocated PN chip sequence buffer (COEFFS_PER_BLOCK elements).
    pn_buf: Vec<f64>,
    /// Seed for PN sequence generation (derived from password).
    seed: [u8; 32],
}

impl TempInputForInference {
    /// Create a new inference buffer pre-allocated for patches up to
    /// `max_patch_size × max_patch_size` pixels.
    pub fn new(max_patch_size: usize) -> Self {
        Self {
            patch_buf: vec![0.0; max_patch_size * max_patch_size],
            max_patch_size,
            pn_buf: vec![0.0; COEFFS_PER_BLOCK],
            seed: [0u8; 32],
        }
    }

    /// Set the password-derived seed for PN sequence generation.
    pub fn set_seed(&mut self, seed: [u8; 32]) {
        self.seed = seed;
    }

    /// Get the current seed.
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Generate a pseudo-random PN (pseudo-noise) chip sequence of ±1 values
    /// for a specific block index. Uses ChaCha20 seeded by a per-block hash
    /// of the master seed + block index.
    ///
    /// The result is written into the internal `pn_buf` and a reference is returned.
    pub fn generate_pn_chip(&mut self, block_idx: usize) -> &[f64] {
        let mut hasher = Sha256::new();
        hasher.update(self.seed);
        hasher.update(block_idx.to_le_bytes());
        let block_seed_hash = hasher.finalize();
        let mut block_seed = [0u8; 32];
        block_seed.copy_from_slice(&block_seed_hash);

        let mut rng = ChaCha20Rng::from_seed(block_seed);
        for v in self.pn_buf.iter_mut() {
            *v = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
        }
        &self.pn_buf
    }

    /// Load a rectangular region from a 2D coefficient array into the patch buffer.
    ///
    /// Copies `rows × cols` values starting at `(start_row, start_col)` from `source`
    /// into the internal patch buffer in row-major order.
    pub fn load_patch(
        &mut self,
        source: &[Vec<f64>],
        start_row: usize,
        start_col: usize,
        rows: usize,
        cols: usize,
    ) {
        assert!(
            rows * cols <= self.patch_buf.len(),
            "Patch {}×{} exceeds buffer capacity (max {}×{})",
            rows,
            cols,
            self.max_patch_size,
            self.max_patch_size
        );
        for r in 0..rows {
            for c in 0..cols {
                self.patch_buf[r * cols + c] = source[start_row + r][start_col + c];
            }
        }
    }

    /// Write the patch buffer contents back to a 2D coefficient array.
    pub fn store_patch(
        &self,
        dest: &mut [Vec<f64>],
        start_row: usize,
        start_col: usize,
        rows: usize,
        cols: usize,
    ) {
        for r in 0..rows {
            for c in 0..cols {
                dest[start_row + r][start_col + c] = self.patch_buf[r * cols + c];
            }
        }
    }

    /// Read-only access to the patch buffer.
    pub fn patch_buffer(&self) -> &[f64] {
        &self.patch_buf
    }

    /// Mutable access to the patch buffer for in-place modification.
    pub fn patch_buffer_mut(&mut self) -> &mut [f64] {
        &mut self.patch_buf
    }

    /// Get the PN buffer (read-only, from last `generate_pn_chip` call).
    pub fn pn_buffer(&self) -> &[f64] {
        &self.pn_buf
    }

    /// Embed a single bit into BLOCK_SIZE×BLOCK_SIZE coefficients in the patch buffer
    /// using additive spread spectrum.
    ///
    /// `offset` is the starting index in `patch_buf` for this block.
    /// The PN chip must have been generated prior to calling this.
    pub fn embed_spread_spectrum(&mut self, offset: usize, bit: bool, alpha: f64) {
        let signal = if bit { 1.0 } else { -1.0 };
        for i in 0..COEFFS_PER_BLOCK {
            self.patch_buf[offset + i] += alpha * self.pn_buf[i] * signal;
        }
    }

    /// Extract a single bit from BLOCK_SIZE×BLOCK_SIZE coefficients in the patch buffer
    /// using spread spectrum correlation.
    ///
    /// Returns `(bit, confidence)` where confidence is normalized correlation strength.
    pub fn extract_spread_spectrum(&self, offset: usize) -> (bool, f64) {
        let mut correlation = 0.0;
        for i in 0..COEFFS_PER_BLOCK {
            correlation += self.patch_buf[offset + i] * self.pn_buf[i];
        }
        let bit = correlation >= 0.0;
        let confidence = (correlation.abs() / COEFFS_PER_BLOCK as f64).min(1.0);
        (bit, confidence)
    }
}

// TempInputForInference is Send (can move between threads) but NOT Sync
// (cannot be shared between threads). Each thread must own its own instance.
// This is the default for structs with no interior mutability issues,
// but we document it explicitly as part of the memory protocol.
unsafe impl Send for TempInputForInference {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pn_chip_deterministic() {
        let seed = [42u8; 32];
        let mut ctx1 = TempInputForInference::new(16);
        let mut ctx2 = TempInputForInference::new(16);
        ctx1.set_seed(seed);
        ctx2.set_seed(seed);

        let pn1 = ctx1.generate_pn_chip(0).to_vec();
        let pn2 = ctx2.generate_pn_chip(0).to_vec();
        assert_eq!(pn1, pn2);
    }

    #[test]
    fn test_pn_chip_different_blocks() {
        let seed = [42u8; 32];
        let mut ctx = TempInputForInference::new(16);
        ctx.set_seed(seed);

        let pn0 = ctx.generate_pn_chip(0).to_vec();
        let pn1 = ctx.generate_pn_chip(1).to_vec();
        assert_ne!(pn0, pn1);
    }

    #[test]
    fn test_pn_chip_values_are_pm1() {
        let mut ctx = TempInputForInference::new(16);
        ctx.set_seed([7u8; 32]);
        let pn = ctx.generate_pn_chip(0);
        for &v in pn {
            assert!(v == 1.0 || v == -1.0, "PN value must be ±1, got {}", v);
        }
    }

    #[test]
    fn test_spread_spectrum_round_trip() {
        let mut ctx = TempInputForInference::new(16);
        ctx.set_seed([99u8; 32]);

        // Fill patch buffer with some baseline values
        for (i, v) in ctx.patch_buffer_mut().iter_mut().enumerate() {
            *v = (i as f64) * 0.5 - 64.0;
        }

        for bit in [true, false] {
            // Reset buffer
            for (i, v) in ctx.patch_buffer_mut().iter_mut().enumerate() {
                *v = (i as f64) * 0.5 - 64.0;
            }
            ctx.generate_pn_chip(0);
            ctx.embed_spread_spectrum(0, bit, 2.0);

            // Re-generate PN for extraction (same chip)
            ctx.generate_pn_chip(0);
            let (extracted, confidence) = ctx.extract_spread_spectrum(0);
            assert_eq!(extracted, bit, "Failed for bit={}", bit);
            assert!(
                confidence > 0.1,
                "Low confidence {} for bit={}",
                confidence,
                bit
            );
        }
    }

    #[test]
    fn test_load_store_patch() {
        let mut ctx = TempInputForInference::new(16);

        let source = vec![
            vec![1.0, 2.0, 3.0, 4.0],
            vec![5.0, 6.0, 7.0, 8.0],
            vec![9.0, 10.0, 11.0, 12.0],
        ];

        ctx.load_patch(&source, 0, 1, 2, 2);
        assert_eq!(&ctx.patch_buffer()[..4], &[2.0, 3.0, 6.0, 7.0]);

        let mut dest = vec![vec![0.0; 4]; 3];
        ctx.store_patch(&mut dest, 1, 2, 2, 2);
        assert_eq!(dest[1][2], 2.0);
        assert_eq!(dest[1][3], 3.0);
        assert_eq!(dest[2][2], 6.0);
        assert_eq!(dest[2][3], 7.0);
    }
}
