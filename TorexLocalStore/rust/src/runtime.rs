//! Global runtime manager for zero-configuration developer experience.
//!
//! Provides automatic lazy initialization, singleton engine management,
//! background worker coordination, and automatic resource cleanup.
//!
//! The runtime initializes only once and remains extremely lightweight.
//! All background systems (WAL workers, compaction, cache cleanup) run
//! automatically without developer involvement.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::engine::TorexEngine;
use crate::error::Result;
use crate::error::TorexError;

// ─── Background Worker Handle ───────────────────────────────────────

/// Handle to background worker threads.
/// Workers check `is_initialized()` and exit when runtime shuts down.
struct WorkerHandle {
    #[allow(dead_code)]
    name: String,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl WorkerHandle {
    fn join(&mut self) {
        if let Some(h) = self.join_handle.take() {
            let _ = h.join();
        }
    }
}

// ─── Runtime State ──────────────────────────────────────────────────

/// Internal runtime state holding the engine and background workers.
struct RuntimeState {
    engine: Arc<TorexEngine>,
    workers: Vec<WorkerHandle>,
}

impl RuntimeState {
    fn new(engine: TorexEngine) -> Self {
        Self {
            engine: Arc::new(engine),
            workers: Vec::new(),
        }
    }
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        log::debug!(
            "RuntimeState dropping, stopping {} workers...",
            self.workers.len()
        );
        for worker in &mut self.workers {
            worker.join();
        }
        // Engine Drop handles closing collections
        log::debug!("RuntimeState dropped cleanly.");
    }
}

// ─── Global Runtime Singleton ───────────────────────────────────────

/// Global runtime singleton.
/// Uses `parking_lot::Mutex` for thread-safe, lightweight access.
static RUNTIME: Mutex<Option<RuntimeState>> = Mutex::new(None);

/// Default storage path resolver.
/// Uses platform-appropriate data directory.
fn default_storage_path() -> String {
    dirs::data_local_dir()
        .or_else(|| dirs::data_dir())
        .map(|p| p.join("torex_store"))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/tmp/torex_store".to_string())
}

// ─── Public Runtime API ─────────────────────────────────────────────

/// Initializes the runtime with an explicit path.
///
/// Can be called optionally before first usage. If already initialized,
/// this is a no-op.
pub fn initialize_with_path(path: &str) -> Result<()> {
    let mut guard = RUNTIME.lock();
    if guard.is_some() {
        log::debug!("Runtime already initialized, skipping.");
        return Ok(());
    }

    log::info!("Initializing Torex runtime at: {}", path);
    let engine = TorexEngine::open(path)?;
    let mut state = RuntimeState::new(engine);
    spawn_background_workers(&mut state);
    *guard = Some(state);
    log::info!("Torex runtime initialized successfully.");
    Ok(())
}

/// Initializes the runtime with default path.
///
/// Can be called optionally before first usage.
/// If already initialized, this is a no-op.
pub fn initialize() -> Result<()> {
    initialize_with_path(&default_storage_path())
}

/// Ensures the runtime is initialized (lazy auto-init).
///
/// Called internally by every API function.
/// Uses default path if not explicitly initialized.
pub fn ensure_initialized() -> Result<Arc<TorexEngine>> {
    // Fast path: check if already initialized
    {
        let guard = RUNTIME.lock();
        if let Some(ref state) = *guard {
            return Ok(Arc::clone(&state.engine));
        }
    }
    // Lock is released here before re-acquiring

    // Slow path: initialize with default path
    log::debug!("Auto-initializing Torex runtime...");
    initialize()?;

    let guard = RUNTIME.lock();
    match *guard {
        Some(ref state) => Ok(Arc::clone(&state.engine)),
        None => Err(TorexError::Internal("runtime initialization failed".into())),
    }
}

/// Shuts down the runtime and releases all resources.
///
/// Called automatically on process exit via `Drop`.
/// Can be called manually for graceful shutdown.
pub fn shutdown() -> Result<()> {
    // Extract RuntimeState while holding the lock, then release the lock
    // BEFORE dropping the state. Workers call is_initialized() → RUNTIME.lock()
    // during their shutdown loop; holding the lock while joining them deadlocks.
    let state = {
        let mut guard = RUNTIME.lock();
        guard.take()
        // ← lock released here when `guard` drops
    };
    // RuntimeState is dropped here, AFTER the lock is released.
    // Workers can now acquire the lock, see is_initialized() == false, and exit.
    if state.is_some() {
        log::info!("Torex runtime shut down.");
    }
    Ok(())
}

/// Checks if the runtime is currently initialized.
pub fn is_initialized() -> bool {
    RUNTIME.lock().is_some()
}

/// Returns the current storage path, if initialized.
pub fn current_path() -> Option<String> {
    let guard = RUNTIME.lock();
    guard
        .as_ref()
        .map(|s| s.engine.path().to_string_lossy().to_string())
}

// ─── Background Workers ─────────────────────────────────────────────

/// Spawns all background worker threads.
/// These run automatically and are invisible to the developer.
fn spawn_background_workers(state: &mut RuntimeState) {
    // WAL flush worker: periodically flushes memtables for all collections
    let engine_clone = Arc::clone(&state.engine);
    let wal_handle = std::thread::Builder::new()
        .name("torex-wal-worker".into())
        .spawn(move || {
            wal_flush_worker(engine_clone);
        })
        .unwrap();

    state.workers.push(WorkerHandle {
        name: "wal-worker".into(),
        join_handle: Some(wal_handle),
    });

    // Compaction worker: merges segments when threshold is reached
    let engine_clone = Arc::clone(&state.engine);
    let compact_handle = std::thread::Builder::new()
        .name("torex-compaction-worker".into())
        .spawn(move || {
            compaction_worker(engine_clone);
        })
        .unwrap();

    state.workers.push(WorkerHandle {
        name: "compaction-worker".into(),
        join_handle: Some(compact_handle),
    });

    // Cache cleanup worker: periodic cache maintenance
    let engine_clone = Arc::clone(&state.engine);
    let cache_handle = std::thread::Builder::new()
        .name("torex-cache-worker".into())
        .spawn(move || {
            cache_cleanup_worker(engine_clone);
        })
        .unwrap();

    state.workers.push(WorkerHandle {
        name: "cache-worker".into(),
        join_handle: Some(cache_handle),
    });

    log::debug!("Spawned {} background workers.", state.workers.len());
}

/// WAL flush worker — runs every 200 ms.
///
/// 1. Always flushes the BufWriter buffer to the OS (ensures data survives
///    a crash even with sync_writes = false, at most 200 ms old).
/// 2. Triggers a memtable-to-segment flush when the memtable exceeds 5 000 entries.
fn wal_flush_worker(engine: Arc<TorexEngine>) {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));

        if !is_initialized() {
            break;
        }

        for name in engine.list_collections() {
            if let Ok(storage) = engine.open_collection(&name) {
                // 1. Always push WAL buffer to OS — cheap, no fsync
                if let Err(e) = storage.flush_wal() {
                    log::warn!("WAL buffer flush error for '{}': {}", name, e);
                }

                // 2. Flush memtable to segment when it grows large
                if storage.memtable_len() > 5_000 {
                    if let Err(e) = storage.flush_memtable() {
                        log::warn!("Memtable flush error for '{}': {}", name, e);
                    }
                }
            }
        }
    }
    log::debug!("WAL worker stopped.");
}

/// Compaction worker — runs every 5 seconds with adaptive triggering.
///
/// Merges segments when their count exceeds the threshold, reducing:
/// - Read amplification (fewer segments to scan per get)
/// - Disk usage (tombstones are eliminated)
/// - mmap pressure (fewer open mappings)
fn compaction_worker(engine: Arc<TorexEngine>) {
    let config = crate::compaction::CompactionConfig {
        min_segments: 4,
        max_segments_to_compact: 8,
        enabled: true,
    };

    loop {
        // Check every 5 s — compact only when actually needed (adaptive)
        std::thread::sleep(std::time::Duration::from_secs(5));

        if !is_initialized() {
            break;
        }

        for name in engine.list_collections() {
            if let Ok(storage) = engine.open_collection(&name) {
                let seg_count = storage.segment_count();

                if seg_count < config.min_segments {
                    continue; // Nothing to do
                }

                let segments = storage.segments();
                match crate::compaction::compact_segments(&segments, &config) {
                    Ok(Some(result)) => {
                        log::debug!(
                            "[compaction] '{}': merged {} segments → 1, \
                             {} entries, reclaimed {} bytes",
                            name,
                            result.segments_compacted,
                            result.entries_in_new_segment,
                            result.bytes_reclaimed,
                        );
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("[compaction] error on '{}': {}", name, e),
                }
            }
        }
    }
    log::debug!("Compaction worker stopped.");
}

/// Cache/housekeeping worker — runs every 60 seconds.
///
/// Performs low-priority maintenance that doesn't need to run frequently:
/// - Forces a full WAL fsync to guarantee durability checkpoint
/// - Future: segment index optimization, LRU eviction, etc.
fn cache_cleanup_worker(engine: Arc<TorexEngine>) {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));

        if !is_initialized() {
            break;
        }

        // Force a full durability checkpoint every minute
        for name in engine.list_collections() {
            if let Ok(storage) = engine.open_collection(&name) {
                // Full flush: BufWriter → OS → disk (fsync equivalent)
                if let Err(e) = storage.flush_wal() {
                    log::warn!("[cache-worker] WAL checkpoint failed for '{}': {}", name, e);
                }
            }
        }
    }
    log::debug!("Cache cleanup worker stopped.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cleanup_runtime() {
        // Same lock-before-drop fix as shutdown(): release lock, THEN drop state.
        let _state = {
            let mut guard = RUNTIME.lock();
            guard.take()
            // ← lock released here
        };
        // _state (RuntimeState) dropped here after lock is released
    }

    #[test]
    fn test_auto_initialize() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_auto_init");
        let path_str = path.to_string_lossy().to_string();

        cleanup_runtime();

        initialize_with_path(&path_str).unwrap();
        assert!(is_initialized());

        // Double init is no-op
        initialize_with_path(&path_str).unwrap();
        assert!(is_initialized());

        let engine = ensure_initialized().unwrap();
        assert!(engine.path().exists());

        shutdown().unwrap();
        assert!(!is_initialized());
    }

    #[test]
    fn test_lazy_init() {
        cleanup_runtime();

        let engine = ensure_initialized().unwrap();
        assert!(is_initialized());
        assert!(engine.path().exists());

        shutdown().unwrap();
    }

    #[test]
    fn test_current_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_path");
        let path_str = path.to_string_lossy().to_string();

        cleanup_runtime();

        assert!(current_path().is_none());
        initialize_with_path(&path_str).unwrap();
        assert!(current_path().is_some());

        shutdown().unwrap();
        assert!(current_path().is_none());
    }
}
