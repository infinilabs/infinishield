pub mod dwt;
pub mod features;

use crate::common::engine::{EmbedInfo, EmbedResult, ExtractResult, WatermarkEngine};
use crate::common::temp_input_for_inference::TempInputForInference;
use crate::common::{ecc, password, scramble};

use features::{detect_keypoints, FeaturePoint, PATCH_SIZE};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};

/// Embedding channel index (green = 1).
const EMBED_CHANNEL: usize = 1;

/// Block size for global DWT mode (16×16).
const GLOBAL_BLOCK_SIZE: usize = 16;

/// Block size for feature-point patch mode (8×8).
/// 64×64 patch / 8 = 8×8 = 64 blocks → 64 bits per patch.
const PATCH_BLOCK_SIZE: usize = 8;
const PATCH_COEFFS_PER_BLOCK: usize = PATCH_BLOCK_SIZE * PATCH_BLOCK_SIZE;

/// Number of blocks in a patch (PATCH_SIZE / PATCH_BLOCK_SIZE)².
const BLOCKS_PER_PATCH: usize = (PATCH_SIZE / PATCH_BLOCK_SIZE) * (PATCH_SIZE / PATCH_BLOCK_SIZE);

/// Minimum keypoints required for feature-point mode.
const MIN_KEYPOINTS: usize = 10;

/// Maximum keypoints to use for embedding.
const MAX_KEYPOINTS: usize = 200;

/// Max message bytes in feature-point mode: 64 bits - 8-bit header = 7 bytes.
const FP_MAX_MESSAGE: usize = BLOCKS_PER_PATCH / 8 - 1;

/// The raster watermark engine for JPEG/PNG/WebP images.
///
/// Uses feature-point-based Local Feature Regions (LFRs) for cropping resistance.
/// Falls back to global DWT for images with insufficient keypoints.
pub struct RasterEngine;

/// Analyze an image and compute embedding info without modifying anything.
fn analyze(
    img: &DynamicImage,
    message: &str,
    intensity: u8,
    output_path: &str,
) -> Result<(EmbedInfo, bool), String> {
    let (width, height) = img.dimensions();
    let intensity = resolve_intensity(intensity, width, height);

    let gray = channel_to_gray(img);
    let keypoints = detect_keypoints(&gray, MAX_KEYPOINTS);

    let use_fp = keypoints.len() >= MIN_KEYPOINTS && message.len() <= FP_MAX_MESSAGE;

    let (mode, max_capacity) = if use_fp {
        ("feature-point".to_string(), FP_MAX_MESSAGE)
    } else {
        let channel = extract_channel(img);
        let coeffs = dwt::forward(&channel);
        let sub_rows = coeffs.hl.len();
        let sub_cols = coeffs.hl[0].len();
        let (_, _, num_blocks) = count_blocks(sub_rows, sub_cols);
        ("global-dwt".to_string(), ecc::max_message_bytes(num_blocks))
    };

    let info = EmbedInfo {
        status: "ok".to_string(),
        mode,
        message: message.to_string(),
        message_bytes: message.len(),
        intensity,
        width,
        height,
        keypoints: keypoints.len(),
        max_capacity,
        output_path: output_path.to_string(),
    };

    Ok((info, use_fp))
}

impl WatermarkEngine for RasterEngine {
    fn embed(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedResult, String> {
        let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;
        let (info, use_fp) = analyze(&img, message, intensity, output_path)?;
        let resolved_intensity = info.intensity;

        let gray = channel_to_gray(&img);
        let keypoints = detect_keypoints(&gray, MAX_KEYPOINTS);

        if use_fp {
            embed_feature_point(
                &img,
                &gray,
                &keypoints,
                message,
                password,
                resolved_intensity,
                output_path,
            )?;
        } else {
            embed_global_dwt(&img, message, password, resolved_intensity, output_path)?;
        }

        Ok(EmbedResult {
            message: info.summary(),
            info,
        })
    }

    fn dry_run(
        &self,
        input_path: &str,
        message: &str,
        _password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedInfo, String> {
        let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;
        let (info, _) = analyze(&img, message, intensity, output_path)?;

        if info.message_bytes > info.max_capacity {
            return Err(format!(
                "Message too long: {} bytes, max capacity: {} bytes (mode: {})",
                info.message_bytes, info.max_capacity, info.mode
            ));
        }

        Ok(info)
    }

    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String> {
        let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;

        let gray = channel_to_gray(&img);
        let keypoints = detect_keypoints(&gray, MAX_KEYPOINTS);

        let channel = extract_channel(&img);

        // Try feature-point mode first if enough keypoints
        if keypoints.len() >= MIN_KEYPOINTS {
            let fp_result = verify_feature_point(&gray, &keypoints, password, &channel)?;
            if fp_result.detected {
                return Ok(fp_result);
            }
        }

        // Fall back to global DWT
        verify_global_dwt_from_channel(&channel, password)
    }
}

// ── Feature-Point Pipeline ───────────────────────────────────────────────
//
// Each 64×64 patch is divided into 8×8 = 64 blocks of 8×8 pixels.
// Spread spectrum is applied directly on pixel values (no DWT on patch).
// Same message bits are embedded at EVERY keypoint (inter-patch redundancy).
// Extraction: per-bit majority vote across all keypoints (Level 2 ECC).

/// Encode message to bits for feature-point mode.
/// Format: [1-byte length] + [message bytes], no repetition ECC.
fn fp_encode(message: &[u8]) -> Result<Vec<bool>, String> {
    if message.len() > FP_MAX_MESSAGE {
        return Err(format!(
            "Message too long for feature-point mode: max {} bytes, got {}",
            FP_MAX_MESSAGE,
            message.len()
        ));
    }

    // Build payload: [len_u8, message..., zero_padding to fill all blocks]
    let total_bytes = BLOCKS_PER_PATCH / 8;
    let mut payload = vec![0u8; total_bytes];
    payload[0] = message.len() as u8;
    payload[1..1 + message.len()].copy_from_slice(message);

    let mut bits = Vec::with_capacity(BLOCKS_PER_PATCH);
    for byte in &payload {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }
    Ok(bits)
}

/// Decode bits from feature-point extraction.
fn fp_decode(bits: &[bool]) -> Result<Vec<u8>, String> {
    if bits.len() < 8 {
        return Err("Not enough bits".to_string());
    }

    // Convert bits to bytes
    let mut bytes = Vec::with_capacity(bits.len() / 8);
    for chunk in bits.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let mut byte = 0u8;
        for (i, &bit) in chunk.iter().enumerate() {
            if bit {
                byte |= 1 << (7 - i);
            }
        }
        bytes.push(byte);
    }

    let msg_len = bytes[0] as usize;
    if msg_len == 0 || 1 + msg_len > bytes.len() {
        return Err("Invalid message length".to_string());
    }

    Ok(bytes[1..1 + msg_len].to_vec())
}

fn embed_feature_point(
    img: &DynamicImage,
    _gray: &GrayImage,
    keypoints: &[FeaturePoint],
    message: &str,
    password: &str,
    intensity: u8,
    output_path: &str,
) -> Result<(), String> {
    let alpha = fp_alpha(intensity);
    let (width, height) = img.dimensions();

    let encoded_bits = fp_encode(message.as_bytes())?;
    let seed = password::password_to_seed(password);
    let perm = scramble::generate_permutation(encoded_bits.len(), &seed);
    let scrambled = scramble::scramble(&encoded_bits, &perm);

    let mut channel = extract_channel(img);
    let ch_height = channel.len();
    let ch_width = if ch_height > 0 { channel[0].len() } else { 0 };

    // Embed the SAME scrambled bits at EVERY keypoint directly in the channel.
    // Each bit modifies a PATCH_BLOCK_SIZE × PATCH_BLOCK_SIZE region using
    // spread spectrum. The 64 bits are laid out in an 8×8 grid of 8×8 blocks
    // centered on the keypoint.
    for kp in keypoints {
        let kx = kp.x as i64;
        let ky = kp.y as i64;
        let half = (PATCH_SIZE / 2) as i64;

        for (bit_idx, &bit) in scrambled.iter().enumerate() {
            let pn = generate_patch_pn_chip(&seed, bit_idx);
            let blocks_per_row = PATCH_SIZE / PATCH_BLOCK_SIZE;
            let block_r = bit_idx / blocks_per_row;
            let block_c = bit_idx % blocks_per_row;

            let signal = if bit { 1.0 } else { -1.0 };

            for i in 0..PATCH_BLOCK_SIZE {
                for j in 0..PATCH_BLOCK_SIZE {
                    let py = ky - half + (block_r * PATCH_BLOCK_SIZE + i) as i64;
                    let px = kx - half + (block_c * PATCH_BLOCK_SIZE + j) as i64;

                    if py >= 0 && px >= 0 && (py as usize) < ch_height && (px as usize) < ch_width {
                        let pn_val = pn[i * PATCH_BLOCK_SIZE + j];
                        channel[py as usize][px as usize] += alpha * pn_val * signal;
                    }
                }
            }
        }
    }

    save_channel_to_image(img, &channel, width, height, output_path)?;

    Ok(())
}

fn verify_feature_point(
    _gray: &GrayImage,
    keypoints: &[FeaturePoint],
    password: &str,
    channel: &[Vec<f64>],
) -> Result<ExtractResult, String> {
    let seed = password::password_to_seed(password);
    let ch_height = channel.len();
    let ch_width = if ch_height > 0 { channel[0].len() } else { 0 };

    // Collect votes per bit across all keypoints
    let mut bit_votes_one = vec![0usize; BLOCKS_PER_PATCH];
    let mut bit_votes_total = vec![0usize; BLOCKS_PER_PATCH];
    let mut total_confidence = 0.0;
    let mut num_keypoints_used = 0usize;

    for kp in keypoints {
        let kx = kp.x as i64;
        let ky = kp.y as i64;
        let half = (PATCH_SIZE / 2) as i64;
        let mut kp_confidence = 0.0;

        for bit_idx in 0..BLOCKS_PER_PATCH {
            let pn = generate_patch_pn_chip(&seed, bit_idx);
            let blocks_per_row = PATCH_SIZE / PATCH_BLOCK_SIZE;
            let block_r = bit_idx / blocks_per_row;
            let block_c = bit_idx % blocks_per_row;

            // Collect block pixel values for mean-centering
            let mut block_vals = Vec::with_capacity(PATCH_COEFFS_PER_BLOCK);
            let mut block_pn = Vec::with_capacity(PATCH_COEFFS_PER_BLOCK);

            for i in 0..PATCH_BLOCK_SIZE {
                for j in 0..PATCH_BLOCK_SIZE {
                    let py = ky - half + (block_r * PATCH_BLOCK_SIZE + i) as i64;
                    let px = kx - half + (block_c * PATCH_BLOCK_SIZE + j) as i64;

                    if py >= 0 && px >= 0 && (py as usize) < ch_height && (px as usize) < ch_width {
                        block_vals.push(channel[py as usize][px as usize]);
                        block_pn.push(pn[i * PATCH_BLOCK_SIZE + j]);
                    }
                }
            }

            if !block_vals.is_empty() {
                // Subtract block mean to remove DC component.
                // Original pixels have large positive values that dominate correlation.
                // Mean-centering isolates the watermark signal.
                let mean = block_vals.iter().sum::<f64>() / block_vals.len() as f64;
                let mut correlation = 0.0;
                for (v, p) in block_vals.iter().zip(block_pn.iter()) {
                    correlation += (v - mean) * p;
                }

                let bit = correlation >= 0.0;
                let confidence = (correlation.abs() / block_vals.len() as f64).min(1.0);
                kp_confidence += confidence;

                bit_votes_total[bit_idx] += 1;
                if bit {
                    bit_votes_one[bit_idx] += 1;
                }
            }
        }

        total_confidence += kp_confidence / BLOCKS_PER_PATCH as f64;
        num_keypoints_used += 1;
    }

    if num_keypoints_used == 0 {
        return Ok(no_detection());
    }

    let avg_confidence = total_confidence / num_keypoints_used as f64;

    // Level 2 ECC: majority vote
    let scrambled_bits: Vec<bool> = bit_votes_one
        .iter()
        .zip(bit_votes_total.iter())
        .map(|(&ones, &total)| ones * 2 > total)
        .collect();

    // Unscramble
    let perm = scramble::generate_permutation(scrambled_bits.len(), &seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

    match fp_decode(&bits) {
        Ok(msg_bytes) => match String::from_utf8(msg_bytes) {
            Ok(message) => Ok(ExtractResult {
                detected: true,
                confidence: avg_confidence,
                message: Some(message),
            }),
            Err(_) => Ok(no_detection()),
        },
        Err(_) => Ok(no_detection()),
    }
}

/// Generate a PN chip for a specific bit index within a patch.
/// Uses ChaCha20 seeded by hash(master_seed + bit_idx).
fn generate_patch_pn_chip(seed: &[u8; 32], bit_idx: usize) -> Vec<f64> {
    use rand::Rng;
    use rand::SeedableRng;
    use rand_chacha::ChaCha20Rng;
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(bit_idx.to_le_bytes());
    let hash = hasher.finalize();
    let mut block_seed = [0u8; 32];
    block_seed.copy_from_slice(&hash);

    let mut rng = ChaCha20Rng::from_seed(block_seed);
    (0..PATCH_COEFFS_PER_BLOCK)
        .map(|_| if rng.gen_bool(0.5) { 1.0 } else { -1.0 })
        .collect()
}

// ── Global DWT Fallback ──────────────────────────────────────────────────

fn embed_global_dwt(
    img: &DynamicImage,
    message: &str,
    password: &str,
    intensity: u8,
    output_path: &str,
) -> Result<(), String> {
    let alpha = intensity_to_alpha(intensity);
    let (width, height) = img.dimensions();

    let channel = extract_channel(img);
    let mut coeffs = dwt::forward(&channel);

    let sub_rows = coeffs.hl.len();
    let sub_cols = coeffs.hl[0].len();
    let (blocks_r, blocks_c, num_blocks) = count_blocks(sub_rows, sub_cols);

    if num_blocks == 0 {
        return Err("Image too small for watermarking".to_string());
    }

    let encoded_bits = ecc::encode(message.as_bytes(), num_blocks)?;
    let seed = password::password_to_seed(password);
    let perm = scramble::generate_permutation(encoded_bits.len(), &seed);
    let scrambled = scramble::scramble(&encoded_bits, &perm);

    let mut ctx = TempInputForInference::new(GLOBAL_BLOCK_SIZE);
    ctx.set_seed(seed);

    for (bit_idx, &bit) in scrambled.iter().enumerate() {
        let br = bit_idx / blocks_c;
        let bc = bit_idx % blocks_c;
        if br >= blocks_r {
            break;
        }

        ctx.generate_pn_chip(bit_idx);
        ctx.load_patch(
            &coeffs.hl,
            br * GLOBAL_BLOCK_SIZE,
            bc * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
        ctx.embed_spread_spectrum(0, bit, alpha);
        ctx.store_patch(
            &mut coeffs.hl,
            br * GLOBAL_BLOCK_SIZE,
            bc * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
    }

    let watermarked_channel = dwt::inverse(&coeffs);
    save_channel_to_image(img, &watermarked_channel, width, height, output_path)?;

    Ok(())
}

fn verify_global_dwt_from_channel(
    channel: &[Vec<f64>],
    password: &str,
) -> Result<ExtractResult, String> {
    let coeffs = dwt::forward(channel);

    let sub_rows = coeffs.hl.len();
    let sub_cols = coeffs.hl[0].len();
    let (_blocks_r, blocks_c, num_blocks) = count_blocks(sub_rows, sub_cols);

    if num_blocks == 0 {
        return Ok(no_detection());
    }

    let seed = password::password_to_seed(password);
    let bits_to_extract = ecc::total_encoded_bits(num_blocks);
    if bits_to_extract == 0 {
        return Ok(no_detection());
    }

    let mut ctx = TempInputForInference::new(GLOBAL_BLOCK_SIZE);
    ctx.set_seed(seed);

    let mut scrambled_bits = Vec::with_capacity(bits_to_extract);
    let mut total_confidence = 0.0;
    let blocks_r = sub_rows / GLOBAL_BLOCK_SIZE;

    for bit_idx in 0..bits_to_extract {
        let br = bit_idx / blocks_c;
        let bc = bit_idx % blocks_c;
        if br >= blocks_r {
            break;
        }

        ctx.generate_pn_chip(bit_idx);
        ctx.load_patch(
            &coeffs.hl,
            br * GLOBAL_BLOCK_SIZE,
            bc * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
        let (bit, confidence) = ctx.extract_spread_spectrum(0);
        scrambled_bits.push(bit);
        total_confidence += confidence;
    }

    if scrambled_bits.is_empty() {
        return Ok(no_detection());
    }

    let avg_confidence = total_confidence / scrambled_bits.len() as f64;
    let perm = scramble::generate_permutation(scrambled_bits.len(), &seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

    match ecc::decode(&bits) {
        Ok(msg_bytes) => match String::from_utf8(msg_bytes) {
            Ok(message) => Ok(ExtractResult {
                detected: true,
                confidence: avg_confidence,
                message: Some(message),
            }),
            Err(_) => Ok(no_detection()),
        },
        Err(_) => Ok(no_detection()),
    }
}

// ── Utility Functions ────────────────────────────────────────────────────

fn no_detection() -> ExtractResult {
    ExtractResult {
        detected: false,
        confidence: 0.0,
        message: None,
    }
}

/// Choose intensity automatically based on image dimensions.
/// Smaller images need higher intensity; larger images can use lower intensity
/// for better invisibility.
///
/// | Megapixels | Auto Intensity |
/// |-----------|----------------|
/// | < 0.5 MP  | 7              |
/// | 0.5-2 MP  | 5              |
/// | 2-8 MP    | 4              |
/// | > 8 MP    | 3              |
fn auto_intensity(width: u32, height: u32) -> u8 {
    let megapixels = (width as f64 * height as f64) / 1_000_000.0;
    if megapixels < 0.5 {
        7
    } else if megapixels < 2.0 {
        5
    } else if megapixels < 8.0 {
        4
    } else {
        3
    }
}

/// Resolve intensity: 0 means auto-detect from image dimensions.
fn resolve_intensity(intensity: u8, width: u32, height: u32) -> u8 {
    if intensity == 0 {
        auto_intensity(width, height)
    } else {
        intensity.clamp(1, 10)
    }
}

fn intensity_to_alpha(intensity: u8) -> f64 {
    0.5 + (intensity as f64 - 1.0) * 0.5
}

/// Feature-point mode needs higher alpha because the mean-centered correlation
/// on raw pixels has more noise than DWT-domain embedding.
fn fp_alpha(intensity: u8) -> f64 {
    (intensity_to_alpha(intensity) * 4.0).max(8.0)
}

fn extract_channel(img: &DynamicImage) -> Vec<Vec<f64>> {
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let mut channel = vec![vec![0.0; w as usize]; h as usize];
    for y in 0..h {
        for x in 0..w {
            channel[y as usize][x as usize] = rgba.get_pixel(x, y).0[EMBED_CHANNEL] as f64;
        }
    }
    channel
}

/// Channel used for keypoint detection (red = 0).
/// MUST be different from EMBED_CHANNEL to ensure keypoints are stable
/// after watermark embedding (embedding modifies green, detection uses red).
const DETECT_CHANNEL: usize = 0;

fn channel_to_gray(img: &DynamicImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let mut gray = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            gray.put_pixel(x, y, Luma([rgba.get_pixel(x, y).0[DETECT_CHANNEL]]));
        }
    }
    gray
}

fn count_blocks(subband_rows: usize, subband_cols: usize) -> (usize, usize, usize) {
    let blocks_r = subband_rows / GLOBAL_BLOCK_SIZE;
    let blocks_c = subband_cols / GLOBAL_BLOCK_SIZE;
    (blocks_r, blocks_c, blocks_r * blocks_c)
}

fn save_channel_to_image(
    img: &DynamicImage,
    channel: &[Vec<f64>],
    width: u32,
    height: u32,
    output_path: &str,
) -> Result<(), String> {
    let mut output = img.to_rgba8();
    let out_h = height.min(channel.len() as u32);
    let out_w = width.min(if channel.is_empty() {
        0
    } else {
        channel[0].len() as u32
    });

    for y in 0..out_h {
        for x in 0..out_w {
            let new_val = channel[y as usize][x as usize].round().clamp(0.0, 255.0) as u8;
            let mut pixel = *output.get_pixel(x, y);
            pixel.0[EMBED_CHANNEL] = new_val;
            output.put_pixel(x, y, pixel);
        }
    }

    output
        .save(output_path)
        .map_err(|e| format!("Failed to save image: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intensity_to_alpha_range() {
        assert_eq!(intensity_to_alpha(1), 0.5);
        assert_eq!(intensity_to_alpha(5), 2.5);
        assert_eq!(intensity_to_alpha(10), 5.0);
    }

    #[test]
    fn test_fp_encode_decode_round_trip() {
        let msg = b"Infini";
        let bits = fp_encode(msg).unwrap();
        assert_eq!(bits.len(), BLOCKS_PER_PATCH);
        let decoded = fp_decode(&bits).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn test_fp_max_message() {
        assert_eq!(FP_MAX_MESSAGE, 7); // 64/8 - 1 = 7
        let msg = vec![b'A'; FP_MAX_MESSAGE];
        assert!(fp_encode(&msg).is_ok());
        let msg_too_long = vec![b'A'; FP_MAX_MESSAGE + 1];
        assert!(fp_encode(&msg_too_long).is_err());
    }

    #[test]
    fn test_embed_extract_global_fallback() {
        // Uniform color → zero keypoints → global DWT fallback
        let width = 512u32;
        let height = 512u32;
        let imgbuf = image::RgbImage::from_pixel(width, height, image::Rgb([128u8, 128, 128]));

        let tmp_dir = std::env::temp_dir();
        let input_path = tmp_dir.join("test_raster_global_input.png");
        let output_path = tmp_dir.join("test_raster_global_wm.png");
        imgbuf.save(&input_path).unwrap();

        let engine = RasterEngine;
        let result = engine
            .embed(
                input_path.to_str().unwrap(),
                "Infini",
                "d1ng0",
                5,
                output_path.to_str().unwrap(),
            )
            .unwrap();

        assert!(
            result.info.mode == "global-dwt",
            "Should use global fallback for uniform image, got mode={}",
            result.info.mode
        );

        let extract = engine
            .verify(output_path.to_str().unwrap(), "d1ng0")
            .unwrap();

        assert!(extract.detected, "Watermark not detected");
        assert_eq!(extract.message.as_deref(), Some("Infini"));

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&output_path);
    }

    #[test]
    fn test_wrong_password() {
        let width = 512u32;
        let height = 512u32;
        let imgbuf = image::RgbImage::from_pixel(width, height, image::Rgb([128u8, 128, 128]));

        let tmp_dir = std::env::temp_dir();
        let input_path = tmp_dir.join("test_raster_wp2_input.png");
        let output_path = tmp_dir.join("test_raster_wp2_output.png");
        imgbuf.save(&input_path).unwrap();

        let engine = RasterEngine;
        engine
            .embed(
                input_path.to_str().unwrap(),
                "Infini",
                "d1ng0",
                5,
                output_path.to_str().unwrap(),
            )
            .unwrap();

        let result = engine
            .verify(output_path.to_str().unwrap(), "wrong_d1ng0")
            .unwrap();
        assert!(!result.detected || result.message.as_deref() != Some("Infini"));

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&output_path);
    }
}
