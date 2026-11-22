// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The database handle: create, open, close, the file lock, refusal of
//! foreign and newer files, the migration mechanism, transactions and the
//! relaxed-durability API.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: version numbers and counters"
)]

use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

mod support;
use support::chain::chain_view;
use support::chain_header;

use purse_errors::Kind;
use purse_store::{
    AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts, BlockByHash,
    BlockRow, Blocks, CFilterRow, CFiltersDetached, ChainBlock, CreditKey, CreditRow, Credits,
    Error, FORMAT_MAGIC, GENESIS_AGAIN_TEXT, Hash32, HeaderBytes, Headers, HeadersDetached, Height,
    IN_MAIN_CHAIN_TEXT, MAX_DURABLE_INTERVAL, MAX_VALUE_SIZE, MetaKey, Migration, OpenOptions,
    SCHEMA_VERSION, Schema, Store, TableId, Tip, WriteTx,
};

fn path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("purse-store-store");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    p
}

fn account(n: u32, name: &str) -> AccountRow {
    AccountRow {
        account_type: 0,
        last_used: [u32::MAX; 2],
        last_returned: [u32::MAX; 2],
        name: name.as_bytes().to_vec(),
        keys: vec![n as u8; 3],
    }
}

fn h(b: u8) -> Hash32 {
    Hash32([b; 32])
}

#[test]
fn create_write_close_open_read() {
    let p = path("roundtrip.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| {
            for (n, name) in [(0, "default"), (1, "one"), (256, "big")] {
                tx.put::<Accounts>(&AccountKey(n), &account(n, name))?;
                tx.put::<AccountNames>(&AccountName(name.into()), &AccountNumber(n))?;
            }
            tx.set_tip(&Tip {
                height: 7,
                hash: h(7),
            })?;
            for i in 0..3u8 {
                tx.put_raw(TableId::Unspent, &[i; 36], b"")?;
            }
            tx.set_processed_marker(&h(6))?;
            Ok::<_, Error>(())
        })
        .unwrap();
    store.close().unwrap();

    let store = Store::open(&p).unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.schema_version().unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(
        tx.tip().unwrap(),
        Some(Tip {
            height: 7,
            hash: h(7)
        })
    );
    assert_eq!(tx.unspent_count().unwrap(), 3);
    assert_eq!(tx.processed_marker().unwrap(), Some(h(6)));
    assert_eq!(
        tx.get::<AccountNames>(&AccountName(b"big".to_vec()))
            .unwrap(),
        Some(AccountNumber(256))
    );
    // Accounts come back in dcrwallet's little-endian order: 0, 256, 1.
    let order: Vec<u32> = tx
        .iter::<Accounts>()
        .unwrap()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(order, [0, 256, 1]);
    let back: Vec<u32> = tx
        .iter::<Accounts>()
        .unwrap()
        .rev()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(back, [1, 256, 0]);
    assert_eq!(tx.len(TableId::Accounts).unwrap(), 3);
    assert_eq!(
        tx.first::<Accounts>().unwrap().unwrap().1,
        account(0, "default")
    );
    assert_eq!(tx.last::<Accounts>().unwrap().unwrap().1, account(1, "one"));
    assert!(tx.contains::<Accounts>(&AccountKey(256)).unwrap());
    assert!(!tx.contains::<Accounts>(&AccountKey(2)).unwrap());
    tx.close().unwrap();
    store.close().unwrap();
}

/// `keys` yields the typed keys of a range in key order from either end,
/// in a read and a write transaction, and never reads a value: a value
/// that does not decode, which `range` refuses, does not stop it.
#[test]
fn keys_walk_without_reading_values() {
    let p = path("keys.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx: &mut WriteTx<'_>| {
            for n in [0u32, 1, 256, 2] {
                tx.put::<Accounts>(&AccountKey(n), &account(n, "a"))?;
            }
            // Account 2's value replaced by bytes no account row decodes from.
            tx.put_raw(TableId::Accounts, &2u32.to_le_bytes(), b"\x01")?;
            let all: Vec<u32> = tx
                .keys::<Accounts>(Bound::Unbounded, Bound::Unbounded)?
                .map(|k| k.map(|k| k.0))
                .collect::<purse_store::Result<_>>()?;
            assert_eq!(all, [0, 256, 1, 2]);
            assert!(
                tx.range::<Accounts>(Bound::Unbounded, Bound::Unbounded)?
                    .any(|r| r.is_err())
            );
            Ok::<_, Error>(())
        })
        .unwrap();
    let tx = store.read().unwrap();
    let back: Vec<u32> = tx
        .keys::<Accounts>(Bound::Unbounded, Bound::Unbounded)
        .unwrap()
        .rev()
        .map(|k| k.unwrap().0)
        .collect();
    assert_eq!(back, [2, 1, 256, 0]);
    let mid: Vec<u32> = tx
        .keys::<Accounts>(
            Bound::Excluded(&AccountKey(0)),
            Bound::Included(&AccountKey(1)),
        )
        .unwrap()
        .map(|k| k.unwrap().0)
        .collect();
    assert_eq!(mid, [256, 1]);
    tx.close().unwrap();
    store.close().unwrap();
}

#[test]
fn every_table_exists_after_create() {
    let p = path("tables.redb");
    let store = Store::create(&p).unwrap();
    let tx = store.read().unwrap();
    for id in TableId::ALL {
        let n = tx.len(*id).unwrap();
        // meta holds the format and the version; the rest is empty.
        assert_eq!(n, if *id == TableId::Meta { 2 } else { 0 }, "{}", id.name());
    }
    assert_eq!(
        tx.get_raw(TableId::Meta, MetaKey::Format.name())
            .unwrap()
            .as_deref(),
        Some(FORMAT_MAGIC)
    );
}

#[test]
fn create_refuses_an_existing_file() {
    let p = path("exists.redb");
    std::fs::write(&p, b"someone else's").unwrap();
    let e = Store::create(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Exist);
    assert_eq!(std::fs::read(&p).unwrap(), b"someone else's");

    // An existing store, too: redb's own create would open it.
    let q = path("exists2.redb");
    Store::create(&q).unwrap().close().unwrap();
    assert_eq!(Store::create(&q).err().unwrap().kind(), Kind::Exist);
}

#[test]
fn a_failed_create_leaves_no_file() {
    let p = path("failed-create.redb");
    let e = OpenOptions::new()
        .create_with(&p, |_| {
            Err(Error::from(redb::Error::Corrupted("test".into())))
        })
        .err()
        .unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert!(!p.exists());
    Store::create(&p).unwrap();
}

#[test]
fn open_refuses_a_missing_file() {
    let e = Store::open(&path("missing.redb")).err().unwrap();
    assert_eq!(e.kind(), Kind::NotExist);
    assert_eq!(e.to_string(), "item does not exist: missing database file");
}

#[test]
fn a_second_open_fails_at_once_while_the_store_is_open() {
    let p = path("locked.redb");
    let store = Store::create(&p).unwrap();
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Other, "{e}");
    assert!(e.engine().is_some());
    assert!(matches!(e.engine(), Some(redb::Error::DatabaseAlreadyOpen)));
    store.close().unwrap();
    let store = Store::open(&p).unwrap();
    drop(store);
    // Dropping releases the lock as closing does.
    Store::open(&p).unwrap();
}

#[test]
fn a_newer_store_is_refused_with_dcrwallets_text() {
    let p = path("newer.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| {
            tx.put_raw(
                TableId::Meta,
                MetaKey::Version.name(),
                &(SCHEMA_VERSION + 1).to_be_bytes(),
            )
        })
        .unwrap();
    store.close().unwrap();
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Invalid);
    assert_eq!(
        e.newer_version_info(),
        Some((SCHEMA_VERSION + 1, SCHEMA_VERSION))
    );
    assert_eq!(
        e.to_string(),
        "invalid operation: database has been upgraded to an unknown newer version"
    );
    let pe: purse_errors::Error = e.into();
    assert_eq!(
        pe.render(purse_errors::DAEMON_SEPARATOR),
        "invalid operation: database has been upgraded to an unknown newer version"
    );
    assert!(pe.is_kind(Kind::Invalid));
}

#[test]
fn a_file_that_is_not_redb_is_refused() {
    let p = path("garbage.redb");
    let mut r = purse_testutil::SplitMix64::from_env("store-garbage");
    std::fs::write(&p, r.bytes(64 * 1024)).unwrap();
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert_eq!(
        e.to_string(),
        "I/O error: Not a redb database: magic number mismatch"
    );
    // An empty file: redb initializes one only when asked to create.
    std::fs::write(&p, b"").unwrap();
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert!(e.to_string().contains("empty"), "{e}");
}

#[test]
fn a_foreign_redb_file_is_refused() {
    const OTHER: redb::TableDefinition<u64, u64> = redb::TableDefinition::new("other");
    let p = path("foreign.redb");
    {
        let db = redb::Database::create(&p).unwrap();
        let txn = db.begin_write().unwrap();
        txn.open_table(OTHER).unwrap().insert(1, 2).unwrap();
        txn.commit().unwrap();
    }
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Invalid);
    assert_eq!(
        e.to_string(),
        "invalid operation: not a dcr-purse wallet store: no meta table"
    );

    // A redb file nothing was committed to: a create that crashed.
    let q = path("uninitialized.redb");
    redb::Database::create(&q).unwrap();
    let e = Store::open(&q).err().unwrap();
    assert_eq!(e.kind(), Kind::NotExist);
    assert_eq!(
        e.to_string(),
        "item does not exist: database has not been initialized"
    );
}

/// Edits a closed store's meta rows with redb directly.
fn edit_meta(p: &Path, f: impl FnOnce(&mut redb::Table<'_, &'static [u8], &'static [u8]>)) {
    const META: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("meta");
    let db = redb::Database::open(p).unwrap();
    let txn = db.begin_write().unwrap();
    {
        let mut t = txn.open_table(META).unwrap();
        f(&mut t);
    }
    txn.commit().unwrap();
}

#[test]
fn damaged_meta_rows_are_refused() {
    let p = path("meta-format.redb");
    Store::create(&p).unwrap().close().unwrap();
    edit_meta(&p, |t| {
        t.insert(b"format".as_slice(), b"something else".as_slice())
            .unwrap();
    });
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Invalid);
    assert!(e.to_string().ends_with("unknown format row"), "{e}");

    let p = path("meta-version.redb");
    Store::create(&p).unwrap().close().unwrap();
    edit_meta(&p, |t| {
        t.remove(b"version".as_slice()).unwrap();
    });
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::NotExist);

    let p = path("meta-version-len.redb");
    Store::create(&p).unwrap().close().unwrap();
    edit_meta(&p, |t| {
        t.insert(b"version".as_slice(), [0u8, 1].as_slice())
            .unwrap();
    });
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert_eq!(e.to_string(), "I/O error: meta: schema version len 2");
    assert_eq!(e.corrupt_row().map(|(t, _)| t), Some(TableId::Meta));
}

#[test]
fn a_missing_table_is_refused() {
    let p = path("missing-table.redb");
    Store::create(&p).unwrap().close().unwrap();
    {
        let db = redb::Database::open(&p).unwrap();
        let txn = db.begin_write().unwrap();
        let def: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("tickets");
        assert!(txn.delete_table(def).unwrap());
        txn.commit().unwrap();
    }
    let e = Store::open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert_eq!(e.to_string(), "I/O error: tickets: table missing");
}

#[test]
fn a_corrupt_row_is_an_io_error_naming_the_table() {
    let p = path("corrupt-row.redb");
    let store = Store::create(&p).unwrap();
    let key = CreditKey {
        hash: h(1),
        height: 2,
        index: 3,
    };
    store
        .update(|tx| {
            use purse_store::Codec as _;
            tx.put_raw(TableId::Credits, &key.encode(), &[1, 2, 3])
        })
        .unwrap();
    let e = store.read().unwrap().get::<Credits>(&key).err().unwrap();
    assert_eq!(e.kind(), Kind::Io);
    assert_eq!(e.to_string(), "I/O error: credits: credit len 3");
}

fn add_marker(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    tx.put::<HeadersDetached>(&h(0xee), &HeaderBytes([0xee; 180]))
}

fn add_second_marker(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    tx.put::<HeadersDetached>(&h(0xef), &HeaderBytes([0xef; 180]))
}

fn failing_step(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    add_marker(tx)?;
    Err(Error::from(redb::Error::Corrupted("step failed".into())))
}

#[test]
fn migrations_run_one_step_per_transaction_and_stamp_the_version() {
    let p = path("migrate.redb");
    Store::create(&p).unwrap().close().unwrap();

    let steps = [
        Migration {
            from: SCHEMA_VERSION,
            apply: add_marker,
        },
        Migration {
            from: SCHEMA_VERSION + 1,
            apply: add_second_marker,
        },
    ];
    let schema = Schema {
        version: SCHEMA_VERSION + 2,
        migrations: &steps,
    };
    let store = OpenOptions::new().schema(schema).open(&p).unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.schema_version().unwrap(), Some(SCHEMA_VERSION + 2));
    assert!(tx.contains::<HeadersDetached>(&h(0xee)).unwrap());
    assert!(tx.contains::<HeadersDetached>(&h(0xef)).unwrap());
    drop(tx);
    store.close().unwrap();

    // This build now refuses it as newer.
    let e = Store::open(&p).err().unwrap();
    assert_eq!(
        e.newer_version_info(),
        Some((SCHEMA_VERSION + 2, SCHEMA_VERSION))
    );
}

#[test]
fn a_failed_step_leaves_the_old_version_and_nothing_of_the_step() {
    let p = path("migrate-fail.redb");
    Store::create(&p).unwrap().close().unwrap();
    let steps = [Migration {
        from: SCHEMA_VERSION,
        apply: failing_step,
    }];
    let schema = Schema {
        version: SCHEMA_VERSION + 1,
        migrations: &steps,
    };
    let e = OpenOptions::new().schema(schema).open(&p).err().unwrap();
    assert_eq!(e.migration_from(), Some(SCHEMA_VERSION));
    assert_eq!(e.kind(), Kind::Io);
    let text = e.to_string();
    let rest = text
        .strip_prefix(&format!(
            "I/O error: migration from schema version {SCHEMA_VERSION}: "
        ))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        rest.contains("step failed") && !rest.contains("I/O error"),
        "{text}"
    );

    let store = Store::open(&p).unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.schema_version().unwrap(), Some(SCHEMA_VERSION));
    assert!(!tx.contains::<HeadersDetached>(&h(0xee)).unwrap());
}

#[test]
fn a_version_without_a_step_is_refused() {
    let p = path("migrate-gap.redb");
    Store::create(&p).unwrap().close().unwrap();
    let schema = Schema {
        version: SCHEMA_VERSION + 1,
        migrations: &[],
    };
    let e = OpenOptions::new().schema(schema).open(&p).err().unwrap();
    assert_eq!(e.kind(), Kind::Invalid);
    assert_eq!(
        e.to_string(),
        format!(
            "invalid operation: no migration from schema version {SCHEMA_VERSION} to {}",
            SCHEMA_VERSION + 1
        )
    );
}

#[test]
fn an_update_that_fails_keeps_nothing() {
    let p = path("abort.redb");
    let store = Store::create(&p).unwrap();
    let r: Result<(), Error> = store.update(|tx| {
        tx.put::<Accounts>(&AccountKey(5), &account(5, "five"))?;
        Err(Error::from(redb::Error::Corrupted("caller".into())))
    });
    let err = r.expect_err("the closure failed");
    // The caller's own error: nothing was committed, nothing is unknown.
    assert!(!err.commit_outcome_unknown());
    assert!(
        !store
            .read()
            .unwrap()
            .contains::<Accounts>(&AccountKey(5))
            .unwrap()
    );
    // The store is still usable.
    store
        .update(|tx| tx.put::<Accounts>(&AccountKey(5), &account(5, "five")))
        .unwrap();
    assert!(
        store
            .read()
            .unwrap()
            .contains::<Accounts>(&AccountKey(5))
            .unwrap()
    );
}

#[test]
fn writes_are_visible_inside_their_transaction() {
    let p = path("own-writes.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| {
            assert!(tx.get::<Accounts>(&AccountKey(1))?.is_none());
            tx.put::<Accounts>(&AccountKey(1), &account(1, "a"))?;
            tx.put::<Accounts>(&AccountKey(2), &account(2, "b"))?;
            assert_eq!(tx.get::<Accounts>(&AccountKey(1))?, Some(account(1, "a")));
            assert_eq!(tx.len(TableId::Accounts)?, 2);
            assert!(tx.delete::<Accounts>(&AccountKey(1))?);
            assert!(!tx.delete::<Accounts>(&AccountKey(1))?);
            let keys: Vec<u32> = tx
                .iter::<Accounts>()?
                .map(|r| r.map(|(k, _)| k.0))
                .collect::<Result<_, _>>()?;
            assert_eq!(keys, [2]);
            Ok::<_, Error>(())
        })
        .unwrap();
}

#[test]
fn ranges_and_prefixes() {
    let p = path("ranges.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| {
            for height in 0..10u32 {
                tx.put::<Blocks>(
                    &Height(height),
                    &BlockRow {
                        hash: h(height as u8),
                        ..BlockRow::default()
                    },
                )?;
            }
            for (hash, index) in [(1u8, 0u32), (1, 1), (1, 256), (2, 0)] {
                tx.put::<Credits>(
                    &CreditKey {
                        hash: h(hash),
                        height: 5,
                        index,
                    },
                    &CreditRow::default(),
                )?;
            }
            Ok::<_, Error>(())
        })
        .unwrap();
    let tx = store.read().unwrap();
    let got: Vec<u32> = tx
        .range::<Blocks>(Bound::Included(&Height(3)), Bound::Excluded(&Height(6)))
        .unwrap()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(got, [3, 4, 5]);
    let got: Vec<u32> = tx
        .range::<Blocks>(Bound::Excluded(&Height(7)), Bound::Unbounded)
        .unwrap()
        .rev()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(got, [9, 8]);
    let got: Vec<u32> = tx
        .prefix::<Credits>(&[1u8; 32])
        .unwrap()
        .map(|r| r.unwrap().0.index)
        .collect();
    assert_eq!(got, [0, 1, 256]);
    // A prefix of 0xff bytes has no successor; it runs to the end.
    assert_eq!(tx.prefix::<Credits>(&[0xff; 4]).unwrap().count(), 0);
    let raw: Vec<_> = tx
        .range_raw(
            TableId::Blocks,
            Bound::Unbounded,
            Bound::Included(&1u32.to_be_bytes()),
        )
        .unwrap()
        .map(|r| r.unwrap().0)
        .collect();
    assert_eq!(
        raw,
        [0u32.to_be_bytes().to_vec(), 1u32.to_be_bytes().to_vec()]
    );
}

#[test]
fn readers_never_wait_for_a_writer() {
    let p = path("concurrency.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| tx.put::<Accounts>(&AccountKey(1), &account(1, "before")))
        .unwrap();
    let (in_txn_tx, in_txn_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    std::thread::scope(|s| {
        let store = &store;
        s.spawn(move || {
            store
                .update(|tx| {
                    tx.put::<Accounts>(&AccountKey(1), &account(1, "after"))?;
                    in_txn_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<_, Error>(())
                })
                .unwrap();
        });
        in_txn_rx.recv().unwrap();
        // The writer holds its transaction open; a reader starts, reads the
        // last commit and finishes without waiting.
        let tx = store.read().unwrap();
        assert_eq!(
            tx.get::<Accounts>(&AccountKey(1)).unwrap().unwrap().name,
            b"before"
        );
        release_tx.send(()).unwrap();
        // A snapshot keeps its view after the writer commits.
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(
            tx.get::<Accounts>(&AccountKey(1)).unwrap().unwrap().name,
            b"before"
        );
    });
    assert_eq!(
        store
            .read()
            .unwrap()
            .get::<Accounts>(&AccountKey(1))
            .unwrap()
            .unwrap()
            .name,
        b"after"
    );
}

#[test]
fn store_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}
    assert_send_sync::<Store>();
    assert_send::<purse_store::ReadTx>();
    assert_send_sync::<Error>();
}

/// A block of a test chain: its row, header and filter.
type TestBlock = (BlockRow, HeaderBytes, CFilterRow);

/// A raw write that breaks one invariant of the chain tables: its name,
/// the start of `check_chain`'s complaint about it (so that each break
/// shows which of the checks catches it), and the write.
type Break = (
    &'static str,
    &'static str,
    fn(&mut WriteTx<'_>, &[TestBlock]),
);

/// The block at `height` of a test chain: hash `salt ‖ height`, a header
/// naming `prev`, a filter of a few bytes.
fn chain_block(height: u32, prev: &Hash32, salt: u8) -> TestBlock {
    let mut hash = [salt; 32];
    hash[..4].copy_from_slice(&height.to_be_bytes());
    let row = BlockRow {
        hash: Hash32(hash),
        time: 1_700_000_000 + i64::from(height),
        ..BlockRow::default()
    };
    let filter = CFilterRow {
        key: [salt; 16],
        filter: vec![height as u8; 3],
    };
    (row, chain_header(height, prev, salt), filter)
}

fn cb<'a>(b: &'a TestBlock) -> ChainBlock<'a> {
    ChainBlock {
        row: &b.0,
        header: &b.1,
        filter: Some(&b.2),
    }
}

/// `n` blocks from `from`, above the block `prev`.
fn branch(from: u32, n: u32, prev: Hash32, salt: u8) -> Vec<TestBlock> {
    let mut out = Vec::new();
    let mut prev = prev;
    for height in from..from + n {
        let b = chain_block(height, &prev, salt);
        prev = b.0.hash;
        out.push(b);
    }
    out
}

/// The chain tables pass the store's check and the tests' independent one.
fn checked(tx: &purse_store::ReadTx) {
    tx.check_chain().unwrap();
    chain_view(tx).unwrap();
}

/// A store holding `blocks` as its main chain from the genesis block.
fn chain_store(name: &str, blocks: &[TestBlock]) -> Store {
    let store = Store::create(&path(name)).unwrap();
    store
        .update(|tx| {
            let v: Vec<ChainBlock<'_>> = blocks.iter().map(cb).collect();
            tx.connect_blocks(0, &v)
        })
        .unwrap();
    store
}

#[test]
fn initial_sync_commits_durably_every_interval_and_at_the_end() {
    let p = path("initial-sync.redb");
    let store = Store::create(&p).unwrap();
    let mut sync = store.initial_sync();
    let mut durable_at = Vec::new();
    let mut prev = Hash32::default();
    for i in 1..=120u32 {
        let b = chain_block(i - 1, &prev, 1);
        let ((), durable) = sync
            .batch(|x| {
                x.connect_block(i - 1, cb(&b))?;
                x.set_processed_marker(&b.0.hash)?;
                assert_eq!(x.block_height(&b.0.hash)?, Some(i - 1));
                assert_eq!(x.header(&b.0.hash)?, Some(b.1));
                assert_eq!(x.tip()?.map(|t| t.height), Some(i - 1));
                Ok::<_, Error>(())
            })
            .unwrap();
        prev = b.0.hash;
        if durable {
            durable_at.push(i);
        }
    }
    assert_eq!(durable_at, [50, 100]);
    assert_eq!(sync.pending(), 20);
    // Readers see non-durable commits at once.
    assert_eq!(store.read().unwrap().tip().unwrap().unwrap().height, 119);
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    for t in [
        TableId::Headers,
        TableId::CFilters,
        TableId::Blocks,
        TableId::BlockByHash,
    ] {
        assert_eq!(tx.len(t).unwrap(), 120, "{}", t.name());
    }
    assert!(tx.is_empty(TableId::HeadersDetached).unwrap());
    checked(&tx);
}

#[test]
fn initial_sync_interval_bounds() {
    let p = path("initial-sync-bounds.redb");
    let store = Store::create(&p).unwrap();
    for bad in [0, MAX_DURABLE_INTERVAL + 1] {
        let e = store.initial_sync().with_interval(bad).err().unwrap();
        assert_eq!(e.kind(), Kind::Invalid);
    }
    let mut every = store.initial_sync().with_interval(1).unwrap();
    for _ in 0..3 {
        let ((), durable) = every.batch(|_| Ok::<_, Error>(())).unwrap();
        assert!(durable);
    }
    assert_eq!(every.pending(), 0);
}

#[test]
fn a_failed_batch_keeps_nothing_and_does_not_count() {
    let p = path("initial-sync-fail.redb");
    let store = Store::create(&p).unwrap();
    let mut sync = store.initial_sync().with_interval(2).unwrap();
    let r = sync.batch(|b| {
        b.insert_detached(&h(1), &HeaderBytes([1; 180]), None)?;
        Err::<(), _>(Error::from(redb::Error::Corrupted("fetch failed".into())))
    });
    assert!(r.is_err());
    assert_eq!(sync.pending(), 0);
    assert!(
        !store
            .read()
            .unwrap()
            .contains::<HeadersDetached>(&h(1))
            .unwrap()
    );
    let ((), durable) = sync.batch(|_| Ok::<_, Error>(())).unwrap();
    assert!(!durable);
    let ((), durable) = sync.batch(|_| Ok::<_, Error>(())).unwrap();
    assert!(durable);
}

/// Main-chain rows are keyed by height and found by hash through the
/// index; a range of filters is one scan; reads inside a write
/// transaction see its own chain writes.
#[test]
fn chain_rows_by_height_and_by_hash() {
    let blocks = branch(0, 10, Hash32::default(), 3);
    let store = chain_store("chain-reads.redb", &blocks);
    let tx = store.read().unwrap();
    checked(&tx);
    for (height, (row, header, filter)) in blocks.iter().enumerate() {
        let height = height as u32;
        assert_eq!(tx.block(height).unwrap(), Some(*row));
        assert_eq!(tx.block_height(&row.hash).unwrap(), Some(height));
        assert_eq!(tx.header_at(height).unwrap(), Some(*header));
        assert_eq!(tx.header(&row.hash).unwrap(), Some(*header));
        assert_eq!(tx.cfilter_at(height).unwrap().as_ref(), Some(filter));
        assert_eq!(tx.cfilter(&row.hash).unwrap().as_ref(), Some(filter));
        assert_eq!(header.height(), height);
    }
    assert_eq!(tx.header(&h(0x99)).unwrap(), None);
    assert_eq!(tx.block_height(&h(0x99)).unwrap(), None);
    let got: Vec<u32> = tx
        .cfilters_in(3..7)
        .unwrap()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(got, [3, 4, 5, 6]);
    let got: Vec<u32> = tx
        .headers_in(8..)
        .unwrap()
        .rev()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(got, [9, 8]);
    let got: Vec<Hash32> = tx
        .blocks_in(..=1)
        .unwrap()
        .map(|r| r.unwrap().1.hash)
        .collect();
    assert_eq!(got, [blocks[0].0.hash, blocks[1].0.hash]);
    assert_eq!(
        tx.tip().unwrap(),
        Some(Tip {
            height: 9,
            hash: blocks[9].0.hash
        })
    );
    // The keys are the heights, big-endian.
    assert_eq!(
        tx.get_raw(TableId::Headers, &5u32.to_be_bytes()).unwrap(),
        Some(blocks[5].1.0.to_vec())
    );
    drop(tx);
    store
        .update(|tx| {
            let next = chain_block(10, &blocks[9].0.hash, 3);
            tx.connect_block(10, cb(&next))?;
            assert_eq!(tx.header(&next.0.hash)?, Some(next.1));
            assert_eq!(tx.cfilters_in(9..)?.count(), 2);
            Ok::<_, Error>(())
        })
        .unwrap();
    checked(&store.read().unwrap());
}

/// A reorganisation moves the detached blocks' headers and filters to the
/// detached tables, where a lookup by hash still finds them, and connects
/// the new branch at the same heights, in one transaction; a block of the
/// old branch connected again takes its rows back.
#[test]
fn a_reorganisation_moves_rows_between_the_layouts() {
    let old = branch(0, 6, Hash32::default(), 1);
    let store = chain_store("chain-reorg.redb", &old);
    let new = branch(4, 4, old[3].0.hash, 2);
    let detached = store
        .update(|tx| {
            let v: Vec<ChainBlock<'_>> = new.iter().map(cb).collect();
            tx.reorganize(3, &v)
        })
        .unwrap();
    assert_eq!(
        detached,
        vec![(5, old[5].0), (4, old[4].0)],
        "detached rows, highest first"
    );
    let tx = store.read().unwrap();
    checked(&tx);
    assert_eq!(tx.tip().unwrap().unwrap().height, 7);
    for (i, b) in new.iter().enumerate() {
        assert_eq!(tx.header_at(4 + i as u32).unwrap(), Some(b.1));
    }
    for b in &old[4..] {
        assert_eq!(tx.block_height(&b.0.hash).unwrap(), None);
        assert_eq!(tx.header(&b.0.hash).unwrap(), Some(b.1), "detached header");
        assert_eq!(tx.cfilter(&b.0.hash).unwrap(), Some(b.2.clone()));
        assert_eq!(tx.get::<HeadersDetached>(&b.0.hash).unwrap(), Some(b.1));
    }
    assert_eq!(tx.len(TableId::HeadersDetached).unwrap(), 2);
    assert_eq!(tx.len(TableId::CFiltersDetached).unwrap(), 2);
    drop(tx);

    // Back to the old branch: the new one is detached, the old blocks
    // promoted from their detached rows, filter included.
    store
        .update(|tx| {
            let gone = tx.disconnect_from(4)?;
            assert_eq!(gone.len(), 4);
            assert_eq!(tx.tip()?.map(|t| t.height), Some(3));
            tx.promote_block(4, &old[4].0)?;
            tx.promote_block(5, &old[5].0)
        })
        .unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    for (height, b) in old.iter().enumerate() {
        assert_eq!(tx.block(height as u32).unwrap(), Some(b.0));
        assert_eq!(tx.cfilter_at(height as u32).unwrap(), Some(b.2.clone()));
    }
    assert_eq!(tx.len(TableId::HeadersDetached).unwrap(), 4);
    assert!(tx.get::<HeadersDetached>(&old[4].0.hash).unwrap().is_none());
    drop(tx);

    // A reorganisation may connect a block of the branch it detaches; a
    // block connected without a filter keeps its detached one.
    store
        .update(|tx| {
            let again = ChainBlock {
                filter: None,
                ..cb(&new[0])
            };
            tx.reorganize(3, &[again])?;
            assert_eq!(tx.cfilter_at(4)?, Some(new[0].2.clone()));
            let back = ChainBlock {
                filter: None,
                ..cb(&old[4])
            };
            tx.reorganize(3, &[back])?;
            assert_eq!(tx.cfilter_at(4)?, Some(old[4].2.clone()));
            Ok::<_, Error>(())
        })
        .unwrap();
    checked(&store.read().unwrap());
}

/// The row count of every chain table, to show a refused call wrote
/// nothing.
fn chain_lens(tx: &mut WriteTx<'_>) -> Vec<u64> {
    [
        TableId::Blocks,
        TableId::Headers,
        TableId::CFilters,
        TableId::BlockByHash,
        TableId::HeadersDetached,
        TableId::CFiltersDetached,
    ]
    .into_iter()
    .map(|t| tx.len(t).unwrap())
    .collect()
}

/// Every refusal of the chain operations writes nothing, and the
/// transaction stays usable after it.
#[test]
fn chain_operations_refuse_what_would_break_the_chain() {
    let blocks = branch(0, 4, Hash32::default(), 1);
    let store = chain_store("chain-refusals.redb", &blocks);
    let before = store.read().unwrap().tip().unwrap();
    store
        .update(|tx| {
            let lens = chain_lens(tx);
            let tip = blocks[3].0.hash;
            let invalid = |e: Error| assert_eq!(e.kind(), Kind::Invalid, "{e}");
            // The block that fits the tip, a real (row, header) pair.
            let b4 = chain_block(4, &tip, 2);
            // Not the next height, above or below, given with a header that
            // states that wrong height: refused for the height or the
            // header, whichever is checked.
            let b = chain_block(5, &tip, 2);
            invalid(tx.connect_block(5, cb(&b)).unwrap_err());
            let b = chain_block(3, &blocks[2].0.hash, 2);
            invalid(tx.connect_block(3, cb(&b)).unwrap_err());
            // The height given is checked on its own: the block that fits
            // the tip, given any height but the next, is refused.
            for wrong in [5, 3, 0, u32::MAX] {
                let e = tx.connect_block(wrong, cb(&b4)).unwrap_err();
                assert_eq!(e.kind(), Kind::Invalid, "{e}");
                assert_eq!(
                    e.to_string(),
                    "invalid operation: a block connects only at the height above the tip",
                    "height {wrong}"
                );
                invalid(tx.connect_blocks(wrong, &[cb(&b4)]).unwrap_err());
                invalid(tx.connect_blocks(wrong, &[]).unwrap_err());
            }
            // A header stating another height, or naming another parent.
            let b = chain_block(5, &tip, 2);
            invalid(tx.connect_block(4, cb(&b)).unwrap_err());
            let b = chain_block(4, &blocks[2].0.hash, 2);
            invalid(tx.connect_block(4, cb(&b)).unwrap_err());
            // Blocks after the first must name the one before.
            let b5 = chain_block(5, &h(0x55), 2);
            invalid(tx.connect_blocks(4, &[cb(&b4), cb(&b5)]).unwrap_err());

            // A main-chain block delivered again, its own header with it:
            // dcrwallet's kind and text, whatever height is given, the
            // check reading the height its header states, as
            // `ExtendMainChain` does before it looks at the parent.
            let exist = |e: Error| {
                assert_eq!(e.kind(), Kind::Exist, "{e}");
                assert_eq!(
                    e.to_string(),
                    format!("item already exists: {IN_MAIN_CHAIN_TEXT}")
                );
            };
            for (j, b) in blocks.iter().enumerate().skip(1) {
                for given in [j as u32, 4, 9] {
                    exist(tx.connect_block(given, cb(b)).unwrap_err());
                }
                // Later in a call, after a block that would connect.
                exist(tx.connect_blocks(4, &[cb(&b4), cb(b)]).unwrap_err());
                // In a reorganisation, at or below the fork point.
                for fork in j as u32..4 {
                    exist(tx.reorganize(fork, &[cb(b)]).unwrap_err());
                }
            }
            // The same block twice in one call: the second copy is a block
            // already in the main chain.
            exist(tx.connect_blocks(4, &[cb(&b4), cb(&b4)]).unwrap_err());
            // The genesis block again: dcrwallet refuses any header stating
            // height 0 with kind `Invalid`.
            let genesis = format!("invalid operation: {GENESIS_AGAIN_TEXT}");
            for given in [0, 4] {
                let e = tx.connect_block(given, cb(&blocks[0])).unwrap_err();
                assert_eq!(e.kind(), Kind::Invalid, "{e}");
                assert_eq!(e.to_string(), genesis);
            }
            let e = tx.reorganize(0, &[cb(&blocks[0])]).unwrap_err();
            assert_eq!(e.to_string(), genesis);
            // A header stating a height of 2^31 or more: dcrwallet reads the
            // height as an `int32`, negative there, and gives the same
            // refusal (`wallet/udb/txmined.go:204-207`), whatever height is
            // given and though the header names the tip.
            for stated in [1u32 << 31, (1 << 31) + 4, u32::MAX] {
                let big = chain_block(stated, &tip, 2);
                for given in [4, stated] {
                    let e = tx.connect_block(given, cb(&big)).unwrap_err();
                    assert_eq!(e.kind(), Kind::Invalid, "{e}");
                    assert_eq!(e.to_string(), genesis, "stated {stated}, given {given}");
                }
                let e = tx.connect_blocks(4, &[cb(&b4), cb(&big)]).unwrap_err();
                assert_eq!(e.to_string(), genesis, "stated {stated}, after b4");
                let e = tx.reorganize(3, &[cb(&big)]).unwrap_err();
                assert_eq!(e.to_string(), genesis, "stated {stated}, reorganised");
            }
            // The last height dcrwallet reads as positive is no such
            // header: refused for not being the next height only.
            let e = tx
                .connect_block(4, cb(&chain_block(i32::MAX as u32, &tip, 2)))
                .unwrap_err();
            assert_eq!(
                e.to_string(),
                "invalid operation: the header does not state the height it is connected at"
            );
            // A main-chain block's hash with a header stating another
            // height is no block delivered again (that header is not the
            // block's), and connecting it would put one hash at two
            // heights: kind `Invalid`.
            let other = (blocks[1].0, chain_header(4, &tip, 9), blocks[1].2.clone());
            invalid(tx.connect_block(4, cb(&other)).unwrap_err());
            invalid(tx.reorganize(3, &[cb(&other)]).unwrap_err());
            let twice = (b4.0, chain_header(5, &b4.0.hash, 2), b4.2.clone());
            invalid(tx.connect_blocks(4, &[cb(&b4), cb(&twice)]).unwrap_err());

            let e = tx
                .insert_detached(&blocks[2].0.hash, &blocks[2].1, None)
                .unwrap_err();
            exist(e);
            // The genesis block stays; a fork point above the tip is no
            // fork point; nothing to promote; no block for a filter.
            invalid(tx.disconnect_from(0).unwrap_err());
            invalid(tx.reorganize(4, &[]).unwrap_err());
            invalid(tx.promote_block(4, &b4.0).unwrap_err());
            invalid(tx.put_cfilter_at(4, &b4.2).unwrap_err());
            // Nothing above the tip to detach.
            assert!(tx.disconnect_from(4)?.is_empty());
            assert_eq!(tx.tip()?, before);
            assert_eq!(chain_lens(tx), lens, "a refused call wrote nothing");
            // Still usable: a valid connect goes through.
            tx.connect_block(4, cb(&b4))
        })
        .unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    assert_eq!(tx.tip().unwrap().unwrap().height, 4);
    assert!(tx.is_empty(TableId::HeadersDetached).unwrap());
    drop(tx);

    // An empty chain starts at height 0, whatever header comes with
    // another height, and has nothing to reorganise.
    let store = Store::create(&path("chain-empty.redb")).unwrap();
    store
        .update(|tx| {
            let b = chain_block(1, &Hash32::default(), 1);
            assert_eq!(
                tx.connect_block(1, cb(&b)).unwrap_err().kind(),
                Kind::Invalid
            );
            for wrong in [1, 3] {
                let e = tx.connect_block(wrong, cb(&blocks[0])).unwrap_err();
                assert_eq!(e.kind(), Kind::Invalid, "{e}");
                assert_eq!(
                    e.to_string(),
                    "invalid operation: an empty main chain starts with a block at height 0"
                );
            }
            // The genesis block twice in one call.
            let e = tx
                .connect_blocks(0, &[cb(&blocks[0]), cb(&blocks[0])])
                .unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("invalid operation: {GENESIS_AGAIN_TEXT}")
            );
            // A first block whose header states 2^31 or more: dcrwallet
            // has no `ExtendMainChain` call for the genesis block, and the
            // store refuses such a header on any chain.
            let big = chain_block(1 << 31, &Hash32::default(), 1);
            let e = tx.connect_block(0, cb(&big)).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("invalid operation: {GENESIS_AGAIN_TEXT}")
            );
            assert_eq!(tx.reorganize(0, &[]).unwrap_err().kind(), Kind::Invalid);
            assert!(tx.disconnect_from(1)?.is_empty());
            assert_eq!(chain_lens(tx), [0; 6]);
            Ok::<_, Error>(())
        })
        .unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.tip().unwrap(), None);
    checked(&tx);
    drop(tx);

    // A tip at height u32::MAX, written raw (the operations never put a
    // block above 2^31 - 1): there is no height above it, so a
    // reorganisation there with blocks to connect is refused by that
    // check, before any block is looked at, and one with none detaches
    // nothing and changes nothing.
    let store = Store::create(&path("chain-max-tip.redb")).unwrap();
    let top = chain_block(u32::MAX, &h(0x01), 4);
    store
        .update(|tx| {
            let key = Height(u32::MAX);
            tx.put::<Blocks>(&key, &top.0)?;
            tx.put::<Headers>(&key, &top.1)?;
            tx.put::<BlockByHash>(&top.0.hash, &key)?;
            tx.set_tip(&Tip {
                height: u32::MAX,
                hash: top.0.hash,
            })
        })
        .unwrap();
    store
        .update(|tx| {
            let lens = chain_lens(tx);
            let tip = tx.tip()?;
            let next = chain_block(u32::MAX, &top.0.hash, 5);
            let e = tx.reorganize(u32::MAX, &[cb(&next)]).unwrap_err();
            assert_eq!(e.kind(), Kind::Invalid, "{e}");
            assert_eq!(
                e.to_string(),
                "invalid operation: no height above the fork point"
            );
            let e = tx.connect_blocks(u32::MAX, &[cb(&next)]).unwrap_err();
            assert_eq!(e.kind(), Kind::Invalid, "{e}");
            assert_eq!(tx.reorganize(u32::MAX, &[])?, []);
            assert_eq!(tx.block(u32::MAX)?, Some(top.0));
            assert_eq!(tx.block_height(&top.0.hash)?, Some(u32::MAX));
            assert_eq!(tx.block_height(&next.0.hash)?, None);
            assert_eq!(tx.tip()?, tip);
            assert_eq!(chain_lens(tx), lens, "a refused call wrote nothing");
            Ok::<_, Error>(())
        })
        .unwrap();
}

/// A filter whose row is longer than bbolt's value limit is refused by
/// every chain operation given it, as the put that writes it would refuse
/// it, but before the call writes anything: the earlier blocks of the
/// call, the detach of a reorganisation and a detached header stay
/// unwritten, and a commit after the refusals leaves a chain that passes
/// both checks.
#[cfg(target_pointer_width = "64")]
#[test]
fn a_filter_over_the_value_limit_is_refused_before_anything_is_written() {
    // Allocated zeroed and never written, so the 2 GiB costs address
    // space, not memory.
    let huge = CFilterRow {
        key: [9; 16],
        filter: vec![0; MAX_VALUE_SIZE - 16 + 1],
    };
    let too_large = |e: Error| {
        assert_eq!(e.kind(), Kind::Invalid, "{e}");
        assert_eq!(e.to_string(), "invalid operation: value too large");
    };
    let blocks = branch(0, 3, Hash32::default(), 1);
    let store = chain_store("chain-huge-filter.redb", &blocks);
    let b3 = chain_block(3, &blocks[2].0.hash, 2);
    let b4 = chain_block(4, &b3.0.hash, 2);
    store
        .update(|tx| {
            let lens = chain_lens(tx);
            let tip = tx.tip()?;
            let huge3 = ChainBlock {
                filter: Some(&huge),
                ..cb(&b3)
            };
            let huge4 = ChainBlock {
                filter: Some(&huge),
                ..cb(&b4)
            };
            let none3 = ChainBlock {
                filter: None,
                ..cb(&b3)
            };
            // The second block's filter, after a first that would connect.
            too_large(tx.connect_blocks(3, &[none3, huge4]).unwrap_err());
            too_large(tx.connect_block(3, huge3).unwrap_err());
            // In a reorganisation, after the detach it would make.
            let x1 = chain_block(1, &blocks[0].0.hash, 3);
            let hugex = ChainBlock {
                filter: Some(&huge),
                ..cb(&x1)
            };
            too_large(tx.reorganize(0, &[hugex]).unwrap_err());
            // A detached header with such a filter: neither is written.
            too_large(
                tx.insert_detached(&b3.0.hash, &b3.1, Some(&huge))
                    .unwrap_err(),
            );
            too_large(tx.put_cfilter_at(2, &huge).unwrap_err());
            assert_eq!(tx.tip()?, tip);
            assert_eq!(chain_lens(tx), lens, "a refused call wrote nothing");
            // The transaction stays usable, and is committed.
            tx.connect_block(3, cb(&b3))
        })
        .unwrap();
    checked(&store.read().unwrap());

    // The same in a relaxed header batch, committed after the refusals.
    let mut sync = store.initial_sync();
    let ((), durable) = sync
        .batch(|b| {
            let r = b.connect_block(
                4,
                ChainBlock {
                    filter: Some(&huge),
                    ..cb(&b4)
                },
            );
            too_large(r.unwrap_err());
            let r = b.insert_detached(&b4.0.hash, &b4.1, Some(&huge));
            too_large(r.unwrap_err());
            assert!(b.header(&b4.0.hash)?.is_none());
            Ok::<_, Error>(())
        })
        .unwrap();
    assert!(!durable);
    assert_eq!(sync.pending(), 1);
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    assert_eq!(tx.tip().unwrap().unwrap().height, 3);
    assert!(tx.is_empty(TableId::HeadersDetached).unwrap());
}

/// A header stating a height of 2^31 or more is refused as dcrwallet's
/// `ExtendMainChain` refuses it, live against the pin through the
/// `oprunner` oracle: there `int32(header.Height)` is negative, so the
/// refusal is `Invalid`, `block 0 cannot be added`, before the parent and
/// the height are looked at (`wallet/udb/txmined.go:204-207`). Both sides
/// extend a chain whose tip is at height 1 with a header naming the tip.
#[test]
fn heights_past_int32_are_refused_as_extend_main_chain_refuses_them() {
    use purse_testutil::serde_json::{Value, json};
    let Some(mut o) = purse_testutil::oracle_or_skip("oprunner") else {
        return;
    };
    let heights = [1u32 << 31, (1 << 31) + 4, u32::MAX - 1, u32::MAX];
    let mut ops = vec![json!({"label": "b1", "op": "store.extend_main_chain"})];
    ops.extend(
        heights
            .iter()
            .map(|h| json!({"op": "store.extend_main_chain", "height": h})),
    );
    let script = json!({
        "format": "purse-opscript 1",
        "name": "headers stating heights past int32",
        "ops": ops,
        "setup": {
            "chain_time": 1_794_000_000,
            "clock": 1_794_990_000,
            "network": "simnet",
            "privpass": "private",
            "pubpass": "public",
            "rand": "16".repeat(32),
            "seed": "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        },
    });
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("purse-store-oprunner-int32");
    let _ = std::fs::remove_dir_all(&dir);
    let run = o
        .try_call(
            "run",
            json!({"script": script, "dir": dir.to_str().expect("utf-8 path")}),
        )
        .unwrap_or_else(|e| panic!("oprunner: {}", e.text));
    let results = run["results"].as_array().expect("results");
    assert_eq!(results.len(), heights.len() + 1, "{run}");
    assert_eq!(results[0]["ok"]["height"], json!(1), "{}", results[0]);

    let blocks = branch(0, 2, Hash32::default(), 1);
    let store = chain_store("chain-int32-heights.redb", &blocks);
    store
        .update(|tx| {
            for (h, go) in heights.iter().zip(&results[1..]) {
                let b = chain_block(*h, &blocks[1].0.hash, 2);
                let e = tx.connect_block(2, cb(&b)).unwrap_err();
                let err: &Value = &go["err"];
                assert_eq!(err["kind"], json!("Invalid"), "height {h}: {go}");
                assert_eq!(e.kind(), Kind::Invalid, "height {h}: {e}");
                assert_eq!(err["text"], json!(e.to_string()), "height {h}");
            }
            Ok::<_, Error>(())
        })
        .unwrap();
    checked(&store.read().unwrap());
}

/// A chain of `n` blocks whose hashes are their headers' (dcrd's
/// serialization and hash, through `dcroxide-wire`), from a genesis block.
fn hashed_chain(n: u32, salt: u8) -> Vec<(BlockRow, HeaderBytes)> {
    let mut out = Vec::new();
    let mut prev = Hash32::default();
    for height in 0..n {
        let raw = chain_header(height, &prev, salt).0;
        let (wire, _) = dcroxide_wire::BlockHeader::from_bytes(&raw).expect("180 bytes decode");
        let header = HeaderBytes(wire.serialize());
        assert_eq!((header.height(), header.prev_block()), (height, prev));
        let row = BlockRow {
            hash: Hash32(wire.block_hash().0),
            ..BlockRow::default()
        };
        prev = row.hash;
        out.push((row, header));
    }
    out
}

/// A main-chain block delivered a second time, as dcrwallet's notification
/// path re-delivers one (each block a real header and the hash of it), is
/// refused with dcrwallet's `Exist`, which that path ignores
/// (`wallet/chainntfns.go:45-49`): `ExtendMainChain` looks for the block
/// at the height its header states before it checks the parent and the
/// height (`wallet/udb/txmined.go:214-226`). The genesis block delivered
/// again is `Invalid`, as dcrwallet refuses any header of height 0.
#[test]
fn a_block_delivered_again_is_refused_as_dcrwallet_refuses_it() {
    let chain = hashed_chain(5, 0x3c);
    let store = Store::create(&path("chain-delivered-again.redb")).unwrap();
    let block = |i: usize| ChainBlock {
        row: &chain[i].0,
        header: &chain[i].1,
        filter: None,
    };
    store
        .update(|tx| {
            let v: Vec<ChainBlock<'_>> = (0..5).map(block).collect();
            tx.connect_blocks(0, &v)
        })
        .unwrap();
    let next = hashed_chain(6, 0x3c).pop().unwrap();
    let next = ChainBlock {
        row: &next.0,
        header: &next.1,
        filter: None,
    };
    store
        .update(|tx| {
            let lens = chain_lens(tx);
            for i in 1..5 {
                // At the next height, at its own, or at any other.
                for given in [5, i as u32, 0, u32::MAX] {
                    let e = tx.connect_block(given, block(i)).unwrap_err();
                    assert_eq!(e.kind(), Kind::Exist, "block {i} at {given}: {e}");
                    assert_eq!(
                        e.to_string(),
                        format!("item already exists: {IN_MAIN_CHAIN_TEXT}")
                    );
                }
                // After the block that does connect, in one call.
                let e = tx.connect_blocks(5, &[next, block(i)]).unwrap_err();
                assert_eq!(e.kind(), Kind::Exist, "{e}");
                // After a rollback to a fork point at or above it.
                let e = tx.reorganize(i as u32, &[block(i)]).unwrap_err();
                assert_eq!(e.kind(), Kind::Exist, "{e}");
            }
            let e = tx.connect_blocks(5, &[next, next]).unwrap_err();
            assert_eq!(e.kind(), Kind::Exist, "{e}");
            let e = tx.connect_block(5, block(0)).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("invalid operation: {GENESIS_AGAIN_TEXT}")
            );
            assert_eq!(chain_lens(tx), lens, "a refused call wrote nothing");
            // A block above the fork point comes back.
            assert_eq!(tx.reorganize(3, &[block(4)])?, vec![(4, chain[4].0)]);
            tx.connect_block(5, next)
        })
        .unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    assert_eq!(tx.tip().unwrap().unwrap().height, 5);
}

/// Detached headers of blocks never connected, and a main-chain block's
/// filter added later.
#[test]
fn detached_inserts_and_missing_filters() {
    let blocks = branch(0, 3, Hash32::default(), 1);
    let store = Store::create(&path("chain-detached.redb")).unwrap();
    store
        .update(|tx| {
            let v: Vec<ChainBlock<'_>> = blocks
                .iter()
                .map(|b| ChainBlock {
                    filter: None,
                    ..cb(b)
                })
                .collect();
            tx.connect_blocks(0, &v)?;
            assert_eq!(tx.cfilter_at(1)?, None);
            tx.put_cfilter_at(1, &blocks[1].2)?;
            let side = chain_block(3, &blocks[2].0.hash, 7);
            tx.insert_detached(&side.0.hash, &side.1, None)?;
            tx.insert_detached(&side.0.hash, &side.1, Some(&side.2))?;
            assert_eq!(tx.header(&side.0.hash)?, Some(side.1));
            assert_eq!(tx.cfilter(&side.0.hash)?, Some(side.2.clone()));
            // Promoted, it becomes the tip with its filter.
            tx.promote_block(3, &side.0)?;
            assert_eq!(tx.cfilter_at(3)?, Some(side.2.clone()));
            Ok::<_, Error>(())
        })
        .unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    let got: Vec<u32> = tx
        .cfilters_in(..)
        .unwrap()
        .map(|r| r.unwrap().0.0)
        .collect();
    assert_eq!(got, [1, 3]);
    assert!(tx.is_empty(TableId::HeadersDetached).unwrap());
    assert!(tx.is_empty(TableId::CFiltersDetached).unwrap());
}

/// `check_chain`, and the tests' independent check, name each invariant
/// broken by a raw write, in either direction of each correspondence.
#[test]
fn check_chain_finds_each_broken_invariant() {
    use purse_store::Codec as _;
    let blocks = branch(0, 4, Hash32::default(), 1);
    checked(&chain_store("chain-check-ok.redb", &blocks).read().unwrap());
    let breaks: [Break; 19] = [
        (
            "gap",
            "blocks hold 3 rows from height 0 to 3, not 0 to 2",
            |tx, _| {
                tx.delete::<Blocks>(&Height(1)).unwrap();
            },
        ),
        // A hole in the main chain with every other table made to agree
        // with it: the header above the hole names the block below it,
        // and no filter, header or index row is left over. Only the
        // contiguity check sees it.
        (
            "hole kept consistent",
            "blocks hold 3 rows from height 0 to 3, not 0 to 2",
            |tx, b| {
                tx.delete::<Blocks>(&Height(1)).unwrap();
                tx.delete::<Headers>(&Height(1)).unwrap();
                tx.delete::<BlockByHash>(&b[1].0.hash).unwrap();
                tx.delete_raw(TableId::CFilters, &Height(1).encode())
                    .unwrap();
                tx.delete_raw(TableId::CFilters, &Height(3).encode())
                    .unwrap();
                tx.put::<Headers>(&Height(2), &chain_header(2, &b[0].0.hash, 1))
                    .unwrap();
            },
        ),
        // The same with the hole at the bottom: no genesis block.
        (
            "no genesis block",
            "blocks hold 3 rows from height 1 to 3, not 0 to 2",
            |tx, b| {
                tx.delete::<Blocks>(&Height(0)).unwrap();
                tx.delete::<Headers>(&Height(0)).unwrap();
                tx.delete::<BlockByHash>(&b[0].0.hash).unwrap();
                tx.delete_raw(TableId::CFilters, &Height(3).encode())
                    .unwrap();
            },
        ),
        ("tip", "the tip row is not the last block", |tx, b| {
            tx.set_tip(&Tip {
                height: 2,
                hash: b[2].0.hash,
            })
            .unwrap();
        }),
        (
            "header missing",
            "headers and blocks differ in length",
            |tx, _| {
                tx.delete::<Headers>(&Height(2)).unwrap();
            },
        ),
        (
            "header height",
            "the header at height 2 states height 9",
            |tx, b| {
                tx.put::<Headers>(&Height(2), &chain_header(9, &b[1].0.hash, 1))
                    .unwrap();
            },
        ),
        (
            "header parent",
            "the header at height 2 does not name the block below",
            |tx, _| {
                tx.put::<Headers>(&Height(2), &chain_header(2, &h(0x44), 1))
                    .unwrap();
            },
        ),
        (
            "index",
            "block_by_hash does not map the block at height 1 to it",
            |tx, b| {
                tx.put::<BlockByHash>(&b[1].0.hash, &Height(3)).unwrap();
            },
        ),
        (
            "detached main",
            "a detached header of a main-chain block",
            |tx, b| {
                tx.put::<HeadersDetached>(&b[1].0.hash, &b[1].1).unwrap();
            },
        ),
        (
            "filter alone",
            "a detached filter without its detached header",
            |tx, b| {
                tx.put::<CFiltersDetached>(&h(0x77), &b[1].2).unwrap();
            },
        ),
        (
            "detached filter of a main-chain block",
            "a detached header of a main-chain block",
            |tx, b| {
                tx.put::<HeadersDetached>(&b[2].0.hash, &b[2].1).unwrap();
                tx.put::<CFiltersDetached>(&b[2].0.hash, &b[2].2).unwrap();
            },
        ),
        (
            "index row missing",
            "block_by_hash and blocks differ in length",
            |tx, b| {
                tx.delete::<BlockByHash>(&b[2].0.hash).unwrap();
            },
        ),
        (
            "stale index row",
            "block_by_hash and blocks differ in length",
            |tx, _| {
                tx.put::<BlockByHash>(&h(0x99), &Height(1)).unwrap();
            },
        ),
        (
            "index row and block row disagree",
            "block_by_hash does not map the block at height 2 to it",
            |tx, b| {
                let mut row = b[2].0;
                row.hash = h(0x98);
                tx.put::<Blocks>(&Height(2), &row).unwrap();
            },
        ),
        // The genesis header moved above the chain: as many headers as
        // blocks, none at a height a filter check reads, and every header
        // above the genesis block in place; only the walk's lookup at
        // height 0 sees it.
        (
            "genesis header moved above the chain",
            "no header at height 0",
            |tx, b| {
                tx.delete::<Headers>(&Height(0)).unwrap();
                tx.put::<Headers>(&Height(b.len() as u32 + 1), &b[0].1)
                    .unwrap();
            },
        ),
        (
            "header above the chain",
            "headers and blocks differ in length",
            |tx, b| {
                tx.put::<Headers>(&Height(4), &chain_header(4, &b[3].0.hash, 1))
                    .unwrap();
            },
        ),
        // A filter at the first height above the tip, and far above it.
        (
            "filter just above the chain",
            "a filter at height 4 above the main chain",
            |tx, _| {
                tx.put_raw(TableId::CFilters, &Height(4).encode(), &[0; 16])
                    .unwrap();
            },
        ),
        (
            "no tip row",
            "the tip row is not the last block",
            |tx, _| {
                tx.delete_raw(TableId::Meta, MetaKey::Tip.name()).unwrap();
            },
        ),
        (
            "tip row with no chain",
            "a tip row with no main chain",
            |tx, b| {
                tx.disconnect_from(1).unwrap();
                tx.delete::<Blocks>(&Height(0)).unwrap();
                tx.delete::<Headers>(&Height(0)).unwrap();
                tx.delete_raw(TableId::CFilters, &Height(0).encode())
                    .unwrap();
                tx.delete::<BlockByHash>(&b[0].0.hash).unwrap();
            },
        ),
    ];
    for (i, (name, why, f)) in breaks.into_iter().enumerate() {
        let store = chain_store(&format!("chain-check-{i}.redb"), &blocks);
        store
            .update(|tx| {
                f(tx, &blocks);
                Ok::<_, Error>(())
            })
            .unwrap();
        let tx = store.read().unwrap();
        let e = tx.check_chain().unwrap_err();
        assert_eq!(e.kind(), Kind::Io, "{name}: {e}");
        let got = e.inconsistency().unwrap_or_default();
        assert!(got.starts_with(why), "{name}: {e}");
        let e = chain_view(&tx).expect_err(name);
        eprintln!("{name}: {e}");
    }
    // A filter far above the main chain.
    let store = chain_store("chain-check-filter.redb", &blocks);
    store
        .update(|tx| tx.put_raw(TableId::CFilters, &Height(9).encode(), &[0; 16]))
        .unwrap();
    let tx = store.read().unwrap();
    let e = tx.check_chain().unwrap_err();
    assert_eq!(
        e.inconsistency(),
        Some("a filter at height 9 above the main chain")
    );
    assert!(chain_view(&tx).is_err());
}

/// A header batch cannot change wallet state without durability: it
/// refuses to connect a block row with wallet transactions or the
/// invalidation flag, and to detach one, and nothing of the refused call
/// is written.
#[test]
fn a_header_batch_refuses_block_rows_with_wallet_state() {
    let mut blocks = branch(0, 4, Hash32::default(), 1);
    blocks[1].0.wallet_txs = 3;
    blocks[3].0.invalid = true;
    let store = chain_store("initial-sync-wallet-state.redb", &blocks);
    let mut sync = store.initial_sync();
    for height in [1, 2, 3] {
        let e = sync.batch(|b| b.disconnect_from(height)).err().unwrap();
        assert_eq!(e.kind(), Kind::Invalid, "{e}");
    }
    let e = sync
        .batch(|b| {
            let x = chain_block(1, &blocks[0].0.hash, 5);
            b.reorganize(0, &[cb(&x)])
        })
        .err()
        .unwrap();
    assert_eq!(e.kind(), Kind::Invalid, "{e}");
    let not_batch = "invalid operation: a block row with wallet transactions or invalidated \
                     is not a header batch's";
    for (txs, invalid) in [(1, false), (0, true)] {
        // The row is refused before the height given is looked at
        // (`connect_block`'s refusals 3 and 4): at the next height or any
        // other, singly or first of several.
        for given in [4, 7, 2, 0, u32::MAX] {
            let e = sync
                .batch(|b| {
                    let mut x = chain_block(4, &blocks[3].0.hash, 5);
                    x.0.wallet_txs = txs;
                    x.0.invalid = invalid;
                    b.connect_block(given, cb(&x))
                })
                .err()
                .unwrap();
            assert_eq!(e.kind(), Kind::Invalid, "{e}");
            assert_eq!(e.to_string(), not_batch, "given {given}");
            let e = sync
                .batch(|b| {
                    let mut x = chain_block(4, &blocks[3].0.hash, 5);
                    x.0.wallet_txs = txs;
                    x.0.invalid = invalid;
                    let y = chain_block(5, &x.0.hash, 5);
                    b.connect_blocks(given, &[cb(&x), cb(&y)])
                })
                .err()
                .unwrap();
            assert_eq!(e.to_string(), not_batch, "given {given}, of two");
        }
    }
    // A main-chain block delivered again, its row carrying wallet state:
    // the refusal is dcrwallet's `Exist`, which comes before the header
    // batch's refusal of the row.
    let e = sync
        .batch(|b| b.connect_block(4, cb(&blocks[1])))
        .err()
        .unwrap();
    assert_eq!(e.kind(), Kind::Exist, "{e}");
    assert_eq!(sync.pending(), 0, "a refused batch commits nothing");
    // A refusal inside a batch leaves it usable.
    let ((), _) = sync
        .batch(|b| {
            assert!(b.disconnect_from(3).is_err());
            let x = chain_block(4, &blocks[3].0.hash, 5);
            b.connect_block(4, cb(&x))
        })
        .unwrap();
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    checked(&tx);
    for (height, b) in blocks.iter().enumerate() {
        assert_eq!(tx.block(height as u32).unwrap(), Some(b.0));
    }
    assert_eq!(tx.tip().unwrap().unwrap().height, 4);
    drop(tx);

    // Through Store::update the same changes go through, durably.
    let gone = store.update(|tx| tx.disconnect_from(1)).unwrap();
    assert_eq!(gone.len(), 4);
    checked(&store.read().unwrap());

    // On an empty chain the row of a first block is refused before the
    // height given is: any height but 0 would be refused for that.
    let store = Store::create(&path("initial-sync-wallet-state-empty.redb")).unwrap();
    let mut sync = store.initial_sync();
    for given in [0, 3] {
        let e = sync
            .batch(|b| {
                let mut x = chain_block(0, &Hash32::default(), 5);
                x.0.wallet_txs = 1;
                b.connect_block(given, cb(&x))
            })
            .err()
            .unwrap();
        assert_eq!(e.to_string(), not_batch, "given {given}");
    }
    assert_eq!(sync.pending(), 0);
    sync.finish().unwrap();
    assert_eq!(store.read().unwrap().tip().unwrap(), None);
}

/// The wallet-wide rows of ADR-0001's `[wallet]`, `[masterkeys]`,
/// `[cryptokeys]`, `[cointypes]` and `[chain]` sections have their typed
/// homes, survive a reopen, and decode in a typed walk of their tables.
#[test]
fn wallet_and_chain_rows_round_trip() {
    use purse_store::wallet::{
        CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, CryptoKeyPub, LastAccount,
        LastImportedAccount, MasterKeyPriv, MasterKeyPub, Slip0044Account0, WatchOnly,
    };
    use purse_store::{
        BirthState, CoinTypeKeys, MasterKeyParams, Meta, SealedKey, Slip0044Acct0Row, Wallet,
        WalletKey,
    };
    let p = path("wallet-rows.redb");
    let store = Store::create(&p).unwrap();
    let params = |b: u8| MasterKeyParams {
        salt: [b; 32],
        digest: [b ^ 0xff; 32],
        n: 262_144,
        r: 8,
        p: 1,
    };
    let birth = BirthState {
        set_from_height: false,
        set_from_time: true,
        hash: h(0xbb),
        height: 0,
        time: 1_700_000_000,
    };
    let acct0 = Slip0044Acct0Row {
        enc_pub: vec![1; 40],
        enc_priv: vec![2; 72],
        next_external: 0,
        next_internal: 0,
        name: b"default".to_vec(),
    };
    store
        .update(|tx| {
            assert_eq!(tx.wallet_row::<WatchOnly>()?, None);
            assert!(!tx.have_cfilters()?);
            tx.put_wallet_row::<WatchOnly>(&false)?;
            tx.put_wallet_row::<LastAccount>(&AccountNumber(3))?;
            tx.put_wallet_row::<LastImportedAccount>(&AccountNumber(0x7fff_ffff))?;
            tx.put_wallet_row::<MasterKeyPub>(&params(1))?;
            tx.put_wallet_row::<MasterKeyPriv>(&params(2))?;
            tx.put_wallet_row::<CryptoKeyPub>(&SealedKey(vec![3; 72]))?;
            tx.put_wallet_row::<CryptoKeyPriv>(&SealedKey(vec![4; 72]))?;
            tx.put_wallet_row::<CoinTypeLegacy>(&CoinTypeKeys {
                enc_pub: Some(vec![5; 150]),
                enc_priv: Some(vec![6; 150]),
            })?;
            tx.put_wallet_row::<CoinTypeSlip0044>(&CoinTypeKeys {
                enc_pub: Some(vec![7; 150]),
                enc_priv: None,
            })?;
            tx.put_wallet_row::<Slip0044Account0>(&acct0)?;
            tx.set_birth_state(&birth)?;
            tx.set_have_cfilters(true)?;
            assert_eq!(tx.wallet_row::<LastAccount>()?, Some(AccountNumber(3)));
            Ok::<_, Error>(())
        })
        .unwrap();
    store.close().unwrap();
    let store = Store::open(&p).unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.wallet_row::<WatchOnly>().unwrap(), Some(false));
    assert_eq!(tx.wallet_row::<MasterKeyPriv>().unwrap(), Some(params(2)));
    assert_eq!(
        tx.wallet_row::<CoinTypeSlip0044>().unwrap(),
        Some(CoinTypeKeys {
            enc_pub: Some(vec![7; 150]),
            enc_priv: None,
        })
    );
    assert_eq!(tx.wallet_row::<Slip0044Account0>().unwrap(), Some(acct0));
    assert_eq!(tx.birth_state().unwrap(), Some(birth));
    assert!(tx.have_cfilters().unwrap());
    let names: Vec<WalletKey> = tx.iter::<Wallet>().unwrap().map(|r| r.unwrap().0).collect();
    assert_eq!(names, WalletKey::ALL, "every row, in name order");
    let meta: Vec<MetaKey> = tx.iter::<Meta>().unwrap().map(|r| r.unwrap().0).collect();
    assert_eq!(
        meta,
        [
            MetaKey::Birth,
            MetaKey::Format,
            MetaKey::HaveCFilters,
            MetaKey::Version
        ]
    );
    drop(tx);
    store
        .update(|tx| {
            assert!(tx.delete_wallet_row::<CryptoKeyPriv>()?);
            assert!(!tx.delete_wallet_row::<CryptoKeyPriv>()?);
            Ok::<_, Error>(())
        })
        .unwrap();
    assert_eq!(
        store.read().unwrap().wallet_row::<CryptoKeyPriv>().unwrap(),
        None
    );
}

/// dcrwallet's `KeyN` of `u` is the length of `unspent`, which redb keeps
/// in the table's root: no counter to maintain, none to drift. In a write
/// transaction it counts the transaction's own writes (QK-0003).
#[test]
fn the_unspent_count_is_the_table_length() {
    let p = path("unspent-count.redb");
    let store = Store::create(&p).unwrap();
    store
        .update(|tx| {
            for i in 0..1_000u32 {
                let mut k = [0u8; 36];
                k[..4].copy_from_slice(&i.to_be_bytes());
                tx.put_raw(TableId::Unspent, &k, b"")?;
            }
            assert_eq!(tx.unspent_count()?, 1_000);
            tx.delete_raw(TableId::Unspent, &[0u8; 36])?;
            assert_eq!(tx.unspent_count()?, 999);
            Ok::<_, Error>(())
        })
        .unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.unspent_count().unwrap(), 999);
    assert_eq!(tx.len(TableId::Unspent).unwrap(), 999);
}

#[test]
fn errors_render_as_dcrwallet_wraps_engine_errors() {
    let e = Error::from(redb::Error::Corrupted("bad page".into()));
    assert_eq!(e.kind(), Kind::Io);
    assert!(e.to_string().starts_with("I/O error: "), "{e}");
    let pe = e.to_purse_error();
    assert_eq!(pe.render(purse_errors::DAEMON_SEPARATOR), e.to_string());
    let e = Error::from(redb::Error::DatabaseAlreadyOpen);
    assert_eq!(e.kind(), Kind::Other);
    // Other contributes no text, as in Go.
    assert_eq!(e.to_string(), redb::Error::DatabaseAlreadyOpen.to_string());
    assert_eq!(
        Error::from(redb::Error::TableDoesNotExist("x".into())).kind(),
        Kind::NotExist
    );
    assert_eq!(
        Error::from(redb::Error::ValueTooLarge(1)).kind(),
        Kind::Invalid
    );
    assert_eq!(
        Error::from(redb::Error::DatabaseClosed).kind(),
        Kind::Invalid
    );
    assert_eq!(
        Error::from(redb::Error::UpgradeRequired(2)).kind(),
        Kind::Invalid
    );
    assert_eq!(
        Error::from(redb::Error::TransactionInProgress).kind(),
        Kind::Bug
    );
}

/// The child half of `another_process_cannot_open_an_open_store`: run with
/// `PURSE_STORE_LOCK_PROBE` set to a store's path, it requires that opening
/// the store fails with the lock error. Without the variable it does
/// nothing.
#[test]
fn lock_probe_child() {
    let Some(p) = std::env::var_os("PURSE_STORE_LOCK_PROBE") else {
        return;
    };
    let e = Store::open(Path::new(&p))
        .err()
        .expect("the open must fail");
    assert_eq!(e.kind(), Kind::Other, "{e}");
    assert!(
        matches!(e.engine(), Some(redb::Error::DatabaseAlreadyOpen)),
        "{e}"
    );
}

#[test]
fn another_process_cannot_open_an_open_store() {
    let p = path("locked-cross-process.redb");
    let store = Store::create(&p).unwrap();
    let probe = |expect_locked: bool| {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lock_probe_child", "--test-threads=1"])
            .env("PURSE_STORE_LOCK_PROBE", &p)
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            expect_locked,
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    };
    probe(true);
    store.close().unwrap();
    // Closed: the child's open succeeds, so its assertion fails.
    probe(false);
}
