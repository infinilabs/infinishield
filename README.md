# infinishield

[![Build](https://github.com/infinilabs/infinishield/actions/workflows/build.yml/badge.svg)](https://github.com/infinilabs/infinishield/actions/workflows/build.yml)

A command-line tool for embedding invisible watermarks into images, SVGs, and videos.

## What It Does

infinishield hides a short text message (e.g., a copyright notice) inside a file. The watermark is invisible, password-protected, and survives common modifications like cropping and compression.

## Supported Formats

| Format | Status | Max Message |
|--------|--------|:-----------:|
| JPEG, PNG, WebP, BMP, TIFF, GIF | Supported | 7 bytes (cropping-resistant) or ~40 bytes (longer, no crop protection) |
| SVG | Supported | 2-7 bytes (depends on path complexity) |
| MP4, WebM, MOV, AVI, MKV | Supported (optional build) | 7 bytes |

## Building

Requires [Rust](https://rustup.rs/) 1.70+.

```bash
make release          # Images + SVG only
make release-video    # Images + SVG + Video (see below)
```

### Video Support (optional)

Video requires extra system packages and takes longer to build (compiles FFmpeg from source):

```bash
# Linux (Debian/Ubuntu)
sudo apt-get install nasm pkg-config gcc make libx264-dev

# macOS
brew install nasm pkg-config x264

# Then build
make release-video
```

## Usage

### Embed

```bash
infinishield embed -i photo.jpg -o watermarked.png                      # image
infinishield embed -i logo.svg -o logo_wm.svg -m "Hi"                   # SVG
infinishield embed -i clip.mp4 -o watermarked.mp4                       # video
infinishield embed -i photo.jpg -m "MyMark" -p "secret" -o out.png      # custom message + password
infinishield embed -i photo.jpg -o out.png --dry-run                    # preview only
```

| Option | Required | Default | Description |
|--------|----------|---------|-------------|
| `-i` | yes | — | Input file |
| `-o` | yes | — | Output file |
| `-m` | no | `"Infini"` | Message to embed |
| `-p` | no | `"d1ng0"` | Password |
| `--intensity` | no | auto | Strength 1-10 (images only) |
| `--dry-run` | no | — | Preview without writing |

### Verify

```bash
infinishield verify -i watermarked.png
infinishield verify -i watermarked.png -p "secret"
```

### Help

```bash
infinishield              # full help
infinishield --version    # version
```

## Best Practices for Preparing Assets

**Images (JPEG/PNG):**
- Use PNG output (`-o out.png`) for best watermark preservation. JPEG output is lossy and degrades the watermark.
- Messages up to 7 bytes (e.g., `"Infini"`) survive cropping. For longer messages, cropping protection is lost.
- Images should be at least 512×512 pixels. Larger images work better.
- Rotation and resizing will destroy the watermark — only cropping and compression are tolerated.

**SVG:**
- Works best on SVGs with complex paths (illustrations, icons with curves). Simple geometric shapes (plain rectangles, circles) have too few coordinates to embed.
- Use `--dry-run` first to check if your SVG has enough qualifying paths.
- Don't open the watermarked SVG in editors that reformat or round coordinate values — this destroys the watermark. View-only or programmatic use is safe.

**Video:**
- Keep messages short (≤ 7 bytes). The watermark is embedded in 1 keyframe per second.
- Output is re-encoded with H.264 (CRF 18). The original codec is not preserved.
- On some videos with complex scenes, 1-2 bits of the message may flip during re-encoding. Detection is reliable but exact message recovery is not always guaranteed.
- Streaming architecture — only 1-2 frames in memory at a time. Handles videos of any length.
- The video binary includes a statically linked FFmpeg — no runtime dependencies needed.

## Running Tests

```bash
make test       # Images + SVG tests
make sanity     # Full check: format + lint + build + all tests (including video)
make clean      # Remove build artifacts
```

## Documentation

- [Technical Details](docs/tech_details.md) — architecture, algorithms, what works and what doesn't
