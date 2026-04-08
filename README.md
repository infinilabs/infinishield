# infinishield

[![Build](https://github.com/infinilabs/infinishield/actions/workflows/build.yml/badge.svg)](https://github.com/infinilabs/infinishield/actions/workflows/build.yml)

A command-line tool for embedding invisible, robust watermarks into images using frequency-domain techniques (DWT + SVD + QIM).

## How It Works

infinishield embeds text messages (e.g., copyright notices, user IDs) into images in a way that is:

- **Invisible** — changes are imperceptible to the human eye
- **Blind** — extraction does not require the original image
- **Password-protected** — only someone with the correct password can extract the watermark

### Algorithm Pipeline

1. **DWT (Discrete Wavelet Transform)** — decomposes the image into frequency subbands using a 1-level Haar wavelet
2. **Spread Spectrum** — each watermark bit is embedded across a 16x16 block (256 coefficients) of the HL detail subband using a pseudo-noise (PN) chip sequence, providing inherent noise resistance through correlation averaging
3. **Scrambling** — ChaCha20-based Fisher-Yates shuffle distributes bits across blocks for security
4. **ECC (Error Correction)** — 3x repetition coding with majority vote for bit error recovery

## Building

Requires Rust 1.70+ (no system libraries needed — pure Rust implementation).

```bash
cargo build --release
```

The binary will be at `target/release/infinishield`.

## Usage

### Embed a Watermark

```bash
# Minimal — uses default message, password, and intensity
infinishield embed -i source.jpg -o output.png

# Full — all parameters explicit
infinishield embed \
  -i source.jpg \
  -m "Infini" \
  -p "d1ng0" \
  -o output.png \
  --intensity 5
```

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `-i, --input` | yes | — | Input image path (PNG or JPEG) |
| `-o, --output` | yes | — | Output image path (PNG recommended) |
| `-m, --message` | no | `"Infini"` | Message to embed as watermark |
| `-p, --password` | no | `"d1ng0"` | Password for scrambling and verification |
| `--intensity` | no | auto | Embedding strength (1-10). Auto-selected from image size if omitted (smaller images → higher intensity) |

### Verify / Extract a Watermark

```bash
# Minimal — uses default password
infinishield verify -i suspicious_image.jpg

# With explicit password
infinishield verify -i suspicious_image.jpg -p "d1ng0"
```

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `-i, --input` | yes | — | Input image path to verify |
| `-p, --password` | no | `"d1ng0"` | Password used during embedding |

Output on success:
```
[分析中] 正在执行频域扫描...
[验证结果] 匹配成功！(置信度: 64.7%)
[提取内容] "Infini"
```

Output on failure (no watermark or wrong password):
```
[验证结果] 失败。未检测到有效水印，或密码错误。
```

## Message Capacity

Capacity depends on image dimensions (with 16x16 blocks and 3x repetition ECC):

| Image Size | Max Message |
|-----------|-------------|
| 512x512 | ~8 bytes |
| 1024x1024 | ~40 bytes |
| 2048x2048 | ~170 bytes |
| 4096x4096 | ~680 bytes |

## Running Tests

```bash
make test-unit         # Unit tests (debug)
make test-integration  # Integration tests with real images (debug)
make test              # All tests (debug)
make test-release      # All tests (release)
make sanity            # Full check: fmt + lint + build + all tests (debug & release)
```

## Current Limitations (v0.1)

- **No JPEG output** — output must be PNG (lossless). JPEG input is supported.
- **No cropping resistance** — watermark extraction assumes original image dimensions are preserved. Compression and noise are tolerated; cropping is not.
- **Simple ECC** — uses 3x repetition coding. A future version will upgrade to Reed-Solomon for better capacity/robustness trade-off.
- **Single channel** — watermark is embedded in the green channel only. This avoids luma conversion rounding errors but means the green channel carries all modification.

## Project Structure

```
src/
├── main.rs       # CLI entry point (clap)
├── lib.rs        # Module declarations
├── dwt.rs        # 1-level 2D Haar wavelet transform (forward/inverse)
├── watermark.rs  # Embed/extract orchestration, spread spectrum block processing
├── ecc.rs        # Repetition-based error correction coding
└── scramble.rs   # ChaCha20-seeded Fisher-Yates bit permutation
```
