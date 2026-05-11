// Torex Local Storage — Zero-Configuration Flutter API
//
// Ultra-high-performance local storage powered by Rust LSM-tree engine.
//
// Usage:
//   await Torex.box("users").put("key", "value");
//   final value = await Torex.box("users").get("key");
//   await Torex.box("users").putJson("profile", {"name": "Ali"});
//   final profile = await Torex.box("users").getJson("profile");
//
// No open/close/dispose required. Auto-initializes on first use.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:path_provider/path_provider.dart';

import 'rust/api.dart' as api;
import 'rust/frb_generated.dart';

// ─── FFI Bridge Initialization ──────────────────────────────────────────────

/// One-time FFI bridge initialization guard.
/// Uses a Completer so concurrent calls all await the same Future safely.
Completer<void>? _ffiInitCompleter;

/// Optional custom path set via [Torex.initialize].
String? _customTorexPath;

/// Initializes the Rust FFI bridge exactly once.
/// Safe to call concurrently from multiple isolates/widgets.
Future<void> _ensureFfiInitialized() async {
  if (_ffiInitCompleter != null) {
    return _ffiInitCompleter!.future;
  }
  _ffiInitCompleter = Completer<void>();
  try {
    if (Platform.isIOS || Platform.isMacOS) {
      await RustLib.init(
        externalLibrary: ExternalLibrary.process(iKnowHowToUseIt: true),
      );
    } else {
      await RustLib.init();
    }
    await _ensureTorexInitialized();
    _ffiInitCompleter!.complete();
  } catch (e, st) {
    // Reset so the next call can retry
    final c = _ffiInitCompleter!;
    _ffiInitCompleter = null;
    c.completeError(e, st);
    rethrow;
  }
}

/// One-time Torex runtime initialization guard.
Completer<void>? _torexInitCompleter;

/// Initializes the Torex runtime with the correct platform path.
Future<void> _ensureTorexInitialized() async {
  if (_torexInitCompleter != null) {
    return _torexInitCompleter!.future;
  }
  _torexInitCompleter = Completer<void>();
  try {
    final path = _customTorexPath ?? await _defaultStoragePath();
    await api.torexInitialize(path: path);
    _torexInitCompleter!.complete();
  } catch (e, st) {
    final c = _torexInitCompleter!;
    _torexInitCompleter = null;
    c.completeError(e, st);
    rethrow;
  }
}

/// Returns the platform-appropriate default storage directory.
Future<String> _defaultStoragePath() async {
  try {
    final dir = await getApplicationDocumentsDirectory();
    return '${dir.path}/torex_store';
  } catch (_) {
    return '/tmp/torex_store';
  }
}

// ─── TorexException ─────────────────────────────────────────────────────────

/// Typed exception thrown by all Torex operations on failure.
class TorexException implements Exception {
  final String message;
  const TorexException(this.message);

  @override
  String toString() => 'TorexException: $message';
}

// ─── TorexCodec<T> ──────────────────────────────────────────────────────────

/// Generic binary serialization contract.
///
/// Implement this to store any Dart object type inside a [TorexBox]:
/// ```dart
/// class UserCodec extends TorexCodec<User> {
///   const UserCodec();
///   @override Uint8List encode(User u) => utf8.encode(jsonEncode(u.toJson()));
///   @override User    decode(Uint8List b) => User.fromJson(jsonDecode(utf8.decode(b)));
/// }
///
/// await Torex.box("users").putObject("id:1", myUser, const UserCodec());
/// final user = await Torex.box("users").getObject("id:1", const UserCodec());
/// ```
abstract class TorexCodec<T> {
  const TorexCodec();
  Uint8List encode(T value);
  T decode(Uint8List bytes);
}

/// Built-in codec: `Map<String, dynamic>` ↔ UTF-8 JSON bytes.
class TorexJsonCodec implements TorexCodec<Map<String, dynamic>> {
  const TorexJsonCodec();

  @override
  Uint8List encode(Map<String, dynamic> value) =>
      Uint8List.fromList(utf8.encode(jsonEncode(value)));

  @override
  Map<String, dynamic> decode(Uint8List bytes) =>
      jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>;
}

/// Built-in codec: `String` ↔ UTF-8 bytes.
class TorexStringCodec implements TorexCodec<String> {
  const TorexStringCodec();

  @override
  Uint8List encode(String value) => Uint8List.fromList(utf8.encode(value));

  @override
  String decode(Uint8List bytes) => utf8.decode(bytes);
}

/// Built-in codec: `List<dynamic>` ↔ UTF-8 JSON bytes.
class TorexListCodec implements TorexCodec<List<dynamic>> {
  const TorexListCodec();

  @override
  Uint8List encode(List<dynamic> value) =>
      Uint8List.fromList(utf8.encode(jsonEncode(value)));

  @override
  List<dynamic> decode(Uint8List bytes) =>
      jsonDecode(utf8.decode(bytes)) as List<dynamic>;
}

// ─── TorexBoxStats ──────────────────────────────────────────────────────────

/// Diagnostic snapshot of a [TorexBox]'s internal state.
class TorexBoxStats {
  /// Collection name.
  final String name;

  /// Number of entries buffered in the in-memory memtable.
  final int memtableEntries;

  /// Number of on-disk segment files.
  final int segmentCount;

  /// Approximate total entry count (memtable + segments).
  final int approximateCount;

  const TorexBoxStats({
    required this.name,
    required this.memtableEntries,
    required this.segmentCount,
    required this.approximateCount,
  });

  @override
  String toString() =>
      'TorexBoxStats(box: $name, ~$approximateCount entries, '
      '$memtableEntries in memtable, $segmentCount segments)';
}

// ─── Torex — Main Entry Point ────────────────────────────────────────────────

/// Zero-configuration storage engine entry point.
///
/// ```dart
/// // No initialization required:
/// await Torex.box("users").put("user:1", "Alice");
/// final name = await Torex.box("users").get("user:1");
///
/// // Optional: explicit init with custom path (call at app startup)
/// await Torex.initialize(path: "/custom/path");
/// ```
class Torex {
  Torex._();

  /// Optional: initializes the engine with a custom storage [path].
  ///
  /// If [path] is omitted, uses [getApplicationDocumentsDirectory]/torex_store.
  /// Safe to call multiple times — subsequent calls are no-ops.
  static Future<void> initialize({String? path}) async {
    _customTorexPath = path;
    await _ensureFfiInitialized();
  }

  /// Returns a [TorexBox] for the named collection.
  ///
  /// The box auto-initializes on first data operation — no setup needed.
  static TorexBox box(String name) => TorexBox._(name);

  /// Returns the Rust engine version string.
  static Future<String> version() async {
    await _ensureFfiInitialized();
    return api.torexVersion();
  }

  /// Lists all currently open collection names.
  static Future<List<String>> listCollections() async {
    await _ensureFfiInitialized();
    return api.torexListCollections();
  }

  /// Returns the current storage path, or null if not yet initialized.
  static Future<String?> currentPath() async {
    await _ensureFfiInitialized();
    return api.torexCurrentPath();
  }

  /// Gracefully shuts down the engine and releases all resources.
  ///
  /// Not required in normal usage — the engine shuts down automatically.
  static Future<void> shutdown() async {
    await _ensureFfiInitialized();
    await api.torexShutdown();
    _ffiInitCompleter = null;
  }
}

// ─── TorexBox — Collection Operations ───────────────────────────────────────

/// A named key-value collection inside the Torex storage engine.
///
/// Each box is an independent LSM-tree instance with its own memtable,
/// WAL, and segment files. Supports:
/// - String, bytes, JSON, and typed-object CRUD
/// - Batch operations (single WAL fsync for N writes)
/// - Range / prefix scans with fluent query builder
/// - Reactive change streams
///
/// ```dart
/// final users = Torex.box("users");
/// await users.put("u:1", "Alice");
/// await users.putJson("u:2", {"name": "Bob", "age": 30});
/// final name = await users.get("u:1");
/// final profile = await users.getJson("u:2");
/// ```
class TorexBox {
  final String name;
  TorexBox._(this.name);

  // ── Helpers ────────────────────────────────────────────────────────────────

  /// Converts a Dart string to UTF-8 bytes for storage.
  static Uint8List _encodeKey(String key) =>
      Uint8List.fromList(utf8.encode(key));

  /// Converts UTF-8 bytes back to a Dart string.
  static String _decodeKey(Uint8List bytes) => utf8.decode(bytes);

  // ── Single-Record: Bytes ───────────────────────────────────────────────────

  /// Stores raw bytes under a raw-bytes key.
  Future<void> putBytes(Uint8List key, Uint8List value) async {
    await _ensureFfiInitialized();
    await api.torexPut(collection: name, key: key, value: value);
  }

  /// Retrieves raw bytes by a raw-bytes key.
  Future<Uint8List?> getBytes(Uint8List key) async {
    await _ensureFfiInitialized();
    return api.torexGet(collection: name, key: key);
  }

  /// Deletes a raw-bytes key.
  Future<void> deleteBytes(Uint8List key) async {
    await _ensureFfiInitialized();
    await api.torexDelete(collection: name, key: key);
  }

  /// Checks existence of a raw-bytes key.
  Future<bool> existsBytes(Uint8List key) async {
    await _ensureFfiInitialized();
    return api.torexExists(collection: name, key: key);
  }

  // ── Single-Record: Strings ─────────────────────────────────────────────────

  /// Stores a UTF-8 string value under a UTF-8 string key.
  Future<void> put(String key, String value) async {
    await _ensureFfiInitialized();
    await api.torexPutString(collection: name, key: key, value: value);
  }

  /// Retrieves a UTF-8 string value by key, or null if not found.
  Future<String?> get(String key) async {
    await _ensureFfiInitialized();
    return api.torexGetString(collection: name, key: key);
  }

  /// Deletes a string key.
  Future<void> delete(String key) async {
    await _ensureFfiInitialized();
    await api.torexDelete(collection: name, key: _encodeKey(key));
  }

  /// Returns true if the key exists in this box.
  Future<bool> exists(String key) async {
    await _ensureFfiInitialized();
    return api.torexExists(collection: name, key: _encodeKey(key));
  }

  /// Alias for [exists] — more idiomatic Dart naming.
  Future<bool> containsKey(String key) => exists(key);

  /// Returns the value for [key], or [defaultValue] if absent.
  ///
  /// ```dart
  /// final theme = await box.getOrDefault("theme", "light");
  /// ```
  Future<String> getOrDefault(String key, String defaultValue) async =>
      (await get(key)) ?? defaultValue;

  /// Atomically reads the current value, applies [updater], and writes back.
  ///
  /// ```dart
  /// await box.update("counter", (v) => "${(int.tryParse(v ?? "0") ?? 0) + 1}");
  /// ```
  Future<void> update(
    String key,
    String Function(String? current) updater,
  ) async {
    final current = await get(key);
    await put(key, updater(current));
  }

  // ── Single-Record: JSON ────────────────────────────────────────────────────

  /// Stores a JSON-serializable map under [key].
  ///
  /// ```dart
  /// await box.putJson("user:1", {"name": "Ali", "age": 25});
  /// ```
  Future<void> putJson(String key, Map<String, dynamic> value) async {
    await _ensureFfiInitialized();
    final bytes = Uint8List.fromList(utf8.encode(jsonEncode(value)));
    await api.torexPut(collection: name, key: _encodeKey(key), value: bytes);
  }

  /// Retrieves a JSON object by [key], or null if absent.
  ///
  /// ```dart
  /// final user = await box.getJson("user:1");
  /// print(user?["name"]); // "Ali"
  /// ```
  Future<Map<String, dynamic>?> getJson(String key) async {
    await _ensureFfiInitialized();
    final bytes = await api.torexGet(collection: name, key: _encodeKey(key));
    if (bytes == null) return null;
    return jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>;
  }

  /// Returns a JSON map for [key], or [defaultValue] if absent.
  Future<Map<String, dynamic>> getJsonOrDefault(
    String key,
    Map<String, dynamic> defaultValue,
  ) async =>
      (await getJson(key)) ?? defaultValue;

  /// Atomically reads a JSON map, applies [updater], and writes back.
  Future<void> updateJson(
    String key,
    Map<String, dynamic> Function(Map<String, dynamic>? current) updater,
  ) async {
    final current = await getJson(key);
    await putJson(key, updater(current));
  }

  // ── Single-Record: Typed Objects ───────────────────────────────────────────

  /// Stores a typed [value] serialized via [codec].
  ///
  /// ```dart
  /// await box.putObject("user:1", myUser, const UserCodec());
  /// ```
  Future<void> putObject<T>(String key, T value, TorexCodec<T> codec) async {
    await _ensureFfiInitialized();
    await api.torexPut(
      collection: name,
      key: _encodeKey(key),
      value: codec.encode(value),
    );
  }

  /// Retrieves a typed object by [key] using [codec], or null if absent.
  ///
  /// ```dart
  /// final user = await box.getObject("user:1", const UserCodec());
  /// ```
  Future<T?> getObject<T>(String key, TorexCodec<T> codec) async {
    await _ensureFfiInitialized();
    final bytes = await api.torexGet(collection: name, key: _encodeKey(key));
    if (bytes == null) return null;
    return codec.decode(bytes);
  }

  /// Returns a typed object for [key], or [defaultValue] if absent.
  Future<T> getObjectOrDefault<T>(
    String key,
    TorexCodec<T> codec,
    T defaultValue,
  ) async =>
      (await getObject(key, codec)) ?? defaultValue;

  // ── Batch Operations ───────────────────────────────────────────────────────

  /// Batch-stores multiple byte key-value pairs in a single WAL fsync.
  /// Up to 1000× faster than calling [putBytes] in a loop.
  Future<void> batchPutBytes(List<(Uint8List, Uint8List)> entries) async {
    await _ensureFfiInitialized();
    await api.torexBatchPut(collection: name, entries: entries);
  }

  /// Batch-stores multiple string key-value pairs in a single WAL fsync.
  Future<void> batchPut(List<(String, String)> entries) async {
    await _ensureFfiInitialized();
    final byteEntries = entries
        .map((e) => (
              Uint8List.fromList(utf8.encode(e.$1)),
              Uint8List.fromList(utf8.encode(e.$2)),
            ))
        .toList();
    await api.torexBatchPut(collection: name, entries: byteEntries);
  }

  /// Deprecated alias for [batchPut]. Use [batchPut] instead.
  @Deprecated('Use batchPut() instead — same signature, cleaner name.')
  Future<void> batchPutStrings(List<(String, String)> entries) =>
      batchPut(entries);

  /// Batch-stores multiple JSON objects in a single WAL fsync.
  Future<void> batchPutJson(
    List<(String, Map<String, dynamic>)> entries,
  ) async {
    await _ensureFfiInitialized();
    final byteEntries = entries
        .map((e) => (
              Uint8List.fromList(utf8.encode(e.$1)),
              Uint8List.fromList(utf8.encode(jsonEncode(e.$2))),
            ))
        .toList();
    await api.torexBatchPut(collection: name, entries: byteEntries);
  }

  /// Batch-stores multiple typed objects in a single WAL fsync.
  Future<void> batchPutObjects<T>(
    List<(String, T)> entries,
    TorexCodec<T> codec,
  ) async {
    await _ensureFfiInitialized();
    final byteEntries = entries
        .map((e) => (
              Uint8List.fromList(utf8.encode(e.$1)),
              codec.encode(e.$2),
            ))
        .toList();
    await api.torexBatchPut(collection: name, entries: byteEntries);
  }

  /// Batch-retrieves multiple keys. Missing keys map to null.
  ///
  /// ```dart
  /// final map = await box.getAll(["u:1", "u:2", "u:3"]);
  /// ```
  Future<Map<String, String?>> getAll(List<String> keys) async {
    final result = <String, String?>{};
    for (final key in keys) {
      result[key] = await get(key);
    }
    return result;
  }

  /// Batch-retrieves multiple JSON objects. Missing keys map to null.
  Future<Map<String, Map<String, dynamic>?>> getAllJson(
    List<String> keys,
  ) async {
    final result = <String, Map<String, dynamic>?>{};
    for (final key in keys) {
      result[key] = await getJson(key);
    }
    return result;
  }

  /// Batch-deletes multiple raw-bytes keys in a single WAL fsync.
  Future<void> batchDeleteBytes(List<Uint8List> keys) async {
    await _ensureFfiInitialized();
    await api.torexBatchDelete(collection: name, keys: keys);
  }

  /// Batch-deletes multiple string keys in a single WAL fsync.
  Future<void> batchDelete(List<String> keys) async {
    await _ensureFfiInitialized();
    final byteKeys =
        keys.map((k) => Uint8List.fromList(utf8.encode(k))).toList();
    await api.torexBatchDelete(collection: name, keys: byteKeys);
  }

  // ── Query / Scan ───────────────────────────────────────────────────────────

  /// Scans entries with byte-level control.
  Future<List<api.TorexEntry>> scan({
    Uint8List? prefix,
    Uint8List? startKey,
    Uint8List? endKey,
    int? limit,
    int? offset,
    bool reverse = false,
  }) async {
    await _ensureFfiInitialized();
    return api.torexScan(
      collection: name,
      prefix: prefix,
      startKey: startKey,
      endKey: endKey,
      limit: limit != null ? BigInt.from(limit) : null,
      offset: offset != null ? BigInt.from(offset) : null,
      reverse: reverse,
    );
  }

  /// Scans entries returning `(String key, String value)` tuples.
  Future<List<(String, String)>> scanStrings({
    String? prefix,
    String? startKey,
    String? endKey,
    int? limit,
    int? offset,
    bool reverse = false,
  }) async {
    await _ensureFfiInitialized();
    return api.torexScanStrings(
      collection: name,
      prefix: prefix,
      startKey: startKey,
      endKey: endKey,
      limit: limit != null ? BigInt.from(limit) : null,
      offset: offset != null ? BigInt.from(offset) : null,
      reverse: reverse,
    );
  }

  /// Scans all entries with a given string prefix.
  ///
  /// ```dart
  /// final users = await box.scanPrefix("user:");
  /// ```
  Future<List<(String, String)>> scanPrefix(
    String prefix, {
    int? limit,
    bool reverse = false,
  }) =>
      scanStrings(prefix: prefix, limit: limit, reverse: reverse);

  /// Scans a key range [startKey, endKey).
  Future<List<(String, String)>> scanRange(
    String startKey,
    String endKey, {
    int? limit,
    bool reverse = false,
  }) =>
      scanStrings(
        startKey: startKey,
        endKey: endKey,
        limit: limit,
        reverse: reverse,
      );

  /// Returns all keys as strings.
  Future<List<String>> keys() async {
    await _ensureFfiInitialized();
    final rawKeys = await api.torexKeys(collection: name);
    return rawKeys.map(_decodeKey).toList();
  }

  /// Returns all keys as raw byte arrays.
  Future<List<Uint8List>> keysBytes() async {
    await _ensureFfiInitialized();
    return api.torexKeys(collection: name);
  }

  /// Returns the approximate number of entries (memtable + segments).
  Future<int> count() async {
    await _ensureFfiInitialized();
    final c = await api.torexCount(collection: name);
    return c.toInt();
  }

  // ── Collection Management ──────────────────────────────────────────────────

  /// Forces an immediate flush of the in-memory buffer to disk.
  Future<void> flush() async {
    await _ensureFfiInitialized();
    await api.torexFlush(collection: name);
  }

  /// Deletes all entries in this box (non-reversible).
  Future<void> clear() async {
    await _ensureFfiInitialized();
    await api.torexClearCollection(collection: name);
  }

  /// Returns a diagnostic snapshot of this box's internal state.
  ///
  /// ```dart
  /// final stats = await Torex.box("users").stats();
  /// print(stats); // TorexBoxStats(box: users, ~1500 entries, ...)
  /// ```
  Future<TorexBoxStats> stats() async {
    await _ensureFfiInitialized();
    final mc = await api.torexMemtableCount(collection: name);
    final sc = await api.torexSegmentCount(collection: name);
    final c = await api.torexCount(collection: name);
    return TorexBoxStats(
      name: name,
      memtableEntries: mc.toInt(),
      segmentCount: sc.toInt(),
      approximateCount: c.toInt(),
    );
  }

  // ── Query Builder ──────────────────────────────────────────────────────────

  /// Creates a fluent query builder for advanced scans.
  ///
  /// ```dart
  /// final results = await Torex.box("users")
  ///     .query()
  ///     .prefix("user:")
  ///     .limit(50)
  ///     .reverse()
  ///     .find();
  /// ```
  TorexQueryBuilder query() => TorexQueryBuilder._(this);

  // ── Reactive Streams ───────────────────────────────────────────────────────

  /// Returns a [Stream] of [api.TorexWatchEvent] for any change in this box.
  ///
  /// Polls the Rust watcher at [interval] (default 50 ms).
  ///
  /// ```dart
  /// box.watch().listen((event) {
  ///   print("Changed: key=${utf8.decode(event.key)}");
  /// });
  /// ```
  Stream<api.TorexWatchEvent> watch({
    Duration interval = const Duration(milliseconds: 50),
  }) =>
      _TorexWatchStream(
        collection: name,
        prefix: null,
        interval: interval,
      ).stream;

  /// Returns a [Stream] for changes to keys starting with [prefix].
  Stream<api.TorexWatchEvent> watchPrefix(
    String prefix, {
    Duration interval = const Duration(milliseconds: 50),
  }) =>
      _TorexWatchStream(
        collection: name,
        prefix: prefix,
        interval: interval,
      ).stream;

  @override
  String toString() => 'TorexBox($name)';
}

// ─── TorexQueryBuilder ───────────────────────────────────────────────────────

/// Fluent query builder for advanced scans on a [TorexBox].
///
/// ```dart
/// final results = await Torex.box("users")
///     .query()
///     .prefix("user:")
///     .limit(50)
///     .offset(10)
///     .reverse()
///     .find();
/// ```
class TorexQueryBuilder {
  final TorexBox _box;
  Uint8List? _prefix;
  Uint8List? _startKey;
  Uint8List? _endKey;
  int? _limit;
  int? _offset;
  bool _reverse = false;

  TorexQueryBuilder._(this._box);

  /// Filters by byte prefix.
  TorexQueryBuilder prefixBytes(Uint8List prefix) {
    _prefix = prefix;
    return this;
  }

  /// Filters keys starting with [prefix] (UTF-8).
  TorexQueryBuilder prefix(String prefix) {
    _prefix = Uint8List.fromList(utf8.encode(prefix));
    return this;
  }

  /// Sets the inclusive start key (bytes).
  TorexQueryBuilder startKeyBytes(Uint8List key) {
    _startKey = key;
    return this;
  }

  /// Sets the inclusive start key (string).
  TorexQueryBuilder startKey(String key) {
    _startKey = Uint8List.fromList(utf8.encode(key));
    return this;
  }

  /// Sets the exclusive end key (bytes).
  TorexQueryBuilder endKeyBytes(Uint8List key) {
    _endKey = key;
    return this;
  }

  /// Sets the exclusive end key (string).
  TorexQueryBuilder endKey(String key) {
    _endKey = Uint8List.fromList(utf8.encode(key));
    return this;
  }

  /// Limits results to [count] entries.
  TorexQueryBuilder limit(int count) {
    _limit = count;
    return this;
  }

  /// Skips the first [count] results.
  TorexQueryBuilder offset(int count) {
    _offset = count;
    return this;
  }

  /// Returns results in reverse (descending) order.
  TorexQueryBuilder reverse() {
    _reverse = true;
    return this;
  }

  /// Executes and returns raw [api.TorexEntry] records.
  Future<List<api.TorexEntry>> findEntries() => _box.scan(
        prefix: _prefix,
        startKey: _startKey,
        endKey: _endKey,
        limit: _limit,
        offset: _offset,
        reverse: _reverse,
      );

  /// Executes and returns `(String key, String value)` pairs.
  Future<List<(String, String)>> find() {
    return _box.scanStrings(
      prefix: _prefix != null ? utf8.decode(_prefix!) : null,
      startKey: _startKey != null ? utf8.decode(_startKey!) : null,
      endKey: _endKey != null ? utf8.decode(_endKey!) : null,
      limit: _limit,
      offset: _offset,
      reverse: _reverse,
    );
  }

  /// Executes and returns only the keys.
  Future<List<String>> findKeys() async {
    final entries = await findEntries();
    return entries.map((e) => utf8.decode(e.key)).toList();
  }

  /// Executes and returns deserialized JSON objects.
  Future<List<(String, Map<String, dynamic>)>> findJson() async {
    final entries = await findEntries();
    return entries.map((e) {
      final key = utf8.decode(e.key);
      final value = jsonDecode(utf8.decode(e.value)) as Map<String, dynamic>;
      return (key, value);
    }).toList();
  }

  /// Executes and returns deserialized typed objects via [codec].
  Future<List<(String, T)>> findObjects<T>(TorexCodec<T> codec) async {
    final entries = await findEntries();
    return entries.map((e) {
      final key = utf8.decode(e.key);
      final value = codec.decode(e.value);
      return (key, value);
    }).toList();
  }
}

// ─── TorexChangeType ────────────────────────────────────────────────────────

/// Constants for [api.TorexWatchEvent.changeType].
class TorexChangeType {
  TorexChangeType._();
  static const int put = 0;
  static const int delete = 1;
  static const int clear = 2;
}

// ─── Internal Watch Stream ───────────────────────────────────────────────────

class _TorexWatchStream {
  final String collection;
  final String? prefix;
  final Duration interval;

  _TorexWatchStream({
    required this.collection,
    required this.prefix,
    required this.interval,
  });

  Stream<api.TorexWatchEvent> get stream {
    late StreamController<api.TorexWatchEvent> controller;
    BigInt? subId;

    Future<void> startPolling() async {
      try {
        await _ensureFfiInitialized();

        subId = prefix != null
            ? await api.torexWatchPrefix(
                collection: collection,
                prefix: utf8.encode(prefix!),
              )
            : await api.torexWatchCollection(collection: collection);

        while (!controller.isClosed) {
          try {
            final events = await api.torexWatchPoll(
              subscriptionId: subId!,
              maxEvents: 100,
            );
            for (final event in events) {
              if (!controller.isClosed) controller.add(event);
            }
          } catch (_) {
            // Transient poll error — continue
          }
          await Future.delayed(interval);
        }
      } catch (e, st) {
        if (!controller.isClosed) controller.addError(e, st);
      }
    }

    Future<void> stopPolling() async {
      if (subId != null) {
        try {
          await api.torexWatchUnsubscribe(subscriptionId: subId!);
        } catch (_) {}
        subId = null;
      }
    }

    controller = StreamController<api.TorexWatchEvent>(
      onListen: () => startPolling(),
      onCancel: () {
        stopPolling();
        controller.close();
      },
    );

    return controller.stream;
  }
}

// ─── Backward Compatibility ──────────────────────────────────────────────────

/// @deprecated Use [Torex] directly. Will be removed in a future version.
@Deprecated('Use Torex.box() instead — no open/close required.')
class TorexStore {
  bool _isOpen = false;
  String? _path;

  TorexStore._();

  @Deprecated('Use Torex.initialize() instead.')
  static Future<void> init() => _ensureFfiInitialized();

  @Deprecated('Use Torex.box() directly. Auto-initialization handles this.')
  static Future<TorexStore> open({String? path}) async {
    await _ensureFfiInitialized();
    final store = TorexStore._();
    final effectivePath = path ?? await _defaultStoragePath();
    await api.torexOpen(path: effectivePath);
    store
      .._path = effectivePath
      .._isOpen = true;
    return store;
  }

  @Deprecated('Use Torex.box() instead.')
  TorexBox box(String name) {
    _checkOpen();
    return TorexBox._(name);
  }

  @Deprecated('No longer needed. Resources are managed automatically.')
  Future<void> close() async {
    if (!_isOpen) return;
    await api.torexClose();
    _isOpen = false;
  }

  @Deprecated('Use Torex.version() instead.')
  Future<String> version() => Torex.version();

  @Deprecated('Use Torex.listCollections() instead.')
  Future<List<String>> listCollections() {
    _checkOpen();
    return Torex.listCollections();
  }

  bool get isOpen => _isOpen;
  String? get path => _path;

  void _checkOpen() {
    if (!_isOpen) {
      throw const TorexException('TorexStore is not open. Call open() first.');
    }
  }
}
