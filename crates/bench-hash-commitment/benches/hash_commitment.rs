//! Benchmark: hash function throughput comparison.
//!
//! Compares raw throughput of keccak256, SHA-256, and Blake3.
//! Groups are organized by input size so that the Criterion violin plots
//! directly compare algorithms at the same data size.
//!
//! Run:
//!   cargo bench -p bench-hash-commitment

use alloy_primitives::Keccak256;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sha2::{Digest as _, Sha256};

fn bench_hash_by_size(c: &mut Criterion) {
    for &size in &[32, 64, 256, 1024, 4096] {
        let mut group = c.benchmark_group(format!("hash_{size}B"));
        let data = vec![0xABu8; size];

        group.bench_function("keccak256", |b| {
            b.iter(|| {
                let mut hasher = Keccak256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function("sha256", |b| {
            b.iter(|| {
                let mut hasher = Sha256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function("blake3", |b| {
            b.iter(|| std::hint::black_box(blake3::hash(&data)));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_hash_by_size);
criterion_main!(benches);
