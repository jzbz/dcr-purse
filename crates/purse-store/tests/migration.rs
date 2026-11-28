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

use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut};
use purse_errors::Kind;
use purse_store::{
    AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts, AcctHistory,
    AcctHistoryKey, AddrAcctShaKey, AddrByAcctSha, AddrByHash, AddrByPath, AddrPathKey, AddrRow,
    AgendaChoice, AgendaKey, AgendaPrefs, BalanceClass, BalanceHeightKey, BalanceHeights,
    BalanceKey, BalanceRow, Balances, BirthState, BlockByHash, BlockRow, Blocks, CFilterRow,
    CFilters, CFiltersDetached, ChainBlock, CoinTypeKeys, CommitmentRow, CommitmentUnspentRow,
    Commitments, CommitmentsUnspent, CreditFlags, CreditKey, CreditRow, Credits,
    CreditsInvalidated, DebitRow, Debits, DebitsInvalidated, Empty, Hash32, Hash160, Headers,
    HeadersDetached, History, HistoryKey, MasterKeyParams, Meta, MetaKey, Migration, MultisigBlock,
    MultisigOutRow, MultisigOuts, MultisigSpender, MultisigUnspent, NEWER_VERSION_TEXT,
    OpenOptions, OutputTag, PiKey, ReadTx, SCHEMA_VERSION, Schema, ScriptLoc, SealedKey,
    Slip0044Acct0Row, Spender, Store, TSpendPolicies, Table, TableId, TicketAgendaKey,
    TicketAgendaPrefs, Tickets, Tip, TreasuryKeyPolicies, TreasuryVote, TxMined, TxMinedRow,
    TxRecKey, TxUnmined, TxUnminedRow, UnminedCreditRow, UnminedCredits, UnminedInputs,
    Unpublished, UnpublishedFlag, Unspent, UnspentRow, VspFeeStatus, VspHosts, VspPubKeys,
    VspTSpendKey, VspTSpendPolicies, VspTicketRow, VspTickets, VspTreasuryKey,
    VspTreasuryKeyPolicies, Wallet, WalletKey, WriteTx, wallet,
};
use purse_testutil::SplitMix64;
use support::chain::chain_view;
use support::powerloss::{Cut, PowerLoss, Shared};
use support::records::records_view;
use support::{StoreDigest, chain_header, digest, raw_digest, short};

// The dummy steps.

/// Version 1 to 2: rewrites every credit, adds 1,500 tickets, deletes every
/// other unspent output, spends every unspent multisig output, moves every
/// VSP ticket to the next fee status, records a new VSP host, clears a
/// treasury policy and adds an agenda choice (the record tables through
/// their operations, so that their indexes change in the step), and adds a
/// meta row: enough pages that the step's commit spans many storage
/// operations.
fn step_1_to_2(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
    let outs: Vec<_> = tx.iter::<MultisigOuts>()?.collect::<Result<_, _>>()?;
    for (op, row) in outs {
        if row.spender.is_none() {
            let spent = MultisigOutRow {
                spender: Some(MultisigSpender {
                    hash: Hash32([0x5e; 32]),
                    input: op.index,
                }),
                ..row
            };
            tx.put_multisig_out(&op, &spent)?;
        }
    }
    let tickets: Vec<_> = tx.iter::<VspTickets>()?.collect::<Result<_, _>>()?;
    let host = tx.add_vsp_host(b"https://vsp.migrated.example", &[0x99; 32])?;
    for (t, row) in tickets {
        tx.put_vsp_ticket(
            &t,
            &VspTicketRow {
                fee_status: row.fee_status + 1,
                host_id: host,
                ..row
            },
        )?;
    }
    let first_policy = tx.first::<TSpendPolicies>()?.map(|(k, _)| k);
    if let Some(k) = first_policy {
        tx.delete::<TSpendPolicies>(&k)?;
    }
    tx.put::<AgendaPrefs>(
        &AgendaKey {
            version: 99,
            agenda: b"migrated".to_vec(),
        },
        &AgendaChoice(b"yes".to_vec()),
    )?;
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
    record_rows(tx, &mut rng, 40)?;
    tx.put_raw(TableId::TxMined, &rng.bytes(36), &rng.bytes(30_000))
}

/// `n` rows of each record table through the record operations: VSP
/// tickets over three hosts, multisig outputs over four scripts (some
/// spent, some unmined), the four treasury policies and the two agenda
/// preferences.
fn record_rows(tx: &mut WriteTx<'_>, rng: &mut SplitMix64, n: u32) -> purse_store::Result<()> {
    let hosts: Vec<u32> = (0..3)
        .map(|i| tx.add_vsp_host(format!("https://vsp{i}.example").as_bytes(), &rng.bytes(33)))
        .collect::<Result<_, _>>()?;
    for i in 0..n {
        let mut h = [0u8; 32];
        rng.fill(&mut h);
        let ticket = Hash32(h);
        tx.put_vsp_ticket(
            &ticket,
            &VspTicketRow {
                fee_hash: if i % 3 == 0 {
                    Hash32::default()
                } else {
                    Hash32([i as u8; 32])
                },
                fee_status: i % 4,
                host_id: hosts[i as usize % 3],
            },
        )?;
        let mut op = [0u8; 32];
        rng.fill(&mut op);
        tx.put_multisig_out(
            &purse_store::Outpoint {
                hash: Hash32(op),
                index: i % 5,
            },
            &MultisigOutRow {
                script_hash: Hash160([(i % 4) as u8; 20]),
                m: 2,
                n: 3,
                stake_tree: i % 7 == 0,
                block: (i % 5 != 0)
                    .then(|| MultisigBlock::new(Hash32([0xb1; 32]), i + 1))
                    .flatten(),
                amount: i64::from(i) * 1_000,
                spender: (i % 3 == 2).then_some(MultisigSpender {
                    hash: Hash32([0x5d; 32]),
                    input: i,
                }),
            },
        )?;
        let vote = TreasuryVote::new(1 + (i % 2) as u8).expect("non-zero");
        tx.put::<TSpendPolicies>(&ticket, &vote)?;
        tx.put::<TreasuryKeyPolicies>(&PiKey(rng.bytes(33)), &vote)?;
        tx.put::<VspTSpendPolicies>(
            &VspTSpendKey {
                ticket,
                tspend: Hash32([i as u8; 32]),
            },
            &vote,
        )?;
        tx.put::<VspTreasuryKeyPolicies>(
            &VspTreasuryKey {
                ticket,
                pikey: rng.bytes(33),
            },
            &vote,
        )?;
        tx.put::<AgendaPrefs>(
            &AgendaKey {
                version: 10 + i % 2,
                agenda: format!("agenda{i}").into_bytes(),
            },
            &AgendaChoice(b"no".to_vec()),
        )?;
        tx.put::<TicketAgendaPrefs>(
            &TicketAgendaKey {
                ticket,
                version: 10,
                agenda: b"treasury".to_vec(),
            },
            &AgendaChoice(b"abstain".to_vec()),
        )?;
    }
    Ok(())
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
        tx.check_records()
            .unwrap_or_else(|e| panic!("cut after {n}: {e}"));
        records_view(&tx).unwrap_or_else(|e| panic!("cut after {n}: {e}"));
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

/// A transaction of the version-1 fixture, serialized: inputs spending
/// `inputs`, and P2PKH outputs of `values` paying hashes of `fill` bytes;
/// with each output's script location as `AddCredit` records it (dcrwallet's
/// P2PKH script type code is 2).
fn fixture_tx(inputs: &[(Hash32, u32)], values: &[i64], fill: u8) -> (Vec<u8>, Vec<ScriptLoc>) {
    let msg = MsgTx {
        tx_in: inputs
            .iter()
            .map(|(hash, index)| {
                let mut prev = OutPoint::default();
                prev.hash.0 = hash.0;
                prev.index = *index;
                TxIn {
                    previous_out_point: prev,
                    sequence: u32::MAX,
                    ..TxIn::default()
                }
            })
            .collect(),
        tx_out: values
            .iter()
            .map(|value| TxOut {
                value: *value,
                version: 0,
                pk_script: [&[0x76, 0xa9, 0x14][..], &[fill; 20], &[0x88, 0xac]].concat(),
            })
            .collect(),
        ..MsgTx::default()
    };
    let locs = msg
        .pk_script_locs()
        .into_iter()
        .map(|offset| ScriptLoc {
            offset: u32::try_from(offset).expect("small"),
            len: 25,
            class: 2,
        })
        .collect();
    (msg.serialize(), locs)
}

/// The rows of the version-1 fixture: at least one valid row in every
/// table, written through the typed API, the chain, record and transaction
/// tables each passing their consistency check, so that the fixture (and a
/// backup of it) passes everything `verify_backup` checks.
fn fixture_rows(tx: &mut WriteTx<'_>) -> purse_store::Result<()> {
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
        // Block 2 disapproves block 1, whose two wallet transactions'
        // credits and debits are invalidated; block 2 holds two more.
        let row = BlockRow {
            hash,
            time: 1_700_000_000 + i64::from(height) * 300,
            vote_bits: u16::from(height != 2),
            invalid: height == 1,
            wallet_txs: if height == 0 { 0 } else { 2 },
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
    // The transaction tables, consistent (`ReadTx::check_tx_tables`): in
    // the disapproved block 1, record 0x7c pays 7 atoms that record 0x7d
    // spends, both rows invalidated; in block 2, record 0x7a pays three
    // outputs (account 0: 1,000,000 atoms that the unmined 0x7b spends, and
    // 2,000; account 1: 5,000, spent by record 0x79); the unpublished 0x7b
    // pays account 1 500 atoms; a ticket's commitment locks 2,000. Each
    // record holds a serialized transaction with those inputs and outputs,
    // so that `purse-udb`'s decoded check (`txstore::verify_backup`) passes
    // too; the records' keys are not their transactions' hashes, which no
    // check reads.
    let (t7c, s7c) = fixture_tx(&[(h32(0x60), 0)], &[7], 0x31);
    let (t7d, s7d) = fixture_tx(&[(h32(0x7c), 0)], &[6], 0x32);
    let (t7a, s7a) = fixture_tx(&[(h32(0x61), 0)], &[1_000_000, 5_000, 2_000], 0x33);
    let (t79, _) = fixture_tx(&[(h32(0x7a), 1)], &[4_000], 0x34);
    let (t7b, s7b) = fixture_tx(&[(h32(0x7a), 0)], &[990_000, 500], 0x35);
    let rec = |hash: Hash32, height: u32, position: u32, tx: Vec<u8>| {
        (
            TxRecKey { hash, height },
            HistoryKey { height, position },
            TxMinedRow {
                position,
                received: 1_700_000_000 + i64::from(height) * 300 + i64::from(position),
                tx,
            },
        )
    };
    for (key, at, row) in [
        rec(h32(0x7c), 1, 0, t7c),
        rec(h32(0x7d), 1, 1, t7d),
        rec(h32(0x7a), 2, 0, t7a),
        rec(h32(0x79), 2, 1, t79),
    ] {
        tx.put::<TxMined>(&key, &row)?;
        tx.put::<History>(&at, &key.hash)?;
    }
    tx.put::<TxUnmined>(
        &h32(0x7b),
        &TxUnminedRow {
            received: 1_700_000_200,
            tx: t7b,
        },
    )?;
    tx.put::<Unpublished>(&h32(0x7b), &UnpublishedFlag)?;
    let credit =
        |amount: i64, account: u32, script: ScriptLoc, spender: Option<Spender>| CreditRow {
            amount,
            flags,
            account,
            script,
            spender,
        };
    let ck = |hash: Hash32, height: u32, index: u32| CreditKey {
        hash,
        height,
        index,
    };
    let spender = |hash: Hash32, height: u32| Spender {
        hash,
        height,
        input: 0,
    };
    // Block 1, invalidated.
    tx.put::<CreditsInvalidated>(
        &ck(h32(0x7c), 1, 0),
        &credit(7, 0, s7c[0], Some(spender(h32(0x7d), 1))),
    )?;
    tx.put::<CreditsInvalidated>(&ck(h32(0x7d), 1, 0), &credit(6, 0, s7d[0], None))?;
    tx.put::<DebitsInvalidated>(
        &ck(h32(0x7d), 1, 0),
        &DebitRow {
            amount: 7,
            credit: ck(h32(0x7c), 1, 0),
        },
    )?;
    // Block 2.
    tx.put::<Credits>(&ck(h32(0x7a), 2, 0), &credit(1_000_000, 0, s7a[0], None))?;
    tx.put::<Credits>(
        &ck(h32(0x7a), 2, 1),
        &credit(5_000, 1, s7a[1], Some(spender(h32(0x79), 2))),
    )?;
    tx.put::<Credits>(&ck(h32(0x7a), 2, 2), &credit(2_000, 0, s7a[2], None))?;
    tx.put::<Debits>(
        &ck(h32(0x79), 2, 0),
        &DebitRow {
            amount: 5_000,
            credit: ck(h32(0x7a), 2, 1),
        },
    )?;
    for (index, amount, script) in [(0u32, 1_000_000i64, s7a[0]), (2, 2_000, s7a[2])] {
        tx.put::<Unspent>(
            &purse_store::Outpoint {
                hash: h32(0x7a),
                index,
            },
            &UnspentRow {
                height: 2,
                account: 0,
                amount,
                flags,
                class: BalanceClass::MinedRegular,
                script,
            },
        )?;
    }
    for (account, position, hash) in [(0u32, 0u32, 0x7a), (1, 0, 0x7a), (1, 1, 0x79)] {
        tx.put::<AcctHistory>(
            &AcctHistoryKey {
                account,
                height: 2,
                position,
            },
            &h32(hash),
        )?;
    }
    // The unmined transaction.
    tx.put::<UnminedInputs>(
        &purse_store::Outpoint {
            hash: h32(0x7a),
            index: 0,
        },
        &h32(0x7b),
    )?;
    tx.put::<UnminedCredits>(
        &purse_store::Outpoint {
            hash: h32(0x7b),
            index: 1,
        },
        &UnminedCreditRow {
            amount: 500,
            flags,
            account: 1,
            class: BalanceClass::UnminedRegularUnpublished,
            script: s7b[1],
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
    // The sums: 0x7a:0 is spent by an unmined transaction and counts in
    // none.
    for (account, class, sum) in [
        (0u32, BalanceClass::MinedRegular, 2_000i64),
        (1, BalanceClass::UnminedRegularUnpublished, 500),
        (0, BalanceClass::Locked, 2_000),
    ] {
        tx.put::<Balances>(
            &BalanceKey { account, class },
            &BalanceRow { sum, count: 1 },
        )?;
    }
    tx.put::<BalanceHeights>(
        &BalanceHeightKey {
            account: 0,
            class: BalanceClass::MinedRegular,
            height: 2,
        },
        &BalanceRow {
            sum: 2_000,
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
    })?;
    // The records Phase 2 adds: an unspent and a spent multisig output of
    // one script, two VSP tickets of one host, one of each policy and
    // preference.
    let script_hash = Hash160([0x33; 20]);
    for (i, spent) in [(0u32, false), (1, true)] {
        tx.put_multisig_out(
            &purse_store::Outpoint {
                hash: h32(0x7f),
                index: i,
            },
            &MultisigOutRow {
                script_hash,
                m: 1,
                n: 2,
                stake_tree: false,
                block: MultisigBlock::new(h32(0xb2), 2),
                amount: 300_000,
                spender: spent.then_some(MultisigSpender {
                    hash: h32(0x7b),
                    input: 1,
                }),
            },
        )?;
    }
    let host = tx.add_vsp_host(b"https://vsp.example.org", &[0x44; 32])?;
    assert_eq!(host, 1, "the first host's id");
    for (t, status) in [(0x7eu8, 0u32), (0x80, 3)] {
        tx.put_vsp_ticket(
            &h32(t),
            &VspTicketRow {
                fee_hash: h32(t ^ 0xff),
                fee_status: status,
                host_id: host,
            },
        )?;
    }
    tx.put::<TSpendPolicies>(&h32(0x90), &TreasuryVote::YES)?;
    tx.put::<TreasuryKeyPolicies>(&PiKey(vec![0x03; 33]), &TreasuryVote::NO)?;
    tx.put::<VspTSpendPolicies>(
        &VspTSpendKey {
            ticket: h32(0x7e),
            tspend: h32(0x90),
        },
        &TreasuryVote::NO,
    )?;
    tx.put::<VspTreasuryKeyPolicies>(
        &VspTreasuryKey {
            ticket: h32(0x7e),
            pikey: vec![0x03; 33],
        },
        &TreasuryVote::YES,
    )?;
    tx.put::<AgendaPrefs>(
        &AgendaKey {
            version: 11,
            agenda: b"maxblocksize".to_vec(),
        },
        &AgendaChoice(b"yes".to_vec()),
    )?;
    tx.put::<TicketAgendaPrefs>(
        &TicketAgendaKey {
            ticket: h32(0x7e),
            version: 11,
            agenda: b"maxblocksize".to_vec(),
        },
        &AgendaChoice(b"no".to_vec()),
    )
}

/// The checked-in store of each released schema version, and its SHA-256:
/// a fixture is never regenerated once released, so a changed file is a
/// mistake.
const FIXTURES: &[(u32, &str, &str)] = &[(
    1,
    "schema-v1.redb",
    "971b62207e4ea9415a1d044304a41e6bdccfe4ab94c04e3d46fe6d1e4a603457",
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
        (TableId::MultisigOuts, count::<MultisigOuts>(tx)),
        (TableId::MultisigUnspent, count::<MultisigUnspent>(tx)),
        (TableId::VspTickets, count::<VspTickets>(tx)),
        (TableId::VspFeeStatus, count::<VspFeeStatus>(tx)),
        (TableId::VspHosts, count::<VspHosts>(tx)),
        (TableId::VspPubKeys, count::<VspPubKeys>(tx)),
        (TableId::TSpendPolicies, count::<TSpendPolicies>(tx)),
        (
            TableId::TreasuryKeyPolicies,
            count::<TreasuryKeyPolicies>(tx),
        ),
        (TableId::VspTSpendPolicies, count::<VspTSpendPolicies>(tx)),
        (
            TableId::VspTreasuryKeyPolicies,
            count::<VspTreasuryKeyPolicies>(tx),
        ),
        (TableId::AgendaPrefs, count::<AgendaPrefs>(tx)),
        (TableId::TicketAgendaPrefs, count::<TicketAgendaPrefs>(tx)),
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
    tx.check_records()
        .expect("the record tables hold their invariants");
    records_view(tx).expect("the record tables pass the independent check");
    tx.check_tx_tables()
        .expect("the transaction tables hold their invariants");
    assert_eq!(
        tx.unspent_multisig_for(&Hash160([0x33; 20]))
            .expect("multisig"),
        [purse_store::Outpoint {
            hash: h32(0x7f),
            index: 0
        }]
    );
    assert_eq!(
        tx.vsp_tickets_with_fee_status(3).expect("by status"),
        [h32(0x80)]
    );
    assert!(tx.processed_marker().expect("marker").is_some());
    assert!(tx.birth_state().expect("birth").is_some());
    assert!(
        tx.wallet_row::<wallet::Slip0044Account0>()
            .expect("acct0")
            .is_some()
    );
}

/// `ReadTx::check_tx_tables` in this crate (ADR-0018 BK4): on the
/// fixture's rows, which pass, one direct row edit per invariant of
/// [`purse_store::txtables`]'s list is refused with that invariant's text,
/// and the rows pass again once the edit is rolled back. `purse-udb`'s
/// tests break every sub-rule on a store its operations build
/// (`each_broken_invariant_of_the_transaction_tables_is_refused`); this
/// test holds the store crate's verify to refuse at least one edit of each
/// invariant on its own.
#[test]
fn the_transaction_tables_check_refuses_each_broken_invariant() {
    type Edit = fn(&mut WriteTx<'_>) -> purse_store::Result<()>;
    let cases: [(&str, &str, Edit); 13] = [
        (
            "1",
            "block 2 counts 3 wallet transactions and has 2 history rows",
            |tx| {
                let mut b = tx.get::<Blocks>(&purse_store::Height(2))?.expect("block");
                b.wallet_txs += 1;
                tx.put::<Blocks>(&purse_store::Height(2), &b)
            },
        ),
        ("2", "record ", |tx| {
            let k = TxRecKey {
                hash: h32(0x7a),
                height: 2,
            };
            let mut m = tx.get::<TxMined>(&k)?.expect("record");
            m.position = 5;
            tx.put::<TxMined>(&k, &m)
        }),
        ("3", "unmined input ", |tx| {
            let o = purse_store::Outpoint {
                hash: h32(0xdd),
                index: 3,
            };
            tx.put::<UnminedInputs>(&o, &h32(0xee))
        }),
        ("4", "unmined credit ", |tx| {
            tx.delete::<Unpublished>(&h32(0x7b)).map(drop)
        }),
        ("5", "unspent output ", |tx| {
            let o = purse_store::Outpoint {
                hash: h32(0x7a),
                index: 2,
            };
            let mut u = tx.get::<Unspent>(&o)?.expect("unspent");
            u.amount += 1;
            tx.put::<Unspent>(&o, &u)
        }),
        ("6", "credits row ", |tx| {
            let k = CreditKey {
                hash: h32(0x7a),
                height: 2,
                index: 2,
            };
            let c = tx.get::<Credits>(&k)?.expect("credit");
            tx.put::<Credits>(
                &CreditKey {
                    hash: h32(0xee),
                    ..k
                },
                &c,
            )
        }),
        ("7", "debits row ", |tx| {
            let k = CreditKey {
                hash: h32(0x79),
                height: 2,
                index: 0,
            };
            let mut d = tx.get::<Debits>(&k)?.expect("debit");
            d.amount += 1;
            tx.put::<Debits>(&k, &d)
        }),
        ("8", "credits_invalidated row ", |tx| {
            // 0x79's debit spends 0x7a:1, not 0x7c:0.
            let k = CreditKey {
                hash: h32(0x7c),
                height: 1,
                index: 0,
            };
            let mut c = tx.get::<CreditsInvalidated>(&k)?.expect("credit");
            c.spender = Some(Spender {
                hash: h32(0x79),
                height: 2,
                input: 0,
            });
            tx.put::<CreditsInvalidated>(&k, &c)
        }),
        ("9", "credits_invalidated row ", |tx| {
            let mut b = tx.get::<Blocks>(&purse_store::Height(1))?.expect("block");
            b.invalid = false;
            tx.put::<Blocks>(&purse_store::Height(1), &b)
        }),
        ("10", "unspent commitment entry ", |tx| {
            let o = purse_store::Outpoint {
                hash: h32(0xee),
                index: 1,
            };
            tx.put::<CommitmentsUnspent>(
                &o,
                &CommitmentUnspentRow {
                    unmined_spent: false,
                },
            )
        }),
        ("11", "the balance of account 0 class MinedRegular ", |tx| {
            let k = BalanceKey {
                account: 0,
                class: BalanceClass::MinedRegular,
            };
            let mut row = tx.get::<Balances>(&k)?.expect("balance");
            row.sum += 1;
            tx.put::<Balances>(&k, &row)
        }),
        (
            "12",
            "account 0 class MinedRegular has no balance row at height 2",
            |tx| {
                let k = BalanceHeightKey {
                    account: 0,
                    class: BalanceClass::MinedRegular,
                    height: 2,
                };
                tx.delete::<BalanceHeights>(&k).map(drop)
            },
        ),
        ("13", "record ", |tx| {
            let k = AcctHistoryKey {
                account: 1,
                height: 2,
                position: 1,
            };
            tx.delete::<AcctHistory>(&k).map(drop)
        }),
    ];
    let store = OpenOptions::new()
        .create_on_backend(Shared(PowerLoss::new(Vec::new(), true)), fixture_rows)
        .expect("create");
    store
        .read()
        .unwrap()
        .check_tx_tables()
        .expect("the fixture");
    for (n, text, edit) in cases {
        let undone = purse_store::Error::inconsistent("undone".to_owned());
        let r = store.update(|tx| {
            tx.check_tx_tables()?;
            edit(tx)?;
            let e = tx
                .check_tx_tables()
                .expect_err(&format!("invariant {n}: not refused"));
            let why = e.inconsistency().expect("an inconsistency");
            assert!(why.starts_with(text), "invariant {n}: refused as {why:?}");
            Err::<(), _>(purse_store::Error::inconsistent("undone".to_owned()))
        });
        assert_eq!(
            r.expect_err("rolled back").inconsistency(),
            undone.inconsistency()
        );
        store
            .read()
            .unwrap()
            .check_tx_tables()
            .unwrap_or_else(|e| panic!("invariant {n}: refused once undone: {e}"));
    }
}

/// `WriteTx::check_rows`, `check_chain`, `check_records` and
/// `check_tx_tables` check a write transaction's own uncommitted writes
/// (the operation-script runner of `purse-udb` runs them after every
/// member of an `update` group, before the group commits or rolls back):
/// on the fixture's rows all four pass inside a transaction; after one
/// direct edit per check, that check refuses inside the transaction while
/// a read transaction opened beside it, which sees the committed store,
/// passes; and the store passes again once the edit is rolled back.
#[test]
fn the_write_transaction_checks_see_its_own_writes() {
    type Check = fn(&mut WriteTx<'_>) -> purse_store::Result<()>;
    type Edit = fn(&mut WriteTx<'_>) -> purse_store::Result<()>;
    let cases: [(&str, Check, Edit); 4] = [
        (
            "rows",
            |tx| tx.check_rows(),
            |tx| tx.put_raw(TableId::Accounts, b"not an account key", b"x"),
        ),
        (
            "chain",
            |tx| tx.check_chain(),
            |tx| tx.delete::<Headers>(&purse_store::Height(1)).map(drop),
        ),
        (
            "records",
            |tx| tx.check_records(),
            |tx| {
                tx.put::<VspHosts>(
                    &purse_store::VspHostId(0xffff),
                    &purse_store::VspHost(b"unpaired".to_vec()),
                )
            },
        ),
        (
            "transaction tables",
            |tx| tx.check_tx_tables(),
            |tx| {
                let k = AcctHistoryKey {
                    account: 1,
                    height: 2,
                    position: 1,
                };
                tx.delete::<AcctHistory>(&k).map(drop)
            },
        ),
    ];
    let store = OpenOptions::new()
        .create_on_backend(Shared(PowerLoss::new(Vec::new(), true)), fixture_rows)
        .expect("create");
    let committed = |store: &Store| -> purse_store::Result<()> {
        let tx = store.read()?;
        tx.check_rows()?;
        tx.check_chain()?;
        tx.check_records()?;
        tx.check_tx_tables()
    };
    committed(&store).expect("the fixture");
    for (name, check, edit) in cases {
        let undone = purse_store::Error::inconsistent("undone".to_owned());
        let r = store.update(|tx| {
            tx.check_rows()?;
            tx.check_chain()?;
            tx.check_records()?;
            tx.check_tx_tables()?;
            edit(tx)?;
            check(tx).expect_err(&format!("{name}: the edit is not refused"));
            committed(&store)
                .unwrap_or_else(|e| panic!("{name}: the committed store refused: {e}"));
            Err::<(), _>(purse_store::Error::inconsistent("undone".to_owned()))
        });
        assert_eq!(
            r.expect_err("rolled back").inconsistency(),
            undone.inconsistency()
        );
        committed(&store).unwrap_or_else(|e| panic!("{name}: refused once undone: {e}"));
    }
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
