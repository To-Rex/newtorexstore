//! Comprehensive benchmarks for the Torex storage engine.
//!
//! Uses `high_throughput` config (sync_writes=false, 64MB memtable)
//! to show the engine's true peak performance potential.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tempfile::TempDir;
use torex_local_store::config::TorexConfig;
use torex_local_store::storage::Storage;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_store(dir: &TempDir) -> Storage {
    let config = TorexConfig::high_throughput(dir.path().join("bench"));
    Storage::open(config).unwrap()
}

// ── individual put ────────────────────────────────────────────────────────────

fn bench_put(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    let mut group = c.benchmark_group("put");
    for size in [64usize, 256, 1024, 16_384, 65_536] {
        let data = vec![0xABu8; size];
        let mut i = 0u64;

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                let key = format!("k{}", i);
                store
                    .put(black_box(key.as_bytes()), black_box(&data))
                    .unwrap();
                i += 1;
            });
        });
    }
    group.finish();
}

// ── batch put ────────────────────────────────────────────────────────────────

fn bench_batch_put(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    let mut group = c.benchmark_group("batch_put");
    for batch_size in [10usize, 100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let entries: Vec<(Vec<u8>, Vec<u8>)> = (0..n)
                    .map(|i| (format!("bk_{}", i).into_bytes(), b"value_data".to_vec()))
                    .collect();

                b.iter(|| {
                    let refs: Vec<(&[u8], &[u8])> = entries
                        .iter()
                        .map(|(k, v)| (k.as_slice(), v.as_slice()))
                        .collect();
                    store.batch_put(black_box(&refs)).unwrap();
                });
            },
        );
    }
    group.finish();
}

// ── get ──────────────────────────────────────────────────────────────────────

fn bench_get(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    // Pre-populate: 10k entries in memtable + force a segment flush
    for i in 0..10_000u64 {
        store
            .put(format!("key_{:08}", i).as_bytes(), b"some_value_data")
            .unwrap();
    }
    store.flush_memtable().unwrap(); // ensure segment mmap is active

    let mut group = c.benchmark_group("get");

    // Hot path: key exists in memtable
    group.bench_function("memtable_hit", |b| {
        store.put(b"hot_key", b"hot_value").unwrap();
        b.iter(|| {
            store.get(black_box(b"hot_key")).unwrap();
        });
    });

    // Cold path: key must be read from segment via mmap
    group.bench_function("segment_hit_mmap", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = format!("key_{:08}", i % 10_000);
            store.get(black_box(key.as_bytes())).unwrap();
            i += 1;
        });
    });

    // Miss: key does not exist
    group.bench_function("miss", |b| {
        b.iter(|| {
            store.get(black_box(b"nonexistent_key_xyz")).unwrap();
        });
    });

    group.finish();
}

// ── delete ────────────────────────────────────────────────────────────────────

fn bench_delete(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    c.bench_function("delete", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = format!("dk_{}", i);
            store.put(key.as_bytes(), b"v").unwrap();
            store.delete(black_box(key.as_bytes())).unwrap();
            i += 1;
        });
    });
}

// ── batch delete ──────────────────────────────────────────────────────────────

fn bench_batch_delete(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    let mut group = c.benchmark_group("batch_delete");
    for batch_size in [10usize, 100, 1_000] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let keys: Vec<Vec<u8>> = (0..n)
                    .map(|i| format!("delkey_{}", i).into_bytes())
                    .collect();

                b.iter(|| {
                    // Pre-populate
                    let puts: Vec<(&[u8], &[u8])> = keys
                        .iter()
                        .map(|k| (k.as_slice(), b"v".as_slice()))
                        .collect();
                    store.batch_put(&puts).unwrap();

                    let refs: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
                    store.batch_delete(black_box(&refs)).unwrap();
                });
            },
        );
    }
    group.finish();
}

// ── mixed read/write ─────────────────────────────────────────────────────────

fn bench_mixed(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let store = make_store(&dir);

    // Seed data
    for i in 0..1000u64 {
        store.put(format!("m_{}", i).as_bytes(), b"val").unwrap();
    }

    let mut group = c.benchmark_group("mixed_rw");
    group.throughput(Throughput::Elements(2));
    group.bench_function("put_then_get", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let key = format!("m_{}", i % 1000);
            store
                .put(black_box(key.as_bytes()), black_box(b"newval"))
                .unwrap();
            store.get(black_box(key.as_bytes())).unwrap();
            i += 1;
        });
    });
    group.finish();
}

// ── registration ─────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_put,
    bench_batch_put,
    bench_get,
    bench_delete,
    bench_batch_delete,
    bench_mixed,
);
criterion_main!(benches);
