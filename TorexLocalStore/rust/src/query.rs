//! Query engine for structured data retrieval.
//!
//! Supports:
//! - Key prefix scans
//! - Range queries (start..end)
//! - Limit and offset
//! - Reverse iteration
//! - Field-level filtering (via secondary indexes)
//!
//! ## Design
//!
//! Queries are compiled into a `QueryPlan` which is then executed
//! against the memtable and segments. The planner chooses the most
//! efficient access path based on available indexes.

/// A query for retrieving entries from a collection.
#[derive(Debug, Clone)]
pub struct Query {
    /// Start key for range scan (inclusive). None = start from beginning.
    pub start_key: Option<Vec<u8>>,
    /// End key for range scan (exclusive). None = scan to end.
    pub end_key: Option<Vec<u8>>,
    /// Prefix filter — only return keys starting with this prefix.
    pub prefix: Option<Vec<u8>>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
    /// Number of results to skip.
    pub offset: Option<usize>,
    /// Reverse the result order.
    pub reverse: bool,
    /// Only return keys (no values).
    pub keys_only: bool,
}

impl Query {
    /// Creates a new empty query (scan all).
    pub fn new() -> Self {
        Self {
            start_key: None,
            end_key: None,
            prefix: None,
            limit: None,
            offset: None,
            reverse: false,
            keys_only: false,
        }
    }

    /// Sets the start key (inclusive).
    pub fn start_at(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.start_key = Some(key.into());
        self
    }

    /// Sets the end key (exclusive).
    pub fn end_at(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.end_key = Some(key.into());
        self
    }

    /// Sets a prefix filter.
    pub fn prefix(mut self, prefix: impl Into<Vec<u8>>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Sets the maximum number of results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Sets the offset (number of results to skip).
    pub fn offset(mut self, n: usize) -> Self {
        self.offset = Some(n);
        self
    }

    /// Reverses the result order.
    pub fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }

    /// Only return keys, not values.
    pub fn keys_only(mut self) -> Self {
        self.keys_only = true;
        self
    }

    /// Checks if a key matches the query's range constraints.
    #[inline]
    pub fn matches_key(&self, key: &[u8]) -> bool {
        if let Some(ref prefix) = self.prefix {
            if !key.starts_with(prefix) {
                return false;
            }
        }

        if let Some(ref start) = self.start_key {
            if key < start.as_slice() {
                return false;
            }
        }

        if let Some(ref end) = self.end_key {
            if key >= end.as_slice() {
                return false;
            }
        }

        true
    }
}

impl Default for Query {
    fn default() -> Self {
        Self::new()
    }
}

/// A single result entry from a query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The key.
    pub key: Vec<u8>,
    /// The value (None if keys_only).
    pub value: Option<Vec<u8>>,
}

/// Query execution plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryPlan {
    /// Full scan of memtable + all segments.
    FullScan,
    /// Prefix scan using sorted iteration.
    PrefixScan,
    /// Range scan with start/end bounds.
    RangeScan,
    /// Point lookup (single key).
    PointLookup,
}

impl Query {
    /// Determines the optimal execution plan for this query.
    pub fn plan(&self) -> QueryPlan {
        if self.start_key.is_none() && self.end_key.is_none() && self.prefix.is_none() {
            return QueryPlan::FullScan;
        }

        if self.prefix.is_some() && self.start_key.is_none() && self.end_key.is_none() {
            return QueryPlan::PrefixScan;
        }

        if self.start_key.is_some() || self.end_key.is_some() {
            return QueryPlan::RangeScan;
        }

        QueryPlan::FullScan
    }
}

/// Applies offset and limit to a sorted list of query results.
/// Returns the final results after applying offset/limit.
pub fn apply_pagination(results: &mut Vec<QueryResult>, offset: Option<usize>, limit: Option<usize>) {
    let off = offset.unwrap_or(0);
    if off > 0 {
        results.drain(..off.min(results.len()));
    }
    if let Some(lim) = limit {
        results.truncate(lim);
    }
}

/// Merges multiple sorted iterators into a single sorted result.
/// Used for merging memtable entries with segment entries.
/// Deduplicates by key, keeping the most recent entry.
pub fn merge_entries(
    memtable_entries: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    segment_entries: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
    reverse: bool,
) -> Vec<QueryResult> {
    let mut merged: Vec<QueryResult> = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();

    // Process memtable entries first (they are the most recent)
    for (key, value) in memtable_entries {
        seen_keys.insert(key.clone());
        if value.is_some() {
            merged.push(QueryResult {
                key,
                value,
            });
        }
        // None value = tombstone, skip
    }

    // Process segment entries (oldest segment first, newer overwrites)
    for entries in segment_entries {
        for (key, value) in entries {
            if seen_keys.contains(&key) {
                continue; // Already have a more recent version
            }
            seen_keys.insert(key.clone());
            merged.push(QueryResult {
                key,
                value: Some(value),
            });
        }
    }

    // Sort by key
    merged.sort_by(|a, b| {
        if reverse {
            b.key.cmp(&a.key)
        } else {
            a.key.cmp(&b.key)
        }
    });

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_builder() {
        let q = Query::new()
            .start_at(b"key_001")
            .end_at(b"key_100")
            .limit(20)
            .offset(5);

        assert_eq!(q.start_key, Some(b"key_001".to_vec()));
        assert_eq!(q.end_key, Some(b"key_100".to_vec()));
        assert_eq!(q.limit, Some(20));
        assert_eq!(q.offset, Some(5));
        assert!(!q.reverse);
    }

    #[test]
    fn test_query_matches_key_range() {
        let q = Query::new()
            .start_at(b"b")
            .end_at(b"f");

        assert!(!q.matches_key(b"a"));
        assert!(q.matches_key(b"b"));
        assert!(q.matches_key(b"c"));
        assert!(q.matches_key(b"e"));
        assert!(!q.matches_key(b"f"));
        assert!(!q.matches_key(b"g"));
    }

    #[test]
    fn test_query_matches_key_prefix() {
        let q = Query::new().prefix(b"user:");

        assert!(q.matches_key(b"user:1"));
        assert!(q.matches_key(b"user:abc"));
        assert!(!q.matches_key(b"item:1"));
        assert!(!q.matches_key(b"usr:1"));
    }

    #[test]
    fn test_query_plan() {
        let q = Query::new();
        assert_eq!(q.plan(), QueryPlan::FullScan);

        let q = Query::new().prefix(b"user:");
        assert_eq!(q.plan(), QueryPlan::PrefixScan);

        let q = Query::new().start_at(b"a").end_at(b"z");
        assert_eq!(q.plan(), QueryPlan::RangeScan);
    }

    #[test]
    fn test_apply_pagination() {
        let mut results: Vec<QueryResult> = (0..10)
            .map(|i| QueryResult {
                key: vec![i],
                value: Some(vec![i]),
            })
            .collect();

        apply_pagination(&mut results, Some(3), Some(4));
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].key, vec![3]);
        assert_eq!(results[3].key, vec![6]);
    }

    #[test]
    fn test_merge_entries_dedup() {
        let memtable = vec![
            (b"key1".to_vec(), Some(b"new_val1".to_vec())),
            (b"key3".to_vec(), None), // tombstone
        ];

        let segments = vec![vec![
            (b"key1".to_vec(), b"old_val1".to_vec()),
            (b"key2".to_vec(), b"val2".to_vec()),
            (b"key3".to_vec(), b"old_val3".to_vec()),
        ]];

        let results = merge_entries(memtable, segments, false);

        assert_eq!(results.len(), 2); // key1 and key2 (key3 is tombstoned)
        assert_eq!(results[0].key, b"key1");
        assert_eq!(results[0].value, Some(b"new_val1".to_vec()));
        assert_eq!(results[1].key, b"key2");
        assert_eq!(results[1].value, Some(b"val2".to_vec()));
    }

    #[test]
    fn test_merge_entries_reverse() {
        let memtable = vec![
            (b"a".to_vec(), Some(b"1".to_vec())),
            (b"c".to_vec(), Some(b"3".to_vec())),
        ];

        let segments = vec![vec![
            (b"b".to_vec(), b"2".to_vec()),
        ]];

        let results = merge_entries(memtable, segments, true);

        assert_eq!(results[0].key, b"c");
        assert_eq!(results[1].key, b"b");
        assert_eq!(results[2].key, b"a");
    }
}
