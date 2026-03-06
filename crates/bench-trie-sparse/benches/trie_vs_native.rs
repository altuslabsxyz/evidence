//! Benchmark comparing reth's Sparse Merkle Patricia Trie (MPT) get/set
//! performance against native Rust data structures (HashMap, BTreeMap).
//!
//! The sparse trie is the in-memory structure reth uses to compute state roots
//! incrementally. This benchmark quantifies the per-operation overhead of trie
//! traversal compared to flat key-value containers.
//!
//! ## Groups
//!
//! - **get/{1000,10000,100000}** — lookup 1,000 existing keys.
//!   HashMap (O(1) amortized) vs BTreeMap (O(log n)) vs SparseTrie (nibble-path traversal).
//!
//! - **set/{1000,10000}** — insert 1,000 new keys into a pre-populated collection.
//!   Measures allocation + structural mutation cost for each data structure.
//!
//! Run:
//!   cargo bench -p bench-trie-sparse

use alloy_primitives::B256;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use rand::Rng;
use reth_trie_common::{Nibbles, TrieNodeV2};
use reth_trie_sparse::{
    provider::DefaultTrieNodeProvider, ParallelSparseTrie, RevealableSparseTrie, SparseTrie,
};
use std::collections::{BTreeMap, HashMap};

/// Generate N random 32-byte keys and convert to 64-nibble paths.
fn generate_keys(n: usize) -> Vec<(B256, Nibbles)> {
    let mut rng = rand::thread_rng();
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let bytes: [u8; 32] = rng.random();
        let key = B256::from(bytes);
        let nibbles = Nibbles::unpack(key);
        keys.push((key, nibbles));
    }
    keys
}

/// Build a sparse trie pre-populated with `keys`.
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

fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("get");

    for &size in &[1_000, 10_000, 100_000] {
        let keys = generate_keys(size);
        let value = alloy_rlp::encode_fixed_size(&alloy_primitives::U256::from(42u64)).to_vec();

        // Build all data structures
        let mut hashmap: HashMap<B256, Vec<u8>> = HashMap::with_capacity(size);
        let mut btreemap: BTreeMap<B256, Vec<u8>> = BTreeMap::new();
        for (key, _) in &keys {
            hashmap.insert(*key, value.clone());
            btreemap.insert(*key, value.clone());
        }

        let sparse_trie = build_sparse_trie(&keys);
        let revealed = sparse_trie.as_revealed_ref().expect("revealed");

        // Pick 1000 random lookup keys from the existing set
        let lookup_count = 1000.min(size);
        let lookup_indices: Vec<usize> =
            (0..lookup_count).map(|i| i * size / lookup_count).collect();

        group.bench_function(BenchmarkId::new("HashMap", size), |b| {
            b.iter(|| {
                for &idx in &lookup_indices {
                    std::hint::black_box(hashmap.get(&keys[idx].0));
                }
            });
        });

        group.bench_function(BenchmarkId::new("BTreeMap", size), |b| {
            b.iter(|| {
                for &idx in &lookup_indices {
                    std::hint::black_box(btreemap.get(&keys[idx].0));
                }
            });
        });

        group.bench_function(BenchmarkId::new("SparseTrie", size), |b| {
            b.iter(|| {
                for &idx in &lookup_indices {
                    std::hint::black_box(revealed.get_leaf_value(&keys[idx].1));
                }
            });
        });
    }

    group.finish();
}

fn bench_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("set");

    for &size in &[1_000, 10_000] {
        let existing_keys = generate_keys(size);
        let new_keys = generate_keys(1000);
        let value = alloy_rlp::encode_fixed_size(&alloy_primitives::U256::from(99u64)).to_vec();

        group.bench_function(BenchmarkId::new("HashMap", size), |b| {
            b.iter_batched(
                || {
                    let mut map: HashMap<B256, Vec<u8>> = HashMap::with_capacity(size);
                    for (key, _) in &existing_keys {
                        map.insert(*key, value.clone());
                    }
                    map
                },
                |mut map| {
                    for (key, _) in &new_keys {
                        map.insert(*key, value.clone());
                    }
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("BTreeMap", size), |b| {
            b.iter_batched(
                || {
                    let mut map: BTreeMap<B256, Vec<u8>> = BTreeMap::new();
                    for (key, _) in &existing_keys {
                        map.insert(*key, value.clone());
                    }
                    map
                },
                |mut map| {
                    for (key, _) in &new_keys {
                        map.insert(*key, value.clone());
                    }
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("SparseTrie", size), |b| {
            b.iter_batched(
                || build_sparse_trie(&existing_keys),
                |mut trie| {
                    let revealed = trie.as_revealed_mut().expect("revealed");
                    let provider = DefaultTrieNodeProvider;
                    for (_, nibbles) in &new_keys {
                        revealed
                            .update_leaf(nibbles.clone(), value.clone(), &provider)
                            .expect("update_leaf");
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, bench_get, bench_set);
criterion_main!(benches);
