//! LZ4 compression module for segment data.
//!
//! Compresses segment data blocks to reduce disk I/O and storage footprint.
//! Uses lz4_flex for high-speed compression/decompression.
//!
//! ## Strategy
//!
//! - Compression is applied per-segment during memtable flush
//! - Decompression is transparent during reads
//! - Small blocks (< 256 bytes) are not compressed (overhead > benefit)
//! - Compression level is tuned for speed, not ratio

use crate::error::{Result, TorexError};

/// Minimum block size to compress. Below this threshold, data is stored uncompressed.
const MIN_COMPRESS_SIZE: usize = 256;

/// Compression flag byte: indicates whether data is compressed.
const FLAG_COMPRESSED: u8 = 1;
const FLAG_UNCOMPRESSED: u8 = 0;

/// Compresses data with LZ4 if beneficial.
///
/// Returns a buffer with format: `[flag: u8][decompressed_len: u32][data]`
/// If compression doesn't reduce size, stores uncompressed.
#[inline]
pub fn compress(data: &[u8]) -> Vec<u8> {
    if data.len() < MIN_COMPRESS_SIZE {
        return encode_uncompressed(data);
    }

    let compressed = lz4_flex::compress_prepend_size(data);
    let total_size = 1 + 4 + compressed.len();
    let uncompressed_size = 1 + 4 + data.len();

    if total_size < uncompressed_size {
        let mut buf = Vec::with_capacity(total_size);
        buf.push(FLAG_COMPRESSED);
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&compressed);
        buf
    } else {
        encode_uncompressed(data)
    }
}

/// Decompresses data that was compressed with [`compress`].
#[inline]
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let flag = data[0];

    match flag {
        FLAG_UNCOMPRESSED => {
            Ok(data[1..].to_vec())
        }
        FLAG_COMPRESSED => {
            if data.len() < 5 {
                return Err(TorexError::Codec("compressed block too short".into()));
            }
            let _decompressed_len = u32::from_le_bytes([
                data[1], data[2], data[3], data[4],
            ]) as usize;

            lz4_flex::decompress_size_prepended(&data[5..])
                .map_err(|e| TorexError::Codec(format!("LZ4 decompress failed: {}", e)))
        }
        _ => {
            Err(TorexError::Codec(format!("invalid compression flag: {}", flag)))
        }
    }
}

/// Returns true if the data block is compressed.
#[inline]
pub fn is_compressed(data: &[u8]) -> bool {
    !data.is_empty() && data[0] == FLAG_COMPRESSED
}

/// Returns the decompressed size without actually decompressing.
#[inline]
pub fn decompressed_size(data: &[u8]) -> Result<usize> {
    if data.is_empty() {
        return Ok(0);
    }

    match data[0] {
        FLAG_UNCOMPRESSED => Ok(data.len() - 1),
        FLAG_COMPRESSED => {
            if data.len() < 5 {
                return Err(TorexError::Codec("compressed block too short".into()));
            }
            Ok(u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize)
        }
        _ => Err(TorexError::Codec("invalid compression flag".into())),
    }
}

fn encode_uncompressed(data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + data.len());
    buf.push(FLAG_UNCOMPRESSED);
    buf.extend_from_slice(data);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_roundtrip() {
        let data = vec![0xABu8; 1024]; // Highly compressible
        let compressed = compress(&data);
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_small_data_not_compressed() {
        let data = b"hello".to_vec();
        let compressed = compress(&data);
        assert!(!is_compressed(&compressed));
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_random_data() {
        let data: Vec<u8> = (0..4096).map(|i| (i * 7 + 13) as u8).collect();
        let compressed = compress(&data);
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_empty_data() {
        let data = b"".to_vec();
        let compressed = compress(&data);
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_decompressed_size() {
        let data = vec![0u8; 2048];
        let compressed = compress(&data);
        let size = decompressed_size(&compressed).unwrap();
        assert_eq!(size, 2048);
    }
}
