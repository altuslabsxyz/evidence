//! Benchmark: authenticated vs plain data structure performance.
//!
//! Compares the Merkle Patricia Trie (MPT) — an authenticated data structure
//! whose root hash cryptographically commits to all stored values — against
//! plain HashMap and BTreeMap, which provide no such commitment.
//!
//! The core question is how much structural overhead the trie imposes on basic
//! operations. In an MPT, every leaf insertion walks a nibble path, potentially
//! splits existing nodes (leaf → branch + leaves, or extending an extension
//! node), and marks dirty paths for later rehashing. Plain structures simply
//! store the value with no cascading work.
//!
//! Note on what is (and is not) measured:
//!
//! - `update_leaf` in reth's sparse trie does NOT perform hashing. It only
//!   modifies the trie structure and marks dirty paths in the `prefix_set`.
//!   Actual keccak256 rehashing is deferred to the `root()` call, which runs
//!   once after all updates (typically at block end). The `set` benchmark
//!   therefore measures pure structural overhead — node splitting, path
//!   walking, and dirty marking — not cryptographic cost.
//!
//! - `get_leaf_value` does not traverse trie nodes. Internally it routes to
//!   the correct subtrie and performs a HashMap lookup on the `values` map.
//!   The `get` benchmark compares flat key-value lookup performance across
//!   all three structures.
//!
//! ## Groups
//!
//! - **get/{1000,10000,100000}** — lookup 1,000 existing keys.
//!   HashMap (O(1) amortized) vs BTreeMap (O(log n)) vs SparseTrie (internal
//!   HashMap lookup with subtrie routing).
//!
//! - **set/{1000,10000}** — insert 1,000 new keys into a pre-populated collection.
//!   This is where the structural cost of the trie is most visible: SparseTrie
//!   must walk the nibble path, potentially split nodes, and mark dirty paths,
//!   while HashMap/BTreeMap simply allocate and insert.
//!
//! Run:
//!   cargo bench -p bench-authenticated-struct

use alloy_primitives::{map::HashMap, B256};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use rand::Rng as _;
use reth_trie_common::{Nibbles, TrieNodeV2};
use reth_trie_sparse::{
    provider::DefaultTrieNodeProvider, ParallelSparseTrie, RevealableSparseTrie, SparseTrie,
};
use std::collections::BTreeMap;

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

/// Build an authenticated data structure (sparse MPT) pre-populated with `keys`.
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

        // Build plain structures (HashMap, BTreeMap)
        let mut hashmap: HashMap<B256, Vec<u8>> = HashMap::default();
        let mut btreemap: BTreeMap<B256, Vec<u8>> = BTreeMap::new();
        for (key, _) in &keys {
            hashmap.insert(*key, value.clone());
            btreemap.insert(*key, value.clone());
        }

        // Build authenticated structure (MPT)
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
                    let mut map: HashMap<B256, Vec<u8>> = HashMap::default();
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
