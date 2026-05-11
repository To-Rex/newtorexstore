//! Bloom filter for fast negative lookups.
//!
//! Before searching a segment, the bloom filter is checked.
//! If the filter says "no", the key definitely isn't in the segment.
//! This avoids expensive disk I/O and mmap page faults for misses.
//!
//! ## Implementation
//!
//! - Uses double-hashing with two hash functions
//! - Configurable false-positive rate (default: 1%)
//! - Compact bit array storage
//! - Serialized to segment header for zero-overhead reads
//!
//! ## Memory Layout
//!
//! ```text
//! [num_hashes: u8][num_bits: u32][bit_array: N bytes]
//! ```

use ahash::RandomState;
use std::hash::{BuildHasher, Hasher};

/// Default false-positive rate (1%).
pub const DEFAULT_FP_RATE: f64 = 0.01;

/// A space-efficient probabilistic data structure for membership testing.
#[derive(Clone, Debug)]
pub struct BloomFilter {
    /// Bit array.
    bits: Vec<u64>,

    /// Number of hash functions.
    num_hashes: u8,

    /// Number of bits.
    num_bits: u64,
}

impl BloomFilter {
    /// Creates a new bloom filter optimized for `expected_items` elements.
    pub fn new(expected_items: usize) -> Self {
        Self::with_fp_rate(expected_items, DEFAULT_FP_RATE)
    }

    /// Creates a bloom filter with a custom false-positive rate.
    pub fn with_fp_rate(expected_items: usize, fp_rate: f64) -> Self {
        let fp_rate = fp_rate.clamp(0.001, 0.5);
        let expected_items = expected_items.max(1);

        // Optimal number of bits: m = -n * ln(p) / (ln2)^2
        let ln2_sq = (std::f64::consts::LN_2).powi(2);
        let num_bits = (-(expected_items as f64) * fp_rate.ln() / ln2_sq) as u64;
        let num_bits = num_bits.max(64).next_power_of_two();

        // Optimal number of hash functions: k = (m/n) * ln2
        let num_hashes = ((num_bits as f64 / expected_items as f64) * std::f64::consts::LN_2) as u8;
        let num_hashes = num_hashes.clamp(2, 16);

        let num_words = (num_bits + 63) / 64;

        Self {
            bits: vec![0u64; num_words as usize],
            num_hashes,
            num_bits,
        }
    }

    /// Inserts a key into the filter.
    pub fn insert(&mut self, key: &[u8]) {
        let (h1, h2) = self.hash_pair(key);

        for i in 0..self.num_hashes {
            let bit_pos = self.bit_position(h1, h2, i as u64);
            let word_idx = (bit_pos / 64) as usize;
            let bit_idx = bit_pos % 64;
            self.bits[word_idx] |= 1 << bit_idx;
        }
    }

    /// Checks if a key might be in the filter.
    /// Returns `true` if the key might be present (may be false positive).
    /// Returns `false` if the key is definitely not present.
    pub fn might_contain(&self, key: &[u8]) -> bool {
        let (h1, h2) = self.hash_pair(key);

        for i in 0..self.num_hashes {
            let bit_pos = self.bit_position(h1, h2, i as u64);
            let word_idx = (bit_pos / 64) as usize;
            let bit_idx = bit_pos % 64;
            if self.bits[word_idx] & (1 << bit_idx) == 0 {
                return false;
            }
        }

        true
    }

    /// Returns the serialized size in bytes.
    pub fn serialized_size(&self) -> usize {
        1 + 8 + (self.bits.len() * 8)
    }

    /// Serializes the bloom filter to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.serialized_size());
        buf.push(self.num_hashes);
        buf.extend_from_slice(&self.num_bits.to_le_bytes());
        for word in &self.bits {
            buf.extend_from_slice(&word.to_le_bytes());
        }
        buf
    }

    /// Deserializes a bloom filter from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < 9 {
            return None;
        }

        let num_hashes = data[0];
        let num_bits = u64::from_le_bytes(data[1..9].try_into().ok()?);
        let num_words = (num_bits + 63) / 64;

        if data.len() < 9 + (num_words as usize) * 8 {
            return None;
        }

        let mut bits = Vec::with_capacity(num_words as usize);
        for i in 0..num_words {
            let offset = 9 + (i as usize) * 8;
            let word = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            bits.push(word);
        }

        Some(Self {
            bits,
            num_hashes,
            num_bits,
        })
    }

    /// Returns the number of bits.
    #[inline]
    pub fn num_bits(&self) -> u64 {
        self.num_bits
    }

    /// Returns the number of hash functions.
    #[inline]
    pub fn num_hashes(&self) -> u8 {
        self.num_hashes
    }

    /// Computes two independent hash values using AHash.
    fn hash_pair(&self, key: &[u8]) -> (u64, u64) {
        // Use two different random states for independent hashing
        let s1 = RandomState::with_seeds(0xDEADBEEF, 0xCAFEBABE, 0x12345678, 0x87654321);
        let s2 = RandomState::with_seeds(0x13579BDF, 0x2468ACE0, 0xFEDCBA98, 0x76543210);

        let mut hasher1 = s1.build_hasher();
        hasher1.write(key);
        let h1 = hasher1.finish();

        let mut hasher2 = s2.build_hasher();
        hasher2.write(key);
        let h2 = hasher2.finish();

        (h1, h2)
    }

    /// Computes the bit position for the i-th hash function.
    #[inline]
    fn bit_position(&self, h1: u64, h2: u64, i: u64) -> u64 {
        // Double hashing: h(i) = h1 + i * h2
        (h1.wrapping_add(i.wrapping_mul(h2))) % self.num_bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_insert_and_check() {
        let mut filter = BloomFilter::new(1000);

        filter.insert(b"key1");
        filter.insert(b"key2");
        filter.insert(b"key3");

        assert!(filter.might_contain(b"key1"));
        assert!(filter.might_contain(b"key2"));
        assert!(filter.might_contain(b"key3"));
    }

    #[test]
    fn test_bloom_negative() {
        let mut filter = BloomFilter::new(1000);

        filter.insert(b"existing_key");

        // Most non-inserted keys should return false
        let mut false_positives = 0;
        for i in 0..1000 {
            let key = format!("nonexistent_{}", i);
            if filter.might_contain(key.as_bytes()) {
                false_positives += 1;
            }
        }

        // Should be well under 5% false positive rate
        assert!(false_positives < 50, "Too many false positives: {}", false_positives);
    }

    #[test]
    fn test_bloom_serialize_deserialize() {
        let mut filter = BloomFilter::new(100);
        filter.insert(b"key1");
        filter.insert(b"key2");

        let serialized = filter.serialize();
        let deserialized = BloomFilter::deserialize(&serialized).unwrap();

        assert!(deserialized.might_contain(b"key1"));
        assert!(deserialized.might_contain(b"key2"));
        assert!(!deserialized.might_contain(b"other"));
    }

    #[test]
    fn test_bloom_large_dataset() {
        let n = 100_000;
        let mut filter = BloomFilter::new(n);

        for i in 0..n {
            let key = format!("key_{}", i);
            filter.insert(key.as_bytes());
        }

        // All inserted keys should be found
        for i in 0..100 {
            let key = format!("key_{}", i);
            assert!(filter.might_contain(key.as_bytes()));
        }

        // Check false positive rate
        let mut false_positives = 0;
        let test_count = 10_000;
        for i in 0..test_count {
            let key = format!("miss_{}", i);
            if filter.might_contain(key.as_bytes()) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / test_count as f64;
        assert!(fp_rate < 0.05, "FP rate too high: {:.4}", fp_rate);
    }
}
