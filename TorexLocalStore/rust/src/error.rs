//! Error types for the Torex storage engine.
//!
//! All errors are categorized for proper handling at every layer.

use thiserror::Error;

/// Result type alias used throughout the engine.
pub type Result<T> = std::result::Result<T, TorexError>;

/// Comprehensive error type for the storage engine.
#[derive(Error, Debug)]
pub enum TorexError {
    /// I/O errors from filesystem operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Key not found in storage.
    #[error("key not found: {0}")]
    NotFound(String),

    /// Database is corrupted.
    #[error("corruption detected: {0}")]
    Corruption(String),

    /// Invalid argument provided.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Storage engine is closed.
    #[error("storage engine is closed")]
    Closed,

    /// WAL (Write-Ahead Log) error.
    #[error("WAL error: {0}")]
    Wal(String),

    /// Serialization/deserialization error.
    #[error("codec error: {0}")]
    Codec(String),

    /// Compaction error.
    #[error("compaction error: {0}")]
    Compaction(String),

    /// Lock poisoned (thread panicked while holding lock).
    #[error("lock poisoned")]
    LockPoisoned,

    /// Storage capacity exceeded.
    #[error("capacity exceeded: {0}")]
    CapacityExceeded(String),

    /// Checksum mismatch — data integrity violation.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    /// Generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<crossbeam::channel::RecvError> for TorexError {
    fn from(_: crossbeam::channel::RecvError) -> Self {
        TorexError::Internal("channel receive error".into())
    }
}
