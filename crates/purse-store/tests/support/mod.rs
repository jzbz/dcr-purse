// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Test support shared by the store's fault-injection, model and migration
//! suites: a power-loss storage backend for redb 5 and a digest of every
//! table.
//!
//! Each test file that needs it says `mod support;`; not every file uses
//! every item.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the support module"
)]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: offsets into in-memory images whose sizes redb bounds"
)]

pub mod powerloss;

use purse_store::{MetaKey, ReadTx, TableId};
use redb::{ReadableDatabase as _, ReadableTable as _, TableDefinition, TableHandle as _};
use sha2::{Digest as _, Sha256};

/// A digest of a store's contents: every table of the schema in
/// declaration order, each as its name, its entry count and every key and
/// value, length-prefixed, in key order. Two stores with the same digest
/// hold the same rows.
pub type StoreDigest = [u8; 32];

fn feed(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_be_bytes());
    h.update(bytes);
}

/// The digest of what a read transaction sees.
pub fn digest(tx: &ReadTx) -> StoreDigest {
    let mut h = Sha256::new();
    for &id in TableId::ALL {
        feed(&mut h, id.name().as_bytes());
        let mut n = 0u64;
        for e in tx
            .range_raw(id, std::ops::Bound::Unbounded, std::ops::Bound::Unbounded)
            .expect("range")
        {
            let (k, v) = e.expect("entry");
            feed(&mut h, &k);
            feed(&mut h, &v);
            n += 1;
        }
        h.update(n.to_be_bytes());
    }
    h.finalize().into()
}

/// The same digest read with redb directly, for an image the store's open
/// would migrate or refuse: tables missing from the file count as empty.
/// Also returns the stored schema version, if any.
pub fn raw_digest(db: &redb::Database) -> (StoreDigest, Option<u32>) {
    let txn = db.begin_read().expect("begin read");
    let names: Vec<String> = txn
        .list_tables()
        .expect("list tables")
        .map(|t| t.name().to_owned())
        .collect();
    let mut h = Sha256::new();
    let mut version = None;
    for &id in TableId::ALL {
        feed(&mut h, id.name().as_bytes());
        let mut n = 0u64;
        if names.iter().any(|x| x == id.name()) {
            let t = txn
                .open_table(TableDefinition::<&[u8], &[u8]>::new(id.name()))
                .expect("open table");
            for e in t.iter().expect("iter") {
                let (k, v) = e.expect("entry");
                if id == TableId::Meta && k.value() == MetaKey::Version.name() {
                    let b: [u8; 4] = v.value().try_into().expect("version row");
                    version = Some(u32::from_be_bytes(b));
                }
                feed(&mut h, k.value());
                feed(&mut h, v.value());
                n += 1;
            }
        }
        h.update(n.to_be_bytes());
    }
    (h.finalize().into(), version)
}

/// A short hex prefix of a digest, for messages.
pub fn short(d: &StoreDigest) -> String {
    purse_testutil::hex(&d[..6])
}
