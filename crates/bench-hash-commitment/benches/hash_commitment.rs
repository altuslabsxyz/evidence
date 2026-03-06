//! Benchmark: hash function throughput comparison.
//!
//! Compares raw throughput of keccak256, SHA-256, and Blake3 across input sizes.
//! This is a pure hash function benchmark — no data structure or commitment
//! strategy overhead is involved.
//!
//! Input sizes:
//! - 32 bytes — single trie node hash (two 16-byte child hashes)
//! - 64 bytes — typical storage slot (key + value)
//! - 256, 1024, 4096 bytes — serialized state of increasing size
//!
//! Run:
//!   cargo bench -p bench-hash-commitment

use alloy_primitives::Keccak256;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sha2::{Digest as _, Sha256};

fn bench_hash_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_function");

    for &size in &[32, 64, 256, 1024, 4096] {
        let data = vec![0xABu8; size];
        let label = format!("{size}B");

        group.bench_function(BenchmarkId::new("keccak256", &label), |b| {
            b.iter(|| {
                let mut hasher = Keccak256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function(BenchmarkId::new("sha256", &label), |b| {
            b.iter(|| {
                let mut hasher = Sha256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function(BenchmarkId::new("blake3", &label), |b| {
            b.iter(|| std::hint::black_box(blake3::hash(&data)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_hash_functions);
criterion_main!(benches);
