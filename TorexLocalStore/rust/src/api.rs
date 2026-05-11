//! FFI API layer for flutter_rust_bridge.
//!
//! Zero-configuration API: all functions auto-initialize the runtime
//! on first call. No explicit `open()` or `close()` required.
//!
//! Usage from Dart:
//! ```dart
//! await Torex.box<User>().put(user);
//! final users = await Torex.box<User>().where("age > 18").find();
//! ```

use std::sync::Arc;

use parking_lot::Mutex;

use crate::runtime;
use crate::watcher::WatchEvent;

/// Global watcher subscriptions.
/// Maps subscriber IDs to their event receivers.
static WATCH_SUBSCRIPTIONS: Mutex<
    Option<std::collections::HashMap<u64, Arc<crossbeam::channel::Receiver<WatchEvent>>>>,
> = Mutex::new(None);

// ─── Lifecycle API (optional) ───────────────────────────────────────

/// Optionally initializes the engine with a custom path.
///
/// If not called, the engine auto-initializes on first data operation
/// using a platform-appropriate default path.
pub fn torex_initialize(path: Option<String>) -> Result<(), String> {
    match path {
        Some(p) => runtime::initialize_with_path(&p),
        None => runtime::initialize(),
    }
    .map_err(|e| e.to_string())
}

/// Checks if the runtime is currently initialized.
pub fn torex_is_initialized() -> bool {
    runtime::is_initialized()
}

/// Gracefully shuts down the runtime.
///
/// Not required — resources are cleaned up automatically on process exit.
/// Useful for explicit shutdown in testing or controlled environments.
pub fn torex_shutdown() -> Result<(), String> {
    runtime::shutdown().map_err(|e| e.to_string())
}

/// Returns the current storage path (if initialized).
pub fn torex_current_path() -> Option<String> {
    runtime::current_path()
}

// ─── Backward-compatible aliases for frb_generated.rs ───────────────

/// Legacy alias: opens the engine at the given path.
/// Kept for compatibility with generated FFI bridge code.
pub fn torex_open(path: String) -> Result<(), String> {
    runtime::initialize_with_path(&path).map_err(|e| e.to_string())
}

/// Legacy alias: closes the engine.
/// Kept for compatibility with generated FFI bridge code.
pub fn torex_close() -> Result<(), String> {
    runtime::shutdown().map_err(|e| e.to_string())
}

// ─── Core CRUD API (auto-init) ──────────────────────────────────────

/// Stores a key-value pair in the specified collection.
/// Auto-initializes the runtime on first call.
pub fn torex_put(collection: String, key: Vec<u8>, value: Vec<u8>) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.put(&key, &value).map_err(|e| e.to_string())?;
    engine.watcher().notify(WatchEvent::put(&collection, &key));
    Ok(())
}

/// Retrieves a value by key from the specified collection.
/// Auto-initializes the runtime on first call.
pub fn torex_get(collection: String, key: Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.get(&key).map_err(|e| e.to_string())
}

/// Deletes a key from the specified collection.
/// Auto-initializes the runtime on first call.
pub fn torex_delete(collection: String, key: Vec<u8>) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.delete(&key).map_err(|e| e.to_string())?;
    engine
        .watcher()
        .notify(WatchEvent::delete(&collection, &key));
    Ok(())
}

/// Checks if a key exists in the specified collection.
/// Auto-initializes the runtime on first call.
pub fn torex_exists(collection: String, key: Vec<u8>) -> Result<bool, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.exists(&key).map_err(|e| e.to_string())
}

/// Flushes the memtable to disk for the specified collection.
pub fn torex_flush(collection: String) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.flush_memtable().map_err(|e| e.to_string())
}

/// Returns the number of entries in the memtable.
pub fn torex_memtable_count(collection: String) -> Result<usize, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    Ok(storage.memtable_len())
}

/// Returns the number of segments.
pub fn torex_segment_count(collection: String) -> Result<usize, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    Ok(storage.segment_count())
}

/// Lists all open collection names.
pub fn torex_list_collections() -> Result<Vec<String>, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    Ok(engine.list_collections())
}

/// Stores a string key-value pair (convenience).
pub fn torex_put_string(collection: String, key: String, value: String) -> Result<(), String> {
    torex_put(collection, key.into_bytes(), value.into_bytes())
}

/// Gets a string value by string key (convenience).
pub fn torex_get_string(collection: String, key: String) -> Result<Option<String>, String> {
    let result = torex_get(collection, key.into_bytes())?;
    match result {
        Some(bytes) => Ok(Some(String::from_utf8(bytes).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Batch puts multiple key-value pairs with a single WAL fsync.
/// Far more efficient than calling torex_put in a loop.
pub fn torex_batch_put(collection: String, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;

    // Build slice of references for zero-copy batch write
    let refs: Vec<(&[u8], &[u8])> = entries
        .iter()
        .map(|(k, v)| (k.as_slice(), v.as_slice()))
        .collect();

    storage.batch_put(&refs).map_err(|e| e.to_string())?;
    Ok(())
}

/// Returns the engine version.
pub fn torex_version() -> String {
    crate::VERSION.to_string()
}

// ============================================================
// Query & Scan API
// ============================================================

/// A serializable query result entry.
#[derive(Debug, Clone)]
pub struct TorexEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Scans all entries in a collection with optional prefix filter and limit.
pub fn torex_scan(
    collection: String,
    prefix: Option<Vec<u8>>,
    start_key: Option<Vec<u8>>,
    end_key: Option<Vec<u8>>,
    limit: Option<usize>,
    offset: Option<usize>,
    reverse: bool,
) -> Result<Vec<TorexEntry>, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;

    let mut query = crate::query::Query::new();
    if let Some(p) = prefix {
        query = query.prefix(p);
    }
    if let Some(s) = start_key {
        query = query.start_at(s);
    }
    if let Some(e) = end_key {
        query = query.end_at(e);
    }
    if let Some(l) = limit {
        query = query.limit(l);
    }
    if let Some(o) = offset {
        query = query.offset(o);
    }
    if reverse {
        query = query.reverse();
    }

    let results = storage.scan(&query).map_err(|e| e.to_string())?;
    Ok(results
        .into_iter()
        .filter_map(|r| {
            r.value.map(|v| TorexEntry {
                key: r.key,
                value: v,
            })
        })
        .collect())
}

/// Scans entries with string keys/values.
pub fn torex_scan_strings(
    collection: String,
    prefix: Option<String>,
    start_key: Option<String>,
    end_key: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    reverse: bool,
) -> Result<Vec<(String, String)>, String> {
    let results = torex_scan(
        collection,
        prefix.map(|p| p.into_bytes()),
        start_key.map(|s| s.into_bytes()),
        end_key.map(|e| e.into_bytes()),
        limit,
        offset,
        reverse,
    )?;

    results
        .into_iter()
        .map(|e| {
            let key = String::from_utf8(e.key).map_err(|err| err.to_string())?;
            let value = String::from_utf8(e.value).map_err(|err| err.to_string())?;
            Ok((key, value))
        })
        .collect()
}

/// Returns all keys in a collection.
pub fn torex_keys(collection: String) -> Result<Vec<Vec<u8>>, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.keys().map_err(|e| e.to_string())
}

/// Returns the approximate count of entries in a collection.
pub fn torex_count(collection: String) -> Result<usize, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;
    storage.count().map_err(|e| e.to_string())
}

/// Deletes all entries in a collection (drops and recreates).
pub fn torex_clear_collection(collection: String) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    engine
        .close_collection(&collection)
        .map_err(|e| e.to_string())?;

    // Delete the collection directory
    let coll_path = engine.path().join("boxes").join(&collection);
    if coll_path.exists() {
        std::fs::remove_dir_all(&coll_path).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Batch deletes multiple keys with a single WAL fsync.
/// Far more efficient than calling torex_delete in a loop.
pub fn torex_batch_delete(collection: String, keys: Vec<Vec<u8>>) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let storage = engine
        .open_collection(&collection)
        .map_err(|e| e.to_string())?;

    let refs: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
    storage.batch_delete(&refs).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Watcher / Reactive Subscription API ────────────────────────

/// A change event from the watcher.
pub struct TorexWatchEvent {
    /// Collection name.
    pub collection: String,
    /// Key that changed.
    pub key: Vec<u8>,
    /// Change type: 0=Put, 1=Delete, 2=Clear.
    pub change_type: u8,
}

/// Subscribes to change events for a collection.
/// Returns a subscription ID used for polling and unsubscribing.
pub fn torex_watch_collection(collection: String) -> Result<u64, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let (id, rx) = engine.watcher().subscribe_collection(&collection);

    let mut subs = WATCH_SUBSCRIPTIONS.lock();
    if subs.is_none() {
        *subs = Some(std::collections::HashMap::new());
    }
    subs.as_mut().unwrap().insert(id, Arc::new(rx));

    Ok(id)
}

/// Subscribes to change events for keys with a prefix in a collection.
pub fn torex_watch_prefix(collection: String, prefix: Vec<u8>) -> Result<u64, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let (id, rx) = engine.watcher().subscribe_prefix(&collection, &prefix);

    let mut subs = WATCH_SUBSCRIPTIONS.lock();
    if subs.is_none() {
        *subs = Some(std::collections::HashMap::new());
    }
    subs.as_mut().unwrap().insert(id, Arc::new(rx));

    Ok(id)
}

/// Subscribes to ALL change events.
pub fn torex_watch_global() -> Result<u64, String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    let (id, rx) = engine.watcher().subscribe_global();

    let mut subs = WATCH_SUBSCRIPTIONS.lock();
    if subs.is_none() {
        *subs = Some(std::collections::HashMap::new());
    }
    subs.as_mut().unwrap().insert(id, Arc::new(rx));

    Ok(id)
}

/// Polls for pending watch events since last call.
/// Returns up to `max_events` events. Returns empty vec if no events pending.
pub fn torex_watch_poll(
    subscription_id: u64,
    max_events: u32,
) -> Result<Vec<TorexWatchEvent>, String> {
    let subs = WATCH_SUBSCRIPTIONS.lock();
    let rx = subs.as_ref().and_then(|m| m.get(&subscription_id)).cloned();

    drop(subs); // Release lock before reading

    let rx = rx.ok_or("subscription not found")?;

    let mut events = Vec::new();
    let limit = max_events.max(1).min(1000) as usize;

    for _ in 0..limit {
        match rx.try_recv() {
            Ok(evt) => {
                let change_type = match evt.change_type {
                    crate::watcher::ChangeType::Put => 0,
                    crate::watcher::ChangeType::Delete => 1,
                    crate::watcher::ChangeType::Clear => 2,
                };
                events.push(TorexWatchEvent {
                    collection: evt.collection,
                    key: evt.key,
                    change_type,
                });
            }
            Err(_) => break,
        }
    }

    Ok(events)
}

/// Unsubscribes from watch events.
pub fn torex_watch_unsubscribe(subscription_id: u64) -> Result<(), String> {
    let engine = runtime::ensure_initialized().map_err(|e| e.to_string())?;
    engine.watcher().unsubscribe(subscription_id);

    let mut subs = WATCH_SUBSCRIPTIONS.lock();
    if let Some(map) = subs.as_mut() {
        map.remove(&subscription_id);
    }

    Ok(())
}
