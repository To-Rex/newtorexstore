//! Background segment compaction engine.
//!
//! Compaction merges multiple segments into fewer, larger segments.
//! This is essential for:
//!
//! - Reclaiming space from deleted/overwritten keys
//! - Reducing the number of segments to search during reads
//! - Maintaining read performance as data grows
//!
//! ## Compaction Strategy
//!
//! Uses a size-tiered approach:
//! 1. Segments are grouped by similar sizes
//! 2. Groups with enough segments are merged
//! 3. Tombstones (deletes) are dropped during merge
//! 4. Old segments are deleted after successful merge
//!
//! ## Concurrency
//!
//! Compaction runs in a background thread/task. During compaction:
//! - Normal reads/writes continue unaffected
//! - New segments may be created during compaction
//! - Only the compacted segments are locked during swap

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::Result;
use crate::memtable::MemtableEntry;
use crate::segment::{Segment, SegmentManager};

/// Compaction configuration.
#[derive(Clone, Debug)]
pub struct CompactionConfig {
    /// Minimum number of segments before compaction triggers.
    pub min_segments: usize,

    /// Maximum number of segments to compact at once.
    pub max_segments_to_compact: usize,

    /// Whether compaction is enabled.
    pub enabled: bool,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            min_segments: 4,
            max_segments_to_compact: 10,
            enabled: true,
        }
    }
}

/// Result of a compaction run.
#[derive(Debug)]
pub struct CompactionResult {
    /// Number of segments compacted.
    pub segments_compacted: usize,

    /// IDs of segments that were removed.
    pub removed_segment_ids: Vec<u64>,

    /// ID of the new merged segment.
    pub new_segment_id: u64,

    /// Number of entries in the new segment.
    pub entries_in_new_segment: usize,

    /// Bytes reclaimed (approximate).
    pub bytes_reclaimed: u64,
}

/// Compacts segments with minimized write-lock duration.
///
/// The write lock is held for only two brief phases:
///  1. Reserving the new segment ID
///  2. Swapping old segments for the new one in memory
///
/// All disk I/O (reading, merging, writing) happens outside any lock.
pub fn compact_segments(
    segment_manager: &Arc<RwLock<SegmentManager>>,
    config: &CompactionConfig,
) -> Result<Option<CompactionResult>> {
    if !config.enabled {
        return Ok(None);
    }

    // ── Phase 1: Plan — brief write lock to reserve next segment ID ──────────
    let (segment_ids, segment_paths, total_old_size, seg_dir, new_segment_id) = {
        let mut mgr = segment_manager.write();
        let all_segments = mgr.segments();

        if all_segments.len() < config.min_segments {
            return Ok(None);
        }

        let count = config.max_segments_to_compact.min(all_segments.len());
        let ids: Vec<u64> = all_segments[..count].iter().map(|s| s.id).collect();
        let paths: Vec<PathBuf> = all_segments[..count]
            .iter()
            .map(|s| s.path.clone())
            .collect();
        let size: u64 = all_segments[..count].iter().map(|s| s.file_size).sum();
        let dir = mgr.directory().to_path_buf();
        let new_id = mgr.take_next_id(); // Reserve ID atomically
        (ids, paths, size, dir, new_id)
    }; // ← Write lock released

    if segment_ids.is_empty() {
        return Ok(None);
    }

    // ── Phase 2: Read + merge — NO LOCK HELD ────────────────────────────────
    let segments_to_compact: Vec<Segment> = segment_ids
        .iter()
        .zip(segment_paths.iter())
        .filter_map(|(&id, path)| Segment::open(path, id).ok())
        .collect();

    let merged = merge_segment_entries(&segments_to_compact)?;

    if merged.is_empty() {
        // All tombstones — remove old segments, write nothing
        let paths_to_delete = {
            let mut mgr = segment_manager.write();
            mgr.remove_segments_from_memory(&segment_ids)
        };
        for path in paths_to_delete {
            let _ = std::fs::remove_file(&path);
        }
        return Ok(Some(CompactionResult {
            segments_compacted: segments_to_compact.len(),
            removed_segment_ids: segment_ids,
            new_segment_id: 0,
            entries_in_new_segment: 0,
            bytes_reclaimed: total_old_size,
        }));
    }

    // ── Phase 3: Create new segment file — NO LOCK HELD ──────────────────────
    let new_path = seg_dir.join(format!("{}.seg", new_segment_id));
    let new_segment = match Segment::create(&new_path, new_segment_id, &merged) {
        Ok(seg) => seg,
        Err(e) => {
            // Creation failed — clean up partial file, leave old segments intact
            let _ = std::fs::remove_file(&new_path);
            return Err(e);
        }
    };
    let new_size = new_segment.file_size;
    let entries_count = merged.len();

    // ── Phase 4: Swap — brief write lock for pointer update only ─────────────
    let paths_to_delete = {
        let mut mgr = segment_manager.write();
        mgr.add_prebuilt_segment(new_segment);
        mgr.remove_segments_from_memory(&segment_ids)
    }; // ← Write lock released

    // ── Phase 5: Delete old files outside lock ────────────────────────────────
    for path in &paths_to_delete {
        if let Err(e) = std::fs::remove_file(path) {
            log::warn!("Failed to delete compacted segment {:?}: {}", path, e);
        }
    }

    log::debug!(
        "Compaction: {} segments → 1, {} entries, {} bytes reclaimed",
        segments_to_compact.len(),
        entries_count,
        total_old_size.saturating_sub(new_size),
    );

    Ok(Some(CompactionResult {
        segments_compacted: segments_to_compact.len(),
        removed_segment_ids: segment_ids,
        new_segment_id,
        entries_in_new_segment: entries_count,
        bytes_reclaimed: total_old_size.saturating_sub(new_size),
    }))
}

/// Merges entries from multiple segments, keeping newest values.
///
/// Reads all segments (oldest first), newer entries overwrite older ones.
/// Tombstones are preserved (they indicate deletion).
fn merge_segment_entries(segments: &[Segment]) -> Result<Vec<(Vec<u8>, MemtableEntry)>> {
    let mut merged: std::collections::BTreeMap<Vec<u8>, MemtableEntry> =
        std::collections::BTreeMap::new();

    for segment in segments {
        let entries = segment.read_all()?;
        for (key, value) in entries {
            if value.is_empty() {
                // Tombstone — mark as deleted
                merged.insert(key, MemtableEntry::Delete);
            } else {
                merged.insert(key, MemtableEntry::Put(value));
            }
        }
    }

    // Remove tombstones (they're not needed after compaction)
    merged.retain(|_, v| !matches!(v, MemtableEntry::Delete));

    Ok(merged.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TorexConfig;
    use crate::storage::Storage;
    use tempfile::TempDir;

    fn make_config(dir: &TempDir) -> TorexConfig {
        let mut config = TorexConfig::new(dir.path().join("test_db"));
        config.memtable_size = 10; // Very small to create many segments
        config.sync_writes = false;
        config
    }

    #[test]
    fn test_compaction_merges_segments() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);

        let store = Storage::open(config).unwrap();

        // Write data to create multiple segments
        for i in 0..50 {
            let key = format!("key_{:04}", i);
            let value = format!("value_{}", i);
            store.put(key.as_bytes(), value.as_bytes()).unwrap();
        }

        let segment_count_before = store.segment_count();
        assert!(
            segment_count_before >= 4,
            "Expected >= 4 segments, got {}",
            segment_count_before
        );

        // Create a shared segment manager for compaction
        // We need to access the internal segment manager
        let seg_mgr = {
            // Re-read segments from disk
            let seg_dir = dir.path().join("test_db/segments");
            Arc::new(RwLock::new(SegmentManager::new(seg_dir).unwrap()))
        };

        let result = compact_segments(&seg_mgr, &CompactionConfig::default()).unwrap();

        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.segments_compacted >= 4);
        assert!(result.bytes_reclaimed > 0 || result.entries_in_new_segment > 0);
    }

    #[test]
    fn test_compaction_removes_tombstones() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);

        let store = Storage::open(config).unwrap();

        // Write and then delete keys
        for i in 0..30 {
            let key = format!("key_{:04}", i);
            store.put(key.as_bytes(), b"value").unwrap();
        }
        for i in 0..15 {
            let key = format!("key_{:04}", i);
            store.delete(key.as_bytes()).unwrap();
        }

        let seg_mgr = {
            let seg_dir = dir.path().join("test_db/segments");
            Arc::new(RwLock::new(SegmentManager::new(seg_dir).unwrap()))
        };

        let result = compact_segments(&seg_mgr, &CompactionConfig::default()).unwrap();

        if let Some(result) = result {
            // After compaction, tombstones should be removed
            // Only keys 15-29 should remain (15 entries)
            assert!(result.entries_in_new_segment <= 15);
        }
    }

    #[test]
    fn test_compaction_disabled() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);

        let store = Storage::open(config).unwrap();

        for i in 0..50 {
            let key = format!("key_{:04}", i);
            store.put(key.as_bytes(), b"value").unwrap();
        }

        let seg_mgr = {
            let seg_dir = dir.path().join("test_db/segments");
            Arc::new(RwLock::new(SegmentManager::new(seg_dir).unwrap()))
        };

        let disabled_config = CompactionConfig {
            enabled: false,
            ..Default::default()
        };

        let result = compact_segments(&seg_mgr, &disabled_config).unwrap();
        assert!(result.is_none());
    }
}
