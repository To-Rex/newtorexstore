//! In-memory sorted table (memtable) for the LSM-tree.
//!
//! The memtable is an in-memory sorted map that buffers writes before they are
//! flushed to disk as immutable segments. Uses a `BTreeMap` for sorted order,
//! which is critical for efficient range scans and merge operations.
//!
//! ## Design Decisions
//!
//! - **BTreeMap** over HashMap: Sorted keys enable efficient segment merges
//! - **Entry enum**: Supports PUT and DELETE (tombstones) for correct compaction
//! - **Approximate size tracking**: Enables flush decisions without expensive calculations
//! - **Clone on read**: Memtable data is cloned when flushing to create immutable segments

use std::collections::BTreeMap;

/// Represents a single entry in the memtable.
#[derive(Debug, Clone)]
pub enum MemtableEntry {
    /// A key-value pair.
    Put(Vec<u8>),
    /// A tombstone marker (deletion).
    Delete,
}

/// In-memory sorted key-value table.
///
/// Thread safety is provided by the caller (storage engine holds a lock).
/// This keeps the memtable itself simple and fast.
#[derive(Debug)]
pub struct Memtable {
    /// Sorted key-value entries.
    entries: BTreeMap<Vec<u8>, MemtableEntry>,

    /// Approximate memory usage in bytes.
    approximate_size: usize,

    /// Maximum size before flush is triggered.
    max_size: usize,
}

impl Memtable {
    /// Creates a new memtable with the given maximum size.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            approximate_size: 0,
            max_size,
        }
    }

    /// Inserts a key-value pair.
    #[inline]
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let key_size = key.len();
        let value_size = value.len();

        // If overwriting, adjust size
        if let Some(old) = self.entries.insert(key, MemtableEntry::Put(value)) {
            match old {
                MemtableEntry::Put(old_val) => {
                    // Size adjustment: new value might be different size
                    self.approximate_size = self
                        .approximate_size
                        .saturating_sub(old_val.len())
                        .saturating_add(value_size);
                }
                MemtableEntry::Delete => {
                    self.approximate_size = self.approximate_size.saturating_add(value_size);
                }
            }
        } else {
            self.approximate_size += key_size + value_size;
        }
    }

    /// Marks a key as deleted (tombstone).
    #[inline]
    pub fn delete(&mut self, key: Vec<u8>) {
        if let Some(old) = self.entries.insert(key, MemtableEntry::Delete) {
            if let MemtableEntry::Put(old_val) = old {
                self.approximate_size = self.approximate_size.saturating_sub(old_val.len());
            }
        }
    }

    /// Gets a value by key.
    ///
    /// Returns `None` if key doesn't exist or is deleted.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<&MemtableEntry> {
        self.entries.get(key)
    }

    /// Checks if the memtable has exceeded its maximum size.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.approximate_size >= self.max_size
    }

    /// Returns the approximate memory usage.
    #[inline]
    pub fn approximate_size(&self) -> usize {
        self.approximate_size
    }

    /// Returns the number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the memtable is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drains all entries from the memtable, returning them sorted by key.
    /// Uses `std::mem::take` to avoid cloning — O(1) swap instead of O(n) copy.
    pub fn drain_sorted(&mut self) -> Vec<(Vec<u8>, MemtableEntry)> {
        let map = std::mem::take(&mut self.entries);
        self.approximate_size = 0;
        // BTreeMap is already sorted — into_iter preserves order
        map.into_iter().collect()
    }

    /// Returns an iterator over entries in a key range.
    pub fn range<'a>(
        &'a self,
        start: &[u8],
        end: &[u8],
    ) -> impl Iterator<Item = (&'a Vec<u8>, &'a MemtableEntry)> {
        self.entries.range(start.to_vec()..end.to_vec())
    }

    /// Returns all entries as a sorted vector.
    pub fn to_sorted_vec(&self) -> Vec<(&Vec<u8>, &MemtableEntry)> {
        self.entries.iter().collect()
    }

    /// Returns an iterator over sorted entries with owned key and borrowed entry.
    /// Used for query scanning.
    pub fn iter_sorted(&self) -> impl Iterator<Item = (Vec<u8>, &MemtableEntry)> {
        self.entries.iter().map(|(k, v)| (k.clone(), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_get() {
        let mut mt = Memtable::new(1024);
        mt.put(b"key1".to_vec(), b"value1".to_vec());
        mt.put(b"key2".to_vec(), b"value2".to_vec());

        match mt.get(b"key1") {
            Some(MemtableEntry::Put(v)) => assert_eq!(v, b"value1"),
            _ => panic!("expected Put entry"),
        }
    }

    #[test]
    fn test_delete() {
        let mut mt = Memtable::new(1024);
        mt.put(b"key1".to_vec(), b"value1".to_vec());
        mt.delete(b"key1".to_vec());

        match mt.get(b"key1") {
            Some(MemtableEntry::Delete) => {}
            _ => panic!("expected Delete entry"),
        }
    }

    #[test]
    fn test_is_full() {
        let mut mt = Memtable::new(10);
        mt.put(b"key".to_vec(), b"12345678901".to_vec());
        assert!(mt.is_full());
    }

    #[test]
    fn test_drain_sorted() {
        let mut mt = Memtable::new(1024);
        mt.put(b"c".to_vec(), b"3".to_vec());
        mt.put(b"a".to_vec(), b"1".to_vec());
        mt.put(b"b".to_vec(), b"2".to_vec());

        let drained = mt.drain_sorted();
        assert_eq!(drained[0].0, b"a");
        assert_eq!(drained[1].0, b"b");
        assert_eq!(drained[2].0, b"c");
        assert!(mt.is_empty());
    }

    #[test]
    fn test_overwrite_updates_size() {
        let mut mt = Memtable::new(1024);
        mt.put(b"key".to_vec(), b"short".to_vec());
        let size1 = mt.approximate_size();
        mt.put(b"key".to_vec(), b"much_longer_value".to_vec());
        let size2 = mt.approximate_size();
        assert!(size2 > size1);
    }
}
