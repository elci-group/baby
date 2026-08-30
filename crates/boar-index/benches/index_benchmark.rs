use std::time::{Duration, SystemTime};

use boar_core::{ArtifactId, StorageLocation};
use boar_index::{BoarIndex, IndexRecord};
use criterion::{Criterion, criterion_group, criterion_main};

fn build_record(seed: u64) -> IndexRecord {
    let mut record = IndexRecord::new(ArtifactId(format!("artifact-{seed:032x}")));
    record.size = 1024 * 1024;
    record.compressed_size = 512 * 1024;
    record.checksum = format!("digest-{seed:064x}");
    record.locations = vec![StorageLocation::Local {
        path: format!("/store/objects/{seed:032x}"),
    }];
    record.last_seen = SystemTime::now();
    record.observed_retrieval_latency_ns = 10_000_000;
    record.historical_compile_time_ns = 250_000_000;
    record.confidence = 95;
    record
}

fn populate(index: &mut BoarIndex, n: usize) {
    for i in 0..n {
        index.put(build_record(i as u64)).unwrap();
    }
}

fn bench_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_lookup");
    group.measurement_time(Duration::from_secs(5));

    for size in [100, 1_000, 10_000] {
        let mut mem = BoarIndex::open_in_memory();
        populate(&mut mem, size);
        let target = build_record(size as u64 / 2);

        group.bench_function(format!("memory/{size}"), |b| {
            b.iter(|| mem.get(&target.artifact_id).unwrap())
        });

        let dir = tempfile::tempdir().unwrap();
        let mut sqlite = BoarIndex::open_sqlite(dir.path().join("index.db")).unwrap();
        populate(&mut sqlite, size);

        group.bench_function(format!("sqlite/{size}"), |b| {
            b.iter(|| sqlite.get(&target.artifact_id).unwrap())
        });
    }

    group.finish();
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_insert");
    group.measurement_time(Duration::from_secs(5));

    for size in [100, 1_000, 10_000] {
        group.bench_function(format!("memory/{size}"), |b| {
            let mut index = BoarIndex::open_in_memory();
            let mut i = 0u64;
            b.iter(|| {
                index.put(build_record(i)).unwrap();
                i += 1;
            });
        });

        group.bench_function(format!("sqlite/{size}"), |b| {
            let dir = tempfile::tempdir().unwrap();
            let mut index = BoarIndex::open_sqlite(dir.path().join("index.db")).unwrap();
            let mut i = 0u64;
            b.iter(|| {
                index.put(build_record(i)).unwrap();
                i += 1;
            });
        });
    }

    group.finish();
}

fn bench_batch_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_batch_insert");
    group.measurement_time(Duration::from_secs(5));

    for size in [100, 1_000, 10_000] {
        group.bench_function(format!("memory/{size}"), |b| {
            let mut i = 0u64;
            b.iter(|| {
                let mut index = BoarIndex::open_in_memory();
                let records: Vec<_> = (0..size).map(|j| build_record(i + j as u64)).collect();
                index.put_batch(&records).unwrap();
                i += size as u64;
            });
        });

        group.bench_function(format!("sqlite/{size}"), |b| {
            let mut i = 0u64;
            b.iter(|| {
                let dir = tempfile::tempdir().unwrap();
                let mut index = BoarIndex::open_sqlite(dir.path().join("index.db")).unwrap();
                let records: Vec<_> = (0..size).map(|j| build_record(i + j as u64)).collect();
                index.put_batch(&records).unwrap();
                i += size as u64;
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_lookup, bench_insert, bench_batch_insert);
criterion_main!(benches);
