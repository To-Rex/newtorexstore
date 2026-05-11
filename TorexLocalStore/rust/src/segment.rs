//! Immutable segment file management.
//!
//! Segments are sorted, immutable files written when a memtable is flushed.
//! Each segment contains sorted key-value entries with an index for fast lookups.
//!
//! ## Segment File Format
//!
//! ```text
//! [Header: 16 bytes]
//!   magic: [u8; 4] = "TRXS"
//!   version: u32
//!   entry_count: u32
//!   flags: u32
//!
//! [Data Block]
//!   entries: sorted key-value pairs (codec encoded)
//!
//! [Index Block]
//!   index_entries: [key_len: u16, key, offset: u64]
//!
//! [Footer: 16 bytes]
//!   index_offset: u64
//!   index_size: u32
//!   crc32: u32
//! ```

use crate::bloom::BloomFilter;
use memmap2::{Mmap, MmapOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::codec::{compute_crc, decode_entry, encode_entry, encoded_size};
use crate::error::{Result, TorexError};
use crate::memtable::MemtableEntry;

/// Segment file header size.
const HEADER_SIZE: usize = 16;

/// Footer size: index_offset(8) + index_size(4) + crc(4) = 16 bytes.
const FOOTER_SIZE: usize = 16;

/// A handle to an immutable segment on disk.
#[derive(Debug)]
pub struct Segment {
    /// Memory-mapped file for zero-copy access.
    mmap: Option<Mmap>,
    /// Bloom filter for O(1) negative lookups — avoids binary search on misses.
    bloom: BloomFilter,
    /// Filesystem path to the segment file.
    pub path: PathBuf,

    /// Segment ID (monotonically increasing).
    pub id: u64,

    /// Number of entries in this segment.
    pub entry_count: u32,

    /// File size in bytes.
    pub file_size: u64,

    /// Sparse index: (key -> file_offset).
    pub index: Vec<(Vec<u8>, u64)>,
}

impl Segment {
    /// Creates a new segment by flushing a memtable to disk.
    pub fn create(path: &Path, id: u64, entries: &[(Vec<u8>, MemtableEntry)]) -> Result<Self> {
        let mut buf = Vec::new();

        // Write header
        buf.extend_from_slice(&crate::MAGIC_BYTES);
        buf.extend_from_slice(&crate::FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // entry_count placeholder
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags

        let mut index_entries: Vec<(Vec<u8>, u64)> = Vec::new();
        let mut entry_count: u32 = 0;

        for (key, mem_entry) in entries {
            match mem_entry {
                MemtableEntry::Put(value) => {
                    let offset = buf.len() as u64;
                    let encoded = encode_entry(key, value);
                    index_entries.push((key.clone(), offset));
                    entry_count += 1;
                    buf.extend_from_slice(&encoded);
                }
                MemtableEntry::Delete => {
                    let offset = buf.len() as u64;
                    let encoded = encode_entry(key, &[]);
                    index_entries.push((key.clone(), offset));
                    entry_count += 1;
                    buf.extend_from_slice(&encoded);
                }
            }
        }

        // Write index
        let index_offset = buf.len() as u64;
        for (key, offset) in &index_entries {
            let key_len = key.len() as u16;
            buf.extend_from_slice(&key_len.to_le_bytes());
            buf.extend_from_slice(key);
            buf.extend_from_slice(&offset.to_le_bytes());
        }
        let index_size = (buf.len() as u64 - index_offset) as u32;

        // Write footer
        buf.extend_from_slice(&index_offset.to_le_bytes());
        buf.extend_from_slice(&index_size.to_le_bytes());

        // Compute CRC over everything so far
        let crc = compute_crc(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        // Update entry_count in header
        let entry_count_bytes = entry_count.to_le_bytes();
        buf[8..12].copy_from_slice(&entry_count_bytes);

        // Write to file (opened read+write so mmap can read it back)
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(&buf)?;
        file.flush()?;

        // SAFETY: file is freshly written, no other writers, mmap covers full file
        let mmap = unsafe { MmapOptions::new().map(&file).ok() };

        // Build bloom filter from all inserted keys — O(1) miss detection on reads
        let mut bloom = BloomFilter::new(entry_count.max(1) as usize);
        for (key, _) in &index_entries {
            bloom.insert(key);
        }

        let file_size = buf.len() as u64;

        Ok(Segment {
            mmap,
            bloom,
            path: path.to_path_buf(),
            id,
            entry_count,
            file_size,
            index: index_entries,
        })
    }

    /// Opens an existing segment and reads its index.
    pub fn open(path: &Path, id: u64) -> Result<Self> {
        let mut file = std::fs::OpenOptions::new().read(true).open(path)?;
        let file_size = file.metadata()?.len();

        // Read entire file into memory for parsing
        file.seek(SeekFrom::Start(0))?;
        let mut data = Vec::with_capacity(file_size as usize);
        file.read_to_end(&mut data)?;
        // SAFETY: file is read-only, no concurrent writers
        let mmap = unsafe { MmapOptions::new().map(&file).ok() };
        drop(file);

        if data.len() < HEADER_SIZE + FOOTER_SIZE {
            return Err(TorexError::Corruption(format!(
                "segment file too small: {:?}",
                path
            )));
        }

        // Verify magic
        if &data[0..4] != &crate::MAGIC_BYTES {
            return Err(TorexError::Corruption(format!(
                "invalid magic bytes in segment: {:?}",
                path
            )));
        }

        let _version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let entry_count = u32::from_le_bytes(data[8..12].try_into().unwrap());

        // Read footer (last 16 bytes)
        let footer_start = data.len() - FOOTER_SIZE;
        let index_offset =
            u64::from_le_bytes(data[footer_start..footer_start + 8].try_into().unwrap());
        let index_size = u32::from_le_bytes(
            data[footer_start + 8..footer_start + 12]
                .try_into()
                .unwrap(),
        );
        let _stored_crc = u32::from_le_bytes(
            data[footer_start + 12..footer_start + 16]
                .try_into()
                .unwrap(),
        );

        // Read index entries
        let index_data_start = index_offset as usize;
        let index_data_end = index_data_start + index_size as usize;

        if index_data_end > footer_start {
            return Err(TorexError::Corruption(format!(
                "index extends into footer: {:?}",
                path
            )));
        }

        let mut index_entries = Vec::new();
        let mut pos = index_data_start;
        while pos + 2 < index_data_end {
            let key_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;

            if pos + key_len + 8 > index_data_end {
                break;
            }

            let key = data[pos..pos + key_len].to_vec();
            pos += key_len;

            let offset = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
            pos += 8;

            index_entries.push((key, offset));
        }

        // Rebuild bloom filter from index — fast to build, avoids storing in file
        let mut bloom = BloomFilter::new(index_entries.len().max(1));
        for (key, _) in &index_entries {
            bloom.insert(key);
        }

        Ok(Segment {
            mmap,
            bloom,
            path: path.to_path_buf(),
            id,
            entry_count,
            file_size,
            index: index_entries,
        })
    }

    /// Looks up a key in this segment using binary search on the index.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // O(1) bloom check — definitively eliminates ~99% of misses
        // with no binary search and no mmap access
        if !self.bloom.might_contain(key) {
            return Ok(None);
        }

        let idx = match self.index.binary_search_by(|(k, _)| k.as_slice().cmp(key)) {
            Ok(i) => i,
            Err(_) => return Ok(None),
        };

        let (_, offset) = &self.index[idx];
        self.read_entry_at(*offset)
    }

    /// Reads and decodes an entry at the given file offset.
    /// Uses mmap for zero-copy access when available — no syscall on the hot path.
    fn read_entry_at(&self, offset: u64) -> Result<Option<Vec<u8>>> {
        let offset = offset as usize;

        if let Some(ref mmap) = self.mmap {
            // Zero-copy: slice directly into mapped memory, no heap alloc
            if offset >= mmap.len() {
                return Ok(None);
            }
            let end = (offset + 65536).min(mmap.len());
            return match decode_entry(&mmap[offset..end]) {
                Ok((_key, value)) => Ok(Some(value)),
                Err(TorexError::ChecksumMismatch { .. }) => Err(TorexError::Corruption(format!(
                    "checksum mismatch in segment: {:?}",
                    self.path
                ))),
                Err(e) => Err(e),
            };
        }

        // Fallback: file I/O (when mmap not available)
        let mut file = std::fs::OpenOptions::new().read(true).open(&self.path)?;
        file.seek(SeekFrom::Start(offset as u64))?;
        let mut buf = vec![0u8; 4096];
        let bytes_read = file.read(&mut buf)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        buf.truncate(bytes_read);
        match decode_entry(&buf) {
            Ok((_key, value)) => Ok(Some(value)),
            Err(TorexError::ChecksumMismatch { .. }) => Err(TorexError::Corruption(format!(
                "checksum mismatch in segment: {:?}",
                self.path
            ))),
            Err(e) => Err(e),
        }
    }

    /// Returns all key-value pairs in this segment (for compaction/scan).
    /// Uses mmap for zero-copy bulk reads when available.
    pub fn read_all(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if let Some(ref mmap) = self.mmap {
            // Zero-copy: parse directly from mapped memory slice
            let start = HEADER_SIZE;
            let end = mmap.len().saturating_sub(FOOTER_SIZE);
            if start >= end {
                return Ok(Vec::new());
            }
            return Self::parse_entries_from_slice(&mmap[start..end]);
        }

        // Fallback: file I/O
        let mut file = std::fs::OpenOptions::new().read(true).open(&self.path)?;
        let file_size = file.metadata()?.len();
        file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let data_len = (file_size as usize).saturating_sub(FOOTER_SIZE + HEADER_SIZE);
        let mut data = vec![0u8; data_len];
        file.read_exact(&mut data)?;
        Self::parse_entries_from_slice(&data)
    }

    /// Parses raw key-value entries from a contiguous data slice.
    /// Shared by both the mmap and file-I/O paths to avoid duplication.
    fn parse_entries_from_slice(data: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut entries = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            let entry_size = {
                if pos + 6 > data.len() {
                    break;
                }
                let key_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                if pos + 2 + key_len + 4 > data.len() {
                    break;
                }
                let value_len = u32::from_le_bytes([
                    data[pos + 2 + key_len],
                    data[pos + 3 + key_len],
                    data[pos + 4 + key_len],
                    data[pos + 5 + key_len],
                ]) as usize;
                encoded_size(key_len, value_len)
            };

            if pos + entry_size > data.len() {
                break;
            }

            match decode_entry(&data[pos..pos + entry_size]) {
                Ok((key, value)) => {
                    entries.push((key, value));
                    pos += entry_size;
                }
                Err(_) => break,
            }
        }

        Ok(entries)
    }
}

/// Manages the set of segment files for a collection.
pub struct SegmentManager {
    /// Directory containing segment files.
    directory: PathBuf,

    /// Active segments, sorted by ID (newest last).
    segments: Vec<Segment>,

    /// Next segment ID.
    next_id: u64,
}

impl SegmentManager {
    /// Creates a new segment manager.
    pub fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;

        let mut segments = Vec::new();
        let mut max_id = 0;

        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "seg") {
                let file_name = path.file_stem().unwrap().to_string_lossy();
                if let Ok(id) = file_name.parse::<u64>() {
                    match Segment::open(&path, id) {
                        Ok(seg) => {
                            max_id = max_id.max(id);
                            segments.push(seg);
                        }
                        Err(e) => {
                            log::warn!("Failed to open segment {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        segments.sort_by_key(|s| s.id);

        Ok(Self {
            directory,
            segments,
            next_id: max_id + 1,
        })
    }

    /// Creates a new segment from a flushed memtable.
    pub fn create_segment(&mut self, entries: &[(Vec<u8>, MemtableEntry)]) -> Result<&Segment> {
        let id = self.next_id;
        self.next_id += 1;

        let path = self.directory.join(format!("{}.seg", id));
        let segment = Segment::create(&path, id, entries)?;

        self.segments.push(segment);
        Ok(self.segments.last().unwrap())
    }

    /// Returns all segments (newest first for reads).
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Returns segments eligible for compaction.
    pub fn segments_for_compaction(&self, threshold_count: usize) -> Vec<&Segment> {
        if self.segments.len() <= threshold_count {
            return Vec::new();
        }
        self.segments[..self.segments.len() - threshold_count]
            .iter()
            .collect()
    }

    /// Removes segments that have been compacted.
    pub fn remove_segments(&mut self, ids: &[u64]) -> Result<()> {
        for id in ids {
            if let Some(pos) = self.segments.iter().position(|s| s.id == *id) {
                let segment = self.segments.remove(pos);
                if std::fs::exists(&segment.path)? {
                    std::fs::remove_file(&segment.path)?;
                }
            }
        }
        Ok(())
    }

    /// Looks up a key across all segments (newest first).
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        for segment in self.segments.iter().rev() {
            match segment.get(key) {
                Ok(Some(value)) => return Ok(Some(value)),
                Ok(None) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_segment_create_and_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("0.seg");

        let entries = vec![
            (b"key1".to_vec(), MemtableEntry::Put(b"value1".to_vec())),
            (b"key2".to_vec(), MemtableEntry::Put(b"value2".to_vec())),
            (b"key3".to_vec(), MemtableEntry::Put(b"value3".to_vec())),
        ];

        let segment = Segment::create(&path, 0, &entries).unwrap();
        assert_eq!(segment.entry_count, 3);

        let result = segment.get(b"key2").unwrap();
        assert_eq!(result, Some(b"value2".to_vec()));
    }

    #[test]
    fn test_segment_missing_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("0.seg");

        let entries = vec![(b"key1".to_vec(), MemtableEntry::Put(b"value1".to_vec()))];

        let segment = Segment::create(&path, 0, &entries).unwrap();
        let result = segment.get(b"nonexistent").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_segment_manager() {
        let dir = TempDir::new().unwrap();
        let seg_dir = dir.path().join("segments");
        let mut manager = SegmentManager::new(seg_dir).unwrap();

        let entries = vec![(b"key1".to_vec(), MemtableEntry::Put(b"value1".to_vec()))];

        let seg = manager.create_segment(&entries).unwrap();
        let created_id = seg.id;

        let result = manager.get(b"key1").unwrap();
        assert_eq!(result, Some(b"value1".to_vec()));

        // Verify segment is in the manager
        assert_eq!(manager.segments().len(), 1);
        assert_eq!(manager.segments()[0].id, created_id);
    }

    #[test]
    fn test_segment_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("42.seg");

        let entries = vec![
            (b"key1".to_vec(), MemtableEntry::Put(b"value1".to_vec())),
            (b"key2".to_vec(), MemtableEntry::Put(b"value2".to_vec())),
        ];

        Segment::create(&path, 42, &entries).unwrap();

        let segment = Segment::open(&path, 42).unwrap();
        assert_eq!(segment.entry_count, 2);
        assert_eq!(segment.id, 42);

        let result = segment.get(b"key1").unwrap();
        assert_eq!(result, Some(b"value1".to_vec()));
    }
}
