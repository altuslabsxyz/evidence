//! Benchmark: authenticated commitment vs rolling hash commitment.
//!
//! ## commitment (single batch)
//!
//! Compares the cost of computing a state commitment over N dirty entries:
//!
//! - **SparseTrie `root()`**: keccak256 Merkle rehash over all dirty paths.
//!   Cost: O(N × depth) keccak256 operations.
//!
//! - **blake3**: Stream N entries into a single blake3 invocation.
//!   Cost: O(N) with a single hash context.
//!
//! Insertion cost is excluded — only the commitment computation is measured.
//!
//! ## sequential_blocks (3 consecutive blocks)
//!
//! Simulates 3 consecutive blocks, each inserting K entries and computing
//! a commitment. Measures the full cycle (insert + commit) per block.
//!
//! - **SparseTrie**: `update_leaf(K)` + `root()` per block. `root()` is
//!   incremental — only rehashes the K paths dirtied in that block.
//!
//! - **HashMap + rolling blake3**: `insert(K)` + `blake3(prev_commitment || delta)`
//!   per block. The commitment chains: each block's hash includes the previous
//!   block's commitment, so only the delta is hashed — O(K) per block regardless
//!   of total state size.
//!
//! Run:
//!   cargo bench -p bench-authenticated-struct -- commitment
//!   cargo bench -p bench-authenticated-struct -- sequential_blocks

use alloy_primitives::{map::HashMap, B256};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use rand::Rng as _;
use reth_trie_common::{Nibbles, TrieNodeV2};
use reth_trie_sparse::{
    provider::DefaultTrieNodeProvider, ParallelSparseTrie, RevealableSparseTrie, SparseTrie,
};

/// Generate N random 32-byte keys and their nibble-path representations.
fn generate_keys(n: usize) -> Vec<(B256, Nibbles)> {
    let mut rng = rand::thread_rng();
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let bytes: [u8; 32] = rng.gen();
        let key = B256::from(bytes);
        let nibbles = Nibbles::unpack(key);
        keys.push((key, nibbles));
    }
    keys
}

/// Build a sparse MPT pre-populated with `keys`. All entries are dirty
/// (never had `root()` called), so the next `root()` call will rehash
/// every path.
fn build_sparse_trie(keys: &[(B256, Nibbles)]) -> RevealableSparseTrie<ParallelSparseTrie> {
    let mut trie = RevealableSparseTrie::<ParallelSparseTrie>::default();
    let revealed = trie.reveal_root(TrieNodeV2::EmptyRoot, None, false).expect("reveal root");

    let value = alloy_rlp::encode_fixed_size(&alloy_primitives::U256::from(42u64)).to_vec();
    let provider = DefaultTrieNodeProvider;

    for (_key, nibbles) in keys {
        revealed.update_leaf(nibbles.clone(), value.clone(), &provider).expect("update_leaf");
    }

    trie
}

fn bench_commitment(c: &mut Criterion) {
    let mut group = c.benchmark_group("commitment");

    for &size in &[100, 1_000, 5_000, 10_000, 50_000, 100_000] {
        let keys = generate_keys(size);
        let value = alloy_rlp::encode_fixed_size(&alloy_primitives::U256::from(42u64)).to_vec();

        // Authenticated: SparseTrie root() — keccak256 Merkle rehash
        group.bench_function(BenchmarkId::new("sparse_trie_root", size), |b| {
            b.iter_batched(
                || build_sparse_trie(&keys),
                |mut trie| {
                    let revealed = trie.as_revealed_mut().expect("revealed");
                    std::hint::black_box(revealed.root());
                },
                BatchSize::SmallInput,
            );
        });

        // Unauthenticated: blake3 over all entries
        group.bench_function(BenchmarkId::new("blake3", size), |b| {
            b.iter(|| {
                let mut hasher = blake3::Hasher::new();
                for (key, _) in &keys {
                    hasher.update(key.as_ref());
                    hasher.update(value.as_ref());
                }
                std::hint::black_box(hasher.finalize())
            });
        });
    }

    group.finish();
}

fn bench_sequential_blocks(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_blocks");

    for &entries_per_block in &[100, 500, 1_000, 5_000, 10_000, 50_000, 100_000] {
        let block1_keys = generate_keys(entries_per_block);
        let block2_keys = generate_keys(entries_per_block);
        let block3_keys = generate_keys(entries_per_block);
        let value = alloy_rlp::encode_fixed_size(&alloy_primitives::U256::from(42u64)).to_vec();

        // SparseTrie: insert + root() per block (incremental rehash)
        group.bench_function(
            BenchmarkId::new("sparse_trie", entries_per_block),
            |b| {
                b.iter_batched(
                    || {
                        let mut trie =
                            RevealableSparseTrie::<ParallelSparseTrie>::default();
                        trie.reveal_root(TrieNodeV2::EmptyRoot, None, false)
                            .expect("reveal root");
                        trie
                    },
                    |mut trie| {
                        let provider = DefaultTrieNodeProvider;

                        for block_keys in [&block1_keys, &block2_keys, &block3_keys] {
                            let revealed = trie.as_revealed_mut().expect("revealed");
                            for (_, nibbles) in block_keys {
                                revealed
                                    .update_leaf(nibbles.clone(), value.clone(), &provider)
                                    .expect("update_leaf");
                            }
                            std::hint::black_box(revealed.root());
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        // HashMap + rolling blake3: insert + blake3(prev || delta) per block
        group.bench_function(
            BenchmarkId::new("hashmap_rolling_blake3", entries_per_block),
            |b| {
                b.iter_batched(
                    || HashMap::<B256, Vec<u8>>::default(),
                    |mut map| {
                        let mut commitment = [0u8; 32];

                        for block_keys in [&block1_keys, &block2_keys, &block3_keys] {
                            let mut hasher = blake3::Hasher::new();
                            hasher.update(&commitment);
                            for (key, _) in block_keys.iter() {
                                map.insert(*key, value.clone());
                                hasher.update(key.as_ref());
                                hasher.update(value.as_ref());
                            }
                            commitment = *hasher.finalize().as_bytes();
                        }

                        std::hint::black_box(commitment)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_commitment, bench_sequential_blocks);
criterion_main!(benches);
