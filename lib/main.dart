import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:torex_local_store/torex_local_store.dart';

void main() {
  runApp(const TorexStoreApp());
}

class TorexStoreApp extends StatelessWidget {
  const TorexStoreApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Torex Local Storage - Benchmark',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepPurple),
        useMaterial3: true,
      ),
      home: const BenchmarkPage(),
    );
  }
}

class BenchmarkPage extends StatefulWidget {
  const BenchmarkPage({super.key});

  @override
  State<BenchmarkPage> createState() => _BenchmarkPageState();
}

class _BenchmarkPageState extends State<BenchmarkPage> {
  bool _isLoading = false;
  String _status = 'Ready — no initialization required';
  final List<String> _benchmarkResults = [];
  final List<String> _consoleLogs = [];

  void _log(String message) {
    final timestamp = DateTime.now().toIso8601String().substring(11, 23);
    final line = '[$timestamp] $message';
    debugPrint(line);
    setState(() {
      _consoleLogs.insert(0, line);
      // Keep last 200 lines
      if (_consoleLogs.length > 200) {
        _consoleLogs.removeRange(200, _consoleLogs.length);
      }
    });
  }

  Future<void> _runBenchmark(String name, Future<void> Function() fn) async {
    setState(() {
      _isLoading = true;
      _status = 'Running: $name...';
    });
    _log('▶ START: $name');

    final stopwatch = Stopwatch()..start();
    try {
      await fn();
      stopwatch.stop();

      final result = '$name: ${stopwatch.elapsedMilliseconds}ms';
      _log('✔ DONE: $result');
      setState(() {
        _benchmarkResults.add(result);
        _status = 'Completed: $name';
        _isLoading = false;
      });
    } catch (e, stackTrace) {
      stopwatch.stop();
      final result = '$name: FAILED - $e';
      _log('✘ ERROR: $result');
      _log('  Stack: $stackTrace');
      setState(() {
        _benchmarkResults.add(result);
        _status = 'Error in $name: $e';
        _isLoading = false;
      });
    }
  }

  // ─── Zero-Config Benchmarks ────────────────────────────────────

  Future<void> _benchmarkWrite() async {
    await _runBenchmark('Write 10,000 entries', () async {
      final box = Torex.box('benchmark');
      _log('  Using box: benchmark (auto-initialized)');
      for (int i = 0; i < 10000; i++) {
        await box.put('key_$i', 'value_$i');
        if (i % 2500 == 0) {
          _log('  Write progress: $i / 10000');
        }
      }
      _log('  Write complete: 10000 entries');
    });
  }

  Future<void> _benchmarkRead() async {
    await _runBenchmark('Read 10,000 entries', () async {
      final box = Torex.box('benchmark');
      int found = 0;
      for (int i = 0; i < 10000; i++) {
        final val = await box.get('key_$i');
        if (val != null) found++;
        if (i % 2500 == 0) {
          _log('  Read progress: $i / 10000 (found: $found)');
        }
      }
      _log('  Read complete: $found / 10000 found');
    });
  }

  Future<void> _benchmarkRandomRead() async {
    await _runBenchmark('Random read 10,000 entries', () async {
      final box = Torex.box('benchmark');
      final random = Random(42);
      int found = 0;
      for (int i = 0; i < 10000; i++) {
        final key = 'key_${random.nextInt(10000)}';
        final val = await box.get(key);
        if (val != null) found++;
      }
      _log('  Random read: $found / 10000 found');
    });
  }

  Future<void> _benchmarkDelete() async {
    await _runBenchmark('Delete 5,000 entries', () async {
      final box = Torex.box('benchmark');
      for (int i = 0; i < 5000; i++) {
        await box.delete('key_$i');
      }
      _log('  Delete complete: 5000 entries removed');
    });
  }

  Future<void> _benchmarkBatchWrite() async {
    await _runBenchmark('Batch write 10,000 entries', () async {
      final box = Torex.box('batch_bench');
      final entries = List.generate(
        10000,
        (i) => ('batch_key_$i', 'batch_value_$i'),
      );
      _log('  Batch size: ${entries.length} entries');
      await box.batchPutStrings(entries);
      _log('  Batch write complete');
    });
  }

  Future<void> _benchmarkExists() async {
    await _runBenchmark('Exists check 10,000 entries', () async {
      final box = Torex.box('benchmark');
      int exists = 0;
      for (int i = 0; i < 10000; i++) {
        final e = await box.exists('key_$i');
        if (e) exists++;
      }
      _log('  Exists: $exists / 10000');
    });
  }

  void _clearResults() {
    setState(() {
      _benchmarkResults.clear();
      _status = 'Results cleared';
    });
    _log('Results cleared');
  }

  void _clearConsole() {
    setState(() {
      _consoleLogs.clear();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Torex Local Storage Benchmark'),
        backgroundColor: Theme.of(context).colorScheme.inversePrimary,
      ),
      body: SafeArea(
        child: Column(
          children: [
            // ─── Status Bar ─────────────────────────────────────────
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.grey.shade200,
                border: Border(bottom: BorderSide(color: Colors.grey.shade300)),
              ),
              child: Text(
                _status,
                style: const TextStyle(fontSize: 14, fontFamily: 'monospace'),
              ),
            ),

            // ─── Info Banner ────────────────────────────────────────
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(8),
              color: Colors.blue.shade50,
              child: const Text(
                '✨ Zero-config: No open/close required. Just use Torex.box("name").put(key, value)',
                style: TextStyle(fontSize: 12, color: Colors.blue),
              ),
            ),

            // ─── Benchmark Buttons ──────────────────────────────────
            Padding(
              padding: const EdgeInsets.all(8.0),
              child: Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  _buildBenchmarkButton('Write 10K', _benchmarkWrite),
                  _buildBenchmarkButton('Read 10K', _benchmarkRead),
                  _buildBenchmarkButton('Random Read', _benchmarkRandomRead),
                  _buildBenchmarkButton('Delete 5K', _benchmarkDelete),
                  _buildBenchmarkButton('Batch Write', _benchmarkBatchWrite),
                  _buildBenchmarkButton('Exists 10K', _benchmarkExists),
                  _buildBenchmarkButton('Clear Results', _clearResults),
                ],
              ),
            ),

            // ─── Results Summary (scrollable) ────────────────────────
            if (_benchmarkResults.isNotEmpty)
              Container(
                width: double.infinity,
                height: 120,
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                color: Colors.green.shade50,
                child: ListView(
                  children: [
                    const Text(
                      'Results:',
                      style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold, color: Colors.green),
                    ),
                    ..._benchmarkResults.map((r) => Text(
                      r,
                      style: const TextStyle(fontSize: 11, fontFamily: 'monospace', color: Colors.green),
                    )),
                  ],
                ),
              ),

            // ─── Console Log ────────────────────────────────────────
            Expanded(
              child: Column(
                children: [
                  Container(
                    width: double.infinity,
                    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
                    color: Colors.grey.shade800,
                    child: Row(
                      children: [
                        const Text(
                          'Console',
                          style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold, color: Colors.white70),
                        ),
                        const Spacer(),
                        GestureDetector(
                          onTap: _clearConsole,
                          child: const Text(
                            'Clear',
                            style: TextStyle(fontSize: 11, color: Colors.white38),
                          ),
                        ),
                      ],
                    ),
                  ),
                  Expanded(
                    child: Container(
                      color: Colors.black87,
                      child: ListView.builder(
                        padding: const EdgeInsets.all(8),
                        itemCount: _consoleLogs.length,
                        itemBuilder: (context, index) {
                          final log = _consoleLogs[index];
                          Color color = Colors.greenAccent;
                          if (log.contains('✘') || log.contains('ERROR')) {
                            color = Colors.redAccent;
                          } else if (log.contains('▶')) {
                            color = Colors.yellowAccent;
                          } else if (log.contains('✔')) {
                            color = Colors.greenAccent;
                          } else {
                            color = Colors.white60;
                          }
                          return Padding(
                            padding: const EdgeInsets.symmetric(vertical: 1),
                            child: Text(
                              log,
                              style: TextStyle(
                                color: color,
                                fontFamily: 'monospace',
                                fontSize: 11,
                              ),
                            ),
                          );
                        },
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildBenchmarkButton(String label, void Function() onPressed) {
    return ElevatedButton(
      onPressed: _isLoading ? null : onPressed,
      child: Text(label),
    );
  }
}
