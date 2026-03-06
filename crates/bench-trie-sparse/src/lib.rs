//! # bench-trie-sparse
//!
//! Benchmarks comparing reth's Sparse Merkle Patricia Trie (MPT) against native
//! Rust data structures (HashMap, BTreeMap).
//!
//! The sparse trie is the in-memory structure reth uses to compute state roots
//! incrementally. These benchmarks quantify the overhead of trie operations
//! relative to flat key-value lookups.
//!
//! ## Benchmarks (`cargo bench -p bench-trie-sparse`)
//!
//! - **get** — lookup 1,000 keys across 1K/10K/100K entry collections.
//!   Compares `HashMap::get`, `BTreeMap::get`, and `SparseTrie::get_leaf_value`.
//!
//! - **set** — insert 1,000 new keys into 1K/10K entry collections.
//!   Compares `HashMap::insert`, `BTreeMap::insert`, and `SparseTrie::update_leaf`.
