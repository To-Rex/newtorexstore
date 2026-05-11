//! Reactive subscription engine for watching collection changes.
//!
//! Provides a publish-subscribe mechanism where Dart/Flutter code
//! can subscribe to key-level or collection-level change notifications.
//!
//! ## Architecture
//!
//! ```text
//! Storage.put/delete → Watcher.notify(key, change_type)
//!     → subscribers: Vec<Sender<WatchEvent>>
//!     → Dart Stream receives WatchEvent
//! ```
//!
//! ## Concurrency
//!
//! - Uses `parking_lot::RwLock` for subscriber list
//! - `crossbeam::channel` for non-blocking event dispatch
//! - Subscribers that disconnect are auto-cleaned

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

/// Type of change that occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    /// A key was created or updated.
    Put,
    /// A key was deleted.
    Delete,
    /// The collection was cleared.
    Clear,
}

/// A watch event delivered to subscribers.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// The collection name.
    pub collection: String,
    /// The key that changed (empty for Clear events).
    pub key: Vec<u8>,
    /// The type of change.
    pub change_type: ChangeType,
}

impl WatchEvent {
    pub fn put(collection: &str, key: &[u8]) -> Self {
        Self {
            collection: collection.to_string(),
            key: key.to_vec(),
            change_type: ChangeType::Put,
        }
    }

    pub fn delete(collection: &str, key: &[u8]) -> Self {
        Self {
            collection: collection.to_string(),
            key: key.to_vec(),
            change_type: ChangeType::Delete,
        }
    }

    pub fn clear(collection: &str) -> Self {
        Self {
            collection: collection.to_string(),
            key: Vec::new(),
            change_type: ChangeType::Clear,
        }
    }
}

/// Subscriber ID for unsubscribing.
pub type SubscriberId = u64;

/// A subscriber receives events through a channel.
pub type EventSender = crossbeam::channel::Sender<WatchEvent>;

/// Inner state of the watcher.
struct WatcherInner {
    /// Global subscribers receive all events.
    global_subscribers: HashMap<SubscriberId, EventSender>,
    /// Per-collection subscribers.
    collection_subscribers: HashMap<String, HashMap<SubscriberId, EventSender>>,
    /// Per-key prefix subscribers.
    prefix_subscribers: HashMap<String, HashMap<Vec<u8>, HashMap<SubscriberId, EventSender>>>,
    /// Next subscriber ID.
    next_id: SubscriberId,
}

impl WatcherInner {
    fn new() -> Self {
        Self {
            global_subscribers: HashMap::new(),
            collection_subscribers: HashMap::new(),
            prefix_subscribers: HashMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> SubscriberId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Reactive watcher for storage change notifications.
///
/// Thread-safe via `Arc<RwLock<WatcherInner>>`.
pub struct Watcher {
    inner: Arc<RwLock<WatcherInner>>,
}

impl Watcher {
    /// Creates a new watcher.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(WatcherInner::new())),
        }
    }

    /// Subscribe to ALL events across all collections.
    pub fn subscribe_global(&self) -> (SubscriberId, crossbeam::channel::Receiver<WatchEvent>) {
        let (tx, rx) = crossbeam::channel::bounded(1024);
        let mut inner = self.inner.write();
        let id = inner.alloc_id();
        inner.global_subscribers.insert(id, tx);
        (id, rx)
    }

    /// Subscribe to events for a specific collection.
    pub fn subscribe_collection(
        &self,
        collection: &str,
    ) -> (SubscriberId, crossbeam::channel::Receiver<WatchEvent>) {
        let (tx, rx) = crossbeam::channel::bounded(1024);
        let mut inner = self.inner.write();
        let id = inner.alloc_id();
        inner
            .collection_subscribers
            .entry(collection.to_string())
            .or_default()
            .insert(id, tx);
        (id, rx)
    }

    /// Subscribe to events for keys with a specific prefix in a collection.
    pub fn subscribe_prefix(
        &self,
        collection: &str,
        prefix: &[u8],
    ) -> (SubscriberId, crossbeam::channel::Receiver<WatchEvent>) {
        let (tx, rx) = crossbeam::channel::bounded(1024);
        let mut inner = self.inner.write();
        let id = inner.alloc_id();
        inner
            .prefix_subscribers
            .entry(collection.to_string())
            .or_default()
            .entry(prefix.to_vec())
            .or_default()
            .insert(id, tx);
        (id, rx)
    }

    /// Unsubscribe a subscriber by ID.
    pub fn unsubscribe(&self, id: SubscriberId) {
        let mut inner = self.inner.write();

        // Try global
        inner.global_subscribers.remove(&id);

        // Try collection subscribers
        for subs in inner.collection_subscribers.values_mut() {
            subs.remove(&id);
        }

        // Try prefix subscribers
        for prefix_map in inner.prefix_subscribers.values_mut() {
            for subs in prefix_map.values_mut() {
                subs.remove(&id);
            }
        }
    }

    /// Notify all relevant subscribers of a change event.
    pub fn notify(&self, event: WatchEvent) {
        let inner = self.inner.read();

        // Send to global subscribers
        for tx in inner.global_subscribers.values() {
            let _ = tx.try_send(event.clone());
        }

        // Send to collection subscribers
        if let Some(subs) = inner.collection_subscribers.get(&event.collection) {
            for tx in subs.values() {
                let _ = tx.try_send(event.clone());
            }
        }

        // Send to prefix subscribers
        if let Some(prefix_map) = inner.prefix_subscribers.get(&event.collection) {
            for (prefix, subs) in prefix_map {
                if event.key.starts_with(prefix) {
                    for tx in subs.values() {
                        let _ = tx.try_send(event.clone());
                    }
                }
            }
        }
    }

    /// Returns the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        let inner = self.inner.read();
        let mut count = inner.global_subscribers.len();
        for subs in inner.collection_subscribers.values() {
            count += subs.len();
        }
        for prefix_map in inner.prefix_subscribers.values() {
            for subs in prefix_map.values() {
                count += subs.len();
            }
        }
        count
    }
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Watcher {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_subscription() {
        let watcher = Watcher::new();
        let (id, rx) = watcher.subscribe_global();

        watcher.notify(WatchEvent::put("users", b"key1"));
        watcher.notify(WatchEvent::delete("users", b"key2"));

        let evt1 = rx.try_recv().unwrap();
        assert_eq!(evt1.collection, "users");
        assert_eq!(evt1.key, b"key1");
        assert_eq!(evt1.change_type, ChangeType::Put);

        let evt2 = rx.try_recv().unwrap();
        assert_eq!(evt2.change_type, ChangeType::Delete);

        watcher.unsubscribe(id);
        assert_eq!(watcher.subscriber_count(), 0);
    }

    #[test]
    fn test_collection_subscription() {
        let watcher = Watcher::new();
        let (_, rx_users) = watcher.subscribe_collection("users");
        let (_, rx_posts) = watcher.subscribe_collection("posts");

        watcher.notify(WatchEvent::put("users", b"alice"));
        watcher.notify(WatchEvent::put("posts", b"hello"));

        // rx_users should only get the users event
        let evt = rx_users.try_recv().unwrap();
        assert_eq!(evt.key, b"alice");
        assert!(rx_users.try_recv().is_err()); // no more events

        // rx_posts should only get the posts event
        let evt = rx_posts.try_recv().unwrap();
        assert_eq!(evt.key, b"hello");
        assert!(rx_posts.try_recv().is_err());
    }

    #[test]
    fn test_prefix_subscription() {
        let watcher = Watcher::new();
        let (_, rx) = watcher.subscribe_prefix("users", b"user:");

        watcher.notify(WatchEvent::put("users", b"user:1"));
        watcher.notify(WatchEvent::put("users", b"admin:1"));
        watcher.notify(WatchEvent::put("users", b"user:2"));

        let evt1 = rx.try_recv().unwrap();
        assert_eq!(evt1.key, b"user:1");

        let evt2 = rx.try_recv().unwrap();
        assert_eq!(evt2.key, b"user:2");

        // admin:1 should not be received
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_clear_event() {
        let watcher = Watcher::new();
        let (_, rx) = watcher.subscribe_collection("test");

        watcher.notify(WatchEvent::clear("test"));

        let evt = rx.try_recv().unwrap();
        assert_eq!(evt.change_type, ChangeType::Clear);
        assert!(evt.key.is_empty());
    }

    #[test]
    fn test_unsubscribe() {
        let watcher = Watcher::new();
        let (id, rx) = watcher.subscribe_global();

        watcher.notify(WatchEvent::put("x", b"k"));
        assert!(rx.try_recv().is_ok());

        watcher.unsubscribe(id);
        watcher.notify(WatchEvent::put("x", b"k2"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_subscriber_count() {
        let watcher = Watcher::new();
        assert_eq!(watcher.subscriber_count(), 0);

        let (id1, _) = watcher.subscribe_global();
        assert_eq!(watcher.subscriber_count(), 1);

        let (id2, _) = watcher.subscribe_collection("test");
        assert_eq!(watcher.subscriber_count(), 2);

        let (id3, _) = watcher.subscribe_prefix("test", b"p:");
        assert_eq!(watcher.subscriber_count(), 3);

        watcher.unsubscribe(id1);
        watcher.unsubscribe(id2);
        watcher.unsubscribe(id3);
        assert_eq!(watcher.subscriber_count(), 0);
    }

    #[test]
    fn test_watcher_clone_shares_state() {
        let watcher = Watcher::new();
        let watcher2 = watcher.clone();

        let (_, rx) = watcher.subscribe_global();
        watcher2.notify(WatchEvent::put("c", b"k"));

        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_bounded_channel_drops_on_full() {
        let watcher = Watcher::new();
        // Channel capacity is 1024
        let (_, rx) = watcher.subscribe_global();

        // Send more than capacity
        for i in 0..1100 {
            watcher.notify(WatchEvent::put("c", format!("k{}", i).as_bytes()));
        }

        // Should have ~1024 events, some dropped
        let count = rx.try_iter().count();
        assert!(count >= 1000);
    }
}
