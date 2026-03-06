//! # bench-mdbx-shard
//!
//! Helpers and primitives for benchmarking MDBX sharding strategies.
//!
//! This crate simulates reth's `AccountsHistory` table layout where each address
//! maps to a list of block numbers stored as `ShardedKey<Address> -> IntegerList`.
//! Values are compressed with [RoaringTreemap](roaring), matching production reth behavior.
//!
//! Two storage strategies are implemented for comparison:
//!
//! - **No-shard**: all block numbers for an address in a single key-value pair.
//! - **Sharded**: block numbers split into fixed-size shards (reth default: 2,000).
//!
//! ## Benchmarks (`cargo bench -p bench-mdbx-shard`)
//!
//! Criterion benchmarks covering append, point query, full scan, and unwind
//! across multiple shard sizes (noshard, 100, 500, 1K, 2K, 5K, 10K, 50K) and
//! history sizes (10K, 100K, 500K, 1M block numbers). HTML reports are generated
//! under `target/criterion/`.
//!
//! ## Analysis tests (`cargo test -p bench-mdbx-shard -- --nocapture`)
//!
//! Integration tests that print storage analysis (compressed value sizes, MDBX
//! B-tree page stats) and RoaringTreemap compression ratios.

use rand::Rng;
use reth_libmdbx::*;
use roaring::RoaringTreemap;
use std::path::Path;

/// MDBX page size on Apple Silicon = 16KB
pub const PAGE_SIZE: usize = 16_384;

/// Each block number is u64 = 8 bytes (before compression)
pub const BLOCK_NUM_SIZE: usize = 8;

// ---------------------------------------------------------------------------
// Environment helpers
// ---------------------------------------------------------------------------

pub fn create_env(path: &Path) -> Environment {
    let mut builder = Environment::builder();
    builder.set_max_dbs(16);
    builder.set_geometry(Geometry {
        size: Some(0..=8usize * 1024 * 1024 * 1024), // up to 8 GiB
        growth_step: Some(128 * 1024 * 1024),
        ..Default::default()
    });
    builder.write_map();
    builder.open(path).expect("failed to open MDBX env")
}

// ---------------------------------------------------------------------------
// RoaringTreemap helpers (matching reth's IntegerList)
// ---------------------------------------------------------------------------

/// Create a RoaringTreemap from a sorted iterator of block numbers.
pub fn bitmap_from_sorted(iter: impl IntoIterator<Item = u64>) -> RoaringTreemap {
    RoaringTreemap::from_sorted_iter(iter).expect("input must be sorted")
}

/// Serialize a RoaringTreemap to bytes (matching reth's Compress for IntegerList).
pub fn bitmap_to_bytes(bm: &RoaringTreemap) -> Vec<u8> {
    let mut buf = Vec::with_capacity(bm.serialized_size());
    bm.serialize_into(&mut buf).expect("serialize failed");
    buf
}

/// Deserialize bytes into a RoaringTreemap (matching reth's Decompress for IntegerList).
pub fn bitmap_from_bytes(data: &[u8]) -> RoaringTreemap {
    RoaringTreemap::deserialize_from(data).expect("deserialize failed")
}

// ---------------------------------------------------------------------------
// Key helpers (matching reth's ShardedKey<Address>)
// ---------------------------------------------------------------------------

/// ShardedKey: address (20 bytes) + highest_block_number (8 bytes BE).
pub fn shard_key(address: &[u8; 20], highest_block: u64) -> Vec<u8> {
    let mut key = address.to_vec();
    key.extend_from_slice(&highest_block.to_be_bytes());
    key
}

/// Sentinel key for an address (highest_block = u64::MAX).
pub fn sentinel_key(address: &[u8; 20]) -> Vec<u8> {
    shard_key(address, u64::MAX)
}

// ---------------------------------------------------------------------------
// No-shard approach: all block numbers in a single RoaringTreemap value
// ---------------------------------------------------------------------------

/// Store all block numbers as a single compressed RoaringTreemap value.
pub fn noshard_store(env: &Environment, db_name: &str, address: &[u8; 20], blocks: &RoaringTreemap) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.create_db(Some(db_name), DatabaseFlags::default()).unwrap();
    let key = sentinel_key(address); // just use address + u64::MAX
    let value = bitmap_to_bytes(blocks);
    txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
    txn.commit().unwrap();
}

/// Append new block numbers to the single value (read-modify-write entire bitmap).
pub fn noshard_append(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    new_blocks: impl IntoIterator<Item = u64>,
) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let key = sentinel_key(address);

    // Read existing
    let existing: Option<Vec<u8>> = txn.get(db.dbi(), &key).unwrap();
    let mut bm = match existing {
        Some(data) => bitmap_from_bytes(&data),
        None => RoaringTreemap::new(),
    };

    // Append
    bm.append(new_blocks).expect("blocks must be sorted and greater than existing");

    // Write back entire bitmap
    let value = bitmap_to_bytes(&bm);
    txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
    txn.commit().unwrap();
}

/// Read the single value and deserialize it.
pub fn noshard_read(env: &Environment, db_name: &str, address: &[u8; 20]) -> RoaringTreemap {
    let txn = env.begin_ro_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let key = sentinel_key(address);
    let val: Option<Vec<u8>> = txn.get(db.dbi(), &key).unwrap();
    bitmap_from_bytes(&val.unwrap())
}

// ---------------------------------------------------------------------------
// Sharded approach: matching reth's append_history_index / unwind_history_shards
// ---------------------------------------------------------------------------

/// Bulk-load block numbers into shards of `shard_size`, simulating initial sync.
/// Frozen shards use their highest block as key, last shard uses u64::MAX.
pub fn sharded_bulk_load(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    all_blocks: &RoaringTreemap,
    shard_size: usize,
) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.create_db(Some(db_name), DatabaseFlags::default()).unwrap();

    let all: Vec<u64> = all_blocks.iter().collect();
    let num_chunks = (all.len() + shard_size - 1) / shard_size;

    for (i, chunk) in all.chunks(shard_size).enumerate() {
        let bm = bitmap_from_sorted(chunk.iter().copied());
        let value = bitmap_to_bytes(&bm);

        let highest = if i == num_chunks - 1 {
            u64::MAX // sentinel for last shard
        } else {
            *chunk.last().unwrap()
        };

        let key = shard_key(address, highest);
        txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
    }
    txn.commit().unwrap();
}

/// Append new block numbers, matching reth's `append_history_index` logic:
/// 1. Read sentinel shard (address + u64::MAX)
/// 2. Deserialize, append new blocks
/// 3. If len <= shard_size: upsert back
/// 4. If len > shard_size: freeze full chunks, keep remainder as sentinel
pub fn sharded_append(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    new_blocks: impl IntoIterator<Item = u64>,
    shard_size: usize,
) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let sent_key = sentinel_key(address);

    // Read the current sentinel shard
    let existing: Option<Vec<u8>> = txn.get(db.dbi(), &sent_key).unwrap();
    let mut last_shard = match existing {
        Some(data) => bitmap_from_bytes(&data),
        None => RoaringTreemap::new(),
    };

    // Append new blocks
    last_shard.append(new_blocks).expect("blocks must be sorted");

    // Fast path: still fits in one shard
    if last_shard.len() <= shard_size as u64 {
        let value = bitmap_to_bytes(&last_shard);
        txn.put(db.dbi(), &sent_key, &value, WriteFlags::empty()).unwrap();
        txn.commit().unwrap();
        return;
    }

    // Slow path: need to split
    let all: Vec<u64> = last_shard.iter().collect();
    let chunks: Vec<&[u64]> = all.chunks(shard_size).collect();
    let num_chunks = chunks.len();

    // Delete old sentinel first
    txn.del(db.dbi(), &sent_key, None).unwrap();

    for (i, chunk) in chunks.iter().enumerate() {
        let bm = bitmap_from_sorted(chunk.iter().copied());
        let value = bitmap_to_bytes(&bm);

        let highest = if i == num_chunks - 1 {
            u64::MAX
        } else {
            *chunk.last().unwrap()
        };

        let key = shard_key(address, highest);
        txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
    }
    txn.commit().unwrap();
}

/// Point query: find the shard containing target_block using cursor.set_range,
/// then check if target_block exists in that shard's bitmap.
pub fn sharded_point_query(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    target_block: u64,
) -> bool {
    let txn = env.begin_ro_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let seek_key = shard_key(address, target_block);
    let mut cursor = txn.cursor(db.dbi()).unwrap();

    let result: Result<Option<(Vec<u8>, Vec<u8>)>> = cursor.set_range(&seek_key);
    match result {
        Ok(Some((key, value))) => {
            // Verify key belongs to same address
            if key.len() >= 20 && &key[..20] == address {
                let bm = bitmap_from_bytes(&value);
                bm.contains(target_block)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Read all shards for an address and reconstruct the full bitmap.
pub fn sharded_full_scan(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
) -> RoaringTreemap {
    let txn = env.begin_ro_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let mut cursor = txn.cursor(db.dbi()).unwrap();

    let start_key = shard_key(address, 0);
    let mut result = RoaringTreemap::new();

    let first: Result<Option<(Vec<u8>, Vec<u8>)>> = cursor.set_range(&start_key);
    match first {
        Ok(Some((key, value))) => {
            if key.len() >= 20 && &key[..20] == address {
                let bm = bitmap_from_bytes(&value);
                result |= bm;
            } else {
                return result;
            }
        }
        _ => return result,
    }

    loop {
        let next: Result<Option<(Vec<u8>, Vec<u8>)>> = cursor.next();
        match next {
            Ok(Some((key, value))) => {
                if key.len() >= 20 && &key[..20] == address {
                    let bm = bitmap_from_bytes(&value);
                    result |= bm;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }

    result
}

/// Unwind: remove all block numbers >= unwind_to, matching reth's unwind_history_shards.
/// Works backwards from sentinel shard.
pub fn sharded_unwind(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    unwind_to: u64,
) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let mut cursor = txn.cursor(db.dbi()).unwrap();

    // Start from sentinel (highest possible key for this address)
    let start = sentinel_key(address);
    // Find the sentinel shard first (like reth's seek_exact on u64::MAX key)
    let mut item: Result<Option<(Vec<u8>, Vec<u8>)>> = cursor.set_key(&start);
    let mut partial_shard = Vec::new();

    // Process from sentinel backwards
    while let Ok(Some((key, value))) = item {
        if key.len() < 20 || &key[..20] != address {
            break;
        }

        // Delete current shard
        cursor.del(WriteFlags::empty()).unwrap();

        let bm = bitmap_from_bytes(&value);
        let first_in_shard = bm.iter().next().unwrap_or(u64::MAX);

        if first_in_shard >= unwind_to {
            // Case 1: entire shard above unwind point → deleted, continue backwards
            item = cursor.prev();
            continue;
        }

        let highest_block = if key.len() >= 28 {
            u64::from_be_bytes(key[20..28].try_into().unwrap())
        } else {
            u64::MAX
        };

        if unwind_to <= highest_block {
            // Case 2: boundary shard → keep blocks below unwind_to
            partial_shard = bm.iter().take_while(|&b| b < unwind_to).collect();
        } else {
            // Case 3: entire shard below unwind point → restore it
            partial_shard = bm.iter().collect();
        }
        break;
    }

    // Reinsert partial shard as new sentinel if non-empty
    if !partial_shard.is_empty() {
        let bm = bitmap_from_sorted(partial_shard.into_iter());
        let value = bitmap_to_bytes(&bm);
        let key = sentinel_key(address);
        txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
    }

    txn.commit().unwrap();
}

/// Same unwind for no-shard approach: read entire bitmap, filter, rewrite.
pub fn noshard_unwind(
    env: &Environment,
    db_name: &str,
    address: &[u8; 20],
    unwind_to: u64,
) {
    let txn = env.begin_rw_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let key = sentinel_key(address);

    let existing: Option<Vec<u8>> = txn.get(db.dbi(), &key).unwrap();
    if let Some(data) = existing {
        let bm = bitmap_from_bytes(&data);
        let kept: Vec<u64> = bm.iter().take_while(|&b| b < unwind_to).collect();
        if kept.is_empty() {
            txn.del(db.dbi(), &key, None).unwrap();
        } else {
            let new_bm = bitmap_from_sorted(kept.into_iter());
            let value = bitmap_to_bytes(&new_bm);
            txn.put(db.dbi(), &key, &value, WriteFlags::empty()).unwrap();
        }
    }
    txn.commit().unwrap();
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

pub struct DbStats {
    pub page_size: u32,
    pub depth: u32,
    pub branch_pages: usize,
    pub leaf_pages: usize,
    pub overflow_pages: usize,
    pub entries: usize,
}

impl std::fmt::Display for DbStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "depth={} branch={} leaf={} overflow={} entries={}",
            self.depth, self.branch_pages, self.leaf_pages, self.overflow_pages, self.entries
        )
    }
}

pub fn get_db_stats(env: &Environment, db_name: &str) -> DbStats {
    let txn = env.begin_ro_txn().unwrap();
    let db = txn.open_db(Some(db_name)).unwrap();
    let stat = txn.db_stat(db.dbi()).unwrap();
    DbStats {
        page_size: stat.page_size(),
        depth: stat.depth(),
        branch_pages: stat.branch_pages(),
        leaf_pages: stat.leaf_pages(),
        overflow_pages: stat.overflow_pages(),
        entries: stat.entries(),
    }
}

pub fn random_address() -> [u8; 20] {
    let mut rng = rand::thread_rng();
    let mut addr = [0u8; 20];
    rng.fill(&mut addr);
    addr
}

/// Generate a RoaringTreemap with consecutive block numbers [0, count).
pub fn generate_blocks(count: u64) -> RoaringTreemap {
    bitmap_from_sorted(0..count)
}

/// Generate a RoaringTreemap with realistic sparse block numbers.
/// Simulates an address that is touched on average every `gap` blocks.
pub fn generate_sparse_blocks(count: u64, avg_gap: u64) -> RoaringTreemap {
    let mut rng = rand::thread_rng();
    let mut blocks = Vec::with_capacity(count as usize);
    let mut current: u64 = rng.gen_range(0..avg_gap);
    for _ in 0..count {
        blocks.push(current);
        current += rng.gen_range(1..=avg_gap * 2);
    }
    bitmap_from_sorted(blocks.into_iter())
}

pub fn format_bytes(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
