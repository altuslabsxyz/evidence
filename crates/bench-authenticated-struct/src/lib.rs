//! # bench-authenticated-struct
//!
//! Performance comparison between an authenticated data structure (Merkle
//! Patricia Trie) and plain data structures (HashMap, BTreeMap).
//!
//! An authenticated data structure like the MPT produces a cryptographic root
//! hash that commits to all stored data, enabling Merkle proofs and stateless
//! verification. Maintaining this structure has a cost: every leaf insertion
//! walks a nibble path, potentially splits nodes, and marks the affected path
//! as dirty for later rehashing. A plain BTreeMap or HashMap simply writes the
//! value in place with no such overhead.
//!
//! In reth's implementation, `update_leaf` performs only structural changes
//! (node splitting, path walking, dirty marking in `prefix_set`). No keccak256
//! hashing occurs during insertion. The actual rehashing cascade runs once when
//! `root()` is called — typically at block end, after all updates are applied.
//! This means the `set` benchmark isolates pure trie structural overhead.
//!
//! For reads, `get_leaf_value` routes to the correct subtrie and performs a
//! HashMap lookup on the internal `values` map. It does not traverse trie nodes.
//! The `get` benchmark therefore compares flat key-value lookup performance.
//!
//! These benchmarks quantify the structural cost difference empirically.
//!
//! ## Benchmarks (`cargo bench -p bench-authenticated-struct`)
//!
//! - **get** — lookup 1,000 keys across 1K/10K/100K entry collections.
//!   Compares `HashMap::get`, `BTreeMap::get`, and `SparseTrie::get_leaf_value`.
//!
//! - **set** — insert 1,000 new keys into 1K/10K entry collections.
//!   Compares `HashMap::insert`, `BTreeMap::insert`, and `SparseTrie::update_leaf`.
