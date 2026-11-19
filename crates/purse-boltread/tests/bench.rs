// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A manual measurement of the read paths (ignored by default):
//!
//! ```text
//! PURSE_BOLTREAD_BENCH_DB=<file> cargo test --release -p purse-boltread \
//!     --test bench -- --ignored --nocapture
//! ```

// Test code; counters cannot overflow here.
#![allow(clippy::arithmetic_side_effects)]

use std::time::Instant;

use purse_boltread::{Db, OpenOptions, dump};
use purse_testutil::SplitMix64;

#[test]
#[ignore = "manual measurement; needs PURSE_BOLTREAD_BENCH_DB"]
fn scan_benchmark() {
    let Some(path) = std::env::var_os("PURSE_BOLTREAD_BENCH_DB") else {
        eprintln!("PURSE_BOLTREAD_BENCH_DB not set");
        return;
    };
    for cache in [purse_boltread::DEFAULT_CACHE_BYTES, 0] {
        let db = Db::open_with(&path, &OpenOptions::new().cache_bytes(cache)).expect("open");
        let t = Instant::now();
        let s = dump(&db, &mut std::io::sink()).expect("dump");
        let t_dump = t.elapsed();

        // A sequential scan of every bucket through cursors, collecting a
        // sample of keys for the lookups below.
        let t = Instant::now();
        let mut stack = vec![Vec::<Vec<u8>>::new()];
        let mut sample = Vec::new();
        let mut n = 0u64;
        while let Some(p) = stack.pop() {
            let b = db.bucket_path(&p).expect("lookup").expect("exists");
            for item in b.iter() {
                let item = item.expect("item");
                n += 1;
                if item.is_bucket() {
                    let mut q = p.clone();
                    q.push(item.key().to_vec());
                    stack.push(q);
                } else if n.is_multiple_of(7) {
                    sample.push((p.clone(), item.key().to_vec()));
                }
            }
        }
        let t_scan = t.elapsed();

        let mut rng = SplitMix64(1);
        let t = Instant::now();
        let mut found = 0u64;
        for _ in 0..200_000 {
            let (p, k) = &sample[rng.below(sample.len() as u64) as usize];
            let b = db.bucket_path(p).expect("lookup").expect("exists");
            found += u64::from(b.get(k).expect("get").is_some());
        }
        let t_get = t.elapsed();
        assert_eq!(found, 200_000);
        eprintln!(
            "cache {cache:>9}: dump {} entries {t_dump:?}; cursor scan {n} entries {t_scan:?}; 200k random gets {t_get:?}",
            s.entries
        );
    }
}
