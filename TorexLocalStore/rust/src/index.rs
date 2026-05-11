//! Sparse index for fast key lookups across segments.
//!
//! The index maintains an in-memory map from keys to their segment locations.
//! It uses a sparse approach: only every Nth key is indexed, and the block
//! containing the target key is scanned linearly.
//!
//! ## Design Decisions
//!
//! - **In-memory hash map**: O(1) lookups for indexed keys
//! - **Sparse sampling**: Keeps memory usage bounded even with billions of keys
//! - **Bloom filter support**: Quickly reject non-existent keys (future)
//! - **Sorted iteration**: Supports range scans via segment indexes

use std::collections::HashMap;

/// Location of a key in a segment.
#[derive(Debug, Clone)]
pub struct KeyLocation {
    /// Segment ID containing this key.
    pub segment_id: u64,

    /// Offset within the segment file.
    pub offset: u64,
}

/// In-memory sparse index mapping keys to their locations.
pub struct SparseIndex {
    /// Maps key bytes to their most recent location.
    index: HashMap<Vec<u8>, KeyLocation>,

    /// Sampling interval: index every Nth key.
    sample_interval: usize,
}

impl SparseIndex {
    /// Creates a new sparse index with the given sampling interval.
    pub fn new(sample_interval: usize) -> Self {
        Self {
            index: HashMap::new(),
            sample_interval,
        }
    }

    /// Inserts a key-location mapping.
    #[inline]
    pub fn insert(&mut self, key: Vec<u8>, location: KeyLocation) {
        self.index.insert(key, location);
    }

    /// Looks up a key's location.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<&KeyLocation> {
        self.index.get(key)
    }

    /// Removes a key from the index.
    #[inline]
    pub fn remove(&mut self, key: &[u8]) {
        self.index.remove(key);
    }

    /// Returns the number of indexed keys.
    #[inline]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Clears the entire index.
    pub fn clear(&mut self) {
        self.index.clear();
    }

    /// Rebuilds the index from segment metadata.
    /// Called during startup and after compaction.
    pub fn rebuild_from_segments(
        &mut self,
        segments: &[(u64, Vec<(Vec<u8>, u64)>)],
    ) {
        self.index.clear();

        // Process segments oldest-first so newer entries overwrite older ones
        for (segment_id, entries) in segments {
            for (i, (key, offset)) in entries.iter().enumerate() {
                // Sparse sampling: only index every Nth key
                if i % self.sample_interval == 0 {
                    self.index.insert(
                        key.clone(),
                        KeyLocation {
                            segment_id: *segment_id,
                            offset: *offset,
                        },
                    );
                }
            }
        }
    }

    /// Returns the sampling interval.
    pub fn sample_interval(&self) -> usize {
        self.sample_interval
    }
}

impl Default for SparseIndex {
    fn default() -> Self {
        Self::new(64) // Index every 64th key by default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut index = SparseIndex::new(1);
        index.insert(
            b"key1".to_vec(),
            KeyLocation {
                segment_id: 0,
                offset: 100,
            },
        );

        let loc = index.get(b"key1").unwrap();
        assert_eq!(loc.segment_id, 0);
        assert_eq!(loc.offset, 100);
    }

    #[test]
    fn test_overwrite() {
        let mut index = SparseIndex::new(1);
        index.insert(
            b"key1".to_vec(),
            KeyLocation {
                segment_id: 0,
                offset: 100,
            },
        );
        index.insert(
            b"key1".to_vec(),
            KeyLocation {
                segment_id: 1,
                offset: 200,
            },
        );

        let loc = index.get(b"key1").unwrap();
        assert_eq!(loc.segment_id, 1);
    }

    #[test]
    fn test_remove() {
        let mut index = SparseIndex::new(1);
        index.insert(
            b"key1".to_vec(),
            KeyLocation {
                segment_id: 0,
                offset: 100,
            },
        );
        index.remove(b"key1");
        assert!(index.get(b"key1").is_none());
    }

    #[test]
    fn test_rebuild() {
        let mut index = SparseIndex::new(1);

        let segments = vec![
            (
                0u64,
                vec![
                    (b"key1".to_vec(), 100u64),
                    (b"key2".to_vec(), 200u64),
                ],
            ),
            (
                1u64,
                vec![
                    (b"key1".to_vec(), 300u64), // Overwrites segment 0
                ],
            ),
        ];

        index.rebuild_from_segments(&segments);

        let loc = index.get(b"key1").unwrap();
        assert_eq!(loc.segment_id, 1);
        assert_eq!(loc.offset, 300);
    }
}
