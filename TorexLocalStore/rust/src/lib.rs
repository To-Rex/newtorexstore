//! # Torex Local Storage Engine
//!
//! Ultra-high-performance embedded storage engine designed for Flutter.
//! Combines LSM-tree concepts with memory-mapped I/O, WAL, and zero-copy reads.
//!
//! ## Architecture
//!
//! ```text
//! Flutter Layer → FFI Bridge → Rust Core Engine → Storage → Filesystem
//! ```
//!
//! ## Key Design Decisions
//!
//! - **LSM-Tree**: Writes go to an in-memory memtable, flushed to sorted segments
//! - **WAL**: All writes are append-only logged for crash recovery
//! - **Memory-mapped I/O**: Zero-copy reads via mmap for segments
//! - **Lock-free reads**: Readers never block writers
//! - **Binary encoding**: No JSON — all data is binary-encoded for speed

#![warn(unsafe_code)]
#![allow(dead_code)]

pub mod api;
pub mod bloom;
pub mod codec;
pub mod compaction;
pub mod compress;
pub mod config;
pub mod engine;
pub mod error;
pub mod index;
pub mod memtable;
pub mod mmap;
pub mod query;
pub mod runtime;
pub mod segment;
pub mod storage;
pub mod transaction;
pub mod wal;
pub mod watcher;
pub mod chunk;

#[allow(unsafe_code)]
mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */

/// Crate version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Magic bytes for file format identification
pub const MAGIC_BYTES: [u8; 4] = [b'T', b'R', b'X', b'S'];

/// Current file format version
pub const FORMAT_VERSION: u32 = 1;
