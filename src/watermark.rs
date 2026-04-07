use crate::dwt;
use crate::ecc;
use crate::scramble;

use image::{DynamicImage, GenericImageView};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};

const BLOCK_SIZE: usize = 16;
const COEFFS_PER_BLOCK: usize = BLOCK_SIZE * BLOCK_SIZE;

/// Result of a successful watermark embedding.
#[derive(Debug)]
pub struct EmbedResult {
    /// Human-readable status message.
    pub message: String,
}

/// Result of a watermark verification/extraction attempt.
#[derive(Debug)]
pub struct ExtractResult {
    /// Whether a valid watermark was detected.
    pub detected: bool,
    /// Detection confidence (0.0 to 1.0).
    pub confidence: f64,
    /// Extracted message, if decoding succeeded.
    pub message: Option<String>,
}

/// Hash the password into a 32-byte seed for the ChaCha20 PRNG.
fn password_to_seed(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&result);
    seed
}

/// Map intensity (1-10) to embedding strength alpha.
/// Higher alpha = more robust but slightly more visible.
fn intensity_to_alpha(intensity: u8) -> f64 {
    0.5 + (intensity as f64 - 1.0) * 0.5
}

/// Extract a single color channel from an image as an f64 matrix.
/// Uses the green channel (index 1) as it carries the most perceptual
/// information and avoids luma conversion rounding errors.
const EMBED_CHANNEL: usize = 1; // green

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

/// Count available 16x16 blocks in the embedding subband.
fn count_blocks(subband_rows: usize, subband_cols: usize) -> (usize, usize, usize) {
    let blocks_r = subband_rows / BLOCK_SIZE;
    let blocks_c = subband_cols / BLOCK_SIZE;
    (blocks_r, blocks_c, blocks_r * blocks_c)
}

/// Generate a pseudo-random PN (pseudo-noise) chip sequence of ±1 values
/// for a specific block, seeded by the password and block index.
fn generate_pn_chip(seed: &[u8; 32], block_idx: usize) -> Vec<f64> {
    // Derive a per-block seed by hashing the master seed with the block index
    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(block_idx.to_le_bytes());
    let block_seed_hash = hasher.finalize();
    let mut block_seed = [0u8; 32];
    block_seed.copy_from_slice(&block_seed_hash);

    let mut rng = ChaCha20Rng::from_seed(block_seed);
    (0..COEFFS_PER_BLOCK)
        .map(|_| if rng.gen_bool(0.5) { 1.0 } else { -1.0 })
        .collect()
}

/// Embed a watermark bit into a 16x16 block of DWT coefficients
/// using additive spread spectrum.
///
/// Each coefficient is modified: coeff += alpha * pn * signal
/// where signal = +1 for bit=1, -1 for bit=0.
fn embed_block(
    subband: &mut [Vec<f64>],
    block_r: usize,
    block_c: usize,
    bit: bool,
    alpha: f64,
    pn: &[f64],
) {
    let signal = if bit { 1.0 } else { -1.0 };
    for i in 0..BLOCK_SIZE {
        for j in 0..BLOCK_SIZE {
            let r = block_r * BLOCK_SIZE + i;
            let c = block_c * BLOCK_SIZE + j;
            subband[r][c] += alpha * pn[i * BLOCK_SIZE + j] * signal;
        }
    }
}

/// Extract a watermark bit from a 16x16 block by correlating with the PN chip.
///
/// Returns (bit, confidence) where confidence is normalized correlation strength.
fn extract_block_bit(
    subband: &[Vec<f64>],
    block_r: usize,
    block_c: usize,
    pn: &[f64],
) -> (bool, f64) {
    let mut correlation = 0.0;
    for i in 0..BLOCK_SIZE {
        for j in 0..BLOCK_SIZE {
            let r = block_r * BLOCK_SIZE + i;
            let c = block_c * BLOCK_SIZE + j;
            correlation += subband[r][c] * pn[i * BLOCK_SIZE + j];
        }
    }
    let bit = correlation >= 0.0;
    // Normalize confidence: |correlation| / num_coefficients
    // Perfect embedding gives correlation = alpha * COEFFS_PER_BLOCK
    let confidence = (correlation.abs() / COEFFS_PER_BLOCK as f64).min(1.0);
    (bit, confidence)
}

/// Embed a watermark message into an image.
///
/// Uses DWT + spread spectrum with ChaCha20-based bit scrambling.
pub fn embed(
    input_path: &str,
    message: &str,
    password: &str,
    intensity: u8,
    output_path: &str,
) -> Result<EmbedResult, String> {
    let intensity = intensity.clamp(1, 10);

    // Load image
    let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;
    let (width, height) = img.dimensions();

    // Extract the embedding channel
    let channel = extract_channel(&img);

    // Forward DWT
    let mut coeffs = dwt::forward(&channel);

    // Compute capacity using the HL subband (vertical detail)
    let sub_rows = coeffs.hl.len();
    let sub_cols = coeffs.hl[0].len();
    let (blocks_r, blocks_c, num_blocks) = count_blocks(sub_rows, sub_cols);

    if num_blocks == 0 {
        return Err("Image too small for watermarking".to_string());
    }

    // Encode message with ECC (padded to full capacity)
    let encoded_bits = ecc::encode(message.as_bytes(), num_blocks)?;

    // Scramble bits
    let seed = password_to_seed(password);
    let perm = scramble::generate_permutation(encoded_bits.len(), &seed);
    let scrambled = scramble::scramble(&encoded_bits, &perm);

    // Embed using spread spectrum on DWT coefficients
    let alpha = intensity_to_alpha(intensity);
    for (bit_idx, &bit) in scrambled.iter().enumerate() {
        let br = bit_idx / blocks_c;
        let bc = bit_idx % blocks_c;

        if br >= blocks_r {
            break;
        }

        let pn = generate_pn_chip(&seed, bit_idx);
        embed_block(&mut coeffs.hl, br, bc, bit, alpha, &pn);
    }

    // Inverse DWT
    let watermarked_channel = dwt::inverse(&coeffs);

    // Apply modified channel to the output image
    let mut output = img.to_rgba8();
    let out_h = height.min(watermarked_channel.len() as u32);
    let out_w = width.min(if watermarked_channel.is_empty() {
        0
    } else {
        watermarked_channel[0].len() as u32
    });

    for y in 0..out_h {
        for x in 0..out_w {
            let new_val = watermarked_channel[y as usize][x as usize]
                .round()
                .clamp(0.0, 255.0) as u8;
            let mut pixel = *output.get_pixel(x, y);
            pixel.0[EMBED_CHANNEL] = new_val;
            output.put_pixel(x, y, pixel);
        }
    }

    // Save as PNG
    output
        .save(output_path)
        .map_err(|e| format!("Failed to save image: {}", e))?;

    Ok(EmbedResult {
        message: format!(
            "[成功] 水印已嵌入。输出文件: {}。预计抗压缩率: {}。",
            output_path,
            match intensity {
                1..=3 => "低",
                4..=7 => "中",
                _ => "高",
            }
        ),
    })
}

/// Verify and extract a watermark from an image.
///
/// Tries all 10 intensity levels to find a valid watermark.
pub fn verify(input_path: &str, password: &str) -> Result<ExtractResult, String> {
    let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;
    let channel = extract_channel(&img);
    let coeffs = dwt::forward(&channel);

    let sub_rows = coeffs.hl.len();
    let sub_cols = coeffs.hl[0].len();
    let (_blocks_r, blocks_c, num_blocks) = count_blocks(sub_rows, sub_cols);

    if num_blocks == 0 {
        return Ok(ExtractResult {
            detected: false,
            confidence: 0.0,
            message: None,
        });
    }

    let seed = password_to_seed(password);

    // Spread spectrum extraction is intensity-independent for the bit decision
    // (correlation sign doesn't depend on alpha). We extract once and try decoding.
    let result = try_extract(&coeffs.hl, blocks_c, num_blocks, &seed);

    Ok(result.unwrap_or(ExtractResult {
        detected: false,
        confidence: 0.0,
        message: None,
    }))
}

/// Attempt extraction using spread spectrum correlation.
fn try_extract(
    hl_subband: &[Vec<f64>],
    blocks_c: usize,
    num_blocks: usize,
    seed: &[u8; 32],
) -> Option<ExtractResult> {
    let bits_to_extract = ecc::total_encoded_bits(num_blocks);
    if bits_to_extract == 0 {
        return None;
    }

    let mut scrambled_bits = Vec::with_capacity(bits_to_extract);
    let mut total_confidence = 0.0;

    let blocks_r = hl_subband.len() / BLOCK_SIZE;

    for bit_idx in 0..bits_to_extract {
        let br = bit_idx / blocks_c;
        let bc = bit_idx % blocks_c;

        if br >= blocks_r {
            break;
        }

        let pn = generate_pn_chip(seed, bit_idx);
        let (bit, confidence) = extract_block_bit(hl_subband, br, bc, &pn);
        scrambled_bits.push(bit);
        total_confidence += confidence;
    }

    if scrambled_bits.is_empty() {
        return None;
    }

    let avg_confidence = total_confidence / scrambled_bits.len() as f64;

    // Unscramble
    let perm = scramble::generate_permutation(scrambled_bits.len(), seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

    // Try ECC decode
    match ecc::decode(&bits) {
        Ok(msg_bytes) => match String::from_utf8(msg_bytes) {
            Ok(message) => Some(ExtractResult {
                detected: true,
                confidence: avg_confidence,
                message: Some(message),
            }),
            Err(_) => None,
        },
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_spectrum_round_trip() {
        // Create a 16x16 block of random-ish coefficients
        let seed = [42u8; 32];
        let mut subband = vec![vec![0.0; 16]; 16];
        for i in 0..16 {
            for j in 0..16 {
                subband[i][j] = (i * 16 + j) as f64 * 0.5 - 64.0;
            }
        }

        for bit in [true, false] {
            let mut test_sub = subband.clone();
            let pn = generate_pn_chip(&seed, 0);
            embed_block(&mut test_sub, 0, 0, bit, 2.0, &pn);
            let (extracted, confidence) = extract_block_bit(&test_sub, 0, 0, &pn);
            assert_eq!(extracted, bit, "Spread spectrum failed for bit={}", bit);
            assert!(
                confidence > 0.1,
                "Low confidence {} for bit={}",
                confidence,
                bit
            );
        }
    }

    #[test]
    fn test_password_to_seed_deterministic() {
        let s1 = password_to_seed("test123");
        let s2 = password_to_seed("test123");
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_password_to_seed_different() {
        let s1 = password_to_seed("password1");
        let s2 = password_to_seed("password2");
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_intensity_to_alpha_range() {
        assert_eq!(intensity_to_alpha(1), 0.5);
        assert_eq!(intensity_to_alpha(5), 2.5);
        assert_eq!(intensity_to_alpha(10), 5.0);
    }

    #[test]
    fn test_embed_extract_round_trip() {
        let width = 512u32;
        let height = 512u32;
        let mut imgbuf = image::RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let v = ((x as f64 / width as f64) * 200.0 + 30.0) as u8;
                imgbuf.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }

        let tmp_dir = std::env::temp_dir();
        let input_path = tmp_dir.join("test_ss_input.png");
        let output_path = tmp_dir.join("test_ss_watermarked.png");

        imgbuf.save(&input_path).unwrap();

        let message = "Infini";
        let password = "d1ng0";

        let result = embed(
            input_path.to_str().unwrap(),
            message,
            password,
            5,
            output_path.to_str().unwrap(),
        )
        .unwrap();

        assert!(result.message.contains("成功"));

        let extract_result = verify(output_path.to_str().unwrap(), password).unwrap();

        assert!(
            extract_result.detected,
            "Watermark not detected. Confidence: {}",
            extract_result.confidence
        );
        assert_eq!(
            extract_result.message.as_deref(),
            Some(message),
            "Extracted message mismatch"
        );

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&output_path);
    }

    #[test]
    fn test_wrong_password() {
        let width = 512u32;
        let height = 512u32;
        let mut imgbuf = image::RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let v = ((x as f64 / width as f64) * 200.0 + 30.0) as u8;
                imgbuf.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }

        let tmp_dir = std::env::temp_dir();
        let input_path = tmp_dir.join("test_ss_wrong_pw_input.png");
        let output_path = tmp_dir.join("test_ss_wrong_pw_output.png");

        imgbuf.save(&input_path).unwrap();

        embed(
            input_path.to_str().unwrap(),
            "Infini",
            "d1ng0",
            5,
            output_path.to_str().unwrap(),
        )
        .unwrap();

        let result = verify(output_path.to_str().unwrap(), "wrong_d1ng0").unwrap();
        assert!(!result.detected || result.message.as_deref() != Some("Infini"));

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&output_path);
    }
}
