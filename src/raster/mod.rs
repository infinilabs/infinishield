pub mod dwt;

use crate::common::engine::{EmbedResult, ExtractResult, WatermarkEngine};
use crate::common::temp_input_for_inference::{TempInputForInference, BLOCK_SIZE};
use crate::common::{ecc, password, scramble};

use image::{DynamicImage, GenericImageView};

/// Embedding channel index (green = 1).
const EMBED_CHANNEL: usize = 1;

/// The raster watermark engine for JPEG/PNG/WebP images.
///
/// Uses global DWT + spread spectrum embedding via `TempInputForInference`.
/// All intermediate data is routed exclusively through the managed inference buffer.
pub struct RasterEngine;

impl WatermarkEngine for RasterEngine {
    fn embed(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedResult, String> {
        let intensity = intensity.clamp(1, 10);
        let alpha = intensity_to_alpha(intensity);

        let img = image::open(input_path).map_err(|e| format!("Failed to open image: {}", e))?;
        let (width, height) = img.dimensions();

        let channel = extract_channel(&img);
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

        // All inference routed through temp_input_for_inference
        let mut ctx = TempInputForInference::new(BLOCK_SIZE);
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
                br * BLOCK_SIZE,
                bc * BLOCK_SIZE,
                BLOCK_SIZE,
                BLOCK_SIZE,
            );
            ctx.embed_spread_spectrum(0, bit, alpha);
            ctx.store_patch(
                &mut coeffs.hl,
                br * BLOCK_SIZE,
                bc * BLOCK_SIZE,
                BLOCK_SIZE,
                BLOCK_SIZE,
            );
        }

        let watermarked_channel = dwt::inverse(&coeffs);

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

    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String> {
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

        let seed = password::password_to_seed(password);
        let result = try_extract(&coeffs.hl, blocks_c, num_blocks, &seed);

        Ok(result.unwrap_or(ExtractResult {
            detected: false,
            confidence: 0.0,
            message: None,
        }))
    }
}

/// Map intensity (1-10) to embedding strength alpha.
fn intensity_to_alpha(intensity: u8) -> f64 {
    0.5 + (intensity as f64 - 1.0) * 0.5
}

/// Extract the green channel from an image as f64 matrix.
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

/// Count available blocks in a subband.
fn count_blocks(subband_rows: usize, subband_cols: usize) -> (usize, usize, usize) {
    let blocks_r = subband_rows / BLOCK_SIZE;
    let blocks_c = subband_cols / BLOCK_SIZE;
    (blocks_r, blocks_c, blocks_r * blocks_c)
}

/// Attempt extraction using spread spectrum correlation via temp_input_for_inference.
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

    let mut ctx = TempInputForInference::new(BLOCK_SIZE);
    ctx.set_seed(*seed);

    let mut scrambled_bits = Vec::with_capacity(bits_to_extract);
    let mut total_confidence = 0.0;
    let blocks_r = hl_subband.len() / BLOCK_SIZE;

    for bit_idx in 0..bits_to_extract {
        let br = bit_idx / blocks_c;
        let bc = bit_idx % blocks_c;
        if br >= blocks_r {
            break;
        }

        ctx.generate_pn_chip(bit_idx);
        ctx.load_patch(
            hl_subband,
            br * BLOCK_SIZE,
            bc * BLOCK_SIZE,
            BLOCK_SIZE,
            BLOCK_SIZE,
        );
        let (bit, confidence) = ctx.extract_spread_spectrum(0);
        scrambled_bits.push(bit);
        total_confidence += confidence;
    }

    if scrambled_bits.is_empty() {
        return None;
    }

    let avg_confidence = total_confidence / scrambled_bits.len() as f64;

    let perm = scramble::generate_permutation(scrambled_bits.len(), seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

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
        let input_path = tmp_dir.join("test_raster_input.png");
        let output_path = tmp_dir.join("test_raster_watermarked.png");
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

        assert!(result.message.contains("成功"));

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
        let mut imgbuf = image::RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let v = ((x as f64 / width as f64) * 200.0 + 30.0) as u8;
                imgbuf.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }

        let tmp_dir = std::env::temp_dir();
        let input_path = tmp_dir.join("test_raster_wp_input.png");
        let output_path = tmp_dir.join("test_raster_wp_output.png");
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
