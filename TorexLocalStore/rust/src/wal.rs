//! Write-Ahead Log (WAL) for crash recovery.
//!
//! The WAL ensures durability by appending every write operation to a log file
//! before applying it to the memtable. On recovery, the WAL is replayed to
//! restore the memtable state.
//!
//! ## Wire Format
//!
//! Each WAL entry:
//! ```text
//! [entry_len: u32][entry_type: u8][key_len: u16][key][value_len: u32][value][crc32: u32]
//! ```
//!
//! ## Design Decisions
//!
//! - **Append-only**: All writes are sequential, maximizing disk throughput
//! - **CRC per entry**: Detect partial writes from crashes
//! - **Entry length prefix**: Enable fast forward scanning during recovery
//! - **Buffered writes**: Batch small writes for throughput, with fsync on flush

use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::codec::compute_crc;
use crate::config::TorexConfig;
use crate::error::Result;

/// WAL entry type: PUT operation.
const ENTRY_TYPE_PUT: u8 = 1;

/// WAL entry type: DELETE operation.
const ENTRY_TYPE_DELETE: u8 = 2;

/// WAL file header: magic bytes + format version.
const WAL_HEADER_SIZE: usize = 8;

/// Write-Ahead Log handle.
pub struct Wal {
    /// Path to the current WAL file.
    path: PathBuf,

    /// Buffered writer for the WAL file.
    writer: BufWriter<std::fs::File>,

    /// Current WAL file size in bytes.
    current_size: Arc<AtomicU64>,

    /// Maximum WAL file size before rotation.
    max_size: u64,

    /// Whether to sync after each write.
    sync_writes: bool,
}

impl Wal {
    /// Creates or opens a WAL file at the given path.
    pub fn open(config: &TorexConfig) -> Result<Self> {
        let path = config.path.join("wal.log");

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        let current_size = file.metadata()?.len();
        let current_size = Arc::new(AtomicU64::new(current_size));

        // 256 KB buffer — large sequential writes for maximum throughput
        let writer = BufWriter::with_capacity(256 * 1024, file);

        Ok(Self {
            path,
            writer,
            current_size,
            max_size: config.wal_max_size,
            sync_writes: config.sync_writes,
        })
    }

    /// Appends a PUT operation to the WAL.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        let entry = self.encode_entry(ENTRY_TYPE_PUT, key, Some(value));
        self.write_entry(&entry)
    }

    /// Appends a DELETE operation to the WAL.
    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        let entry = self.encode_entry(ENTRY_TYPE_DELETE, key, None);
        self.write_entry(&entry)
    }

    /// Appends a PUT to the WAL buffer WITHOUT syncing.
    /// Use this inside batch operations — call `flush()` once at the end.
    pub fn put_no_sync(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        let entry = self.encode_entry(ENTRY_TYPE_PUT, key, Some(value));
        self.writer.write_all(&entry)?;
        self.current_size
            .fetch_add(entry.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Appends a DELETE to the WAL buffer WITHOUT syncing.
    /// Use this inside batch operations — call `flush()` once at the end.
    pub fn delete_no_sync(&mut self, key: &[u8]) -> Result<()> {
        let entry = self.encode_entry(ENTRY_TYPE_DELETE, key, None);
        self.writer.write_all(&entry)?;
        self.current_size
            .fetch_add(entry.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Writes a batch of PUT entries with a SINGLE flush+fsync at the end.
    /// Compared to N individual puts, this is up to 1000x faster for large batches.
    pub fn write_puts_batch(&mut self, entries: &[(&[u8], &[u8])]) -> Result<()> {
        // Encode + buffer all entries — no syscall yet
        for &(key, value) in entries {
            let entry = self.encode_entry(ENTRY_TYPE_PUT, key, Some(value));
            self.writer.write_all(&entry)?;
            self.current_size
                .fetch_add(entry.len() as u64, Ordering::Relaxed);
        }
        // Single flush + optional fsync for the entire batch
        self.writer.flush()?;
        if self.sync_writes {
            self.writer.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Writes a batch of DELETE entries with a SINGLE flush+fsync at the end.
    pub fn write_deletes_batch(&mut self, keys: &[&[u8]]) -> Result<()> {
        for &key in keys {
            let entry = self.encode_entry(ENTRY_TYPE_DELETE, key, None);
            self.writer.write_all(&entry)?;
            self.current_size
                .fetch_add(entry.len() as u64, Ordering::Relaxed);
        }
        self.writer.flush()?;
        if self.sync_writes {
            self.writer.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Flushes buffered writes to disk.
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        if self.sync_writes {
            self.writer.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Returns the current WAL file size.
    pub fn size(&self) -> u64 {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Checks if the WAL should be rotated.
    pub fn should_rotate(&self) -> bool {
        self.current_size.load(Ordering::Relaxed) >= self.max_size
    }

    /// Returns the WAL file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Truncates the WAL after a successful memtable flush.
    ///
    /// Uses atomic rename(2) instead of truncate(2) for crash safety:
    /// - Writes an empty file to `wal.tmp`
    /// - Atomically renames it over `wal.log`
    /// - Reopens the new empty file for appending
    ///
    /// If the process crashes between segment creation and truncation,
    /// the WAL is replayed on next startup — duplicate entries are
    /// harmless because they produce the same memtable state.
    pub fn truncate(&mut self) -> Result<()> {
        // 1. Flush any buffered bytes to the OS page cache
        self.writer.flush()?;

        // 2. Create an empty replacement file at a temp path
        let temp_path = self.path.with_extension("tmp");
        {
            let new_file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_path)?;
            // Sync empty file to disk before rename so it survives a crash
            new_file.sync_all()?;
        }

        // 3. Atomic rename — POSIX rename(2) is crash-safe
        std::fs::rename(&temp_path, &self.path)?;

        // 4. Reopen the (now empty) WAL file for appending
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.writer = BufWriter::with_capacity(256 * 1024, file);
        self.current_size.store(0, Ordering::Release);

        Ok(())
    }

    /// Encodes a WAL entry.
    fn encode_entry(&self, entry_type: u8, key: &[u8], value: Option<&[u8]>) -> Vec<u8> {
        let value = value.unwrap_or(&[]);
        let payload_size = 1 + 2 + key.len() + 4 + value.len(); // type + key_len + key + value_len + value
        let total_size = 4 + payload_size + 4; // entry_len + payload + crc

        let mut buf = Vec::with_capacity(total_size);

        // Entry length (excluding this field and CRC)
        buf.extend_from_slice(&(payload_size as u32).to_le_bytes());

        // Entry type
        buf.push(entry_type);

        // Key
        buf.extend_from_slice(&(key.len() as u16).to_le_bytes());
        buf.extend_from_slice(key);

        // Value
        buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
        buf.extend_from_slice(value);

        // CRC over everything except the CRC itself
        let crc = compute_crc(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Writes an encoded entry to the WAL file.
    fn write_entry(&mut self, entry: &[u8]) -> Result<()> {
        // Sequential buffered write — already near-optimal for append-only WAL
        self.writer.write_all(entry)?;
        self.current_size
            .fetch_add(entry.len() as u64, Ordering::Relaxed);

        if self.sync_writes {
            self.writer.flush()?;
            self.writer.get_ref().sync_all()?;
        }

        Ok(())
    }
}

/// WAL entry recovered during replay.
#[derive(Debug)]
pub enum WalEntry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// Replays a WAL file and returns all entries in order.
pub fn replay_wal(path: &Path) -> Result<Vec<WalEntry>> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let file_size = file.metadata()?.len();
    if file_size == 0 {
        return Ok(Vec::new());
    }

    let mut data = Vec::with_capacity(file_size as usize);
    file.read_to_end(&mut data)?;
    drop(file);

    let mut entries = Vec::new();
    let mut pos = 0usize;
    let mut partial_writes = 0usize; // truncated at end of file
    let mut crc_failures = 0usize; // corrupted mid-file

    while pos + 4 < data.len() {
        let payload_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;

        let entry_start = pos;
        let payload_end = pos + 4 + payload_len;
        let crc_end = payload_end + 4;

        if crc_end > data.len() {
            // Entry extends past EOF — partial write from crash, safe to stop
            partial_writes += 1;
            break;
        }

        // Verify CRC
        let expected_crc = u32::from_le_bytes(data[payload_end..crc_end].try_into().unwrap());
        let actual_crc = compute_crc(&data[entry_start..payload_end]);

        if expected_crc != actual_crc {
            // Corruption — stop here, do not replay further
            crc_failures += 1;
            log::warn!(
                "WAL CRC mismatch at offset {} (expected={:#010x} actual={:#010x}); \
                 stopping replay with {} recovered entries",
                pos,
                expected_crc,
                actual_crc,
                entries.len()
            );
            break;
        }

        // Parse entry
        let entry_type = data[pos + 4];
        let key_len = u16::from_le_bytes([data[pos + 5], data[pos + 6]]) as usize;
        let key_start = pos + 7;
        let key_end = key_start + key_len;

        if key_end + 4 > payload_end {
            partial_writes += 1;
            break;
        }

        let key = data[key_start..key_end].to_vec();

        match entry_type {
            ENTRY_TYPE_PUT => {
                let value_len = u32::from_le_bytes([
                    data[key_end],
                    data[key_end + 1],
                    data[key_end + 2],
                    data[key_end + 3],
                ]) as usize;
                let value_start = key_end + 4;
                let value_end = value_start + value_len;

                if value_end > payload_end {
                    partial_writes += 1;
                    break;
                }

                entries.push(WalEntry::Put {
                    key,
                    value: data[value_start..value_end].to_vec(),
                });
            }
            ENTRY_TYPE_DELETE => {
                entries.push(WalEntry::Delete { key });
            }
            _ => {
                log::warn!(
                    "WAL unknown entry type {} at offset {}; stopping replay",
                    entry_type,
                    pos
                );
                break;
            }
        }

        pos = crc_end;
    }

    if !entries.is_empty() || partial_writes > 0 || crc_failures > 0 {
        log::info!(
            "WAL replay: {} entries recovered, {} partial writes, {} CRC failures ({} bytes scanned)",
            entries.len(), partial_writes, crc_failures, data.len()
        );
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> TorexConfig {
        TorexConfig::new(dir.path().join("test_db"))
    }

    #[test]
    fn test_wal_put_and_replay() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        {
            let mut wal = Wal::open(&config).unwrap();
            wal.put(b"key1", b"value1").unwrap();
            wal.put(b"key2", b"value2").unwrap();
            wal.flush().unwrap();
        }

        let entries = replay_wal(&config.path.join("wal.log")).unwrap();
        assert_eq!(entries.len(), 2);

        match &entries[0] {
            WalEntry::Put { key, value } => {
                assert_eq!(key, b"key1");
                assert_eq!(value, b"value1");
            }
            _ => panic!("expected Put"),
        }
    }

    #[test]
    fn test_wal_delete_and_replay() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        {
            let mut wal = Wal::open(&config).unwrap();
            wal.put(b"key1", b"value1").unwrap();
            wal.delete(b"key1").unwrap();
            wal.flush().unwrap();
        }

        let entries = replay_wal(&config.path.join("wal.log")).unwrap();
        assert_eq!(entries.len(), 2);

        match &entries[1] {
            WalEntry::Delete { key } => assert_eq!(key, b"key1"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn test_wal_truncate() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        let mut wal = Wal::open(&config).unwrap();
        wal.put(b"key1", b"value1").unwrap();
        wal.flush().unwrap();
        assert!(wal.size() > 0);

        wal.truncate().unwrap();
        assert_eq!(wal.size(), 0);

        // Verify the file on disk is actually empty (atomic rename worked)
        let file_size = std::fs::metadata(wal.path()).unwrap().len();
        assert_eq!(
            file_size, 0,
            "WAL file on disk should be empty after truncation"
        );

        // Should still be writable after truncation
        wal.put(b"key2", b"value2").unwrap();
        wal.flush().unwrap();
        assert!(wal.size() > 0);
    }

    #[test]
    fn test_wal_recovery_ignores_partial_writes() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        {
            let mut wal = Wal::open(&config).unwrap();
            wal.put(b"key1", b"value1").unwrap();
            wal.flush().unwrap();
        }

        // Append garbage to simulate partial write
        let wal_path = config.path.join("wal.log");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&wal_path)
            .unwrap();
        file.write_all(&[0xFF, 0xFF, 0xFF]).unwrap();

        let entries = replay_wal(&wal_path).unwrap();
        assert_eq!(entries.len(), 1); // Only the valid entry
    }
}
