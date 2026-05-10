# Torex Local Storage

Ultra-high-performance embedded storage engine for Flutter, powered by Rust.

## Architecture

```
Flutter Layer (Dart API)
         ↓
    FFI Bridge Layer
         ↓
   Rust Core Engine
         ↓
  Storage Engine (LSM-Tree)
         ↓
     Filesystem
```

## Features

- **LSM-Tree Storage**: Writes go to in-memory memtable, flushed to sorted segments
- **WAL (Write-Ahead Log)**: Crash recovery with append-only logging
- **Binary Encoding**: Zero-copy binary serialization with CRC32 checksums
- **Segment Compaction**: Background merge of sorted segments
- **Collection-Oriented API**: Multiple independent boxes/collections
- **Cross-Platform**: Android, iOS, macOS, Linux, Windows

## Rust Core Modules

| Module | Description |
|--------|-------------|
| `engine` | Top-level engine managing multiple collections |
| `storage` | Core LSM-tree storage (memtable + WAL + segments) |
| `memtable` | In-memory sorted key-value table |
| `wal` | Write-Ahead Log for crash recovery |
| `segment` | Immutable sorted disk segments |
| `codec` | Binary encoding/decoding with CRC32 |
| `index` | Sparse in-memory index |
| `config` | Engine configuration |
| `api` | FFI API layer for flutter_rust_bridge |

## Usage

```dart
import 'package:torexstore/torex_store.dart';

// Open the store
final store = await TorexStore.open();

// Put and get values
await store.box('users').put('user1', 'Alice');
final name = await store.box('users').get('user1');

// Close when done
await store.close();
```

## Building

### Rust Core
```bash
cd TorexLocalStore
cargo build --release
cargo test
```

### Flutter App
```bash
flutter pub get
flutter run
```

## Performance Targets

- Ultra-fast reads via sorted segments + sparse index
- Ultra-fast writes via in-memory memtable + append-only WAL
- Low latency through minimal allocations
- Low RAM via bounded memtable size
- Crash recovery via WAL replay

## License

MIT
