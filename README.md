# infinishield

[English](README.md) | [简体中文](README_cn.md)

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
| `--intensity` | no | auto | Strength 1-10 (images only, see below) |
| `--dry-run` | no | — | Preview without writing |

**Intensity:** When omitted, infinishield automatically selects optimal strength using a logarithmic curve based on image size — smaller images get lighter watermarks to stay invisible, larger images get stronger ones. When set manually (`--intensity 1..10`), your value scales the auto curve (1 = 50%, 5 = ~auto, 10 = 200%), so the watermark remains size-aware.

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
- Images should be at least 512×512 pixels. Larger images produce better results with less visible impact.
- Rotation and resizing will destroy the watermark — only cropping and compression are tolerated.
- Auto intensity adapts to image size: small images (~0.3 MP) get subtle embedding, large images (8+ MP) get stronger embedding. Override with `--intensity` if needed.

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

## Web UI

A browser-based testing interface for embedding and verifying watermarks. Requires [Go](https://go.dev/) 1.21+.

```bash
make webapp     # Build + run on http://localhost:1983
```

Features:
- Upload images, SVGs, or videos for watermarking
- Configure message, password, and intensity with real-time dry-run preview
- Side-by-side comparison of original vs watermarked assets
- Verify tab for extracting watermarks from files
- Event logging to `webapp/logs/`

## Running Tests

```bash
make test       # Images + SVG tests
make sanity     # Full check: format + lint + build + all tests (including video)
make clean      # Remove build artifacts
```

## Troubleshooting

### Visible noise or artifacts after embedding

The watermark modifies pixel values in the green channel. On some images — especially small ones or images with large flat/uniform areas (sky, solid backgrounds, simple illustrations) — this can produce faint visible noise.

**Try these steps in order:**

1. **Use a lower intensity.** The default auto mode is tuned for a good balance, but you can reduce it:
   ```bash
   infinishield embed -i photo.jpg -o out.png --intensity 3
   infinishield embed -i photo.jpg -o out.png --intensity 1   # minimum strength
   ```
   Lower intensity = less visible noise but weaker watermark. Use `--dry-run` to preview before committing.

2. **Use a larger source image.** The watermark is spread across more pixels in larger images, making it less visible per pixel. If possible, watermark the full-resolution original rather than a resized thumbnail. Images under 512×512 pixels are not recommended.

3. **Check the output format.** Always use PNG output (`-o out.png`). JPEG re-encodes with lossy compression, which can amplify watermark artifacts. The input can be any supported format.

4. **Try `--dry-run` first.** Preview the embedding parameters without writing a file:
   ```bash
   infinishield embed -i photo.jpg -o out.png --dry-run
   ```
   This shows the mode, intensity, keypoint count, and capacity — useful for understanding what the tool will do.

5. **Understand the image characteristics.** Watermarks are naturally less visible in textured, detailed areas (landscapes, photographs) and more visible in flat, uniform areas (logos, line art, solid backgrounds). This is inherent to all spatial-domain watermarking.

### Watermark not detected after cropping

- Only messages up to **7 bytes** support cropping resistance (feature-point mode). Longer messages use global DWT mode, which has no cropping resistance.
- Cropping must preserve enough keypoint regions. Very aggressive cropping (> 75%) may remove too many feature points.
- Resizing or rotating the image **will** destroy the watermark — only cropping and compression are tolerated.

### Wrong message extracted

- Make sure you use the same password for embed and verify (`-p` flag).
- Very low intensity (`--intensity 1`) on large images may cause occasional bit errors. Use a higher intensity or auto mode for reliable extraction.
- JPEG output degrades the watermark. Always verify from the PNG output, not a re-encoded JPEG.

### Video watermark issues

- Video re-encodes with H.264 (CRF 18). On complex scenes, 1-2 bits may flip — detection is reliable but exact byte recovery is not always guaranteed.
- If verifying a trimmed video, ensure at least some watermarked keyframes remain. Keyframes are watermarked at 1 per second.

## Documentation

- [Technical Details](docs/tech_details.md) — architecture, algorithms, what works and what doesn't
