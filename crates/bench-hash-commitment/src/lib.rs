//! # bench-hash-commitment
//!
//! Benchmarks comparing hash algorithm performance and commitment strategies
//! relevant to Ethereum state root computation.
//!
//! ## Hash functions
//!
//! Compares raw throughput of three hash algorithms across input sizes (32B to 4KiB):
//! - **Keccak-256** — Ethereum's native hash, used in the Merkle Patricia Trie.
//! - **SHA-256** — widely used alternative with hardware acceleration (SHA-NI).
//! - **Blake3** — modern, parallelizable hash designed for speed.
//!
//! ## Commitment strategies
//!
//! Compares two approaches to committing a batch of state changes:
//! - **Authenticated (trie rehash)** — each changed entry rehashes `depth` nodes
//!   along its Merkle path (the current Ethereum approach).
//! - **Rolling hash** — serializes all changes and hashes once, producing a
//!   non-authenticated commitment (no per-entry proofs).
//!
//! ## Run
//!
//! ```bash
//! cargo bench -p bench-hash-commitment
//! ```
