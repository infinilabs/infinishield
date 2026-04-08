//! Integration tests using real test images from testing_data/pic/.
//!
//! Outputs are written to testing_output/ (gitignored) and overwritten on each run.

use std::path::{Path, PathBuf};

use image::GenericImageView;
use infinishield::common::WatermarkEngine;
use infinishield::raster::RasterEngine;

const DEFAULT_MESSAGE: &str = "Infini";
const DEFAULT_PASSWORD: &str = "d1ng0";
/// Long message that exceeds feature-point capacity (>7 bytes) → forces global DWT fallback.
const LONG_MESSAGE: &str = "Copyright: InfiniLabs";

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn input_dir() -> PathBuf {
    project_root().join("testing_data").join("pic")
}

fn output_dir() -> PathBuf {
    let dir = project_root().join("testing_output");
    std::fs::create_dir_all(&dir).expect("failed to create testing_output/");
    dir
}

fn output_path(name: &str) -> PathBuf {
    output_dir().join(name)
}

/// Helper: embed a message, then verify it round-trips correctly.
fn assert_embed_verify(
    input: &Path,
    message: &str,
    password: &str,
    intensity: u8,
    output_name: &str,
) {
    let out = output_path(output_name);

    let result = RasterEngine
        .embed(
            input.to_str().unwrap(),
            message,
            password,
            intensity,
            out.to_str().unwrap(),
        )
        .expect(&format!("embed failed for {}", input.display()));

    assert!(
        result.message.contains("成功"),
        "Embed did not report success for {}",
        input.display()
    );
    assert!(out.exists(), "Output file not created: {}", out.display());

    let extract = RasterEngine
        .verify(out.to_str().unwrap(), password)
        .expect(&format!("verify failed for {}", out.display()));

    assert!(
        extract.detected,
        "Watermark not detected in {} (confidence: {:.1}%)",
        out.display(),
        extract.confidence * 100.0
    );
    assert_eq!(
        extract.message.as_deref(),
        Some(message),
        "Extracted message mismatch for {}",
        out.display()
    );
}

/// Helper: verify that a wrong password does NOT extract the correct message.
fn assert_wrong_password_fails(output: &Path, correct_message: &str) {
    let result = RasterEngine
        .verify(output.to_str().unwrap(), "totally_wrong_password_xyz")
        .expect("verify call itself should not error");

    // Either not detected, or message doesn't match
    if result.detected {
        assert_ne!(
            result.message.as_deref(),
            Some(correct_message),
            "Wrong password should not extract correct message from {}",
            output.display()
        );
    }
}

// ── PNG input tests ──────────────────────────────────────────────────────

#[test]
fn test_png_coco_handdraw_embed_verify() {
    // 526x524 PNG — small image, uses short message due to limited capacity
    let input = input_dir().join("coco_handdraw.png");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        5,
        "coco_handdraw_wm.png",
    );
}

#[test]
fn test_png_coco_handdraw_wrong_password() {
    let input = input_dir().join("coco_handdraw.png");
    let out = output_path("coco_handdraw_wp.png");

    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            5,
            out.to_str().unwrap(),
        )
        .unwrap();

    assert_wrong_password_fails(&out, DEFAULT_MESSAGE);
}

// ── JPEG input tests ─────────────────────────────────────────────────────

#[test]
fn test_jpg_abc_embed_verify() {
    // 700x496 JPEG — moderate size, uses short message due to limited capacity
    let input = input_dir().join("abc.jpg");
    assert_embed_verify(&input, DEFAULT_MESSAGE, DEFAULT_PASSWORD, 5, "abc_wm.png");
}

#[test]
fn test_jpg_fender_embed_verify() {
    // 1832x4016 JPEG — tall image, full default message
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        5,
        "fender_wm.png",
    );
}

#[test]
fn test_jpg_meili_embed_verify() {
    // 3840x2160 JPEG — large 4K image, full default message
    let input = input_dir().join("梅里.jpg");
    assert_embed_verify(&input, DEFAULT_MESSAGE, DEFAULT_PASSWORD, 5, "meili_wm.png");
}

// ── Intensity variation tests ────────────────────────────────────────────

#[test]
fn test_intensity_low() {
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        1,
        "fender_int1.png",
    );
}

#[test]
fn test_intensity_high() {
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        10,
        "fender_int10.png",
    );
}

// ── Edge cases ───────────────────────────────────────────────────────────

#[test]
fn test_single_char_message() {
    // Minimum viable message — still uses default password
    let input = input_dir().join("abc.jpg");
    assert_embed_verify(&input, "X", DEFAULT_PASSWORD, 5, "abc_single_char.png");
}

#[test]
fn test_unicode_message() {
    // UTF-8 multi-byte characters
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        "版权所有",
        DEFAULT_PASSWORD,
        5,
        "fender_unicode.png",
    );
}

#[test]
fn test_message_too_long() {
    // 526x524 image has limited capacity (~6 bytes)
    let input = input_dir().join("coco_handdraw.png");
    let out = output_path("coco_toolong.png");
    let long_msg = "This message is definitely way too long for a small image";

    let result = RasterEngine.embed(
        input.to_str().unwrap(),
        long_msg,
        DEFAULT_PASSWORD,
        5,
        out.to_str().unwrap(),
    );

    assert!(
        result.is_err(),
        "Should reject message that exceeds capacity"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("too long") || err.contains("capacity"),
        "Error should mention capacity: {}",
        err
    );
}

#[test]
fn test_verify_unwatermarked_image() {
    // Original image without any watermark
    let input = input_dir().join("abc.jpg");
    let result = RasterEngine
        .verify(input.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    // Should either not detect, or detect garbage (not a valid message)
    if result.detected {
        // If it false-positives, the message should be garbage, not something meaningful
        println!(
            "Warning: false positive on unwatermarked image (confidence: {:.1}%, message: {:?})",
            result.confidence * 100.0,
            result.message
        );
    }
}

// ── Re-embed (overwrite) test ────────────────────────────────────────────

#[test]
fn test_output_overwrite() {
    let input = input_dir().join("abc.jpg");
    let out = output_path("abc_overwrite.png");

    // First embed with default password
    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            5,
            out.to_str().unwrap(),
        )
        .unwrap();

    let meta1 = std::fs::metadata(&out).unwrap();

    // Second embed overwrites with a different password
    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            "d1ng0_alt",
            5,
            out.to_str().unwrap(),
        )
        .unwrap();

    let meta2 = std::fs::metadata(&out).unwrap();
    assert!(
        meta2.modified().unwrap() >= meta1.modified().unwrap(),
        "Output file should be overwritten"
    );

    // Verify with the second password
    let result = RasterEngine
        .verify(out.to_str().unwrap(), "d1ng0_alt")
        .unwrap();
    assert!(result.detected);
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));

    // First password should not extract the same message
    let result = RasterEngine
        .verify(out.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();
    if result.detected {
        assert_ne!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
    }
}

// ── Cropping resistance tests ────────────────────────────────────────────

/// Crop an image: keep only the specified region.
fn crop_image(input: &Path, output: &Path, x: u32, y: u32, width: u32, height: u32) {
    let img = image::open(input).unwrap();
    let cropped = img.crop_imm(x, y, width, height);
    cropped.save(output).unwrap();
}

#[test]
fn test_crop_25_percent_top_left() {
    // Embed in full image, crop to top-left 75%, verify extraction
    let input = input_dir().join("梅里.jpg");
    let wm_path = output_path("fender_crop25_wm.png");
    let cropped_path = output_path("fender_crop25_cropped.png");

    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            5,
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    // Crop off 25% from bottom-right
    let img = image::open(&wm_path).unwrap();
    let (w, h) = img.dimensions();
    let crop_w = w * 3 / 4;
    let crop_h = h * 3 / 4;
    crop_image(&wm_path, &cropped_path, 0, 0, crop_w, crop_h);

    let result = RasterEngine
        .verify(cropped_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    assert!(
        result.detected,
        "Watermark should survive 25% crop (confidence: {:.1}%)",
        result.confidence * 100.0
    );
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
}

#[test]
fn test_crop_25_percent_bottom_right() {
    // Crop from top-left corner (remove top-left 25%)
    let input = input_dir().join("梅里.jpg");
    let wm_path = output_path("fender_crop25br_wm.png");
    let cropped_path = output_path("fender_crop25br_cropped.png");

    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            5,
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    let img = image::open(&wm_path).unwrap();
    let (w, h) = img.dimensions();
    let offset_x = w / 4;
    let offset_y = h / 4;
    crop_image(
        &wm_path,
        &cropped_path,
        offset_x,
        offset_y,
        w - offset_x,
        h - offset_y,
    );

    let result = RasterEngine
        .verify(cropped_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    assert!(
        result.detected,
        "Watermark should survive 25% crop from top-left (confidence: {:.1}%)",
        result.confidence * 100.0
    );
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
}

#[test]
fn test_crop_50_percent_center() {
    // Keep only the center 50% of the image
    let input = input_dir().join("梅里.jpg");
    let wm_path = output_path("meili_crop50_wm.png");
    let cropped_path = output_path("meili_crop50_cropped.png");

    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            7,
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    let img = image::open(&wm_path).unwrap();
    let (w, h) = img.dimensions();
    let offset_x = w / 4;
    let offset_y = h / 4;
    crop_image(&wm_path, &cropped_path, offset_x, offset_y, w / 2, h / 2);

    let result = RasterEngine
        .verify(cropped_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    assert!(
        result.detected,
        "Watermark should survive 50% center crop (confidence: {:.1}%)",
        result.confidence * 100.0
    );
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
}

// ── Long message (global DWT fallback) tests ─────────────────────────────

#[test]
fn test_long_message_fender() {
    // LONG_MESSAGE (21 bytes) exceeds FP capacity (7 bytes) → global DWT fallback
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        LONG_MESSAGE,
        DEFAULT_PASSWORD,
        5,
        "fender_long_msg.png",
    );
}

#[test]
fn test_long_message_meili() {
    // LONG_MESSAGE on large 4K image → global DWT fallback
    let input = input_dir().join("梅里.jpg");
    assert_embed_verify(
        &input,
        LONG_MESSAGE,
        DEFAULT_PASSWORD,
        5,
        "meili_long_msg.png",
    );
}

// ── Auto intensity tests (intensity=0) ───────────────────────────────────

#[test]
fn test_auto_intensity_small_image() {
    // 526x524 = 0.28MP → auto intensity 7
    let input = input_dir().join("coco_handdraw.png");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "coco_auto_int.png",
    );
}

#[test]
fn test_auto_intensity_medium_image() {
    // 700x496 = 0.35MP → auto intensity 7
    let input = input_dir().join("abc.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "abc_auto_int.png",
    );
}

#[test]
fn test_auto_intensity_large_image() {
    // 1832x4016 = 7.4MP → auto intensity 4
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "fender_auto_int.png",
    );
}

#[test]
fn test_auto_intensity_4k_image() {
    // 3840x2160 = 8.3MP → auto intensity 3
    let input = input_dir().join("梅里.jpg");
    assert_embed_verify(
        &input,
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "meili_auto_int.png",
    );
}

// ── Long message + auto intensity ────────────────────────────────────────

#[test]
fn test_long_message_auto_intensity_fender() {
    // Long message → global DWT fallback, auto intensity
    let input = input_dir().join("fender_hybrid2_st_sss_limited_black.jpg");
    assert_embed_verify(
        &input,
        LONG_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "fender_long_auto.png",
    );
}

#[test]
fn test_long_message_auto_intensity_meili() {
    // Long message on 4K → global DWT fallback, auto intensity
    let input = input_dir().join("梅里.jpg");
    assert_embed_verify(
        &input,
        LONG_MESSAGE,
        DEFAULT_PASSWORD,
        0, // auto
        "meili_long_auto.png",
    );
}

// ── Cropping + auto intensity ────────────────────────────────────────────

#[test]
fn test_crop_auto_intensity() {
    // Auto intensity + cropping resistance
    let input = input_dir().join("梅里.jpg");
    let wm_path = output_path("meili_crop_auto_wm.png");
    let cropped_path = output_path("meili_crop_auto_cropped.png");

    RasterEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0, // auto
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    // Crop 25% from bottom-right (less aggressive, suitable for auto intensity)
    let img = image::open(&wm_path).unwrap();
    let (w, h) = img.dimensions();
    crop_image(&wm_path, &cropped_path, 0, 0, w * 3 / 4, h * 3 / 4);

    let result = RasterEngine
        .verify(cropped_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    assert!(
        result.detected,
        "Watermark should survive 25% crop with auto intensity (confidence: {:.1}%)",
        result.confidence * 100.0
    );
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
}
