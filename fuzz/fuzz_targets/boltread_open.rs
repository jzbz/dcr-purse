// SPDX-License-Identifier: ISC
//! bbolt reader fuzz target: a `wallet.db` handed to the importer may be
//! corrupt or hostile.
//!
//! Whatever the bytes, opening, walking, dumping and seeking must return
//! `Ok` or `Err`: no panic, no hang, no allocation beyond the limits the
//! options set.  A dump that succeeds is the same when taken twice.
//!
//! The first byte picks the mode.  Raw bytes almost never get past the meta
//! page checksum, so the other mode starts from a real database written by
//! bbolt and applies the input as a list of patches (a 3-byte offset, a
//! length byte, then the bytes), which reaches the page, element and bucket
//! decoding behind the meta pages.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_boltread::{Db, OpenOptions, dump};

static BASE: &[u8] = include_bytes!("../../crates/purse-boltread/tests/data/frozen-4k.db");

/// Bounds a walk: a file of this size has far fewer entries.
const MAX_STEPS: usize = 100_000;

fn exercise(bytes: Vec<u8>, seek_keys: &[u8]) {
    let options = OpenOptions::new()
        .lock(false)
        .cache_bytes(64 * 1024)
        .max_page_bytes(1 << 20);
    let Ok(db) = Db::from_source(bytes, &options) else {
        return;
    };
    let _ = db.freelist();
    let _ = db.verify_unchanged();
    let root = db.root();
    let _ = root.stats();

    let mut first = Vec::new();
    if let Ok(summary) = dump(&db, &mut first) {
        let mut second = Vec::new();
        let again = dump(&db, &mut second).expect("a dump that succeeded succeeds again");
        assert_eq!(summary.sha256, again.sha256);
        assert_eq!(first, second);
    }

    let mut cursor = root.cursor();
    let mut item = cursor.first();
    let mut steps = 0;
    while let Ok(Some(entry)) = item {
        if entry.is_bucket() {
            if let Ok(Some(bucket)) = root.bucket(entry.key()) {
                let _ = bucket.stats();
                for (n, entry) in bucket.iter().enumerate() {
                    if entry.is_err() || n >= MAX_STEPS {
                        break;
                    }
                }
                let mut inner = bucket.cursor();
                for key in seek_keys.chunks(3) {
                    let _ = inner.seek(key);
                    let _ = inner.next();
                    let _ = inner.prev();
                    let _ = bucket.get(key);
                }
                let _ = inner.last();
                let _ = inner.prev();
            }
        }
        steps += 1;
        if steps >= MAX_STEPS {
            break;
        }
        item = cursor.next();
    }
    let _ = cursor.last();
    let _ = cursor.prev();
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        exercise(rest.to_vec(), rest.get(..24).unwrap_or(rest));
        return;
    }
    let mut bytes = BASE.to_vec();
    let mut patches = rest;
    while let [a, b, c, len, tail @ ..] = patches {
        let offset = usize::from(*a) << 16 | usize::from(*b) << 8 | usize::from(*c);
        let len = usize::from(*len & 0x0f) + 1;
        let Some((patch, tail)) = tail.split_at_checked(len) else {
            break;
        };
        let offset = offset % bytes.len();
        let end = (offset + len).min(bytes.len());
        bytes[offset..end].copy_from_slice(&patch[..end - offset]);
        patches = tail;
    }
    exercise(bytes, rest.get(..24).unwrap_or(rest));
});
