# infinishield

[![Build](https://github.com/infinilabs/infinishield/actions/workflows/build.yml/badge.svg)](https://github.com/infinilabs/infinishield/actions/workflows/build.yml)

A command-line tool for embedding invisible watermarks into images and SVG files.

## What It Does

infinishield hides a short text message (e.g., a copyright notice) inside an image or SVG file. The watermark is:

- **Invisible** — no visible changes to the file
- **Password-protected** — only someone with the password can extract it
- **Cropping-resistant** — survives partial image cropping (for short messages)

## Supported Formats

| Format | Status |
|--------|--------|
| JPEG, PNG, WebP, BMP, TIFF, GIF | Supported |
| SVG | Supported |
| MP4, WebM (video) | Planned |

## Building

Requires [Rust](https://rustup.rs/) 1.70+.

```bash
make release    # Build optimized binary
```

The binary will be at `target/release/infinishield`. Run `make help` to see all available commands.

## Usage

### Embed a Watermark

```bash
# Simplest — embeds default message "Infini" with default password
infinishield embed -i photo.jpg -o watermarked.png

# Custom message and password
infinishield embed -i photo.jpg -m "MyMark" -p "secret" -o watermarked.png

# SVG file
infinishield embed -i logo.svg -o logo_wm.svg -m "Hi"

# Preview without writing (dry run)
infinishield embed -i photo.jpg -o out.png --dry-run
```

| Option | Required | Default | Description |
|--------|----------|---------|-------------|
| `-i` | yes | — | Input file |
| `-o` | yes | — | Output file (PNG recommended for images) |
| `-m` | no | `"Infini"` | Message to hide (max 7 bytes for cropping resistance) |
| `-p` | no | `"d1ng0"` | Password |
| `--intensity` | no | auto | Embedding strength (1-10, images only) |
| `--dry-run` | no | — | Show info without writing output |

### Verify / Extract

```bash
# Check if a file contains a watermark
infinishield verify -i watermarked.png

# With a specific password
infinishield verify -i watermarked.png -p "secret"
```

Example output:
```
[验证结果] 匹配成功！(置信度: 99.3%)
[提取内容] "Infini"
```

### Help

```bash
infinishield              # Show full help
infinishield --version    # Show version
```

## Limitations

**Images:**
- Messages up to 7 bytes survive cropping. Longer messages work but lose cropping protection.
- Rotation and resizing are not supported — only cropping and compression.
- Saving as JPEG degrades the watermark. Use PNG for best results.

**SVG:**
- Only works on SVGs with complex paths. Simple shapes (rectangles, circles) have too few coordinates.
- Message capacity is typically 2-7 bytes depending on SVG complexity.
- SVG editors that reformat coordinates may destroy the watermark.

**General:**
- Video support is planned but not yet available.

## Running Tests

```bash
make test       # All tests (debug)
make sanity     # Full check: format + lint + build + all tests
make clean      # Remove build artifacts and test outputs
```

## Documentation

- [Technical Details](docs/tech_details.md) — architecture, algorithms, design decisions
