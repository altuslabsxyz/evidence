//! Criterion benchmarks for MDBX sharding strategies.
//!
//! Compares no-shard (single bitmap) vs sharded approaches across four operations:
//!
//! 1. **Append** — add 1 new block number to existing history.
//!    No-shard must deserialize/reserialize the entire bitmap; sharded only
//!    touches the last (sentinel) shard.
//!
//! 2. **Point query** — check if an address was modified at a specific block.
//!    No-shard reads and deserializes the full bitmap; sharded uses
//!    `cursor.set_range` to locate the single relevant shard.
//!
//! 3. **Full scan** — read all block numbers for one address.
//!    No-shard is a single read; sharded iterates and merges all shards.
//!
//! 4. **Unwind** — remove the last 100 block numbers (chain reorg simulation).
//!    No-shard rewrites the entire bitmap; sharded deletes/modifies only
//!    the affected tail shards.
//!
//! Run:
//!   cargo bench -p bench-mdbx-shard
//!
//! HTML reports are generated under `target/criterion/`.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use bench_mdbx_shard::*;
use tempfile::TempDir;

/// Shard sizes to compare. 0 means no sharding (single bitmap). reth uses 2,000.
const SHARD_SIZES: &[usize] = &[0, 100, 500, 1_000, 2_000, 5_000, 10_000, 50_000];

/// History sizes to test.
const HISTORY_SIZES: &[(u64, &str)] = &[
    (10_000, "10K"),
    (100_000, "100K"),
    (500_000, "500K"),
    (1_000_000, "1M"),
];

const AVG_GAP: u64 = 10;

fn shard_label(ss: usize) -> String {
    if ss == 0 {
        "noshard".to_string()
    } else {
        format!("s={}", ss)
    }
}

/// Helper: populate a DB with the given blocks and shard size.
fn populate(
    env: &reth_libmdbx::Environment,
    addr: &[u8; 20],
    blocks: &roaring::RoaringTreemap,
    shard_size: usize,
) {
    if shard_size == 0 {
        noshard_store(env, "db", addr, blocks);
    } else {
        sharded_bulk_load(env, "db", addr, blocks, shard_size);
    }
}

// ---------------------------------------------------------------------------
// 1. Append: add 1 new block number to existing history
// ---------------------------------------------------------------------------

fn bench_append(c: &mut Criterion) {
    for &(count, hist_label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);
        let max_block = blocks.iter().next_back().unwrap();

        let mut group = c.benchmark_group(format!("append/{}", hist_label));
        if count >= 500_000 {
            group.sample_size(20);
        } else {
            group.sample_size(50);
        }

        for &shard_size in SHARD_SIZES {
            let blocks = blocks.clone();
            group.bench_function(&shard_label(shard_size), |b| {
                let dir = TempDir::new().unwrap();
                let env = create_env(dir.path());
                let addr = random_address();
                populate(&env, &addr, &blocks, shard_size);

                let counter = std::cell::Cell::new(max_block + 1);

                b.iter(|| {
                    let next = counter.get();
                    counter.set(next + 1);
                    if shard_size == 0 {
                        noshard_append(&env, "db", &addr, std::iter::once(next));
                    } else {
                        sharded_append(&env, "db", &addr, std::iter::once(next), shard_size);
                    }
                });
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// 2. Point query: find which shard contains a specific block number
// ---------------------------------------------------------------------------

fn bench_point_query(c: &mut Criterion) {
    for &(count, hist_label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);
        let all: Vec<u64> = blocks.iter().collect();
        let target = all[all.len() / 2]; // middle of the history

        let mut group = c.benchmark_group(format!("point_query/{}", hist_label));

        for &shard_size in SHARD_SIZES {
            let blocks = blocks.clone();
            group.bench_function(&shard_label(shard_size), |b| {
                let dir = TempDir::new().unwrap();
                let env = create_env(dir.path());
                let addr = random_address();
                populate(&env, &addr, &blocks, shard_size);

                b.iter(|| {
                    if shard_size == 0 {
                        let bm = noshard_read(&env, "db", &addr);
                        std::hint::black_box(bm.contains(target));
                    } else {
                        std::hint::black_box(sharded_point_query(
                            &env, "db", &addr, target,
                        ));
                    }
                });
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// 3. Full scan: read all block numbers for one address
// ---------------------------------------------------------------------------

fn bench_full_scan(c: &mut Criterion) {
    for &(count, hist_label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);

        let mut group = c.benchmark_group(format!("full_scan/{}", hist_label));
        if count >= 500_000 {
            group.sample_size(30);
        }

        for &shard_size in SHARD_SIZES {
            let blocks = blocks.clone();
            group.bench_function(&shard_label(shard_size), |b| {
                let dir = TempDir::new().unwrap();
                let env = create_env(dir.path());
                let addr = random_address();
                populate(&env, &addr, &blocks, shard_size);

                b.iter(|| {
                    if shard_size == 0 {
                        std::hint::black_box(noshard_read(&env, "db", &addr));
                    } else {
                        std::hint::black_box(sharded_full_scan(&env, "db", &addr));
                    }
                });
            });
        }
        group.finish();
    }
}

// ---------------------------------------------------------------------------
// 4. Unwind: remove last 100 block numbers (reorg simulation)
// ---------------------------------------------------------------------------

fn bench_unwind(c: &mut Criterion) {
    for &(count, hist_label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);
        let all: Vec<u64> = blocks.iter().collect();
        let unwind_to = all[all.len().saturating_sub(100)];

        let mut group = c.benchmark_group(format!("unwind/{}", hist_label));
        group.sample_size(20);

        for &shard_size in SHARD_SIZES {
            let blocks = blocks.clone();
            group.bench_function(&shard_label(shard_size), |b| {
                let dir = TempDir::new().unwrap();
                let env = create_env(dir.path());
                let addr = random_address();

                b.iter_batched(
                    || {
                        // Setup: populate fresh data before each measurement
                        populate(&env, &addr, &blocks, shard_size);
                    },
                    |()| {
                        // Measured: unwind
                        if shard_size == 0 {
                            noshard_unwind(&env, "db", &addr, unwind_to);
                        } else {
                            sharded_unwind(&env, "db", &addr, unwind_to);
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
        }
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_append,
    bench_point_query,
    bench_full_scan,
    bench_unwind,
);
criterion_main!(benches);
