//! Simple repetition-based error correction coding.
//!
//! Each bit is repeated `REPETITION_FACTOR` times during encoding.
//! During decoding, majority vote recovers the original bit.
//!
//! This provides basic error correction at the cost of reduced capacity.
//! A future version may upgrade to Reed-Solomon coding for better efficiency.

const REPETITION_FACTOR: usize = 3;

/// Compute how many message bytes can fit given a number of available blocks.
pub fn max_message_bytes(num_blocks: usize) -> usize {
    let total_data_bits = num_blocks / REPETITION_FACTOR;
    let total_data_bytes = total_data_bits / 8;
    // Subtract 2 bytes for the length header
    total_data_bytes.saturating_sub(2)
}

/// Compute the total number of encoded bits for a given block count.
/// This is the fixed-size output length used for both embed and extract.
pub fn total_encoded_bits(num_blocks: usize) -> usize {
    let total_data_bits = num_blocks / REPETITION_FACTOR;
    let total_data_bytes = total_data_bits / 8;
    total_data_bytes * 8 * REPETITION_FACTOR
}

/// Encode a byte slice with repetition coding, padded to fill the capacity
/// determined by `num_blocks`.
///
/// A 2-byte length header (big-endian u16) is prepended. The payload is
/// zero-padded to fill the full capacity so that the encoded bit length
/// is always `total_encoded_bits(num_blocks)`.
pub fn encode(message: &[u8], num_blocks: usize) -> Result<Vec<bool>, String> {
    let max_msg = max_message_bytes(num_blocks);
    if max_msg == 0 {
        return Err("Image too small: insufficient capacity".to_string());
    }
    if message.len() > max_msg {
        return Err(format!(
            "Message too long: max {} bytes, got {} bytes",
            max_msg,
            message.len()
        ));
    }

    let total_data_bits = num_blocks / REPETITION_FACTOR;
    let total_data_bytes = total_data_bits / 8;

    // Build payload: [len_high, len_low, message..., zero_padding...]
    let msg_len = message.len() as u16;
    let mut payload = vec![0u8; total_data_bytes];
    payload[0] = (msg_len >> 8) as u8;
    payload[1] = (msg_len & 0xFF) as u8;
    payload[2..2 + message.len()].copy_from_slice(message);

    // Convert to bits
    let mut bits = Vec::with_capacity(total_data_bytes * 8);
    for byte in &payload {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }

    // Apply repetition coding
    let mut encoded = Vec::with_capacity(bits.len() * REPETITION_FACTOR);
    for bit in &bits {
        for _ in 0..REPETITION_FACTOR {
            encoded.push(*bit);
        }
    }

    Ok(encoded)
}

/// Decode bits with repetition coding and majority vote.
///
/// `bits` should contain the raw extracted bits (scrambled order already resolved).
/// Returns the decoded message bytes.
pub fn decode(bits: &[bool]) -> Result<Vec<u8>, String> {
    if bits.len() < REPETITION_FACTOR * 16 {
        return Err("Not enough data to decode (need at least 2 header bytes)".to_string());
    }

    // Majority vote to recover original bits
    let num_original_bits = bits.len() / REPETITION_FACTOR;
    let mut decoded_bits = Vec::with_capacity(num_original_bits);

    for i in 0..num_original_bits {
        let mut ones = 0usize;
        for r in 0..REPETITION_FACTOR {
            if bits[i * REPETITION_FACTOR + r] {
                ones += 1;
            }
        }
        decoded_bits.push(ones > REPETITION_FACTOR / 2);
    }

    // Convert bits to bytes
    let mut bytes = Vec::with_capacity(decoded_bits.len() / 8);
    for chunk in decoded_bits.chunks(8) {
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

    // Read length header
    if bytes.len() < 2 {
        return Err("Decoded data too short".to_string());
    }
    let msg_len = ((bytes[0] as u16) << 8) | (bytes[1] as u16);
    let msg_len = msg_len as usize;

    if msg_len == 0 {
        return Err("Invalid message length: 0".to_string());
    }

    if 2 + msg_len > bytes.len() {
        return Err(format!(
            "Message length {} exceeds available data {}",
            msg_len,
            bytes.len() - 2
        ));
    }

    Ok(bytes[2..2 + msg_len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_round_trip() {
        let message = b"Hello, World!";
        // Need enough blocks: (2 + 13) * 8 * 3 = 360 blocks minimum
        let num_blocks = 400;
        let encoded = encode(message, num_blocks).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn test_error_correction() {
        let message = b"Test";
        let num_blocks = 256;
        let mut encoded = encode(message, num_blocks).unwrap();

        // Flip some bits (within correction capability)
        // Each original bit has 3 copies; flipping 1 of 3 should be correctable
        for i in (0..encoded.len()).step_by(REPETITION_FACTOR) {
            encoded[i] = !encoded[i]; // flip first copy of each group
        }

        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn test_max_message_bytes() {
        // 1024 blocks, rep factor 3: 1024/3 = 341 bits = 42 bytes - 2 header = 40
        assert_eq!(max_message_bytes(1024), 40);
        // 256 blocks: 256/3 = 85 bits = 10 bytes - 2 header = 8
        assert_eq!(max_message_bytes(256), 8);
    }

    #[test]
    fn test_total_encoded_bits() {
        // 256 blocks: 256/3=85 bits, 85/8=10 bytes, 10*8*3 = 240 bits
        assert_eq!(total_encoded_bits(256), 240);
    }

    #[test]
    fn test_padded_output_is_fixed_size() {
        let num_blocks = 256;
        let expected_len = total_encoded_bits(num_blocks);

        let short = encode(b"Hi", num_blocks).unwrap();
        let long = encode(b"Longer!!", num_blocks).unwrap();
        assert_eq!(short.len(), expected_len);
        assert_eq!(long.len(), expected_len);
    }

    #[test]
    fn test_empty_message_rejected() {
        let result = decode(&vec![false; 48]); // 48 bits = 2 header bytes (all zero)
        assert!(result.is_err()); // msg_len = 0 is invalid
    }
}
