// TorexLocalStore — Example Application
//
// Demonstrates the full API surface:
//  • Basic CRUD (put / get / delete / exists)
//  • JSON storage
//  • Typed objects with TorexCodec
//  • Batch operations
//  • Query builder (prefix / range / reverse)
//  • Reactive watch streams
//  • Collection stats
//
// Run: flutter run
// Build native first: cd .. && ./scripts/build_native.sh <platform>

import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:torex_local_store/torex_local_store.dart';

void main() => runApp(const TorexExampleApp());

// ─── Example data model ───────────────────────────────────────────────────────

class User {
  final String id;
  final String name;
  final int age;

  const User({required this.id, required this.name, required this.age});

  Map<String, dynamic> toJson() => {'id': id, 'name': name, 'age': age};

  factory User.fromJson(Map<String, dynamic> j) =>
      User(id: j['id'] as String, name: j['name'] as String, age: j['age'] as int);

  @override
  String toString() => 'User($id, $name, $age)';
}

/// Custom TorexCodec for the User model.
class UserCodec extends TorexCodec<User> {
  const UserCodec();

  @override
  Uint8List encode(User value) =>
      Uint8List.fromList(utf8.encode(jsonEncode(value.toJson())));

  @override
  User decode(Uint8List bytes) =>
      User.fromJson(jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>);
}

// ─── App ──────────────────────────────────────────────────────────────────────

class TorexExampleApp extends StatelessWidget {
  const TorexExampleApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'TorexLocalStore Example',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepPurple),
        useMaterial3: true,
      ),
      home: const ExamplePage(),
    );
  }
}

// ─── ExamplePage ──────────────────────────────────────────────────────────────

class ExamplePage extends StatefulWidget {
  const ExamplePage({super.key});

  @override
  State<ExamplePage> createState() => _ExamplePageState();
}

class _ExamplePageState extends State<ExamplePage> {
  final List<String> _logs = [];
  bool _running = false;

  void _log(String line) => setState(() => _logs.insert(0, line));

  void _sep(String title) => _log('── $title ──────────────────');

  // ── Demos ──────────────────────────────────────────────────────────────────

  Future<void> _runBasicCrud() async {
    _sep('Basic CRUD');
    final box = Torex.box('demo');

    await box.put('greeting', 'Hello, Torex!');
    final val = await box.get('greeting');
    _log('put/get:     $val');

    final exists = await box.exists('greeting');
    _log('exists:      $exists');

    final missing = await box.getOrDefault('no_such_key', 'default_value');
    _log('getOrDefault: $missing');

    await box.update(
      'counter',
      (v) => '${(int.tryParse(v ?? '0') ?? 0) + 1}',
    );
    _log('update:      ${await box.get('counter')}');

    await box.delete('greeting');
    _log('after delete: ${await box.get('greeting')}');

    _log('✅ Basic CRUD OK');
  }

  Future<void> _runJsonStorage() async {
    _sep('JSON Storage');
    final box = Torex.box('demo');

    await box.putJson('config', {
      'theme': 'dark',
      'language': 'uz',
      'version': 1,
    });
    final config = await box.getJson('config');
    _log('putJson/getJson: $config');

    await box.updateJson(
      'config',
      (c) => {...?c, 'version': (c?['version'] as int? ?? 0) + 1},
    );
    _log('updateJson:  ${await box.getJson('config')}');

    final fallback = await box.getJsonOrDefault('no_key', {'status': 'empty'});
    _log('getJsonOrDefault: $fallback');

    _log('✅ JSON Storage OK');
  }

  Future<void> _runTypedObjects() async {
    _sep('Typed Objects (TorexCodec)');
    final box = Torex.box('users');
    const codec = UserCodec();

    final users = [
      const User(id: 'u:1', name: 'Ali', age: 25),
      const User(id: 'u:2', name: 'Vali', age: 30),
      const User(id: 'u:3', name: 'Hasan', age: 22),
    ];

    // Batch write with typed objects
    await box.batchPutObjects(
      users.map((u) => (u.id, u)).toList(),
      codec,
    );
    _log('batchPutObjects: ${users.length} users');

    final loaded = await box.getObject('u:2', codec);
    _log('getObject u:2:   $loaded');

    final fallback = await box.getObjectOrDefault(
      'u:999',
      codec,
      const User(id: 'u:999', name: 'Guest', age: 0),
    );
    _log('getObjectOrDefault: $fallback');

    _log('✅ Typed Objects OK');
  }

  Future<void> _runBatchOps() async {
    _sep('Batch Operations');
    final box = Torex.box('batch_demo');

    // Batch put — single WAL fsync for all 1000 entries
    final entries = List.generate(
      1000,
      (i) => ('item:${i.toString().padLeft(4, '0')}', 'value_$i'),
    );
    final sw = Stopwatch()..start();
    await box.batchPut(entries);
    sw.stop();
    _log('batchPut 1000: ${sw.elapsedMicroseconds} µs');

    // Batch JSON
    await box.batchPutJson([
      ('product:1', {'name': 'Phone', 'price': 500}),
      ('product:2', {'name': 'Laptop', 'price': 1200}),
    ]);
    _log('batchPutJson: 2 products');

    // getAll
    final results = await box.getAll(['item:0000', 'item:0001', 'no_key']);
    _log('getAll:       $results');

    // Batch delete
    await box.batchDelete(['item:0000', 'item:0001', 'item:0002']);
    _log('batchDelete:  3 keys removed');

    _log('✅ Batch Operations OK');
  }

  Future<void> _runQueryBuilder() async {
    _sep('Query Builder');
    final box = Torex.box('users');

    // All users with prefix "u:"
    final all = await box.query().prefix('u:').find();
    _log('prefix u::  ${all.length} results');

    // Limit + reverse
    final top2 = await box.query().prefix('u:').limit(2).reverse().find();
    _log('limit(2) reverse: ${top2.map((e) => e.$1).toList()}');

    // Range scan
    await Torex.box('items').batchPut(
      List.generate(5, (i) => ('item_${String.fromCharCode(65 + i)}', 'val')),
    );
    final range = await Torex.box('items')
        .query()
        .startKey('item_B')
        .endKey('item_D')
        .find();
    _log('range B..D: ${range.map((e) => e.$1).toList()}');

    // Scan shortcuts
    final prefixed = await box.scanPrefix('u:');
    _log('scanPrefix: ${prefixed.length} users');

    // findJson with query builder
    final jsonResults = await box
        .query()
        .prefix('product:')
        .findJson();
    _log('findJson products: ${jsonResults.map((e) => e.$2['name']).toList()}');

    _log('✅ Query Builder OK');
  }

  Future<void> _runWatchStream() async {
    _sep('Reactive Watch Stream');
    final box = Torex.box('watch_demo');

    final events = <String>[];
    final sub = box.watch().listen((e) {
      events.add('type=${e.changeType} key=${utf8.decode(e.key)}');
    });

    await box.put('watched_key', 'initial');
    await box.put('watched_key', 'updated');
    await box.delete('watched_key');

    // Give the poll loop time to catch up
    await Future.delayed(const Duration(milliseconds: 150));

    _log('watch events: ${events.length} received');
    for (final e in events) {
      _log('  → $e');
    }

    await sub.cancel();
    _log('✅ Watch Stream OK');
  }

  Future<void> _runStats() async {
    _sep('Stats & Management');
    final box = Torex.box('demo');

    final count = await box.count();
    _log('count:    $count');

    final keys = await box.keys();
    _log('keys:     $keys');

    final stats = await box.stats();
    _log('stats:    $stats');

    await box.flush();
    _log('flush:    OK');

    final path = await Torex.currentPath();
    _log('path:     $path');

    final version = await Torex.version();
    _log('version:  $version');

    _log('✅ Stats & Management OK');
  }

  Future<void> _runAll() async {
    setState(() {
      _running = true;
      _logs.clear();
    });
    try {
      await _runBasicCrud();
      await _runJsonStorage();
      await _runTypedObjects();
      await _runBatchOps();
      await _runQueryBuilder();
      await _runWatchStream();
      await _runStats();
      _log('');
      _log('🎉 All examples completed successfully!');
    } catch (e, st) {
      _log('❌ Error: $e');
      _log('   $st');
    } finally {
      setState(() => _running = false);
    }
  }

  // ── Build ──────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;

    return Scaffold(
      appBar: AppBar(
        backgroundColor: cs.inversePrimary,
        title: const Text('TorexLocalStore Example'),
        actions: [
          if (_logs.isNotEmpty)
            IconButton(
              icon: const Icon(Icons.clear_all),
              tooltip: 'Clear logs',
              onPressed: () => setState(_logs.clear),
            ),
        ],
      ),
      body: Column(
        children: [
          // ── Run button ────────────────────────────────────────────
          Padding(
            padding: const EdgeInsets.all(16),
            child: FilledButton.icon(
              onPressed: _running ? null : _runAll,
              icon: _running
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: Colors.white,
                      ),
                    )
                  : const Icon(Icons.play_arrow_rounded),
              label: Text(_running ? 'Running...' : 'Run All Examples'),
              style: FilledButton.styleFrom(
                minimumSize: const Size(double.infinity, 48),
              ),
            ),
          ),

          // ── Individual demos ──────────────────────────────────────
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Wrap(
              spacing: 8,
              runSpacing: 4,
              children: [
                _chip('CRUD', _runBasicCrud),
                _chip('JSON', _runJsonStorage),
                _chip('Typed', _runTypedObjects),
                _chip('Batch', _runBatchOps),
                _chip('Query', _runQueryBuilder),
                _chip('Watch', _runWatchStream),
                _chip('Stats', _runStats),
              ],
            ),
          ),

          const SizedBox(height: 8),

          // ── Console log ───────────────────────────────────────────
          Expanded(
            child: Container(
              color: Colors.black87,
              child: ListView.builder(
                padding: const EdgeInsets.all(10),
                itemCount: _logs.length,
                itemBuilder: (_, i) {
                  final line = _logs[i];
                  Color color = Colors.white60;
                  if (line.startsWith('✅') || line.startsWith('🎉')) {
                    color = Colors.greenAccent;
                  } else if (line.startsWith('❌')) {
                    color = Colors.redAccent;
                  } else if (line.startsWith('──')) {
                    color = Colors.yellowAccent;
                  }
                  return Text(
                    line,
                    style: TextStyle(
                      color: color,
                      fontFamily: 'monospace',
                      fontSize: 12,
                    ),
                  );
                },
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _chip(String label, Future<void> Function() fn) {
    return ActionChip(
      label: Text(label, style: const TextStyle(fontSize: 12)),
      onPressed: _running
          ? null
          : () async {
              setState(() {
                _running = true;
                _logs.clear();
              });
              try {
                await fn();
              } catch (e) {
                _log('❌ $e');
              } finally {
                setState(() => _running = false);
              }
            },
    );
  }
}
