# evidence

Reproducible benchmarks that back engineering decisions with data — covering databases, data structures, hash functions, and blockchain infrastructure.

Each benchmark topic lives in its own crate under `crates/`.

## Project Structure

```
crates/
├── bench-mdbx-shard/         MDBX sharding strategy benchmarks
│   ├── src/lib.rs             Shared helpers (env setup, shard/noshard operations, stats)
│   ├── benches/               Criterion benchmarks (append, point query, full scan, unwind)
│   └── tests/                 Data analysis tests (storage sizes, compression ratios)
│
├── bench-authenticated-struct/         Authenticated vs plain data structure benchmarks
│   ├── src/lib.rs              (crate root)
│   └── benches/                Criterion benchmark: authenticated_vs_plain (get, set)
│
└── bench-hash-commitment/     Hash function & commitment strategy benchmarks
    ├── src/lib.rs              (crate root)
    └── benches/                Criterion benchmarks (hash_function, commitment_strategy)
```

## Running Benchmarks & Viewing Reports

All crates use [Criterion.rs](https://bheisler.github.io/criterion.rs/book/) for
statistical benchmarking. Every `cargo bench` run automatically generates HTML
reports with violin plots, histograms, and regression analysis.

```bash
# Run all benchmarks across the entire workspace
cargo bench

# Run benchmarks for a specific crate
cargo bench -p bench-mdbx-shard

# Run a specific benchmark group within a crate
cargo bench -p bench-mdbx-shard -- append
```

Reports are generated under `target/criterion/` with the following structure:

```
target/criterion/
├── report/index.html                        # top-level overview of all groups
├── hash_function/report/index.html          # per-group report with violin plots
├── commitment_strategy/report/index.html
├── append_10K/report/index.html
│   ├── noshard/report/index.html            # per-function detail
│   ├── s=2000/report/index.html
│   └── ...
└── ...
```

```bash
# Open a specific group report (macOS)
open target/criterion/hash_function/report/index.html

# Or browse the top-level overview
open target/criterion/report/index.html
```

From the second run onward, Criterion automatically compares against the
previous baseline and highlights regressions or improvements (e.g., `+2.3%`,
`-5.1%`). Reports persist across runs until manually deleted.

---

## bench-mdbx-shard

Benchmarks MDBX sharding strategies for reth's `AccountsHistory` table.
Compares **no-shard** (all block numbers in one key-value pair) vs **sharded**
(block numbers split into fixed-size chunks, reth default: 2,000).

Values are compressed with RoaringTreemap, matching production reth behavior.

**Shard sizes tested:** noshard, 100, 500, 1K, 2K, 5K, 10K, 50K
**History sizes tested:** 10K, 100K, 500K, 1M block numbers

### Criterion benchmarks

```bash
# Run all benchmarks (generates HTML reports under target/criterion/)
cargo bench -p bench-mdbx-shard

# Run a specific benchmark group
cargo bench -p bench-mdbx-shard -- append
cargo bench -p bench-mdbx-shard -- point_query
cargo bench -p bench-mdbx-shard -- full_scan
cargo bench -p bench-mdbx-shard -- unwind
```

| Group | What it measures |
|-------|-----------------|
| `append/{10K,100K,500K,1M}` | Append 1 new block number to existing history |
| `point_query/{10K,100K,500K,1M}` | Check if an address was modified at a specific block |
| `full_scan/{10K,100K,500K,1M}` | Read all block numbers for one address |
| `unwind/{10K,100K,500K,1M}` | Remove last 100 block numbers (chain reorg simulation) |

### Data analysis tests

```bash
# Run with --nocapture to see printed tables
cargo test -p bench-mdbx-shard -- --nocapture
```

| Test | What it reports |
|------|----------------|
| `storage_value_sizes_and_page_stats` | Compressed value sizes per shard size; MDBX B-tree page layout (depth, leaf/branch/overflow pages) for 1M blocks |
| `compression_ratio` | RoaringTreemap compressed size vs raw u64 array across history sizes |

### Insights: What to learn from each benchmark

#### Append

Measures the cost of appending a single new block number to an address's history,
which is the hot path during block execution in reth.

- **No-shard** must read-modify-write the entire bitmap every time. As history
  grows (100K+ blocks), deserialization and reserialization of the full
  RoaringTreemap dominates. Expect append latency to grow roughly linearly
  with history size.
- **Sharded** only touches the last (sentinel) shard, which contains at most
  `shard_size` entries. Append time stays nearly constant regardless of total
  history length — this is the primary motivation for sharding.
- **Smaller shards** (100, 500) keep append fast but create more MDBX entries.
  **Larger shards** (10K, 50K) converge toward no-shard behavior for append.
- The reth default of **2,000** is a sweet spot: append is fast (small sentinel)
  without excessive entry count.

#### Point query

Measures how quickly we can answer "was this address modified at block N?",
which is the core operation for historical state lookups.

- **No-shard** must deserialize the entire bitmap to call `.contains()`.
  With 1M blocks this involves deserializing ~250 KiB of data per query.
- **Sharded** uses MDBX's B-tree cursor (`set_range`) to jump directly to the
  one shard that could contain the target block, then deserializes only that
  shard. This is an O(log n) seek in the B-tree + O(1) bitmap lookup.
- Expect sharded point queries to be **orders of magnitude faster** than
  no-shard at large history sizes.
- Among shard sizes, smaller shards mean smaller per-shard bitmaps (faster
  deserialization) but the difference is modest since `set_range` is already
  efficient.

#### Full scan

Measures reading all block numbers for an address — needed for operations like
computing the full transaction history of an account.

- **No-shard** wins here: it's a single MDBX read + one deserialization.
- **Sharded** must iterate all shards with a cursor, deserialize each bitmap,
  and merge them. More shards (smaller `shard_size`) = more iterations = slower.
- This is the **tradeoff of sharding**: append and point query get faster at
  the expense of full scan.
- With shard sizes >= 2,000 the overhead is reasonable. Very small shards
  (100) can make full scan significantly slower due to the large number of
  MDBX entries to iterate.

#### Unwind

Simulates a chain reorganization where the last 100 blocks are rolled back.
This measures how efficiently we can remove recent history.

- **No-shard** reads the entire bitmap, filters out blocks >= unwind point,
  reserializes, and rewrites. Cost scales with total history size.
- **Sharded** only needs to delete or modify the last few shards. With
  `shard_size=2000` and 100 blocks to unwind, typically only the sentinel
  shard is affected.
- Expect sharded unwind to be **much faster and more consistent** than
  no-shard, especially at large history sizes.
- Very small shards (100) may require deleting multiple shard entries during
  unwind, adding some overhead.

#### Storage analysis (test)

Reports compressed value sizes and MDBX page statistics.

- Shows how RoaringTreemap compression ratio changes with shard size. Larger
  bitmaps compress better (more runs, better delta encoding), so no-shard has
  the best per-entry compression but the worst operational characteristics.
- MDBX page stats reveal the B-tree structure: **overflow pages** appear when
  a single value exceeds the page size (16 KiB on Apple Silicon). No-shard
  with large histories will have overflow pages; sharded avoids them by
  keeping values small.
- **B-tree depth** stays shallow (2-3) for sharded approaches, ensuring
  consistent seek performance.

#### Compression ratio (test)

Shows the space efficiency of RoaringTreemap compared to raw `u64` arrays.

- With sparse block numbers (avg gap ~10), expect compression ratios of
  **3-5x** — meaning a 1M-entry history that would be ~8 MiB as raw u64
  values compresses to ~1.5-2.5 MiB.
- Per-entry cost decreases as history grows (RoaringTreemap amortizes
  container overhead over more entries).
- This confirms that RoaringTreemap is a good choice for reth's IntegerList:
  reasonable compression without expensive decompression.

### Summary: Sharding tradeoff matrix

| Operation | No-shard | Small shard (100-500) | Medium shard (1K-5K) | Large shard (10K-50K) |
|-----------|----------|-----------------------|----------------------|-----------------------|
| Append | Slow (scales with history) | Fast (constant) | Fast (constant) | Moderate |
| Point query | Slow (full deserialize) | Fast (tiny shard) | Fast | Fast |
| Full scan | Fastest (1 read) | Slow (many entries) | Good balance | Good |
| Unwind | Slow (full rewrite) | Fast (few entries) | Fast | Moderate |
| Storage overhead | Minimal (1 entry) | High (many entries) | Moderate | Low |
| MDBX overflow pages | Likely at large history | None | None | Possible |

**Conclusion:** reth's default shard size of 2,000 provides a well-balanced
tradeoff — fast append and point query with acceptable full scan performance
and minimal storage overhead.

---

## bench-authenticated-struct

Measures the structural overhead of an authenticated data structure (Merkle Patricia Trie) compared to plain data structures (HashMap, BTreeMap).

An authenticated data structure like the MPT produces a cryptographic root hash that commits to every value it contains, enabling Merkle proofs and stateless verification. Maintaining this trie structure has a cost: every leaf insertion walks a nibble path, potentially splits nodes, and marks the affected path as dirty for later rehashing. Plain structures simply store the value in place with no such overhead.

An important detail of reth's implementation: `update_leaf` performs only structural changes — node splitting, path walking, and dirty marking in the `prefix_set`. No keccak256 hashing occurs during insertion. The actual rehashing cascade runs once when `root()` is called, typically at block end after all updates are applied. This means the `set` benchmark isolates pure trie structural overhead, separate from cryptographic cost (which is covered by the `bench-hash-commitment` crate).

For reads, `get_leaf_value` does not traverse trie nodes. Internally it routes to the correct subtrie and performs a HashMap lookup on the `values` map. The `get` benchmark therefore compares flat key-value lookup performance across all three structures.

```bash
# Run all benchmarks
cargo bench -p bench-authenticated-struct
```

| Group | What it measures |
|-------|-----------------|
| `get/{1000,10000,100000}` | Lookup 1,000 existing keys: HashMap O(1) vs BTreeMap O(log n) vs SparseTrie (internal HashMap lookup with subtrie routing) |
| `set/{1000,10000}` | Insert 1,000 new keys into a pre-populated collection — where the trie's structural overhead is most visible |

### Results (Apple Silicon, 1,000 key operations)

All structures use the same hasher (foldhash via `alloy_primitives::map::HashMap`) to ensure a fair comparison of pure data structure overhead.

#### Get — lookup 1,000 keys

| Collection size | HashMap | BTreeMap | SparseTrie | Trie vs HashMap |
|-----------------|---------|----------|------------|-----------------|
| 1K entries | 4.6 µs | 11.5 µs | 10.1 µs | 2.2x slower |
| 10K entries | 6.1 µs | 19.0 µs | 13.4 µs | 2.2x slower |
| 100K entries | 9.3 µs | 32.4 µs | 13.8 µs | 1.5x slower |

SparseTrie reads are slower than HashMap but faster than BTreeMap. This is because reth's SparseTrie does not traverse trie nodes on read — it maintains a separate internal HashMap (`values`) for fast value access, and `get_leaf_value` simply routes to the correct subtrie and performs a HashMap lookup. The trie node structure exists solely for `root()` hash computation. At 100K entries the gap with HashMap narrows because the trie distributes values across smaller subtrie HashMaps, improving cache locality.

#### Set — insert 1,000 keys

| Pre-populated size | HashMap | BTreeMap | SparseTrie | Trie vs HashMap |
|--------------------|---------|----------|------------|-----------------|
| 1K entries | 59 µs | 65 µs | 273 µs | 4.6x slower |
| 10K entries | 120 µs | 215 µs | 558 µs | 4.7x slower |

Writes are where the trie's structural overhead becomes visible. Each insert walks the nibble path, potentially splits nodes, and marks dirty paths — even without any hashing (which is deferred to `root()`). The full authentication cost including the keccak256 rehashing cascade is additive on top of this; see `bench-hash-commitment` for that dimension.

---

## bench-hash-commitment

Benchmarks comparing hash algorithm throughput and commitment strategies relevant
to Ethereum state root computation.

```bash
cargo bench -p bench-hash-commitment
```

| Group | What it measures |
|-------|-----------------|
| `hash_function/keccak256/{32B,256B,1024B,4096B}` | Keccak-256 throughput across input sizes |
| `hash_function/sha256/{32B,256B,1024B,4096B}` | SHA-256 throughput across input sizes |
| `hash_function/blake3/{32B,256B,1024B,4096B}` | Blake3 throughput across input sizes |
| `commitment_strategy/authenticated_depth8_keccak/{100,500,1K,5K}` | Simulated Merkle trie rehash: each entry triggers 8 sequential keccak256 ops along its dirty path |
| `commitment_strategy/rolling_keccak/{100,500,1K,5K}` | Rolling hash: stream all entries into one keccak256 invocation |
| `commitment_strategy/rolling_sha256/{100,500,1K,5K}` | Rolling hash with SHA-256 |
| `commitment_strategy/rolling_blake3/{100,500,1K,5K}` | Rolling hash with Blake3 |

### Insights: What to learn from each benchmark

#### Hash function throughput

Measures raw speed of each hash algorithm at various input sizes, independent of
any trie or commitment structure.

- **Keccak-256** is Ethereum's native hash. It is a sponge-based construction
  that processes data in 136-byte blocks. It has no hardware acceleration on
  most platforms, making it the slowest of the three at larger inputs.
- **SHA-256** benefits from dedicated CPU instructions (SHA-NI on x86, hardware
  crypto extensions on ARM). On Apple Silicon, expect SHA-256 to match or beat
  keccak256, especially at larger input sizes where the hardware pipeline
  stays saturated.
- **Blake3** is designed for parallelism (internal SIMD and tree hashing). For
  small inputs (32B) the setup overhead may dominate, but at 1 KiB+ it
  typically outperforms both keccak and SHA-256 significantly.
- The 32-byte case is particularly important: it represents hashing a single
  trie node (combining two 16-byte child hashes), which is the dominant
  operation during state root computation.

#### Commitment strategies

Compares two fundamentally different approaches to committing a batch of state
changes, answering: "How expensive is authenticated commitment compared to a
simple rolling hash?"

- **Authenticated (trie rehash)** simulates what Ethereum actually does: for
  each changed entry, rehash all nodes along its Merkle path (depth=8 is
  realistic for the Ethereum state trie). This means `N * depth` sequential
  hash operations. The cost scales linearly with both the number of changes
  and the trie depth.
- **Rolling hash** simply streams the previous commitment and all changed
  entries into a single hash invocation. This produces a commitment (useful
  for detecting changes) but does not support per-entry Merkle proofs.
- Expect rolling hash to be **orders of magnitude faster** than authenticated
  commitment — the gap widens with more entries because rolling hash is O(N)
  total hashing while authenticated is O(N * depth).
- Among rolling hash variants, Blake3 should be fastest, followed by SHA-256
  (with hardware support), then keccak256.
- This quantifies the **cost of Merkle proofs**: the performance gap between
  authenticated and rolling commitment is exactly the price Ethereum pays for
  supporting stateless verification and light clients.

### Summary

| | Keccak-256 | SHA-256 | Blake3 |
|---|---|---|---|
| Small input (32B) | Baseline | Comparable | Comparable (setup overhead) |
| Large input (4 KiB) | Slowest | Fast (HW accel) | Fastest (SIMD) |
| Ethereum compatibility | Native | Not compatible | Not compatible |

The key takeaway is that Ethereum's choice of keccak256 is the most expensive
option from a pure throughput perspective, and the authenticated trie commitment
model multiplies that cost by the trie depth for every changed entry. This
motivates research into alternative state commitment schemes (e.g., Verkle
trees with Pedersen commitments) that reduce the per-entry proof cost.

---

## Adding a New Benchmark Crate

1. Create `crates/bench-<topic>/` with `Cargo.toml`, `src/lib.rs`, and `benches/`.
2. Add the path to the root `Cargo.toml` workspace members:

```toml
[workspace]
members = [
    "crates/bench-mdbx-shard",
    "crates/bench-authenticated-struct",
    "crates/bench-hash-commitment",
    "crates/bench-<topic>",
]
```

3. Run `cargo bench -p bench-<topic>` to execute.
