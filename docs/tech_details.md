# Technical Details

## Architecture

infinishield uses a trait-based engine architecture. Each file format has a dedicated engine that implements the `WatermarkEngine` trait. The CLI auto-detects the format by file extension and routes to the correct engine.

```
infinishield CLI
  │
  ├── RasterEngine (JPEG/PNG/WebP/BMP/TIFF/GIF)
  ├── VectorEngine (SVG)
  └── VideoEngine  (MP4/WebM/MOV/AVI/MKV via ffmpeg-next, statically linked)
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
    └── mod.rs                       # VideoEngine: keyframe watermarking via ffmpeg-next + RasterEngine
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

The auto-intensity system balances watermark invisibility against extraction reliability. The `fp_alpha` floor adapts to image size: small images use a lower floor (4.0) because every pixel modification is more visible, while large images use a higher floor (8.0) to ensure reliable extraction across more area. With 200 keypoints doing inter-patch majority voting, small images can afford lower per-pixel alpha without sacrificing detection confidence.

| Image Size | Auto Intensity | fp_alpha floor | Effective fp_alpha |
|-----------|:--------------:|:--------------:|:------------------:|
| < 0.5 MP | 3 | 4.0 | max(1.5×4, 4) = 6.0 |
| 0.5–2 MP | 4 | 4.0 | max(2.0×4, 4) = 8.0 |
| 2–8 MP | 5 | 8.0 | max(2.5×4, 8) = 10.0 |
| > 8 MP | 4 | 8.0 | max(2.0×4, 8) = 8.0 |

The `fp_alpha` floor threshold is 1.0 megapixels: images below 1 MP get floor=4.0, images at or above get floor=8.0. Users can override auto-intensity with `--intensity 1..10`; the floor still applies based on image size.

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

## Video Engine

### Architecture

The video engine uses `ffmpeg-next` (statically linked via the `build` feature) for in-memory frame processing. No external ffmpeg binary needed at runtime.

**Pipeline (single-pass streaming, O(1) memory):**
1. **Read** a packet from the input container
2. **If video:** decode → convert to RGB → conditionally watermark via `RasterEngine::embed_buffer` (in-memory, zero disk I/O) → convert back to YUV → encode with H.264 → write to output
3. **If audio:** copy packet directly to output (same loop, no second pass)
4. **Repeat** until EOF, then flush decoder and encoder

Only 1-2 frames are held in memory at any time. Videos of any length can be processed.

**Why RasterEngine per-keyframe (not raw 3D-SVD):**

The spec mandated 3D-SVD (8×8×8 spatiotemporal blocks, nalgebra SVD, modify singular values). We implemented and tested three approaches:

1. **3D-SVD on singular values + MPEG4:** SVD modifies singular values of the flattened 64×8 matrix. After MPEG4 re-encoding, DCT quantization destroys the SVD modifications. Result: 0% extraction.

2. **3D-SVD on singular values + H.264 CRF 18:** Same SVD approach but with H.264. Only 8 singular values per block (min(64,8)=8) → max 8 bits per block → 0 bytes of usable message (8 bits = 1-byte header only). Increasing block temporal depth or using multiple SVs per bit doesn't help because the 8-bit capacity limit is fundamental to the matrix dimensions.

3. **Temporal spread spectrum + H.264:** One bit per spatial position (64 bits per block), spread across 8 temporal frames. The codec's DCT quantization on individual frames destroys the per-pixel temporal modifications. Result: 0% extraction even with alpha=25.

4. **RasterEngine per-keyframe + H.264:** The proven spatial spread spectrum (8×8 pixel blocks, mean-centered correlation) applied to individual keyframes. This works because the 8×8 block size aligns with the codec's DCT processing. H.264 CRF 18 preserves enough of the spatial watermark for reliable detection.

**Bottom line:** Codec compression is a DCT-based spatial operation. Only spatial watermarking techniques that align with the codec's block structure survive re-encoding. Temporal or SVD-domain modifications get destroyed because they don't map to the codec's processing domain.

**H.264 requirement:** `libx264-dev` must be installed at build time. The `build-lib-x264` feature in ffmpeg-sys-next does NOT compile x264 from source — it links against the system-installed library, which then gets statically linked into the final binary.

### Build Requirements

Video support is behind a cargo feature flag. Default builds exclude it entirely:

```toml
# Cargo.toml
[features]
video = ["dep:ffmpeg-next", "dep:nalgebra"]

[dependencies]
ffmpeg-next = { version = "7", features = ["build", "build-license-gpl", "build-lib-x264"], optional = true }
nalgebra = { version = "0.33", optional = true }
```

System build tools needed: `nasm`, `pkg-config`, `gcc`, `make`, `libx264-dev`.

### Supported Formats

| Container | Decode | Encode |
|-----------|--------|--------|
| MP4 | Yes | Yes (H.264) |
| WebM | Yes | Yes (H.264) |
| MOV | Yes | Yes (H.264) |
| AVI | Yes | Yes (H.264) |
| MKV | Yes | Yes (H.264) |

Output is H.264 encoded via libx264 (CRF 18). Requires `libx264-dev` at build time.

### Limitations

- H.264 re-encoding may cause minor bit errors on complex scenes (detection is reliable, exact byte recovery is not always guaranteed)
- Message limit: 7 bytes (same as raster feature-point mode)
- Temporal trim may lose watermark if all watermarked keyframes are trimmed away
- Audio is stream-copied (codec compatibility depends on output container)

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
| Video integration | 13 | 3 original + 6 generated videos (720p/1080p/60fps/portrait/VP9/MKV), wrong password, custom message, temporal trim, dry-run |
| Doc tests | 1 | TempInputForInference usage example |
| **Total** | **91** | |

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

| `ffmpeg-next` | No (C FFmpeg) | Video decode/encode, statically linked (optional, `video` feature) |
| `nalgebra` | Yes | Matrix operations (optional, `video` feature) |

Core dependencies are pure Rust. The `video` feature adds `ffmpeg-next` which builds FFmpeg + libx264 from source during `cargo build` and statically links them — no runtime dependencies needed.
