//! Data analysis tests for MDBX sharding strategies.
//!
//! These are not performance benchmarks — they measure data characteristics:
//!
//! - **storage_value_sizes_and_page_stats** — prints compressed value sizes per
//!   shard size and MDBX B-tree page layout (depth, leaf/branch/overflow pages)
//!   for 1M sparse block numbers.
//!
//! - **compression_ratio** — compares raw `u64` array sizes against
//!   RoaringTreemap-compressed sizes across different history lengths.
//!
//! Run with `--nocapture` to see printed output:
//!   cargo test -p bench-mdbx-shard -- --nocapture

use bench_mdbx_shard::*;
use tempfile::tempdir;

/// Shard sizes to compare (reth uses 2,000)
const SHARD_SIZES: &[usize] = &[100, 500, 1_000, 2_000, 5_000, 10_000, 50_000];

/// History sizes: number of block numbers per address
const HISTORY_SIZES: &[(u64, &str)] = &[
    (10_000, "10K"),
    (100_000, "100K"),
    (500_000, "500K"),
    (1_000_000, "1M"),
];

const AVG_GAP: u64 = 10;

// ---------------------------------------------------------------------------
// Storage: compressed value sizes and MDBX page stats per shard size
// ---------------------------------------------------------------------------

#[test]
fn storage_value_sizes_and_page_stats() {
    println!("\n{}", "=".repeat(120));
    println!("STORAGE: Value sizes (compressed) and page layout per shard size");
    println!("  'no_shard' = all block numbers in 1 key-value pair (1 RoaringTreemap)");
    println!("{}", "=".repeat(120));

    print!("{:>8} |", "blocks");
    print!(" {:>10} |", "no_shard");
    for &ss in SHARD_SIZES {
        print!(" {:>8}", format!("s={}", ss));
    }
    println!();

    print!("{:>8} |", "");
    print!(" {:>10} |", "val_size");
    for _ in SHARD_SIZES {
        print!(" {:>8}", "val_size");
    }
    println!();
    println!("{}", "-".repeat(120));

    for &(count, label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);
        let noshard_bytes = bitmap_to_bytes(&blocks);

        print!("{:>8} |", label);
        print!(" {:>10} |", format_bytes(noshard_bytes.len()));

        for &shard_size in SHARD_SIZES {
            let all: Vec<u64> = blocks.iter().collect();
            let num_shards = (all.len() + shard_size - 1) / shard_size;
            let mut total_bytes = 0usize;
            for chunk in all.chunks(shard_size) {
                let bm = bitmap_from_sorted(chunk.iter().copied());
                total_bytes += bitmap_to_bytes(&bm).len();
            }
            let avg = total_bytes / num_shards;
            print!(" {:>8}", format_bytes(avg));
        }
        println!();
    }

    // Page stats detail for 1M
    println!("\n  Page stats for 1M block numbers (sparse, avg_gap={}):", AVG_GAP);
    println!(
        "  {:>12} | {:>6} {:>6} {:>8} {:>8} {:>8}",
        "approach", "depth", "leaf", "branch", "overflow", "entries"
    );
    println!("  {}", "-".repeat(65));

    let blocks_1m = generate_sparse_blocks(1_000_000, AVG_GAP);

    {
        let dir = tempdir().unwrap();
        let env = create_env(dir.path());
        let addr = random_address();
        noshard_store(&env, "ns", &addr, &blocks_1m);
        let st = get_db_stats(&env, "ns");
        println!(
            "  {:>12} | {:>6} {:>6} {:>8} {:>8} {:>8}",
            "no_shard", st.depth, st.leaf_pages, st.branch_pages, st.overflow_pages, st.entries
        );
    }

    for &shard_size in SHARD_SIZES {
        let dir = tempdir().unwrap();
        let env = create_env(dir.path());
        let addr = random_address();
        let db_name = format!("s{}", shard_size);
        sharded_bulk_load(&env, &db_name, &addr, &blocks_1m, shard_size);
        let st = get_db_stats(&env, &db_name);
        println!(
            "  {:>12} | {:>6} {:>6} {:>8} {:>8} {:>8}",
            format!("s={}", shard_size),
            st.depth, st.leaf_pages, st.branch_pages, st.overflow_pages, st.entries
        );
    }
}

// ---------------------------------------------------------------------------
// Compression ratio: RoaringTreemap size vs raw u64 array
// ---------------------------------------------------------------------------

#[test]
fn compression_ratio() {
    println!("\n{}", "=".repeat(120));
    println!("COMPRESSION: RoaringTreemap size vs raw u64 array");
    println!("{}", "=".repeat(120));
    println!(
        "{:>8} | {:>12} {:>12} {:>8} | {:>10}",
        "blocks", "raw_size", "compressed", "ratio", "per_entry"
    );
    println!("{}", "-".repeat(65));

    for &(count, label) in HISTORY_SIZES {
        let blocks = generate_sparse_blocks(count, AVG_GAP);
        let raw_size = count as usize * BLOCK_NUM_SIZE;
        let compressed = bitmap_to_bytes(&blocks);
        let ratio = raw_size as f64 / compressed.len() as f64;
        let per_entry = compressed.len() as f64 / count as f64;

        println!(
            "{:>8} | {:>12} {:>12} {:>7.1}x | {:>8.1} B",
            label,
            format_bytes(raw_size),
            format_bytes(compressed.len()),
            ratio,
            per_entry,
        );
    }
}
