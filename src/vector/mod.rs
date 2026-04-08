//! Vector engine (SVG) — coordinate QIM watermarking.
//!
//! Embeds watermark bits by quantizing coordinate values in SVG path `d` attributes.
//! SVG is lossless text, so QIM-encoded coordinates survive save/reload exactly.
//! Same watermark in every qualifying path (inter-path voting for redundancy).
//!
//! Capacity is determined by message size, not path structure. This ensures the
//! scramble permutation is stable even if paths are deleted.

use crate::common::engine::{EmbedInfo, EmbedResult, ExtractResult, WatermarkEngine};
use crate::common::{password, scramble};

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};

/// QIM quantization step — power of 2 to avoid IEEE 754 precision issues.
const Q: f64 = 0.0625;

/// Max coordinates to use from each path.
const MAX_COORDS: usize = 64;

/// Max message bytes: MAX_COORDS/8 - 1 header = 7.
const SVG_MAX_MESSAGE: usize = MAX_COORDS / 8 - 1;

pub struct VectorEngine;

impl WatermarkEngine for VectorEngine {
    fn embed(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        _intensity: u8,
        output_path: &str,
    ) -> Result<EmbedResult, String> {
        if message.len() > SVG_MAX_MESSAGE {
            return Err(format!(
                "Message too long for SVG: max {} bytes, got {}",
                SVG_MAX_MESSAGE,
                message.len()
            ));
        }

        let svg =
            std::fs::read_to_string(input_path).map_err(|e| format!("Failed to read: {}", e))?;

        // Bit count is determined by MESSAGE SIZE, not path structure.
        // This ensures the scramble permutation is the same regardless of
        // which paths survive (element deletion resilience).
        let msg_bits = (message.len() + 1) * 8; // +1 for length header
        let actual_bits = msg_bits.min(MAX_COORDS);

        let qualifying = count_qualifying_paths(&svg, actual_bits);
        if qualifying == 0 {
            return Err(format!(
                "SVG has no paths with ≥{} coordinates for this message",
                actual_bits
            ));
        }

        let bits = svg_encode_n(message.as_bytes(), actual_bits)?;
        let seed = password::password_to_seed(password);
        let perm = scramble::generate_permutation(bits.len(), &seed);
        let scrambled = scramble::scramble(&bits, &perm);

        let (modified, num_paths) = embed_in_svg(&svg, &scrambled, seed)?;

        std::fs::write(output_path, &modified).map_err(|e| format!("Failed to write: {}", e))?;

        let info = EmbedInfo {
            status: "ok".to_string(),
            mode: "vector-qim".to_string(),
            message: message.to_string(),
            message_bytes: message.len(),
            intensity: 0,
            width: 0,
            height: 0,
            keypoints: num_paths,
            max_capacity: SVG_MAX_MESSAGE,
            output_path: output_path.to_string(),
        };

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
        _intensity: u8,
        output_path: &str,
    ) -> Result<EmbedInfo, String> {
        if message.len() > SVG_MAX_MESSAGE {
            return Err(format!(
                "Message too long: {} bytes, max: {} (mode: vector-qim)",
                message.len(),
                SVG_MAX_MESSAGE
            ));
        }

        let svg =
            std::fs::read_to_string(input_path).map_err(|e| format!("Failed to read: {}", e))?;

        let msg_bits = (message.len() + 1) * 8;
        let qualifying = count_qualifying_paths(&svg, msg_bits.min(MAX_COORDS));

        if qualifying == 0 {
            return Err("SVG has no qualifying paths for this message".to_string());
        }

        Ok(EmbedInfo {
            status: "ok".to_string(),
            mode: "vector-qim".to_string(),
            message: message.to_string(),
            message_bytes: message.len(),
            intensity: 0,
            width: 0,
            height: 0,
            keypoints: qualifying,
            max_capacity: SVG_MAX_MESSAGE,
            output_path: output_path.to_string(),
        })
    }

    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String> {
        let svg =
            std::fs::read_to_string(input_path).map_err(|e| format!("Failed to read: {}", e))?;

        let seed = password::password_to_seed(password);

        // Try different message lengths (1..=SVG_MAX_MESSAGE bytes) since
        // the scramble depends on message length, not path structure.
        for msg_len in 1..=SVG_MAX_MESSAGE {
            let actual_bits = (msg_len + 1) * 8;
            if let Some(result) = try_extract(&svg, seed, actual_bits) {
                return Ok(result);
            }
        }

        Ok(no_detection())
    }
}

fn try_extract(svg: &str, seed: [u8; 32], actual_bits: usize) -> Option<ExtractResult> {
    let d_attrs = find_path_d_attrs(svg);

    let mut bit_votes_one = vec![0usize; actual_bits];
    let mut bit_votes_total = vec![0usize; actual_bits];
    let mut total_conf = 0.0;
    let mut num_paths = 0;

    // Generate all PN values once (not per-path)
    let pn_values: Vec<f64> = (0..actual_bits).map(|i| generate_pn(seed, i)).collect();

    for d in &d_attrs {
        let numbers = parse_numbers(d);
        if numbers.len() < actual_bits {
            continue;
        }
        num_paths += 1;
        let mut path_conf = 0.0;

        for (i, &(val, _, _)) in numbers.iter().enumerate().take(actual_bits) {
            let (bit, conf) = qim_extract(val, pn_values[i]);
            bit_votes_total[i] += 1;
            if bit {
                bit_votes_one[i] += 1;
            }
            path_conf += conf;
        }
        total_conf += path_conf / actual_bits as f64;
    }

    if num_paths == 0 {
        return None;
    }

    let avg_conf = total_conf / num_paths as f64;

    let scrambled_bits: Vec<bool> = bit_votes_one
        .iter()
        .zip(bit_votes_total.iter())
        .map(|(&ones, &total)| if total == 0 { false } else { ones * 2 > total })
        .collect();

    let perm = scramble::generate_permutation(scrambled_bits.len(), &seed);
    let bits = scramble::unscramble(&scrambled_bits, &perm);

    match svg_decode(&bits) {
        Ok(msg) => match String::from_utf8(msg) {
            Ok(m) => Some(ExtractResult {
                detected: true,
                confidence: avg_conf,
                message: Some(m),
            }),
            Err(_) => None,
        },
        Err(_) => None,
    }
}

// ── SVG Text-Level Processing ────────────────────────────────────────────

/// Find all `d="..."` attribute values in the SVG text.
fn find_path_d_attrs(svg: &str) -> Vec<String> {
    let mut results = Vec::new();
    let pattern = " d=\"";
    let mut search_from = 0;

    while let Some(start) = svg[search_from..].find(pattern) {
        let attr_start = search_from + start + pattern.len();
        if let Some(end) = svg[attr_start..].find('"') {
            results.push(svg[attr_start..attr_start + end].to_string());
        }
        search_from = attr_start;
    }
    results
}

/// Count paths that have at least `min_coords` numeric values.
fn count_qualifying_paths(svg: &str, min_coords: usize) -> usize {
    find_path_d_attrs(svg)
        .iter()
        .filter(|d| count_numbers(d) >= min_coords)
        .count()
}

fn count_numbers(d: &str) -> usize {
    parse_numbers(d).len()
}

/// Parse all numeric values from a path `d` attribute string.
/// Returns (value, byte_start, byte_end) for each number.
fn parse_numbers(d: &str) -> Vec<(f64, usize, usize)> {
    let mut numbers = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];

        // Start of a number: digit, minus, plus, or dot followed by digit
        let is_num_start = ch.is_ascii_digit()
            || ch == b'.'
            || ((ch == b'-' || ch == b'+')
                && i + 1 < bytes.len()
                && (bytes[i + 1].is_ascii_digit() || bytes[i + 1] == b'.'));

        if !is_num_start {
            i += 1; // Skip ANY non-numeric character (letters, commas, spaces, newlines, tabs)
            continue;
        }

        let start = i;
        // Sign
        if bytes[i] == b'-' || bytes[i] == b'+' {
            i += 1;
        }
        // Integer/decimal digits
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        // Scientific notation
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }

        if i > start {
            let token = &d[start..i];
            if let Ok(val) = token.parse::<f64>() {
                numbers.push((val, start, i));
            }
        }
    }
    numbers
}

/// Embed watermark into all qualifying paths in the SVG text.
fn embed_in_svg(svg: &str, scrambled: &[bool], seed: [u8; 32]) -> Result<(String, usize), String> {
    let pattern = " d=\"";
    let mut num_paths = 0;

    // Pre-generate all PN values
    let pn_values: Vec<f64> = (0..scrambled.len()).map(|i| generate_pn(seed, i)).collect();

    // Collect all (attr_start, attr_end) positions first
    let d_positions: Vec<(usize, usize)> = {
        let mut positions = Vec::new();
        let mut search_from = 0;
        while let Some(start) = svg[search_from..].find(pattern) {
            let attr_start = search_from + start + pattern.len();
            if let Some(end) = svg[attr_start..].find('"') {
                positions.push((attr_start, attr_start + end));
            }
            search_from = attr_start + 1;
        }
        positions
    };

    // Build result using a single pass with collected replacements
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();

    for &(attr_start, attr_end) in &d_positions {
        let d_value = &svg[attr_start..attr_end];
        let numbers = parse_numbers(d_value);

        if numbers.len() < scrambled.len() {
            continue;
        }
        num_paths += 1;

        // Build modified d value efficiently
        let mut new_d = String::with_capacity(d_value.len() + 64);
        let mut last_end = 0;

        for (i, &(val, tok_start, tok_end)) in numbers.iter().enumerate().take(scrambled.len()) {
            new_d.push_str(&d_value[last_end..tok_start]);
            let new_val = qim_embed(val, scrambled[i], pn_values[i]);
            new_d.push_str(&format!("{:.6}", new_val));
            last_end = tok_end;
        }
        new_d.push_str(&d_value[last_end..]);

        replacements.push((attr_start, attr_end, new_d));
    }

    if num_paths == 0 {
        return Err("SVG has no qualifying paths".to_string());
    }

    // Apply replacements in reverse order to preserve offsets
    let mut result = svg.to_string();
    for (start, end, replacement) in replacements.into_iter().rev() {
        result.replace_range(start..end, &replacement);
    }

    Ok((result, num_paths))
}

// ── QIM ──────────────────────────────────────────────────────────────────

fn qim_embed(val: f64, bit: bool, pn: f64) -> f64 {
    let base = (val / Q).floor() * Q;
    if (bit as i8 * 2 - 1) as f64 * pn > 0.0 {
        base + 0.75 * Q
    } else {
        base + 0.25 * Q
    }
}

fn qim_extract(val: f64, pn: f64) -> (bool, f64) {
    let v = ((val % Q) + Q) % Q;
    let normalized = v / Q;
    let raw_bit = normalized >= 0.5;
    let bit = raw_bit == (pn > 0.0);

    // Confidence = distance to the NEAREST decision boundary (0.0, 0.5, or 1.0).
    // QIM values sit at 0.25 or 0.75. Boundaries are at 0.0, 0.5, and 1.0.
    // dist_to_boundary = min(|normalized - 0.0|, |normalized - 0.5|, |normalized - 1.0|)
    let dist = normalized
        .min(1.0 - normalized)
        .min((normalized - 0.5).abs());
    // Normalize: max possible distance is 0.25 (at the QIM target points)
    let conf = (dist / 0.25).min(1.0);
    (bit, conf)
}

/// Generate a PN value for a specific bit index.
fn generate_pn(seed: [u8; 32], idx: usize) -> f64 {
    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(idx.to_le_bytes());
    let hash = hasher.finalize();
    let mut block_seed = [0u8; 32];
    block_seed.copy_from_slice(&hash);
    let mut rng = ChaCha20Rng::from_seed(block_seed);
    if rng.gen_bool(0.5) {
        1.0
    } else {
        -1.0
    }
}

// ── Message Encoding ─────────────────────────────────────────────────────

fn svg_encode_n(message: &[u8], total_bits: usize) -> Result<Vec<bool>, String> {
    let payload_bytes = total_bits.div_ceil(8);
    if payload_bytes == 0 || message.len() + 1 > payload_bytes {
        return Err(format!(
            "Message too long: max {} bytes, got {}",
            payload_bytes.saturating_sub(1),
            message.len()
        ));
    }
    let mut payload = vec![0u8; payload_bytes];
    payload[0] = message.len() as u8;
    payload[1..1 + message.len()].copy_from_slice(message);

    let mut bits = Vec::with_capacity(total_bits);
    for byte in &payload {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }
    bits.truncate(total_bits);
    Ok(bits)
}

fn svg_decode(bits: &[bool]) -> Result<Vec<u8>, String> {
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

fn no_detection() -> ExtractResult {
    ExtractResult {
        detected: false,
        confidence: 0.0,
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_svg_encode_decode() {
        let msg = b"Hi";
        let bits = svg_encode_n(msg, 24).unwrap(); // 3 bytes = 24 bits
        assert_eq!(bits.len(), 24);
        assert_eq!(svg_decode(&bits).unwrap(), msg);
    }

    #[test]
    fn test_svg_max_message() {
        assert_eq!(SVG_MAX_MESSAGE, 7);
        assert!(svg_encode_n(&vec![b'A'; 7], MAX_COORDS).is_ok());
        assert!(svg_encode_n(&vec![b'A'; 8], MAX_COORDS).is_err());
    }

    #[test]
    fn test_qim_round_trip() {
        for val in [10.0, 50.5, 100.125, 200.75, -30.0, 0.0] {
            for pn in [1.0f64, -1.0] {
                for bit in [true, false] {
                    let embedded = qim_embed(val, bit, pn);
                    let (extracted, conf) = qim_extract(embedded, pn);
                    assert_eq!(
                        extracted, bit,
                        "QIM failed for val={}, pn={}, bit={}",
                        val, pn, bit
                    );
                    assert!(conf > 0.9, "Low conf {:.3} for val={}", conf, val);
                }
            }
        }
    }

    #[test]
    fn test_confidence_near_boundary() {
        // Value exactly at the 0.5 boundary should have zero confidence
        let val = 0.5 * Q; // normalized = 0.5 exactly
        let (_, conf) = qim_extract(val, 1.0);
        assert!(
            conf < 0.1,
            "Boundary value should have low confidence: {}",
            conf
        );

        // Value at 0.25*Q (QIM target) should have high confidence
        let val = 0.25 * Q;
        let (_, conf) = qim_extract(val, 1.0);
        assert!(
            conf > 0.9,
            "QIM target should have high confidence: {}",
            conf
        );

        // Value near 1.0 (≡0.0) boundary should have low confidence
        let val = 0.99 * Q;
        let (_, conf) = qim_extract(val, 1.0);
        assert!(
            conf < 0.1,
            "Near-boundary should have low confidence: {}",
            conf
        );
    }

    #[test]
    fn test_parse_numbers_with_whitespace() {
        // Ensure no infinite loop on newlines, tabs, etc.
        let d = "M 100,200\n L 300\t400\rZ";
        let nums = parse_numbers(d);
        assert_eq!(nums.len(), 4);
        assert_eq!(nums[0].0, 100.0);
        assert_eq!(nums[1].0, 200.0);
        assert_eq!(nums[2].0, 300.0);
        assert_eq!(nums[3].0, 400.0);
    }

    #[test]
    fn test_parse_numbers_negative() {
        let d = "M -10.5 -20.3 L 30 -40";
        let nums = parse_numbers(d);
        assert_eq!(nums.len(), 4);
        assert_eq!(nums[0].0, -10.5);
        assert_eq!(nums[1].0, -20.3);
    }

    #[test]
    fn test_find_path_d_attrs() {
        let svg = r#"<svg><path d="M 0 0 L 10 10 Z"/><path d="M 5 5 L 15 15 Z"/></svg>"#;
        let attrs = find_path_d_attrs(svg);
        assert_eq!(attrs.len(), 2);
    }

    #[test]
    fn test_embed_verify_round_trip() {
        let engine = VectorEngine;
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/testing_data/svg/shapes.svg");
        let output = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testing_output/shapes_unit_test.svg"
        );

        std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/testing_output")).ok();

        let result = engine.embed(input, "Hi", "d1ng0", 0, output);
        assert!(result.is_ok(), "Embed failed: {:?}", result.err());

        let verify = engine.verify(output, "d1ng0").unwrap();
        assert!(verify.detected, "Watermark not detected");
        assert_eq!(verify.message.as_deref(), Some("Hi"));

        let _ = std::fs::remove_file(output);
    }
}
