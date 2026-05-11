# Changelog

All notable changes to TorexLocalStore will be documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## 0.1.3

**Internal improvements and bug fixes.**

---

## 0.1.2

**Fixed pub.dev score issues — improved static analysis, corrected repository URLs, included FRB-generated bindings in package.**

### Infrastructure
- Included FRB-generated Dart bindings (`lib/src/rust/`) in package to fix static analysis on pub.dev
- Fixed homepage, repository, issue tracker and documentation URLs to point to correct GitHub repository
- Resolved all Dart analysis warnings and lints

---

## 0.1.1

**Internal improvements and bug fixes.**

### Rust
- Various internal engine improvements and optimizations.

---

## 0.1.0

**Initial release — ultra-high-performance embedded storage for Flutter.**

### Storage Engine (Rust)
- LSM-tree core: Memtable → WAL → immutable Segments
- Memory-mapped I/O (`mmap`) for zero-copy segment reads
- Per-segment Bloom filters — O(1) miss detection at **28 ns**
- Append-only Write-Ahead Log (WAL) with 256 KB BufWriter
- Background WAL flush worker (every 200 ms)
- Background compaction worker (every 5 s, adaptive threshold)
- LZ4 compression for segment data blocks (>256 B)
- CRC32 integrity verification per WAL entry and segment
- Lock-free read path — readers never block writers
- Crash recovery via WAL replay on startup

### Dart API
- `Torex.box(name)` — zero-config, auto-initialising collections
- Full CRUD: `put`, `get`, `delete`, `exists`, `getOrDefault`, `update`
- JSON helpers: `putJson`, `getJson`, `getJsonOrDefault`, `updateJson`
- Typed objects: `putObject<T>`, `getObject<T>` with `TorexCodec<T>`
- Built-in codecs: `TorexJsonCodec`, `TorexStringCodec`, `TorexListCodec`
- Batch operations (single WAL fsync): `batchPut`, `batchDelete`, `batchPutJson`, `batchPutObjects`
- Bulk read: `getAll`, `getAllJson`
- Scan API: `scan`, `scanStrings`, `scanPrefix`, `scanRange`
- Fluent query builder: `.prefix()`, `.startKey()`, `.endKey()`, `.limit()`, `.offset()`, `.reverse()`
- Query terminals: `.find()`, `.findJson()`, `.findObjects<T>()`, `.findKeys()`
- Reactive streams: `watch()`, `watchPrefix()`
- Collection management: `flush()`, `clear()`, `count()`, `keys()`, `stats()`

### Configuration
- `TorexConfig.new(path)` — safe defaults (`sync_writes: true`, 4 MB memtable)
- `TorexConfig.high_throughput(path)` — 64 MB memtable, async writes, auto CPU workers
- `TorexConfig.low_memory(path)` — 1 MB memtable, minimal footprint
- `TorexConfig.ultra(path)` — 128 MB memtable, no fsync, maximum throughput

### Platform Support
- iOS (arm64 device · arm64 + x86_64 simulator)
- Android (arm64-v8a · armeabi-v7a · x86_64)
- macOS (Apple Silicon + Intel universal binary)
- Linux (x86_64)
- Windows (x86_64)

### Benchmark Results (Apple M-series)
| Operation | Latency |
|---|---|
| `get` — memtable hit | 34 ns |
| `get` — segment (mmap) | 294 ns |
| `get` — miss (Bloom) | 28 ns |
| `put` 64 B | ~1 µs |
| `batch_put` 100 entries | 17 µs (5.8 M/s) |
| `delete` | 595 ns |
| Mixed put + get | 6.3 M ops/s |
