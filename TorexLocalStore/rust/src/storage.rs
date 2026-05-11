//! Core storage engine combining memtable, WAL, and segments.
//!
//! This is the heart of the LSM-tree implementation:
//!
//! ```text
//! Write → WAL → Memtable → (flush) → Segment
//! Read  → Memtable → Segments (newest first)
//! ```
//!
//! ## Concurrency Model
//!
//! - Single writer: `RwLock` on the active memtable
//! - Multiple readers: Read lock allows concurrent reads
//! - Background flush: Memtable is swapped atomically, old one flushed
//!
//! ## Crash Recovery
//!
//! On startup, the WAL is replayed to restore the memtable state.
//! Segments are immutable and always consistent.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::config::TorexConfig;
use crate::error::Result;
use crate::memtable::{Memtable, MemtableEntry};
use crate::query::{apply_pagination, merge_entries, Query, QueryResult};
use crate::segment::SegmentManager;
use crate::wal::{self, Wal};

/// Core storage engine for a single collection (box).
pub struct Storage {
    /// Configuration.
    config: TorexConfig,

    /// Active memtable (writes go here).
    memtable: Arc<RwLock<Memtable>>,

    /// Write-Ahead Log.
    wal: Arc<RwLock<Wal>>,

    /// Segment manager.
    segments: Arc<RwLock<SegmentManager>>,
}

impl Storage {
    /// Opens or creates a storage engine at the given path.
    pub fn open(config: TorexConfig) -> Result<Self> {
        config.validate()?;

        // Ensure directory exists
        std::fs::create_dir_all(&config.path)?;

        // Open WAL
        let mut wal = Wal::open(&config)?;

        // Open segment manager
        let seg_dir = config.path.join("segments");
        let mut segments = SegmentManager::new(seg_dir)?;

        // Recovery: replay WAL
        let wal_path = config.path.join("wal.log");
        if wal_path.exists() {
            match wal::replay_wal(&wal_path) {
                Ok(entries) => {
                    if !entries.is_empty() {
                        log::info!("Recovering {} WAL entries", entries.len());
                        let mut memtable = Memtable::new(config.memtable_size);

                        for entry in entries {
                            match entry {
                                wal::WalEntry::Put { key, value } => {
                                    memtable.put(key, value);
                                }
                                wal::WalEntry::Delete { key } => {
                                    memtable.delete(key);
                                }
                            }
                        }

                        // If memtable has entries, flush it to a segment
                        if !memtable.is_empty() {
                            let drained = memtable.drain_sorted();
                            segments.create_segment(&drained)?;
                            wal.truncate()?;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("WAL recovery failed: {}", e);
                }
            }
        }

        let memtable = Memtable::new(config.memtable_size);

        Ok(Self {
            config,
            memtable: Arc::new(RwLock::new(memtable)),
            wal: Arc::new(RwLock::new(wal)),
            segments: Arc::new(RwLock::new(segments)),
        })
    }

    /// Stores a key-value pair.
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        // 1. Write to WAL first (durability) using mmap
        {
            let mut wal = self.wal.write();
            wal.put(key, value)?;
        }

        // 2. Write to memtable
        let should_flush = {
            let mut mt = self.memtable.write();
            mt.put(key.to_vec(), value.to_vec());
            mt.is_full()
        };

        // 3. Flush if memtable is full
        if should_flush {
            self.flush_memtable()?;
        }

        Ok(())
    }

    /// Retrieves a value by key.
    ///
    /// Lookup order: memtable → segments (newest first)
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // 1. Check memtable first
        {
            let mt = self.memtable.read();
            match mt.get(key) {
                Some(MemtableEntry::Put(value)) => return Ok(Some(value.clone())),
                Some(MemtableEntry::Delete) => return Ok(None),
                None => {}
            }
        }

        // 2. Check segments using mmap
        let segments = self.segments.read();
        segments.get(key)
    }

    /// Deletes a key.
    pub fn delete(&self, key: &[u8]) -> Result<()> {
        // 1. Write tombstone to WAL using mmap
        {
            let mut wal = self.wal.write();
            wal.delete(key)?;
        }

        // 2. Write tombstone to memtable
        let should_flush = {
            let mut mt = self.memtable.write();
            mt.delete(key.to_vec());
            mt.is_full()
        };

        if should_flush {
            self.flush_memtable()?;
        }

        Ok(())
    }

    /// Inserts multiple key-value pairs atomically with a SINGLE WAL fsync.
    /// Up to 1000x faster than calling `put()` in a loop for large batches.
    pub fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        // Write entire batch to WAL with single fsync
        {
            let mut wal = self.wal.write();
            wal.write_puts_batch(entries)?;
        }

        // Write all entries to memtable under a single lock acquisition
        let should_flush = {
            let mut mt = self.memtable.write();
            for &(key, value) in entries {
                mt.put(key.to_vec(), value.to_vec());
            }
            mt.is_full()
        };

        if should_flush {
            self.flush_memtable()?;
        }

        Ok(())
    }

    /// Deletes multiple keys atomically with a SINGLE WAL fsync.
    pub fn batch_delete(&self, keys: &[&[u8]]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }

        // Write entire batch to WAL with single fsync
        {
            let mut wal = self.wal.write();
            wal.write_deletes_batch(keys)?;
        }

        // Write tombstones to memtable under a single lock acquisition
        let should_flush = {
            let mut mt = self.memtable.write();
            for &key in keys {
                mt.delete(key.to_vec());
            }
            mt.is_full()
        };

        if should_flush {
            self.flush_memtable()?;
        }

        Ok(())
    }

    /// Checks if a key exists.
    pub fn exists(&self, key: &[u8]) -> Result<bool> {
        match self.get(key) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Returns the number of entries in the memtable.
    pub fn memtable_len(&self) -> usize {
        self.memtable.read().len()
    }

    /// Returns the number of segments.
    pub fn segment_count(&self) -> usize {
        self.segments.read().segments().len()
    }

    /// Returns a clone of the segments Arc for background compaction.
    pub fn segments(&self) -> Arc<RwLock<SegmentManager>> {
        Arc::clone(&self.segments)
    }

    /// Forces a flush of the memtable to disk.
    pub fn flush_memtable(&self) -> Result<()> {
        let entries = {
            let mut mt = self.memtable.write();
            if mt.is_empty() {
                return Ok(());
            }
            mt.drain_sorted()
        };

        if entries.is_empty() {
            return Ok(());
        }

        // Write to segment
        {
            let mut segments = self.segments.write();
            segments.create_segment(&entries)?;
        }

        // Truncate WAL
        self.wal.write().truncate()?;

        log::debug!("Flushed {} entries to segment", entries.len());
        Ok(())
    }

    /// Flushes the WAL BufWriter buffer to the OS without fsync.
    /// Called by the background WAL worker every 200ms to ensure
    /// buffered writes are visible to recovery even without sync_writes.
    pub fn flush_wal(&self) -> Result<()> {
        self.wal.write().flush()
    }

    /// Closes the storage engine gracefully.
    pub fn close(&self) -> Result<()> {
        // Flush remaining memtable entries
        self.flush_memtable()?;
        self.wal.write().flush()?;
        Ok(())
    }

    /// Returns the storage path.
    pub fn path(&self) -> &std::path::Path {
        &self.config.path
    }

    /// Scans all entries matching a query.
    ///
    /// Merges memtable and segment entries, applies filtering, deduplication,
    /// and pagination.
    pub fn scan(&self, query: &Query) -> Result<Vec<QueryResult>> {
        // 1. Collect memtable entries matching the query
        let memtable_entries: Vec<(Vec<u8>, Option<Vec<u8>>)> = {
            let mt = self.memtable.read();
            mt.iter_sorted()
                .filter(|(key, _)| query.matches_key(key))
                .map(|(key, entry)| match entry {
                    MemtableEntry::Put(value) => (key, Some(value.clone())),
                    MemtableEntry::Delete => (key, None),
                })
                .collect()
        };

        // 2. Collect segment entries matching the query
        let segment_entries: Vec<Vec<(Vec<u8>, Vec<u8>)>> = {
            let segments = self.segments.read();
            segments
                .segments()
                .iter()
                .rev()
                .map(|segment| {
                    segment
                        .read_all()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(key, _)| query.matches_key(key))
                        .collect()
                })
                .collect()
        };

        // 3. Merge and deduplicate
        let mut results = merge_entries(memtable_entries, segment_entries, query.reverse);

        // 4. Apply pagination
        apply_pagination(&mut results, query.offset, query.limit);

        // 5. Apply keys_only if needed
        if query.keys_only {
            for r in &mut results {
                r.value = None;
            }
        }

        Ok(results)
    }

    /// Returns all keys in the collection.
    pub fn keys(&self) -> Result<Vec<Vec<u8>>> {
        let query = Query::new().keys_only();
        let results = self.scan(&query)?;
        Ok(results.into_iter().map(|r| r.key).collect())
    }

    /// Returns the approximate total number of entries.
    pub fn count(&self) -> Result<usize> {
        let mt_count = self.memtable.read().len();
        let seg_count: usize = {
            let segments = self.segments.read();
            segments
                .segments()
                .iter()
                .map(|s| s.entry_count as usize)
                .sum()
        };
        // This is approximate because tombstones are counted
        Ok(mt_count + seg_count)
    }

    /// Executes a range scan from start_key to end_key.
    pub fn range_scan(
        &self,
        start_key: &[u8],
        end_key: &[u8],
        limit: Option<usize>,
    ) -> Result<Vec<QueryResult>> {
        let query = Query::new().start_at(start_key).end_at(end_key);

        let query = match limit {
            Some(n) => query.limit(n),
            None => query,
        };

        self.scan(&query)
    }

    /// Executes a prefix scan.
    pub fn prefix_scan(&self, prefix: &[u8], limit: Option<usize>) -> Result<Vec<QueryResult>> {
        let query = Query::new().prefix(prefix);

        let query = match limit {
            Some(n) => query.limit(n),
            None => query,
        };

        self.scan(&query)
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            log::error!("Error closing storage: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_storage(dir: &TempDir) -> Storage {
        let config = TorexConfig::new(dir.path().join("test_db"));
        Storage::open(config).unwrap()
    }

    #[test]
    fn test_put_and_get() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"key1", b"value1").unwrap();
        store.put(b"key2", b"value2").unwrap();

        assert_eq!(store.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(store.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        assert_eq!(store.get(b"key3").unwrap(), None);
    }

    #[test]
    fn test_delete() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"key1", b"value1").unwrap();
        store.delete(b"key1").unwrap();

        assert_eq!(store.get(b"key1").unwrap(), None);
    }

    #[test]
    fn test_exists() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"key1", b"value1").unwrap();
        assert!(store.exists(b"key1").unwrap());
        assert!(!store.exists(b"key2").unwrap());
    }

    #[test]
    fn test_overwrite() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"key1", b"value1").unwrap();
        store.put(b"key1", b"value2").unwrap();

        assert_eq!(store.get(b"key1").unwrap(), Some(b"value2".to_vec()));
    }

    #[test]
    fn test_flush_creates_segment() {
        let dir = TempDir::new().unwrap();
        let config = TorexConfig::new(dir.path().join("test_db"));
        let config = TorexConfig {
            memtable_size: 100, // Very small to trigger flush
            ..config
        };

        let store = Storage::open(config).unwrap();

        // Write enough data to trigger flush
        for i in 0..100 {
            let key = format!("key_{:04}", i);
            let value = vec![0u8; 50];
            store.put(key.as_bytes(), &value).unwrap();
        }

        assert!(store.segment_count() > 0);

        // Verify data is still accessible
        for i in 0..100 {
            let key = format!("key_{:04}", i);
            assert!(store.get(key.as_bytes()).unwrap().is_some());
        }
    }

    #[test]
    fn test_recovery_after_close() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_db");

        // Write data
        {
            let store = Storage::open(TorexConfig::new(&path)).unwrap();
            store.put(b"key1", b"value1").unwrap();
            store.put(b"key2", b"value2").unwrap();
            store.flush_memtable().unwrap();
        }

        // Reopen and verify
        {
            let store = Storage::open(TorexConfig::new(&path)).unwrap();
            assert_eq!(store.get(b"key1").unwrap(), Some(b"value1".to_vec()));
            assert_eq!(store.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        }
    }

    // ─── Scan / Query Tests ───────────────────────────────────────

    #[test]
    fn test_scan_all() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"key1", b"val1").unwrap();
        store.put(b"key2", b"val2").unwrap();
        store.put(b"key3", b"val3").unwrap();

        let query = Query::new();
        let results = store.scan(&query).unwrap();
        assert_eq!(results.len(), 3);

        // Results should be sorted by key
        assert_eq!(results[0].key, b"key1");
        assert_eq!(results[1].key, b"key2");
        assert_eq!(results[2].key, b"key3");
    }

    #[test]
    fn test_scan_prefix() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"user:1", b"alice").unwrap();
        store.put(b"user:2", b"bob").unwrap();
        store.put(b"post:1", b"hello").unwrap();
        store.put(b"post:2", b"world").unwrap();

        let query = Query::new().prefix(b"user:");
        let results = store.scan(&query).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].value.as_ref().unwrap(), b"alice");
        assert_eq!(results[1].value.as_ref().unwrap(), b"bob");
    }

    #[test]
    fn test_scan_range() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        for i in 0..10 {
            let key = format!("key_{:03}", i);
            store.put(key.as_bytes(), b"val").unwrap();
        }

        // Range from key_003 to key_007 (exclusive end)
        let query = Query::new().start_at(b"key_003").end_at(b"key_007");
        let results = store.scan(&query).unwrap();
        assert_eq!(results.len(), 4); // key_003, key_004, key_005, key_006
        assert_eq!(results[0].key, b"key_003");
        assert_eq!(results[3].key, b"key_006");
    }

    #[test]
    fn test_scan_with_limit_and_offset() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        for i in 0..20 {
            let key = format!("key_{:03}", i);
            store.put(key.as_bytes(), b"val").unwrap();
        }

        let query = Query::new().offset(5).limit(3);
        let results = store.scan(&query).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].key, b"key_005");
        assert_eq!(results[1].key, b"key_006");
        assert_eq!(results[2].key, b"key_007");
    }

    #[test]
    fn test_scan_reverse() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"a", b"1").unwrap();
        store.put(b"b", b"2").unwrap();
        store.put(b"c", b"3").unwrap();

        let query = Query::new().reverse();
        let results = store.scan(&query).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].key, b"c");
        assert_eq!(results[1].key, b"b");
        assert_eq!(results[2].key, b"a");
    }

    #[test]
    fn test_keys() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"alpha", b"1").unwrap();
        store.put(b"beta", b"2").unwrap();
        store.put(b"gamma", b"3").unwrap();

        let all_keys = store.keys().unwrap();
        assert_eq!(all_keys.len(), 3);
        assert_eq!(all_keys[0], b"alpha");
        assert_eq!(all_keys[1], b"beta");
        assert_eq!(all_keys[2], b"gamma");
    }

    #[test]
    fn test_count() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        assert_eq!(store.count().unwrap(), 0);

        for i in 0..50 {
            let key = format!("k{:04}", i);
            store.put(key.as_bytes(), b"v").unwrap();
        }
        assert_eq!(store.count().unwrap(), 50);

        // Delete some
        store.delete(b"k0010").unwrap();
        store.delete(b"k0020").unwrap();
        // Count is approximate (tombstones still counted in memtable)
        assert!(store.count().unwrap() >= 48);
    }

    #[test]
    fn test_prefix_scan_convenience() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"item:apple", b"red").unwrap();
        store.put(b"item:banana", b"yellow").unwrap();
        store.put(b"order:1", b"first").unwrap();

        let results = store.prefix_scan(b"item:", None).unwrap();
        assert_eq!(results.len(), 2);

        let limited = store.prefix_scan(b"item:", Some(1)).unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn test_range_scan_convenience() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        for i in 0..100 {
            let key = format!("rec_{:05}", i);
            store.put(key.as_bytes(), b"data").unwrap();
        }

        let results = store.range_scan(b"rec_00020", b"rec_00030", None).unwrap();
        assert_eq!(results.len(), 10); // 00020..00030 exclusive

        let limited = store
            .range_scan(b"rec_00000", b"rec_00100", Some(5))
            .unwrap();
        assert_eq!(limited.len(), 5);
    }

    #[test]
    fn test_scan_with_tombstones() {
        let dir = TempDir::new().unwrap();
        let store = make_storage(&dir);

        store.put(b"k1", b"v1").unwrap();
        store.put(b"k2", b"v2").unwrap();
        store.put(b"k3", b"v3").unwrap();

        // Delete k2
        store.delete(b"k2").unwrap();

        // Scan should skip tombstones
        let results = store.scan(&Query::new()).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].key, b"k1");
        assert_eq!(results[1].key, b"k3");
    }
}
