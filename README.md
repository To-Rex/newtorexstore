# Torex Local Storage — Demo & Benchmark App

A Flutter test application that showcases and benchmarks **TorexLocalStore** — an ultra-high-performance embedded key-value storage engine for Flutter, powered by a Rust core with LSM-Tree architecture.

> **Note:** This repository is a monorepo. The runnable Flutter app lives at the root (`torexstore/`) and the storage package lives in the bundled `TorexLocalStore/` subdirectory, referenced as a local path dependency.

---

## Screenshots

| Benchmark Page | Data Manager Page |
|---|---|
| _Terminal-style console shows timestamped log lines for each benchmark step. A results summary panel accumulates timing results above the console. Buttons trigger individual benchmark suites (Write 10K, Read 10K, Random Read, Delete, Batch Write, Exists)._ | _Collection selector lets you switch between named boxes (`my_data`, `users`, `settings`, `products`, `orders`) or type a custom name. A key/value form adds new entries. The scrollable record list supports swipe-to-delete, inline edit/delete buttons, and copy-to-clipboard per entry._ |

---

## Features

### Benchmark Page
- **Write 10,000 entries** — sequential `put` with per-2,500-entry progress logs
- **Read 10,000 entries** — sequential `get` with hit-rate reporting
- **Random Read 10,000 entries** — seeded random-key `get` to test index locality
- **Delete 5,000 entries** — sequential `delete` half the benchmark dataset
- **Batch Write 10,000 entries** — single `batchPut` call to measure bulk ingestion throughput
- **Exists Check 10,000 entries** — `exists` query loop with hit-rate reporting
- **Terminal-style console** — colour-coded log lines (▶ yellow = start, ✔ green = success, ✘ red = error) with timestamps, up to 200 retained lines
- **Results panel** — accumulated `name: Xms` summary cards above the console
- **Zero-config API** — no explicit `open`/`close` needed; just call `Torex.box("name").put(key, value)`

### Data Manager Page (CrudPage)
- **Collection selector** — switch between `my_data`, `users`, `settings`, `products`, `orders`, or enter a custom collection name
- **Add / Edit form** — key + value text fields with validation; pre-fills on edit
- **Searchable record list** — live filter across all keys and values in the active collection
- **Swipe-to-delete** — dismiss a record with a swipe gesture
- **Per-row actions** — edit button and delete button on every record card
- **Copy to clipboard** — tap the copy icon to copy a value
- **Clear collection** — wipe all records from the active collection in one tap
- **Real-time refresh** — list reloads after every write, edit, or delete

---

## Tech Stack

| Layer | Technology |
|---|---|
| UI framework | Flutter 3.29 + Dart 3.11 |
| Design system | Material 3 (`useMaterial3: true`) |
| Navigation | `NavigationBar` + `IndexedStack` (2 tabs) |
| Storage package | `torex_local_store` (local path: `TorexLocalStore/`) |
| Platform directories | `path_provider ^2.1.5` |
| Storage core | Rust (LSM-Tree, WAL, mmap, bloom filters) |
| FFI bridge | `flutter_rust_bridge` |

---

## Prerequisites

- **Flutter SDK** ≥ 3.29 (`flutter --version`)
- **Dart SDK** ≥ 3.11 (bundled with Flutter)
- **Rust toolchain** via [rustup](https://rustup.rs/) (not Homebrew — see [note below](#homebrew-vs-rustup-on-macos))
  ```torexstore/TorexLocalStore/rust/src/lib.rs#L1-1
  // rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim
  ```
- **CocoaPods** (for iOS/macOS) — `gem install cocoapods`
- **Xcode** (for iOS builds) and/or **Android Studio** (for Android builds)

---

## Getting Started

### 1. Build the native Rust library

The Rust core must be compiled into a platform-specific native library before Flutter can use it.

```torexstore/TorexLocalStore/scripts/build_native.sh#L1-1
#!/usr/bin/env bash
```

**iOS (device + simulator):**
```torexstore/TorexLocalStore/scripts/build_xcframework.sh#L1-1
#!/usr/bin/env bash
```

Run from the repo root:

```/dev/null/shell.sh#L1-5
cd TorexLocalStore
./scripts/build_native.sh ios        # builds for iOS simulator & device
# or for macOS desktop:
./scripts/build_native.sh macos
cd ..
```

### 2. Install CocoaPods (iOS / macOS)

```/dev/null/shell.sh#L1-4
# iOS
cd ios && pod install && cd ..

# macOS (if running the desktop target)
cd macos && pod install && cd ..
```

### 3. Install Flutter dependencies

```/dev/null/shell.sh#L1-1
flutter pub get
```

### 4. Run the app

```/dev/null/shell.sh#L1-5
# iOS Simulator
flutter run -d "iPhone 16"

# Android
flutter run -d emulator-5554

# macOS desktop
flutter run -d macos
```

---

## Homebrew vs rustup on macOS

If you have **both** Homebrew Rust (`brew install rust`) and **rustup** installed, the build scripts may pick up the wrong `rustc`, causing linker errors or missing targets.

**Symptoms:**
- `rustup target add` succeeds but the build still fails with "target not found"
- Linker errors referencing architectures like `aarch64-apple-ios`

**Fix — Option A: set `RUSTC` explicitly before building**

```/dev/null/shell.sh#L1-2
export RUSTC=$(rustup which rustc)
./scripts/build_native.sh ios
```

**Fix — Option B: prepend `~/.cargo/bin` to `PATH` in your shell profile**

```/dev/null/shell.sh#L1-3
# Add to ~/.zshrc or ~/.bash_profile
export PATH="$HOME/.cargo/bin:$PATH"
```

This ensures `rustup`-managed binaries shadow any Homebrew equivalents. After editing, run `source ~/.zshrc` (or restart the terminal), then retry the build.

---

## Project Structure

```/dev/null/tree.txt#L1-35
torexstore/                        ← This Flutter app (repo root)
├── lib/
│   ├── main.dart                  ← App entry point, TorexStoreApp, BenchmarkPage
│   └── pages/
│       └── crud_page.dart         ← Data Manager (CRUD) page
│
├── TorexLocalStore/               ← Storage package (local path dependency)
│   ├── lib/
│   │   └── src/
│   │       └── torex_store.dart   ← Public Dart API (Torex, TorexBox)
│   │
│   ├── rust/
│   │   └── src/
│   │       ├── api.rs             ← FFI API layer (flutter_rust_bridge)
│   │       ├── engine.rs          ← Top-level engine, collection manager
│   │       ├── storage.rs         ← LSM-tree core (memtable + WAL + segments)
│   │       ├── memtable.rs        ← In-memory sorted key-value table
│   │       ├── wal.rs             ← Write-Ahead Log (crash recovery)
│   │       ├── segment.rs         ← Immutable sorted disk segments
│   │       ├── codec.rs           ← Binary encoding / CRC32 checksums
│   │       ├── index.rs           ← Sparse in-memory index
│   │       ├── bloom.rs           ← Bloom filter (negative lookup optimisation)
│   │       ├── mmap.rs            ← Memory-mapped file I/O
│   │       ├── compaction.rs      ← Background segment merge
│   │       ├── compress.rs        ← Block compression
│   │       ├── transaction.rs     ← Atomic multi-key transactions
│   │       ├── query.rs           ← Range / prefix query support
│   │       ├── watcher.rs         ← Change-notification watcher
│   │       ├── runtime.rs         ← Tokio async runtime bridge
│   │       ├── config.rs          ← Engine configuration
│   │       └── error.rs           ← Unified error types
│   │
│   └── scripts/
│       ├── build_native.sh        ← Cross-compile Rust for target platform
│       ├── build_xcframework.sh   ← Package into XCFramework (iOS)
│       └── organize_artifacts.sh  ← Copy compiled artifacts into Flutter plugin dirs
│
├── ios/                           ← iOS runner
├── android/                       ← Android runner
├── macos/                         ← macOS desktop runner
├── linux/                         ← Linux desktop runner
├── windows/                       ← Windows desktop runner
├── web/                           ← Web runner
├── test/                          ← Widget & unit tests
├── pubspec.yaml
└── README.md
```

---

## Page Details

### Benchmark Page (`lib/main.dart` — `BenchmarkPage`)

Runs isolated benchmark suites against the `benchmark` and `batch_bench` collections. Each suite is wrapped in a `Stopwatch` and the elapsed milliseconds are shown in both the results panel and the console.

**Benchmark suites:**

| Button | Operation | Collection | Entry count |
|---|---|---|---|
| Write 10K | Sequential `put` | `benchmark` | 10,000 |
| Read 10K | Sequential `get` | `benchmark` | 10,000 |
| Random Read | Seeded random `get` | `benchmark` | 10,000 |
| Delete 5K | Sequential `delete` | `benchmark` | 5,000 |
| Batch Write | Single `batchPut` | `batch_bench` | 10,000 |
| Exists 10K | Sequential `exists` | `benchmark` | 10,000 |

The console retains the last **200** log lines (newest at top). Each line is prefixed with an `HH:MM:SS.mmm` timestamp.

### Data Manager Page (`lib/pages/crud_page.dart` — `CrudPage`)

A fully interactive CRUD interface for exploring stored data:

1. **Collection selector** — a horizontal chip list of preset box names plus a text field for arbitrary names. Switching collections immediately reloads the record list.
2. **Add/Edit form** — two `TextField` widgets (Key, Value). When editing an existing entry, the fields are pre-populated and the submit button label changes to *Update*.
3. **Search bar** — filters the record list in real time without a server round-trip.
4. **Record list** — each card shows the key (bold) and value. Supports:
   - Swipe left → delete confirmation
   - Edit icon → populate form fields
   - Delete icon → immediate delete
   - Copy icon → copy value to clipboard via `Clipboard.setData`
5. **Clear All** button in the app bar wipes the entire active collection.

---

## Benchmark Results

Results below are indicative and depend on device, OS, and engine configuration (memtable flush threshold, compaction strategy, etc.).

| Operation | Dataset | Typical result | Notes |
|---|---|---|---|
| Sequential write | 10,000 entries | ~200 – 800 ms | Hot memtable path; WAL append-only |
| Sequential read (warm) | 10,000 entries | ~130 ms | Served from memtable |
| Sequential read (cold) | 10,000 entries | ~3,000 ms | Served from flushed segments via mmap |
| Random read | 10,000 entries | ~150 – 500 ms | Bloom filter cuts unnecessary disk seeks |
| Batch write | 10,000 entries | ~2 ms | ~5M entries/sec; bypasses per-entry overhead |
| Exists check | 10,000 entries | ~100 – 300 ms | Bloom filter short-circuits misses |
| Delete | 5,000 entries | ~100 – 400 ms | Tombstone written to WAL; cleaned up at compaction |

> Run the **Write 10K** benchmark first so subsequent reads have data to work with.

---

## Development Notes

### Rebuilding the Rust library after source changes

Any change to files under `TorexLocalStore/rust/src/` requires recompiling the native library:

```/dev/null/shell.sh#L1-5
cd TorexLocalStore

# iOS simulator + device
./scripts/build_native.sh ios

# macOS desktop
./scripts/build_native.sh macos
```

Then hot-restart the Flutter app (`r` in the terminal, or the restart button in your IDE). A full hot-reload (`R`) is **not** sufficient after native changes — you must stop and re-run.

### Generating FFI bindings

The FFI bridge code (`rust/src/frb_generated.rs` and the Dart counterpart) is generated by `flutter_rust_bridge`. After adding or changing `#[flutter_rust_bridge::frb]`-annotated functions in `api.rs`, regenerate with:

```/dev/null/shell.sh#L1-3
cd TorexLocalStore
flutter_rust_bridge_codegen generate
```

### Running tests

```/dev/null/shell.sh#L1-5
# Widget & unit tests
flutter test

# Rust unit tests
cd TorexLocalStore && cargo test
```

---

## Dependencies

```/dev/null/yaml.txt#L1-16
# pubspec.yaml (abridged)
dependencies:
  flutter:
    sdk: flutter
  cupertino_icons: ^1.0.8
  torex_local_store:
    path: TorexLocalStore        # local monorepo package
  path_provider: ^2.1.5

dev_dependencies:
  flutter_test:
    sdk: flutter
  flutter_lints: ^6.0.0
  ffi: ^2.1.4
  integration_test:
    sdk: flutter
```

---

## License

This project is licensed under the **MIT License** — see [`TorexLocalStore/LICENSE`](TorexLocalStore/LICENSE) for details.
