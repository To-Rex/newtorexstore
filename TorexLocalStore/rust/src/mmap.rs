//! Memory-mapped segment reader for zero-copy reads.
//!
//! Uses `memmap2` to map segment files into virtual memory, enabling:
//! - Zero-copy key/value access (no intermediate buffers)
//! - OS-level page caching (automatic hot data caching)
//! - Lazy loading (pages loaded on demand)
//! - Efficient sequential and random access patterns
//!
//! ## Memory Strategy
//!
//! Each segment is mmap'd independently. The OS manages page eviction
//! based on available RAM. For read-heavy workloads, frequently accessed
//! segments stay in page cache automatically.
//!
//! ## Safety
//!
//! - mmap regions are read-only; writes go through normal I/O
//! - Segment files are immutable after creation, so no data races
//! - Drop unmaps automatically via Mmap destructor

use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;

use crate::codec::{decode_entry_zero_copy, encoded_size, encoded_size_from_header};
use crate::error::{Result, TorexError};

const HEADER_SIZE: usize = 16;
const FOOTER_SIZE: usize = 16;

/// A memory-mapped segment for zero-copy reads.
///
/// This is the preferred way to read segment data — no intermediate
/// buffers, no syscalls per read, OS handles page cache automatically.
pub struct MmapSegment {
    /// The memory-mapped region.
    mmap: Mmap,

    /// Segment file size.
    file_size: u64,

    /// Number of entries.
    entry_count: u32,

    /// Sparse index loaded from the segment footer.
    index: Vec<(Vec<u8>, u64)>,
}

impl MmapSegment {
    /// Opens a segment file as a memory-mapped region.
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(path)?;

        let file_size = file.metadata()?.len();

        if file_size < (HEADER_SIZE + FOOTER_SIZE) as u64 {
            return Err(TorexError::Corruption(format!(
                "mmap segment too small: {:?}",
                path
            )));
        }

        // Safety: file is opened read-only, segment files are immutable
        let mmap = unsafe { Mmap::map(&file)? };

        // Verify magic bytes
        if &mmap[0..4] != &crate::MAGIC_BYTES {
            return Err(TorexError::Corruption(format!(
                "invalid magic in mmap segment: {:?}",
                path
            )));
        }

        let entry_count = u32::from_le_bytes([mmap[8], mmap[9], mmap[10], mmap[11]]);

        // Parse footer
        let footer_start = mmap.len() - FOOTER_SIZE;
        let index_offset = u64::from_le_bytes(
            mmap[footer_start..footer_start + 8].try_into().unwrap()
        );
        let index_size = u32::from_le_bytes(
            mmap[footer_start + 8..footer_start + 12].try_into().unwrap()
        );

        // Parse index entries from mmap
        let index = Self::parse_index(&mmap, index_offset as usize, index_size as usize)?;

        Ok(Self {
            mmap,
            file_size,
            entry_count,
            index,
        })
    }

    /// Parse the sparse index from the mmap'd data.
    fn parse_index(data: &[u8], start: usize, size: usize) -> Result<Vec<(Vec<u8>, u64)>> {
        let end = start + size;
        let mut entries = Vec::new();
        let mut pos = start;

        while pos + 2 < end {
            let key_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;

            if pos + key_len + 8 > end {
                break;
            }

            let key = data[pos..pos + key_len].to_vec();
            pos += key_len;

            let offset = u64::from_le_bytes(
                data[pos..pos + 8].try_into().unwrap()
            );
            pos += 8;

            entries.push((key, offset));
        }

        Ok(entries)
    }

    /// Looks up a key using binary search on the sparse index.
    /// Returns a zero-copy slice of the value bytes.
    pub fn get_zero_copy(&self, key: &[u8]) -> Result<Option<&[u8]>> {
        let idx = match self.index.binary_search_by(|(k, _)| k.as_slice().cmp(key)) {
            Ok(i) => i,
            Err(_) => return Ok(None),
        };

        let (_, offset) = &self.index[idx];
        self.read_entry_zero_copy(*offset as usize)
    }

    /// Looks up a key and returns an owned copy.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match self.get_zero_copy(key)? {
            Some(slice) => Ok(Some(slice.to_vec())),
            None => Ok(None),
        }
    }

    /// Reads an entry at the given offset, returning a zero-copy slice.
    fn read_entry_zero_copy(&self, offset: usize) -> Result<Option<&[u8]>> {
        if offset >= self.mmap.len() {
            return Ok(None);
        }

        let data = &self.mmap[offset..];

        // Read key_len and value_len to compute total entry size
        let (key_len, value_len) = encoded_size_from_header(data)?;
        let entry_size = encoded_size(key_len, value_len);

        if data.len() < entry_size {
            return Ok(None);
        }

        let entry_data = &data[..entry_size];

        // Verify CRC
        match decode_entry_zero_copy(entry_data) {
            Ok((_key, value)) => Ok(Some(value)),
            Err(TorexError::ChecksumMismatch { .. }) => {
                Err(TorexError::Corruption("checksum mismatch in mmap segment".into()))
            }
            Err(e) => Err(e),
        }
    }

    /// Returns the raw mmap data for advanced operations.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

    /// Returns the number of entries.
    #[inline]
    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Returns the file size.
    #[inline]
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Returns the sparse index.
    #[inline]
    pub fn index(&self) -> &[(Vec<u8>, u64)] {
        &self.index
    }

    /// Scans all entries, calling the provided function for each.
    /// Zero-copy: the key and value slices point directly into mmap memory.
    pub fn scan_entries<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(&[u8], &[u8]) -> bool,
    {
        let mut pos = HEADER_SIZE;
        let footer_start = self.mmap.len() - FOOTER_SIZE;

        while pos < footer_start {
            let remaining = footer_start - pos;
            if remaining < 10 {
                break;
            }

            let data = &self.mmap[pos..footer_start];

            let (key_len, value_len) = encoded_size_from_header(data)?;
            let entry_size = encoded_size(key_len, value_len);

            if data.len() < entry_size {
                break;
            }

            match decode_entry_zero_copy(&data[..entry_size]) {
                Ok((key, value)) => {
                    if !f(key, value) {
                        break;
                    }
                }
                Err(_) => break,
            }

            pos += entry_size;
        }

        Ok(())
    }
}

/// Thread-safe reference-counted mmap segment.
pub type SharedMmapSegment = Arc<MmapSegment>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::MemtableEntry;
    use crate::segment::Segment;
    use tempfile::TempDir;

    fn create_test_segment(dir: &TempDir) -> std::path::PathBuf {
        let path = dir.path().join("0.seg");
        let entries = vec![
            (b"key1".to_vec(), MemtableEntry::Put(b"value1".to_vec())),
            (b"key2".to_vec(), MemtableEntry::Put(b"value2".to_vec())),
            (b"key3".to_vec(), MemtableEntry::Put(b"value3".to_vec())),
        ];
        Segment::create(&path, 0, &entries).unwrap();
        path
    }

    #[test]
    fn test_mmap_open_and_get() {
        let dir = TempDir::new().unwrap();
        let path = create_test_segment(&dir);

        let mmap_seg = MmapSegment::open(&path).unwrap();
        assert_eq!(mmap_seg.entry_count(), 3);

        let result = mmap_seg.get(b"key2").unwrap();
        assert_eq!(result, Some(b"value2".to_vec()));
    }

    #[test]
    fn test_mmap_zero_copy() {
        let dir = TempDir::new().unwrap();
        let path = create_test_segment(&dir);

        let mmap_seg = MmapSegment::open(&path).unwrap();

        let result = mmap_seg.get_zero_copy(b"key1").unwrap();
        assert_eq!(result, Some(b"value1".as_slice()));
    }

    #[test]
    fn test_mmap_missing_key() {
        let dir = TempDir::new().unwrap();
        let path = create_test_segment(&dir);

        let mmap_seg = MmapSegment::open(&path).unwrap();
        let result = mmap_seg.get(b"nonexistent").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_mmap_scan_entries() {
        let dir = TempDir::new().unwrap();
        let path = create_test_segment(&dir);

        let mmap_seg = MmapSegment::open(&path).unwrap();

        let mut entries = Vec::new();
        mmap_seg.scan_entries(|key, value| {
            entries.push((key.to_vec(), value.to_vec()));
            true
        }).unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, b"key1");
        assert_eq!(entries[1].0, b"key2");
        assert_eq!(entries[2].0, b"key3");
    }
}
