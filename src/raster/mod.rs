pub mod dwt;
pub mod features;

use crate::common::engine::{EmbedInfo, EmbedResult, ExtractResult, WatermarkEngine};
use crate::common::temp_input_for_inference::TempInputForInference;
use crate::common::{ecc, password, scramble};

use features::{detect_keypoints, FeaturePoint, PATCH_SIZE};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};

/// Embedding channel index (green = 1).
const EMBED_CHANNEL: usize = 1;

/// Channel for keypoint detection (red = 0). Must differ from EMBED_CHANNEL.
const DETECT_CHANNEL: usize = 0;

/// Block size for global DWT mode (16×16 blocks in HL subband).
const GLOBAL_BLOCK_SIZE: usize = 16;

/// Block size for feature-point mode (8×8 pixel blocks with mean-centered correlation).
///
/// Design rationale: DWT-on-patch with 4×4 blocks requires exact patch alignment
/// between embed and extract. After cropping, keypoint positions shift and the
/// adapted FAST threshold produces a different keypoint set, breaking alignment.
/// Pixel-domain spread spectrum with mean-centered correlation is inherently
/// position-agnostic per block and survives keypoint re-detection after cropping.
const FP_BLOCK_SIZE: usize = 8;
const FP_COEFFS_PER_BLOCK: usize = FP_BLOCK_SIZE * FP_BLOCK_SIZE;

/// Number of blocks in a 64×64 patch.
const BLOCKS_PER_PATCH: usize = (PATCH_SIZE / FP_BLOCK_SIZE) * (PATCH_SIZE / FP_BLOCK_SIZE);

const MIN_KEYPOINTS: usize = 10;
const MAX_KEYPOINTS: usize = 200;

/// Max message bytes: BLOCKS_PER_PATCH/8 - 1 header = 7 bytes.
const FP_MAX_MESSAGE: usize = BLOCKS_PER_PATCH / 8 - 1;

/// Raster watermark engine with dual-mode embedding.
///
/// **Feature-point mode** (messages ≤ 7 bytes, cropping-resistant):
/// Oriented FAST keypoints from red channel. Per-keypoint spread spectrum
/// in 8×8 pixel blocks of green channel with mean-centered correlation.
/// All PN generation routed through `TempInputForInference`.
/// Inter-patch majority voting (Level 2 ECC) across all keypoints.
///
/// **Global DWT mode** (longer messages):
/// 1-level Haar DWT on full green channel, spread spectrum in HL subband
/// (16×16 blocks), 3× repetition ECC (Level 1). Routed through
/// `TempInputForInference`.
pub struct RasterEngine;

// ── Analysis ─────────────────────────────────────────────────────────────

fn analyze(
    img: &DynamicImage,
    message: &str,
    intensity: u8,
    output_path: &str,
) -> Result<(EmbedInfo, bool), String> {
    let (w, h) = img.dimensions();
    let intensity = resolve_intensity(intensity, w, h);
    let gray = channel_to_gray(img);
    let kps = detect_keypoints(&gray, MAX_KEYPOINTS);
    let use_fp = kps.len() >= MIN_KEYPOINTS && message.len() <= FP_MAX_MESSAGE;

    let (mode, cap) = if use_fp {
        ("feature-point".to_string(), FP_MAX_MESSAGE)
    } else {
        let ch = extract_channel(img);
        let c = dwt::forward(&ch);
        let (_, _, nb) = count_blocks(c.hl.len(), c.hl[0].len());
        ("global-dwt".to_string(), ecc::max_message_bytes(nb))
    };

    Ok((
        EmbedInfo {
            status: "ok".to_string(),
            mode,
            message: message.to_string(),
            message_bytes: message.len(),
            intensity,
            width: w,
            height: h,
            keypoints: kps.len(),
            max_capacity: cap,
            output_path: output_path.to_string(),
        },
        use_fp,
    ))
}

// ── WatermarkEngine ──────────────────────────────────────────────────────

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
        let ri = info.intensity;
        let gray = channel_to_gray(&img);
        let kps = detect_keypoints(&gray, MAX_KEYPOINTS);

        if use_fp {
            embed_feature_point(&img, &kps, message, password, ri, output_path)?;
        } else {
            embed_global_dwt(&img, message, password, ri, output_path)?;
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
        let kps = detect_keypoints(&gray, MAX_KEYPOINTS);
        let channel = extract_channel(&img);

        if kps.len() >= MIN_KEYPOINTS {
            let fp = verify_feature_point(&kps, password, &channel)?;
            if fp.detected {
                return Ok(fp);
            }
        }
        verify_global_dwt_from_channel(&channel, password)
    }
}

// ── In-Memory Buffer API (for VideoEngine) ───────────────────────────────
//
// These methods work on raw RGB byte buffers, avoiding file I/O entirely.
// The file-based WatermarkEngine methods above are NOT modified.

impl RasterEngine {
    /// Embed a watermark into a raw RGB buffer in-place.
    /// `rgb` is width × height × 3 bytes in row-major order.
    /// Modifies the green channel of `rgb` directly.
    pub fn embed_buffer(
        &self,
        rgb: &mut [u8],
        width: u32,
        height: u32,
        message: &str,
        password: &str,
        intensity: u8,
    ) -> Result<(), String> {
        let intensity = resolve_intensity(intensity, width, height);
        let gray = gray_from_rgb(rgb, width, height, DETECT_CHANNEL);
        let kps = detect_keypoints(&gray, MAX_KEYPOINTS);
        let channel = channel_from_rgb(rgb, width, height, EMBED_CHANNEL);

        let use_fp = kps.len() >= MIN_KEYPOINTS && message.len() <= FP_MAX_MESSAGE;

        if use_fp {
            let mut ch = channel;
            let alpha = fp_alpha(intensity);
            let encoded_bits = fp_encode(message.as_bytes())?;
            let seed = password::password_to_seed(password);
            let perm = scramble::generate_permutation(encoded_bits.len(), &seed);
            let scrambled = scramble::scramble(&encoded_bits, &perm);

            let ch_h = ch.len();
            let ch_w = if ch_h > 0 { ch[0].len() } else { 0 };
            let mut ctx = TempInputForInference::with_block_size(FP_BLOCK_SIZE, FP_BLOCK_SIZE);
            ctx.set_seed(seed);
            let half = (PATCH_SIZE / 2) as i64;
            let bpr = PATCH_SIZE / FP_BLOCK_SIZE;

            for kp in &kps {
                let kx = kp.x as i64;
                let ky = kp.y as i64;
                for (bit_idx, &bit) in scrambled.iter().enumerate() {
                    let br = bit_idx / bpr;
                    let bc = bit_idx % bpr;
                    let signal = if bit { 1.0 } else { -1.0 };
                    ctx.generate_pn_chip(bit_idx);
                    let pn = ctx.pn_buffer().to_vec();
                    for i in 0..FP_BLOCK_SIZE {
                        for j in 0..FP_BLOCK_SIZE {
                            let py = ky - half + (br * FP_BLOCK_SIZE + i) as i64;
                            let px = kx - half + (bc * FP_BLOCK_SIZE + j) as i64;
                            if py >= 0 && px >= 0 && (py as usize) < ch_h && (px as usize) < ch_w {
                                ch[py as usize][px as usize] +=
                                    alpha * pn[i * FP_BLOCK_SIZE + j] * signal;
                            }
                        }
                    }
                }
            }
            write_channel_to_rgb(rgb, &ch, width, height, EMBED_CHANNEL);
        } else {
            // Global DWT mode — fully in-memory, no temp files
            let mut coeffs = dwt::forward(&channel);
            let (br, bc, nb) = count_blocks(coeffs.hl.len(), coeffs.hl[0].len());
            if nb == 0 {
                return Err("Image too small for watermarking".to_string());
            }
            let bits = ecc::encode(message.as_bytes(), nb)?;
            let seed = password::password_to_seed(password);
            let perm = scramble::generate_permutation(bits.len(), &seed);
            let scrambled = scramble::scramble(&bits, &perm);
            let alpha = intensity_to_alpha(intensity);
            let mut ctx = TempInputForInference::new(GLOBAL_BLOCK_SIZE);
            ctx.set_seed(seed);
            for (i, &bit) in scrambled.iter().enumerate() {
                let r = i / bc;
                let c = i % bc;
                if r >= br {
                    break;
                }
                ctx.generate_pn_chip(i);
                ctx.load_patch(
                    &coeffs.hl,
                    r * GLOBAL_BLOCK_SIZE,
                    c * GLOBAL_BLOCK_SIZE,
                    GLOBAL_BLOCK_SIZE,
                    GLOBAL_BLOCK_SIZE,
                );
                ctx.embed_spread_spectrum(0, bit, alpha);
                ctx.store_patch(
                    &mut coeffs.hl,
                    r * GLOBAL_BLOCK_SIZE,
                    c * GLOBAL_BLOCK_SIZE,
                    GLOBAL_BLOCK_SIZE,
                    GLOBAL_BLOCK_SIZE,
                );
            }
            let wm = dwt::inverse(&coeffs);
            write_channel_to_rgb(rgb, &wm, width, height, EMBED_CHANNEL);
        }
        Ok(())
    }

    /// Verify/extract a watermark from a raw RGB buffer.
    pub fn verify_buffer(
        &self,
        rgb: &[u8],
        width: u32,
        height: u32,
        password: &str,
    ) -> Result<ExtractResult, String> {
        let gray = gray_from_rgb(rgb, width, height, DETECT_CHANNEL);
        let kps = detect_keypoints(&gray, MAX_KEYPOINTS);
        let channel = channel_from_rgb(rgb, width, height, EMBED_CHANNEL);

        if kps.len() >= MIN_KEYPOINTS {
            let fp = verify_feature_point(&kps, password, &channel)?;
            if fp.detected {
                return Ok(fp);
            }
        }
        verify_global_dwt_from_channel(&channel, password)
    }
}

/// Extract a single channel from raw RGB bytes as f64 matrix.
fn channel_from_rgb(rgb: &[u8], w: u32, h: u32, ch_idx: usize) -> Vec<Vec<f64>> {
    let mut ch = vec![vec![0.0; w as usize]; h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            ch[y][x] = rgb[(y * w as usize + x) * 3 + ch_idx] as f64;
        }
    }
    ch
}

/// Build a GrayImage from a specific channel of raw RGB bytes.
fn gray_from_rgb(rgb: &[u8], w: u32, h: u32, ch_idx: usize) -> GrayImage {
    let mut g = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 3) as usize + ch_idx;
            g.put_pixel(x, y, Luma([rgb[idx]]));
        }
    }
    g
}

/// Write a modified f64 channel back to raw RGB bytes.
fn write_channel_to_rgb(rgb: &mut [u8], ch: &[Vec<f64>], w: u32, h: u32, ch_idx: usize) {
    let w = w as usize;
    for (y, row) in ch.iter().enumerate().take(h as usize) {
        for (x, val) in row.iter().enumerate().take(w) {
            rgb[(y * w + x) * 3 + ch_idx] = val.round().clamp(0.0, 255.0) as u8;
        }
    }
}

// ── Feature-Point Pipeline ───────────────────────────────────────────────
//
// Per keypoint:
//   1. Generate PN chips via TempInputForInference (8×8 = 64 coefficients each)
//   2. Embed: add alpha * pn * signal to 8×8 pixel blocks in green channel
//   3. Extract: mean-centered correlation on 8×8 blocks (subtracts DC to
//      isolate watermark from host pixel values)
//   4. Level 2 ECC: majority vote across all keypoints

fn fp_encode(message: &[u8]) -> Result<Vec<bool>, String> {
    if message.len() > FP_MAX_MESSAGE {
        return Err(format!(
            "Message too long for feature-point mode: max {} bytes, got {}",
            FP_MAX_MESSAGE,
            message.len()
        ));
    }
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

fn fp_decode(bits: &[bool]) -> Result<Vec<u8>, String> {
    if bits.len() < 8 {
        return Err("Not enough bits".to_string());
    }
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
    let len = bytes[0] as usize;
    if len == 0 || 1 + len > bytes.len() {
        return Err("Invalid message length".to_string());
    }
    Ok(bytes[1..1 + len].to_vec())
}

fn embed_feature_point(
    img: &DynamicImage,
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
    let ch_h = channel.len();
    let ch_w = if ch_h > 0 { channel[0].len() } else { 0 };

    // All PN generation routed through TempInputForInference
    let mut ctx = TempInputForInference::with_block_size(FP_BLOCK_SIZE, FP_BLOCK_SIZE);
    ctx.set_seed(seed);

    let half = (PATCH_SIZE / 2) as i64;
    let bpr = PATCH_SIZE / FP_BLOCK_SIZE;

    for kp in keypoints {
        let kx = kp.x as i64;
        let ky = kp.y as i64;

        for (bit_idx, &bit) in scrambled.iter().enumerate() {
            let br = bit_idx / bpr;
            let bc = bit_idx % bpr;
            let signal = if bit { 1.0 } else { -1.0 };

            ctx.generate_pn_chip(bit_idx);
            let pn = ctx.pn_buffer().to_vec();

            for i in 0..FP_BLOCK_SIZE {
                for j in 0..FP_BLOCK_SIZE {
                    let py = ky - half + (br * FP_BLOCK_SIZE + i) as i64;
                    let px = kx - half + (bc * FP_BLOCK_SIZE + j) as i64;
                    if py >= 0 && px >= 0 && (py as usize) < ch_h && (px as usize) < ch_w {
                        channel[py as usize][px as usize] +=
                            alpha * pn[i * FP_BLOCK_SIZE + j] * signal;
                    }
                }
            }
        }
    }

    save_channel_to_image(img, &channel, width, height, output_path)
}

fn verify_feature_point(
    keypoints: &[FeaturePoint],
    password: &str,
    channel: &[Vec<f64>],
) -> Result<ExtractResult, String> {
    let seed = password::password_to_seed(password);
    let ch_h = channel.len();
    let ch_w = if ch_h > 0 { channel[0].len() } else { 0 };

    let mut bit_votes_one = vec![0usize; BLOCKS_PER_PATCH];
    let mut bit_votes_total = vec![0usize; BLOCKS_PER_PATCH];
    let mut total_conf = 0.0;
    let mut num_used = 0usize;

    let mut ctx = TempInputForInference::with_block_size(FP_BLOCK_SIZE, FP_BLOCK_SIZE);
    ctx.set_seed(seed);

    let half = (PATCH_SIZE / 2) as i64;
    let bpr = PATCH_SIZE / FP_BLOCK_SIZE;

    for kp in keypoints {
        let kx = kp.x as i64;
        let ky = kp.y as i64;
        let mut kp_conf = 0.0;

        for bit_idx in 0..BLOCKS_PER_PATCH {
            let br = bit_idx / bpr;
            let bc = bit_idx % bpr;

            ctx.generate_pn_chip(bit_idx);
            let pn = ctx.pn_buffer().to_vec();

            // Collect block values for mean-centered correlation
            let mut vals = Vec::with_capacity(FP_COEFFS_PER_BLOCK);
            let mut pn_vals = Vec::with_capacity(FP_COEFFS_PER_BLOCK);

            for i in 0..FP_BLOCK_SIZE {
                for j in 0..FP_BLOCK_SIZE {
                    let py = ky - half + (br * FP_BLOCK_SIZE + i) as i64;
                    let px = kx - half + (bc * FP_BLOCK_SIZE + j) as i64;
                    if py >= 0 && px >= 0 && (py as usize) < ch_h && (px as usize) < ch_w {
                        vals.push(channel[py as usize][px as usize]);
                        pn_vals.push(pn[i * FP_BLOCK_SIZE + j]);
                    }
                }
            }

            if !vals.is_empty() {
                // Mean-center: subtract DC component to isolate watermark signal
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                let mut corr = 0.0;
                for (v, p) in vals.iter().zip(pn_vals.iter()) {
                    corr += (v - mean) * p;
                }
                let bit = corr >= 0.0;
                let conf = (corr.abs() / vals.len() as f64).min(1.0);
                kp_conf += conf;

                bit_votes_total[bit_idx] += 1;
                if bit {
                    bit_votes_one[bit_idx] += 1;
                }
            }
        }

        total_conf += kp_conf / BLOCKS_PER_PATCH as f64;
        num_used += 1;
    }

    if num_used == 0 {
        return Ok(no_detection());
    }

    let avg_conf = total_conf / num_used as f64;

    // Level 2 ECC: per-bit majority vote across all keypoints
    let scrambled_bits: Vec<bool> = bit_votes_one
        .iter()
        .zip(bit_votes_total.iter())
        .map(|(&ones, &total)| ones * 2 > total)
        .collect();

    let perm = scramble::generate_permutation(scrambled_bits.len(), &seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

    match fp_decode(&bits) {
        Ok(msg_bytes) => match String::from_utf8(msg_bytes) {
            Ok(m) => Ok(ExtractResult {
                detected: true,
                confidence: avg_conf,
                message: Some(m),
            }),
            Err(_) => Ok(no_detection()),
        },
        Err(_) => Ok(no_detection()),
    }
}

// ── Global DWT Mode ──────────────────────────────────────────────────────

fn embed_global_dwt(
    img: &DynamicImage,
    message: &str,
    password: &str,
    intensity: u8,
    output_path: &str,
) -> Result<(), String> {
    let alpha = intensity_to_alpha(intensity);
    let (w, h) = img.dimensions();
    let ch = extract_channel(img);
    let mut coeffs = dwt::forward(&ch);
    let (br, bc, nb) = count_blocks(coeffs.hl.len(), coeffs.hl[0].len());
    if nb == 0 {
        return Err("Image too small for watermarking".to_string());
    }

    let bits = ecc::encode(message.as_bytes(), nb)?;
    let seed = password::password_to_seed(password);
    let perm = scramble::generate_permutation(bits.len(), &seed);
    let scrambled = scramble::scramble(&bits, &perm);

    let mut ctx = TempInputForInference::new(GLOBAL_BLOCK_SIZE);
    ctx.set_seed(seed);

    for (i, &bit) in scrambled.iter().enumerate() {
        let r = i / bc;
        let c = i % bc;
        if r >= br {
            break;
        }
        ctx.generate_pn_chip(i);
        ctx.load_patch(
            &coeffs.hl,
            r * GLOBAL_BLOCK_SIZE,
            c * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
        ctx.embed_spread_spectrum(0, bit, alpha);
        ctx.store_patch(
            &mut coeffs.hl,
            r * GLOBAL_BLOCK_SIZE,
            c * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
    }

    let wm = dwt::inverse(&coeffs);
    save_channel_to_image(img, &wm, w, h, output_path)
}

fn verify_global_dwt_from_channel(
    channel: &[Vec<f64>],
    password: &str,
) -> Result<ExtractResult, String> {
    let coeffs = dwt::forward(channel);
    let (_, bc, nb) = count_blocks(coeffs.hl.len(), coeffs.hl[0].len());
    if nb == 0 {
        return Ok(no_detection());
    }

    let seed = password::password_to_seed(password);
    let total = ecc::total_encoded_bits(nb);
    if total == 0 {
        return Ok(no_detection());
    }

    let mut ctx = TempInputForInference::new(GLOBAL_BLOCK_SIZE);
    ctx.set_seed(seed);
    let br = coeffs.hl.len() / GLOBAL_BLOCK_SIZE;

    let mut scrambled = Vec::with_capacity(total);
    let mut sum_conf = 0.0;

    for i in 0..total {
        let r = i / bc;
        let c = i % bc;
        if r >= br {
            break;
        }
        ctx.generate_pn_chip(i);
        ctx.load_patch(
            &coeffs.hl,
            r * GLOBAL_BLOCK_SIZE,
            c * GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
            GLOBAL_BLOCK_SIZE,
        );
        let (bit, conf) = ctx.extract_spread_spectrum(0);
        scrambled.push(bit);
        sum_conf += conf;
    }

    if scrambled.is_empty() {
        return Ok(no_detection());
    }

    let avg = sum_conf / scrambled.len() as f64;
    let perm = scramble::generate_permutation(scrambled.len(), &seed);
    let bits = scramble::unscramble(&scrambled, &perm);

    match ecc::decode(&bits) {
        Ok(msg) => match String::from_utf8(msg) {
            Ok(m) => Ok(ExtractResult {
                detected: true,
                confidence: avg,
                message: Some(m),
            }),
            Err(_) => Ok(no_detection()),
        },
        Err(_) => Ok(no_detection()),
    }
}

// ── Utilities ────────────────────────────────────────────────────────────

fn no_detection() -> ExtractResult {
    ExtractResult {
        detected: false,
        confidence: 0.0,
        message: None,
    }
}

fn auto_intensity(w: u32, h: u32) -> u8 {
    let mp = (w as f64 * h as f64) / 1_000_000.0;
    if mp < 0.5 {
        4
    } else if mp < 2.0 {
        4
    } else if mp < 8.0 {
        3
    } else {
        3
    }
}

fn resolve_intensity(i: u8, w: u32, h: u32) -> u8 {
    if i == 0 {
        auto_intensity(w, h)
    } else {
        i.clamp(1, 10)
    }
}

fn intensity_to_alpha(i: u8) -> f64 {
    0.5 + (i as f64 - 1.0) * 0.5
}

/// Feature-point alpha. Pixel-domain spread spectrum with mean-centered
/// correlation needs higher alpha than DWT-domain (which has zero-mean HL).
fn fp_alpha(i: u8) -> f64 {
    (intensity_to_alpha(i) * 4.0).max(8.0)
}

fn extract_channel(img: &DynamicImage) -> Vec<Vec<f64>> {
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let mut ch = vec![vec![0.0; w as usize]; h as usize];
    for y in 0..h {
        for x in 0..w {
            ch[y as usize][x as usize] = rgba.get_pixel(x, y).0[EMBED_CHANNEL] as f64;
        }
    }
    ch
}

fn channel_to_gray(img: &DynamicImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let mut g = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            g.put_pixel(x, y, Luma([rgba.get_pixel(x, y).0[DETECT_CHANNEL]]));
        }
    }
    g
}

fn count_blocks(rows: usize, cols: usize) -> (usize, usize, usize) {
    let r = rows / GLOBAL_BLOCK_SIZE;
    let c = cols / GLOBAL_BLOCK_SIZE;
    (r, c, r * c)
}

fn save_channel_to_image(
    img: &DynamicImage,
    channel: &[Vec<f64>],
    width: u32,
    height: u32,
    path: &str,
) -> Result<(), String> {
    let mut out = img.to_rgba8();
    let oh = height.min(channel.len() as u32);
    let ow = width.min(if channel.is_empty() {
        0
    } else {
        channel[0].len() as u32
    });
    for y in 0..oh {
        for x in 0..ow {
            let v = channel[y as usize][x as usize].round().clamp(0.0, 255.0) as u8;
            let mut px = *out.get_pixel(x, y);
            px.0[EMBED_CHANNEL] = v;
            out.put_pixel(x, y, px);
        }
    }
    out.save(path)
        .map_err(|e| format!("Failed to save image: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fp_encode_decode() {
        let msg = b"Infini";
        let bits = fp_encode(msg).unwrap();
        assert_eq!(bits.len(), BLOCKS_PER_PATCH);
        assert_eq!(fp_decode(&bits).unwrap(), msg);
    }

    #[test]
    fn test_fp_max_message() {
        assert_eq!(FP_MAX_MESSAGE, 7);
        assert!(fp_encode(&vec![b'A'; 7]).is_ok());
        assert!(fp_encode(&vec![b'A'; 8]).is_err());
    }

    #[test]
    fn test_embed_extract_global_fallback() {
        let imgbuf = image::RgbImage::from_pixel(512, 512, image::Rgb([128u8, 128, 128]));
        let tmp = std::env::temp_dir();
        let inp = tmp.join("test_g_in.png");
        let out = tmp.join("test_g_wm.png");
        imgbuf.save(&inp).unwrap();

        let e = RasterEngine;
        let r = e
            .embed(
                inp.to_str().unwrap(),
                "Infini",
                "d1ng0",
                5,
                out.to_str().unwrap(),
            )
            .unwrap();
        assert_eq!(r.info.mode, "global-dwt");

        let v = e.verify(out.to_str().unwrap(), "d1ng0").unwrap();
        assert!(v.detected);
        assert_eq!(v.message.as_deref(), Some("Infini"));

        let _ = std::fs::remove_file(&inp);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn test_wrong_password() {
        let imgbuf = image::RgbImage::from_pixel(512, 512, image::Rgb([128u8, 128, 128]));
        let tmp = std::env::temp_dir();
        let inp = tmp.join("test_w_in.png");
        let out = tmp.join("test_w_out.png");
        imgbuf.save(&inp).unwrap();

        let e = RasterEngine;
        e.embed(
            inp.to_str().unwrap(),
            "Infini",
            "d1ng0",
            5,
            out.to_str().unwrap(),
        )
        .unwrap();
        let v = e.verify(out.to_str().unwrap(), "wrong_d1ng0").unwrap();
        assert!(!v.detected || v.message.as_deref() != Some("Infini"));

        let _ = std::fs::remove_file(&inp);
        let _ = std::fs::remove_file(&out);
    }
}
