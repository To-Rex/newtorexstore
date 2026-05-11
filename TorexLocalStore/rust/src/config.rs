//! Configuration for the Torex storage engine.
//!
//! All tuning parameters are centralized here for easy optimization.

use std::path::PathBuf;

/// Default memtable size before flush (4 MB).
pub const DEFAULT_MEMTABLE_SIZE: usize = 4 * 1024 * 1024;

/// Default WAL max file size before rotation (8 MB).
pub const DEFAULT_WAL_MAX_SIZE: u64 = 8 * 1024 * 1024;

/// Default segment max size (16 MB).
pub const DEFAULT_SEGMENT_MAX_SIZE: u64 = 16 * 1024 * 1024;

/// Default number of segments to compact at once.
pub const DEFAULT_COMPACTION_THRESHOLD: usize = 4;

/// Default block size for mmap reads (4 KB).
pub const DEFAULT_BLOCK_SIZE: usize = 4 * 1024;

/// Default bloom filter expected items.
pub const DEFAULT_BLOOM_EXPECTED_ITEMS: usize = 100_000;

/// Default bloom filter false positive rate.
pub const DEFAULT_BLOOM_FP_RATE: f64 = 0.01;

/// Storage engine configuration.
#[derive(Debug, Clone)]
pub struct TorexConfig {
    /// Base directory for all storage files.
    pub path: PathBuf,

    /// Maximum memtable size in bytes before flushing to segment.
    pub memtable_size: usize,

    /// Maximum WAL file size before rotation.
    pub wal_max_size: u64,

    /// Maximum segment file size before creating a new one.
    pub segment_max_size: u64,

    /// Number of segments to trigger compaction.
    pub compaction_threshold: usize,

    /// Block size for index and mmap reads.
    pub block_size: usize,

    /// Whether to sync WAL on every write (durable but slower).
    pub sync_writes: bool,

    /// Whether to use compression for segments.
    pub compression: bool,

    /// Number of background worker threads.
    pub worker_threads: usize,

    /// Whether to verify checksums on reads.
    pub verify_checksums: bool,
}

impl TorexConfig {
    /// Creates a new config with the given path and defaults.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            memtable_size: DEFAULT_MEMTABLE_SIZE,
            wal_max_size: DEFAULT_WAL_MAX_SIZE,
            segment_max_size: DEFAULT_SEGMENT_MAX_SIZE,
            compaction_threshold: DEFAULT_COMPACTION_THRESHOLD,
            block_size: DEFAULT_BLOCK_SIZE,
            sync_writes: true,
            compression: true,
            worker_threads: 2,
            verify_checksums: true,
        }
    }

    /// Creates a config optimized for maximum throughput (benchmark / write-heavy workloads).
    /// - 64 MB memtable (fewer flushes)
    /// - No fsync on every write (async durability)
    /// - No compression overhead
    /// - No checksum verification on reads
    /// - Large WAL buffer
    pub fn high_throughput(path: impl Into<PathBuf>) -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .max(4);
        Self {
            path: path.into(),
            memtable_size: 64 * 1024 * 1024, // 64 MB — flush less often
            wal_max_size: 256 * 1024 * 1024, // 256 MB WAL before rotation
            segment_max_size: 256 * 1024 * 1024, // 256 MB segments
            compaction_threshold: 16,
            block_size: 64 * 1024, // 64 KB blocks
            sync_writes: false,    // async durability — no fsync per write
            compression: false,    // skip LZ4 on hot path
            worker_threads: cpus,
            verify_checksums: false, // skip CRC on reads
        }
    }

    /// Creates a config optimized for minimal memory usage.
    pub fn low_memory(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            memtable_size: 1 * 1024 * 1024,    // 1 MB
            wal_max_size: 2 * 1024 * 1024,     // 2 MB
            segment_max_size: 4 * 1024 * 1024, // 4 MB
            compaction_threshold: 2,
            block_size: 2 * 1024, // 2 KB
            sync_writes: true,
            compression: true,
            worker_threads: 1,
            verify_checksums: true,
        }
    }

    /// Ultra-performance preset: trades durability for maximum speed.
    /// Suitable for non-critical caches, ephemeral data, or benchmarks.
    /// WARNING: data may be lost on crash — use only when acceptable.
    pub fn ultra(path: impl Into<PathBuf>) -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .max(4);
        Self {
            path: path.into(),
            memtable_size: 128 * 1024 * 1024, // 128 MB in-memory buffer
            wal_max_size: 512 * 1024 * 1024,  // 512 MB WAL
            segment_max_size: 512 * 1024 * 1024,
            compaction_threshold: 32,
            block_size: 128 * 1024, // 128 KB
            sync_writes: false,
            compression: false,
            worker_threads: cpus,
            verify_checksums: false,
        }
    }

    /// Validate the configuration.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.memtable_size == 0 {
            return Err(crate::error::TorexError::InvalidArgument(
                "memtable_size must be > 0".into(),
            ));
        }
        if self.block_size == 0 {
            return Err(crate::error::TorexError::InvalidArgument(
                "block_size must be > 0".into(),
            ));
        }
        if self.worker_threads == 0 {
            return Err(crate::error::TorexError::InvalidArgument(
                "worker_threads must be > 0".into(),
            ));
        }
        Ok(())
    }
}
