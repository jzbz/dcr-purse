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

use purse_errors::Kind;
use purse_store::{
    AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts, BlockRow, Blocks,
    CFilterRow, CreditKey, CreditRow, Credits, Error, FORMAT_MAGIC, Hash32, HeaderBytes, Headers,
    Height, MAX_DURABLE_INTERVAL, MetaKey, Migration, OpenOptions, SCHEMA_VERSION, Schema, Store,
    TableId, Tip, WriteTx,
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
    tx.put::<Headers>(&h(0xee), &HeaderBytes([0xee; 180]))
}

fn add_second_marker(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    tx.put::<Headers>(&h(0xef), &HeaderBytes([0xef; 180]))
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
    assert!(tx.contains::<Headers>(&h(0xee)).unwrap());
    assert!(tx.contains::<Headers>(&h(0xef)).unwrap());
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
    assert!(!tx.contains::<Headers>(&h(0xee)).unwrap());
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

#[test]
fn initial_sync_commits_durably_every_interval_and_at_the_end() {
    let p = path("initial-sync.redb");
    let store = Store::create(&p).unwrap();
    let mut sync = store.initial_sync();
    let mut durable_at = Vec::new();
    for i in 1..=120u32 {
        let ((), durable) = sync
            .batch(|b| {
                let hash = Hash32({
                    let mut x = [0u8; 32];
                    x[..4].copy_from_slice(&i.to_be_bytes());
                    x
                });
                b.put_header(&hash, &HeaderBytes([i as u8; 180]))?;
                b.put_cfilter(
                    &hash,
                    &CFilterRow {
                        key: [1; 16],
                        filter: vec![i as u8; 3],
                    },
                )?;
                b.put_block(
                    i,
                    &BlockRow {
                        hash,
                        ..BlockRow::default()
                    },
                )?;
                b.set_tip(&Tip { height: i, hash })?;
                b.set_processed_marker(&hash)?;
                assert_eq!(b.block_height(&hash)?, Some(i));
                Ok::<_, Error>(())
            })
            .unwrap();
        if durable {
            durable_at.push(i);
        }
    }
    assert_eq!(durable_at, [50, 100]);
    assert_eq!(sync.pending(), 20);
    // Readers see non-durable commits at once.
    assert_eq!(store.read().unwrap().tip().unwrap().unwrap().height, 120);
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.len(TableId::Headers).unwrap(), 120);
    assert_eq!(tx.len(TableId::CFilters).unwrap(), 120);
    assert_eq!(tx.len(TableId::Blocks).unwrap(), 120);
    assert_eq!(tx.len(TableId::BlockByHash).unwrap(), 120);
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
        b.put_header(&h(1), &HeaderBytes([1; 180]))?;
        Err::<(), _>(Error::from(redb::Error::Corrupted("fetch failed".into())))
    });
    assert!(r.is_err());
    assert_eq!(sync.pending(), 0);
    assert!(!store.read().unwrap().contains::<Headers>(&h(1)).unwrap());
    let ((), durable) = sync.batch(|_| Ok::<_, Error>(())).unwrap();
    assert!(!durable);
    let ((), durable) = sync.batch(|_| Ok::<_, Error>(())).unwrap();
    assert!(durable);
}

#[test]
fn a_header_reorganisation_keeps_detached_headers() {
    let p = path("initial-sync-reorg.redb");
    let store = Store::create(&p).unwrap();
    let mut sync = store.initial_sync();
    sync.batch(|b| {
        b.put_header(&h(1), &HeaderBytes([1; 180]))?;
        b.put_block(
            1,
            &BlockRow {
                hash: h(1),
                ..BlockRow::default()
            },
        )
    })
    .unwrap();
    sync.batch(|b| {
        let old = b.remove_block(1)?.unwrap();
        assert_eq!(old.hash, h(1));
        assert_eq!(b.remove_block(1)?, None);
        b.put_header(&h(2), &HeaderBytes([2; 180]))?;
        b.put_block(
            1,
            &BlockRow {
                hash: h(2),
                ..BlockRow::default()
            },
        )
    })
    .unwrap();
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    assert!(tx.contains::<Headers>(&h(1)).unwrap());
    assert_eq!(tx.get::<Blocks>(&Height(1)).unwrap().unwrap().hash, h(2));
    assert_eq!(tx.len(TableId::BlockByHash).unwrap(), 1);
}

/// A header batch cannot change wallet state without durability: a block
/// row with wallet transactions or the invalidation flag is refused, as a
/// new row, as a row to replace and as a row to remove, and nothing of the
/// refused call is written.
#[test]
fn a_header_batch_refuses_block_rows_with_wallet_state() {
    let p = path("initial-sync-wallet-state.redb");
    let store = Store::create(&p).unwrap();
    let with_txs = BlockRow {
        hash: h(10),
        wallet_txs: 3,
        ..BlockRow::default()
    };
    let invalid = BlockRow {
        hash: h(12),
        invalid: true,
        ..BlockRow::default()
    };
    store
        .update(|tx| {
            for (height, row) in [(10, with_txs), (12, invalid)] {
                tx.put::<Blocks>(&Height(height), &row)?;
                tx.put::<purse_store::BlockByHash>(&row.hash, &Height(height))?;
            }
            Ok::<_, Error>(())
        })
        .unwrap();
    let mut sync = store.initial_sync();
    for height in [10, 12] {
        let e = sync.batch(|b| b.remove_block(height)).err().unwrap();
        assert_eq!(e.kind(), Kind::Invalid, "{e}");
        let e = sync
            .batch(|b| {
                b.put_block(
                    height,
                    &BlockRow {
                        hash: h(0x40),
                        ..BlockRow::default()
                    },
                )
            })
            .err()
            .unwrap();
        assert_eq!(e.kind(), Kind::Invalid, "{e}");
    }
    for row in [with_txs, invalid] {
        let e = sync
            .batch(|b| {
                b.put_header(&h(0x41), &HeaderBytes([0x41; 180]))?;
                b.put_block(11, &row)
            })
            .err()
            .unwrap();
        assert_eq!(e.kind(), Kind::Invalid, "{e}");
    }
    assert_eq!(sync.pending(), 0, "a refused batch commits nothing");
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    assert_eq!(tx.get::<Blocks>(&Height(10)).unwrap(), Some(with_txs));
    assert_eq!(tx.get::<Blocks>(&Height(12)).unwrap(), Some(invalid));
    assert_eq!(tx.get::<Blocks>(&Height(11)).unwrap(), None);
    assert!(!tx.contains::<Headers>(&h(0x41)).unwrap());
    assert_eq!(tx.len(TableId::BlockByHash).unwrap(), 2);
    drop(tx);

    // Through Store::update the same changes go through, durably.
    store
        .update(|tx| {
            tx.delete::<Blocks>(&Height(10))?;
            tx.delete::<purse_store::BlockByHash>(&h(10))
        })
        .unwrap();
}

/// Replacing a main-chain block row in a header batch drops the replaced
/// block's hash index, so `block_by_hash` never names a block no longer at
/// that height.
#[test]
fn a_replaced_block_takes_its_hash_index_with_it() {
    let p = path("initial-sync-replace.redb");
    let store = Store::create(&p).unwrap();
    let mut sync = store.initial_sync();
    for hash in [h(1), h(2)] {
        sync.batch(|b| {
            b.put_block(
                5,
                &BlockRow {
                    hash,
                    ..BlockRow::default()
                },
            )
        })
        .unwrap();
    }
    sync.finish().unwrap();
    let tx = store.read().unwrap();
    assert_eq!(
        tx.get::<purse_store::BlockByHash>(&h(1)).unwrap(),
        None,
        "the replaced block's index stayed"
    );
    assert_eq!(
        tx.get::<purse_store::BlockByHash>(&h(2)).unwrap(),
        Some(Height(5))
    );
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
