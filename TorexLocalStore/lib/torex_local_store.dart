// Torex Local Storage — Ultra-high-performance local storage for Flutter.
//
// Powered by a Rust LSM-tree engine with mmap zero-copy reads,
// Bloom-filter misses, WAL crash recovery, and background compaction.
//
// ─── Quick Start ──────────────────────────────────────────────────────────
//
//   // String CRUD
//   await Torex.box("settings").put("theme", "dark");
//   final theme = await Torex.box("settings").get("theme");
//
//   // JSON objects
//   await Torex.box("users").putJson("u:1", {"name": "Ali", "age": 25});
//   final user = await Torex.box("users").getJson("u:1");
//
//   // Typed objects with custom codec
//   await Torex.box("users").putObject("u:2", myUser, const UserCodec());
//
//   // Batch writes (single WAL fsync — up to 5M writes/sec)
//   await Torex.box("logs").batchPut([("k1","v1"), ("k2","v2")]);
//
//   // Query builder
//   final results = await Torex.box("users")
//       .query()
//       .prefix("u:")
//       .limit(20)
//       .find();
//
//   // Reactive streams
//   Torex.box("users").watch().listen((event) {
//     print("Changed: ${event.changeType}");
//   });
//
// ─── No open/close/dispose required ───────────────────────────────────────

// Zero-config high-level API + all public types
export 'src/torex_store.dart'
    show
        Torex,
        TorexBox,
        TorexQueryBuilder,
        TorexCodec,
        TorexJsonCodec,
        TorexStringCodec,
        TorexListCodec,
        TorexBoxStats,
        TorexChangeType,
        TorexException,
        // ignore: deprecated_member_use_from_same_package
        TorexStore;

// FRB-generated low-level bindings (for advanced / direct FFI usage)
export 'src/rust/api.dart' show TorexEntry, TorexWatchEvent;
export 'src/rust/frb_generated.dart';
