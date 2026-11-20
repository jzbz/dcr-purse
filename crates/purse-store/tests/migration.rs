// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The migration mechanism under power loss (brief §6 Phase 2 exit), the
//! refusal of a newer store, and the fixture of every released schema
//! version (ADR-0003 Decision item 6).
//!
//! The crash test opens a version-1 store with a build whose schema is
//! version 2, through a dummy step that rewrites, adds and deletes rows in
//! several tables, and loses power at every storage operation of that open
//! (the open's own writes and the repair included). The image left is read
//! with redb directly, without the store's open, which would migrate it:
//! it must hold exactly the version-1 rows with version 1, or exactly the
//! version-2 rows with version 2. Opened by the version-2 build it must
//! then end at version 2 with the version-2 rows; opened by a version-1
//! build, a version-2 image must be refused. The same runs for a two-step
//! chain, whose images may also stand at the middle version, never between
//! two.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and generated sizes"
)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use purse_errors::Kind;
use purse_store::{
    AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts, AcctHistory,
    AcctHistoryKey, AddrAcctShaKey, AddrByAcctSha, AddrByHash, AddrByPath, AddrPathKey, AddrRow,
    BalanceClass, BalanceHeightKey, BalanceHeights, BalanceKey, BalanceRow, Balances, BirthState,
    BlockByHash, BlockRow, Blocks, CFilterRow, CFilters, CFiltersDetached, ChainBlock,
    CoinTypeKeys, CommitmentRow, CommitmentUnspentRow, Commitments, CommitmentsUnspent,
    CreditFlags, CreditKey, CreditRow, Credits, CreditsInvalidated, DebitRow, Debits,
    DebitsInvalidated, Empty, Hash32, Hash160, Headers, HeadersDetached, History, HistoryKey,
    MasterKeyParams, Meta, MetaKey, Migration, NEWER_VERSION_TEXT, OpenOptions, OutputTag, ReadTx,
    SCHEMA_VERSION, Schema, ScriptLoc, SealedKey, Slip0044Acct0Row, Spender, Store, Table, TableId,
    Tickets, Tip, TxMined, TxMinedRow, TxRecKey, TxUnmined, TxUnminedRow, UnminedCreditRow,
    UnminedCredits, UnminedInputs, Unpublished, UnpublishedFlag, Unspent, UnspentRow, Wallet,
    WalletKey, WriteTx, wallet,
};
use purse_testutil::SplitMix64;
use support::chain::chain_view;
use support::powerloss::{Cut, PowerLoss, Shared};
use support::{StoreDigest, chain_header, digest, raw_digest, short};

// The dummy steps.

/// Version 1 to 2: rewrites every credit, adds 1,500 tickets, deletes every
/// other unspent output and adds a meta row: enough pages that the step's
/// commit spans many storage operations.
fn step_1_to_2(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    let credits: Vec<_> = tx
        .range_raw(
            TableId::Credits,
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )?
        .collect::<Result<_, _>>()?;
    for (k, mut v) in credits {
        v.reverse();
        tx.put_raw(TableId::Credits, &k, &v)?;
    }
    for i in 0u32..1_500 {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&i.wrapping_mul(0x9e37_79b9).to_be_bytes());
        tx.put::<Tickets>(&Hash32(h), &Empty)?;
    }
    let unspent: Vec<Vec<u8>> = tx
        .range_raw(
            TableId::Unspent,
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )?
        .map(|e| e.map(|(k, _)| k))
        .collect::<Result<_, _>>()?;
    for k in unspent.iter().step_by(2) {
        tx.delete_raw(TableId::Unspent, k)?;
    }
    tx.put_raw(TableId::Meta, b"dummy_step", b"2")
}

/// Version 2 to 3: deletes the tickets the first step added and rewrites
/// the accounts.
fn step_2_to_3(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    let tickets: Vec<Vec<u8>> = tx
        .range_raw(
            TableId::Tickets,
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )?
        .map(|e| e.map(|(k, _)| k))
        .collect::<Result<_, _>>()?;
    for k in tickets {
        tx.delete_raw(TableId::Tickets, &k)?;
    }
    let accounts: Vec<_> = tx
        .range_raw(
            TableId::Accounts,
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )?
        .collect::<Result<_, _>>()?;
    for (k, mut v) in accounts {
        v.push(3);
        tx.put_raw(TableId::Accounts, &k, &v)?;
    }
    tx.put_raw(TableId::Meta, b"dummy_step", b"3")
}

const STEPS: &[Migration] = &[
    Migration {
        from: 1,
        apply: step_1_to_2,
    },
    Migration {
        from: 2,
        apply: step_2_to_3,
    },
];

fn schema(version: u32) -> Schema<'static> {
    Schema {
        version,
        migrations: STEPS,
    }
}

// The version-1 store the sweeps start from.

/// Wallet-shaped version-1 rows: 300 credits, 300 unspent outputs, 20
/// accounts and a large transaction record; and the chain tables in the
/// height layout: a main chain of 60 blocks after a reorganisation that
/// left a branch of three detached, with their headers and filters.
fn base_rows(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    let chain = |from: u32, n: u32, mut prev: Hash32, salt: u8| {
        (from..from + n)
            .map(|height| {
                let mut hash = [salt; 32];
                hash[..4].copy_from_slice(&height.to_be_bytes());
                let row = BlockRow {
                    hash: Hash32(hash),
                    time: i64::from(height) * 300,
                    ..BlockRow::default()
                };
                let header = chain_header(height, &prev, salt);
                let filter = CFilterRow {
                    key: [salt; 16],
                    filter: vec![salt; 200],
                };
                prev = row.hash;
                (row, header, filter)
            })
            .collect::<Vec<_>>()
    };
    let a = chain(0, 60, Hash32::default(), 1);
    tx.connect_blocks(0, &as_chain(&a))?;
    let side = chain(57, 3, a[56].0.hash, 2);
    tx.reorganize(56, &as_chain(&side))?;
    let back: Vec<_> = a[57..]
        .iter()
        .map(|(row, header, _)| ChainBlock {
            row,
            header,
            filter: None,
        })
        .collect();
    tx.reorganize(56, &back)?;
    let mut rng = SplitMix64(0x6d69_6772_6174_6531);
    for _ in 0..300 {
        let mut k = vec![0u8; 40];
        rng.fill(&mut k);
        tx.put_raw(TableId::Credits, &k, &rng.bytes(60))?;
        let mut o = vec![0u8; 36];
        rng.fill(&mut o);
        tx.put_raw(TableId::Unspent, &o, &rng.bytes(25))?;
    }
    for n in 0u32..20 {
        tx.put_raw(TableId::Accounts, &n.to_le_bytes(), &rng.bytes(40))?;
    }
    tx.put_raw(TableId::TxMined, &rng.bytes(36), &rng.bytes(30_000))
}

/// The blocks of `v` to connect, each with its filter.
fn as_chain(v: &[(BlockRow, purse_store::HeaderBytes, CFilterRow)]) -> Vec<ChainBlock<'_>> {
    v.iter()
        .map(|(row, header, filter)| ChainBlock {
            row,
            header,
            filter: Some(filter),
        })
        .collect()
}

/// The image of a clean version-1 store holding [`base_rows`].
fn base_image() -> Vec<u8> {
    let b = PowerLoss::new(Vec::new(), true);
    let store = OpenOptions::new()
        .schema(schema(1))
        .create_on_backend(Shared(b.clone()), base_rows)
        .expect("create v1");
    store.close().expect("close");
    b.image()
}

/// The digest and version of an image, read with redb directly.
fn inspect(b: &Arc<PowerLoss>) -> (StoreDigest, Option<u32>) {
    let db = redb::Database::builder()
        .create_with_backend(Shared(b.clone()))
        .expect("redb opens the image");
    let r = raw_digest(&db);
    drop(db);
    r
}

/// The digests of the base image migrated to each version up to `to`
/// (index 0: version 1), and the storage operations of the open that
/// migrates it to `to`.
fn reference(base: &[u8], to: u32) -> (Vec<StoreDigest>, u64) {
    let mut digests = Vec::new();
    for v in 1..=to {
        let b = PowerLoss::new(base.to_vec(), true);
        let store = OpenOptions::new()
            .schema(schema(v))
            .open_on_backend(Shared(b.clone()))
            .expect("migrate");
        drop(store);
        let (d, ver) = inspect(&b);
        assert_eq!(ver, Some(v));
        digests.push(d);
    }
    let b = PowerLoss::new(base.to_vec(), true);
    let before = b.ops();
    drop(
        OpenOptions::new()
            .schema(schema(to))
            .open_on_backend(Shared(b.clone()))
            .expect("migrate"),
    );
    (digests, b.ops() - before)
}

#[derive(Debug, Default)]
struct Outcome {
    trials: u64,
    cut_reached: u64,
    /// Images found at each version, by version - 1.
    at_version: Vec<u64>,
}

/// Loses power at every storage operation of the open that migrates the
/// base image to version `to`, and checks every image left.
fn sweep(to: u32, cut: Cut) -> Outcome {
    let base = base_image();
    let (digests, total) = reference(&base, to);
    for (i, d) in digests.iter().enumerate() {
        eprintln!("version {}: {}", i + 1, short(d));
    }
    eprintln!("migration to {to}: {total} storage operations");
    let mut out = Outcome {
        at_version: vec![0; to as usize],
        ..Outcome::default()
    };
    for n in 0..=total {
        out.trials += 1;
        let b = PowerLoss::new(base.clone(), true);
        b.power_fails_after(n);
        let opened = OpenOptions::new()
            .schema(schema(to))
            .open_on_backend(Shared(b.clone()));
        if !b.power_failed() {
            assert!(opened.is_ok(), "cut after {n}: open failed with power on");
        }
        // Dropping the store makes the clean shutdown's writes, which the
        // cut may also fall among.
        drop(opened);
        if b.power_failed() {
            out.cut_reached += 1;
        }
        let image = b.cut(match cut {
            Cut::Torn(seed) => Cut::Torn(seed ^ n),
            other => other,
        });

        // What the image holds, read without migrating it.
        let b2 = PowerLoss::new(image, true);
        let (got, ver) = inspect(&b2);
        let v = ver.unwrap_or_else(|| panic!("cut after {n}: no version row"));
        assert!(
            (1..=to).contains(&v),
            "cut after {n}: version {v} out of range"
        );
        assert_eq!(
            got,
            digests[v as usize - 1],
            "cut after {n}: the image says version {v} but holds other rows ({})",
            short(&got)
        );
        out.at_version[v as usize - 1] += 1;

        // A build one version older than the image refuses it without
        // changing any row. (redb's open and close still write the file's
        // header and commit slots, so its bytes change.)
        if v > 1 {
            let err = OpenOptions::new()
                .schema(schema(v - 1))
                .open_on_backend(Shared(b2.clone()))
                .err()
                .unwrap_or_else(|| panic!("cut after {n}: an older build opened version {v}"));
            assert_eq!(err.kind(), Kind::Invalid, "{err}");
            assert_eq!(err.newer_version_info(), Some((v, v - 1)));
            assert_eq!(
                inspect(&b2).0,
                got,
                "cut after {n}: the refusal changed a row"
            );
        }

        // The current build finishes the migration.
        let store = OpenOptions::new()
            .schema(schema(to))
            .open_on_backend(Shared(b2.clone()))
            .unwrap_or_else(|e| panic!("cut after {n}: reopen: {e}"));
        let tx = store.read().expect("read");
        assert_eq!(tx.schema_version().expect("version"), Some(to));
        assert_eq!(
            digest(&tx),
            *digests.last().expect("digests"),
            "cut after {n}: the resumed migration ended elsewhere"
        );
        tx.check_chain()
            .unwrap_or_else(|e| panic!("cut after {n}: {e}"));
        chain_view(&tx).unwrap_or_else(|e| panic!("cut after {n}: {e}"));
    }
    eprintln!("{out:?}");
    assert!(out.cut_reached > 0);
    // Cuts before the commit leave version 1, cuts after it the last.
    assert!(out.at_version[0] > 0, "{out:?}");
    assert!(out.at_version[to as usize - 1] > 0, "{out:?}");
    out
}

#[test]
fn a_migration_cut_at_every_write_leaves_the_old_or_the_new_version() {
    sweep(2, Cut::Clean);
}

#[test]
fn a_torn_migration_leaves_the_old_or_the_new_version() {
    sweep(2, Cut::Torn(0x6d69_6772));
}

#[test]
fn a_killed_migration_leaves_the_old_or_the_new_version() {
    sweep(2, Cut::Kept);
}

#[test]
fn a_two_step_migration_cut_anywhere_stands_at_a_version() {
    let out = sweep(3, Cut::Clean);
    assert!(
        out.at_version[1] > 0,
        "no cut left the middle version: {out:?}"
    );
}

#[test]
fn a_store_newer_than_the_build_is_refused_with_its_rows_unchanged() {
    let b = PowerLoss::new(base_image(), true);
    drop(
        OpenOptions::new()
            .schema(schema(3))
            .open_on_backend(Shared(b.clone()))
            .expect("migrate to 3"),
    );
    let before = inspect(&b);
    for older in [1, 2] {
        let err = OpenOptions::new()
            .schema(schema(older))
            .open_on_backend(Shared(b.clone()))
            .err()
            .expect("refused");
        assert_eq!(err.kind(), Kind::Invalid);
        assert_eq!(
            err.to_string(),
            format!("invalid operation: {NEWER_VERSION_TEXT}")
        );
        assert_eq!(err.newer_version_info(), Some((3, older)));
    }
    assert_eq!(inspect(&b), before, "the refusals changed a row");
    // The chain tables, in the height layout, are whole, and the build of
    // the store's own version reads them.
    let store = OpenOptions::new()
        .schema(schema(3))
        .open_on_backend(Shared(b))
        .expect("the newer build opens its store");
    let tx = store.read().unwrap();
    tx.check_chain().unwrap();
    let view = chain_view(&tx).unwrap();
    assert_eq!(view.main.len(), 60);
    assert_eq!(view.detached.len(), 3);
    assert_eq!(view.detached_filters.len(), 3);
    assert_eq!(view.filters.len(), 60);
}

/// The refusal is not read-only at the byte level: redb takes the file for
/// writing before the store reads the version, and its open and clean
/// close write the header and commit slots. A power cut at every one of
/// those storage operations leaves the newer store with its rows and its
/// version, which the current build then opens.
#[test]
fn a_refused_open_cut_anywhere_leaves_the_newer_store() {
    let b = PowerLoss::new(base_image(), true);
    drop(
        OpenOptions::new()
            .schema(schema(2))
            .open_on_backend(Shared(b.clone()))
            .expect("migrate to 2"),
    );
    let newer = b.image();
    let (want, ver) = inspect(&PowerLoss::new(newer.clone(), true));
    assert_eq!(ver, Some(2));
    let total = {
        let b = PowerLoss::new(newer.clone(), true);
        let before = b.ops();
        let err = OpenOptions::new()
            .schema(schema(1))
            .open_on_backend(Shared(b.clone()))
            .err()
            .expect("refused");
        assert_eq!(err.newer_version_info(), Some((2, 1)));
        let ops = b.ops() - before;
        assert!(ops > 0, "the refused open made no storage operation");
        assert_ne!(b.image(), newer, "the refused open left the bytes alone");
        ops
    };
    for n in 0..=total {
        for cut in [Cut::Clean, Cut::Torn(0x7265_6675 ^ n), Cut::Kept] {
            let b = PowerLoss::new(newer.clone(), true);
            b.power_fails_after(n);
            let r = OpenOptions::new()
                .schema(schema(1))
                .open_on_backend(Shared(b.clone()));
            assert!(r.is_err(), "cut after {n}: an older build opened version 2");
            drop(r);
            let image = b.cut(cut);
            let b2 = PowerLoss::new(image, true);
            assert_eq!(
                inspect(&b2),
                (want, Some(2)),
                "cut after {n} ({cut:?}): the newer store changed"
            );
            let store = OpenOptions::new()
                .schema(schema(2))
                .open_on_backend(Shared(b2))
                .unwrap_or_else(|e| panic!("cut after {n}: {e}"));
            assert_eq!(digest(&store.read().unwrap()), want);
        }
    }
}

/// A migration whose commit fails at a storage operation reports an
/// unknown outcome (`Error::commit_outcome_unknown`, through the migration
/// error), and only then may the store hold the new version: every other
/// failed open leaves the old one. One storage operation of the migrating
/// open fails, at each operation in turn, the power staying on.
#[test]
fn a_failed_migration_commit_reports_an_unknown_outcome() {
    let base = base_image();
    let (digests, total) = reference(&base, 2);
    let (mut unknown, mut known) = (0, 0);
    for n in 0..total {
        let b = PowerLoss::new(base.clone(), true);
        b.fail_once_after(n);
        let r = OpenOptions::new()
            .schema(schema(2))
            .open_on_backend(Shared(b.clone()));
        let Err(err) = r else {
            // The failed operation was the clean close's, after the
            // migration committed.
            continue;
        };
        let (got, ver) = inspect(&b);
        let v = ver.expect("a version row");
        assert_eq!(got, digests[v as usize - 1], "op {n}: rows of no version");
        if err.commit_outcome_unknown() {
            assert_eq!(err.migration_from(), Some(1), "op {n}: {err}");
            unknown += 1;
        } else {
            assert_eq!(v, 1, "op {n}: {err} but the migration is kept");
            known += 1;
        }
    }
    eprintln!("{total} operations: {unknown} unknown outcomes, {known} known");
    assert!(unknown > 0);
}

// The fixture of every released schema version.

fn h32(b: u8) -> Hash32 {
    Hash32([b; 32])
}

/// The rows of the version-1 fixture: at least one valid row in every
/// table, written through the typed API.
fn fixture_rows(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    let script = ScriptLoc {
        offset: 107,
        len: 25,
        class: 2,
    };
    let flags = CreditFlags {
        change: true,
        tag: OutputTag::NonStake,
        coinbase: false,
        expiry: false,
    };
    for (n, name) in [(0u32, "default"), (1, "savings"), (0x7fff_ffff, "imported")] {
        tx.put::<Accounts>(
            &AccountKey(n),
            &AccountRow {
                account_type: 0,
                last_used: [4, 1],
                last_returned: [5, 1],
                name: name.as_bytes().to_vec(),
                keys: vec![n as u8; 8],
            },
        )?;
        tx.put::<AccountNames>(&AccountName(name.as_bytes().to_vec()), &AccountNumber(n))?;
    }
    let addr = Hash160([0x11; 20]);
    let imported = Hash160([0x22; 20]);
    tx.put::<AddrByHash>(
        &addr,
        &AddrRow::Derived {
            account: 0,
            branch: 0,
            index: 4,
        },
    )?;
    tx.put::<AddrByHash>(
        &imported,
        &AddrRow::Imported {
            account: 0x7fff_ffff,
            data: vec![0x02; 33],
        },
    )?;
    tx.put::<AddrByPath>(
        &AddrPathKey {
            account: 0,
            branch: 0,
            index: 4,
        },
        &addr,
    )?;
    tx.put::<AddrByAcctSha>(&AddrAcctShaKey::for_address(0, &addr), &addr)?;
    tx.put::<AddrByAcctSha>(
        &AddrAcctShaKey::for_address(0x7fff_ffff, &imported),
        &imported,
    )?;
    // A main chain of three blocks (which also sets the tip) and a
    // detached block with its filter.
    let mut prev = Hash32::default();
    for height in 0u32..3 {
        let hash = h32(0xb0 + height as u8);
        let header = chain_header(height, &prev, height as u8);
        let filter = CFilterRow {
            key: [height as u8; 16],
            filter: vec![0xf0; 20],
        };
        let row = BlockRow {
            hash,
            time: 1_700_000_000 + i64::from(height) * 300,
            vote_bits: 1,
            invalid: false,
            wallet_txs: u32::from(height == 2),
        };
        tx.connect_block(
            height,
            ChainBlock {
                row: &row,
                header: &header,
                filter: Some(&filter),
            },
        )?;
        prev = hash;
    }
    tx.insert_detached(
        &h32(0xbf),
        &chain_header(2, &h32(0xb1), 0xbf),
        Some(&CFilterRow {
            key: [0xbf; 16],
            filter: vec![0xf1; 24],
        }),
    )?;
    let txh = h32(0x7a);
    tx.put::<TxMined>(
        &TxRecKey {
            hash: txh,
            height: 2,
        },
        &TxMinedRow {
            position: 1,
            received: 1_700_000_100,
            tx: vec![0x01; 250],
        },
    )?;
    tx.put::<TxUnmined>(
        &h32(0x7b),
        &TxUnminedRow {
            received: 1_700_000_200,
            tx: vec![0x02; 220],
        },
    )?;
    tx.put::<Unpublished>(&h32(0x7b), &UnpublishedFlag)?;
    tx.put::<History>(
        &HistoryKey {
            height: 2,
            position: 1,
        },
        &txh,
    )?;
    tx.put::<AcctHistory>(
        &AcctHistoryKey {
            account: 0,
            height: 2,
            position: 1,
        },
        &txh,
    )?;
    let ck = CreditKey {
        hash: txh,
        height: 2,
        index: 0,
    };
    let credit = CreditRow {
        amount: 1_000_000,
        flags,
        account: 0,
        script,
        spender: None,
    };
    tx.put::<Credits>(&ck, &credit)?;
    tx.put::<CreditsInvalidated>(
        &CreditKey { index: 1, ..ck },
        &CreditRow {
            spender: Some(Spender {
                hash: h32(0x7c),
                height: 3,
                input: 0,
            }),
            ..credit
        },
    )?;
    let op = purse_store::Outpoint {
        hash: txh,
        index: 0,
    };
    tx.put::<Unspent>(
        &op,
        &UnspentRow {
            height: 2,
            account: 0,
            amount: 1_000_000,
            flags,
            class: BalanceClass::MinedRegular,
            script,
        },
    )?;
    let uop = purse_store::Outpoint {
        hash: h32(0x7b),
        index: 1,
    };
    tx.put::<UnminedCredits>(
        &uop,
        &UnminedCreditRow {
            amount: 500,
            flags,
            account: 1,
            class: BalanceClass::UnminedRegularUnpublished,
            script,
        },
    )?;
    tx.put::<UnminedInputs>(&op, &h32(0x7b))?;
    tx.put::<Debits>(
        &CreditKey {
            hash: h32(0x7b),
            height: 0,
            index: 0,
        },
        &DebitRow {
            amount: 1_000_000,
            credit: ck,
        },
    )?;
    tx.put::<DebitsInvalidated>(
        &CreditKey {
            hash: h32(0x7d),
            height: 1,
            index: 0,
        },
        &DebitRow {
            amount: 7,
            credit: ck,
        },
    )?;
    let ticket = purse_store::Outpoint {
        hash: h32(0x7e),
        index: 0,
    };
    tx.put::<Tickets>(&ticket.hash, &Empty)?;
    tx.put::<Commitments>(
        &ticket,
        &CommitmentRow {
            amount: 2_000,
            account: 0,
        },
    )?;
    tx.put::<CommitmentsUnspent>(
        &ticket,
        &CommitmentUnspentRow {
            unmined_spent: false,
        },
    )?;
    tx.put::<Balances>(
        &BalanceKey {
            account: 0,
            class: BalanceClass::MinedRegular,
        },
        &BalanceRow {
            sum: 1_000_000,
            count: 1,
        },
    )?;
    tx.put::<BalanceHeights>(
        &BalanceHeightKey {
            account: 0,
            class: BalanceClass::MinedRegular,
            height: 2,
        },
        &BalanceRow {
            sum: 1_000_000,
            count: 1,
        },
    )?;
    assert_eq!(
        tx.tip()?,
        Some(Tip {
            height: 2,
            hash: h32(0xb2),
        })
    );
    tx.set_processed_marker(&h32(0xb2))?;
    tx.set_birth_state(&BirthState {
        set_from_height: false,
        set_from_time: false,
        hash: h32(0xb0),
        height: 0,
        time: 1_700_000_000,
    })?;
    tx.set_have_cfilters(true)?;
    // The wallet-wide rows, as wallet creation writes them.
    let params = |b: u8| MasterKeyParams {
        salt: [b; 32],
        digest: [b ^ 0xff; 32],
        n: 262_144,
        r: 8,
        p: 1,
    };
    tx.put_wallet_row::<wallet::WatchOnly>(&false)?;
    tx.put_wallet_row::<wallet::LastAccount>(&AccountNumber(1))?;
    tx.put_wallet_row::<wallet::LastImportedAccount>(&AccountNumber(0x7fff_ffff))?;
    tx.put_wallet_row::<wallet::MasterKeyPub>(&params(0x01))?;
    tx.put_wallet_row::<wallet::MasterKeyPriv>(&params(0x02))?;
    tx.put_wallet_row::<wallet::CryptoKeyPub>(&SealedKey(vec![0x03; 72]))?;
    tx.put_wallet_row::<wallet::CryptoKeyPriv>(&SealedKey(vec![0x04; 72]))?;
    tx.put_wallet_row::<wallet::CoinTypeLegacy>(&CoinTypeKeys {
        enc_pub: Some(vec![0x05; 150]),
        enc_priv: Some(vec![0x06; 150]),
    })?;
    tx.put_wallet_row::<wallet::CoinTypeSlip0044>(&CoinTypeKeys {
        enc_pub: Some(vec![0x07; 150]),
        enc_priv: Some(vec![0x08; 150]),
    })?;
    tx.put_wallet_row::<wallet::Slip0044Account0>(&Slip0044Acct0Row {
        enc_pub: vec![0x09; 150],
        enc_priv: vec![0x0a; 150],
        next_external: 0,
        next_internal: 0,
        name: b"default".to_vec(),
    })
}

/// The checked-in store of each released schema version, and its SHA-256:
/// a fixture is never regenerated once released, so a changed file is a
/// mistake.
const FIXTURES: &[(u32, &str, &str)] = &[(
    1,
    "schema-v1.redb",
    "2153a18e9d5bc02012abd42bf4348206dae41628af14c68746e795d7ced3ee56",
)];

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn count<T: Table>(tx: &ReadTx) -> usize {
    let mut n = 0;
    for r in tx.iter::<T>().expect("iter") {
        r.expect("every row decodes as its table's types");
        n += 1;
    }
    n
}

/// Every row of every table decodes, and every table has at least one.
fn every_table_decodes(tx: &ReadTx) {
    let counts = [
        (TableId::Meta, count::<Meta>(tx)),
        (TableId::Wallet, count::<Wallet>(tx)),
        (TableId::Accounts, count::<Accounts>(tx)),
        (TableId::AccountNames, count::<AccountNames>(tx)),
        (TableId::AddrByHash, count::<AddrByHash>(tx)),
        (TableId::AddrByPath, count::<AddrByPath>(tx)),
        (TableId::AddrByAcctSha, count::<AddrByAcctSha>(tx)),
        (TableId::Headers, count::<Headers>(tx)),
        (TableId::Blocks, count::<Blocks>(tx)),
        (TableId::BlockByHash, count::<BlockByHash>(tx)),
        (TableId::CFilters, count::<CFilters>(tx)),
        (TableId::HeadersDetached, count::<HeadersDetached>(tx)),
        (TableId::CFiltersDetached, count::<CFiltersDetached>(tx)),
        (TableId::TxMined, count::<TxMined>(tx)),
        (TableId::TxUnmined, count::<TxUnmined>(tx)),
        (TableId::Unpublished, count::<Unpublished>(tx)),
        (TableId::History, count::<History>(tx)),
        (TableId::AcctHistory, count::<AcctHistory>(tx)),
        (TableId::Credits, count::<Credits>(tx)),
        (TableId::CreditsInvalidated, count::<CreditsInvalidated>(tx)),
        (TableId::Unspent, count::<Unspent>(tx)),
        (TableId::UnminedCredits, count::<UnminedCredits>(tx)),
        (TableId::UnminedInputs, count::<UnminedInputs>(tx)),
        (TableId::Debits, count::<Debits>(tx)),
        (TableId::DebitsInvalidated, count::<DebitsInvalidated>(tx)),
        (TableId::Tickets, count::<Tickets>(tx)),
        (TableId::Commitments, count::<Commitments>(tx)),
        (TableId::CommitmentsUnspent, count::<CommitmentsUnspent>(tx)),
        (TableId::Balances, count::<Balances>(tx)),
        (TableId::BalanceHeights, count::<BalanceHeights>(tx)),
    ];
    assert_eq!(counts.len(), TableId::ALL.len(), "a table is missing here");
    for (t, n) in counts {
        assert!(n > 0, "table {} is empty in the fixture", t.name());
    }
    assert_eq!(
        count::<Meta>(tx),
        MetaKey::ALL.len(),
        "a meta row is missing"
    );
    assert_eq!(
        count::<Wallet>(tx),
        WalletKey::ALL.len(),
        "a wallet row is missing"
    );
    assert!(tx.tip().expect("tip").is_some());
    tx.check_chain()
        .expect("the chain tables hold their invariants");
    chain_view(tx).expect("the chain tables pass the independent check");
    assert!(tx.processed_marker().expect("marker").is_some());
    assert!(tx.birth_state().expect("birth").is_some());
    assert!(
        tx.wallet_row::<wallet::Slip0044Account0>()
            .expect("acct0")
            .is_some()
    );
}

/// The rows the current build should hold after opening the fixture of
/// `version`: the fixture's rows, migrated. With version 1 current there
/// is nothing to migrate; a later schema adds its steps' effect here.
fn expected_after_upgrade(version: u32) -> StoreDigest {
    assert_eq!(version, 1, "no migration exists yet");
    let b = PowerLoss::new(Vec::new(), true);
    let store = OpenOptions::new()
        .create_on_backend(Shared(b), fixture_rows)
        .expect("create");
    digest(&store.read().expect("read"))
}

#[test]
fn every_released_schema_fixture_opens_and_upgrades_to_current() {
    assert_eq!(
        FIXTURES.last().map(|f| f.0),
        Some(SCHEMA_VERSION),
        "the current schema version has no fixture"
    );
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("purse-store-fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    for &(version, name, sha) in FIXTURES {
        let src = fixture_path(name);
        let bytes = std::fs::read(&src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
        let got = {
            use sha2::Digest as _;
            purse_testutil::hex(&sha2::Sha256::digest(&bytes))
        };
        assert_eq!(
            got, sha,
            "{name} changed; a released fixture is never regenerated"
        );
        // Opening writes to the file; work on a copy.
        let p = dir.join(name);
        std::fs::write(&p, &bytes).unwrap();
        let store = Store::open(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
        let tx = store.read().unwrap();
        assert_eq!(tx.schema_version().unwrap(), Some(SCHEMA_VERSION), "{name}");
        every_table_decodes(&tx);
        assert_eq!(digest(&tx), expected_after_upgrade(version), "{name}");
        drop(tx);
        store.close().unwrap();
        // And it opens again after the upgrade.
        Store::open(&p).unwrap().close().unwrap();
        std::fs::remove_file(&p).unwrap();
    }
}

/// Writes the fixture of the current schema version to
/// `PURSE_STORE_FIXTURE_OUT` (a file path outside the repository). Run once
/// when a schema version is released, then check the file in under
/// `tests/data/` and record its SHA-256 in [`FIXTURES`].
#[test]
#[ignore = "writes a fixture; run by hand when a schema version is released"]
fn write_current_schema_fixture() {
    let out = PathBuf::from(
        std::env::var_os("PURSE_STORE_FIXTURE_OUT").expect("set PURSE_STORE_FIXTURE_OUT"),
    );
    let store = Store::create(&out).expect("create");
    store.update(fixture_rows).expect("rows");
    store.close().expect("close");
    let bytes = std::fs::read(&out).unwrap();
    use sha2::Digest as _;
    eprintln!(
        "{}: {} bytes, sha256 {}",
        out.display(),
        bytes.len(),
        purse_testutil::hex(&sha2::Sha256::digest(&bytes))
    );
}
