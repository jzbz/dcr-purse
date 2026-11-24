// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The transaction store without the oracle: the upstream tests, and the
//! property tests.
//!
//! Upstream (`wallet/udb` at the parity pin):
//!
//! - `TestSetBirthState` (`txmined_test.go:32-80`), with the random state
//!   drawn from this crate's seeded generator;
//! - `TestStakeInvalidationOfTip` (`stakevalidation_test.go:42-187`):
//!   `txcommon_test.go`'s header chain from the testnet3 genesis block
//!   (`blockGenerator`, `emptyFilters`; the zero `time.Time` serialized as
//!   wire does, and block 1's hash pinned to Go's) connected with
//!   `ExtendMainChain`, the two transactions inserted unmined with their
//!   credits and then mined, and the third block's vote bits invalidating
//!   the second, with Go's balance and unspent-output assertions;
//! - `TestInsertsCreditsDebitsRollbacks` (`tx_test.go:20-351`) and
//!   `TestCoinbases` (`tx_test.go:353-574`), with Go's assertions (and, for
//!   `TestCoinbases`, the spendable balance its table states and its loop
//!   never compares). These three run Rust only: Go runs them itself, and
//!   the oracle compares the same operations in scripts.
//!
//! Properties (random sequences from `PURSE_TEST_SEED`, over every
//! operation of the slice on a store with no address manager, which the
//! transaction store never reads): the rows' invariants after every
//! operation (`check_rows`, and `purse-store`'s `check_chain`); refusals
//! writing nothing; a reopened store answering as the live one did;
//! `TxRecord` round trips; the dependency order of `unmined_txs`; the
//! guard of the later slices' tables; the rows the range walks read, the
//! values the existence tests do not read, and the block rows
//! `IterateTickets` reads (one per block, not per ticket). Two targeted
//! tests reach states no random sequence does: a ticket's live debit whose
//! credit `AddCredit` overwrote in an invalidated block (`acct_history`'s
//! spender rule), and `UnspentTickets` at Go's maturity boundary, at its
//! `int32` wrap (QK-0784) and over a dangling unmined-inputs entry
//! (QK-0048).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

mod support;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use dcroxide_chainhash::Hash;
use dcroxide_gcs::{FilterV2, blockcf2};
use dcroxide_stake::TxType;
use dcroxide_wire::{BlockHeader, MsgTx, OutPoint, TxIn, TxOut};
use purse_errors::Kind;
use purse_store::{
    AcctHistory, AcctHistoryKey, BalanceClass, BalanceHeights, BalanceRow, Balances, BirthState,
    Blocks, CreditKey, Credits, CreditsInvalidated, Debits, Hash32, HeaderBytes, History, ReadTx,
    Store, Table, TableId, Tickets, Tip, TxMined, TxUnmined, UnminedCredits, UnminedInputs,
    Unpublished, Unspent, WriteTx,
};
use purse_testutil::SplitMix64;
use purse_udb::addrmgr::ChainParams;
use purse_udb::txstore::{
    BlockMeta, LATER_TABLES, TxRead, TxRecord, TxStore, mined_class, unmined_class,
};
use support::scratch;

/// The error of a test's write transaction: the store's, or a deliberate
/// rollback.
#[derive(Debug)]
enum TErr {
    Store(#[allow(dead_code, reason = "printed by Debug")] purse_store::Error),
    Rollback,
}

impl From<purse_store::Error> for TErr {
    fn from(e: purse_store::Error) -> Self {
        TErr::Store(e)
    }
}

/// A store holding only the transaction store's rows.
fn tx_store(net: &str) -> (Store, TxStore, std::path::PathBuf) {
    let dir = scratch("txstore");
    let path = dir.join(purse_store::STORE_FILE_NAME);
    let store = Store::create(&path).expect("create store");
    let ts = TxStore::new(&ChainParams::by_name(net).expect("network")).expect("TxStore");
    store
        .update(|tx| {
            ts.create(tx).expect("create the transaction store");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    (store, ts, path)
}

fn header_bytes(h: &BlockHeader) -> HeaderBytes {
    HeaderBytes(h.serialize())
}

/// The timestamp field of a header whose Go `Timestamp` is the zero
/// `time.Time`: wire writes `uint32(Timestamp.Unix())`, and the zero
/// time's Unix seconds, −62135596800 (`ZERO_TIME`), wrap to 2288912640.
const GO_ZERO_TIMESTAMP: u32 = 2_288_912_640;

/// Block 1 of `txcommon_test.go`'s chain (`blockGenerator`, vote bits 1),
/// as Go's `BlockHash` prints it at the parity pin.
const GO_BLOCK1_HASH: &str = "87cb586fd226a0c335267af514acb56e527a9c92ec56b1b98eee611dddd7c6ba";

/// `txcommon_test.go`'s `blockGenerator`: headers with only the parent,
/// the vote bits and the height set, from the testnet3 genesis block; the
/// timestamp is Go's zero time as wire serializes it.
struct BlockGenerator {
    last_hash: Hash,
    last_height: u32,
}

impl BlockGenerator {
    fn new() -> BlockGenerator {
        BlockGenerator {
            last_hash: dcroxide_chaincfg::testnet3_params().genesis_hash,
            last_height: 0,
        }
    }

    fn generate(&mut self, vote_bits: u16) -> BlockHeader {
        let h = BlockHeader {
            version: 0,
            prev_block: self.last_hash,
            merkle_root: Hash::ZERO,
            stake_root: Hash::ZERO,
            vote_bits,
            final_state: [0; 6],
            voters: 0,
            fresh_stake: 0,
            revocations: 0,
            pool_size: 0,
            bits: 0,
            sbits: 0,
            height: self.last_height + 1,
            size: 0,
            timestamp: GO_ZERO_TIMESTAMP,
            nonce: 0,
            extra_data: [0; 32],
            stake_version: 0,
        };
        self.last_hash = h.block_hash();
        self.last_height += 1;
        h
    }
}

/// `insertMainChainHeaders` with `emptyFilters`: each header extends the
/// main chain with an empty filter.
fn insert_main_chain_headers(ts: &TxStore, tx: &mut WriteTx<'_>, headers: &[&BlockHeader]) {
    let empty = FilterV2::from_bytes(blockcf2::B, blockcf2::M, &[]).expect("empty filter");
    for h in headers {
        let hash = Hash32(*h.block_hash().as_bytes());
        ts.extend_main_chain(tx, &header_bytes(h), &hash, Some(empty.bytes()))
            .expect("ExtendMainChain");
    }
}

/// `TestSetBirthState`.
#[test]
fn set_birth_state_round_trips() {
    let mut rng = SplitMix64::from_env("purse-udb-txstore-birth");
    let (store, _, _) = tx_store("simnet");
    for _ in 0..16 {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&rng.bytes(32));
        let want = BirthState {
            set_from_height: rng.below(2) == 0,
            set_from_time: rng.below(2) == 0,
            hash: Hash32(hash),
            height: u32::try_from(rng.below(100_000)).expect("small"),
            time: i64::try_from(rng.below(100_000_000)).expect("small"),
        };
        store
            .update(|tx| {
                purse_udb::txstore::set_birth_state(tx, &want).expect("SetBirthState");
                Ok::<_, purse_store::Error>(())
            })
            .expect("update");
        let mut r = store.read().expect("read");
        let got = purse_udb::txstore::birth_state(&mut r)
            .expect("BirthState")
            .expect("a state");
        assert_eq!(got, want);
    }
}

/// A block meta as `txcommon_test.go`'s `makeBlockMeta` builds it: the
/// header's hash and height, Go's zero time, vote bits 0.
fn make_block_meta(h: &BlockHeader) -> BlockMeta {
    BlockMeta {
        block: purse_udb::txstore::Block {
            hash: Hash32(*h.block_hash().as_bytes()),
            height: i32::try_from(h.height).expect("small"),
        },
        time: purse_udb::txstore::ZERO_TIME,
        vote_bits: 0,
    }
}

/// A transaction with no input and outputs of `values`, as the upstream
/// tests write `wire.MsgTx{TxOut: ...}`.
fn outputs_tx(values: &[i64]) -> MsgTx {
    MsgTx {
        tx_out: values
            .iter()
            .map(|v| TxOut {
                value: *v,
                version: 0,
                pk_script: Vec::new(),
            })
            .collect(),
        ..MsgTx::default()
    }
}

/// `spendOutput` (`tx_test.go:385-397`): one input spending `hash:index`
/// of `tree`, outputs of `values`.
fn spend_output(hash: Hash, index: u32, tree: i8, values: &[i64]) -> MsgTx {
    MsgTx {
        tx_in: vec![TxIn {
            previous_out_point: OutPoint { hash, index, tree },
            ..TxIn::default()
        }],
        ..outputs_tx(values)
    }
}

/// The outpoints of `UnspentOutputs`, with their trees.
fn unspent_set(ts: &TxStore, tx: &mut WriteTx<'_>) -> BTreeSet<(Hash32, u32, i8)> {
    ts.unspent_outputs(tx)
        .expect("UnspentOutputs")
        .iter()
        .map(|c| (Hash32(c.outpoint.hash.0), c.outpoint.index, c.outpoint.tree))
        .collect()
}

/// `TestStakeInvalidationOfTip` (`stakevalidation_test.go:42-187`):
/// `txcommon_test.go`'s header chain from the testnet3 genesis block, two
/// transactions inserted unmined with a credit each (the second spending
/// the first's), mined in blocks 1 and 2, and the third block's vote bits
/// invalidating the second: the balances and unspent outputs before and
/// after.
#[test]
fn stake_invalidation_of_tip() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let block1 = g.generate(1);
    let block2 = g.generate(1);
    let block3 = g.generate(0);
    assert_eq!(
        i64::from(GO_ZERO_TIMESTAMP),
        purse_udb::txstore::ZERO_TIME.rem_euclid(1 << 32)
    );
    // The chain is Go's: block 1's hash is the one the upstream test's
    // generator makes.
    assert_eq!(block1.block_hash().to_string(), GO_BLOCK1_HASH);
    let block1_tx = outputs_tx(&[200_000_000]);
    let block2_tx = spend_output(block1_tx.tx_hash(), 0, 0, &[100_000_000]);
    let rec1 = TxRecord::from_msg_tx(block1_tx, purse_udb::txstore::ZERO_TIME).expect("record");
    let rec2 = TxRecord::from_msg_tx(block2_tx, purse_udb::txstore::ZERO_TIME).expect("record");
    let h1 = Hash32(*block1.block_hash().as_bytes());
    let h2 = Hash32(*block2.block_hash().as_bytes());
    store
        .update(|tx| {
            ts.insert_mem_pool_tx(tx, &rec1).expect("InsertMemPoolTx");
            ts.add_credit(tx, &rec1, None, 0, false, 0)
                .expect("AddCredit");
            ts.insert_mem_pool_tx(tx, &rec2).expect("InsertMemPoolTx");
            ts.add_credit(tx, &rec2, None, 0, false, 0)
                .expect("AddCredit");
            let bal = ts.account_balance(tx, 0, 0).expect("AccountBalance");
            assert_eq!(bal.total, 100_000_000, "before mining either: {bal:?}");

            insert_main_chain_headers(&ts, tx, &[&block1, &block2]);
            ts.insert_mined_tx(tx, &rec1, &h1).expect("InsertMinedTx");
            ts.insert_mined_tx(tx, &rec2, &h2).expect("InsertMinedTx");
            // Only the credit of block 2's transaction is unspent.
            let bal = ts.account_balance(tx, 1, 0).expect("AccountBalance");
            assert_eq!(bal.total, 100_000_000, "{bal:?}");
            let credits = ts.unspent_outputs(tx).expect("UnspentOutputs");
            assert_eq!(credits.len(), 1);
            assert_eq!(Hash32(credits[0].outpoint.hash.0), *rec2.hash());
            assert_eq!(credits[0].amount, 100_000_000);
            assert_eq!(
                ts.block_in_main_chain(tx, &h2).expect("query"),
                (true, false)
            );

            // The next header invalidates block 2's regular tree: only the
            // output of block 1's transaction is unspent.
            insert_main_chain_headers(&ts, tx, &[&block3]);
            let bal = ts.account_balance(tx, 1, 0).expect("AccountBalance");
            assert_eq!(bal.total, 200_000_000, "{bal:?}");
            let credits = ts.unspent_outputs(tx).expect("UnspentOutputs");
            assert_eq!(credits.len(), 1);
            assert_eq!(Hash32(credits[0].outpoint.hash.0), *rec1.hash());
            assert_eq!(credits[0].amount, 200_000_000);
            // Beyond Go's assertions: the flag, the records left in their
            // blocks, the block row's time (Go's zero time as wire wrote
            // it) and the row invariants.
            assert_eq!(
                ts.block_in_main_chain(tx, &h2).expect("query"),
                (true, true)
            );
            assert_eq!(ts.tx_block_height(tx, rec2.hash()).expect("height"), 2);
            let d = ts.tx_details(tx, rec2.hash()).expect("TxDetails");
            assert_eq!(d.block.block.hash, h2);
            assert_eq!(d.block.time, i64::from(GO_ZERO_TIMESTAMP));
            assert!(ts.unmined_tx_hashes(tx).expect("hashes").is_empty());
            check_rows(tx);
            check_utxo_queries(&ts, tx);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// `TestInsertsCreditsDebitsRollbacks` (`tx_test.go:20-351`): a credit
/// inserted unmined, mined, rolled back and mined again, spent by a mined
/// transaction that a rollback returns to the unmined set, and an unmined
/// transaction spending an output the wallet never saw; after each step,
/// in its own transaction, the spendable and unconfirmed balances at one
/// confirmation, the unspent outputs and the unmined transactions.
#[test]
fn inserts_credits_debits_rollbacks() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let b1 = g.generate(1);
    let b2 = g.generate(1);
    let b3 = g.generate(1);
    let (b1h, b2h) = (
        Hash32(*b1.block_hash().as_bytes()),
        Hash32(*b2.block_hash().as_bytes()),
    );
    let tx1 = outputs_tx(&[200_000_000]);
    let tx1_rec =
        TxRecord::from_msg_tx(tx1.clone(), purse_udb::txstore::ZERO_TIME).expect("record");
    let mut s_tx1 = spend_output(tx1.tx_hash(), 0, 0, &[200_000_000]);
    s_tx1.tx_in[0].value_in = 200_000_000;
    s_tx1.tx_in[0].block_height = b2.height;
    s_tx1.tx_in[0].block_index = 0;
    let s_tx1_rec = TxRecord::from_msg_tx(s_tx1, purse_udb::txstore::ZERO_TIME).expect("record");
    let tx2 = outputs_tx(&[300_000_000]);
    let mut s_tx2 = spend_output(tx2.tx_hash(), 0, 0, &[300_000_000]);
    s_tx2.tx_in[0].value_in = 300_000_000;
    s_tx2.tx_in[0].block_height = b3.height;
    s_tx2.tx_in[0].block_index = 0;
    let s_tx2_rec = TxRecord::from_msg_tx(s_tx2, purse_udb::txstore::ZERO_TIME).expect("record");
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&b1, &b2, &b3]);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");

    // A step: its name, what it does, then the spendable and unconfirmed
    // balances, the unspent outputs and the unmined transactions after it.
    type Step<'a> = (
        &'static str,
        Box<dyn Fn(&mut WriteTx<'_>) + 'a>,
        i64,
        i64,
        Vec<(Hash32, u32, i8)>,
        Vec<Hash32>,
    );
    let tx1_out = (*tx1_rec.hash(), 0u32, 0i8);
    let steps: Vec<Step<'_>> = vec![
        ("new store", Box::new(|_| {}), 0, 0, vec![], vec![]),
        (
            "txout insert",
            Box::new(|tx| {
                ts.insert_mem_pool_tx(tx, &tx1_rec)
                    .expect("InsertMemPoolTx");
                ts.add_credit(tx, &tx1_rec, None, 0, false, 0)
                    .expect("AddCredit");
            }),
            0,
            200_000_000,
            vec![tx1_out],
            vec![*tx1_rec.hash()],
        ),
        (
            "insert duplicate unconfirmed",
            Box::new(|tx| {
                ts.insert_mem_pool_tx(tx, &tx1_rec)
                    .expect("InsertMemPoolTx");
            }),
            0,
            200_000_000,
            vec![tx1_out],
            vec![*tx1_rec.hash()],
        ),
        (
            "confirmed txout insert",
            Box::new(|tx| {
                ts.insert_mined_tx(tx, &tx1_rec, &b1h)
                    .expect("InsertMinedTx");
            }),
            200_000_000,
            0,
            vec![tx1_out],
            vec![],
        ),
        (
            "rollback confirmed credit",
            Box::new(|tx| {
                ts.rollback(tx, 1).expect("Rollback");
            }),
            0,
            200_000_000,
            vec![tx1_out],
            vec![*tx1_rec.hash()],
        ),
        (
            "insert duplicate confirmed",
            Box::new(|tx| {
                insert_main_chain_headers(&ts, tx, &[&b1, &b2, &b3]);
                // Go ignores this insert's error.
                let _ = ts.insert_mined_tx(tx, &tx1_rec, &b1h);
            }),
            200_000_000,
            0,
            vec![tx1_out],
            vec![],
        ),
        (
            "insert confirmed double spend",
            Box::new(|tx| {
                ts.insert_mined_tx(tx, &s_tx1_rec, &b2h)
                    .expect("InsertMinedTx");
                ts.insert_mined_tx(tx, &s_tx1_rec, &b2h)
                    .expect("InsertMinedTx");
            }),
            0,
            0,
            vec![],
            vec![],
        ),
        (
            "rollback after spending tx",
            Box::new(|tx| {
                ts.rollback(tx, 2).expect("Rollback");
            }),
            0,
            0,
            vec![],
            vec![*s_tx1_rec.hash()],
        ),
        (
            "insert unconfirmed debit",
            Box::new(|tx| {
                ts.insert_mem_pool_tx(tx, &s_tx2_rec)
                    .expect("InsertMemPoolTx");
            }),
            0,
            0,
            vec![],
            vec![*s_tx1_rec.hash(), *s_tx2_rec.hash()],
        ),
    ];
    for (name, f, bal, unc, unspents, unmined) in &steps {
        store
            .update(|tx| {
                f(tx);
                let b = ts.account_balance(tx, 1, 0).expect("AccountBalance");
                assert_eq!(b.spendable, *bal, "{name}: balance");
                assert_eq!(b.unconfirmed, *unc, "{name}: unconfirmed balance");
                assert_eq!(
                    unspent_set(&ts, tx),
                    unspents.iter().copied().collect(),
                    "{name}: unspent outputs"
                );
                let got: BTreeSet<Hash32> = ts
                    .unmined_txs(tx)
                    .expect("UnminedTxs")
                    .iter()
                    .map(|r| *r.hash())
                    .collect();
                assert_eq!(
                    got,
                    unmined.iter().copied().collect(),
                    "{name}: unmined transactions"
                );
                check_rows(tx);
                check_utxo_queries(&ts, tx);
                Ok::<_, purse_store::Error>(())
            })
            .expect("update");
    }
}

/// `TestCoinbases` (`tx_test.go:353-574`): a coinbase with two credits
/// in block 1 of testnet3, immature until its 16-block maturity is
/// reached at block 17, partly spent there, and the immature amount after
/// rolling back block 17 and then block 1. Go's table also states the
/// spendable balance, which its loop never compares (it checks the
/// immature amount twice); the port compares both.
#[test]
fn coinbases() {
    let (store, ts, _) = tx_store("testnet3");
    let cb = MsgTx {
        tx_in: vec![TxIn {
            previous_out_point: OutPoint {
                hash: Hash::ZERO,
                index: u32::MAX,
                tree: 0,
            },
            ..TxIn::default()
        }],
        ..outputs_tx(&[2_000_000_000, 1_000_000_000, 3_000_000_000])
    };
    let cb_rec = TxRecord::from_msg_tx(cb, purse_udb::txstore::ZERO_TIME).expect("record");
    let mut g = BlockGenerator::new();
    let headers: Vec<BlockHeader> = (0..19).map(|_| g.generate(1)).collect();
    let b1_meta = make_block_meta(&headers[0]);
    let b1h = Hash32(*headers[0].block_hash().as_bytes());
    store
        .update(|tx| {
            let hs: Vec<&BlockHeader> = headers.iter().collect();
            insert_main_chain_headers(&ts, tx, &hs[0..1]);
            ts.insert_mined_tx(tx, &cb_rec, &b1h)
                .expect("InsertMinedTx");
            ts.add_credit(tx, &cb_rec, Some(&b1_meta), 0, false, 0)
                .expect("AddCredit");
            ts.add_credit(tx, &cb_rec, Some(&b1_meta), 2, false, 0)
                .expect("AddCredit");
            let maturity = |tx: &mut WriteTx<'_>, step: &str, immature: i64, spendable: i64| {
                let b = ts.account_balance(tx, 0, 0).expect("AccountBalance");
                assert_eq!(b.immature_coinbase_rewards, immature, "{step}: immature");
                assert_eq!(b.spendable, spendable, "{step}: spendable");
                check_rows(tx);
                check_utxo_queries(&ts, tx);
            };
            maturity(tx, "block 1", 5_000_000_000, 0);
            insert_main_chain_headers(&ts, tx, &hs[1..7]);
            maturity(tx, "block 7", 5_000_000_000, 0);
            insert_main_chain_headers(&ts, tx, &hs[7..13]);
            maturity(tx, "block 13", 5_000_000_000, 0);
            insert_main_chain_headers(&ts, tx, &hs[13..16]);
            maturity(tx, "block 16", 5_000_000_000, 0);
            insert_main_chain_headers(&ts, tx, &hs[16..17]);
            maturity(tx, "block 17", 0, 5_000_000_000);
            // Spending output 0 leaves output 2 mature.
            let spender = spend_output(Hash(cb_rec.hash().0), 0, 0, &[500_000_000, 1_500_000_000]);
            let spender_rec = TxRecord::from_msg_tx(spender, 1_795_300_000).expect("record");
            let b17h = Hash32(*headers[16].block_hash().as_bytes());
            ts.insert_mined_tx(tx, &spender_rec, &b17h)
                .expect("InsertMinedTx");
            maturity(tx, "spent at block 17", 0, 3_000_000_000);
            // Block 17 rolled back: output 2 immature again, output 0
            // spent by the returned (unmined) spender.
            ts.rollback(tx, 17).expect("Rollback");
            maturity(tx, "block 17 rolled back", 3_000_000_000, 0);
            // Block 1 rolled back: the coinbase is gone.
            ts.rollback(tx, 1).expect("Rollback");
            maturity(tx, "block 1 rolled back", 0, 0);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// ADR-0019 FB4, FB9 and FB10, which no operation script reaches (Go's
/// `ExtendMainChain` always takes a filter): below the birthday height a
/// block is connected without its filter, at or above it one is refused
/// without writing; `MissingCFiltersHeight` walks from the birthday height,
/// the birth row's height when the birthday is resolved or pending from a
/// height (pending time birthdays: the next test); the readers of
/// filters answer `NotExist` for a block without one, as Go answers for a
/// missing filter.
#[test]
fn filters_below_the_birthday_follow_adr_0019() {
    let (store, ts, _) = tx_store("simnet");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-birthday"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    let birth = |height: u32, from_height: bool| BirthState {
        set_from_height: from_height,
        set_from_time: false,
        hash: Hash32([0x42; 32]),
        height,
        time: 0,
    };
    store
        .update(|tx| {
            // No birth row: the birthday height is 0, so every block needs
            // its filter.
            assert!(tx.delete_raw(TableId::Meta, b"birth").expect("delete"));
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            let e = ts
                .extend_main_chain(tx, &h, &hash, None)
                .expect_err("refused");
            assert_eq!(e.as_kind(), Some(Kind::Invalid), "{e:?}");
            assert_eq!(dump(tx), before, "a refusal wrote");
            // Birthday height 3 (a pending height birthday): blocks 1 and 2
            // without filters, 3 refused without one, taken with one.
            purse_udb::txstore::set_birth_state(tx, &birth(3, true)).expect("birth");
            for want in 1..=2 {
                let (h, hash) = sim.header(&ts, tx);
                ts.extend_main_chain(tx, &h, &hash, None)
                    .expect("below the birthday");
                sim.blocks.push(hash);
                assert_eq!(ts.main_chain_tip(tx).expect("tip").1, want);
            }
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            let e = ts
                .extend_main_chain(tx, &h, &hash, None)
                .expect_err("at the birthday");
            assert_eq!(e.as_kind(), Some(Kind::Invalid), "{e:?}");
            assert_eq!(dump(tx), before, "a refusal wrote");
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("with its filter");
            sim.blocks.push(hash);
            // Nothing is missing at or above the birthday height.
            let e = ts.missing_cfilters_height(tx).expect_err("none missing");
            assert_eq!(e.as_kind(), Some(Kind::NotExist), "{e:?}");
            assert!(!ts.is_missing_main_chain_cfilters(tx).expect("flag"));
            // The readers of a filterless block's filter: Go's NotExist.
            let b1 = sim.blocks[0];
            let e = ts.cfilter_v2(tx, &b1).expect_err("no filter");
            assert_eq!(e.as_kind(), Some(Kind::NotExist), "{e:?}");
            let genesis = ts
                .get_main_chain_block_hash_for_height(tx, 0)
                .expect("genesis");
            let e = ts
                .get_main_chain_cfilters(tx, &genesis, true, 4)
                .expect_err("block 1 has none");
            assert_eq!(e.as_kind(), Some(Kind::NotExist), "{e:?}");
            let got = ts
                .get_main_chain_cfilters(tx, &sim.blocks[1], false, 4)
                .expect("from block 3");
            assert_eq!(got.len(), 1);
            // The birthday height is the row's height, resolved or pending
            // from a height (FB12 (a)): moved to 1 with the flags cleared,
            // block 1 is missing.
            purse_udb::txstore::set_birth_state(tx, &birth(1, false)).expect("birth");
            assert_eq!(ts.missing_cfilters_height(tx).expect("missing"), 1);
            purse_udb::txstore::set_birth_state(tx, &birth(2, true)).expect("birth");
            assert_eq!(ts.missing_cfilters_height(tx).expect("missing"), 2);
            // A rollback below the birthday and a reconnection keep the rule.
            purse_udb::txstore::set_birth_state(tx, &birth(3, true)).expect("birth");
            ts.rollback(tx, 2).expect("rollback");
            let (h, hash) = sim.header(&ts, tx);
            ts.extend_main_chain(tx, &h, &hash, None)
                .expect("block 2 again");
            let (h, hash) = sim.header(&ts, tx);
            assert!(ts.extend_main_chain(tx, &h, &hash, None).is_err());
            check_rows(tx);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// ADR-0019 FB11 (i) and FB12 (a), the pending states, which no operation
/// script reaches either: while a birthday set from a time T is pending, a
/// block is connected without its filter when its own timestamp is not
/// after T, and the block whose timestamp is after T, the one resolving
/// the birthday, and every block after it are refused without one, with
/// nothing written; `MissingCFiltersHeight` answers `NotExist` though
/// blocks lack filters, and walks from the row's height again once the
/// birthday is resolved. A row pending from both a height H and a time T
/// takes a block without its filter only below both, and the walk starts
/// at H.
#[test]
fn pending_birthdays_follow_fb11_and_fb12() {
    let (store, ts, _) = tx_store("simnet");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-pending-birthday"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    // Sim::header stamps its n-th header 1_795_400_000 + n.
    let stamp = |n: u32| 1_795_400_000 + i64::from(n);
    let birth = |height: u32, from_height: bool, time: i64| BirthState {
        set_from_height: from_height,
        set_from_time: true,
        hash: Hash32([0x42; 32]),
        height,
        time,
    };
    store
        .update(|tx| {
            // Pending from time stamp(2): blocks 1 and 2 (stamps 1 and 2)
            // without filters; block 3 (stamp 3, after T) resolves the
            // birthday and is refused without one.
            purse_udb::txstore::set_birth_state(tx, &birth(0, false, stamp(2))).expect("birth");
            for want in 1..=2 {
                let (h, hash) = sim.header(&ts, tx);
                ts.extend_main_chain(tx, &h, &hash, None)
                    .expect("not after the birthday's time");
                sim.blocks.push(hash);
                assert_eq!(ts.main_chain_tip(tx).expect("tip").1, want);
            }
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            let e = ts
                .extend_main_chain(tx, &h, &hash, None)
                .expect_err("the resolving block");
            assert_eq!(e.as_kind(), Some(Kind::Invalid), "{e:?}");
            assert_eq!(dump(tx), before, "a refusal wrote");
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("with its filter");
            sim.blocks.push(hash);
            // Every later block needs its filter too.
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            ts.extend_main_chain(tx, &h, &hash, None)
                .expect_err("after the resolving block");
            assert_eq!(dump(tx), before, "a refusal wrote");
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("with its filter");
            sim.blocks.push(hash);
            // No birthday height while the time birthday is pending:
            // NotExist, though blocks 1 and 2 have no filter.
            let e = ts
                .missing_cfilters_height(tx)
                .expect_err("pending from time");
            assert_eq!(e.as_kind(), Some(Kind::NotExist), "{e:?}");
            assert!(!ts.is_missing_main_chain_cfilters(tx).expect("flag"));
            // Resolved at block 1, or pending from height 1: block 1 is the
            // first missing.
            let mut resolved = birth(1, false, stamp(2));
            resolved.set_from_time = false;
            purse_udb::txstore::set_birth_state(tx, &resolved).expect("birth");
            assert_eq!(ts.missing_cfilters_height(tx).expect("missing"), 1);
            resolved.set_from_height = true;
            purse_udb::txstore::set_birth_state(tx, &resolved).expect("birth");
            assert_eq!(ts.missing_cfilters_height(tx).expect("missing"), 1);
            // Pending from height 7 and time stamp(6): block 5 (stamp 5)
            // lies below both and is taken without a filter; block 6 (stamp
            // 6) too; block 7 (stamp 7) is after T and refused, below H or
            // not. The walk starts at H = 2, at block 2.
            purse_udb::txstore::set_birth_state(tx, &birth(7, true, stamp(6))).expect("birth");
            for want in 5..=6 {
                let (h, hash) = sim.header(&ts, tx);
                ts.extend_main_chain(tx, &h, &hash, None)
                    .expect("below both");
                sim.blocks.push(hash);
                assert_eq!(ts.main_chain_tip(tx).expect("tip").1, want);
            }
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            ts.extend_main_chain(tx, &h, &hash, None)
                .expect_err("after the time");
            assert_eq!(dump(tx), before, "a refusal wrote");
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("with its filter");
            sim.blocks.push(hash);
            // Pending from height 8 and a late time: block 8 is at H and
            // refused without its filter, its timestamp not after T.
            purse_udb::txstore::set_birth_state(tx, &birth(8, true, stamp(100))).expect("birth");
            let (h, hash) = sim.header(&ts, tx);
            let before = dump(tx);
            ts.extend_main_chain(tx, &h, &hash, None)
                .expect_err("at the height");
            assert_eq!(dump(tx), before, "a refusal wrote");
            purse_udb::txstore::set_birth_state(tx, &birth(2, true, stamp(100))).expect("birth");
            assert_eq!(ts.missing_cfilters_height(tx).expect("missing"), 2);
            check_rows(tx);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// `extend_main_chain` given, beside a header disapproving the tip, the
/// hash of a block already in the main chain at another height (which the
/// store refuses; the operation scripts always pass the header's own hash):
/// refused with nothing written, the tip's stake validity included.
#[test]
fn a_refused_connection_leaves_the_tip_valid() {
    let (store, ts, _) = tx_store("simnet");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-connect"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    store
        .update(|tx| {
            for _ in 0..2 {
                let (h, hash) = sim.header(&ts, tx);
                ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                    .expect("extend");
                sim.blocks.push(hash);
            }
            let tip = sim.blocks[1];
            let (h, _) = sim.header(&ts, tx);
            let (mut header, _) = BlockHeader::from_bytes(&h.0).expect("parse");
            header.vote_bits = 0;
            let before = dump(tx);
            let e = ts
                .extend_main_chain(tx, &header_bytes(&header), &sim.blocks[0], Some(&[]))
                .expect_err("a hash in the main chain at height 1");
            assert_eq!(e.as_kind(), Some(Kind::Invalid), "{e:?}");
            assert_eq!(dump(tx), before, "a refusal wrote");
            assert_eq!(
                ts.block_in_main_chain(tx, &tip).expect("query"),
                (true, false)
            );
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// Every row of the slice's tables, for comparing whole stores.
fn dump(tx: &mut WriteTx<'_>) -> Vec<(TableId, Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    for t in TableId::ALL {
        for r in tx
            .range_raw(*t, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
        {
            let (k, v) = r.expect("row");
            out.push((*t, k, v));
        }
    }
    out
}

/// The invariants the module documentation lists for the rows the first
/// slice keeps (crate `txstore`, "Rows"), checked by a full walk.
fn check_rows(tx: &mut WriteTx<'_>) {
    // A block row's count is the number of its history rows, at positions
    // 0 to count - 1, each naming a tx_mined row at that height with that
    // position.
    let blocks: Vec<_> = tx
        .iter::<Blocks>()
        .expect("blocks")
        .map(|r| r.expect("row"))
        .collect();
    let history: Vec<_> = tx
        .iter::<History>()
        .expect("history")
        .map(|r| r.expect("row"))
        .collect();
    let mut per_height: BTreeMap<u32, Vec<(u32, Hash32)>> = BTreeMap::new();
    for (k, h) in &history {
        per_height
            .entry(k.height)
            .or_default()
            .push((k.position, *h));
    }
    let heights: BTreeSet<u32> = blocks.iter().map(|(h, _)| h.0).collect();
    for (h, b) in &blocks {
        let rows = per_height.remove(&h.0).unwrap_or_default();
        assert_eq!(
            rows.len(),
            b.wallet_txs as usize,
            "history rows of block {}",
            h.0
        );
        for (i, (pos, hash)) in rows.iter().enumerate() {
            assert_eq!(*pos as usize, i, "history positions of block {}", h.0);
            let row = tx
                .get::<TxMined>(&purse_store::TxRecKey {
                    hash: *hash,
                    height: h.0,
                })
                .expect("get")
                .expect("the history row's record");
            assert_eq!(row.position, *pos);
        }
    }
    assert!(per_height.is_empty(), "history rows above the main chain");
    // Every tx_mined row is at a main-chain height and has its history row.
    let mined: Vec<_> = tx
        .iter::<TxMined>()
        .expect("mined")
        .map(|r| r.expect("row"))
        .collect();
    let named: BTreeSet<(u32, Hash32)> = history.iter().map(|(k, h)| (k.height, *h)).collect();
    for (k, _) in &mined {
        assert!(heights.contains(&k.height), "a record above the main chain");
        assert!(
            named.contains(&(k.height, k.hash)),
            "a record without history"
        );
    }
    // Every unmined_inputs row names an unmined record spending that
    // outpoint (spec 03 I8, the direction Go keeps).
    let inputs: Vec<_> = tx
        .iter::<UnminedInputs>()
        .expect("inputs")
        .map(|r| r.expect("row"))
        .collect();
    for (op, spender) in inputs {
        let row = tx
            .get::<TxUnmined>(&spender)
            .expect("get")
            .expect("the spender's unmined record");
        let (m, _) = MsgTx::from_bytes(&row.tx).expect("record decodes");
        assert!(
            m.tx_in
                .iter()
                .any(|i| i.previous_out_point.hash.0 == op.hash.0
                    && i.previous_out_point.index == op.index),
            "an unmined input row names a transaction not spending it"
        );
    }
    // The next slice's tables stay empty.
    for t in LATER_TABLES {
        assert!(tx.is_empty(t).expect("empty"), "{} holds rows", t.name());
    }
    check_credit_rows(tx);
    tx.check_chain_rows();
}

/// The UTXO queries against the rows:
///
/// - `UnspentOutputs` gives each unspent row no unmined input names, in
///   key order (from the outpoint's unmined credit when it has one, as
///   Go's `outputCreditInfo` looks there first), then each unmined credit
///   no unmined input names whose transaction is not unpublished, each
///   with the script of its output in the record's own transaction (so
///   the stored script locations are checked against a decoding);
/// - `ForEachUnspentOutpoint` gives the same outpoints, trees included, in
///   the same order;
/// - `ExistsUTXO` holds for every unspent row and unmined credit, and
///   `UnspentOutput` answers for each (with the mempool for an unmined
///   credit);
/// - `IsUnspentOutpoint` is false exactly for an outpoint an unmined input
///   names or whose transaction is unpublished (QK-0088);
/// - `PreviousPkScripts` of each mined record in its block gives one
///   script per debit, that of the output the debit spends.
fn check_utxo_queries(ts: &TxStore, tx: &mut WriteTx<'_>) {
    let unspent = all::<Unspent>(tx);
    let ucredits = all::<UnminedCredits>(tx);
    let uinputs: BTreeSet<purse_store::Outpoint> = all::<UnminedInputs>(tx)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let unpublished: BTreeSet<Hash32> =
        all::<Unpublished>(tx).into_iter().map(|(k, _)| k).collect();
    let ucmap: BTreeMap<purse_store::Outpoint, purse_store::UnminedCreditRow> =
        ucredits.iter().cloned().collect();
    let tree = |tag: purse_store::OutputTag| i8::from(tag != purse_store::OutputTag::NonStake);
    // (outpoint, tree, height, amount, coinbase, expiry)
    let mut want = Vec::new();
    for (op, u) in &unspent {
        if uinputs.contains(op) {
            continue;
        }
        want.push(match ucmap.get(op) {
            Some(m) => (*op, tree(m.flags.tag), -1, m.amount, false, m.flags.expiry),
            None => (
                *op,
                tree(u.flags.tag),
                i32::try_from(u.height).expect("height"),
                u.amount,
                u.flags.coinbase,
                u.flags.expiry,
            ),
        });
    }
    for (op, m) in &ucredits {
        if !uinputs.contains(op) && !unpublished.contains(&op.hash) {
            want.push((*op, tree(m.flags.tag), -1, m.amount, false, m.flags.expiry));
        }
    }
    let got = ts.unspent_outputs(tx).expect("UnspentOutputs");
    assert_eq!(got.len(), want.len(), "UnspentOutputs: count");
    for (c, w) in got.iter().zip(&want) {
        let op = purse_store::Outpoint {
            hash: Hash32(c.outpoint.hash.0),
            index: c.outpoint.index,
        };
        assert_eq!(
            (
                op,
                c.outpoint.tree,
                c.block.block.height,
                c.amount,
                c.from_coinbase,
                c.has_expiry
            ),
            *w,
            "UnspentOutputs: {c:?}"
        );
        assert_eq!(c.block.time, purse_udb::txstore::ZERO_TIME);
        let (bytes, received) = if c.block.block.height < 0 {
            assert_eq!(c.block.block.hash, Hash32::default());
            let row = tx.get::<TxUnmined>(&op.hash).expect("get").expect("record");
            (row.tx, row.received)
        } else {
            let height = u32::try_from(c.block.block.height).expect("height");
            let b = tx
                .get::<Blocks>(&purse_store::Height(height))
                .expect("get")
                .expect("block");
            assert_eq!(c.block.block.hash, b.hash, "the credit's block hash");
            let row = tx
                .get::<TxMined>(&purse_store::TxRecKey {
                    hash: op.hash,
                    height,
                })
                .expect("get")
                .expect("record");
            (row.tx, row.received)
        };
        let (m, _) = MsgTx::from_bytes(&bytes).expect("record decodes");
        assert_eq!(
            c.pk_script, m.tx_out[op.index as usize].pk_script,
            "the script of {op:?}"
        );
        assert_eq!(c.received, received);
    }
    let mut each = Vec::new();
    ts.for_each_unspent_outpoint(tx, |op| {
        each.push(*op);
        Ok(())
    })
    .expect("ForEachUnspentOutpoint");
    assert_eq!(
        each,
        got.iter().map(|c| c.outpoint).collect::<Vec<_>>(),
        "ForEachUnspentOutpoint"
    );
    let wire = |op: &purse_store::Outpoint| OutPoint {
        hash: Hash(op.hash.0),
        index: op.index,
        tree: 0,
    };
    for (op, _) in &unspent {
        assert!(ts.exists_utxo(tx, &wire(op)).expect("ExistsUTXO"));
        let c = ts
            .unspent_output(tx, &wire(op), false)
            .expect("UnspentOutput");
        assert_eq!(c.block.block.height < 0, ucmap.contains_key(op));
    }
    for (op, _) in &ucredits {
        assert!(ts.exists_utxo(tx, &wire(op)).expect("ExistsUTXO"));
        let c = ts
            .unspent_output(tx, &wire(op), true)
            .expect("UnspentOutput");
        assert_eq!(c.block.block.height, -1);
        assert_eq!(
            ts.is_unspent_outpoint(tx, &wire(op))
                .expect("IsUnspentOutpoint"),
            !uinputs.contains(op) && !unpublished.contains(&op.hash)
        );
    }
    for op in &uinputs {
        assert!(
            !ts.is_unspent_outpoint(tx, &wire(op))
                .expect("IsUnspentOutpoint")
        );
    }
    let debits = all::<Debits>(tx);
    for (k, row) in all::<TxMined>(tx) {
        let n = debits
            .iter()
            .filter(|(d, _)| d.hash == k.hash && d.height == k.height)
            .count();
        let b = tx
            .get::<Blocks>(&purse_store::Height(k.height))
            .expect("get")
            .expect("block");
        let rec = TxRecord::new(&row.tx, row.received).expect("record");
        let block = purse_udb::txstore::Block {
            hash: b.hash,
            height: i32::try_from(k.height).expect("height"),
        };
        let mine: Vec<_> = debits
            .iter()
            .filter(|(d, _)| d.hash == k.hash && d.height == k.height)
            .map(|(_, d)| d.credit)
            .collect();
        let res = ts.previous_pk_scripts(tx, &rec, Some(&block));
        // A debit whose credit is not live (a spender recorded below the
        // transaction it spends, whose block was rolled back; or one moved
        // to the invalidated table): Go's IO.
        let mut missing = false;
        for ck in &mine {
            missing |= tx.get::<Credits>(ck).expect("get").is_none();
        }
        if missing {
            let e = res.expect_err("PreviousPkScripts with a debit's credit missing");
            assert_eq!(e.as_kind(), Some(Kind::Io), "{e:?}");
            assert!(format!("{e}").contains("missing credit val for key"), "{e}");
            continue;
        }
        let scripts = res.expect("PreviousPkScripts");
        assert_eq!(scripts.len(), n, "PreviousPkScripts of {k:?}");
        for (ck, s) in mine.iter().zip(&scripts) {
            let r = tx
                .get::<TxMined>(&purse_store::TxRecKey {
                    hash: ck.hash,
                    height: ck.height,
                })
                .expect("get")
                .expect("the spent credit's record");
            let (m, _) = MsgTx::from_bytes(&r.tx).expect("record decodes");
            assert_eq!(*s, m.tx_out[ck.index as usize].pk_script);
        }
    }
}

/// All rows of table `T`.
fn all<T: Table>(tx: &mut WriteTx<'_>) -> Vec<(T::Key, T::Value)> {
    tx.iter::<T>()
        .expect("iter")
        .map(|r| r.expect("row"))
        .collect()
}

/// The credit rows' invariants (crate `txstore`, "The credit slice"):
///
/// - every unspent output's credit is a live credit at the height the
///   unspent row names, and the row is what that credit makes;
/// - an unspent output's credit names no spender, with one exception, the
///   state of QK-0785, which Go makes and dcr-purse reproduces: a credit
///   whose spender is a live debit that stake invalidation of the credit's
///   block does not move, that of a stake transaction or of a transaction
///   recorded in another block (below the credit's, QK-0101's state);
///   invalidation moves the credit, spender kept, to the invalidated table
///   and leaves that debit live, and stake validation moves the credit
///   back with an unspent entry;
/// - every unmined credit's class is the one its flags and its
///   transaction's unpublished mark give;
/// - `balances` and `balance_heights` are the sums, by account and class
///   (and height for the mined classes), of the unspent outputs and
///   unmined credits no unmined transaction spends;
/// - `acct_history` holds exactly, for every mined record, the accounts of
///   its live credits and of the credits (live or invalidated) its live
///   debits spend that name the debit as their spender.
fn check_credit_rows(tx: &mut WriteTx<'_>) {
    let mut sums: BTreeMap<(u32, BalanceClass), (i64, i64)> = BTreeMap::new();
    let mut heights: BTreeMap<(u32, BalanceClass, u32), (i64, i64)> = BTreeMap::new();
    let mut add = |account: u32, class: BalanceClass, height: u32, amount: i64| {
        let e = sums.entry((account, class)).or_default();
        e.0 = e.0.wrapping_add(amount);
        e.1 += 1;
        if class.is_windowed() {
            let e = heights.entry((account, class, height)).or_default();
            e.0 = e.0.wrapping_add(amount);
            e.1 += 1;
        }
    };
    for (op, u) in all::<Unspent>(tx) {
        let ck = CreditKey {
            hash: op.hash,
            height: u.height,
            index: op.index,
        };
        let c = tx
            .get::<Credits>(&ck)
            .expect("get")
            .unwrap_or_else(|| panic!("unspent {op:?} without its credit at {}", u.height));
        assert_eq!(u.amount, c.amount, "unspent amount");
        assert_eq!(u.account, c.account, "unspent account");
        assert_eq!(u.flags, c.flags, "unspent flags");
        assert_eq!(u.script, c.script, "unspent script");
        assert_eq!(u.class, mined_class(&c.flags), "unspent class");
        if let Some(s) = c.spender {
            assert!(
                spent_by_a_debit_invalidation_leaves(tx, &ck, &s),
                "unspent {op:?} whose credit names the spender {s:?}, which is not \
                 a live debit stake invalidation leaves (QK-0785's one exception)"
            );
        }
        if !tx.contains::<UnminedInputs>(&op).expect("contains") {
            add(u.account, u.class, u.height, u.amount);
        }
    }
    for (op, m) in all::<UnminedCredits>(tx) {
        let unpublished = tx.contains::<Unpublished>(&op.hash).expect("contains");
        assert_eq!(
            m.class,
            unmined_class(&m.flags, unpublished),
            "unmined credit class"
        );
        if !tx.contains::<UnminedInputs>(&op).expect("contains") {
            add(m.account, m.class, 0, m.amount);
        }
    }
    let have: BTreeMap<(u32, BalanceClass), (i64, i64)> = all::<Balances>(tx)
        .into_iter()
        .map(|(k, BalanceRow { sum, count })| ((k.account, k.class), (sum, count)))
        .collect();
    assert_eq!(have, sums, "balances against the rows");
    let have: BTreeMap<(u32, BalanceClass, u32), (i64, i64)> = all::<BalanceHeights>(tx)
        .into_iter()
        .map(|(k, BalanceRow { sum, count })| ((k.account, k.class, k.height), (sum, count)))
        .collect();
    assert_eq!(have, heights, "balance_heights against the rows");
    let mut want = BTreeSet::new();
    for (k, row) in all::<TxMined>(tx) {
        let lo = CreditKey {
            hash: k.hash,
            height: k.height,
            index: 0,
        };
        let hi = CreditKey {
            index: u32::MAX,
            ..lo
        };
        let credits: Vec<_> = tx
            .range::<Credits>(Bound::Included(&lo), Bound::Included(&hi))
            .expect("range")
            .map(|r| r.expect("row").1.account)
            .collect();
        let debits: Vec<_> = tx
            .range::<Debits>(Bound::Included(&lo), Bound::Included(&hi))
            .expect("range")
            .map(|r| r.expect("row"))
            .collect();
        let mut accounts: Vec<u32> = credits;
        for (dk, d) in debits {
            let c = match tx.get::<Credits>(&d.credit).expect("get") {
                Some(c) => Some(c),
                None => tx.get::<CreditsInvalidated>(&d.credit).expect("get"),
            };
            let me = purse_store::Spender {
                hash: dk.hash,
                height: dk.height,
                input: dk.index,
            };
            accounts.extend(c.filter(|c| c.spender == Some(me)).map(|c| c.account));
        }
        for a in accounts {
            want.insert((
                AcctHistoryKey {
                    account: a,
                    height: k.height,
                    position: row.position,
                },
                k.hash,
            ));
        }
    }
    let have: BTreeSet<_> = all::<AcctHistory>(tx).into_iter().collect();
    assert_eq!(have, want, "acct_history against the rows");
}

/// Whether `s` is a live debit of credit `ck` that stake invalidation of
/// the credit's block leaves in place, by a mined transaction that is not
/// regular or that is recorded at another height: the only spender an
/// unspent output's credit may name (QK-0785).
fn spent_by_a_debit_invalidation_leaves(
    tx: &mut WriteTx<'_>,
    ck: &CreditKey,
    s: &purse_store::Spender,
) -> bool {
    let dk = CreditKey {
        hash: s.hash,
        height: s.height,
        index: s.input,
    };
    let Some(d) = tx.get::<Debits>(&dk).expect("get") else {
        return false;
    };
    let Some(row) = tx
        .get::<TxMined>(&purse_store::TxRecKey {
            hash: s.hash,
            height: s.height,
        })
        .expect("get")
    else {
        return false;
    };
    let (m, _) = MsgTx::from_bytes(&row.tx).expect("record decodes");
    let rec = TxRecord::from_msg_tx(m, 0).expect("record");
    d.credit == *ck && (rec.tx_type() != TxType::Regular || s.height != ck.height)
}

/// `ReadTx::check_chain` from a write transaction, through a check of the
/// same invariants on its rows.
trait CheckChain {
    fn check_chain_rows(&mut self);
}

impl CheckChain for WriteTx<'_> {
    fn check_chain_rows(&mut self) {
        let blocks: Vec<_> = self
            .iter::<Blocks>()
            .expect("blocks")
            .map(|r| r.expect("row"))
            .collect();
        let tip = self.tip().expect("tip").expect("a tip");
        let (last, row) = blocks.last().expect("a main chain");
        assert_eq!((tip.height, tip.hash), (last.0, row.hash), "tip row");
        for (i, (h, b)) in blocks.iter().enumerate() {
            assert_eq!(h.0 as usize, i, "no gaps");
            assert_eq!(self.block_height(&b.hash).expect("index"), Some(h.0));
            let header = self.header_at(h.0).expect("header").expect("a header");
            assert_eq!(header.height(), h.0);
        }
    }
}

/// A P2PKH script to a hash of `fill` bytes, tagged with the stake opcode
/// `tag`.
fn p2pkh(tag: Option<u8>, fill: u64) -> Vec<u8> {
    let mut s = Vec::with_capacity(26);
    s.extend(tag);
    s.extend_from_slice(&[0x76, 0xa9, 0x14]);
    s.extend_from_slice(&[u8::try_from(fill % 256).expect("byte"); 20]);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

/// `OP_RETURN` with one push of `payload` (at most 75 bytes).
fn op_return(payload: &[u8]) -> Vec<u8> {
    let mut s = vec![0x6a, u8::try_from(payload.len()).expect("short")];
    s.extend_from_slice(payload);
    s
}

/// A simulated network of transactions for the random sequences: each
/// built transaction spends earlier outputs or foreign ones.
struct Sim {
    rng: SplitMix64,
    recs: Vec<TxRecord>,
    tickets: Vec<Hash32>,
    blocks: Vec<Hash32>,
    nonce: u32,
}

impl Sim {
    fn hash(&mut self) -> Hash {
        let mut b = [0u8; 32];
        b[0] = u8::try_from(self.rng.below(4)).expect("small") + 1;
        Hash(b)
    }

    fn build(&mut self) -> TxRecord {
        let n_in = 1 + self.rng.below(3);
        let mut tx_in = Vec::new();
        for _ in 0..n_in {
            let prev = self.prev();
            tx_in.push(TxIn {
                previous_out_point: prev,
                sequence: u32::MAX,
                value_in: 1000,
                ..TxIn::default()
            });
        }
        let n_out = 1 + self.rng.below(3);
        let tx_out = (0..n_out)
            .map(|i| TxOut {
                value: i64::try_from(100 + i).expect("small"),
                version: 0,
                pk_script: vec![0x6a, 0x01, u8::try_from(self.rng.below(256)).expect("byte")],
            })
            .collect();
        let m = MsgTx {
            tx_in,
            tx_out,
            expiry: if self.rng.below(5) == 0 {
                u32::try_from(1 + self.rng.below(6)).expect("small")
            } else {
                0
            },
            ..MsgTx::default()
        };
        let rec = TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
        self.recs.push(rec.clone());
        rec
    }

    /// A ticket purchase: one input, the stake submission output, its
    /// commitment and a change output.
    fn build_ticket(&mut self) -> TxRecord {
        let prev = self.prev();
        let mut commitment = vec![7u8; 20];
        commitment.extend_from_slice(&1100i64.to_le_bytes());
        commitment.extend_from_slice(&0x5800u16.to_le_bytes());
        let m = MsgTx {
            tx_in: vec![TxIn {
                previous_out_point: prev,
                sequence: u32::MAX,
                value_in: 1100,
                ..TxIn::default()
            }],
            tx_out: vec![
                TxOut {
                    value: [1000, 2000][usize::try_from(self.rng.below(2)).expect("small")],
                    version: 0,
                    pk_script: p2pkh(Some(0xba), self.rng.below(256)),
                },
                TxOut {
                    value: 0,
                    version: 0,
                    pk_script: op_return(&commitment),
                },
                TxOut {
                    value: 0,
                    version: 0,
                    pk_script: p2pkh(Some(0xbd), self.rng.below(256)),
                },
            ],
            ..MsgTx::default()
        };
        self.push(m, TxType::SStx)
    }

    /// A vote spending `ticket` on the block `block` at `height`.
    fn build_vote(&mut self, ticket: &Hash32, block: &Hash32, height: u32) -> TxRecord {
        let mut voted = block.0.to_vec();
        voted.extend_from_slice(&height.to_le_bytes());
        let m = MsgTx {
            tx_in: vec![
                TxIn {
                    previous_out_point: OutPoint {
                        hash: Hash::ZERO,
                        index: u32::MAX,
                        tree: 0,
                    },
                    sequence: u32::MAX,
                    value_in: 300,
                    block_height: dcroxide_wire::NULL_BLOCK_HEIGHT,
                    block_index: dcroxide_wire::NULL_BLOCK_INDEX,
                    signature_script: vec![0x00, 0x00],
                },
                TxIn {
                    previous_out_point: OutPoint {
                        hash: Hash(ticket.0),
                        index: 0,
                        tree: 1,
                    },
                    sequence: u32::MAX,
                    value_in: 1000,
                    ..TxIn::default()
                },
            ],
            tx_out: vec![
                TxOut {
                    value: 0,
                    version: 0,
                    pk_script: op_return(&voted),
                },
                TxOut {
                    value: 0,
                    version: 0,
                    pk_script: op_return(&[1, 0]),
                },
                TxOut {
                    value: 1300,
                    version: 0,
                    pk_script: p2pkh(Some(0xbb), self.rng.below(256)),
                },
            ],
            ..MsgTx::default()
        };
        self.push(m, TxType::SSGen)
    }

    /// A revocation of `ticket`.
    fn build_revocation(&mut self, ticket: &Hash32) -> TxRecord {
        let m = MsgTx {
            version: 2,
            tx_in: vec![TxIn {
                previous_out_point: OutPoint {
                    hash: Hash(ticket.0),
                    index: 0,
                    tree: 1,
                },
                sequence: u32::MAX,
                value_in: 1000,
                ..TxIn::default()
            }],
            tx_out: vec![TxOut {
                value: 1000,
                version: 0,
                pk_script: p2pkh(Some(0xbc), self.rng.below(256)),
            }],
            ..MsgTx::default()
        };
        self.push(m, TxType::SSRtx)
    }

    /// A previous outpoint: an output of a built transaction, or a foreign
    /// one.
    fn prev(&mut self) -> OutPoint {
        if !self.recs.is_empty() && self.rng.below(3) != 0 {
            let i = usize::try_from(self.rng.below(self.recs.len() as u64)).expect("small");
            let r = &self.recs[i];
            let n = r.msg_tx().tx_out.len().max(1) as u64;
            OutPoint {
                hash: Hash(r.hash().0),
                index: u32::try_from(self.rng.below(n)).expect("small"),
                tree: 0,
            }
        } else {
            OutPoint {
                hash: self.hash(),
                index: u32::try_from(self.rng.below(3)).expect("small"),
                tree: 0,
            }
        }
    }

    fn push(&mut self, m: MsgTx, want: TxType) -> TxRecord {
        let rec = TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
        assert_eq!(rec.tx_type(), want, "the simulated transaction's type");
        if want == TxType::SStx {
            self.tickets.push(*rec.hash());
        }
        self.recs.push(rec.clone());
        rec
    }

    /// A ticket the simulation built, if any.
    fn ticket(&mut self) -> Option<Hash32> {
        if self.tickets.is_empty() {
            return None;
        }
        let i = usize::try_from(self.rng.below(self.tickets.len() as u64)).expect("small");
        Some(self.tickets[i])
    }

    /// A stake transaction: a ticket, or a vote on the tip or another
    /// main-chain block, or a revocation, of a ticket built earlier.
    fn build_stake(&mut self, ts: &TxStore, tx: &mut WriteTx<'_>) -> TxRecord {
        let Some(ticket) = self.ticket() else {
            return self.build_ticket();
        };
        match self.rng.below(4) {
            0 => self.build_ticket(),
            1 => self.build_revocation(&ticket),
            _ => {
                let (tip, height) = ts.main_chain_tip(tx).expect("tip");
                let h = if height > 0 && self.rng.below(2) == 0 {
                    u32::try_from(self.rng.below(u64::try_from(height).expect("height")))
                        .expect("small")
                } else {
                    u32::try_from(height).expect("height")
                };
                let block = ts
                    .get_main_chain_block_hash_for_height(tx, i32::try_from(h).expect("small"))
                    .unwrap_or(tip);
                self.build_vote(&ticket, &block, h)
            }
        }
    }

    fn header(&mut self, ts: &TxStore, tx: &mut WriteTx<'_>) -> (HeaderBytes, Hash32) {
        let (tip, height) = ts.main_chain_tip(tx).expect("tip");
        self.nonce += 1;
        let h = BlockHeader {
            version: 7,
            prev_block: Hash(tip.0),
            merkle_root: Hash([u8::try_from(self.nonce % 256).expect("byte"); 32]),
            stake_root: Hash::ZERO,
            vote_bits: if self.rng.below(4) == 0 { 0 } else { 1 },
            final_state: [0; 6],
            voters: 0,
            fresh_stake: 0,
            revocations: 0,
            pool_size: 0,
            bits: 0,
            sbits: 0,
            height: u32::try_from(height + 1).expect("height"),
            size: 0,
            timestamp: 1_795_400_000 + self.nonce,
            nonce: self.nonce,
            extra_data: [0; 32],
            stake_version: 0,
        };
        (header_bytes(&h), Hash32(*h.block_hash().as_bytes()))
    }
}

/// Answers of every query of the slice, for comparing a store before and
/// after a reopen.
fn answers(ts: &TxStore, r: &mut ReadTx, sim: &Sim) -> String {
    let mut out = format!("{:?}\n", ts.main_chain_tip(r).expect("tip"));
    out += &format!("{:?}\n", ts.unmined_txs(r).expect("unmined"));
    for minconf in [0, 1, 6] {
        out += &format!("{:?}\n", ts.account_balances(r, minconf).expect("balances"));
    }
    let (_, tip) = ts.main_chain_tip(r).expect("tip");
    out += &format!(
        "{:?}\n",
        ts.unspent_tickets(r, tip, false).expect("tickets")
    );
    out += &format!("{:?}\n", ts.iterate_tickets(r).expect("tickets"));
    out += &format!("{:?}\n", ts.unspent_outputs(r).expect("UnspentOutputs"));
    for rec in &sim.recs {
        out += &format!(
            "{:?} {:?} {:?}\n",
            ts.tx_details(r, rec.hash()).map_err(|e| e.as_kind()),
            ts.tx_block_height(r, rec.hash()).map_err(|e| e.as_kind()),
            ts.exists_tx_mined_or_unmined(r, rec.hash())
                .expect("exists"),
        );
        for index in 0..rec.msg_tx().tx_out.len() {
            let op = OutPoint {
                hash: Hash(rec.hash().0),
                index: u32::try_from(index).expect("small"),
                tree: 0,
            };
            out += &format!(
                "{:?} {:?} {:?} {:?}\n",
                ts.exists_utxo(r, &op).expect("ExistsUTXO"),
                ts.is_unspent_outpoint(r, &op).expect("IsUnspentOutpoint"),
                ts.unspent_output(r, &op, true).map_err(|e| e.as_kind()),
                ts.unspent_output(r, &op, false).map_err(|e| e.as_kind()),
            );
        }
    }
    let mut batches = Vec::new();
    ts.range_transactions(r, -1, -1, |d| {
        batches.push(format!("{d:?}"));
        Ok(false)
    })
    .expect("range");
    out += &batches.join("\n");
    out
}

/// Every row of every table through a read transaction.
fn dump_read(r: &ReadTx) -> Vec<(TableId, Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    for t in TableId::ALL {
        for row in r
            .range_raw(*t, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
        {
            let (k, v) = row.expect("row");
            out.push((*t, k, v));
        }
    }
    out
}

/// Random sequences of every operation of the slice (regular, ticket,
/// vote and revocation transactions, inserted unmined or mined), each in
/// its own write transaction:
///
/// - the row invariants hold after every operation that succeeds;
/// - an operation that fails has written nothing when it returns, with
///   the one exception Go has too: a vote or revocation that removed an
///   unmined vote for one input and then double spends another unmined
///   transaction (`DoubleSpend`) has removed rows and added none;
/// - the abandoned transaction leaves the store's rows as they were;
/// - a reopened store holds the same rows and answers as the live one did.
#[test]
fn random_sequences_keep_the_row_invariants() {
    let mut seeds = SplitMix64::from_env("purse-udb-txstore-sequences");
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut credits_added = 0usize;
    for _ in 0..12 {
        let (mut store, ts, path) = tx_store("simnet");
        let mut sim = Sim {
            rng: SplitMix64(seeds.next_u64()),
            recs: Vec::new(),
            tickets: Vec::new(),
            blocks: Vec::new(),
            nonce: 0,
        };
        for step in 0..120 {
            let choice = sim.rng.below(18);
            let pre = dump_read(&store.read().expect("read"));
            let mut failed = None;
            let r: Result<(), TErr> = store.update(|tx| {
                let before = dump(tx);
                let mut stake_insert = false;
                let res = match choice {
                    0 | 1 => {
                        let (h, hash) = sim.header(&ts, tx);
                        let res = ts.extend_main_chain(tx, &h, &hash, Some(&[]));
                        if res.is_ok() {
                            sim.blocks.push(hash);
                        }
                        res
                    }
                    2 | 3 => {
                        let rec = sim.build();
                        ts.insert_mem_pool_tx(tx, &rec)
                    }
                    4 | 5 if !sim.recs.is_empty() => {
                        let i =
                            usize::try_from(sim.rng.below(sim.recs.len() as u64)).expect("small");
                        let rec = sim.recs[i].clone();
                        let block = if sim.blocks.is_empty() || sim.rng.below(2) == 0 {
                            ts.main_chain_tip(tx).expect("tip").0
                        } else {
                            let j = usize::try_from(sim.rng.below(sim.blocks.len() as u64))
                                .expect("small");
                            sim.blocks[j]
                        };
                        ts.insert_mined_tx(tx, &rec, &block)
                    }
                    6 if !sim.recs.is_empty() => {
                        let i =
                            usize::try_from(sim.rng.below(sim.recs.len() as u64)).expect("small");
                        let rec = sim.recs[i].clone();
                        ts.remove_unconfirmed(tx, rec.msg_tx(), rec.hash())
                    }
                    7 => {
                        let (_, tip) = ts.main_chain_tip(tx).expect("tip");
                        let h = i32::try_from(sim.rng.below(u64::try_from(tip + 3).expect("tip")))
                            .expect("small")
                            - 1;
                        ts.rollback(tx, h).map(drop)
                    }
                    8 => {
                        let sdiff =
                            [0, 1000, 2000][usize::try_from(sim.rng.below(3)).expect("small")];
                        ts.prune_unmined(tx, sdiff).map(drop).map_err(|e| e.error)
                    }
                    9 if !sim.recs.is_empty() => {
                        let i =
                            usize::try_from(sim.rng.below(sim.recs.len() as u64)).expect("small");
                        let h = *sim.recs[i].hash();
                        ts.set_published(tx, &h, sim.rng.below(2) == 0)
                    }
                    10 | 11 => {
                        let rec = sim.build_stake(&ts, tx);
                        stake_insert = matches!(rec.tx_type(), TxType::SSGen | TxType::SSRtx);
                        ts.insert_mem_pool_tx(tx, &rec)
                    }
                    12 => {
                        let rec = sim.build_stake(&ts, tx);
                        let tip = ts.main_chain_tip(tx).expect("tip").0;
                        ts.insert_mined_tx(tx, &rec, &tip)
                    }
                    13 if !sim.recs.is_empty() => {
                        let i =
                            usize::try_from(sim.rng.below(sim.recs.len() as u64)).expect("small");
                        let rec = sim.recs[i].clone();
                        stake_insert = matches!(rec.tx_type(), TxType::SSGen | TxType::SSRtx);
                        ts.insert_mem_pool_tx(tx, &rec)
                    }
                    14..=17 if !sim.recs.is_empty() => {
                        // A credit: unmined, or in a block the record is
                        // mined in (mostly its last), or in a block it is
                        // not; an output index sometimes out of range.
                        let i =
                            usize::try_from(sim.rng.below(sim.recs.len() as u64)).expect("small");
                        let rec = sim.recs[i].clone();
                        let n = rec.msg_tx().tx_out.len() as u64;
                        let index = u32::try_from(sim.rng.below(n + 1)).expect("small");
                        let account = u32::try_from(sim.rng.below(3)).expect("small");
                        let change = sim.rng.below(2) == 0;
                        let (mined, unmined) = ts
                            .exists_tx_mined_or_unmined(tx, rec.hash())
                            .expect("exists");
                        let own = match ts.tx_block_height(tx, rec.hash()).unwrap_or(-1) {
                            h if h >= 0 => ts
                                .get_main_chain_block_hash_for_height(tx, h)
                                .ok()
                                .and_then(|b| ts.get_block_meta_for_hash(tx, &b).ok()),
                            _ => None,
                        };
                        let block = match sim.rng.below(8) {
                            0 => None,
                            1 if !sim.blocks.is_empty() => {
                                let j = usize::try_from(sim.rng.below(sim.blocks.len() as u64))
                                    .expect("small");
                                ts.get_block_meta_for_hash(tx, &sim.blocks[j]).ok()
                            }
                            _ if unmined || !mined => None,
                            _ => own,
                        };
                        let block: Option<BlockMeta> = block;
                        let r = ts.add_credit(tx, &rec, block.as_ref(), index, change, account);
                        if r.is_ok() {
                            credits_added += 1;
                        }
                        r
                    }
                    _ => {
                        let rec = sim.build();
                        ts.insert_mem_pool_tx(tx, &rec)
                    }
                };
                match res {
                    Ok(()) => {
                        check_rows(tx);
                        check_utxo_queries(&ts, tx);
                        Ok(())
                    }
                    Err(e) => {
                        assert_ne!(e.as_kind(), Some(Kind::Bug), "step {step}: {e:?}");
                        let after = dump(tx);
                        if stake_insert && e.as_kind() == Some(Kind::DoubleSpend) && after != before
                        {
                            let rows: BTreeSet<_> = before.iter().collect();
                            assert!(
                                after.iter().all(|r| rows.contains(r)),
                                "step {step}: a vote's double spend added rows: {e:?}"
                            );
                            failed = Some(
                                "insert_mem_pool_tx vote eviction, then DoubleSpend".to_owned(),
                            );
                        } else {
                            assert_eq!(
                                after, before,
                                "step {step} choice {choice}: a failure wrote: {e:?}"
                            );
                            failed = Some(format!("choice {choice} {:?}", e.as_kind()));
                        }
                        // As Go's callers, abandon the transaction.
                        Err(TErr::Rollback)
                    }
                }
            });
            match r {
                Ok(()) => {}
                Err(TErr::Rollback) => {
                    assert_eq!(
                        dump_read(&store.read().expect("read")),
                        pre,
                        "step {step}: an abandoned transaction changed the store"
                    );
                    *failures.entry(failed.unwrap_or_default()).or_default() += 1;
                }
                Err(e) => panic!("update: {e:?}"),
            }
            if step % 40 == 39 {
                let (live, rows) = {
                    let mut r = store.read().expect("read");
                    (answers(&ts, &mut r, &sim), dump_read(&r))
                };
                store.close().expect("close");
                store = Store::open(&path).expect("reopen");
                let mut r = store.read().expect("read");
                assert_eq!(dump_read(&r), rows, "rows after a reopen");
                assert_eq!(answers(&ts, &mut r, &sim), live, "answers after a reopen");
                r.check_chain().expect("check_chain");
            }
        }
    }
    eprintln!(
        "failed operations checked: {failures:?}; AddCredit calls that succeeded: {credits_added}"
    );
    assert!(
        credits_added >= 60,
        "too few credits added: {credits_added}"
    );
    assert!(
        failures.len() >= 4,
        "too few kinds of failure exercised: {failures:?}"
    );
}

/// `TxRecord::new` keeps the caller's bytes, trailing bytes included, and
/// agrees with `from_msg_tx` on the hash and type; a reader serializes the
/// decoded transaction again.
#[test]
fn tx_records_round_trip() {
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-records"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    let (store, ts, _) = tx_store("simnet");
    for _ in 0..64 {
        let rec = sim.build();
        let mut bytes = rec.serialized().to_vec();
        assert_eq!(bytes, rec.msg_tx().serialize());
        let trailing = sim.rng.below(3) as usize;
        bytes.extend(std::iter::repeat_n(0xee, trailing));
        let raw = TxRecord::new(&bytes, rec.received()).expect("NewTxRecord");
        assert_eq!(raw.hash(), rec.hash());
        assert_eq!(raw.tx_type(), rec.tx_type());
        assert_eq!(raw.serialized(), &bytes[..]);
        // Truncated bytes are refused, with kind Other.
        let cut = TxRecord::new(&bytes[..bytes.len() / 2], 0).expect_err("truncated");
        assert_eq!(cut.as_kind(), None);
        store
            .update(|tx| {
                // A record is stored verbatim and read back re-serialized.
                if ts.insert_mem_pool_tx(tx, &raw).is_ok() {
                    let d = ts.tx_details(tx, raw.hash()).expect("details");
                    assert_eq!(d.record.serialized(), &rec.msg_tx().serialize()[..]);
                    let stored = tx.get::<TxUnmined>(raw.hash()).expect("get").expect("row");
                    assert_eq!(stored.tx, bytes);
                }
                Err::<(), TErr>(TErr::Rollback)
            })
            .expect_err("rolled back");
    }
}

/// `unmined_txs` puts every transaction after the unmined transactions it
/// spends, and its order does not depend on the order of insertion.
#[test]
fn unmined_txs_are_in_dependency_order() {
    let mut seeds = SplitMix64::from_env("purse-udb-txstore-kahn");
    for _ in 0..16 {
        let mut sim = Sim {
            rng: SplitMix64(seeds.next_u64()),
            recs: Vec::new(),
            tickets: Vec::new(),
            blocks: Vec::new(),
            nonce: 0,
        };
        // A chain of spends without double spends: each spends one output
        // of an earlier transaction no other spends, or a foreign one.
        let mut spent = BTreeSet::new();
        let mut recs: Vec<TxRecord> = Vec::new();
        for i in 0..12u8 {
            let mut tx_in = Vec::new();
            for r in &recs {
                if sim.rng.below(4) == 0 && spent.insert(*r.hash()) {
                    tx_in.push(TxIn {
                        previous_out_point: OutPoint {
                            hash: Hash(r.hash().0),
                            index: 0,
                            tree: 0,
                        },
                        ..TxIn::default()
                    });
                }
            }
            tx_in.push(TxIn {
                previous_out_point: OutPoint {
                    hash: Hash([0xf0; 32]),
                    index: u32::from(i),
                    tree: 0,
                },
                ..TxIn::default()
            });
            let m = MsgTx {
                tx_in,
                tx_out: vec![TxOut::default()],
                ..MsgTx::default()
            };
            recs.push(TxRecord::from_msg_tx(m, 0).expect("record"));
        }
        let mut orders = Vec::new();
        for shuffle in 0..2 {
            let (store, ts, _) = tx_store("simnet");
            let mut order: Vec<usize> = (0..recs.len()).collect();
            if shuffle == 1 {
                // Children first: InsertMemPoolTx accepts them unordered.
                order.reverse();
            }
            store
                .update(|tx| {
                    for i in &order {
                        ts.insert_mem_pool_tx(tx, &recs[*i]).expect("insert");
                    }
                    Ok::<_, purse_store::Error>(())
                })
                .expect("update");
            let mut r = store.read().expect("read");
            let got = ts.unmined_txs(&mut r).expect("unmined");
            assert_eq!(got.len(), recs.len());
            let pos: BTreeMap<Hash32, usize> = got
                .iter()
                .enumerate()
                .map(|(i, r)| (*r.hash(), i))
                .collect();
            for (i, rec) in got.iter().enumerate() {
                for input in &rec.msg_tx().tx_in {
                    if let Some(j) = pos.get(&Hash32(input.previous_out_point.hash.0)) {
                        assert!(*j < i, "a transaction before one it spends");
                    }
                }
            }
            orders.push(got.iter().map(|r| *r.hash()).collect::<Vec<_>>());
        }
        assert_eq!(orders[0], orders[1], "order depends on insertion order");
    }
}

/// A row in the next slice's tables (the ticket commitments and the
/// multisig outputs), which only a write outside the transaction store can
/// make, stops every operation that would read or change it with kind
/// `Bug`, before anything is written; every other operation is not stopped
/// by it. All 4 tables against every operation that has a guard, and
/// against the mutators that have none.
#[test]
fn the_guard_stops_on_later_slice_rows() {
    let (store, ts, _) = tx_store("simnet");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-guard"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    // Block 1 holds a mined regular transaction and a mined ticket; an
    // unmined transaction with inputs and an unmined one without wait.
    let mined = sim.build();
    let ticket = sim.build_ticket();
    let unmined = sim.build();
    let inputless = TxRecord::from_msg_tx(
        MsgTx {
            tx_out: vec![TxOut {
                value: 5,
                version: 0,
                pk_script: vec![0x6a],
            }],
            ..MsgTx::default()
        },
        0,
    )
    .expect("record");
    let fresh = sim.build();
    let b1 = store
        .update(|tx| {
            let (h, hash) = sim.header(&ts, tx);
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("extend");
            ts.insert_mined_tx(tx, &mined, &hash).expect("mined");
            ts.insert_mined_tx(tx, &ticket, &hash).expect("ticket");
            ts.insert_mem_pool_tx(tx, &unmined).expect("unmined");
            ts.insert_mem_pool_tx(tx, &inputless).expect("inputless");
            let _ = h;
            Ok::<_, purse_store::Error>(hash)
        })
        .expect("update");
    let ticket_details = {
        let mut r = store.read().expect("read");
        ts.tx_details(&mut r, ticket.hash()).expect("details")
    };
    let block1 = purse_udb::txstore::Block {
        hash: b1,
        height: 1,
    };
    // A header disapproving block 1, which holds transactions.
    let disapprove = |tx: &mut WriteTx<'_>| {
        let (mut h, _) = BlockHeader::from_bytes(&ts.get_block_header(tx, &b1).expect("header").0)
            .expect("parse");
        h.prev_block = Hash(b1.0);
        h.height = 2;
        h.vote_bits = 0;
        h.nonce ^= 0x5555;
        let hash = Hash32(*h.block_hash().as_bytes());
        ts.extend_main_chain(tx, &header_bytes(&h), &hash, Some(&[]))
    };
    // The operations, by letter:
    //   m insert_mined_tx of a new transaction with inputs
    //   M insert_mined_tx moving the unmined transaction without inputs
    //   r rollback(1)              u remove_unconfirmed
    //   d tx_details               D unique_tx_details
    //   R range_transactions       t iterate_tickets
    //   T ticket_details           e extend_main_chain disapproving block 1
    //   p insert_mem_pool_tx       x prune_unmined (removing nothing)
    //   s set_published            n unspent_output_count
    // (`ticket_details` reads the spender's details through `tx_details`,
    // so it stops for the tables that guard does.)
    const OPS: &str = "mMrudDRtTepxsn";
    for (table, stops) in [
        (TableId::Commitments, "ru"),
        (TableId::CommitmentsUnspent, "ru"),
        (TableId::MultisigOuts, "ru"),
        (TableId::MultisigUnspent, "ru"),
    ] {
        assert!(LATER_TABLES.contains(&table));
        for op in OPS.chars() {
            store
                .update(|tx| {
                    tx.put_raw(table, &[0u8; 36], &[0u8; 8]).expect("raw row");
                    let before = dump(tx);
                    let r = match op {
                        'm' => ts.insert_mined_tx(tx, &fresh, &b1),
                        'M' => ts.insert_mined_tx(tx, &inputless, &b1),
                        'r' => ts.rollback(tx, 1).map(drop),
                        'u' => ts.remove_unconfirmed(tx, unmined.msg_tx(), unmined.hash()),
                        'd' => ts.tx_details(tx, mined.hash()).map(drop),
                        'D' => ts
                            .unique_tx_details(tx, mined.hash(), Some(&block1))
                            .map(drop),
                        'R' => ts.range_transactions(tx, 0, -1, |_| Ok(false)),
                        't' => ts.iterate_tickets(tx).map(drop),
                        'T' => ts.ticket_details(tx, &ticket_details).map(drop),
                        'e' => disapprove(tx),
                        'p' => ts.insert_mem_pool_tx(tx, &fresh),
                        'x' => ts.prune_unmined(tx, 0).map(drop).map_err(|e| e.error),
                        's' => ts.set_published(tx, unmined.hash(), false),
                        _ => ts.unspent_output_count(tx).map(drop),
                    };
                    if stops.contains(op) {
                        let e = r.expect_err("the guard stops");
                        assert_eq!(e.as_kind(), Some(Kind::Bug), "{} {op}: {e:?}", table.name());
                        assert_eq!(dump(tx), before, "{} {op}: the guard wrote", table.name());
                    } else {
                        r.unwrap_or_else(|e| panic!("{} {op}: stopped: {e:?}", table.name()));
                    }
                    Err::<(), TErr>(TErr::Rollback)
                })
                .expect_err("rolled back");
        }
    }
    // The ticket set is not a later slice's table.
    store
        .update(|tx| {
            tx.put::<Tickets>(&Hash32([9; 32]), &purse_store::Empty)
                .expect("ticket");
            ts.rollback(tx, 1).expect("rollback");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// A reader that counts the rows it returns with their values, by table,
/// the keys it returns without values (`keys` and `contains`), and the
/// range lookups it makes, to hold the walks to what they visit and the
/// existence tests to reading no value.
struct Counting<'a> {
    inner: ReadTx,
    rows: &'a Cell<usize>,
    ranges: &'a Cell<usize>,
    keys: &'a Cell<usize>,
    by_table: &'a RefCell<BTreeMap<TableId, usize>>,
}

impl<'a> Counting<'a> {
    fn new(
        inner: ReadTx,
        rows: &'a Cell<usize>,
        ranges: &'a Cell<usize>,
        keys: &'a Cell<usize>,
        by_table: &'a RefCell<BTreeMap<TableId, usize>>,
    ) -> Counting<'a> {
        Counting {
            inner,
            rows,
            ranges,
            keys,
            by_table,
        }
    }

    fn reset(&self) {
        self.rows.set(0);
        self.ranges.set(0);
        self.keys.set(0);
        self.by_table.borrow_mut().clear();
    }

    fn values(&self, t: TableId, n: usize) {
        self.rows.set(self.rows.get() + n);
        *self.by_table.borrow_mut().entry(t).or_default() += n;
    }

    /// The rows read with their values from table `t`.
    fn read_from(&self, t: TableId) -> usize {
        self.by_table.borrow().get(&t).copied().unwrap_or(0)
    }
}

impl TxRead for Counting<'_> {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        let r = TxRead::get::<T>(&mut self.inner, key)?;
        if r.is_some() {
            self.values(T::ID, 1);
        }
        Ok(r)
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        let r = TxRead::contains::<T>(&mut self.inner, key)?;
        if r {
            self.keys.set(self.keys.get() + 1);
        }
        Ok(r)
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        self.ranges.set(self.ranges.get() + 1);
        let v = TxRead::rows::<T>(&mut self.inner, lo, hi, rev, limit)?;
        self.values(T::ID, v.len());
        Ok(v)
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        self.ranges.set(self.ranges.get() + 1);
        let v = TxRead::keys::<T>(&mut self.inner, lo, hi, rev, limit)?;
        self.keys.set(self.keys.get() + v.len());
        Ok(v)
    }
    fn table_is_empty(&mut self, t: TableId) -> purse_store::Result<bool> {
        TxRead::table_is_empty(&mut self.inner, t)
    }
    fn table_len(&mut self, t: TableId) -> purse_store::Result<u64> {
        TxRead::table_len(&mut self.inner, t)
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        self.inner.tip_row()
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        self.inner.birth_row()
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        self.inner.have_cfilters_row()
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        self.inner.processed_row()
    }
}

/// The range walks read about as many rows as they visit, as Go's cursor
/// walks do: `RangeTransactions`, `RangeBlocks` and
/// `MissingCFiltersHeight` stopped by their first answer read a handful
/// of rows on a chain of 2,048 blocks, and `RangeTransactions` reads no
/// row of a block without wallet transactions. The answers are checked
/// against the chain in both directions, a block's records in position
/// order across the walk's pages.
#[test]
fn range_walks_read_what_they_visit() {
    const N: u32 = 2048;
    const MISSING: u32 = 10;
    let (store, ts, _) = tx_store("testnet3");
    let empty = FilterV2::from_bytes(blockcf2::B, blockcf2::M, &[]).expect("empty filter");
    let mut g = BlockGenerator::new();
    let mut hashes = vec![Hash32(
        *dcroxide_chaincfg::testnet3_params().genesis_hash.as_bytes(),
    )];
    // Records: one in block 1, three in block 3, one in block N - 1.
    let placement = [1, 3, 3, 3, N - 1];
    let mut recs: Vec<(u32, TxRecord)> = Vec::new();
    store
        .update(|tx| {
            // Above the birthday no block may lack its filter; the birthday
            // is lowered once the chain is built.
            let birth = |height| BirthState {
                set_from_height: true,
                set_from_time: false,
                hash: Hash32::default(),
                height,
                time: 0,
            };
            purse_udb::txstore::set_birth_state(tx, &birth(N + 1)).expect("SetBirthState");
            for h in 1..=N {
                let hdr = g.generate(1);
                let hash = Hash32(*hdr.block_hash().as_bytes());
                let filter = (h != MISSING).then(|| empty.bytes());
                ts.extend_main_chain(tx, &header_bytes(&hdr), &hash, filter)
                    .expect("ExtendMainChain");
                hashes.push(hash);
            }
            for (i, h) in placement.iter().enumerate() {
                let m = MsgTx {
                    tx_out: vec![TxOut {
                        value: 1 + i64::try_from(i).expect("small"),
                        version: 0,
                        pk_script: Vec::new(),
                    }],
                    ..MsgTx::default()
                };
                let rec = TxRecord::from_msg_tx(m, 0).expect("record");
                ts.insert_mined_tx(tx, &rec, &hashes[*h as usize])
                    .expect("InsertMinedTx");
                recs.push((*h, rec));
            }
            purse_udb::txstore::set_birth_state(tx, &birth(5)).expect("SetBirthState");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");

    let (rows, ranges, keys) = (Cell::new(0), Cell::new(0), Cell::new(0));
    let by_table = RefCell::new(BTreeMap::new());
    let mut r = Counting::new(
        store.read().expect("read"),
        &rows,
        &ranges,
        &keys,
        &by_table,
    );

    // Stopped at the first block with records: its history row, the next
    // page of two (the walk knows block 1's records are all read when it
    // meets block 3's), the block row and the record. Before the walk was
    // paged, this call read all 2,049 block rows first.
    let mut calls = 0;
    ts.range_transactions(&mut r, 0, -1, |d| {
        calls += 1;
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].record.hash(), recs[0].1.hash());
        Ok(true)
    })
    .expect("RangeTransactions");
    assert_eq!(calls, 1);
    assert_eq!(rows.get(), 5, "a stopped walk read {} rows", rows.get());

    // A full walk each way: one call per block with records, in position
    // order within the block, and no read for the 2,045 empty blocks.
    let want: Vec<(i32, Vec<Hash32>)> = {
        let mut m: BTreeMap<u32, Vec<Hash32>> = BTreeMap::new();
        for (h, rec) in &recs {
            m.entry(*h).or_default().push(*rec.hash());
        }
        m.into_iter()
            .map(|(h, v)| (i32::try_from(h).expect("small"), v))
            .collect()
    };
    for (begin, end) in [(0, -1), (-1, 0)] {
        r.reset();
        let mut got = Vec::new();
        ts.range_transactions(&mut r, begin, end, |d| {
            let b = d[0].block.block;
            assert_eq!(b.hash, hashes[b.height as usize]);
            got.push((
                b.height,
                d.iter().map(|x| *x.record.hash()).collect::<Vec<_>>(),
            ));
            Ok(false)
        })
        .expect("RangeTransactions");
        let mut want = want.clone();
        if begin < 0 {
            want.reverse();
        }
        assert_eq!(got, want, "RangeTransactions({begin}, {end})");
        // Five history rows, three block rows, five records; the unmined
        // set is empty. Each record adds two range lookups, its credits'
        // and its debits' (none here), as Go's credit and debit iterators
        // seek once each.
        assert_eq!(rows.get(), 13, "RangeTransactions({begin}, {end}) rows");
        assert!(ranges.get() <= 5 + 2 * 5, "{} range lookups", ranges.get());
    }

    // RangeBlocks stopped at once reads one row, either way.
    for (begin, end, first) in [(0, -1, 0), (-1, 0, N), (7, 3, 7), (3, 7, 3)] {
        r.reset();
        let mut seen = Vec::new();
        ts.range_blocks(&mut r, begin, end, |b| {
            seen.push(*b);
            Ok(true)
        })
        .expect("RangeBlocks");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].height, i32::try_from(first).expect("small"));
        assert_eq!(seen[0].hash, hashes[first as usize]);
        assert_eq!(rows.get(), 1, "RangeBlocks({begin}, {end}) rows");
    }
    // A full RangeBlocks visits every block once, in order, with a range
    // lookup per page.
    r.reset();
    let mut heights = Vec::new();
    ts.range_blocks(&mut r, -1, 0, |b| {
        heights.push(b.height);
        Ok(false)
    })
    .expect("RangeBlocks");
    let want: Vec<i32> = (0..=i32::try_from(N).expect("small")).rev().collect();
    assert_eq!(heights, want);
    assert_eq!(rows.get(), N as usize + 1);
    assert!(ranges.get() <= 16, "{} range lookups", ranges.get());

    // MissingCFiltersHeight from the birthday (5) stops at block 10,
    // reading heights and testing filters without reading a block row's
    // or a filter's value: block heights 5 to 11 in pages of one, two and
    // four, and the filters of blocks 5 to 9 found present.
    r.reset();
    assert_eq!(
        ts.missing_cfilters_height(&mut r)
            .expect("MissingCFiltersHeight"),
        i32::try_from(MISSING).expect("small")
    );
    assert_eq!(rows.get(), 0, "MissingCFiltersHeight read values");
    assert_eq!(keys.get(), 7 + 5, "MissingCFiltersHeight keys");
}

/// A ticket purchase of the simulation's shape spending an outpoint no
/// other call's ticket spends (`n` distinct), so tickets never conflict.
fn unique_ticket(sim: &mut Sim, n: u32) -> TxRecord {
    let mut m = sim.build_ticket().msg_tx().clone();
    let mut b = [0xa5u8; 32];
    b[..4].copy_from_slice(&n.to_le_bytes());
    m.tx_in[0].previous_out_point = OutPoint {
        hash: Hash(b),
        index: 0,
        tree: 0,
    };
    let rec = TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
    assert_eq!(rec.tx_type(), TxType::SStx);
    rec
}

/// The existence tests and `UnminedTxHashes` read no value, as Go's
/// `Get(k) != nil` and key-only cursor loops copy none: `ExistsTx`,
/// `ExistsTxMinedOrUnmined`, `TxBlockHeight`, `OwnTicket` and
/// `UnminedTxHashes` answer from keys alone, mined, unmined and absent.
#[test]
fn existence_tests_read_no_value() {
    let (store, ts, _) = tx_store("testnet3");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-exists"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    let (mut mined, mut unmined) = (Vec::new(), Vec::new());
    store
        .update(|tx| {
            let (h, hash) = sim.header(&ts, tx);
            ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                .expect("extend");
            for n in 0..3 {
                let rec = unique_ticket(&mut sim, n);
                ts.insert_mined_tx(tx, &rec, &hash).expect("InsertMinedTx");
                mined.push(*rec.hash());
            }
            for n in 3..6 {
                let rec = unique_ticket(&mut sim, n);
                ts.insert_mem_pool_tx(tx, &rec).expect("InsertMemPoolTx");
                unmined.push(*rec.hash());
            }
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    let absent = Hash32([0xee; 32]);
    let (rows, ranges, keys) = (Cell::new(0), Cell::new(0), Cell::new(0));
    let by_table = RefCell::new(BTreeMap::new());
    let mut r = Counting::new(
        store.read().expect("read"),
        &rows,
        &ranges,
        &keys,
        &by_table,
    );
    for h in &mined {
        assert_eq!(
            ts.exists_tx_mined_or_unmined(&mut r, h).expect("q"),
            (true, false)
        );
        assert!(ts.exists_tx(&mut r, h).expect("q"));
        assert_eq!(ts.tx_block_height(&mut r, h).expect("q"), 1);
        assert!(ts.own_ticket(&mut r, h).expect("q"));
    }
    for h in &unmined {
        assert_eq!(
            ts.exists_tx_mined_or_unmined(&mut r, h).expect("q"),
            (false, true)
        );
        assert!(ts.exists_tx(&mut r, h).expect("q"));
        assert_eq!(ts.tx_block_height(&mut r, h).expect("q"), -1);
        assert!(ts.own_ticket(&mut r, h).expect("q"));
    }
    assert_eq!(
        ts.exists_tx_mined_or_unmined(&mut r, &absent).expect("q"),
        (false, false)
    );
    assert!(!ts.exists_tx(&mut r, &absent).expect("q"));
    let e = ts.tx_block_height(&mut r, &absent).expect_err("absent");
    assert_eq!(e.as_kind(), Some(Kind::NotExist), "{e:?}");
    assert!(!ts.own_ticket(&mut r, &absent).expect("q"));
    let mut want = unmined.clone();
    want.sort();
    assert_eq!(ts.unmined_tx_hashes(&mut r).expect("UnminedTxHashes"), want);
    assert_eq!(rows.get(), 0, "values read: {:?}", by_table.borrow());
    assert!(keys.get() > 0);
}

/// `IterateTickets` reads the block row of each distinct block holding a
/// mined ticket once, not once per ticket (Go reads none: its key carries
/// the block hash): 64 tickets mined in block 1 and two in block 2 cost two
/// block rows, and the answers name each ticket's block.
#[test]
fn iterate_tickets_reads_each_block_once() {
    let (store, ts, _) = tx_store("testnet3");
    let mut sim = Sim {
        rng: SplitMix64::from_env("purse-udb-txstore-tickets"),
        recs: Vec::new(),
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
    };
    let mut want: BTreeMap<Hash32, i32> = BTreeMap::new();
    let mut n = 0;
    store
        .update(|tx| {
            for (height, count) in [(1, 64), (2, 2)] {
                let (h, hash) = sim.header(&ts, tx);
                ts.extend_main_chain(tx, &h, &hash, Some(&[]))
                    .expect("extend");
                sim.blocks.push(hash);
                for _ in 0..count {
                    let rec = unique_ticket(&mut sim, n);
                    n += 1;
                    ts.insert_mined_tx(tx, &rec, &hash).expect("InsertMinedTx");
                    want.insert(*rec.hash(), height);
                }
            }
            let rec = unique_ticket(&mut sim, n);
            ts.insert_mem_pool_tx(tx, &rec).expect("InsertMemPoolTx");
            want.insert(*rec.hash(), -1);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    let (rows, ranges, keys) = (Cell::new(0), Cell::new(0), Cell::new(0));
    let by_table = RefCell::new(BTreeMap::new());
    let mut r = Counting::new(
        store.read().expect("read"),
        &rows,
        &ranges,
        &keys,
        &by_table,
    );
    let tickets = ts.iterate_tickets(&mut r).expect("IterateTickets");
    let got: BTreeMap<Hash32, i32> = tickets
        .iter()
        .map(|t| (*t.record.hash(), t.block.height))
        .collect();
    assert_eq!(got, want);
    for t in &tickets {
        let block = match t.block.height {
            -1 => Hash32::default(),
            h => sim.blocks[usize::try_from(h - 1).expect("height")],
        };
        assert_eq!(t.block.hash, block, "ticket {:?}", t.record.hash());
    }
    assert_eq!(r.read_from(TableId::Blocks), 2, "block rows read");
    assert_eq!(r.read_from(TableId::TxMined), 66, "mined records read");
    assert_eq!(r.read_from(TableId::TxUnmined), 1, "unmined records read");
    assert_eq!(r.read_from(TableId::Tickets), 0, "ticket values read");
    assert_eq!(keys.get(), 67, "ticket keys read");
}

/// A regular transaction paying the wallet (output 0, 1,100 atoms) and a
/// ticket spending that output, for the tests below that need a ticket
/// whose debit spends a regular credit of its own block.
fn funded_ticket() -> (TxRecord, TxRecord) {
    let a_tx = MsgTx {
        tx_in: vec![TxIn {
            previous_out_point: OutPoint {
                hash: Hash([9u8; 32]),
                index: 0,
                tree: 0,
            },
            sequence: u32::MAX,
            value_in: 5000,
            ..TxIn::default()
        }],
        tx_out: vec![TxOut {
            value: 1100,
            version: 0,
            pk_script: p2pkh(None, 3),
        }],
        ..MsgTx::default()
    };
    let mut commitment = vec![7u8; 20];
    commitment.extend_from_slice(&1100i64.to_le_bytes());
    commitment.extend_from_slice(&0x5800u16.to_le_bytes());
    let t_tx = MsgTx {
        tx_in: vec![TxIn {
            previous_out_point: OutPoint {
                hash: a_tx.tx_hash(),
                index: 0,
                tree: 0,
            },
            sequence: u32::MAX,
            value_in: 1100,
            ..TxIn::default()
        }],
        tx_out: vec![
            TxOut {
                value: 1000,
                version: 0,
                pk_script: p2pkh(Some(0xba), 4),
            },
            TxOut {
                value: 0,
                version: 0,
                pk_script: op_return(&commitment),
            },
            TxOut {
                value: 0,
                version: 0,
                pk_script: p2pkh(Some(0xbd), 5),
            },
        ],
        ..MsgTx::default()
    };
    let a = TxRecord::from_msg_tx(a_tx, 1_795_400_000).expect("record");
    let t = TxRecord::from_msg_tx(t_tx, 1_795_400_000).expect("record");
    assert_eq!(a.tx_type(), TxType::Regular);
    assert_eq!(t.tx_type(), TxType::SStx);
    (a, t)
}

/// `acct_history`'s rule that a debit names its credit's account only
/// while that credit names the debit as its spender. A ticket mined in
/// block 1 spends a regular credit of block 1; block 2 disapproves block
/// 1, which moves the credit, spender kept, to the invalidated table and
/// leaves the ticket's debit live (stake invalidation moves only regular
/// transactions' rows); `AddCredit` then overwrites the invalidated credit
/// with another account and no spender. The ticket's live debit now names
/// a credit that does not name it, and its record names no account; the
/// row checker recomputes `acct_history` from the rows after each step.
/// No generated script or random sequence reaches this state; the same
/// steps run against Go, through what Go exposes, at the end of
/// `33-txstore-credit-edges`.
#[test]
fn an_overwritten_invalidated_credit_drops_its_old_spenders_account() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let block1 = g.generate(1);
    let block2 = g.generate(0);
    let (a, t) = funded_ticket();
    let h1 = Hash32(*block1.block_hash().as_bytes());
    let accounts_of = |tx: &mut WriteTx<'_>, hash: &Hash32| -> BTreeSet<u32> {
        all::<AcctHistory>(tx)
            .into_iter()
            .filter(|(_, h)| h == hash)
            .map(|(k, _)| k.account)
            .collect()
    };
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&block1]);
            ts.insert_mined_tx(tx, &a, &h1).expect("InsertMinedTx a");
            ts.add_credit(tx, &a, Some(&make_block_meta(&block1)), 0, false, 1)
                .expect("AddCredit a");
            ts.insert_mined_tx(tx, &t, &h1).expect("InsertMinedTx t");
            check_rows(tx);
            assert_eq!(accounts_of(tx, t.hash()), BTreeSet::from([1]));
            insert_main_chain_headers(&ts, tx, &[&block2]);
            check_rows(tx);
            assert_eq!(all::<CreditsInvalidated>(tx).len(), 1);
            assert_eq!(all::<Debits>(tx).len(), 1, "the ticket's debit stays live");
            assert_eq!(accounts_of(tx, t.hash()), BTreeSet::from([1]));
            ts.add_credit(tx, &a, Some(&make_block_meta(&block1)), 0, false, 2)
                .expect("AddCredit over the invalidated credit");
            let inv = all::<CreditsInvalidated>(tx);
            assert_eq!(inv.len(), 1);
            assert_eq!(inv[0].1.account, 2);
            assert_eq!(inv[0].1.spender, None);
            check_rows(tx);
            assert_eq!(
                accounts_of(tx, t.hash()),
                BTreeSet::new(),
                "the ticket's debit no longer names an account"
            );
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// `UnspentTickets` without immature tickets at Go's `ticketMatured`
/// boundary (a ticket mined at height h is mature from the height
/// `h + TicketMaturity + 1`, `TicketMaturity` being 16 on testnet3) and
/// at the `int32` wrap Go's subtraction makes
/// (QK-0784, reproduced: a sync height more than 2^31 below h counts the
/// ticket as mature); and its `IO` where an unmined-inputs entry names a
/// spender with no unmined record, at which Go panics (QK-0048, ADR-0012
/// Rule 2(a)). No operation writes that entry without its record, so the
/// test writes it directly.
#[test]
fn unspent_tickets_follow_ticket_matured_and_refuse_a_dangling_spender() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let block1 = g.generate(1);
    let (a, t) = funded_ticket();
    let h1 = Hash32(*block1.block_hash().as_bytes());
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&block1]);
            ts.insert_mined_tx(tx, &a, &h1).expect("InsertMinedTx a");
            ts.insert_mined_tx(tx, &t, &h1).expect("InsertMinedTx t");
            ts.add_credit(tx, &t, Some(&make_block_meta(&block1)), 0, false, 0)
                .expect("AddCredit t");
            let ticket = vec![*t.hash()];
            for (sync, want) in [
                (1, false),
                (16, false),
                (17, false),
                (18, true),
                (i32::MAX, true),
                // 1 - 2^31: the difference is i32::MIN, no wrap.
                (-2_147_483_647, false),
                // 2^31 + 1 below: Go's difference wraps to i32::MAX.
                (i32::MIN, true),
            ] {
                let got = ts.unspent_tickets(tx, sync, false).expect("UnspentTickets");
                let want = if want { ticket.clone() } else { Vec::new() };
                assert_eq!(got, want, "sync height {sync}");
                assert_eq!(
                    ts.unspent_tickets(tx, sync, true).expect("UnspentTickets"),
                    ticket,
                    "sync height {sync}, immature included"
                );
            }
            let op = purse_store::Outpoint {
                hash: *t.hash(),
                index: 0,
            };
            tx.put::<UnminedInputs>(&op, &Hash32([0x42; 32]))
                .expect("put");
            let e = ts
                .unspent_tickets(tx, 300, true)
                .expect_err("a spender with no unmined record");
            assert_eq!(e.as_kind(), Some(Kind::Io), "{e:?}");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// QK-0785 (reproduced, pending owner): a ticket mined in block 1 spends a
/// regular credit of block 1; block 2 disapproves block 1, which moves the
/// credit, spender kept, to the invalidated table and leaves the ticket's
/// debit live; `Rollback` over block 1 then fails with `IO` and writes
/// nothing, and once block 2 is detached and another block 2 approves
/// block 1, stake validation moves the credit back with an unspent entry
/// although it is still spent, as Go's does: an unspent output, counted as
/// spendable, whose credit `TxDetails` reports spent. The row checker
/// accepts an unspent output with a spent credit in this state alone (with
/// a stake spender, as here, or a spender in another block); Go's answers for the same steps are pinned by
/// `a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again`
/// in `tests/oracle.rs`.
#[test]
fn a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let block1 = g.generate(1);
    let mut g2 = BlockGenerator {
        last_hash: g.last_hash,
        last_height: g.last_height,
    };
    let block2 = g.generate(0);
    let block2x = g2.generate(1);
    let (a, t) = funded_ticket();
    let h1 = Hash32(*block1.block_hash().as_bytes());
    let a_out = |tx: &mut WriteTx<'_>| {
        ts.unspent_outputs(tx)
            .expect("UnspentOutputs")
            .into_iter()
            .filter(|c| c.outpoint.hash.as_bytes() == &a.hash().0)
            .count()
    };
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&block1]);
            ts.insert_mined_tx(tx, &a, &h1).expect("InsertMinedTx a");
            ts.add_credit(tx, &a, Some(&make_block_meta(&block1)), 0, false, 0)
                .expect("AddCredit a");
            ts.insert_mined_tx(tx, &t, &h1).expect("InsertMinedTx t");
            ts.add_credit(tx, &t, Some(&make_block_meta(&block1)), 0, false, 0)
                .expect("AddCredit t");
            insert_main_chain_headers(&ts, tx, &[&block2]);
            check_rows(tx);
            assert_eq!(ts.unspent_output_count(tx).expect("count"), 1);
            let before = dump(tx);
            let e = ts.rollback(tx, 1).expect_err("the missing credit");
            assert_eq!(e.as_kind(), Some(Kind::Io), "{e:?}");
            assert!(format!("{e}").contains("missing credit"), "{e}");
            assert_eq!(dump(tx), before, "a refused Rollback writes nothing");
            ts.rollback(tx, 2).expect("Rollback of block 2");
            insert_main_chain_headers(&ts, tx, &[&block2x]);
            check_rows(tx);
            assert_eq!(ts.unspent_output_count(tx).expect("count"), 2);
            assert_eq!(a_out(tx), 1, "a's spent output listed as unspent");
            let b = ts.account_balances(tx, 0).expect("AccountBalances");
            assert_eq!((b[0].spendable, b[0].voting_authority), (1100, 1000));
            let d = ts.tx_details(tx, a.hash()).expect("TxDetails");
            assert!(d.credits[0].spent, "{:?}", d.credits);
            ts.rollback(tx, 1).expect("Rollback over block 1");
            check_rows(tx);
            assert_eq!(a_out(tx), 0);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}
