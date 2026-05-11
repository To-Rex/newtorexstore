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

    /// Truncates the WAL (called after a successful memtable flush).
    pub fn truncate(&mut self) -> Result<()> {
        self.writer.get_ref().set_len(0)?;
        self.current_size.store(0, Ordering::Relaxed);
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
    let mut file = std::fs::File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    drop(file);

    let mut entries = Vec::new();
    let mut pos = 0;

    while pos + 4 < data.len() {
        // Read entry length
        let payload_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;

        let entry_start = pos;
        let payload_end = pos + 4 + payload_len;
        let crc_end = payload_end + 4;

        if crc_end > data.len() {
            // Partial write from crash — stop here
            break;
        }

        // Verify CRC
        let expected_crc = u32::from_le_bytes([
            data[payload_end],
            data[payload_end + 1],
            data[payload_end + 2],
            data[payload_end + 3],
        ]);
        let actual_crc = compute_crc(&data[entry_start..payload_end]);

        if expected_crc != actual_crc {
            // Corrupted entry — stop here
            break;
        }

        // Parse entry type
        let entry_type = data[pos + 4];
        let key_len = u16::from_le_bytes([data[pos + 5], data[pos + 6]]) as usize;
        let key_start = pos + 7;
        let key_end = key_start + key_len;

        if key_end + 4 > payload_end {
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
                    break;
                }

                let value = data[value_start..value_end].to_vec();
                entries.push(WalEntry::Put { key, value });
            }
            ENTRY_TYPE_DELETE => {
                entries.push(WalEntry::Delete { key });
            }
            _ => {
                // Unknown entry type — stop
                break;
            }
        }

        pos = crc_end;
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
