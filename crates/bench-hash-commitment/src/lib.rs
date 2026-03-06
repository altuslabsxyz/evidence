//! # bench-hash-commitment
//!
//! Compares raw throughput of three hash algorithms across input sizes
//! (32B to 4KiB):
//!
//! - **Keccak-256** — Ethereum's native hash, no hardware acceleration.
//! - **SHA-256** — hardware-accelerated (SHA-NI on x86, crypto extensions on ARM).
//! - **Blake3** — modern, SIMD-optimized, parallelizable hash.
//!
//! ## Run
//!
//! ```bash
//! cargo bench -p bench-hash-commitment
//! ```
