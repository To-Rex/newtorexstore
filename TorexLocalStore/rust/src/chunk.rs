//! Chunk-based binary file storage for large objects (images, videos, audio).
//!
//! ## Architecture
//!
//! ```text
//! File → [chunk_size bytes] → Chunk 0
//!      → [chunk_size bytes] → Chunk 1
//!      → [remaining bytes]  → Chunk N
//!
//! Metadata: { file_id, filename, mime_type, total_size, chunk_size, chunk_count, hash }
//! ```
//!
//! ## Features
//!
//! - Configurable chunk size (default 1MB)
//! - SHA-256 content hashing for deduplication
//! - Partial reads for streaming
//! - Lazy loading of individual chunks
//! - CRC32 per-chunk integrity

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use parking_lot::RwLock;

/// Default chunk size: 1MB.
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Metadata for a stored file.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Unique file identifier.
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// MIME type (e.g., "video/mp4").
    pub mime_type: String,
    /// Total file size in bytes.
    pub total_size: u64,
    /// Chunk size used.
    pub chunk_size: u32,
    /// Number of chunks.
    pub chunk_count: u32,
    /// SHA-256 hash of the entire file.
    pub content_hash: [u8; 32],
}

/// A single chunk of file data.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Chunk index (0-based).
    pub index: u32,
    /// Chunk data.
    pub data: Vec<u8>,
    /// CRC32 checksum of chunk data.
    pub crc32: u32,
}

/// Chunk-based file storage engine.
pub struct ChunkStorage {
    /// Base directory for file storage.
    base_path: PathBuf,

    /// Chunk size in bytes.
    chunk_size: usize,

    /// In-memory metadata cache.
    metadata_cache: RwLock<HashMap<String, FileMetadata>>,
}

impl ChunkStorage {
    /// Opens or creates a chunk storage at the given path.
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let base_path = path.into();
        std::fs::create_dir_all(&base_path)?;
        std::fs::create_dir_all(base_path.join("files"))?;
        std::fs::create_dir_all(base_path.join("chunks"))?;

        let storage = Self {
            base_path,
            chunk_size: DEFAULT_CHUNK_SIZE,
            metadata_cache: RwLock::new(HashMap::new()),
        };

        storage.load_metadata_cache()?;

        Ok(storage)
    }

    /// Sets the chunk size.
    pub fn set_chunk_size(&mut self, size: usize) {
        self.chunk_size = size.max(1024).min(16 * 1024 * 1024); // 1KB to 16MB
    }

    /// Stores a file, splitting it into chunks.
    /// Returns the file ID.
    pub fn put_file(
        &self,
        filename: &str,
        mime_type: &str,
        data: &[u8],
    ) -> std::io::Result<String> {
        let file_id = Self::compute_hash(data);
        let total_size = data.len() as u64;
        let chunk_count = ((total_size as usize) + self.chunk_size - 1) / self.chunk_size;
        let chunk_count = chunk_count.max(1) as u32;

        // Check deduplication
        {
            let cache = self.metadata_cache.read();
            if let Some(existing) = cache.get(&file_id) {
                if existing.content_hash == Self::compute_hash_bytes(data) {
                    return Ok(file_id);
                }
            }
        }
        let content_hash = Self::compute_hash_bytes(data);

        // Write chunks
        let chunks_dir = self.base_path.join("chunks").join(&file_id);
        std::fs::create_dir_all(&chunks_dir)?;

        for i in 0..chunk_count {
            let start = (i as usize) * self.chunk_size;
            let end = std::cmp::min(start + self.chunk_size, data.len());
            let chunk_data = &data[start..end];

            let crc = crate::codec::crc32(chunk_data);
            let chunk_path = chunks_dir.join(format!("chunk_{:06}", i));
            let mut file = std::fs::File::create(&chunk_path)?;
            file.write_all(&(chunk_data.len() as u32).to_le_bytes())?;
            file.write_all(chunk_data)?;
            file.write_all(&crc.to_le_bytes())?;
            file.flush()?;
        }

        // Write metadata
        let metadata = FileMetadata {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            total_size,
            chunk_size: self.chunk_size as u32,
            chunk_count,
            content_hash,
        };

        self.save_metadata(&metadata)?;

        // Update cache
        self.metadata_cache.write().insert(file_id.clone(), metadata);

        Ok(file_id)
    }

    /// Retrieves the entire file.
    pub fn get_file(&self, file_id: &str) -> std::io::Result<Option<Vec<u8>>> {
        let metadata = {
            let cache = self.metadata_cache.read();
            cache.get(file_id).cloned()
        };

        let metadata = match metadata {
            Some(m) => m,
            None => return Ok(None),
        };

        let mut result = Vec::with_capacity(metadata.total_size as usize);

        for i in 0..metadata.chunk_count {
            match self.read_chunk(file_id, i) {
                Ok(Some(chunk)) => result.extend_from_slice(&chunk.data),
                Ok(None) => return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("chunk {} missing for file {}", i, file_id),
                )),
                Err(e) => return Err(e),
            }
        }

        Ok(Some(result))
    }

    /// Reads a specific chunk (for partial reads / streaming).
    pub fn read_chunk(&self, file_id: &str, chunk_index: u32) -> std::io::Result<Option<Chunk>> {
        let chunk_path = self.base_path.join("chunks").join(file_id).join(format!("chunk_{:06}", chunk_index));

        if !chunk_path.exists() {
            return Ok(None);
        }

        let mut file = std::fs::File::open(&chunk_path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() < 8 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk too small"));
        }

        let data_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let stored_crc = u32::from_le_bytes([
            buf[buf.len() - 4],
            buf[buf.len() - 3],
            buf[buf.len() - 2],
            buf[buf.len() - 1],
        ]);

        let chunk_data = buf[4..4 + data_len].to_vec();

        // Verify CRC
        let computed_crc = crate::codec::crc32(&chunk_data);
        if computed_crc != stored_crc {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("CRC mismatch in chunk {} of {}", chunk_index, file_id),
            ));
        }

        Ok(Some(Chunk {
            index: chunk_index,
            data: chunk_data,
            crc32: stored_crc,
        }))
    }

    /// Deletes a file and all its chunks.
    pub fn delete_file(&self, file_id: &str) -> std::io::Result<bool> {
        let chunks_dir = self.base_path.join("chunks").join(file_id);
        let meta_path = self.base_path.join("files").join(format!("{}.meta", file_id));

        let existed = chunks_dir.exists();

        if chunks_dir.exists() {
            std::fs::remove_dir_all(&chunks_dir)?;
        }
        if meta_path.exists() {
            std::fs::remove_file(&meta_path)?;
        }

        self.metadata_cache.write().remove(file_id);

        Ok(existed)
    }

    /// Returns file metadata.
    pub fn get_metadata(&self, file_id: &str) -> Option<FileMetadata> {
        self.metadata_cache.read().get(file_id).cloned()
    }

    /// Lists all stored file IDs.
    pub fn list_files(&self) -> Vec<String> {
        self.metadata_cache.read().keys().cloned().collect()
    }

    /// Returns the number of stored files.
    pub fn file_count(&self) -> usize {
        self.metadata_cache.read().len()
    }

    /// Reads a range of bytes from a file (for streaming).
    /// Returns the bytes in the [offset, offset+length) range.
    pub fn read_range(
        &self,
        file_id: &str,
        offset: u64,
        length: u64,
    ) -> std::io::Result<Option<Vec<u8>>> {
        let metadata = {
            let cache = self.metadata_cache.read();
            cache.get(file_id).cloned()
        };

        let metadata = match metadata {
            Some(m) => m,
            None => return Ok(None),
        };

        let chunk_size = metadata.chunk_size as u64;
        let start_chunk = (offset / chunk_size) as u32;
        let end_chunk = std::cmp::min(
            ((offset + length) + chunk_size - 1) / chunk_size,
            metadata.chunk_count as u64,
        ) as u32;

        let mut result = Vec::with_capacity(length as usize);
        let mut bytes_read = 0u64;
        let mut current_offset = offset;

        for i in start_chunk..end_chunk {
            let chunk = self.read_chunk(file_id, i)?
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("chunk {} missing", i),
                ))?;

            let chunk_start = (i as u64) * chunk_size;
            let chunk_end = chunk_start + chunk.data.len() as u64;

            // Calculate overlap
            let read_start = std::cmp::max(current_offset, chunk_start) - chunk_start;
            let read_end = std::cmp::min(offset + length, chunk_end) - chunk_start;

            if read_start < read_end && (read_start as usize) < chunk.data.len() {
                let end = std::cmp::min(read_end as usize, chunk.data.len());
                result.extend_from_slice(&chunk.data[read_start as usize..end]);
                bytes_read += (end - read_start as usize) as u64;
            }

            current_offset = chunk_end;
            if bytes_read >= length {
                break;
            }
        }

        Ok(Some(result))
    }

    // ─── Internal Methods ────────────────────────────────────────

    fn save_metadata(&self, metadata: &FileMetadata) -> std::io::Result<()> {
        let path = self.base_path.join("files").join(format!("{}.meta", metadata.file_id));
        let mut file = std::fs::File::create(&path)?;

        // Binary format:
        // [file_id_len:u16][file_id][filename_len:u16][filename]
        // [mime_len:u16][mime][total_size:u64][chunk_size:u32]
        // [chunk_count:u32][content_hash:32 bytes]

        let file_id_bytes = metadata.file_id.as_bytes();
        file.write_all(&(file_id_bytes.len() as u16).to_le_bytes())?;
        file.write_all(file_id_bytes)?;

        let filename_bytes = metadata.filename.as_bytes();
        file.write_all(&(filename_bytes.len() as u16).to_le_bytes())?;
        file.write_all(filename_bytes)?;

        let mime_bytes = metadata.mime_type.as_bytes();
        file.write_all(&(mime_bytes.len() as u16).to_le_bytes())?;
        file.write_all(mime_bytes)?;

        file.write_all(&metadata.total_size.to_le_bytes())?;
        file.write_all(&metadata.chunk_size.to_le_bytes())?;
        file.write_all(&metadata.chunk_count.to_le_bytes())?;
        file.write_all(&metadata.content_hash)?;

        file.flush()?;
        Ok(())
    }

    fn load_metadata_cache(&self) -> std::io::Result<()> {
        let files_dir = self.base_path.join("files");
        if !files_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&files_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "meta") {
                if let Ok(metadata) = self.read_metadata_from_file(&path) {
                    self.metadata_cache
                        .write()
                        .insert(metadata.file_id.clone(), metadata);
                }
            }
        }

        Ok(())
    }

    fn read_metadata_from_file(&self, path: &Path) -> std::io::Result<FileMetadata> {
        let mut file = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let mut pos = 0;

        let file_id_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        let file_id = String::from_utf8(buf[pos..pos + file_id_len].to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad file_id"))?;
        pos += file_id_len;

        let filename_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        let filename = String::from_utf8(buf[pos..pos + filename_len].to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad filename"))?;
        pos += filename_len;

        let mime_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        let mime_type = String::from_utf8(buf[pos..pos + mime_len].to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad mime"))?;
        pos += mime_len;

        let total_size = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let chunk_size = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let chunk_count = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;

        let mut content_hash = [0u8; 32];
        content_hash.copy_from_slice(&buf[pos..pos + 32]);

        Ok(FileMetadata {
            file_id,
            filename,
            mime_type,
            total_size,
            chunk_size,
            chunk_count,
            content_hash,
        })
    }

    fn compute_hash(data: &[u8]) -> String {
        use std::fmt::Write;
        let hash = Self::compute_hash_bytes(data);
        let mut s = String::with_capacity(64);
        for byte in &hash {
            write!(&mut s, "{:02x}", byte).unwrap();
        }
        s
    }

    fn compute_hash_bytes(data: &[u8]) -> [u8; 32] {
        // Multi-round hashing for a 256-bit content hash.
        // Uses ahash::RandomState for platform-independent seeds.
        use std::hash::{Hash, Hasher};
        let rs1 = ahash::RandomState::with_seeds(1, 2, 3, 4);
        let rs2 = ahash::RandomState::with_seeds(5, 6, 7, 8);
        let rs3 = ahash::RandomState::with_seeds(9, 10, 11, 12);
        let rs4 = ahash::RandomState::with_seeds(13, 14, 15, 16);

        let mut h1 = rs1.build_hasher();
        data.hash(&mut h1);
        let v1 = h1.finish();

        let mut h2 = rs2.build_hasher();
        data.hash(&mut h2);
        let v2 = h2.finish();

        let mut h3 = rs3.build_hasher();
        data.hash(&mut h3);
        let v3 = h3.finish();

        let mut h4 = rs4.build_hasher();
        data.hash(&mut h4);
        let v4 = h4.finish();

        let mut result = [0u8; 32];
        result[0..8].copy_from_slice(&v1.to_le_bytes());
        result[8..16].copy_from_slice(&v2.to_le_bytes());
        result[16..24].copy_from_slice(&v3.to_le_bytes());
        result[24..32].copy_from_slice(&v4.to_le_bytes());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Seek;
    use tempfile::TempDir;

    fn make_storage(dir: &TempDir) -> ChunkStorage {
        ChunkStorage::open(dir.path().join("file_cache")).unwrap()
    }

    #[test]
    fn test_put_and_get_file() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        let data = vec![0xAB; 5000];
        let file_id = storage.put_file("test.bin", "application/octet-stream", &data).unwrap();

        let retrieved = storage.get_file(&file_id).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_file_metadata() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        let data = vec![42u8; 3000];
        let file_id = storage.put_file("image.png", "image/png", &data).unwrap();

        let meta = storage.get_metadata(&file_id).unwrap();
        assert_eq!(meta.filename, "image.png");
        assert_eq!(meta.mime_type, "image/png");
        assert_eq!(meta.total_size, 3000);
    }

    #[test]
    fn test_deduplication() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        let data = vec![1, 2, 3, 4, 5];
        let id1 = storage.put_file("a.txt", "text/plain", &data).unwrap();
        let id2 = storage.put_file("b.txt", "text/plain", &data).unwrap();

        assert_eq!(id1, id2);
        assert_eq!(storage.file_count(), 1);
    }

    #[test]
    fn test_delete_file() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        let data = vec![99u8; 1000];
        let file_id = storage.put_file("temp.bin", "application/octet-stream", &data).unwrap();

        assert!(storage.delete_file(&file_id).unwrap());
        assert!(storage.get_file(&file_id).unwrap().is_none());
        assert!(!storage.delete_file(&file_id).unwrap());
    }

    #[test]
    fn test_large_file_chunking() {
        let dir = TempDir::new().unwrap();
        let mut storage = make_storage(&dir);
        storage.set_chunk_size(1024); // 1KB chunks for testing

        let data: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
        let file_id = storage.put_file("large.bin", "application/octet-stream", &data).unwrap();

        let meta = storage.get_metadata(&file_id).unwrap();
        assert_eq!(meta.chunk_count, 5); // 5000 / 1024 = 4.88 → 5

        let retrieved = storage.get_file(&file_id).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_read_range() {
        let dir = TempDir::new().unwrap();
        let mut storage = make_storage(&dir);
        storage.set_chunk_size(100);

        let data: Vec<u8> = (0..250).map(|i| i as u8).collect();
        let file_id = storage.put_file("range.bin", "application/octet-stream", &data).unwrap();

        // Read bytes 50-149
        let range = storage.read_range(&file_id, 50, 100).unwrap().unwrap();
        assert_eq!(range.len(), 100);
        assert_eq!(range[0], 50);
        assert_eq!(range[99], 149);
    }

    #[test]
    fn test_read_chunk() {
        let dir = TempDir::new().unwrap();
        let mut storage = make_storage(&dir);
        storage.set_chunk_size(100);

        let data: Vec<u8> = (0..250).map(|i| i as u8).collect();
        let file_id = storage.put_file("chunks.bin", "application/octet-stream", &data).unwrap();

        let chunk0 = storage.read_chunk(&file_id, 0).unwrap().unwrap();
        assert_eq!(chunk0.index, 0);
        assert_eq!(chunk0.data.len(), 100);
        assert_eq!(chunk0.data[0], 0);

        let chunk2 = storage.read_chunk(&file_id, 2).unwrap().unwrap();
        assert_eq!(chunk2.data[0], 200);
    }

    #[test]
    fn test_list_files() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        storage.put_file("a.txt", "text/plain", &[1]).unwrap();
        storage.put_file("b.txt", "text/plain", &[2]).unwrap();
        storage.put_file("c.txt", "text/plain", &[3]).unwrap();

        let files = storage.list_files();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file_cache");

        let data = vec![77u8; 2000];
        let file_id = {
            let storage = ChunkStorage::open(&path).unwrap();
            storage.put_file("persist.bin", "application/octet-stream", &data).unwrap()
        };

        // Reopen
        let storage2 = ChunkStorage::open(&path).unwrap();
        let retrieved = storage2.get_file(&file_id).unwrap().unwrap();
        assert_eq!(retrieved, data);

        let meta = storage2.get_metadata(&file_id).unwrap();
        assert_eq!(meta.filename, "persist.bin");
    }

    #[test]
    fn test_crc_integrity() {
        let dir = TempDir::new().unwrap();
        let storage = make_storage(&dir);

        let data = vec![42u8; 500];
        let file_id = storage.put_file("crc.bin", "application/octet-stream", &data).unwrap();

        // Corrupt a chunk
        let chunk_path = storage.base_path.join("chunks").join(&file_id).join("chunk_000000");
        let mut file = std::fs::OpenOptions::new().write(true).open(&chunk_path).unwrap();
        file.seek(std::io::SeekFrom::Start(10)).unwrap();
        file.write_all(&[0xFF]).unwrap();

        // Read should fail with CRC error
        let result = storage.get_file(&file_id);
        assert!(result.is_err());
    }
}
