# Technical Details

## Architecture

infinishield uses a trait-based engine architecture. Each file format has a dedicated engine that implements the `WatermarkEngine` trait. The CLI auto-detects the format by file extension and routes to the correct engine.

```
infinishield CLI
  │
  ├── RasterEngine (JPEG/PNG/WebP/BMP/TIFF/GIF)
  ├── VectorEngine (SVG)
  └── VideoEngine  (planned)
  │
  Common Layer: ECC, ChaCha20 scrambling, SHA256 password hashing,
                TempInputForInference managed buffer
```

## Project Structure

```
src/
├── main.rs                          # CLI, format auto-detection, lossy output warnings
├── lib.rs                           # Module declarations
├── common/
│   ├── engine.rs                    # WatermarkEngine trait, EmbedResult, ExtractResult, EmbedInfo
│   ├── ecc.rs                       # 3× repetition ECC with majority vote (global DWT mode)
│   ├── scramble.rs                  # ChaCha20-seeded Fisher-Yates bit permutation
│   ├── password.rs                  # SHA256 password → 32-byte seed
│   └── temp_input_for_inference.rs  # Managed scratch buffer (configurable block size)
├── raster/
│   ├── mod.rs                       # RasterEngine: dual-mode embed/verify orchestration
│   ├── dwt.rs                       # 1-level 2D Haar wavelet transform (forward/inverse)
│   └── features.rs                  # Oriented FAST keypoint detection, patch extraction
├── vector/
│   └── mod.rs                       # VectorEngine: SVG coordinate QIM watermarking
└── video/
    └── mod.rs                       # Stub (Phase 3)
```

## Raster Engine

### Dual-Mode Strategy

The raster engine automatically selects between two modes:

**Feature-Point Mode** (messages ≤ 7 bytes):
- Detects oriented FAST keypoints from the **red** channel (stable — never modified)
- Embeds watermark in the **green** channel at keypoint locations
- Each keypoint region: 64×64 pixels divided into 8×8 blocks (64 blocks total)
- Spread spectrum: each block's pixels are perturbed by `alpha * PN * signal`
- Mean-centered correlation during extraction removes host-signal DC interference
- Same bits at every keypoint → inter-patch majority voting (Level 2 ECC)
- Cropping resistance: surviving keypoints still carry the full message

**Global DWT Mode** (messages > 7 bytes):
- 1-level Haar DWT on the full green channel
- Spread spectrum in the HL (vertical detail) subband
- 16×16 blocks with 3× repetition coding (Level 1 ECC)
- Higher capacity but no cropping resistance

### Auto Intensity

| Image Size | Auto Intensity | fp_alpha |
|-----------|:--------------:|:--------:|
| < 0.5 MP | 7 | max(3.5×4, 8) = 28 |
| 0.5–2 MP | 5 | max(2.5×4, 8) = 10 |
| 2–8 MP | 4 | max(2.0×4, 8) = 8 |
| > 8 MP | 3 | max(1.5×4, 8) = 8 |

### Why Pixel-Domain Instead of DWT-on-Patch

The spec called for DWT on 64×64 patches with rotation normalization. We tried three variants:

1. **DWT + Gaussian blending** — 4×4 blocks (16 coefficients) had insufficient SNR after blending attenuation
2. **DWT + rotation + Gaussian** — rotation interpolation + blending compounded to kill SNR
3. **DWT + rotation + direct delta** — 23/24 tests pass, but 50% crop always fails because FAST adaptive thresholding produces different keypoints after cropping, breaking patch-level DWT alignment

**Root cause:** DWT on a full 64×64 patch mixes all pixels, so any spatial misalignment corrupts all HL coefficients. Pixel-domain 8×8 blocks are independent — each block is self-contained and detectable regardless of patch alignment.

**Conclusion:** Pixel-domain is correct for cropping resistance; DWT-on-patch is correct for rotation resistance (future work).

## Vector Engine (SVG)

### Coordinate QIM

SVG files are lossless text. Watermark bits are embedded by quantizing path coordinate values using QIM (Quantization Index Modulation):

- Parse SVG text to find `d="..."` path attributes
- Extract numeric coordinate values from each path
- QIM: quantize each coordinate to encode one bit
  - `bit=1 (with pn>0)`: round to `base + 0.75 * Q`
  - `bit=0 (with pn>0)`: round to `base + 0.25 * Q`
  - PN sign flips the mapping for security
- Quantization step `Q = 0.0625` (power of 2, exactly representable in IEEE 754)
- Same bits in every qualifying path → inter-path majority voting

### Capacity

Capacity is determined by **message size**, not path structure:
- `actual_bits = (message_length + 1) * 8`
- Paths need at least `actual_bits` coordinate values to qualify
- Extraction tries all possible message lengths (1-7 bytes)

This design ensures the scramble permutation is stable regardless of which paths are deleted.

### What Didn't Work

1. **Fourier descriptors:** DFT on sampled path vertices → perturb magnitudes → IDFT → reconstruct segments. The segment reconstruction (DFT points → bezier/line segments) is lossy.
2. **usvg tree modification:** `usvg` normalizes coordinate formats during parse, making text-level replacement impossible with parsed data.
3. **Non-power-of-2 Q (0.04):** `0.04` has infinite binary representation in IEEE 754, causing `%` (modulo) to produce wrong extraction results.

## Common Layer

### TempInputForInference

Singular managed buffer for all inference workloads. Pre-allocates scratch space for patches, PN chips, and coefficients. Configurable block size (16×16 for global DWT, 8×8 for feature-point pixel blocks).

### Scrambling

ChaCha20-seeded Fisher-Yates shuffle. The password is hashed via SHA256 to produce a 32-byte seed. Deterministic: same password + same data length → same permutation.

### ECC

- **Level 1 (intra-patch):** 3× repetition coding with majority vote. Used in global DWT mode only.
- **Level 2 (inter-patch/path):** Same message at every keypoint/path. Per-bit majority vote across all copies. Used in both raster feature-point mode and SVG mode.

## Test Coverage

| Category | Tests | Coverage |
|----------|:-----:|---------|
| Unit tests | 37 | DWT round-trip, QIM, scramble, ECC, PN generation, parsing, confidence |
| Raster integration | 24 | 4 images × multiple configs, cropping (25%/50%), auto/explicit intensity, long messages |
| SVG integration | 16 | 10 SVG files, wrong password, unwatermarked, too-simple, element deletion, viewBox change |
| Doc tests | 1 | TempInputForInference usage example |
| **Total** | **78** | |

## Dependencies

| Crate | Pure Rust | Purpose |
|-------|:---------:|---------|
| `clap` | Yes | CLI argument parsing |
| `image` | Yes | Raster image I/O |
| `imageproc` | Yes | Oriented FAST keypoint detection |
| `sha2` | Yes | Password hashing |
| `rand` + `rand_chacha` | Yes | ChaCha20 PRNG for scrambling and PN generation |
| `usvg` | Yes | SVG parsing and path qualification |
| `tiny-skia-path` | Yes | SVG path segment iteration |
| `rustfft` | Yes | FFT (available for future use) |
| `num-complex` | Yes | Complex number arithmetic |

All dependencies are pure Rust — no system libraries required.
