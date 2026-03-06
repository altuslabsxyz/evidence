//! Benchmark comparing hash algorithms and commitment strategies.
//!
//! ## Groups
//!
//! ### hash_function
//!
//! Raw throughput of keccak256, SHA-256, and Blake3 across input sizes:
//! - 32 bytes (single trie node hash)
//! - 256, 1024, 4096 bytes (serialized state of increasing size)
//!
//! ### commitment_strategy
//!
//! Compares two ways to commit a batch of N state changes (100, 500, 1K, 5K):
//! - **authenticated_depth8_keccak** — simulates Merkle trie rehash: each entry
//!   requires `depth` (8) sequential keccak256 operations along its dirty path.
//! - **rolling_keccak / rolling_sha256 / rolling_blake3** — streams all entries
//!   into a single hash invocation (no per-entry proof, just a commitment).
//!
//! Run:
//!   cargo bench -p bench-hash-commitment

use alloy_primitives::{B256, Keccak256};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sha2::{Digest as _, Sha256};

/// Simulate hashing N changed entries through an authenticated trie path.
/// Each entry requires `depth` hash operations (dirty path rehashing).
fn authenticated_commitment(entries: &[Vec<u8>], depth: usize) {
    for entry in entries {
        let mut current = Keccak256::new();
        current.update(entry);
        let mut hash = current.finalize();
        for _ in 1..depth {
            let mut hasher = Keccak256::new();
            hasher.update(hash.as_slice());
            hash = hasher.finalize();
        }
        std::hint::black_box(hash);
    }
}

/// Rolling hash with keccak256: serialize all changes, hash once.
fn rolling_hash_keccak(prev_commitment: &B256, entries: &[Vec<u8>]) -> B256 {
    let mut hasher = Keccak256::new();
    hasher.update(prev_commitment.as_slice());
    for entry in entries {
        hasher.update(entry);
    }
    hasher.finalize()
}

/// Rolling hash with SHA-256.
fn rolling_hash_sha256(prev_commitment: &[u8; 32], entries: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prev_commitment);
    for entry in entries {
        hasher.update(entry);
    }
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Rolling hash with Blake3.
fn rolling_hash_blake3(prev_commitment: &[u8; 32], entries: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev_commitment);
    for entry in entries {
        hasher.update(entry);
    }
    *hasher.finalize().as_bytes()
}

fn bench_hash_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_function");

    // Hash a single 32-byte input (simulates one node hash)
    let input = [0x42u8; 32];

    group.bench_function("keccak256/32B", |b| {
        b.iter(|| {
            let mut hasher = Keccak256::new();
            hasher.update(&input);
            std::hint::black_box(hasher.finalize())
        });
    });

    group.bench_function("sha256/32B", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(&input);
            std::hint::black_box(hasher.finalize())
        });
    });

    group.bench_function("blake3/32B", |b| {
        b.iter(|| {
            let hash = blake3::hash(&input);
            std::hint::black_box(hash)
        });
    });

    // Larger inputs (simulates hashing serialized state)
    for &size in &[256, 1024, 4096] {
        let data = vec![0xABu8; size];

        group.bench_function(BenchmarkId::new("keccak256", format!("{size}B")), |b| {
            b.iter(|| {
                let mut hasher = Keccak256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function(BenchmarkId::new("sha256", format!("{size}B")), |b| {
            b.iter(|| {
                let mut hasher = Sha256::new();
                hasher.update(&data);
                std::hint::black_box(hasher.finalize())
            });
        });

        group.bench_function(BenchmarkId::new("blake3", format!("{size}B")), |b| {
            b.iter(|| std::hint::black_box(blake3::hash(&data)));
        });
    }

    group.finish();
}

fn bench_commitment_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("commitment_strategy");

    // Simulate N changed entries, each ~64 bytes (typical storage slot RLP)
    for &num_changes in &[100, 500, 1000, 5000] {
        let entries: Vec<Vec<u8>> = (0..num_changes)
            .map(|i| {
                let mut v = vec![0u8; 64];
                v[0..8].copy_from_slice(&(i as u64).to_le_bytes());
                v
            })
            .collect();

        let prev_commitment = B256::ZERO;
        let prev_bytes = [0u8; 32];

        // Authenticated: each entry rehashes `depth` nodes (keccak256)
        group.bench_function(
            BenchmarkId::new("authenticated_depth8_keccak", num_changes),
            |b| {
                b.iter(|| authenticated_commitment(&entries, 8));
            },
        );

        // Rolling hash with keccak256
        group.bench_function(BenchmarkId::new("rolling_keccak", num_changes), |b| {
            b.iter(|| std::hint::black_box(rolling_hash_keccak(&prev_commitment, &entries)));
        });

        // Rolling hash with SHA-256
        group.bench_function(BenchmarkId::new("rolling_sha256", num_changes), |b| {
            b.iter(|| std::hint::black_box(rolling_hash_sha256(&prev_bytes, &entries)));
        });

        // Rolling hash with Blake3
        group.bench_function(BenchmarkId::new("rolling_blake3", num_changes), |b| {
            b.iter(|| std::hint::black_box(rolling_hash_blake3(&prev_bytes, &entries)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_hash_functions, bench_commitment_strategies);
criterion_main!(benches);
