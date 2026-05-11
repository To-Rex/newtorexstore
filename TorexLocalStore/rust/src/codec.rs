//! Binary codec for zero-copy serialization.
//!
//! ## Wire Format
//!
//! Each entry is encoded as:
//! ```text
//! [key_len: u16][key_bytes: N][value_len: u32][value_bytes: M][crc32: u32]
//! ```
//!
//! This layout enables:
//! - Zero-copy key/value access via mmap
//! - Fast sequential scanning
//! - Integrity verification via CRC32

use crate::error::{Result, TorexError};

/// CRC32 checksum size.
const CRC_SIZE: usize = 4;

/// Encode a key-value pair into a binary entry.
///
/// Format: `[key_len: u16][key][value_len: u32][value][crc32: u32]`
#[inline]
pub fn encode_entry(key: &[u8], value: &[u8]) -> Vec<u8> {
    let key_len = key.len() as u16;
    let value_len = value.len() as u32;
    let header_size = 2 + 4; // key_len + value_len
    let total_size = header_size + key.len() + value.len() + CRC_SIZE;

    let mut buf = Vec::with_capacity(total_size);
    buf.extend_from_slice(&key_len.to_le_bytes());
    buf.extend_from_slice(key);
    buf.extend_from_slice(&value_len.to_le_bytes());
    buf.extend_from_slice(value);

    let crc = compute_crc(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());

    buf
}

/// Decode a key-value pair from a binary entry.
#[inline]
pub fn decode_entry(data: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // Need at least: key_len(2) + value_len(4) + crc(4) = 10 bytes
    if data.len() < 10 {
        return Err(TorexError::Codec("entry too short".into()));
    }

    // Read key length
    let key_len = u16::from_le_bytes([data[0], data[1]]) as usize;

    // Need: key_len(2) + key(N) + value_len(4) + crc(4)
    if data.len() < 2 + key_len + 4 + CRC_SIZE {
        return Err(TorexError::Codec("entry truncated".into()));
    }

    // Read key
    let key = data[2..2 + key_len].to_vec();

    // Read value length
    let value_len_offset = 2 + key_len;
    let value_len = u32::from_le_bytes([
        data[value_len_offset],
        data[value_len_offset + 1],
        data[value_len_offset + 2],
        data[value_len_offset + 3],
    ]) as usize;

    // Read value
    let value_offset = value_len_offset + 4;
    let payload_end = value_offset + value_len;

    if data.len() < payload_end + CRC_SIZE {
        return Err(TorexError::Codec("entry truncated value".into()));
    }

    let value = data[value_offset..payload_end].to_vec();

    // Verify CRC
    let expected_crc = u32::from_le_bytes([
        data[payload_end],
        data[payload_end + 1],
        data[payload_end + 2],
        data[payload_end + 3],
    ]);
    let actual_crc = compute_crc(&data[..payload_end]);

    if expected_crc != actual_crc {
        return Err(TorexError::ChecksumMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }

    Ok((key, value))
}

/// Compute the total encoded size for a key-value pair.
#[inline]
pub fn encoded_size(key_len: usize, value_len: usize) -> usize {
    2 + key_len + 4 + value_len + CRC_SIZE
}

/// Read key_len and value_len from an encoded entry header without full decode.
/// Returns (key_len, value_len).
#[inline]
pub fn encoded_size_from_header(data: &[u8]) -> Result<(usize, usize)> {
    if data.len() < 6 {
        return Err(TorexError::Codec("header too short".into()));
    }

    let key_len = u16::from_le_bytes([data[0], data[1]]) as usize;

    if data.len() < 2 + key_len + 4 {
        return Err(TorexError::Codec("header truncated".into()));
    }

    let value_len = u32::from_le_bytes([
        data[2 + key_len],
        data[3 + key_len],
        data[4 + key_len],
        data[5 + key_len],
    ]) as usize;

    Ok((key_len, value_len))
}

/// Zero-copy decode: returns references into the original buffer.
/// The returned key and value slices point directly into `data`.
#[inline]
pub fn decode_entry_zero_copy(data: &[u8]) -> Result<(&[u8], &[u8])> {
    if data.len() < 10 {
        return Err(TorexError::Codec("entry too short".into()));
    }

    let key_len = u16::from_le_bytes([data[0], data[1]]) as usize;

    if data.len() < 2 + key_len + 4 + CRC_SIZE {
        return Err(TorexError::Codec("entry truncated".into()));
    }

    let key = &data[2..2 + key_len];

    let value_len_offset = 2 + key_len;
    let value_len = u32::from_le_bytes([
        data[value_len_offset],
        data[value_len_offset + 1],
        data[value_len_offset + 2],
        data[value_len_offset + 3],
    ]) as usize;

    let value_offset = value_len_offset + 4;
    let payload_end = value_offset + value_len;

    if data.len() < payload_end + CRC_SIZE {
        return Err(TorexError::Codec("entry truncated value".into()));
    }

    let value = &data[value_offset..payload_end];

    // Verify CRC
    let expected_crc = u32::from_le_bytes([
        data[payload_end],
        data[payload_end + 1],
        data[payload_end + 2],
        data[payload_end + 3],
    ]);
    let actual_crc = compute_crc(&data[..payload_end]);

    if expected_crc != actual_crc {
        return Err(TorexError::ChecksumMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }

    Ok((key, value))
}

/// Compute CRC32 checksum.
#[inline]
pub fn compute_crc(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = generate_crc32_table();
    let mut crc: u32 = 0xFFFFFFFF;

    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[index] ^ (crc >> 8);
    }

    !crc
}

/// Alias for [`compute_crc`] — shorthand used by chunk module.
#[inline]
pub fn crc32(data: &[u8]) -> u32 {
    compute_crc(data)
}

/// Generate CRC32 lookup table at compile time.
const fn generate_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let key = b"test_key";
        let value = b"test_value_with_some_data";
        let encoded = encode_entry(key, value);
        let (decoded_key, decoded_value) = decode_entry(&encoded).unwrap();

        assert_eq!(decoded_key, key.to_vec());
        assert_eq!(decoded_value, value.to_vec());
    }

    #[test]
    fn test_encode_decode_empty_value() {
        let key = b"key";
        let value = b"";
        let encoded = encode_entry(key, value);
        let (decoded_key, decoded_value) = decode_entry(&encoded).unwrap();

        assert_eq!(decoded_key, key.to_vec());
        assert_eq!(decoded_value, value.to_vec());
    }

    #[test]
    fn test_crc_detects_corruption() {
        let key = b"key";
        let value = b"value";
        let mut encoded = encode_entry(key, value);

        // Corrupt a byte in the value area
        encoded[5] ^= 0xFF;

        let result = decode_entry(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_encoded_size() {
        let key = b"hello";
        let value = b"world";
        let encoded = encode_entry(key, value);
        assert_eq!(encoded.len(), encoded_size(key.len(), value.len()));
    }

    #[test]
    fn test_crc32_consistency() {
        let data = b"hello world";
        let crc1 = compute_crc(data);
        let crc2 = compute_crc(data);
        assert_eq!(crc1, crc2);
    }

    #[test]
    fn test_decode_with_extra_data() {
        let key = b"key";
        let value = b"value";
        let mut encoded = encode_entry(key, value);
        // Append extra data (simulates reading from a buffer with more entries)
        encoded.extend_from_slice(b"extra_data");

        let (decoded_key, decoded_value) = decode_entry(&encoded).unwrap();
        assert_eq!(decoded_key, key.to_vec());
        assert_eq!(decoded_value, value.to_vec());
    }
}
