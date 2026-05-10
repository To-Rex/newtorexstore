import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
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
  TorexStore? _store;
  bool _isLoading = false;
  String _status = 'Not initialized';
  final List<String> _benchmarkResults = [];

  @override
  void dispose() {
    _store?.close();
    super.dispose();
  }

  Future<void> _initStore() async {
    setState(() {
      _isLoading = true;
      _status = 'Initializing...';
    });

    try {
      final dir = await getApplicationDocumentsDirectory();
      final path = '${dir.path}/torex_benchmark_db';
      _store = await TorexStore.open(path: path);

      setState(() {
        _status = 'Store opened at: $path';
        _isLoading = false;
      });
    } catch (e) {
      setState(() {
        _status = 'Error: $e';
        _isLoading = false;
      });
    }
  }

  Future<void> _runBenchmark(String name, Future<void> Function() fn) async {
    if (_store == null) {
      setState(() => _status = 'Store not initialized!');
      return;
    }

    setState(() {
      _isLoading = true;
      _status = 'Running: $name...';
    });

    final stopwatch = Stopwatch()..start();
    try {
      await fn();
      stopwatch.stop();

      final result = '$name: ${stopwatch.elapsedMilliseconds}ms';
      setState(() {
        _benchmarkResults.add(result);
        _status = 'Completed: $name';
        _isLoading = false;
      });
    } catch (e) {
      stopwatch.stop();
      setState(() {
        _benchmarkResults.add('$name: FAILED - $e');
        _status = 'Error in $name: $e';
        _isLoading = false;
      });
    }
  }

  Future<void> _benchmarkWrite() async {
    await _runBenchmark('Write 10,000 entries', () async {
      final box = _store!.box('benchmark');
      for (int i = 0; i < 10000; i++) {
        await box.put('key_$i', 'value_$i');
      }
    });
  }

  Future<void> _benchmarkRead() async {
    await _runBenchmark('Read 10,000 entries', () async {
      final box = _store!.box('benchmark');
      for (int i = 0; i < 10000; i++) {
        await box.get('key_$i');
      }
    });
  }

  Future<void> _benchmarkRandomRead() async {
    await _runBenchmark('Random read 10,000 entries', () async {
      final box = _store!.box('benchmark');
      final random = Random(42);
      for (int i = 0; i < 10000; i++) {
        final key = 'key_${random.nextInt(10000)}';
        await box.get(key);
      }
    });
  }

  Future<void> _benchmarkDelete() async {
    await _runBenchmark('Delete 5,000 entries', () async {
      final box = _store!.box('benchmark');
      for (int i = 0; i < 5000; i++) {
        await box.delete('key_$i');
      }
    });
  }

  Future<void> _benchmarkBatchWrite() async {
    await _runBenchmark('Batch write 10,000 entries', () async {
      final box = _store!.box('batch_bench');
      final entries = List.generate(
        10000,
        (i) => ('batch_key_$i', 'batch_value_$i'),
      );
      await box.batchPut(entries);
    });
  }

  Future<void> _benchmarkExists() async {
    await _runBenchmark('Exists check 10,000 entries', () async {
      final box = _store!.box('benchmark');
      for (int i = 0; i < 10000; i++) {
        await box.exists('key_$i');
      }
    });
  }

  void _clearResults() {
    setState(() {
      _benchmarkResults.clear();
      _status = 'Results cleared';
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Torex Local Storage Benchmark'),
        backgroundColor: Theme.of(context).colorScheme.inversePrimary,
      ),
      body: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Status bar
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.grey.shade200,
                borderRadius: BorderRadius.circular(8),
              ),
              child: Text(
                _status,
                style: const TextStyle(fontSize: 14, fontFamily: 'monospace'),
              ),
            ),
            const SizedBox(height: 16),

            // Initialize button
            ElevatedButton.icon(
              onPressed: _isLoading ? null : _initStore,
              icon: const Icon(Icons.folder_open),
              label: const Text('Initialize Store'),
            ),
            const SizedBox(height: 8),

            // Benchmark buttons
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                _buildBenchmarkButton('Write 10K', _benchmarkWrite),
                _buildBenchmarkButton('Read 10K', _benchmarkRead),
                _buildBenchmarkButton('Random Read', _benchmarkRandomRead),
                _buildBenchmarkButton('Delete 5K', _benchmarkDelete),
                _buildBenchmarkButton('Batch Write', _benchmarkBatchWrite),
                _buildBenchmarkButton('Exists 10K', _benchmarkExists),
                _buildBenchmarkButton('Clear', _clearResults),
              ],
            ),
            const SizedBox(height: 16),

            // Results
            const Text(
              'Benchmark Results:',
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 8),
            Expanded(
              child: Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: Colors.black87,
                  borderRadius: BorderRadius.circular(8),
                ),
                child: ListView.builder(
                  itemCount: _benchmarkResults.length,
                  itemBuilder: (context, index) {
                    return Padding(
                      padding: const EdgeInsets.symmetric(vertical: 2),
                      child: Text(
                        _benchmarkResults[index],
                        style: const TextStyle(
                          color: Colors.greenAccent,
                          fontFamily: 'monospace',
                          fontSize: 13,
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
    );
  }

  Widget _buildBenchmarkButton(String label, void Function() onPressed) {
    return ElevatedButton(
      onPressed: _isLoading ? null : onPressed,
      child: Text(label),
    );
  }
}
