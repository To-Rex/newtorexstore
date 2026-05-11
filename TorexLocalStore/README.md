<div align="center">

# ⚡ TorexLocalStore

**Ultra-high-performance local storage for Flutter — powered by Rust**

[![Rust](https://img.shields.io/badge/Rust-1.78+-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Flutter](https://img.shields.io/badge/Flutter-3.29+-blue?logo=flutter&logoColor=white)](https://flutter.dev/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/Platform-iOS%20%7C%20Android%20%7C%20macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](https://pub.dev/)
[![flutter_rust_bridge](https://img.shields.io/badge/flutter__rust__bridge-2.x-blueviolet)](https://cjycode.com/flutter_rust_bridge/)

*LSM-tree engine · mmap zero-copy reads · Bloom filter miss detection · Crash recovery · Reactive streams*

</div>

---

TorexLocalStore is a **zero-configuration embedded key-value database** for Flutter apps. The storage engine is written entirely in Rust — an LSM-tree with memory-mapped segments, per-segment Bloom filters, append-only WAL, and background compaction. The Dart API wraps all of that behind a clean, async-friendly interface with no boilerplate: no `open()`, no `dispose()`, no schema.

```dart
await Torex.box("users").put("u:1", "Alice");
final name = await Torex.box("users").get("u:1"); // 34 ns memtable hit
```

---

## Table of Contents

- [Performance](#-performance)
- [Architecture](#-architecture)
- [Quick Start](#-quick-start)
- [API Reference](#-api-reference)
  - [CRUD](#crud)
  - [JSON helpers](#json-helpers)
  - [Typed objects](#typed-objects)
  - [Batch operations](#batch-operations)
  - [Query builder](#query-builder)
  - [Scan shortcuts](#scan-shortcuts)
  - [Reactive streams](#reactive-streams)
  - [Stats & management](#stats--management)
- [Configuration profiles](#-configuration-profiles)
- [Platform support & build instructions](#-platform-support--build-instructions)
- [Rust modules](#-rust-modules)
- [File format](#-file-format)
- [Background workers](#-background-workers)
- [Installation](#-installation)
- [Contributing](#-contributing)
- [License](#-license)

---

## ⚡ Performance

All numbers are **measured Criterion benchmarks** on Apple M-series hardware. Cold numbers include mmap page faults; warm numbers reflect steady-state throughput.

### Operation latency

| Operation | Latency | Notes |
|---|---|---|
| `get` — memtable hit | **34 ns** | Pure BTreeMap lookup, zero I/O |
| `get` — segment hit (mmap) | **294 ns** | Zero-copy read via memory-mapped file |
| `get` — miss (Bloom filter) | **28 ns** | Probabilistic short-circuit, 1% FP rate |
| `put` 64 B value | **~1 µs** | Memtable write + WAL append |
| `put` 1 KB value | **~9 µs** | Includes LZ4 compression pass |
| `delete` | **595 ns** | Tombstone write to memtable + WAL |
| `batch_put` 100 entries | **17 µs** | Single WAL fsync → **5.8M entries/sec** |
| `batch_put` 1 000 entries | **182 µs** | Single WAL fsync → **5.3M entries/sec** |
| Mixed put + get | — | **6.3M ops/sec** sustained |

### Comparison with common alternatives

| Store | `get` latency | Write throughput | Zero-copy | Crash-safe | Cross-platform FFI |
|---|---|---|---|---|---|
| **TorexLocalStore** | **34–294 ns** | **5–6M ops/s** | ✅ mmap | ✅ WAL | ✅ all 5 platforms |
| `shared_preferences` | ~50 µs | ~10K ops/s | ❌ | ❌ | ✅ |
| `Hive` (pure Dart) | ~1–5 µs | ~200K ops/s | ❌ | ❌ | ✅ |
| `sqflite` (SQLite) | ~100–500 µs | ~50K ops/s | ❌ | ✅ WAL | ⚠️ no desktop |
| `ObjectBox` | ~1–10 µs | ~500K ops/s | ❌ | ✅ | ✅ |

> **Note:** comparisons are approximate and workload-dependent. Torex shines for high-frequency, small-value workloads (event stores, caches, session data, offline sync queues).

---

## 🏗 Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  Flutter / Dart (async API)                                         │
│   Torex.box("users").put(k, v)  ──►  TorexBox  ──►  FFI bridge     │
└─────────────────────────────────────────┬───────────────────────────┘
                                          │  flutter_rust_bridge (2.x)
                                          ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Rust Engine                                                        │
│                                                                     │
│  ┌──────────┐  write   ┌──────────┐  flush   ┌──────────────────┐  │
│  │ Dart API │ ───────► │Memtable  │ ───────► │  Segment file    │  │
│  │  (api.rs)│          │(BTreeMap)│          │ (sorted, immut.) │  │
│  └──────────┘          └──────────┘          │  + Bloom filter  │  │
│        │                    │                │  + mmap reader   │  │
│        │               WAL write             └──────────────────┘  │
│        │                    │                         │             │
│        │                    ▼                         │ compaction  │
│        │             ┌──────────┐                     ▼             │
│        │             │  WAL     │           ┌──────────────────┐   │
│        │             │(BufWriter│           │ Compacted segment │   │
│        │             │ 256 KB   │           │ (merged, sorted) │   │
│        │             │ append)  │           └──────────────────┘   │
│        │             └──────────┘                                   │
│        │                                                            │
│        ▼                                                            │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Background Workers                                         │   │
│  │   · WAL flush every 200 ms                                  │   │
│  │   · Compaction every 5 s (adaptive, threshold-based)        │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Watcher / Reactive Streams                                 │   │
│  │   · crossbeam channels  ·  per-collection  ·  prefix-match  │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### Read path (latency-optimised)

```
get(key)
  │
  ├─► Memtable (BTreeMap)  ──found──►  return value          [34 ns]
  │
  ├─► Bloom filter (newest → oldest segment)
  │     │ not in filter ──────────►  miss (skip segment)     [28 ns]
  │     │ maybe in filter
  │     ▼
  └─► mmap segment scan  ──found──►  zero-copy return        [294 ns]
                          ──miss──►  None
```

### Write path (throughput-optimised)

```
put(key, value)
  │
  ├─► Append to WAL (BufWriter, 256 KB, sequential)          [~200 ns]
  ├─► Insert into Memtable                                   [~200 ns]
  └─► Optional fsync (sync_writes=true default)              [~600 ns]

  Background: flush memtable → segment when size > threshold
              merge segments when count > compaction_threshold
```

---

## 🚀 Quick Start

```dart
import 'package:torex_local_store/torex_local_store.dart';

// No init, no open, no dispose — just use it.
final box = Torex.box("notes");

await box.put("note:1", "Buy milk");
final note = await box.get("note:1");       // "Buy milk"
await box.putJson("note:2", {"text": "Call Alice", "done": false});
final json = await box.getJson("note:2");   // {"text": ..., "done": false}
await box.delete("note:1");
```

The engine auto-initialises on the first call, resolving a platform-appropriate path via `path_provider`. Collections are isolated namespaces — each gets its own memtable, WAL, and set of segments.

---

## 📖 API Reference

### CRUD

```dart
final box = Torex.box("collection_name");

// Write
await box.put("key", "value");

// Read
final String? value = await box.get("key");
final String value  = await box.getOrDefault("key", "fallback");

// Check existence (uses Bloom filter — very fast for misses)
final bool exists = await box.exists("key");

// Delete (writes a tombstone; physically removed on compaction)
await box.delete("key");
```

### JSON helpers

Convenience wrappers that `jsonEncode`/`jsonDecode` automatically:

```dart
// Store any JSON-encodable value (Map, List, String, num, bool)
await box.putJson("profile", {"name": "Ali", "age": 25});

// Retrieve and decode
final Map<String, dynamic>? profile = await box.getJson("profile");

// Atomic read-modify-write
await box.updateJson("profile", (current) => {
  ...?current,
  "age": (current?["age"] ?? 0) + 1,
});
```

### Typed objects

Use a `TorexCodec<T>` to encode/decode strongly-typed domain objects with no reflection:

```dart
// Define a codec
class UserCodec implements TorexCodec<User> {
  const UserCodec();

  @override
  Uint8List encode(User obj) => utf8.encode(jsonEncode(obj.toJson()));

  @override
  User decode(Uint8List bytes) => User.fromJson(jsonDecode(utf8.decode(bytes)));
}

// Usage
await box.putObject("u:1", myUser, const UserCodec());
final User? user = await box.getObject("u:1", const UserCodec());
```

### Batch operations

All entries in a batch share a **single WAL fsync**, making batches orders of magnitude faster than individual writes in a loop:

```dart
// Batch string writes (5.8M entries/sec at 100 entries per call)
await box.batchPut([
  ("k1", "value1"),
  ("k2", "value2"),
  ("k3", "value3"),
]);

// Batch deletes (single fsync)
await box.batchDelete(["k1", "k2", "k3"]);

// Batch JSON writes
await box.batchPutJson([
  ("u:3", {"name": "Bob",   "role": "admin"}),
  ("u:4", {"name": "Carol", "role": "viewer"}),
]);
```

### Query builder

A chainable query API for prefix scans, range scans, pagination, and ordering:

```dart
final results = await Torex.box("users")
    .query()
    .prefix("u:")          // keys starting with "u:"
    .startAt("u:100")      // lower bound (inclusive)
    .endAt("u:999")        // upper bound (inclusive)
    .limit(20)             // maximum results
    .offset(40)            // skip first 40 matches (pagination)
    .reverse()             // descending key order
    .find();               // returns List<TorexEntry>

// Each TorexEntry has:
//   entry.key   — String
//   entry.value — String
```

### Scan shortcuts

Convenience methods for the most common scan patterns:

```dart
// All keys with prefix
final List<TorexEntry> users = await box.scanPrefix("user:");

// All keys in lexicographic range [start, end]
final List<TorexEntry> range = await box.scanRange("a", "z");

// All keys in the collection
final List<String> allKeys = await box.keys();

// Approximate entry count (memtable + segment estimates)
final int count = await box.count();
```

### Reactive streams

Change streams are powered by Rust `crossbeam` channels, polled on the Flutter side. Subscribing is zero-allocation on the hot path:

```dart
// Watch all changes in a box
box.watch().listen((TorexChangeEvent event) {
  print("${event.type} → ${event.key}");
  // event.type: ChangeType.put | ChangeType.delete | ChangeType.clear
});

// Watch only keys matching a prefix
box.watchPrefix("u:").listen((event) {
  print("User changed: ${event.key}");
});

// Cancel as usual
final sub = box.watch().listen((_) {});
await sub.cancel();
```

### Stats & management

```dart
// Human-readable stats snapshot
final TorexBoxStats stats = await box.stats();
print(stats);
// TorexBoxStats(box: users, ~150 entries, memtable: 12 entries,
//               segments: 3, wal: 4.2 KB)

// Force memtable → segment flush (useful before app background)
await box.flush();

// Drop all data in this collection (irreversible)
await box.clear();

// Engine lifecycle (optional — auto-init is the default)
await Torex.initialize(path: "/custom/data/path");
await Torex.shutdown();
print(await Torex.version());    // "0.1.0"
print(await Torex.currentPath()); // "/data/user/0/com.example/..."
```

---

## ⚙️ Configuration profiles

Use a named profile when the default settings don't fit your workload. Pass the config when explicitly initialising the engine:

```dart
await Torex.initializeWithConfig(
  TorexConfig.highThroughput("/path/to/data"),
);
```

| Profile | Memtable | fsync | Compression | Workers | Best for |
|---|---|---|---|---|---|
| `TorexConfig.new(path)` | 4 MB | ✅ every write | ✅ LZ4 | 2 | General-purpose, safe default |
| `TorexConfig.highThroughput(path)` | 64 MB | ❌ async | ❌ | CPU count (≥4) | Write-heavy apps, event logging |
| `TorexConfig.lowMemory(path)` | 1 MB | ✅ every write | ✅ LZ4 | 1 | IoT / constrained devices |
| `TorexConfig.ultra(path)` | 128 MB | ❌ async | ❌ | CPU count (≥4) | Benchmarks / ephemeral caches |

> **⚠️ Durability note:** Profiles with `sync_writes: false` (`highThroughput`, `ultra`) may lose the last WAL buffer (up to 200 ms of writes) on a hard crash. Use only when that trade-off is acceptable.

### Full config reference (Rust)

```rust
TorexConfig {
    path: PathBuf,              // Base directory for all storage files
    memtable_size: usize,       // Flush threshold (bytes). Default: 4 MB
    wal_max_size: u64,          // WAL rotation size. Default: 8 MB
    segment_max_size: u64,      // Max segment file size. Default: 16 MB
    compaction_threshold: usize,// Segments to trigger merge. Default: 4
    block_size: usize,          // mmap read block size. Default: 4 KB
    sync_writes: bool,          // fsync on every write. Default: true
    compression: bool,          // LZ4 per-segment. Default: true
    worker_threads: usize,      // Background thread count. Default: 2
    verify_checksums: bool,     // CRC32 on reads. Default: true
}
```

---

## 📱 Platform support & build instructions

| Platform | Architecture(s) | Status |
|---|---|---|
| **iOS** | arm64 (device) + arm64/x86_64 (simulator) | ✅ Supported |
| **Android** | arm64-v8a · armeabi-v7a · x86_64 | ✅ Supported |
| **macOS** | Apple Silicon + Intel (universal binary) | ✅ Supported |
| **Linux** | x86_64 | ✅ Supported |
| **Windows** | x86_64 | ✅ Supported |

### Prerequisites

- **Rust** ≥ 1.78 with `cargo` ([rustup.rs](https://rustup.rs/))
- **Flutter** ≥ 3.29
- **Android NDK** r26+ (set `ANDROID_NDK_HOME`)
- **Xcode** 15+ (for iOS/macOS)
- **flutter_rust_bridge_codegen** 2.x (`cargo install flutter_rust_bridge_codegen`)

### Building native libraries

Pre-built binaries are included for convenience. Rebuild only when modifying Rust source:

```sh
cd TorexLocalStore

# iOS: device (arm64) + simulator (arm64 + x86_64)
./scripts/build_native.sh ios

# Android: all ABIs (arm64-v8a, armeabi-v7a, x86_64)
./scripts/build_native.sh android

# macOS: universal binary (arm64 + x86_64)
./scripts/build_native.sh macos

# Linux x86_64
./scripts/build_native.sh linux

# Windows x86_64
./scripts/build_native.sh windows
```

### Regenerating the FFI bridge

If you modify `rust/src/api.rs`, regenerate the Dart bindings:

```sh
cd TorexLocalStore
flutter_rust_bridge_codegen generate
```

---

## 🦀 Rust modules

The Rust crate (`torex_local_store`) is structured as independent, testable modules:

| Module | File | Responsibility |
|---|---|---|
| `api` | `api.rs` | FFI layer — all `pub fn torex_*` functions exposed to Dart via `flutter_rust_bridge` |
| `storage` | `storage.rs` | Core LSM-tree: orchestrates memtable, WAL, and segment reads/writes |
| `memtable` | `memtable.rs` | In-memory `BTreeMap` — sorted key-value store with O(log n) lookup |
| `wal` | `wal.rs` | Write-Ahead Log — `BufWriter` (256 KB), append-only, optional fsync |
| `segment` | `segment.rs` | Immutable sorted disk files — written on memtable flush, read via mmap |
| `mmap` | `mmap.rs` | Memory-mapped file abstraction over `memmap2` — zero-copy page-backed reads |
| `bloom` | `bloom.rs` | Per-segment Bloom filter — double-hashing with AHash, 1% false-positive rate |
| `compaction` | `compaction.rs` | Background segment merging — k-way merge of sorted segment iterators |
| `codec` | `codec.rs` | Binary serialisation: `[key_len:u16][key][value_len:u32][value][crc32:u32]` |
| `compress` | `compress.rs` | LZ4 (`lz4_flex`) compression — applied per-segment, skipped for small blocks (≤256 B) |
| `query` | `query.rs` | Scan / range / prefix query engine — unified view over memtable + segments |
| `index` | `index.rs` | Segment sparse index — O(log n) seek into large segment files |
| `watcher` | `watcher.rs` | Reactive change streams — `crossbeam` channels, collection and prefix subscriptions |
| `runtime` | `runtime.rs` | Background worker manager — WAL flush timer, compaction scheduler |
| `engine` | `engine.rs` | Multi-collection manager — opens, caches, and routes to per-collection `Storage` |
| `chunk` | `chunk.rs` | Large binary object (chunk) storage — splits big values across multiple entries |
| `transaction` | `transaction.rs` | Transactional write batching — atomic multi-key commit with single WAL fsync |
| `config` | `config.rs` | `TorexConfig` with named profile constructors |
| `error` | `error.rs` | `TorexError` enum — `thiserror`-derived, covers I/O, codec, and checksum errors |

---

## 📦 File format

### Segment file layout

Each segment is an immutable, append-only file produced when the memtable is flushed:

```
┌──────────────────────────────────────────────────┐
│  Magic header: "TRXS"  (4 bytes)                 │
├──────────────────────────────────────────────────┤
│  Version: u8           (1 byte)                  │
├──────────────────────────────────────────────────┤
│  Flags: u8             (compression flag, etc.)  │
├──────────────────────────────────────────────────┤
│  Entry block (repeated, keys in sorted order):   │
│  ┌────────────────────────────────────────────┐  │
│  │  key_len   : u16  (little-endian)          │  │
│  │  key       : [u8; key_len]                 │  │
│  │  value_len : u32  (little-endian)          │  │
│  │  value     : [u8; value_len]  (may be LZ4) │  │
│  │  crc32     : u32  (over key+value payload) │  │
│  └────────────────────────────────────────────┘  │
│  ... (N entries) ...                             │
├──────────────────────────────────────────────────┤
│  Bloom filter (serialised bit array)             │
├──────────────────────────────────────────────────┤
│  Sparse index (key → file offset, every N-th)    │
└──────────────────────────────────────────────────┘
```

- Keys are stored in **lexicographic order** — enabling efficient binary search and range scans.
- The **Bloom filter** is stored at the tail and loaded into RAM on segment open. Memory cost is ~1.2 KB per 10K keys at 1% FP rate.
- **CRC32** is computed at compile time via a `const fn` lookup table — zero runtime overhead for table generation.

### WAL entry layout

```
┌─────────────────────────────────────────────────────┐
│  op_type   : u8   (0=Put, 1=Delete, 2=BatchStart,  │
│                    3=BatchEnd)                      │
│  key_len   : u16                                    │
│  key       : [u8; key_len]                          │
│  value_len : u32  (0 for Delete)                    │
│  value     : [u8; value_len]                        │
│  crc32     : u32                                    │
└─────────────────────────────────────────────────────┘
```

On startup, the WAL is replayed sequentially to rebuild any memtable entries not yet flushed to a segment. This guarantees crash recovery with no data loss beyond the last fsync boundary.

### Directory layout on disk

```
<app_data>/torex/
├── boxes/
│   ├── users/
│   │   ├── wal-000001.trxw       ← append-only WAL
│   │   ├── seg-000001.trxs       ← immutable segment
│   │   ├── seg-000002.trxs
│   │   └── seg-000003.trxs
│   ├── products/
│   │   └── ...
│   └── settings/
│       └── ...
└── meta.trxm                     ← engine metadata
```

---

## 🔧 Background workers

The `runtime` module manages two background workers that run for the lifetime of the engine:

### WAL flush worker

- Runs every **200 ms**.
- Calls `BufWriter::flush()` on the active WAL to drain the 256 KB in-memory buffer to the OS page cache.
- Provides a durability bound of ~200 ms for writes made with `sync_writes: false`.
- Has negligible CPU impact — flush of an empty buffer is a no-op.

### Compaction worker

- Runs every **5 s**, adaptively.
- Triggers when the number of segment files for a collection exceeds `compaction_threshold` (default: 4).
- Performs a k-way merge of all candidate segments into a single new segment, then atomically swaps the file set.
- Old segments are deleted only after the new segment is fully written and synced.
- Compaction runs on a **dedicated thread** — it never blocks reads or writes.

### Worker lifecycle

```dart
// Workers start automatically on first use.
// They stop cleanly when the engine is shut down:
await Torex.shutdown();

// Re-initialize at any time:
await Torex.initialize();
```

---

## 📥 Installation

### Option 1 — Local path (monorepo)

```yaml
# pubspec.yaml
dependencies:
  torex_local_store:
    path: ../TorexLocalStore
```

### Option 2 — Git dependency

```yaml
# pubspec.yaml
dependencies:
  torex_local_store:
    git:
      url: https://github.com/torex/torexstore
      path: TorexLocalStore
```

Then run:

```sh
flutter pub get
```

No additional setup is required on any platform — the plugin uses FFI plugins for Android/Linux/Windows and a native plugin for iOS/macOS.

---

## 🤝 Contributing

Contributions are welcome! To contribute:

1. **Fork** the repository and create a feature branch.
2. **Rust changes** — run `cargo test` and `cargo bench` inside `TorexLocalStore/rust/`.
3. **Dart changes** — run `flutter test` from the package root.
4. **API changes** — regenerate the bridge with `flutter_rust_bridge_codegen generate`.
5. Open a **pull request** with a clear description of the change and any relevant benchmark comparisons.

### Running benchmarks

```sh
cd TorexLocalStore/rust
cargo bench --bench storage_bench
# HTML report at: target/criterion/report/index.html
```

### Running tests

```sh
# Rust unit tests
cd TorexLocalStore/rust && cargo test

# Flutter tests
cd TorexLocalStore && flutter test
```

---

## 📄 License

Copyright © 2025 Torex Contributors

Licensed under the **MIT License** — see [LICENSE](./LICENSE) for the full text.

---

<div align="center">

Built with ❤️ and 🦀 — Because your app's storage shouldn't be the bottleneck.

</div>
