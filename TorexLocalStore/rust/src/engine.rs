//! High-level storage engine managing multiple collections (boxes).
//!
//! The engine provides a collection-oriented API where each collection (box)
//! is an independent LSM-tree instance with its own memtable, WAL, and segments.
//!
//! Lifecycle management is handled automatically by the [`runtime`] module.
//! Developers should use the zero-config API via the Flutter layer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::config::TorexConfig;
use crate::error::Result;
use crate::storage::Storage;
use crate::watcher::Watcher;

/// The top-level storage engine.
pub struct TorexEngine {
    /// Base directory for all collections.
    base_path: PathBuf,

    /// Engine configuration.
    config: TorexConfig,

    /// Open collections (boxes).
    collections: Arc<RwLock<HashMap<String, Arc<Storage>>>>,

    /// Reactive watcher for change notifications.
    watcher: Watcher,
}

impl TorexEngine {
    /// Opens or creates the storage engine at the given path.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let config = TorexConfig::new(&path);

        std::fs::create_dir_all(&path)?;

        let engine = Self {
            base_path: path,
            config,
            collections: Arc::new(RwLock::new(HashMap::new())),
            watcher: Watcher::new(),
        };

        engine.discover_collections()?;

        Ok(engine)
    }

    /// Opens or creates a collection (box) by name.
    pub fn open_collection(&self, name: &str) -> Result<Arc<Storage>> {
        // Check if already open
        {
            let collections = self.collections.read();
            if let Some(storage) = collections.get(name) {
                return Ok(Arc::clone(storage));
            }
        }

        // Open new collection
        let collection_path = self.base_path.join("boxes").join(name);
        let config = TorexConfig {
            path: collection_path,
            ..self.config.clone()
        };

        let storage = Arc::new(Storage::open(config)?);

        let mut collections = self.collections.write();
        collections.insert(name.to_string(), Arc::clone(&storage));

        Ok(storage)
    }

    /// Closes a specific collection.
    pub fn close_collection(&self, name: &str) -> Result<()> {
        let mut collections = self.collections.write();
        if let Some(storage) = collections.remove(name) {
            storage.close()?;
        }
        Ok(())
    }

    /// Lists all open collection names.
    pub fn list_collections(&self) -> Vec<String> {
        let collections = self.collections.read();
        collections.keys().cloned().collect()
    }

    /// Closes all collections and the engine.
    pub fn close(&self) -> Result<()> {
        let mut collections = self.collections.write();
        for (_, storage) in collections.drain() {
            storage.close()?;
        }
        Ok(())
    }

    /// Returns the base path.
    pub fn path(&self) -> &std::path::Path {
        &self.base_path
    }

    /// Returns a reference to the watcher for reactive subscriptions.
    pub fn watcher(&self) -> &Watcher {
        &self.watcher
    }

    /// Discovers existing collections on disk.
    fn discover_collections(&self) -> Result<()> {
        let boxes_dir = self.base_path.join("boxes");
        if !boxes_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&boxes_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                log::debug!("Discovered collection: {}", name);
            }
        }

        Ok(())
    }
}

impl Drop for TorexEngine {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            log::error!("Error closing engine: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_engine_open_close() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("torex_db");

        let engine = TorexEngine::open(&path).unwrap();
        assert!(path.exists());
        engine.close().unwrap();
    }

    #[test]
    fn test_engine_collection_operations() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("torex_db");

        let engine = TorexEngine::open(&path).unwrap();

        let users = engine.open_collection("users").unwrap();
        users.put(b"user1", b"Alice").unwrap();
        users.put(b"user2", b"Bob").unwrap();

        assert_eq!(users.get(b"user1").unwrap(), Some(b"Alice".to_vec()));

        let users2 = engine.open_collection("users").unwrap(); // Same instance
        assert_eq!(users2.get(b"user2").unwrap(), Some(b"Bob".to_vec()));

        engine.close().unwrap();
    }

    #[test]
    fn test_engine_multiple_collections() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("torex_db");

        let engine = TorexEngine::open(&path).unwrap();

        let users = engine.open_collection("users").unwrap();
        let products = engine.open_collection("products").unwrap();

        users.put(b"u1", b"Alice").unwrap();
        products.put(b"p1", b"Widget").unwrap();

        assert_eq!(users.get(b"u1").unwrap(), Some(b"Alice".to_vec()));
        assert_eq!(products.get(b"p1").unwrap(), Some(b"Widget".to_vec()));
        assert_eq!(users.get(b"p1").unwrap(), None); // Different collection

        engine.close().unwrap();
    }

    #[test]
    fn test_engine_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("torex_db");

        // Write
        {
            let engine = TorexEngine::open(&path).unwrap();
            let users = engine.open_collection("users").unwrap();
            users.put(b"u1", b"Alice").unwrap();
            users.flush_memtable().unwrap();
            engine.close().unwrap();
        }

        // Read
        {
            let engine = TorexEngine::open(&path).unwrap();
            let users = engine.open_collection("users").unwrap();
            assert_eq!(users.get(b"u1").unwrap(), Some(b"Alice".to_vec()));
            engine.close().unwrap();
        }
    }
}
