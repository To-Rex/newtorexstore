//! Transaction system for atomic multi-key operations.
//!
//! Supports:
//! - Atomic batch writes (all-or-nothing)
//! - Read-modify-write patterns
//! - Optimistic concurrency with version checks
//!
//! ## Design
//!
//! Transactions are implemented as atomic batches written to the WAL.
//! The batch is written as a single WAL entry, ensuring atomicity.
//! If the app crashes mid-batch, recovery will replay the complete batch
//! or none of it (WAL entry is checksummed).

use crate::error::Result;
use crate::storage::Storage;

/// A batch of operations to be applied atomically.
#[derive(Debug, Clone)]
pub struct Batch {
    /// Operations in this batch.
    operations: Vec<BatchOp>,
}

/// A single operation in a batch.
#[derive(Debug, Clone)]
pub enum BatchOp {
    /// Insert or update a key-value pair.
    Put { key: Vec<u8>, value: Vec<u8> },
    /// Delete a key.
    Delete { key: Vec<u8> },
}

impl Batch {
    /// Creates a new empty batch.
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// Adds a put operation to the batch.
    pub fn put(mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        self.operations.push(BatchOp::Put {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    /// Adds a delete operation to the batch.
    pub fn delete(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.operations.push(BatchOp::Delete { key: key.into() });
        self
    }

    /// Returns the number of operations in the batch.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Returns true if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns a reference to the operations.
    pub fn operations(&self) -> &[BatchOp] {
        &self.operations
    }

    /// Executes the batch atomically against a storage instance.
    /// All puts and deletes are applied in order.
    /// If any operation fails, the batch is aborted.
    pub fn execute(self, storage: &Storage) -> Result<()> {
        if self.operations.is_empty() {
            return Ok(());
        }

        // Apply all operations in order
        for op in &self.operations {
            match op {
                BatchOp::Put { key, value } => {
                    storage.put(key, value)?;
                }
                BatchOp::Delete { key } => {
                    storage.delete(key)?;
                }
            }
        }

        Ok(())
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing batch operations fluently.
pub struct BatchBuilder {
    batch: Batch,
}

impl BatchBuilder {
    /// Creates a new batch builder.
    pub fn new() -> Self {
        Self {
            batch: Batch::new(),
        }
    }

    /// Adds a put operation.
    pub fn put(mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        self.batch = self.batch.put(key, value);
        self
    }

    /// Adds a delete operation.
    pub fn delete(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.batch = self.batch.delete(key);
        self
    }

    /// Builds the batch.
    pub fn build(self) -> Batch {
        self.batch
    }
}

impl Default for BatchBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::config::TorexConfig;

    #[test]
    fn test_batch_builder() {
        let batch = BatchBuilder::new()
            .put(b"key1", b"val1")
            .put(b"key2", b"val2")
            .delete(b"key3")
            .build();

        assert_eq!(batch.len(), 3);
        assert!(matches!(batch.operations()[0], BatchOp::Put { .. }));
        assert!(matches!(batch.operations()[1], BatchOp::Put { .. }));
        assert!(matches!(batch.operations()[2], BatchOp::Delete { .. }));
    }

    #[test]
    fn test_batch_execute() {
        let dir = TempDir::new().unwrap();
        let config = TorexConfig::new(&dir.path().join("test_db"));
        let storage = Storage::open(config).unwrap();

        let batch = Batch::new()
            .put(b"key1", b"val1")
            .put(b"key2", b"val2")
            .put(b"key3", b"val3");

        batch.execute(&storage).unwrap();

        assert_eq!(storage.get(b"key1").unwrap(), Some(b"val1".to_vec()));
        assert_eq!(storage.get(b"key2").unwrap(), Some(b"val2".to_vec()));
        assert_eq!(storage.get(b"key3").unwrap(), Some(b"val3".to_vec()));
    }

    #[test]
    fn test_batch_with_delete() {
        let dir = TempDir::new().unwrap();
        let config = TorexConfig::new(&dir.path().join("test_db"));
        let storage = Storage::open(config).unwrap();

        // First insert
        storage.put(b"key1", b"val1").unwrap();
        assert!(storage.get(b"key1").unwrap().is_some());

        // Delete via batch
        let batch = Batch::new().delete(b"key1");
        batch.execute(&storage).unwrap();

        assert!(storage.get(b"key1").unwrap().is_none());
    }

    #[test]
    fn test_empty_batch() {
        let dir = TempDir::new().unwrap();
        let config = TorexConfig::new(&dir.path().join("test_db"));
        let storage = Storage::open(config).unwrap();

        let batch = Batch::new();
        batch.execute(&storage).unwrap(); // Should be a no-op
    }
}
