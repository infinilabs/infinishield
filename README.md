# infinishield

[![Build](https://github.com/infinilabs/infinishield/actions/workflows/build.yml/badge.svg)](https://github.com/infinilabs/infinishield/actions/workflows/build.yml)

A command-line tool for embedding invisible, robust watermarks into images with cropping resistance.

## How It Works

infinishield embeds text messages (e.g., copyright notices, user IDs) into images in a way that is:

- **Invisible** — changes are imperceptible to the human eye
- **Blind** — extraction does not require the original image
- **Password-protected** — only someone with the correct password can extract the watermark
- **Cropping-resistant** — watermark survives partial image cropping (feature-point mode)

### Dual-Mode Embedding

infinishield automatically selects the best embedding mode based on image content and message length:

**Feature-Point Mode** (default for short messages ≤ 7 bytes):
- Detects stable keypoints using oriented FAST corners
- Embeds the same watermark at every keypoint via spread spectrum
- Survives cropping: majority vote across surviving keypoints recovers the message
- Max message: 7 bytes

**Global DWT Mode** (fallback for longer messages):
- 1-level Haar wavelet transform on the full image
- Spread spectrum embedding in the HL detail subband
- Higher capacity but no cropping resistance
- Max message depends on image size (e.g., ~40 bytes for 1024×1024)

### Auto Intensity

When `--intensity` is omitted, infinishield selects optimal strength based on image size:

| Image Size | Auto Intensity |
|-----------|:--------------:|
| < 0.5 MP | 7 |
| 0.5–2 MP | 5 |
| 2–8 MP | 4 |
| > 8 MP | 3 |

Smaller images need higher intensity to survive u8 quantization; larger images can use lower intensity for better invisibility.

## Building

Requires Rust 1.70+ (no system libraries needed — pure Rust implementation).

```bash
cargo build --release
```

The binary will be at `target/release/infinishield`.

## Usage

### Embed a Watermark

```bash
# Minimal — uses default message, password, and auto intensity
infinishield embed -i source.jpg -o output.png

# Full — all parameters explicit
infinishield embed \
  -i source.jpg \
  -m "Infini" \
  -p "d1ng0" \
  -o output.png \
  --intensity 5

# Dry run — preview without writing output
infinishield embed -i source.jpg -o output.png --dry-run
```

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `-i, --input` | yes | — | Input image path (PNG, JPEG, WebP, BMP, TIFF, GIF) |
| `-o, --output` | yes | — | Output image path (PNG recommended for lossless) |
| `-m, --message` | no | `"Infini"` | Message to embed as watermark |
| `-p, --password` | no | `"d1ng0"` | Password for scrambling and verification |
| `--intensity` | no | auto | Embedding strength (1-10). Auto-selected from image size if omitted |
| `--dry-run` | no | — | Preview embedding info without writing the output file |

**Output format:** Any format supported by the `image` crate (PNG, JPEG, WebP, BMP, TIFF, GIF). PNG is recommended because it is lossless. A warning is printed if the output format is lossy (JPEG, WebP, GIF) as compression may degrade the watermark.

Example output:
```
[成功] 水印已嵌入。
[信息] 模式: feature-point | 消息: "Infini" (6 字节) | 强度: 7 | 抗压缩率: 中
[信息] 图像: 700x496 | 特征点: 200 | 容量上限: 7 字节
[信息] 输出: output.png
```

Dry-run output:
```
[模拟] 水印嵌入预览 (未生成文件):
[信息] 模式: feature-point | 消息: "Infini" (6 字节) | 强度: 3 | 抗压缩率: 低
[信息] 图像: 3840x2160 | 特征点: 200 | 容量上限: 7 字节
[信息] 输出: output.png
```

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
[验证结果] 匹配成功！(置信度: 99.3%)
[提取内容] "Infini"
```

Output on failure (no watermark or wrong password):
```
[验证结果] 失败。未检测到有效水印，或密码错误。
```

## Running Tests

```bash
make test-unit         # Unit tests (debug)
make test-integration  # Integration tests with real images (debug)
make test              # All tests (debug)
make test-release      # All tests (release)
make sanity            # Full check: fmt + lint + build + all tests (debug & release)
```

## Current Limitations

- **Short messages only for cropping resistance** — messages up to 7 bytes (e.g., "Infini") survive cropping. Longer messages (e.g., "Copyright: InfiniLabs") still work but lose cropping protection.
- **No rotation or scaling resistance** — the watermark survives cropping and compression, but not if the image is rotated or resized.
- **Raster images only** — JPEG, PNG, WebP, BMP, TIFF, GIF. SVG and video support is planned.
- **Lossy output degrades watermark** — saving as JPEG or WebP compresses the watermark. PNG or BMP output is recommended. A warning is printed for lossy formats.

## Project Structure

```
src/
├── main.rs                # CLI entry point with format auto-detection
├── lib.rs                 # Module declarations
├── common/
│   ├── engine.rs          # WatermarkEngine trait, EmbedResult, ExtractResult, EmbedInfo
│   ├── ecc.rs             # Repetition-based error correction coding
│   ├── scramble.rs        # ChaCha20-seeded Fisher-Yates bit permutation
│   ├── password.rs        # SHA256 password hashing
│   └── temp_input_for_inference.rs  # Managed inference buffer
├── raster/
│   ├── mod.rs             # RasterEngine: feature-point + global DWT dual mode
│   ├── dwt.rs             # 1-level 2D Haar wavelet transform
│   └── features.rs        # Oriented FAST keypoint detection + patch extraction
├── vector/                # (Phase 2: SVG Fourier descriptor watermarking)
└── video/                 # (Phase 3: Video temporal Harris + 3D-SVD)
```
