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
//!   never compares, QK-0650). These three run Rust only here; the
//!   hand-written script `38-upstream-tx-tests` replays the same steps,
//!   with the same queries after each, through Go and the Rust runner
//!   (`tests/oracle.rs`, `tests/frozen.rs`), and `tests/guards.rs` checks
//!   that `TestCoinbases`' balances asserted here
//!   (`support::UPSTREAM_COINBASE_BALANCES`) are the ones Go returned.
//!
//! Properties (random sequences from `PURSE_TEST_SEED`, over every
//! operation of the slice on a store with no address manager, which the
//! transaction store never reads): the rows' invariants after every
//! operation (`check_rows`, and `purse-store`'s `check_chain`); refusals
//! writing nothing; a reopened store answering as the live one did;
//! `TxRecord` round trips; the dependency order of `unmined_txs`; every
//! selection, `RandomUTXO` and `Spender` answer against the rows; the
//! rows the range walks read, the
//! values the existence tests do not read, and the block rows
//! `IterateTickets` reads (one per block, not per ticket). Two targeted
//! tests reach states no random sequence does: a ticket's live debit whose
//! credit `AddCredit` overwrote in an invalidated block (`acct_history`'s
//! spender rule), and `UnspentTickets` at Go's maturity boundary, at its
//! `int32` wrap (QK-0784) and over a dangling unmined-inputs entry
//! (QK-0048). Another gives `Spender`'s refusals over unmined-inputs
//! entries written directly: one naming no unmined record, one naming an
//! unmined transaction that does not spend the output, and one naming the
//! zero hash. QK-0734's fix (the merge of a revalidated transaction's
//! unmined copy) is reached by planned revalidations inside the random
//! sequences, and by its own test in eight variants, each step checked
//! against the rows and across a reopen.
//!
//! The library's consistency check of the transaction tables (ADR-0018
//! BK4: `purse-store`'s `check_tx_tables` and `txstore::check_decoded`)
//! runs inside `check_rows`, so every state above passes it, and the reopen
//! steps run it on the read transaction with the chain and record checks.
//! `each_broken_invariant_of_the_transaction_tables_is_refused` breaks
//! each invariant and each sub-rule of one by one direct row edit (55
//! edits) on a store the operations built, each refused by the check that
//! holds it with its text and passing once undone, and writes four states
//! of Go's reproduced defects that the check accepts;
//! `verify_backup_runs_the_transaction_tables_check` takes the same edits
//! through `txstore::verify_backup`'s path;
//! `the_schema_fixture_passes_the_full_verify` verifies `purse-store`'s
//! checked-in fixture with it;
//! `verify_accepts_an_agenda_key_at_dcrwallet_s_limit` verifies a store
//! holding the auxiliary records' longest keys (a 32,800-byte
//! `ticket_agenda_prefs` key) beside the transaction tables; and
//! `every_state_of_go_s_reproduced_defects_passes_the_check` writes, by the
//! operations alone, each state of the transaction tables that a defect of
//! Go's reproduced in docs/QUIRKS.md leaves (22 states, 16 QK numbers),
//! asserts the rows that make it, and holds the check, the read side's
//! checks and `txstore::verify_backup_on_backend` of a backup to accept it.

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
    Blocks, Commitments, CommitmentsUnspent, CreditKey, Credits, CreditsInvalidated, Debits,
    Hash32, HeaderBytes, History, MultisigOuts, MultisigUnspent, MultisigUnspentKey, ReadTx, Store,
    Table, TableId, Tip, TxMined, TxUnmined, UnminedCredits, UnminedInputs, Unpublished, Unspent,
    WriteTx,
};
use purse_testutil::SplitMix64;
use purse_udb::addrmgr::ChainParams;
use purse_udb::txstore::{BlockMeta, TxRead, TxRecord, TxStore, mined_class, unmined_class};
use support::{UPSTREAM_COINBASE_BALANCES, scratch};

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
/// immature amount twice, QK-0650); the port compares both, against
/// `support::UPSTREAM_COINBASE_BALANCES`, the values Go returns for these
/// steps in `38-upstream-tx-tests`.
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
            let mut steps = UPSTREAM_COINBASE_BALANCES.iter();
            let mut maturity = |tx: &mut WriteTx<'_>| {
                let (step, immature, spendable) = steps.next().expect("a step");
                let b = ts.account_balance(tx, 0, 0).expect("AccountBalance");
                assert_eq!(b.immature_coinbase_rewards, *immature, "{step}: immature");
                assert_eq!(b.spendable, *spendable, "{step}: spendable");
                check_rows(tx);
                check_utxo_queries(&ts, tx);
            };
            // Block 1.
            maturity(tx);
            insert_main_chain_headers(&ts, tx, &hs[1..7]);
            // Block 7.
            maturity(tx);
            insert_main_chain_headers(&ts, tx, &hs[7..13]);
            // Block 13.
            maturity(tx);
            insert_main_chain_headers(&ts, tx, &hs[13..16]);
            // Block 16.
            maturity(tx);
            insert_main_chain_headers(&ts, tx, &hs[16..17]);
            // Block 17.
            maturity(tx);
            // Spending output 0 leaves output 2 mature.
            let spender = spend_output(Hash(cb_rec.hash().0), 0, 0, &[500_000_000, 1_500_000_000]);
            let spender_rec = TxRecord::from_msg_tx(spender, 1_795_300_000).expect("record");
            let b17h = Hash32(*headers[16].block_hash().as_bytes());
            ts.insert_mined_tx(tx, &spender_rec, &b17h)
                .expect("InsertMinedTx");
            // Spent at block 17.
            maturity(tx);
            // Block 17 rolled back: output 2 immature again, output 0
            // spent by the returned (unmined) spender.
            ts.rollback(tx, 17).expect("Rollback");
            maturity(tx);
            // Block 1 rolled back: the coinbase is gone.
            ts.rollback(tx, 1).expect("Rollback");
            maturity(tx);
            assert!(steps.next().is_none(), "every step asserted");
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
        plan: Vec::new(),
        vote: None,
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
        plan: Vec::new(),
        vote: None,
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
        plan: Vec::new(),
        vote: None,
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
    // Every unmined credit belongs to an unmined record (Go's
    // `RemoveUnconfirmed`, `moveMinedTx` and QK-0734's fix,
    // `merge_unmined_copy`, delete them together). An outpoint that is both
    // an unspent output and an unmined credit, Go's state after a
    // revalidation of a transaction also unmined (QK-0734, fixed), is not
    // refused by the rows alone: QK-0781's state (one transaction recorded
    // in two blocks, the higher one rolled back, reproduced) holds the same
    // rows once the lower block's credit and an unmined credit are added,
    // so the tests of the fix check after each approval that the
    // revalidated transaction is mined only, and `selectable` takes such
    // an outpoint twice, as Go's `MakeInputSource` does.
    for (op, _) in all::<UnminedCredits>(tx) {
        assert!(
            tx.contains::<TxUnmined>(&op.hash).expect("contains"),
            "unmined credit {op:?} without its unmined record"
        );
    }
    check_commitment_rows(tx);
    check_multisig_rows(tx);
    check_credit_rows(tx);
    tx.check_chain_rows();
    check_library(tx);
}

/// The library's consistency check of the transaction tables (ADR-0018
/// BK4): `purse-store`'s `check_tx_tables` and `purse-udb`'s
/// `check_decoded`, which `verify_backup` runs, must pass every state the
/// recomputation above passes, so that the check never refuses a store the
/// operations (and so Go's reproduced defects) legitimately leave. Every
/// call of `check_rows` runs it, after every step of every random
/// sequence and targeted test.
fn check_library(tx: &mut WriteTx<'_>) {
    if let Err(e) = tx.check_tx_tables() {
        panic!("check_tx_tables refuses a state the row checker passes: {e}");
    }
    if let Err(e) = purse_udb::txstore::check_decoded(tx) {
        panic!("check_decoded refuses a state the row checker passes: {e}");
    }
}

/// The same checks on a read transaction, with the record tables' (the
/// multisig index) and the chain's, as `verify_backup` runs them.
fn check_library_read(r: &ReadTx) {
    r.check_chain().expect("check_chain");
    r.check_records().expect("check_records");
    r.check_tx_tables().expect("check_tx_tables");
    let mut r = r;
    purse_udb::txstore::check_decoded(&mut r).expect("check_decoded");
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

/// A seeded stream for the store's random selections: dcrd's generator
/// keyed directly, as the operation-script runner keys it.
struct Stream(dcroxide_crypto::rand::Prng);

impl purse_snacl::RandomSource for Stream {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), purse_snacl::RandomSourceError> {
        self.0.read(buf);
        Ok(())
    }
}

fn stream(key: u8) -> Stream {
    Stream(dcroxide_crypto::rand::Prng::from_seed([key; 32]))
}

/// The outputs `SelectInputs` may select for `account` at `min_conf`
/// confirmations at `sync`, recomputed from the rows by dcrwallet's rules
/// (`MakeInputSource`): the unspent outputs no unmined transaction spends,
/// confirmed, of the account, not spent, not zero, not a ticket output,
/// mature, paying a P2PKH or P2PK script; and at `min_conf` 0 the unmined
/// credits of published transactions of the account without a stake tag
/// paying such a script (whether or not an unmined transaction spends
/// them, QK-0105). Each as (outpoint, tree, amount), with the number of
/// times it is taken: twice for an outpoint that is both such an unspent
/// output and such an unmined credit, which `MakeInputSource` enumerates
/// from both buckets (its `seen` set is filled only by the random phase,
/// which never runs at `min_conf` 0). That state is QK-0781's (a
/// transaction recorded mined in two blocks, the higher one rolled back,
/// then credited without a block; reproduced, and selected twice by Go
/// too: `a_transaction_mined_twice_and_rolled_back_is_listed_counted_and_selected_twice`
/// in `tests/oracle.rs`). The rows alone do not tell it from QK-0734's
/// state, whose path is fixed (`check_rows`); the random sequences assert
/// after each planned approval that the revalidated transaction is mined
/// only.
fn selectable(tx: &mut WriteTx<'_>, account: u32, min_conf: i32, sync: i32) -> Selectable {
    use dcroxide_txscript::stdscript::{ScriptType, determine_script_type};
    use purse_store::OutputTag;
    let params = dcroxide_chaincfg::simnet_params();
    let (cb, change) = (
        i32::from(params.coinbase_maturity),
        i32::from(params.sstx_change_maturity),
    );
    let confirmed = |h: i32| {
        let c = if h == -1 || h > sync {
            0
        } else {
            sync.wrapping_sub(h).wrapping_add(1)
        };
        c >= min_conf
    };
    let matured = |m: i32, h: i32| h >= 0 && sync.wrapping_sub(h).wrapping_add(1) > m;
    let payable = |script: &[u8]| {
        let (sub, _) =
            purse_txrules::rules::stake_sub_script_type(determine_script_type(0, script));
        matches!(
            sub,
            ScriptType::PubKeyHashEcdsaSecp256k1 | ScriptType::PubKeyEcdsaSecp256k1
        )
    };
    let uinputs: BTreeSet<purse_store::Outpoint> = all::<UnminedInputs>(tx)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let mut want = Selectable::new();
    for (op, u) in all::<Unspent>(tx) {
        let h = i32::try_from(u.height).expect("height");
        if uinputs.contains(&op) || !confirmed(h) {
            continue;
        }
        let c = tx
            .get::<Credits>(&CreditKey {
                hash: op.hash,
                height: u.height,
                index: op.index,
            })
            .expect("get")
            .expect("an unspent output's credit");
        if c.account != account || c.spender.is_some() || c.amount == 0 {
            continue;
        }
        let mature = match c.flags.tag {
            OutputTag::Ticket => false,
            OutputTag::NonStake if c.flags.coinbase => matured(cb, h),
            OutputTag::Vote | OutputTag::Revocation | OutputTag::TreasuryGen => matured(cb, h),
            OutputTag::TicketChange => matured(change, h),
            OutputTag::NonStake => true,
        };
        let rec = tx
            .get::<TxMined>(&purse_store::TxRecKey {
                hash: op.hash,
                height: u.height,
            })
            .expect("get")
            .expect("record");
        let (m, _) = MsgTx::from_bytes(&rec.tx).expect("record decodes");
        if mature && payable(&m.tx_out[op.index as usize].pk_script) {
            let tree = i8::from(c.flags.tag != OutputTag::NonStake);
            *want.entry((op, tree, c.amount)).or_default() += 1;
        }
    }
    if min_conf == 0 {
        let unpublished: BTreeSet<Hash32> =
            all::<Unpublished>(tx).into_iter().map(|(k, _)| k).collect();
        for (op, m) in all::<UnminedCredits>(tx) {
            if unpublished.contains(&op.hash)
                || m.flags.tag != OutputTag::NonStake
                || m.account != account
            {
                continue;
            }
            let row = tx.get::<TxUnmined>(&op.hash).expect("get").expect("record");
            let (t, _) = MsgTx::from_bytes(&row.tx).expect("record decodes");
            if payable(&t.tx_out[op.index as usize].pk_script) {
                // An outpoint also taken from `unspent` above is taken
                // twice (QK-0781's state).
                *want.entry((op, 0, m.amount)).or_default() += 1;
            }
        }
    }
    want
}

/// Selected outputs as (outpoint, tree, amount), with the number of times
/// each is taken.
type Selectable = BTreeMap<(purse_store::Outpoint, i8, i64), usize>;

/// The number of outpoints `want` takes more than once.
fn doubled(want: &Selectable) -> usize {
    let mut n: BTreeMap<&purse_store::Outpoint, usize> = BTreeMap::new();
    for ((op, _, _), k) in want {
        *n.entry(op).or_default() += k;
    }
    n.values().filter(|&&k| k > 1).count()
}

/// Input selection, `RandomUTXO` and `Spender` against the rows: every
/// output selection may take is taken by a selection of everything
/// (target 0), as many times as [`selectable`] counts (once, or twice in
/// QK-0781's state), with the amount their sum; a selection for a target
/// takes only such outputs, no more times than that, and all of them when
/// it falls short; `RandomUTXO` gives an unspent output no unmined
/// transaction spends, confirmed, or none when there is none; and
/// `Spender` answers from each credit's spender or the unmined-inputs
/// index. Returns the outputs selectable, the `RandomUTXO` calls that
/// found one, and the outpoints selectable twice, summed over the
/// accounts and depths checked.
fn check_selection(ts: &TxStore, tx: &mut WriteTx<'_>, key: u8) -> (usize, usize, usize) {
    let (_, tip) = ts.main_chain_tip(tx).expect("tip");
    let as_multiset = |d: &purse_txauthor::InputDetail| {
        let mut taken = Selectable::new();
        let mut sum = 0i64;
        for input in &d.inputs {
            let op = purse_store::Outpoint {
                hash: Hash32(input.previous_out_point.hash.0),
                index: input.previous_out_point.index,
            };
            *taken
                .entry((op, input.previous_out_point.tree, input.value_in))
                .or_default() += 1;
            sum = sum.wrapping_add(input.value_in);
        }
        assert_eq!(d.amount.0, sum, "the amount is the inputs' sum");
        assert_eq!(d.scripts.len(), d.inputs.len());
        assert_eq!(d.redeem_script_sizes.len(), d.inputs.len());
        taken
    };
    let (mut selected, mut found, mut twice) = (0, 0, 0);
    for account in [0, 1] {
        for min_conf in [0, 1, 2] {
            let want = selectable(tx, account, min_conf, tip);
            selected += want.values().sum::<usize>();
            twice += doubled(&want);
            let mut rng = stream(key);
            let mut src = ts
                .make_input_source(tx, &mut rng, account, min_conf, tip, None)
                .expect("source");
            let all_of = as_multiset(src.select_inputs(purse_dcrutil::Amount(0)).expect("select"));
            assert_eq!(all_of, want, "account {account} minconf {min_conf}");
            let target = 3000;
            let mut rng = stream(key.wrapping_add(1));
            let mut src = ts
                .make_input_source(tx, &mut rng, account, min_conf, tip, None)
                .expect("source");
            let d = src
                .select_inputs(purse_dcrutil::Amount(target))
                .expect("select")
                .clone();
            let some = as_multiset(&d);
            for (k, n) in &some {
                assert!(
                    want.get(k).is_some_and(|w| n <= w),
                    "{k:?} selected {n} times, outside the selectable outputs {want:?}"
                );
            }
            if d.amount.0 < target {
                assert_eq!(some, want, "short of the target with outputs left");
            }
        }
    }
    let uinputs: BTreeMap<purse_store::Outpoint, Hash32> =
        all::<UnminedInputs>(tx).into_iter().collect();
    let pickable: BTreeSet<purse_store::Outpoint> = all::<Unspent>(tx)
        .into_iter()
        .filter(|(op, u)| !uinputs.contains_key(op) && u.height <= u32::try_from(tip).unwrap_or(0))
        .map(|(op, _)| op)
        .collect();
    let mut rng = stream(key);
    match ts.random_utxo(tx, &mut rng, 1, tip).expect("RandomUTXO") {
        Some(c) => {
            found += 1;
            assert!(pickable.contains(&purse_store::Outpoint {
                hash: Hash32(c.outpoint.hash.0),
                index: c.outpoint.index
            }));
        }
        None => assert!(pickable.is_empty(), "RandomUTXO found none of {pickable:?}"),
    }
    let latest = |tx: &mut WriteTx<'_>, h: &Hash32| {
        all::<TxMined>(tx)
            .into_iter()
            .filter(|(k, _)| k.hash == *h)
            .map(|(k, _)| k.height)
            .max()
    };
    let unmined_spender = |tx: &mut WriteTx<'_>, op: &purse_store::Outpoint, h: &Hash32| {
        let row = tx
            .get::<TxUnmined>(h)
            .expect("get")
            .expect("the spender's record");
        let (m, _) = MsgTx::from_bytes(&row.tx).expect("decodes");
        m.tx_in
            .iter()
            .position(|i| {
                i.previous_out_point.hash.0 == op.hash.0 && i.previous_out_point.index == op.index
            })
            .map(|i| u32::try_from(i).expect("small"))
    };
    let wire = |op: &purse_store::Outpoint| OutPoint {
        hash: Hash(op.hash.0),
        index: op.index,
        tree: 0,
    };
    for (ck, c) in all::<Credits>(tx) {
        if latest(tx, &ck.hash) != Some(ck.height) {
            continue;
        }
        let op = purse_store::Outpoint {
            hash: ck.hash,
            index: ck.index,
        };
        let got = ts.spender(tx, &wire(&op));
        match (c.spender, uinputs.get(&op)) {
            // QK-0095 over QK-0781, then a rollback of the spender's
            // block, leaves a credit naming a spender with no record
            // there; Go's `Spender` fails to read it (kind `IO`).
            (Some(sp), _)
                if !tx
                    .contains::<TxMined>(&purse_store::TxRecKey {
                        hash: sp.hash,
                        height: sp.height,
                    })
                    .expect("contains") =>
            {
                assert_eq!(
                    got.expect_err("the spender's record is gone").as_kind(),
                    Some(Kind::Io)
                );
            }
            (Some(sp), _) => {
                let (m, i) = got.expect("Spender of a spent credit");
                assert_eq!((Hash32(*m.tx_hash().as_bytes()), i), (sp.hash, sp.input));
            }
            (None, Some(h)) => match unmined_spender(tx, &op, h) {
                Some(i) => {
                    let (m, gi) = got.expect("Spender of an output an unmined tx spends");
                    assert_eq!((Hash32(*m.tx_hash().as_bytes()), gi), (*h, i));
                }
                None => assert_eq!(got.expect_err("not spent").as_kind(), Some(Kind::NotExist)),
            },
            (None, None) => {
                assert_eq!(got.expect_err("unspent").as_kind(), Some(Kind::NotExist));
            }
        }
    }
    for (op, _) in all::<UnminedCredits>(tx) {
        if latest(tx, &op.hash).is_some() {
            continue;
        }
        let got = ts.spender(tx, &wire(&op));
        match uinputs.get(&op) {
            Some(h) => {
                let i = unmined_spender(tx, &op, h).expect("the spender spends it");
                let (m, gi) = got.expect("Spender of an unmined credit");
                assert_eq!((Hash32(*m.tx_hash().as_bytes()), gi), (*h, i));
            }
            None => assert_eq!(got.expect_err("unspent").as_kind(), Some(Kind::NotExist)),
        }
    }
    (selected, found, twice)
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
/// - an unspent output's credit names no spender, with two exceptions
///   from Go's stake validation, which moves an invalidated credit back,
///   spender kept, with an unspent entry, and re-spends only what the
///   validated block's own debits spend (Go makes both states and
///   dcr-purse reproduces them): the state of QK-0785, a credit whose
///   spender is a live debit of it that stake invalidation of the credit's
///   block does not move, that of a stake transaction or of a transaction
///   recorded in another block (below the credit's, QK-0101's state); and
///   QK-0095 over QK-0781, a credit whose spender's debit, live or
///   invalidated, is gone or spends the outpoint's credit at another
///   height;
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
    // A commitment counts in (account, Locked) while its unspent entry
    // exists without the unmined-spent flag (`balanceFullScan`'s last
    // pass).
    for (op, u) in all::<CommitmentsUnspent>(tx) {
        if u.unmined_spent {
            continue;
        }
        let c = tx
            .get::<Commitments>(&op)
            .expect("get")
            .expect("checked by check_commitment_rows");
        add(c.account, BalanceClass::Locked, 0, c.amount);
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

/// The commitment rows' invariant: every unspent entry has its commitment
/// (Go writes an unspent entry only for a commitment it tracks and deletes
/// both together). The `Locked` sums are checked with the other balance
/// sums (`check_credit_rows`).
fn check_commitment_rows(tx: &mut WriteTx<'_>) {
    for (op, _) in all::<CommitmentsUnspent>(tx) {
        assert!(
            tx.contains::<Commitments>(&op).expect("contains"),
            "an unspent commitment entry {op:?} without its commitment"
        );
    }
}

/// The multisig rows' invariant (`purse-store`'s `check_records`, through
/// the write transaction): `multisig_unspent` holds exactly one entry per
/// unspent record, under the record's script hash.
fn check_multisig_rows(tx: &mut WriteTx<'_>) {
    let want: BTreeSet<MultisigUnspentKey> = all::<MultisigOuts>(tx)
        .into_iter()
        .filter(|(_, row)| row.spender.is_none())
        .map(|(op, row)| MultisigUnspentKey {
            script_hash: row.script_hash,
            outpoint: op,
        })
        .collect();
    let have: BTreeSet<MultisigUnspentKey> = all::<MultisigUnspent>(tx)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(have, want, "multisig_unspent against the records");
}

/// Whether an unspent output's credit `ck` may name the spender `s`:
/// `s`'s debit, live or invalidated, is gone or spends the outpoint's
/// credit at another height (QK-0095 over QK-0781, the credit's block
/// then validated again), or `s` is a live debit of `ck` that stake
/// invalidation of the credit's block leaves in place, by a mined
/// transaction that is not regular or that is recorded at another height
/// (QK-0785).
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
    let other_height = |d: &purse_store::DebitRow| {
        (d.credit.hash, d.credit.index) == (ck.hash, ck.index) && d.credit.height != ck.height
    };
    let Some(d) = tx.get::<Debits>(&dk).expect("get") else {
        return match tx.get::<purse_store::DebitsInvalidated>(&dk).expect("get") {
            None => true,
            Some(d) => other_height(&d),
        };
    };
    if other_height(&d) {
        return true;
    }
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
    /// The steps left of a planned revalidation, last first: while any is
    /// left, a step of the random sequences takes the next one instead of
    /// drawing an operation.
    plan: Vec<Planned>,
    /// The vote bits of the next header, when a plan sets them.
    vote: Option<u16>,
}

/// A step of a planned revalidation (QK-0734's sequence, whose fix,
/// `merge_unmined_copy`, no generated script reaches): the tip, holding a
/// regular record, is disapproved by the next block; some previous outputs
/// of the record's inputs are credited; the record is mined again in the
/// block after, with credits for some of its outputs; both blocks are
/// rolled back, and another block approves the first, which merges the
/// record's unmined copy.
#[derive(Clone)]
enum Planned {
    /// A block extending the tip with these vote bits; with `Some((rec,
    /// h))` it approves block `h` again, after which `rec`, mined there and
    /// unmined before, must be mined only.
    Extend(u16, Option<(TxRecord, u32)>),
    /// `AddCredit` of output `index` of the record under the block of its
    /// latest mined record (unmined when it has none), for a drawn account.
    Credit(TxRecord, u32),
    /// `InsertMinedTx` of the record into the tip.
    Remine(TxRecord),
    /// `Rollback` from this height.
    Rollback(i32),
}

/// The choice standing for a planned step in the random sequences.
const PLANNED: u64 = u64::MAX;

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
        // Half the outputs (by the drawn byte, so the draws are as they
        // were) pay a P2PKH script, which input selection takes.
        let tx_out = (0..n_out)
            .map(|i| {
                let b = self.rng.below(256);
                TxOut {
                    value: i64::try_from(100 + i).expect("small"),
                    version: 0,
                    pk_script: if b.is_multiple_of(2) {
                        p2pkh(None, b)
                    } else {
                        vec![0x6a, 0x01, u8::try_from(b).expect("byte")]
                    },
                }
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

    /// A regular transaction paying P2SH scripts: the multisig scripts of
    /// [`ms_scripts`] (one under a stake tag, so in the stake tree), the
    /// script that is no multisig script, a script never imported, or an
    /// `OP_RETURN`.
    fn build_p2sh(&mut self) -> TxRecord {
        let scripts = ms_scripts();
        let prev = self.prev();
        let n_out = 1 + self.rng.below(4);
        let tx_out = (0..n_out)
            .map(|i| TxOut {
                value: i64::try_from(500 + i).expect("small"),
                version: 0,
                pk_script: match self.rng.below(7) {
                    0 | 1 => p2sh(None, &scripts[0]),
                    2 => p2sh(None, &scripts[1]),
                    3 => p2sh(Some(0xbd), &scripts[0]),
                    4 => p2sh(None, &scripts[2]),
                    5 => p2sh(None, &[0x52]),
                    _ => op_return(&[0x01]),
                },
            })
            .collect();
        let m = MsgTx {
            tx_in: vec![TxIn {
                previous_out_point: prev,
                sequence: u32::MAX,
                value_in: 3000,
                ..TxIn::default()
            }],
            tx_out,
            ..MsgTx::default()
        };
        self.push(m, TxType::Regular)
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

    /// A transaction the simulation built (at least one has been).
    fn pick(&mut self) -> TxRecord {
        let i = usize::try_from(self.rng.below(self.recs.len() as u64)).expect("small");
        self.recs[i].clone()
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

    /// Starts a planned revalidation of a regular record of the tip, if
    /// the tip holds one: the next header disapproves the tip, and the rest
    /// of the sequence ([`Planned`]) follows.
    fn start_plan(&mut self, ts: &TxStore, tx: &mut WriteTx<'_>) {
        let (_, tip) = ts.main_chain_tip(tx).expect("tip");
        let tip = u32::try_from(tip).expect("height");
        let regular: Vec<TxRecord> = self
            .recs
            .iter()
            .filter(|r| {
                r.tx_type() == TxType::Regular
                    && tx
                        .contains::<TxMined>(&purse_store::TxRecKey {
                            hash: *r.hash(),
                            height: tip,
                        })
                        .expect("contains")
            })
            .cloned()
            .collect();
        if regular.is_empty() {
            return;
        }
        let rec =
            regular[usize::try_from(self.rng.below(regular.len() as u64)).expect("small")].clone();
        self.vote = Some(0);
        let mut plan = Vec::new();
        for input in &rec.msg_tx().tx_in {
            let prev = &input.previous_out_point;
            if self.rng.below(2) == 0
                && let Some(p) = self.recs.iter().find(|r| r.hash().0 == prev.hash.0)
            {
                plan.push(Planned::Credit(p.clone(), prev.index));
            }
        }
        plan.push(Planned::Extend(1, None));
        plan.push(Planned::Remine(rec.clone()));
        for index in 0..rec.msg_tx().tx_out.len() {
            if self.rng.below(2) == 0 {
                plan.push(Planned::Credit(
                    rec.clone(),
                    u32::try_from(index).expect("small"),
                ));
            }
        }
        plan.push(Planned::Rollback(i32::try_from(tip + 1).expect("small")));
        plan.push(Planned::Extend(1, Some((rec, tip))));
        plan.reverse();
        self.plan = plan;
    }

    fn header(&mut self, ts: &TxStore, tx: &mut WriteTx<'_>) -> (HeaderBytes, Hash32) {
        let (tip, height) = ts.main_chain_tip(tx).expect("tip");
        self.nonce += 1;
        let h = BlockHeader {
            version: 7,
            prev_block: Hash(tip.0),
            merkle_root: Hash([u8::try_from(self.nonce % 256).expect("byte"); 32]),
            stake_root: Hash::ZERO,
            vote_bits: match self.vote.take() {
                Some(v) => v,
                None if self.rng.below(4) == 0 => 0,
                None => 1,
            },
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
    for rec in &sim.recs {
        for index in 0..rec.msg_tx().tx_out.len() {
            let op = OutPoint {
                hash: Hash(rec.hash().0),
                index: u32::try_from(index).expect("small"),
                tree: 0,
            };
            out += &format!(
                "{:?}\n",
                ts.get_multisig_output(r, &op).map_err(|e| e.as_kind())
            );
        }
    }
    for script in ms_scripts() {
        let addr = dcroxide_txscript::stdaddr::new_address_script_hash_v0(
            &script,
            &ChainParams::by_name("simnet").expect("simnet"),
        )
        .expect("address");
        out += &format!(
            "{:?}\n",
            ts.unspent_multisig_credits_for_address(r, &addr)
                .map_err(|e| e.as_kind())
        );
    }
    let (_, tip) = ts.main_chain_tip(r).expect("tip");
    for (min_conf, target) in [(0, 0), (1, 4000), (1, 0)] {
        let mut rng = stream(7);
        let mut src = ts
            .make_input_source(r, &mut rng, 0, min_conf, tip, None)
            .expect("source");
        out += &format!(
            "{:?}\n",
            src.select_inputs(purse_dcrutil::Amount(target))
                .map_err(|e| e.as_kind())
        );
    }
    out += &format!(
        "{:?}\n",
        ts.random_utxo(r, &mut stream(9), 1, tip)
            .map_err(|e| e.as_kind())
    );
    for rec in &sim.recs {
        for index in 0..rec.msg_tx().tx_out.len() {
            let op = OutPoint {
                hash: Hash(rec.hash().0),
                index: u32::try_from(index).expect("small"),
                tree: 0,
            };
            out += &format!(
                "{:?}\n",
                ts.spender(r, &op)
                    .map(|(m, i)| (m.tx_hash(), i))
                    .map_err(|e| e.as_kind())
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

/// The redeem scripts the random sequences pay to P2SH: a 1-of-2 and a
/// 2-of-3 multisig script and `OP_TRUE`, which is no multisig script.
fn ms_scripts() -> [Vec<u8>; 3] {
    let (k1, k2, k3) = (
        format!("02{}", "11".repeat(32)),
        format!("03{}", "22".repeat(32)),
        format!("02{}", "33".repeat(32)),
    );
    [
        purse_testutil::unhex(&format!("5121{k1}21{k2}52ae")),
        purse_testutil::unhex(&format!("5221{k1}21{k2}21{k3}53ae")),
        vec![0x51],
    ]
}

/// Records the redeem scripts of [`ms_scripts`] as imported scripts, the
/// rows the address manager's `ImportScript` writes (its sealed hash left
/// empty: the transaction store reads only the script).
fn import_ms_scripts(tx: &mut WriteTx<'_>) {
    for script in ms_scripts() {
        let h = dcroxide_txscript::stdaddr::hash160(&script);
        tx.put::<purse_store::AddrByHash>(
            &purse_store::Hash160(h),
            &purse_store::AddrRow::Imported {
                account: 0x7fff_ffff,
                data: purse_udb::addrmgr::ImportedData::Script {
                    enc_hash: Vec::new(),
                    script,
                }
                .encode(),
            },
        )
        .expect("imported script");
    }
}

/// A P2SH script paying `script`, tagged with the stake opcode `tag`.
fn p2sh(tag: Option<u8>, script: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = tag.into_iter().collect();
    s.extend_from_slice(&[0xa9, 0x14]);
    s.extend_from_slice(&dcroxide_txscript::stdaddr::hash160(script));
    s.push(0x87);
    s
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
    let (failures, counts) = sequences("purse-udb-txstore-sequences", 18, 12);
    eprintln!("failed operations checked: {failures:?}; successes: {counts:?}");
    // The guards follow `PURSE_TEST_SEED`: each is at or below the lowest
    // count over seeds 0 to 399 (selectable outputs 116, random_utxo found
    // 379, merged revalidations 1, credits added 75, kinds of failure 13).
    // Seeds 76, 78 and 257 of those reach QK-0781's doubled output.
    for (what, least) in [
        ("selectable outputs", 60),
        ("random_utxo found", 200),
        ("merged revalidations", 1),
    ] {
        let n = counts.get(what).copied().unwrap_or(0);
        assert!(n >= least, "too few {what}: {n}");
    }
    let credits_added = counts.get("add_credit").copied().unwrap_or(0);
    assert!(
        credits_added >= 60,
        "too few credits added: {credits_added}"
    );
    assert!(
        failures.len() >= 4,
        "too few kinds of failure exercised: {failures:?}"
    );
}

/// The random sequences with the commitment and multisig operations drawn
/// too (`AddTicketCommitment`, `RedeemTicketCommitments`, `AddMultisigOut`,
/// `SpendMultisigOut`, and transactions paying P2SH scripts the store's
/// address rows hold), on a store holding those scripts as imported
/// scripts: the same properties, the `Locked` sums and the multisig index
/// checked against the rows (`check_rows`), and the multisig queries among
/// the answers compared after a reopen.
#[test]
fn random_sequences_with_commitments_and_multisig_keep_the_row_invariants() {
    let (failures, counts) = sequences("purse-udb-txstore-ms-sequences", 26, 10);
    eprintln!("failed operations checked: {failures:?}; successes: {counts:?}");
    // At or below the lowest count over seeds 0 to 399 (add_ticket_commitment
    // 20, redeem_ticket_commitments 13, add_multisig_out 8,
    // spend_multisig_out 3, random_utxo found 266, merged revalidations 4,
    // kinds of failure 23), so that no supported seed fails a guard.
    for (op, least) in [
        ("add_ticket_commitment", 12),
        ("redeem_ticket_commitments", 8),
        ("add_multisig_out", 6),
        ("spend_multisig_out", 2),
        ("random_utxo found", 100),
        ("merged revalidations", 2),
    ] {
        let n = counts.get(op).copied().unwrap_or(0);
        assert!(n >= least, "too few successful {op}: {n}");
    }
    assert!(
        failures.len() >= 6,
        "too few kinds of failure exercised: {failures:?}"
    );
}

/// `rounds` random sequences of 120 steps from the seed named `label`,
/// drawing among the first `choices` operations (18 without the commitment
/// and multisig ones, 26 with them, which also records the multisig
/// scripts in the store); returns the failures checked, by operation and
/// kind, and the successes of the store operations that count, by name.
fn sequences(
    label: &str,
    choices: u64,
    rounds: usize,
) -> (BTreeMap<String, usize>, BTreeMap<&'static str, usize>) {
    let mut seeds = SplitMix64::from_env(label);
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for _ in 0..rounds {
        let (mut store, ts, path) = tx_store("simnet");
        if choices > 18 {
            store
                .update(|tx| {
                    import_ms_scripts(tx);
                    Ok::<_, purse_store::Error>(())
                })
                .expect("update");
        }
        let mut sim = Sim {
            rng: SplitMix64(seeds.next_u64()),
            recs: Vec::new(),
            tickets: Vec::new(),
            blocks: Vec::new(),
            nonce: 0,
            plan: Vec::new(),
            vote: None,
        };
        for step in 0..120 {
            let planned = sim.plan.pop();
            let choice = if planned.is_some() {
                PLANNED
            } else {
                sim.rng.below(choices)
            };
            let pre = dump_read(&store.read().expect("read"));
            let mut failed = None;
            let r: Result<(), TErr> = store.update(|tx| {
                let before = dump(tx);
                let mut stake_insert = false;
                let res = match choice {
                    PLANNED => match planned.clone().expect("a planned step") {
                        Planned::Extend(vote, approves) => {
                            sim.vote = Some(vote);
                            let pre = approves.as_ref().is_some_and(|(r, h)| {
                                tx.contains::<TxUnmined>(r.hash()).expect("contains")
                                    && tx
                                        .contains::<TxMined>(&purse_store::TxRecKey {
                                            hash: *r.hash(),
                                            height: *h,
                                        })
                                        .expect("contains")
                            });
                            let (h, hash) = sim.header(&ts, tx);
                            let res = ts.extend_main_chain(tx, &h, &hash, Some(&[]));
                            if res.is_ok() {
                                sim.blocks.push(hash);
                                if let (true, Some((r, _))) = (pre, &approves) {
                                    assert!(
                                        !tx.contains::<TxUnmined>(r.hash()).expect("contains"),
                                        "step {step}: a revalidated record keeps its unmined copy"
                                    );
                                    *counts.entry("merged revalidations").or_default() += 1;
                                }
                            }
                            res
                        }
                        Planned::Credit(r, index) => {
                            let block = match ts.tx_block_height(tx, r.hash()).unwrap_or(-1) {
                                h if h >= 0 => ts
                                    .get_main_chain_block_hash_for_height(tx, h)
                                    .ok()
                                    .and_then(|b| ts.get_block_meta_for_hash(tx, &b).ok()),
                                _ => None,
                            };
                            let account = u32::try_from(sim.rng.below(3)).expect("small");
                            ts.add_credit(tx, &r, block.as_ref(), index, false, account)
                        }
                        Planned::Remine(r) => {
                            let tip = ts.main_chain_tip(tx).expect("tip").0;
                            ts.insert_mined_tx(tx, &r, &tip)
                        }
                        Planned::Rollback(h) => ts.rollback(tx, h).map(drop),
                    },
                    0 | 1 => {
                        if sim.plan.is_empty() && sim.rng.below(3) == 0 {
                            sim.start_plan(&ts, tx);
                        }
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
                            *counts.entry("add_credit").or_default() += 1;
                        }
                        r
                    }
                    18 | 19 if !sim.recs.is_empty() => {
                        // A commitment: mostly of a ticket, at any index
                        // (odd ones, even ones and one past the end).
                        let (rec, ticket) = match sim.ticket() {
                            Some(t) if sim.rng.below(3) != 0 => (
                                sim.recs
                                    .iter()
                                    .find(|r| *r.hash() == t)
                                    .cloned()
                                    .expect("built"),
                                true,
                            ),
                            _ => (sim.pick(), false),
                        };
                        let n = rec.msg_tx().tx_out.len() as u64;
                        let index = if ticket && sim.rng.below(2) == 0 {
                            1
                        } else {
                            u32::try_from(sim.rng.below(n + 1)).expect("small")
                        };
                        let account = u32::try_from(sim.rng.below(3)).expect("small");
                        let r = ts.add_ticket_commitment(tx, &rec, index, account);
                        if r.is_ok() {
                            *counts.entry("add_ticket_commitment").or_default() += 1;
                        }
                        r
                    }
                    20 if !sim.recs.is_empty() => {
                        // A redemption: mostly of a vote or revocation,
                        // unmined or mined (the tip's meta: only whether a
                        // block is given matters).
                        let stake: Vec<TxRecord> = sim
                            .recs
                            .iter()
                            .filter(|r| matches!(r.tx_type(), TxType::SSGen | TxType::SSRtx))
                            .cloned()
                            .collect();
                        let rec = if !stake.is_empty() && sim.rng.below(4) != 0 {
                            stake
                                [usize::try_from(sim.rng.below(stake.len() as u64)).expect("small")]
                            .clone()
                        } else {
                            sim.pick()
                        };
                        let block = if sim.rng.below(2) == 0 {
                            None
                        } else {
                            let tip = ts.main_chain_tip(tx).expect("tip").0;
                            ts.get_block_meta_for_hash(tx, &tip).ok()
                        };
                        let r = ts.redeem_ticket_commitments(tx, &rec, block.as_ref());
                        if r.is_ok() {
                            *counts.entry("redeem_ticket_commitments").or_default() += 1;
                        }
                        r
                    }
                    21 if !sim.recs.is_empty() => {
                        // A multisig output: unmined, in a main-chain block
                        // (the tip or a random one; the store does not
                        // check the record is mined there, as Go does not),
                        // at any index.
                        let p2sh_outs: Vec<(TxRecord, u32)> = sim
                            .recs
                            .iter()
                            .flat_map(|r| {
                                r.msg_tx()
                                    .tx_out
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, o)| o.pk_script.last() == Some(&0x87))
                                    .map(move |(i, _)| {
                                        (r.clone(), u32::try_from(i).expect("small"))
                                    })
                            })
                            .collect();
                        let (rec, index) = if !p2sh_outs.is_empty() && sim.rng.below(4) != 0 {
                            p2sh_outs[usize::try_from(sim.rng.below(p2sh_outs.len() as u64))
                                .expect("small")]
                            .clone()
                        } else {
                            let rec = sim.pick();
                            let n = rec.msg_tx().tx_out.len() as u64;
                            let index = u32::try_from(sim.rng.below(n + 1)).expect("small");
                            (rec, index)
                        };
                        let block = match sim.rng.below(3) {
                            0 => None,
                            1 => {
                                let tip = ts.main_chain_tip(tx).expect("tip").0;
                                ts.get_block_meta_for_hash(tx, &tip).ok()
                            }
                            _ if !sim.blocks.is_empty() => {
                                let j = usize::try_from(sim.rng.below(sim.blocks.len() as u64))
                                    .expect("small");
                                ts.get_block_meta_for_hash(tx, &sim.blocks[j]).ok()
                            }
                            _ => None,
                        };
                        let r = ts.add_multisig_out(tx, &rec, block.as_ref(), index);
                        if r.is_ok() {
                            *counts.entry("add_multisig_out").or_default() += 1;
                        }
                        r
                    }
                    22 if !sim.recs.is_empty() => {
                        // A multisig spend of any built output by an input
                        // of any built transaction.
                        let recorded: Vec<purse_store::Outpoint> = all::<MultisigOuts>(tx)
                            .into_iter()
                            .map(|(k, _)| k)
                            .collect();
                        let op = if !recorded.is_empty() && sim.rng.below(4) != 0 {
                            let k = recorded[usize::try_from(sim.rng.below(recorded.len() as u64))
                                .expect("small")];
                            OutPoint {
                                hash: Hash(k.hash.0),
                                index: k.index,
                                tree: 0,
                            }
                        } else {
                            let rec = sim.pick();
                            let n = rec.msg_tx().tx_out.len().max(1) as u64;
                            OutPoint {
                                hash: Hash(rec.hash().0),
                                index: u32::try_from(sim.rng.below(n)).expect("small"),
                                tree: 0,
                            }
                        };
                        let spender = sim.pick();
                        let input = u32::try_from(sim.rng.below(2)).expect("small");
                        let r = ts.spend_multisig_out(tx, &op, spender.hash(), input);
                        if r.is_ok() {
                            *counts.entry("spend_multisig_out").or_default() += 1;
                        }
                        r
                    }
                    23..=25 => {
                        let rec = sim.build_p2sh();
                        if sim.rng.below(3) == 0 {
                            let tip = ts.main_chain_tip(tx).expect("tip").0;
                            ts.insert_mined_tx(tx, &rec, &tip)
                        } else {
                            ts.insert_mem_pool_tx(tx, &rec)
                        }
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
                        let (n, f, d) =
                            check_selection(&ts, tx, u8::try_from(step % 251).expect("small"));
                        *counts.entry("selectable outputs").or_default() += n;
                        *counts.entry("random_utxo found").or_default() += f;
                        *counts.entry("selectable twice (QK-0781)").or_default() += d;
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
                            let what = if choice == PLANNED {
                                "planned".to_owned()
                            } else {
                                format!("choice {choice}")
                            };
                            failed = Some(format!("{what} {:?}", e.as_kind()));
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
                check_library_read(&r);
            }
        }
    }
    (failures, counts)
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
        plan: Vec::new(),
        vote: None,
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
            plan: Vec::new(),
            vote: None,
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

/// `RangeTransactions` marks a credit spent when an unmined transaction
/// spends it (Go's `existsRawUnminedInput`, a lookup of the encoded
/// outpoint per credit), with none, some or all of a record's credits
/// spent that way, and its answer equals `TxDetails`'. A key of another
/// length in the unmined-inputs index matches no credit and fails no
/// query, as Go's bucket `Get` neither matches nor decodes it.
#[test]
fn range_marks_credits_spent_by_unmined_as_tx_details_does() {
    const OUTS: u32 = 8;
    for (spent, foreign_key) in [(0, false), (3, false), (OUTS, false), (3, true)] {
        let (store, ts, _) = tx_store("testnet3");
        let mut g = BlockGenerator::new();
        let b1 = g.generate(1);
        let values: Vec<i64> = (1..=i64::from(OUTS)).collect();
        let mined = outputs_tx(&values);
        let mined_hash = mined.tx_hash();
        let m = TxRecord::from_msg_tx(mined, 0).expect("record");
        store
            .update(|tx| {
                insert_main_chain_headers(&ts, tx, &[&b1]);
                let meta = make_block_meta(&b1);
                ts.insert_mined_tx(tx, &m, &meta.block.hash)
                    .expect("InsertMinedTx");
                for i in 0..OUTS {
                    ts.add_credit(tx, &m, Some(&meta), i, i % 2 == 1, 0)
                        .expect("AddCredit");
                }
                for i in 0..spent {
                    let u = TxRecord::from_msg_tx(spend_output(mined_hash, i, 0, &[1]), 0)
                        .expect("record");
                    ts.insert_mem_pool_tx(tx, &u).expect("InsertMemPoolTx");
                }
                if foreign_key {
                    tx.put_raw(TableId::UnminedInputs, &[7u8; 35], &[0u8; 32])?;
                }
                Ok::<_, purse_store::Error>(())
            })
            .expect("update");
        let mut r = store.read().expect("read");
        assert_eq!(
            r.table_len(TableId::UnminedInputs).expect("len"),
            u64::from(spent) + u64::from(foreign_key)
        );
        let (mut got, mut unmined) = (Vec::new(), 0u32);
        ts.range_transactions(&mut r, 0, -1, |d| {
            for x in d {
                if x.height() < 0 {
                    unmined += 1;
                } else {
                    got.push(x.credits.clone());
                }
            }
            Ok(false)
        })
        .expect("RangeTransactions");
        assert_eq!(unmined, spent);
        assert_eq!(got.len(), 1);
        let credits = &got[0];
        assert_eq!(credits.len(), OUTS as usize);
        for c in credits {
            assert_eq!(
                c.spent,
                c.index < spent,
                "credit {} ({spent} spent, foreign key {foreign_key})",
                c.index
            );
        }
        let details = ts.tx_details(&mut r, m.hash()).expect("TxDetails");
        assert_eq!(
            &details.credits, credits,
            "{spent} spent, foreign key {foreign_key}"
        );
    }
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
        plan: Vec::new(),
        vote: None,
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
        plan: Vec::new(),
        vote: None,
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

/// QK-0785 (reproduced, decided by the project owner on 2026-11-24): a
/// ticket mined in block 1 spends a
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

/// A stream that hands out fixed bytes, XORed into the caller's buffer as
/// dcrd's generator does.
struct Scripted {
    bytes: Vec<u8>,
    at: usize,
}

impl purse_snacl::RandomSource for Scripted {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), purse_snacl::RandomSourceError> {
        let src = self
            .bytes
            .get(self.at..self.at + buf.len())
            .ok_or_else(|| purse_snacl::RandomSourceError::new("exhausted"))?;
        for (b, s) in buf.iter_mut().zip(src) {
            *b ^= s;
        }
        self.at += buf.len();
        Ok(())
    }
}

/// Go's `randomUTXO` (`txmined.go:2358-2422`) written out over
/// `purse-store`'s `ReadCursor`, whose `Seek`, `Next` and `Prev` are
/// bbolt's at the bucket ends (held to bbolt by `purse-store`'s
/// conformance tests against the `boltcursor` oracle): the key it returns
/// for the seek key `key`, the direction bit and the `IntN` draw `x`.
fn go_random_utxo(
    r: &ReadTx,
    key: &[u8; 32],
    prev_first: bool,
    x: u64,
    skip: &dyn Fn(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    let mut c = r.cursor(TableId::Unspent).expect("cursor");
    let collect = |c: &mut purse_store::ReadCursor<'_>,
                   mut e: Option<(Vec<u8>, Vec<u8>)>,
                   back: bool|
     -> Vec<Vec<u8>> {
        let mut keys: Vec<Vec<u8>> = Vec::new();
        while let Some((k, _)) = e {
            if keys.first().is_some_and(|f| f[..32] != k[..32]) {
                break;
            }
            if !skip(&k) {
                keys.push(k);
            }
            e = if back { c.prev() } else { c.next() }.expect("step");
        }
        keys
    };
    let mut e = c.seek(key).expect("seek");
    if prev_first {
        e = c.prev().expect("prev");
    }
    let mut keys = collect(&mut c, e, prev_first);
    if keys.is_empty() {
        let e = if prev_first {
            c.seek(key).expect("seek")
        } else {
            c.seek(key).expect("seek");
            c.prev().expect("prev")
        };
        keys = collect(&mut c, e, !prev_first);
    }
    if keys.is_empty() {
        return None;
    }
    let n = keys.len() as u128;
    let wide = u128::from(x) * n;
    assert!((wide as u64) >= n as u64, "a draw IntN would reject");
    Some(keys[(wide >> 64) as usize].clone())
}

/// `RandomUTXO`'s walk of `unspent` (`store::walk` over typed ranges)
/// against [`go_random_utxo`] at the cursor's edges (spec 04 Q-31): an
/// empty table, then six transactions of one to three outputs in four
/// blocks, one output spent by an unmined transaction; seek keys of all
/// zero bits and all one bits (before the first key and past the last),
/// each transaction's hash exactly (a 32-byte key sorting before its
/// 36-byte outpoints) and the values just below and above it, both
/// directions, at confirmation depths that skip none, some or every
/// output. Every answer is the cursor's.
#[test]
fn random_utxo_walks_as_bbolt_s_cursor_does() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let blocks: Vec<BlockHeader> = (0..4).map(|_| g.generate(1)).collect();
    let script = p2pkh(None, 7);
    let mut txs = Vec::new();
    for (i, n) in [1usize, 3, 2, 1, 3, 2].into_iter().enumerate() {
        let mut m = outputs_tx(&vec![1000 + i64::try_from(i).expect("small"); n]);
        for o in &mut m.tx_out {
            o.pk_script = script.clone();
        }
        m.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: Hash([u8::try_from(i).expect("small"); 32]),
                index: 0,
                tree: 0,
            },
            ..TxIn::default()
        });
        txs.push((
            TxRecord::from_msg_tx(m, 1_795_400_000).expect("record"),
            i % 4,
            n,
        ));
    }
    let x = 0x9e37_79b9_7f4a_7c15u64;
    let check = |store: &Store, cases: &mut usize, nones: &mut usize| {
        let r = store.read().expect("read");
        let mut wr = store.read().expect("read");
        let uinputs: BTreeSet<Vec<u8>> = TxRead::keys::<UnminedInputs>(
            &mut wr,
            Bound::Unbounded,
            Bound::Unbounded,
            false,
            usize::MAX,
        )
        .expect("keys")
        .into_iter()
        .map(|k| [k.hash.0.as_slice(), &k.index.to_be_bytes()].concat())
        .collect();
        let rows: BTreeMap<Vec<u8>, u32> = TxRead::rows::<Unspent>(
            &mut wr,
            Bound::Unbounded,
            Bound::Unbounded,
            false,
            usize::MAX,
        )
        .expect("rows")
        .into_iter()
        .map(|(k, u)| {
            (
                [k.hash.0.as_slice(), &k.index.to_be_bytes()].concat(),
                u.height,
            )
        })
        .collect();
        let mut keys: Vec<[u8; 32]> = vec![[0; 32], [0xff; 32]];
        for k in rows.keys() {
            let mut h = [0u8; 32];
            h.copy_from_slice(&k[..32]);
            let v = u128::from_be_bytes(h[16..].try_into().expect("16"));
            for d in [v.wrapping_sub(1), v, v.wrapping_add(1)] {
                let mut k2 = h;
                k2[16..].copy_from_slice(&d.to_be_bytes());
                keys.push(k2);
            }
        }
        for (min_conf, sync) in [(1, 4), (2, 4), (4, 4), (5, 4)] {
            for key in &keys {
                for dir in [0u8, 1] {
                    let skip = |k: &[u8]| {
                        let h = i32::try_from(rows[k]).expect("height");
                        uinputs.contains(k) || !(sync - h + 1 >= min_conf && h <= sync)
                    };
                    let want = go_random_utxo(&r, key, dir == 1, x, &skip);
                    let mut bytes = key.to_vec();
                    bytes.push(dir);
                    bytes.extend_from_slice(&x.to_le_bytes());
                    let mut s = Scripted { bytes, at: 0 };
                    let got = ts
                        .random_utxo(&mut wr, &mut s, min_conf, sync)
                        .expect("RandomUTXO")
                        .map(|c| {
                            [
                                c.outpoint.hash.0.as_slice(),
                                &c.outpoint.index.to_be_bytes(),
                            ]
                            .concat()
                        });
                    assert_eq!(got, want, "key {key:02x?} dir {dir} minconf {min_conf}");
                    assert_eq!(s.at, if want.is_some() { 41 } else { 33 }, "bytes drawn");
                    *cases += 1;
                    *nones += usize::from(want.is_none());
                }
            }
        }
    };
    let (mut cases, mut nones) = (0, 0);
    check(&store, &mut cases, &mut nones);
    assert_eq!(cases, nones, "an empty table gives nothing");
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &blocks.iter().collect::<Vec<_>>());
            for (rec, b, n) in &txs {
                ts.insert_mined_tx(tx, rec, &Hash32(*blocks[*b].block_hash().as_bytes()))
                    .expect("InsertMinedTx");
                for i in 0..*n {
                    let i = u32::try_from(i).expect("small");
                    ts.add_credit(tx, rec, Some(&make_block_meta(&blocks[*b])), i, false, 0)
                        .expect("AddCredit");
                }
            }
            let spend = spend_output(Hash(txs[1].0.hash().0), 1, 0, &[10]);
            let spend = TxRecord::from_msg_tx(spend, 1_795_400_000).expect("record");
            ts.insert_mem_pool_tx(tx, &spend).expect("InsertMemPoolTx");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    let (mut cases, mut nones) = (0, 0);
    check(&store, &mut cases, &mut nones);
    assert!(
        nones > 0 && nones < cases,
        "{nones} of {cases} found nothing"
    );
}

/// `Spender`'s refusals that no operation of either side reaches, with
/// the unmined-inputs entries written directly (Go's texts and kinds,
/// `txquery.go:640-725`): an entry naming a transaction with no unmined
/// record (`NotExist` "missing unmined spending tx", for a mined and for
/// an unmined credit), an entry naming an unmined transaction that does
/// not spend the output (`NotExist` "recorded spending tx does not spend
/// credit"), and an entry naming the zero hash, which Go's test of the
/// spender hash for zero sends to the unmined credit's lookup (`Invalid`
/// "output is not a credit" for a mined credit). Every other branch is
/// compared with Go by `37-txstore-selection`.
#[test]
fn spender_refuses_a_dangling_or_wrong_unmined_spender() {
    let (store, ts, _) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let block1 = g.generate(1);
    let (a, _) = funded_ticket();
    let h1 = Hash32(*block1.block_hash().as_bytes());
    let b = TxRecord::from_msg_tx(spend_output(Hash([5; 32]), 0, 0, &[300]), 1_795_400_000)
        .expect("record");
    let u = TxRecord::from_msg_tx(spend_output(Hash([6; 32]), 0, 0, &[100]), 1_795_400_000)
        .expect("record");
    let op = |rec: &TxRecord| purse_store::Outpoint {
        hash: *rec.hash(),
        index: 0,
    };
    let wire_op = |rec: &TxRecord| OutPoint {
        hash: Hash(rec.hash().0),
        index: 0,
        tree: 0,
    };
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&block1]);
            ts.insert_mined_tx(tx, &a, &h1).expect("InsertMinedTx a");
            ts.add_credit(tx, &a, Some(&make_block_meta(&block1)), 0, false, 0)
                .expect("AddCredit a");
            ts.insert_mem_pool_tx(tx, &b).expect("InsertMemPoolTx b");
            ts.add_credit(tx, &b, None, 0, false, 0)
                .expect("AddCredit b");
            ts.insert_mem_pool_tx(tx, &u).expect("InsertMemPoolTx u");
            let refusal = |tx: &mut WriteTx<'_>, rec: &TxRecord| {
                let e = ts.spender(tx, &wire_op(rec)).expect_err("a refusal");
                (e.as_kind(), e.to_string())
            };
            for rec in [&a, &b] {
                let (kind, text) = refusal(tx, rec);
                assert_eq!(kind, Some(Kind::NotExist), "{text}");
                assert!(text.contains("credit is unspent"), "{text}");
                tx.put::<UnminedInputs>(&op(rec), &Hash32([0x42; 32]))
                    .expect("put");
                let (kind, text) = refusal(tx, rec);
                assert_eq!(kind, Some(Kind::NotExist), "{text}");
                assert!(text.contains("missing unmined spending tx"), "{text}");
                tx.put::<UnminedInputs>(&op(rec), u.hash()).expect("put");
                let (kind, text) = refusal(tx, rec);
                assert_eq!(kind, Some(Kind::NotExist), "{text}");
                assert!(
                    text.contains("recorded spending tx does not spend credit"),
                    "{text}"
                );
            }
            tx.put::<UnminedInputs>(&op(&a), &Hash32::default())
                .expect("put");
            let (kind, text) = refusal(tx, &a);
            assert_eq!(kind, Some(Kind::Invalid), "{text}");
            assert!(text.contains("output is not a credit"), "{text}");
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
}

/// One write transaction of `f`, after which the row invariants, the UTXO
/// queries and the selections are checked against the rows, and then a
/// reopen, after which the store must hold the same rows and give the same
/// answers.
fn checked_step(
    store: Store,
    path: &std::path::Path,
    ts: &TxStore,
    sim: &Sim,
    what: &str,
    f: impl FnOnce(&mut WriteTx<'_>),
) -> Store {
    store
        .update(|tx| {
            f(tx);
            check_rows(tx);
            check_utxo_queries(ts, tx);
            check_selection(ts, tx, 5);
            Ok::<_, purse_store::Error>(())
        })
        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let (live, rows) = {
        let mut r = store.read().expect("read");
        (answers(ts, &mut r, sim), dump_read(&r))
    };
    store.close().expect("close");
    let store = Store::open(path).expect("reopen");
    {
        let mut r = store.read().expect("read");
        assert_eq!(dump_read(&r), rows, "{what}: rows after a reopen");
        assert_eq!(
            answers(ts, &mut r, sim),
            live,
            "{what}: answers after a reopen"
        );
        check_library_read(&r);
    }
    store
}

/// QK-0734's fix (fixed under ADR-0012 Rule 7; `merge_unmined_copy` in
/// `mined.rs`) on its own path, which the random sequences reach only
/// through their planned revalidations: a transaction mined in block 3,
/// block 3 disapproved, the transaction mined again in block 5, blocks 4
/// and 5 rolled back and block 3 approved by another block 4, then the
/// copy's dependents removed and block 3 detached again. After every step
/// the row invariants (with the fix's own: no outpoint both an unspent
/// output and an unmined credit), the UTXO queries, the selections and a
/// reopen are checked, and after the approval the transaction is mined
/// only. Each variant adds complications, alone and all together: an
/// input whose output is credited only after the first mining (debited
/// at block 3 by the merge), an output credited only by the second mining
/// (moved into a mined credit), an unmined spender of the copy's output
/// (removed afterwards), the copy marked unpublished, an earlier
/// transaction of the same block re-mined with it whose unmined credit the
/// copy spends, and multisig records (an output of the transaction and an
/// output it spends; QK-0789). Go's answers for the same steps, where they
/// differ, are pinned by
/// `a_revalidated_transaction_merges_what_only_its_unmined_copy_holds` in
/// `tests/oracle.rs`.
#[test]
fn a_revalidated_transaction_merges_its_unmined_copy_keeping_the_row_invariants() {
    const LATE_INPUT: u8 = 1;
    const LATE_OUTPUT: u8 = 2;
    const SPENDER: u8 = 4;
    const UNPUBLISHED: u8 = 8;
    const SAME_BLOCK: u8 = 16;
    const MULTISIG: u8 = 32;
    for variant in [
        0,
        LATE_INPUT,
        LATE_OUTPUT,
        SPENDER,
        UNPUBLISHED,
        SAME_BLOCK,
        MULTISIG,
        63,
    ] {
        let on = |flag: u8| variant & flag != 0;
        let (store, ts, path) = tx_store("testnet3");
        let mut g = BlockGenerator::new();
        let (b1, b2, b3) = (g.generate(1), g.generate(1), g.generate(1));
        let mut g2 = BlockGenerator {
            last_hash: g.last_hash,
            last_height: g.last_height,
        };
        let (b4, b5, b4x) = (g.generate(0), g.generate(1), g2.generate(1));
        let hash_of = |b: &BlockHeader| Hash32(*b.block_hash().as_bytes());
        let ms = ms_scripts();
        let input = |hash: Hash, index: u32| TxIn {
            previous_out_point: OutPoint {
                hash,
                index,
                tree: 0,
            },
            sequence: u32::MAX,
            value_in: 1000,
            ..TxIn::default()
        };
        let out = |value: i64, pk_script: Vec<u8>| TxOut {
            value,
            version: 0,
            pk_script,
        };
        let rec = |m: MsgTx| TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
        let f = rec(MsgTx {
            tx_in: vec![input(Hash([0x0f; 32]), 0)],
            tx_out: vec![
                out(1000, p2pkh(None, 1)),
                out(2000, p2pkh(None, 2)),
                out(3000, p2sh(None, &ms[0])),
            ],
            ..MsgTx::default()
        });
        let c = rec(MsgTx {
            tx_in: vec![input(Hash([0x0c; 32]), 0)],
            tx_out: vec![out(700, p2pkh(None, 3))],
            ..MsgTx::default()
        });
        let fh = Hash(f.hash().0);
        let mut a_in = vec![
            input(fh, 0),
            input(fh, 1),
            input(fh, 2),
            input(Hash([0x0a; 32]), 0),
        ];
        if on(SAME_BLOCK) {
            a_in.push(input(Hash(c.hash().0), 0));
        }
        let a = rec(MsgTx {
            tx_in: a_in,
            tx_out: vec![
                out(900, p2pkh(None, 4)),
                out(500, p2pkh(None, 5)),
                out(400, p2sh(None, &ms[0])),
            ],
            ..MsgTx::default()
        });
        let u = rec(MsgTx {
            tx_in: vec![input(Hash(a.hash().0), 0)],
            tx_out: vec![out(800, p2pkh(None, 6))],
            ..MsgTx::default()
        });
        let sim = Sim {
            rng: SplitMix64(1),
            recs: vec![f.clone(), c.clone(), a.clone(), u.clone()],
            tickets: Vec::new(),
            blocks: Vec::new(),
            nonce: 0,
            plan: Vec::new(),
            vote: None,
        };
        let imported = 0x7fff_ffff;
        let credit = |tx: &mut WriteTx<'_>, r: &TxRecord, b: &BlockHeader, i: u32, acct: u32| {
            ts.add_credit(tx, r, Some(&make_block_meta(b)), i, false, acct)
                .unwrap_or_else(|e| panic!("variant {variant}: AddCredit {i}: {e:?}"));
        };
        let v = format!("variant {variant}");
        let mut store = checked_step(store, &path, &ts, &sim, &format!("{v}: setup"), |tx| {
            if on(MULTISIG) {
                import_ms_scripts(tx);
            }
            insert_main_chain_headers(&ts, tx, &[&b1, &b2, &b3]);
            ts.insert_mined_tx(tx, &f, &hash_of(&b1)).expect("f");
            credit(tx, &f, &b1, 0, 0);
            if !on(LATE_INPUT) {
                credit(tx, &f, &b1, 1, 1);
            }
            if on(MULTISIG) {
                credit(tx, &f, &b1, 2, imported);
                ts.add_multisig_out(tx, &f, Some(&make_block_meta(&b1)), 2)
                    .expect("AddMultisigOut f:2");
            }
        });
        let mine = |tx: &mut WriteTx<'_>, b: &BlockHeader, first: bool| {
            if on(SAME_BLOCK) {
                ts.insert_mined_tx(tx, &c, &hash_of(b)).expect("c");
                credit(tx, &c, b, 0, 0);
            }
            ts.insert_mined_tx(tx, &a, &hash_of(b)).expect("a");
            credit(tx, &a, b, 0, 0);
            if !first || !on(LATE_OUTPUT) {
                credit(tx, &a, b, 1, 2);
            }
            if on(MULTISIG) {
                credit(tx, &a, b, 2, imported);
                let r = ts.add_multisig_out(tx, &a, Some(&make_block_meta(b)), 2);
                // The second mining's is refused: the record is mined.
                assert_eq!(r.is_ok(), first, "{r:?}");
                let fop = OutPoint {
                    hash: fh,
                    index: 2,
                    tree: 0,
                };
                ts.spend_multisig_out(tx, &fop, a.hash(), 2)
                    .expect("SpendMultisigOut f:2");
            }
        };
        store = checked_step(store, &path, &ts, &sim, &format!("{v}: mined"), |tx| {
            mine(tx, &b3, true);
        });
        store = checked_step(
            store,
            &path,
            &ts,
            &sim,
            &format!("{v}: disapproved"),
            |tx| {
                insert_main_chain_headers(&ts, tx, &[&b4]);
                if on(LATE_INPUT) {
                    credit(tx, &f, &b1, 1, 1);
                }
            },
        );
        store = checked_step(
            store,
            &path,
            &ts,
            &sim,
            &format!("{v}: mined again"),
            |tx| {
                insert_main_chain_headers(&ts, tx, &[&b5]);
                mine(tx, &b5, false);
            },
        );
        store = checked_step(
            store,
            &path,
            &ts,
            &sim,
            &format!("{v}: rolled back"),
            |tx| {
                ts.rollback(tx, 4).expect("Rollback");
                assert_eq!(
                    ts.exists_tx_mined_or_unmined(tx, a.hash()).expect("exists"),
                    (false, true)
                );
                if on(SPENDER) {
                    ts.insert_mem_pool_tx(tx, &u).expect("u");
                }
                if on(UNPUBLISHED) {
                    ts.set_published(tx, a.hash(), false).expect("SetPublished");
                }
            },
        );
        store = checked_step(store, &path, &ts, &sim, &format!("{v}: approved"), |tx| {
            insert_main_chain_headers(&ts, tx, &[&b4x]);
            for r in [&a, &c] {
                if r.hash() == c.hash() && !on(SAME_BLOCK) {
                    continue;
                }
                assert_eq!(
                    ts.exists_tx_mined_or_unmined(tx, r.hash()).expect("exists"),
                    (true, false),
                    "{v}: mined only"
                );
            }
            assert!(!tx.contains::<Unpublished>(a.hash()).expect("contains"));
            for index in 0..2 {
                let k = CreditKey {
                    hash: *a.hash(),
                    height: 3,
                    index,
                };
                assert!(
                    tx.contains::<Credits>(&k).expect("contains"),
                    "{v}: a:{index}"
                );
            }
            let fop = |index: u32| OutPoint {
                hash: fh,
                index,
                tree: 0,
            };
            for index in [0, 1] {
                let (m, i) = ts.spender(tx, &fop(index)).expect("Spender");
                assert_eq!((m.tx_hash(), i), (Hash(a.hash().0), index), "{v}");
                let k = CreditKey {
                    hash: *a.hash(),
                    height: 3,
                    index,
                };
                assert!(
                    tx.contains::<Debits>(&k).expect("contains"),
                    "{v}: debit {index}"
                );
            }
            if on(MULTISIG) {
                let aop = OutPoint {
                    hash: Hash(a.hash().0),
                    index: 2,
                    tree: 0,
                };
                let m = ts.get_multisig_output(tx, &aop).expect("a:2");
                assert_eq!(m.block_height, 3, "{v}: a:2 marked mined again");
                let m = ts.get_multisig_output(tx, &fop(2)).expect("f:2");
                assert_eq!(
                    m.spender.map(|s| (s.hash, s.input)),
                    Some((*a.hash(), 2)),
                    "{v}: f:2 marked spent again"
                );
            }
        });
        if on(SPENDER) {
            store = checked_step(
                store,
                &path,
                &ts,
                &sim,
                &format!("{v}: spender removed"),
                |tx| {
                    ts.remove_unconfirmed(tx, u.msg_tx(), u.hash())
                        .expect("RemoveUnconfirmed");
                },
            );
        }
        store = checked_step(store, &path, &ts, &sim, &format!("{v}: detached"), |tx| {
            ts.rollback(tx, 3).expect("Rollback");
            assert_eq!(
                ts.exists_tx_mined_or_unmined(tx, a.hash()).expect("exists"),
                (false, true)
            );
        });
        drop(store);
    }
}

/// QK-0781's state (reproduced) under the property tests' checks, the
/// sequence `random_sequences_keep_the_row_invariants` reaches at other
/// seeds: a transaction recorded mined in blocks 1 and 2 with its credit
/// at block 1, block 2 rolled back, then the output credited without a
/// block. The outpoint is then an unspent output and an unmined credit at
/// once, which QK-0734's fix removes only on its own path:
/// `UnspentOutputs` lists it twice, `AccountBalances` counts it twice and
/// `SelectInputs` at depth 0 selects it twice, as Go does
/// (`a_transaction_mined_twice_and_rolled_back_is_listed_counted_and_selected_twice`
/// in `tests/oracle.rs`), and the selection checks take it twice at depth
/// 0 for its account only. The row invariants, the UTXO queries and a
/// reopen are checked after every step.
#[test]
fn a_transaction_mined_twice_and_rolled_back_is_selected_twice_as_go_does() {
    let (store, ts, path) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let (b1, b2) = (g.generate(1), g.generate(1));
    let hash_of = |b: &BlockHeader| Hash32(*b.block_hash().as_bytes());
    let t = TxRecord::from_msg_tx(
        MsgTx {
            tx_in: vec![TxIn {
                previous_out_point: OutPoint {
                    hash: Hash([0x11; 32]),
                    index: 0,
                    tree: 0,
                },
                sequence: u32::MAX,
                value_in: 6000,
                ..TxIn::default()
            }],
            tx_out: vec![TxOut {
                value: 5000,
                version: 0,
                pk_script: p2pkh(None, 1),
            }],
            ..MsgTx::default()
        },
        1_795_400_000,
    )
    .expect("record");
    let sim = Sim {
        rng: SplitMix64(1),
        recs: vec![t.clone()],
        tickets: Vec::new(),
        blocks: Vec::new(),
        nonce: 0,
        plan: Vec::new(),
        vote: None,
    };
    let store = checked_step(store, &path, &ts, &sim, "mined twice", |tx| {
        insert_main_chain_headers(&ts, tx, &[&b1, &b2]);
        ts.insert_mined_tx(tx, &t, &hash_of(&b1)).expect("b1");
        ts.add_credit(tx, &t, Some(&make_block_meta(&b1)), 0, false, 0)
            .expect("credit at b1");
        ts.insert_mined_tx(tx, &t, &hash_of(&b2)).expect("b2");
    });
    let store = checked_step(store, &path, &ts, &sim, "rolled back", |tx| {
        ts.rollback(tx, 2).expect("Rollback");
        assert_eq!(
            ts.exists_tx_mined_or_unmined(tx, t.hash()).expect("exists"),
            (false, true)
        );
        assert_eq!(ts.unspent_outputs(tx).expect("UnspentOutputs").len(), 1);
    });
    let store = checked_step(store, &path, &ts, &sim, "credited unmined", |tx| {
        ts.add_credit(tx, &t, None, 0, false, 0)
            .expect("unmined credit");
        let outs = ts.unspent_outputs(tx).expect("UnspentOutputs");
        assert_eq!(outs.len(), 2);
        assert_eq!(outs[0].outpoint, outs[1].outpoint);
        let b = ts.account_balances(tx, 0).expect("balances");
        assert_eq!((b[0].spendable, b[0].total), (10000, 10000));
        let (_, _, twice) = check_selection(&ts, tx, 5);
        assert_eq!(twice, 1, "taken twice at depth 0 for account 0 only");
    });
    drop(store);
}

// ------------------------------------------- the consistency check (BK4)

/// A transaction with inputs spending `inputs` and P2PKH outputs of
/// `values` (scripts to hashes filled from `fill`).
fn paying(inputs: &[(Hash, u32)], values: &[i64], fill: u64) -> MsgTx {
    MsgTx {
        tx_in: inputs
            .iter()
            .map(|(hash, index)| TxIn {
                previous_out_point: OutPoint {
                    hash: *hash,
                    index: *index,
                    tree: 0,
                },
                sequence: u32::MAX,
                ..TxIn::default()
            })
            .collect(),
        tx_out: values
            .iter()
            .zip(fill..)
            .map(|(v, f)| TxOut {
                value: *v,
                version: 0,
                pk_script: p2pkh(None, f),
            })
            .collect(),
        ..MsgTx::default()
    }
}

/// The hashes of the records of [`corruption_base`].
struct Base {
    rec1: Hash32,
    rec2: Hash32,
    rec3: Hash32,
    rec4: Hash32,
}

/// A store built by the operations that holds a row of every kind the
/// consistency check reads (testnet3): `a` mined in block 1 paying account
/// 0, the ticket `t` spending it mined in block 2 with its credit and a
/// tracked commitment; `rec1` mined in block 1 paying accounts 0 and 1;
/// `rec2` spending `rec1:0` in block 2; `rec3` spending `rec2:0` in block 3,
/// which block 4 disapproves (so `rec3`'s credit and debit are
/// invalidated and `rec2:0` is unspent again); the unpublished unmined
/// `rec4` spending `rec1:1` and paying account 1.
fn corruption_base() -> (Store, std::path::PathBuf, Base) {
    let (store, ts, path) = tx_store("testnet3");
    let mut g = BlockGenerator::new();
    let blocks = [g.generate(1), g.generate(1), g.generate(1), g.generate(0)];
    let hash = |b: &BlockHeader| Hash32(*b.block_hash().as_bytes());
    let (a, t) = funded_ticket();
    let rec = |m: MsgTx| TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
    let rec1 = rec(paying(&[(Hash([8; 32]), 0)], &[200_000_000, 50_000], 10));
    let rec2 = rec(paying(
        &[(Hash(rec1.hash().0), 0)],
        &[100_000_000, 7_000],
        20,
    ));
    let rec3 = rec(paying(&[(Hash(rec2.hash().0), 0)], &[90_000_000], 30));
    let rec4 = rec(paying(&[(Hash(rec1.hash().0), 1)], &[40_000], 40));
    store
        .update(|tx| {
            insert_main_chain_headers(&ts, tx, &[&blocks[0], &blocks[1], &blocks[2]]);
            let mined =
                |tx: &mut WriteTx<'_>, r: &TxRecord, b: &BlockHeader, outs: &[(u32, u32)]| {
                    ts.insert_mined_tx(tx, r, &hash(b)).expect("InsertMinedTx");
                    for &(index, account) in outs {
                        ts.add_credit(tx, r, Some(&make_block_meta(b)), index, false, account)
                            .expect("AddCredit");
                    }
                };
            mined(tx, &a, &blocks[0], &[(0, 0)]);
            mined(tx, &rec1, &blocks[0], &[(0, 0), (1, 1)]);
            mined(tx, &t, &blocks[1], &[(0, 0)]);
            ts.add_ticket_commitment(tx, &t, 1, 0)
                .expect("AddTicketCommitment");
            mined(tx, &rec2, &blocks[1], &[(0, 0), (1, 1)]);
            mined(tx, &rec3, &blocks[2], &[(0, 0)]);
            insert_main_chain_headers(&ts, tx, &[&blocks[3]]);
            ts.insert_mem_pool_tx(tx, &rec4).expect("InsertMemPoolTx");
            ts.add_credit(tx, &rec4, None, 0, false, 1)
                .expect("AddCredit");
            ts.set_published(tx, rec4.hash(), false)
                .expect("SetPublished");
            check_rows(tx);
            for t in [
                TableId::History,
                TableId::TxMined,
                TableId::TxUnmined,
                TableId::Unpublished,
                TableId::UnminedInputs,
                TableId::UnminedCredits,
                TableId::Credits,
                TableId::CreditsInvalidated,
                TableId::Debits,
                TableId::DebitsInvalidated,
                TableId::Unspent,
                TableId::Commitments,
                TableId::CommitmentsUnspent,
                TableId::Balances,
                TableId::BalanceHeights,
                TableId::AcctHistory,
            ] {
                assert!(!tx.is_empty(t).expect("is_empty"), "{} is empty", t.name());
            }
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    let base = Base {
        rec1: *rec1.hash(),
        rec2: *rec2.hash(),
        rec3: *rec3.hash(),
        rec4: *rec4.hash(),
    };
    (store, path, base)
}

/// Puts every table back as `rows` (a [`dump`]) holds it.
fn restore_rows(tx: &mut WriteTx<'_>, rows: &[(TableId, Vec<u8>, Vec<u8>)]) {
    for t in TableId::ALL {
        let keys: Vec<Vec<u8>> = tx
            .range_raw(*t, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
            .map(|r| r.expect("row").0)
            .collect();
        for k in keys {
            tx.delete_raw(*t, &k).expect("delete");
        }
    }
    for (t, k, v) in rows {
        tx.put_raw(*t, k, v).expect("put");
    }
}

/// Which check refuses a corruption: `purse-store`'s or `purse-udb`'s
/// decoded part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refuser {
    Store,
    Decoded,
}

/// A direct row edit breaking one invariant, the check that refuses it,
/// and the start of its text.
type Edit = Box<dyn Fn(&mut WriteTx<'_>, &Base)>;

fn corruptions() -> Vec<(&'static str, Refuser, &'static str, Edit)> {
    use purse_store::{
        BalanceKey, CommitmentUnspentRow, CreditRow, DebitRow, DebitsInvalidated, Height,
        HistoryKey, Outpoint, Spender, TxRecKey, UnminedCreditRow, UnspentRow,
    };
    let edit = |f: fn(&mut WriteTx<'_>, &Base)| -> Edit { Box::new(f) };
    vec![
        (
            "1: a block counting one more wallet transaction",
            Refuser::Store,
            "block 2 counts 3 wallet transactions and has 2 history rows",
            edit(|tx, _| {
                let mut b = tx.get::<Blocks>(&Height(2)).unwrap().unwrap();
                b.wallet_txs += 1;
                tx.put::<Blocks>(&Height(2), &b).unwrap();
            }),
        ),
        (
            "1: a history row above the main chain",
            Refuser::Store,
            "a history row at height 9 above the main chain",
            edit(|tx, b| {
                let k = purse_store::HistoryKey {
                    height: 9,
                    position: 0,
                };
                tx.put::<History>(&k, &b.rec1).unwrap();
            }),
        ),
        (
            "2: a record at another position than its history row",
            Refuser::Store,
            "record ",
            edit(|tx, b| {
                let k = TxRecKey {
                    hash: b.rec2,
                    height: 2,
                };
                let mut m = tx.get::<TxMined>(&k).unwrap().unwrap();
                m.position += 7;
                tx.put::<TxMined>(&k, &m).unwrap();
            }),
        ),
        (
            "3: an unmined input naming no unmined record",
            Refuser::Store,
            "unmined input ",
            edit(|tx, _| {
                let o = Outpoint {
                    hash: Hash32([0xdd; 32]),
                    index: 3,
                };
                tx.put::<UnminedInputs>(&o, &Hash32([0xee; 32])).unwrap();
            }),
        ),
        (
            "3 (decoded): an unmined input naming a transaction not spending it",
            Refuser::Decoded,
            "unmined input ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: Hash32([0xdd; 32]),
                    index: 3,
                };
                tx.put::<UnminedInputs>(&o, &b.rec4).unwrap();
            }),
        ),
        (
            "4: an unmined credit without its unmined record",
            Refuser::Store,
            "unmined credit ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec4,
                    index: 0,
                };
                let row: UnminedCreditRow = tx.get::<UnminedCredits>(&o).unwrap().unwrap();
                let o = Outpoint {
                    hash: Hash32([0xee; 32]),
                    index: 0,
                };
                tx.put::<UnminedCredits>(&o, &row).unwrap();
            }),
        ),
        (
            "4: an unpublished mark removed without reclassing its credits",
            Refuser::Store,
            "unmined credit ",
            edit(|tx, b| {
                tx.delete::<Unpublished>(&b.rec4).unwrap();
            }),
        ),
        (
            "5: an unspent output at a height without its credit",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 0,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.height = 1;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output whose amount is not its credit's",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.amount += 1;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output of another class than its flags give",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.class = BalanceClass::MinedCoinbase;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output whose credit's spender debits another output of its transaction",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                // rec1:1 (unspent, an unmined input names it) names rec2's
                // live debit, which spends rec1:0: same hash, another
                // index. (A spender with no debit at all is QK-0095's over
                // QK-0781's, accepted.)
                let k = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 1,
                };
                let mut c: CreditRow = tx.get::<Credits>(&k).unwrap().unwrap();
                c.spender = Some(Spender {
                    hash: b.rec2,
                    height: 2,
                    input: 0,
                });
                tx.put::<Credits>(&k, &c).unwrap();
            }),
        ),
        (
            "5 (decoded): an unspent output spent by a regular debit of its own block",
            Refuser::Decoded,
            "unspent output ",
            edit(|tx, b| {
                // rec2 "spends" its own output 1 at input 5: a live debit
                // naming the credit, of a regular record at its height.
                // rec2 has no input 5, which `check_decoded` would refuse
                // too (D3), but it checks QK-0785's spender first.
                let credit = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let debit = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 5,
                };
                let mut c: CreditRow = tx.get::<Credits>(&credit).unwrap().unwrap();
                tx.put::<Debits>(
                    &debit,
                    &DebitRow {
                        amount: c.amount,
                        credit,
                    },
                )
                .unwrap();
                c.spender = Some(Spender {
                    hash: b.rec2,
                    height: 2,
                    input: 5,
                });
                tx.put::<Credits>(&credit, &c).unwrap();
            }),
        ),
        (
            "6: a credit without its record",
            Refuser::Store,
            "credits row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let c: CreditRow = tx.get::<Credits>(&k).unwrap().unwrap();
                let k = CreditKey {
                    hash: Hash32([0xee; 32]),
                    ..k
                };
                tx.put::<Credits>(&k, &c).unwrap();
            }),
        ),
        (
            "6: a debit without its record",
            Refuser::Store,
            "debits row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: Hash32([0xee; 32]),
                    height: 2,
                    index: 0,
                };
                let credit = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 0,
                };
                tx.put::<Debits>(&k, &DebitRow { amount: 1, credit })
                    .unwrap();
            }),
        ),
        (
            "7: a debit naming a credit that is not recorded",
            Refuser::Store,
            "debits row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 9,
                };
                let credit = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 9,
                };
                tx.put::<Debits>(&k, &DebitRow { amount: 1, credit })
                    .unwrap();
            }),
        ),
        (
            "8: an invalidated credit naming a spender that is no debit of it",
            Refuser::Store,
            "credits_invalidated row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let mut c: CreditRow = tx.get::<CreditsInvalidated>(&k).unwrap().unwrap();
                c.spender = Some(Spender {
                    hash: b.rec2,
                    height: 2,
                    input: 0,
                });
                tx.put::<CreditsInvalidated>(&k, &c).unwrap();
            }),
        ),
        (
            "9: invalidated rows in a block not marked invalid",
            Refuser::Store,
            "credits_invalidated row ",
            edit(|tx, _| {
                let mut row = tx.get::<Blocks>(&Height(3)).unwrap().unwrap();
                row.invalid = false;
                tx.put::<Blocks>(&Height(3), &row).unwrap();
            }),
        ),
        (
            "10: an unspent commitment entry without its commitment",
            Refuser::Store,
            "unspent commitment entry ",
            edit(|tx, _| {
                let o = Outpoint {
                    hash: Hash32([0xee; 32]),
                    index: 1,
                };
                tx.put::<CommitmentsUnspent>(
                    &o,
                    &CommitmentUnspentRow {
                        unmined_spent: false,
                    },
                )
                .unwrap();
            }),
        ),
        (
            "11: a balance sum off by one atom",
            Refuser::Store,
            "the balance of account 0 class MinedRegular ",
            edit(|tx, _| {
                let k = BalanceKey {
                    account: 0,
                    class: BalanceClass::MinedRegular,
                };
                let mut row = tx.get::<Balances>(&k).unwrap().unwrap();
                row.sum += 1;
                tx.put::<Balances>(&k, &row).unwrap();
            }),
        ),
        (
            "11: a balance row missing",
            Refuser::Store,
            "account 0 class Locked has no balance row",
            edit(|tx, _| {
                let k = BalanceKey {
                    account: 0,
                    class: BalanceClass::Locked,
                };
                assert!(tx.delete::<Balances>(&k).unwrap());
            }),
        ),
        (
            "11: a balance row of no output",
            Refuser::Store,
            "the balance of account 5 class MinedRegular ",
            edit(|tx, _| {
                let k = BalanceKey {
                    account: 5,
                    class: BalanceClass::MinedRegular,
                };
                tx.put::<Balances>(&k, &BalanceRow { sum: 0, count: 0 })
                    .unwrap();
            }),
        ),
        (
            "12: a per-height balance at the wrong height",
            Refuser::Store,
            "the balance of account 1 class MinedRegular at height ",
            edit(|tx, _| {
                let rows: Vec<_> = tx
                    .iter::<BalanceHeights>()
                    .unwrap()
                    .map(|r| r.unwrap())
                    .filter(|(k, _)| k.account == 1)
                    .collect();
                let (k, row) = rows[0];
                tx.delete::<BalanceHeights>(&k).unwrap();
                let k = purse_store::BalanceHeightKey {
                    height: k.height + 1,
                    ..k
                };
                tx.put::<BalanceHeights>(&k, &row).unwrap();
            }),
        ),
        (
            "13: an account history row missing",
            Refuser::Store,
            "record ",
            edit(|tx, b| {
                let rows: Vec<_> = tx
                    .iter::<AcctHistory>()
                    .unwrap()
                    .map(|r| r.unwrap())
                    .filter(|(_, h)| *h == b.rec2)
                    .collect();
                assert!(tx.delete::<AcctHistory>(&rows[0].0).unwrap());
            }),
        ),
        (
            "13: an account history row too many",
            Refuser::Store,
            "acct_history holds ",
            edit(|tx, b| {
                let k = AcctHistoryKey {
                    account: 9,
                    height: 1,
                    position: 0,
                };
                tx.put::<AcctHistory>(&k, &b.rec1).unwrap();
            }),
        ),
        // Each sub-rule of the invariants above, so that weakening any one
        // of them fails this test.
        (
            "1: a history position skipped, the record and account history moved with it",
            Refuser::Store,
            "the history of block 2 has position 2 where 1 is next",
            edit(|tx, b| {
                let rk = TxRecKey {
                    hash: b.rec2,
                    height: 2,
                };
                let mut m = tx.get::<TxMined>(&rk).unwrap().unwrap();
                assert_eq!(m.position, 1, "rec2 is block 2's last record");
                tx.delete::<History>(&HistoryKey {
                    height: 2,
                    position: 1,
                })
                .unwrap();
                tx.put::<History>(
                    &HistoryKey {
                        height: 2,
                        position: 2,
                    },
                    &b.rec2,
                )
                .unwrap();
                m.position = 2;
                tx.put::<TxMined>(&rk, &m).unwrap();
                let rows: Vec<_> = tx
                    .iter::<AcctHistory>()
                    .unwrap()
                    .map(|r| r.unwrap())
                    .filter(|(_, h)| *h == b.rec2)
                    .collect();
                assert!(!rows.is_empty());
                for (k, h) in rows {
                    tx.delete::<AcctHistory>(&k).unwrap();
                    tx.put::<AcctHistory>(&AcctHistoryKey { position: 2, ..k }, &h)
                        .unwrap();
                }
            }),
        ),
        (
            "2: a history row naming no record, its block counting it",
            Refuser::Store,
            "history holds 6 rows for 5 records",
            edit(|tx, _| {
                let mut row = tx.get::<Blocks>(&Height(2)).unwrap().unwrap();
                let n = row.wallet_txs;
                row.wallet_txs += 1;
                tx.put::<Blocks>(&Height(2), &row).unwrap();
                tx.put::<History>(
                    &HistoryKey {
                        height: 2,
                        position: n,
                    },
                    &Hash32([0xee; 32]),
                )
                .unwrap();
            }),
        ),
        (
            "2: a history row naming another hash than its record",
            Refuser::Store,
            "record ",
            edit(|tx, _| {
                let k = HistoryKey {
                    height: 2,
                    position: 1,
                };
                tx.put::<History>(&k, &Hash32([0xee; 32])).unwrap();
            }),
        ),
        (
            "5: an unspent output of another account than its credit",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.account = 0;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output with other flags than its credit",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.flags.change = !u.flags.change;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output at another script location than its credit",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut u = tx.get::<Unspent>(&o).unwrap().unwrap();
                u.script.offset += 1;
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "5: an unspent output whose credit names an invalidated debit of it",
            Refuser::Store,
            "unspent output ",
            edit(|tx, b| {
                let ck = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let mut c: CreditRow = tx.get::<Credits>(&ck).unwrap().unwrap();
                tx.put::<DebitsInvalidated>(
                    &CreditKey {
                        hash: b.rec3,
                        height: 3,
                        index: 5,
                    },
                    &DebitRow {
                        amount: c.amount,
                        credit: ck,
                    },
                )
                .unwrap();
                c.spender = Some(Spender {
                    hash: b.rec3,
                    height: 3,
                    input: 5,
                });
                tx.put::<Credits>(&ck, &c).unwrap();
            }),
        ),
        (
            "7: a debit naming a missing credit at its own height",
            Refuser::Store,
            "debits row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 9,
                };
                let credit = CreditKey {
                    hash: Hash32([0xee; 32]),
                    height: 2,
                    index: 0,
                };
                tx.put::<Debits>(&k, &DebitRow { amount: 1, credit })
                    .unwrap();
            }),
        ),
        (
            "7: a debit whose amount is not its credit's",
            Refuser::Store,
            "debits row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 0,
                };
                let mut d: DebitRow = tx.get::<Debits>(&k).unwrap().unwrap();
                d.amount += 12_345;
                tx.put::<Debits>(&k, &d).unwrap();
            }),
        ),
        (
            "9: a live debit moved to the invalidated table of a valid block",
            Refuser::Store,
            "debits_invalidated row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 0,
                };
                let d: DebitRow = tx.get::<Debits>(&k).unwrap().unwrap();
                tx.delete::<Debits>(&k).unwrap();
                tx.put::<DebitsInvalidated>(&k, &d).unwrap();
            }),
        ),
        (
            "12: a per-height balance row missing",
            Refuser::Store,
            "account 1 class MinedRegular has no balance row at height 2",
            edit(|tx, _| {
                let k = purse_store::BalanceHeightKey {
                    account: 1,
                    class: BalanceClass::MinedRegular,
                    height: 2,
                };
                assert!(tx.delete::<BalanceHeights>(&k).unwrap());
            }),
        ),
        (
            "13: an account history row naming another hash",
            Refuser::Store,
            "record ",
            edit(|tx, b| {
                let (k, _) = tx
                    .iter::<AcctHistory>()
                    .unwrap()
                    .map(|r| r.unwrap())
                    .find(|(_, h)| *h == b.rec2)
                    .unwrap();
                tx.put::<AcctHistory>(&k, &Hash32([0xee; 32])).unwrap();
            }),
        ),
        (
            "6: an invalidated credit without its record",
            Refuser::Store,
            "credits_invalidated row ",
            edit(|tx, b| {
                // In block 3, which is marked invalid, so 9 holds.
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let c: CreditRow = tx.get::<CreditsInvalidated>(&k).unwrap().unwrap();
                let k = CreditKey {
                    hash: Hash32([0xee; 32]),
                    ..k
                };
                tx.put::<CreditsInvalidated>(&k, &c).unwrap();
            }),
        ),
        (
            "6: an invalidated debit without its record",
            Refuser::Store,
            "debits_invalidated row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let d: DebitRow = tx.get::<DebitsInvalidated>(&k).unwrap().unwrap();
                let k = CreditKey {
                    hash: Hash32([0xee; 32]),
                    ..k
                };
                tx.put::<DebitsInvalidated>(&k, &d).unwrap();
            }),
        ),
        (
            "7: a debit whose amount is not its invalidated credit's",
            Refuser::Store,
            "debits row ",
            edit(|tx, b| {
                // Block 1 marked invalid and rec1:0, which rec2 spends,
                // moved to the invalidated table, its account history row
                // with it (the same edit without the extra atom passes the
                // store's part); rec2's debit one atom off.
                let mut row = tx.get::<Blocks>(&Height(1)).unwrap().unwrap();
                row.invalid = true;
                tx.put::<Blocks>(&Height(1), &row).unwrap();
                let k = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 0,
                };
                let c: CreditRow = tx.get::<Credits>(&k).unwrap().unwrap();
                tx.delete::<Credits>(&k).unwrap();
                tx.put::<CreditsInvalidated>(&k, &c).unwrap();
                let rk = TxRecKey {
                    hash: b.rec1,
                    height: 1,
                };
                let position = tx.get::<TxMined>(&rk).unwrap().unwrap().position;
                tx.delete::<AcctHistory>(&AcctHistoryKey {
                    account: c.account,
                    height: 1,
                    position,
                })
                .unwrap();
                let dk = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 0,
                };
                let mut d: DebitRow = tx.get::<Debits>(&dk).unwrap().unwrap();
                d.amount += 1;
                tx.put::<Debits>(&dk, &d).unwrap();
            }),
        ),
        (
            "8: a credit naming a debit of another output of its transaction",
            Refuser::Store,
            "credits row ",
            edit(|tx, b| {
                // rec1:1 names rec2's debit, which spends rec1:0; its
                // unspent row dropped (an unmined input names it, so no
                // balance counts it) so that 5 does not see it first.
                let o = Outpoint {
                    hash: b.rec1,
                    index: 1,
                };
                assert!(tx.contains::<UnminedInputs>(&o).unwrap());
                assert!(tx.delete::<Unspent>(&o).unwrap());
                let k = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 1,
                };
                let mut c: CreditRow = tx.get::<Credits>(&k).unwrap().unwrap();
                c.spender = Some(Spender {
                    hash: b.rec2,
                    height: 2,
                    input: 0,
                });
                tx.put::<Credits>(&k, &c).unwrap();
            }),
        ),
        (
            "8: an invalidated credit naming an invalidated debit of another outpoint",
            Refuser::Store,
            "credits_invalidated row ",
            edit(|tx, b| {
                // rec3:0 names rec3's own invalidated debit, which spends
                // rec2:0.
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let mut c: CreditRow = tx.get::<CreditsInvalidated>(&k).unwrap().unwrap();
                c.spender = Some(Spender {
                    hash: b.rec3,
                    height: 3,
                    input: 0,
                });
                tx.put::<CreditsInvalidated>(&k, &c).unwrap();
            }),
        ),
        // The decoded invariants (`txstore::check_decoded`).
        (
            "D1 (decoded): a credit and its unspent output at another script location",
            Refuser::Decoded,
            "credits row ",
            edit(|tx, b| {
                let ck = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut c: CreditRow = tx.get::<Credits>(&ck).unwrap().unwrap();
                let mut u: UnspentRow = tx.get::<Unspent>(&o).unwrap().unwrap();
                c.script.offset += 1;
                u.script.offset += 1;
                tx.put::<Credits>(&ck, &c).unwrap();
                tx.put::<Unspent>(&o, &u).unwrap();
            }),
        ),
        (
            "D1 (decoded): a credit and its unspent output past the record's outputs",
            Refuser::Decoded,
            "credits row ",
            edit(|tx, b| {
                let ck = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let c: CreditRow = tx.get::<Credits>(&ck).unwrap().unwrap();
                let u: UnspentRow = tx.get::<Unspent>(&o).unwrap().unwrap();
                tx.put::<Credits>(&CreditKey { index: 5, ..ck }, &c)
                    .unwrap();
                tx.put::<Unspent>(&Outpoint { index: 5, ..o }, &u).unwrap();
                add_to_sums(tx, u.account, u.class, Some(u.height), u.amount);
            }),
        ),
        (
            "D1 (decoded): a credit and its unspent output holding another amount",
            Refuser::Decoded,
            "credits row ",
            edit(|tx, b| {
                let ck = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let o = Outpoint {
                    hash: b.rec2,
                    index: 1,
                };
                let mut c: CreditRow = tx.get::<Credits>(&ck).unwrap().unwrap();
                let mut u: UnspentRow = tx.get::<Unspent>(&o).unwrap().unwrap();
                c.amount += 1;
                u.amount += 1;
                tx.put::<Credits>(&ck, &c).unwrap();
                tx.put::<Unspent>(&o, &u).unwrap();
                // One atom more in the sums, the output counted once.
                add_to_sums(tx, u.account, u.class, Some(u.height), 1);
                undo_count(tx, u.account, u.class, Some(u.height));
            }),
        ),
        (
            "D1 (decoded): an invalidated credit of another script type",
            Refuser::Decoded,
            "credits_invalidated row ",
            edit(|tx, b| {
                let ck = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let mut c: CreditRow = tx.get::<CreditsInvalidated>(&ck).unwrap().unwrap();
                c.script.class = if c.script.class == 6 { 2 } else { 6 };
                tx.put::<CreditsInvalidated>(&ck, &c).unwrap();
            }),
        ),
        (
            "D2 (decoded): an unmined credit holding another amount",
            Refuser::Decoded,
            "unmined credit ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec4,
                    index: 0,
                };
                let mut m: UnminedCreditRow = tx.get::<UnminedCredits>(&o).unwrap().unwrap();
                m.amount += 1;
                tx.put::<UnminedCredits>(&o, &m).unwrap();
                add_to_sums(tx, m.account, m.class, None, 1);
                undo_count(tx, m.account, m.class, None);
            }),
        ),
        (
            "D3 (decoded): a debit of an input the record does not have",
            Refuser::Decoded,
            "debits row ",
            edit(|tx, b| {
                // rec2's one input spends rec1:0; a second debit of input 1
                // names rec1:1, which the unmined rec4 spends.
                let credit = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 1,
                };
                let c: CreditRow = tx.get::<Credits>(&credit).unwrap().unwrap();
                tx.put::<Debits>(
                    &CreditKey {
                        hash: b.rec2,
                        height: 2,
                        index: 1,
                    },
                    &DebitRow {
                        amount: c.amount,
                        credit,
                    },
                )
                .unwrap();
            }),
        ),
        (
            "D3 (decoded): an invalidated debit naming another outpoint than its input spends",
            Refuser::Decoded,
            "debits_invalidated row ",
            edit(|tx, b| {
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let credit = CreditKey {
                    hash: b.rec2,
                    height: 2,
                    index: 1,
                };
                let c: CreditRow = tx.get::<Credits>(&credit).unwrap().unwrap();
                tx.put::<DebitsInvalidated>(
                    &k,
                    &DebitRow {
                        amount: c.amount,
                        credit,
                    },
                )
                .unwrap();
            }),
        ),
        (
            "D3 (decoded): an invalidated debit naming another transaction's output of its index",
            Refuser::Decoded,
            "debits_invalidated row ",
            edit(|tx, b| {
                // rec3's input 0 spends rec2:0; the debit names rec1:0.
                let credit = CreditKey {
                    hash: b.rec1,
                    height: 1,
                    index: 0,
                };
                let c: CreditRow = tx.get::<Credits>(&credit).unwrap().unwrap();
                tx.put::<DebitsInvalidated>(
                    &CreditKey {
                        hash: b.rec3,
                        height: 3,
                        index: 0,
                    },
                    &DebitRow {
                        amount: c.amount,
                        credit,
                    },
                )
                .unwrap();
            }),
        ),
        (
            "D4 (decoded): an unmined input naming its spender at another index",
            Refuser::Decoded,
            "unmined input ",
            edit(|tx, b| {
                let o = Outpoint {
                    hash: b.rec1,
                    index: 5,
                };
                tx.put::<UnminedInputs>(&o, &b.rec4).unwrap();
            }),
        ),
        (
            "D6 (decoded): a mined record no credit or debit names that holds no transaction",
            Refuser::Decoded,
            "record 7777",
            edit(|tx, _| {
                let mut row = tx.get::<Blocks>(&Height(2)).unwrap().unwrap();
                let position = row.wallet_txs;
                row.wallet_txs += 1;
                tx.put::<Blocks>(&Height(2), &row).unwrap();
                let x = Hash32([0x77; 32]);
                tx.put::<History>(
                    &HistoryKey {
                        height: 2,
                        position,
                    },
                    &x,
                )
                .unwrap();
                tx.put::<TxMined>(
                    &TxRecKey { hash: x, height: 2 },
                    &purse_store::TxMinedRow {
                        position,
                        received: 1,
                        tx: vec![0xff, 0x01],
                    },
                )
                .unwrap();
            }),
        ),
        (
            "D6 (decoded): an unmined record no row names that holds no transaction",
            Refuser::Decoded,
            "unmined record 7878",
            edit(|tx, _| {
                tx.put::<TxUnmined>(
                    &Hash32([0x78; 32]),
                    &purse_store::TxUnminedRow {
                        received: 1,
                        tx: vec![0xff, 0x01],
                    },
                )
                .unwrap();
            }),
        ),
        (
            "D7 (decoded): a valid block with regular records marked invalid",
            Refuser::Decoded,
            "a block marked invalid at height 2 holds record ",
            edit(|tx, _| {
                let mut row = tx.get::<Blocks>(&Height(2)).unwrap().unwrap();
                row.invalid = true;
                tx.put::<Blocks>(&Height(2), &row).unwrap();
            }),
        ),
        (
            "D7 (decoded): a live credit of a regular record in a block marked invalid",
            Refuser::Decoded,
            "a block marked invalid at height 3 holds record ",
            edit(|tx, b| {
                // rec3:0 moved back to the live table, with the account
                // history row a live credit gives its record.
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let c: CreditRow = tx.get::<CreditsInvalidated>(&k).unwrap().unwrap();
                tx.delete::<CreditsInvalidated>(&k).unwrap();
                tx.put::<Credits>(&k, &c).unwrap();
                let rk = TxRecKey {
                    hash: b.rec3,
                    height: 3,
                };
                let position = tx.get::<TxMined>(&rk).unwrap().unwrap().position;
                tx.put::<AcctHistory>(
                    &AcctHistoryKey {
                        account: c.account,
                        height: 3,
                        position,
                    },
                    &b.rec3,
                )
                .unwrap();
            }),
        ),
        (
            "D7 (decoded): a live debit of a regular record in a block marked invalid",
            Refuser::Decoded,
            "a block marked invalid at height 3 holds record ",
            edit(|tx, b| {
                // rec3's debit of rec2:0 moved back to the live table;
                // rec2:0 names no spender, so no account history row is
                // due.
                let k = CreditKey {
                    hash: b.rec3,
                    height: 3,
                    index: 0,
                };
                let d: DebitRow = tx.get::<DebitsInvalidated>(&k).unwrap().unwrap();
                tx.delete::<DebitsInvalidated>(&k).unwrap();
                tx.put::<Debits>(&k, &d).unwrap();
            }),
        ),
    ]
}

/// Adds `amount` and one output to the sums of (`account`, `class`), and
/// at `height` for a windowed class.
fn add_to_sums(
    tx: &mut WriteTx<'_>,
    account: u32,
    class: BalanceClass,
    height: Option<u32>,
    amount: i64,
) {
    let k = purse_store::BalanceKey { account, class };
    let mut row = tx.get::<Balances>(&k).unwrap().unwrap_or_default();
    row.sum += amount;
    row.count += 1;
    tx.put::<Balances>(&k, &row).unwrap();
    if let Some(height) = height.filter(|_| class.is_windowed()) {
        let k = purse_store::BalanceHeightKey {
            account,
            class,
            height,
        };
        let mut row = tx.get::<BalanceHeights>(&k).unwrap().unwrap_or_default();
        row.sum += amount;
        row.count += 1;
        tx.put::<BalanceHeights>(&k, &row).unwrap();
    }
}

/// Takes one output off the counts [`add_to_sums`] added it to.
fn undo_count(tx: &mut WriteTx<'_>, account: u32, class: BalanceClass, height: Option<u32>) {
    let k = purse_store::BalanceKey { account, class };
    let mut row = tx.get::<Balances>(&k).unwrap().unwrap();
    row.count -= 1;
    tx.put::<Balances>(&k, &row).unwrap();
    if let Some(height) = height.filter(|_| class.is_windowed()) {
        let k = purse_store::BalanceHeightKey {
            account,
            class,
            height,
        };
        let mut row = tx.get::<BalanceHeights>(&k).unwrap().unwrap();
        row.count -= 1;
        tx.put::<BalanceHeights>(&k, &row).unwrap();
    }
}

/// The check, purse-store's part and then the decoded part, as
/// `txstore::verify_backup` runs them; which refused, and the text.
fn refusal(tx: &mut WriteTx<'_>) -> Option<(Refuser, String)> {
    if let Err(e) = tx.check_tx_tables() {
        let why = e.inconsistency().expect("an inconsistency").to_owned();
        return Some((Refuser::Store, why));
    }
    match purse_udb::txstore::check_decoded(tx) {
        Ok(()) => None,
        Err(e) => Some((
            Refuser::Decoded,
            e.inconsistency().expect("an inconsistency").to_owned(),
        )),
    }
}

/// ADR-0018 BK4: for each invariant of the transaction tables, and each
/// sub-rule of one (every field an unspent output shares with its credit,
/// each direction of the history index, the amount of a debit, ...), a
/// store with exactly that rule broken by a direct row edit is refused by
/// the check that holds it, with that invariant's text, and passes again
/// once the edit is undone (in the same transaction, the rows put back as
/// they were): 55 edits, 39 refused by `purse-store`'s `check_tx_tables`
/// and 16 by the decoded part, `check_decoded` (one of those, QK-0785's
/// spender, also leaves a debit of an input its record lacks, which
/// `check_decoded` checks later). The base store, built by the
/// operations, passes; so do the
/// states of Go's reproduced defects written the same way: a debit naming
/// a credit recorded above it that is gone (QK-0101), an unmined input
/// row missing for an unmined transaction's input (QK-0060), an outpoint
/// both an unspent output and an unmined credit, counted in both
/// (QK-0781), and an `unpublished` mark of a hash with no unmined record
/// (QK-0098).
#[test]
fn each_broken_invariant_of_the_transaction_tables_is_refused() {
    let (store, _, base) = corruption_base();
    let cases = corruptions();
    assert_eq!(cases.len(), 55);
    let decoded = cases.iter().filter(|c| c.1 == Refuser::Decoded).count();
    assert_eq!(decoded, 16, "refused by the decoded part");
    for (what, refuser, text, edit) in &cases {
        let r: Result<(), TErr> = store.update(|tx| {
            let before = dump(tx);
            assert_eq!(refusal(tx), None, "{what}: the base refused");
            edit(tx, &base);
            let (by, why) = refusal(tx).unwrap_or_else(|| panic!("{what}: not refused"));
            assert_eq!(by, *refuser, "{what}: refused by the other check: {why}");
            assert!(why.starts_with(text), "{what}: refused as {why:?}");
            restore_rows(tx, &before);
            assert_eq!(dump(tx), before);
            assert_eq!(refusal(tx), None, "{what}: refused once undone");
            Err(TErr::Rollback)
        });
        assert!(matches!(r, Err(TErr::Rollback)));
    }
    // States the check accepts.
    store
        .update(|tx| {
            use purse_store::{DebitRow, Outpoint, UnspentRow};
            // QK-0101: rec1 (block 1) debits, at its input 0, a credit of
            // the outpoint that input spends recorded in block 2 and gone,
            // as after block 2's rollback.
            let gone = CreditKey {
                hash: Hash32([8; 32]),
                height: 2,
                index: 0,
            };
            let debit = CreditKey {
                hash: base.rec1,
                height: 1,
                index: 0,
            };
            tx.put::<Debits>(
                &debit,
                &DebitRow {
                    amount: 3,
                    credit: gone,
                },
            )
            .unwrap();
            assert_eq!(refusal(tx), None, "QK-0101's dangling debit");
            // QK-0060: rec4's input row deleted by the removal of another
            // spender; rec1:1 counts in the balance again, as the mutators
            // count it.
            let o = Outpoint {
                hash: base.rec1,
                index: 1,
            };
            let u: UnspentRow = tx.get::<Unspent>(&o).unwrap().unwrap();
            tx.delete::<UnminedInputs>(&o).unwrap();
            let bump = |tx: &mut WriteTx<'_>, account, class, height: Option<u32>, amount| {
                let k = purse_store::BalanceKey { account, class };
                let mut row = tx.get::<Balances>(&k).unwrap().unwrap_or_default();
                row.sum += amount;
                row.count += 1;
                tx.put::<Balances>(&k, &row).unwrap();
                if let Some(height) = height {
                    let k = purse_store::BalanceHeightKey {
                        account,
                        class,
                        height,
                    };
                    let mut row = tx.get::<BalanceHeights>(&k).unwrap().unwrap_or_default();
                    row.sum += amount;
                    row.count += 1;
                    tx.put::<BalanceHeights>(&k, &row).unwrap();
                }
            };
            bump(tx, u.account, u.class, Some(u.height), u.amount);
            assert_eq!(refusal(tx), None, "QK-0060's missing input row");
            // QK-0781: rec2:1 also an unmined credit of an unmined copy of
            // rec2, each counted.
            let rec2: purse_store::TxMinedRow = tx
                .get::<TxMined>(&purse_store::TxRecKey {
                    hash: base.rec2,
                    height: 2,
                })
                .unwrap()
                .unwrap();
            tx.put::<TxUnmined>(
                &base.rec2,
                &purse_store::TxUnminedRow {
                    received: rec2.received,
                    tx: rec2.tx,
                },
            )
            .unwrap();
            let o = Outpoint {
                hash: base.rec2,
                index: 1,
            };
            let u: UnspentRow = tx.get::<Unspent>(&o).unwrap().unwrap();
            let class = BalanceClass::unmined(&u.flags, false);
            tx.put::<UnminedCredits>(
                &o,
                &purse_store::UnminedCreditRow {
                    amount: u.amount,
                    flags: u.flags,
                    account: u.account,
                    class,
                    script: u.script,
                },
            )
            .unwrap();
            bump(tx, u.account, class, None, u.amount);
            assert_eq!(refusal(tx), None, "QK-0781's doubled output");
            // QK-0098: a mark of a hash with no unmined record.
            tx.put::<Unpublished>(&Hash32([0xee; 32]), &purse_store::UnpublishedFlag)
                .unwrap();
            assert_eq!(refusal(tx), None, "QK-0098's stray mark");
            Err::<(), _>(TErr::Rollback)
        })
        .expect_err("abandoned");
}

/// `txstore::check_decoded` reads each table it walks (`unmined_inputs`,
/// `unspent`, `tx_mined`, `tx_unmined`, and `unmined_credits` when a
/// credit lacks its record) a page of 512 rows at a time. Each of these
/// tables gets 2 × 512 + 1 rows the decoded check accepts and then one
/// row, sorting last, that it refuses, which only a walk past the second
/// page reaches: the check refuses it with that row's text, and passes
/// once it is deleted. (The rows are not a store the operations write;
/// only the decoded part is run, and it reads nothing else.)
#[test]
fn check_decoded_reads_past_the_first_page() {
    use purse_store::{
        CreditRow, DebitRow, Outpoint, Spender, TxMinedRow, TxRecKey, TxUnminedRow, UnspentRow,
    };
    const N: u32 = 2 * 512 + 1;
    let last = Hash32([0xff; 32]);
    let key = |i: u32| {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&i.to_be_bytes());
        Hash32(h)
    };
    let valid = TxRecord::from_msg_tx(paying(&[(Hash([0x31; 32]), 0)], &[1000], 1), 0)
        .expect("record")
        .serialized()
        .to_vec();
    let junk = vec![0xff, 0x01];
    let (store, ts, _) = tx_store("testnet3");
    let case = |what: &str,
                text: &str,
                fill: &dyn Fn(&TxStore, &mut WriteTx<'_>),
                undo: &dyn Fn(&mut WriteTx<'_>)| {
        let r: Result<(), TErr> = store.update(|tx| {
            fill(&ts, tx);
            let e = purse_udb::txstore::check_decoded(tx)
                .expect_err(&format!("{what}: the row past two pages is refused"));
            let why = e.inconsistency().expect("an inconsistency");
            assert!(why.starts_with(text), "{what}: refused as {why:?}");
            undo(tx);
            purse_udb::txstore::check_decoded(tx)
                .unwrap_or_else(|e| panic!("{what}: refused once undone: {e}"));
            Err(TErr::Rollback)
        });
        assert!(matches!(r, Err(TErr::Rollback)));
    };
    case(
        "tx_mined",
        "record ffff",
        &|_, tx| {
            for i in 0..N {
                let row = TxMinedRow {
                    position: i,
                    received: 0,
                    tx: valid.clone(),
                };
                tx.put::<TxMined>(
                    &TxRecKey {
                        hash: key(i),
                        height: 1,
                    },
                    &row,
                )
                .unwrap();
            }
            let row = TxMinedRow {
                position: N,
                received: 0,
                tx: junk.clone(),
            };
            tx.put::<TxMined>(
                &TxRecKey {
                    hash: last,
                    height: 1,
                },
                &row,
            )
            .unwrap();
        },
        &|tx| {
            assert!(
                tx.delete::<TxMined>(&TxRecKey {
                    hash: last,
                    height: 1
                })
                .unwrap()
            );
        },
    );
    case(
        "tx_unmined",
        "unmined record ffff",
        &|_, tx| {
            for i in 0..N {
                let row = TxUnminedRow {
                    received: 0,
                    tx: valid.clone(),
                };
                tx.put::<TxUnmined>(&key(i), &row).unwrap();
            }
            let row = TxUnminedRow {
                received: 0,
                tx: junk.clone(),
            };
            tx.put::<TxUnmined>(&last, &row).unwrap();
        },
        &|tx| {
            assert!(tx.delete::<TxUnmined>(&last).unwrap());
        },
    );
    case(
        "unmined_inputs",
        "unmined input ffff",
        &|_, tx| {
            let inputs: Vec<(Hash, u32)> = (0..N).map(|i| (Hash([0x11; 32]), i)).collect();
            let x = TxRecord::from_msg_tx(paying(&inputs, &[1], 2), 0).expect("record");
            let row = TxUnminedRow {
                received: 0,
                tx: x.serialized().to_vec(),
            };
            tx.put::<TxUnmined>(x.hash(), &row).unwrap();
            for i in 0..N {
                let o = Outpoint {
                    hash: Hash32([0x11; 32]),
                    index: i,
                };
                tx.put::<UnminedInputs>(&o, x.hash()).unwrap();
            }
            let o = Outpoint {
                hash: last,
                index: 0,
            };
            tx.put::<UnminedInputs>(&o, x.hash()).unwrap();
        },
        &|tx| {
            let o = Outpoint {
                hash: last,
                index: 0,
            };
            assert!(tx.delete::<UnminedInputs>(&o).unwrap());
        },
    );
    case(
        "unspent",
        "unspent output ffff",
        &|_, tx| {
            let row = UnspentRow {
                height: 1,
                account: 0,
                amount: 1,
                flags: purse_store::CreditFlags::default(),
                class: BalanceClass::MinedRegular,
                script: purse_store::ScriptLoc::default(),
            };
            for i in 0..N {
                let o = Outpoint {
                    hash: key(i),
                    index: 0,
                };
                tx.put::<Unspent>(&o, &row).unwrap();
            }
            // The last one's credit is spent by a live debit of it, of a
            // regular transaction of its own block: QK-0785 allows only a
            // stake spender or one at another height.
            let s =
                TxRecord::from_msg_tx(paying(&[(Hash(last.0), 0)], &[1], 3), 0).expect("record");
            let ck = CreditKey {
                hash: last,
                height: 1,
                index: 0,
            };
            tx.put::<Unspent>(
                &Outpoint {
                    hash: last,
                    index: 0,
                },
                &row,
            )
            .unwrap();
            let credit = CreditRow {
                amount: 1,
                spender: Some(Spender {
                    hash: *s.hash(),
                    height: 1,
                    input: 0,
                }),
                ..CreditRow::default()
            };
            tx.put::<Credits>(&ck, &credit).unwrap();
            let dk = CreditKey {
                hash: *s.hash(),
                height: 1,
                index: 0,
            };
            tx.put::<Debits>(
                &dk,
                &DebitRow {
                    amount: 1,
                    credit: ck,
                },
            )
            .unwrap();
            let rec = TxMinedRow {
                position: 0,
                received: 0,
                tx: s.serialized().to_vec(),
            };
            tx.put::<TxMined>(
                &TxRecKey {
                    hash: *s.hash(),
                    height: 1,
                },
                &rec,
            )
            .unwrap();
        },
        &|tx| {
            let o = Outpoint {
                hash: last,
                index: 0,
            };
            assert!(tx.delete::<Unspent>(&o).unwrap());
        },
    );
    case(
        "unmined_credits",
        "unmined credit ffff",
        &|ts, tx| {
            let values: Vec<i64> = (0..N).map(|i| 1000 + i64::from(i)).collect();
            let y = TxRecord::from_msg_tx(paying(&[(Hash([0x12; 32]), 0)], &values, 4), 0)
                .expect("record");
            ts.insert_mem_pool_tx(tx, &y).expect("InsertMemPoolTx");
            for i in 0..N {
                ts.add_credit(tx, &y, None, i, false, 0).expect("AddCredit");
            }
            let o = Outpoint {
                hash: *y.hash(),
                index: 0,
            };
            let row = tx.get::<UnminedCredits>(&o).unwrap().unwrap();
            tx.put::<UnminedCredits>(
                &Outpoint {
                    hash: last,
                    index: 0,
                },
                &row,
            )
            .unwrap();
        },
        &|tx| {
            let o = Outpoint {
                hash: last,
                index: 0,
            };
            assert!(tx.delete::<UnminedCredits>(&o).unwrap());
        },
    );
    store.close().expect("close");
}

/// A storage backend over shared bytes, so that a backup written into it
/// can be verified from it (`Store::backup_to_backend`).
#[derive(Clone, Debug, Default)]
struct Image(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl purse_store::StorageBackend for Image {
    fn len(&self) -> Result<u64, purse_store::BackendError> {
        Ok(self.0.lock().expect("lock").len() as u64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), purse_store::BackendError> {
        let data = self.0.lock().expect("lock");
        let a = usize::try_from(offset).expect("offset");
        out.copy_from_slice(&data[a..a + out.len()]);
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), purse_store::BackendError> {
        let len = usize::try_from(len).expect("length");
        self.0.lock().expect("lock").resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), purse_store::BackendError> {
        Ok(())
    }

    fn write(&self, offset: u64, src: &[u8]) -> Result<(), purse_store::BackendError> {
        let mut data = self.0.lock().expect("lock");
        let a = usize::try_from(offset).expect("offset");
        if data.len() < a + src.len() {
            data.resize(a + src.len(), 0);
        }
        data[a..a + src.len()].copy_from_slice(src);
        Ok(())
    }
}

/// ADR-0018 BK4 through the verify path. A backup file of the corruption
/// base verifies with `txstore::verify_backup`, and so does a plain copy
/// of the stopped store. Then, for each corruption, a backup of the store
/// with the edit committed (into memory, `backup_to_backend`, as
/// `purse-store`'s fault-injection tests take theirs) is refused by
/// `txstore::verify_backup_on_backend` with the same check's text, while
/// `purse-store`'s verify alone refuses exactly the store's part; a backup
/// taken once the edit is undone verifies again; and the last bad backup,
/// written to a file, is refused by the file path too.
#[test]
fn verify_backup_runs_the_transaction_tables_check() {
    let (mut store, path, base) = corruption_base();
    let dir = path.parent().expect("dir").to_path_buf();
    let backups = dir.join("backups");
    let scratch = dir.join("verify");
    let info = purse_store::BackupInfo {
        network: "testnet3",
        version: "test",
        time: 0,
    };
    let b = store.backup(&backups, "base", &info).expect("backup");
    let v = purse_udb::txstore::verify_backup(&b.path, &scratch, Some("testnet3"))
        .expect("the base verifies");
    assert!(v.consistency_checked);
    assert_eq!(v.record.as_ref(), Some(&b.record));
    store.close().expect("close");
    let plain = dir.join("plain.redb");
    std::fs::copy(&path, &plain).expect("copy the stopped store");
    let v = purse_udb::txstore::verify_backup(&plain, &scratch, None).expect("a plain copy");
    assert_eq!(v.record, None);
    store = Store::open(&path).expect("reopen");
    let image = |store: &Store| {
        let out = Image::default();
        let record = store.backup_to_backend(out.clone(), &info).expect("backup");
        (out, record)
    };
    let cases = corruptions();
    for (n, (what, refuser, text, edit)) in cases.iter().enumerate() {
        let before = dump_read(&store.read().expect("read"));
        store
            .update(|tx| {
                edit(tx, &base);
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit the edit");
        let (bad, _) = image(&store);
        let snapshot = bad.0.lock().expect("lock").clone();
        let e =
            purse_udb::txstore::verify_backup_on_backend(bad, Some("testnet3")).expect_err(what);
        let why = e.inconsistency().unwrap_or_else(|| panic!("{what}: {e}"));
        assert!(why.starts_with(text), "{what}: refused as {why:?}");
        let store_only = purse_store::verify_backup_on_backend(
            Image(std::sync::Arc::new(std::sync::Mutex::new(snapshot))),
            Some("testnet3"),
        );
        assert_eq!(
            store_only.is_ok(),
            *refuser == Refuser::Decoded,
            "{what}: purse-store's verify alone: {store_only:?}"
        );
        if n + 1 == cases.len() {
            let b = store.backup(&backups, "bad", &info).expect("backup");
            let e = purse_udb::txstore::verify_backup(&b.path, &scratch, Some("testnet3"))
                .expect_err(what);
            assert!(
                e.inconsistency().is_some_and(|w| w.starts_with(text)),
                "{e}"
            );
        }
        store
            .update(|tx| {
                restore_rows(tx, &before);
                Ok::<_, purse_store::Error>(())
            })
            .expect("undo the edit");
        assert_eq!(dump_read(&store.read().expect("read")), before);
        let (good, record) = image(&store);
        let v = purse_udb::txstore::verify_backup_on_backend(good, Some("testnet3"))
            .unwrap_or_else(|e| panic!("{what}: undone, refused: {e}"));
        assert_eq!(v.record, Some(record));
    }
    store.close().expect("close");
}

/// `purse-store`'s checked-in version-1 fixture (a row in every table,
/// `crates/purse-store/tests/data/schema-v1.redb`) passes the full verify,
/// `txstore::verify_backup` with its decoded part, as a plain copy of a
/// stopped store; and so does a backup of it.
#[test]
fn the_schema_fixture_passes_the_full_verify() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../purse-store/tests/data/schema-v1.redb");
    let dir = scratch("fixture-verify");
    let copy = dir.join("schema-v1.redb");
    std::fs::copy(&src, &copy).expect("copy the fixture");
    let verify = dir.join("verify");
    let v = purse_udb::txstore::verify_backup(&copy, &verify, None)
        .unwrap_or_else(|e| panic!("the fixture fails the full verify: {e}"));
    assert!(v.consistency_checked);
    assert_eq!(v.record, None);
    let store = Store::open(&copy).expect("open the copy");
    let info = purse_store::BackupInfo {
        network: "simnet",
        version: "test",
        time: 0,
    };
    let b = store
        .backup(&dir.join("backups"), "fixture", &info)
        .expect("backup");
    store.close().expect("close");
    let v = purse_udb::txstore::verify_backup(&b.path, &verify, Some("simnet"))
        .unwrap_or_else(|e| panic!("a backup of the fixture fails the full verify: {e}"));
    assert_eq!(v.record.as_ref(), Some(&b.record));
}

/// The auxiliary records' longest keys beside the transaction tables pass
/// every check verify runs. `put_ticket_agenda_pref` applies bbolt's key
/// limit to dcrwallet's version ‖ agenda key inside the ticket's bucket,
/// so its `ticket_agenda_prefs` key, the ticket hash before it, is 32,800
/// bytes, 32 more than `MAX_KEY_SIZE`; a VSP host of `MAX_KEY_SIZE` bytes
/// is a `vsp_hosts` value and a `vsp_pub_keys` key, which the record
/// tables' index check reads. On the corruption base, the library's check
/// accepts both inside the write transaction, the read side's checks
/// (rows, chain, records, transaction tables and their decoded part)
/// accept them once committed, and `txstore::verify_backup` accepts a
/// backup file of the store and `verify_backup_on_backend` an in-memory
/// one. No check reads a key's length (the store's writes hold the
/// limits: `purse-store`'s `ticket_agenda_choices_take_dcrwallets_key_limit`
/// refuses one byte more), so there is no longer key for verify to refuse.
#[test]
fn verify_accepts_an_agenda_key_at_dcrwallet_s_limit() {
    use purse_store::{AgendaChoice, MAX_KEY_SIZE, TicketAgendaKey, TicketAgendaPrefs};
    let (store, path, _) = corruption_base();
    let key = TicketAgendaKey {
        ticket: Hash32([0x3c; 32]),
        version: 10,
        agenda: vec![b'a'; MAX_KEY_SIZE - 4],
    };
    let host = vec![b'h'; MAX_KEY_SIZE];
    store
        .update(|tx| {
            tx.put_ticket_agenda_pref(&key, &AgendaChoice(b"yes".to_vec()))?;
            assert_eq!(tx.add_vsp_host(&host, &[7; 32])?, 1);
            if let Some((by, why)) = refusal(tx) {
                panic!("refused by {by:?}: {why}");
            }
            check_rows(tx);
            Ok::<_, purse_store::Error>(())
        })
        .expect("update");
    {
        let r = store.read().expect("read");
        let keys: Vec<usize> = r
            .range_raw(
                TableId::TicketAgendaPrefs,
                Bound::Unbounded,
                Bound::Unbounded,
            )
            .expect("range")
            .map(|e| e.expect("row").0.len())
            .collect();
        assert_eq!(keys, [MAX_KEY_SIZE + 32]);
        assert_eq!(
            r.get::<TicketAgendaPrefs>(&key).expect("get"),
            Some(AgendaChoice(b"yes".to_vec()))
        );
        r.check_rows().expect("check_rows");
        check_library_read(&r);
    }
    let dir = path.parent().expect("dir").to_path_buf();
    let info = purse_store::BackupInfo {
        network: "testnet3",
        version: "test",
        time: 0,
    };
    let b = store
        .backup(&dir.join("backups"), "agenda", &info)
        .expect("backup");
    let v = purse_udb::txstore::verify_backup(&b.path, &dir.join("verify"), Some("testnet3"))
        .unwrap_or_else(|e| panic!("a backup file fails the full verify: {e}"));
    assert!(v.consistency_checked);
    assert_eq!(v.record.as_ref(), Some(&b.record));
    let image = Image::default();
    let record = store
        .backup_to_backend(image.clone(), &info)
        .expect("backup");
    let v = purse_udb::txstore::verify_backup_on_backend(image, Some("testnet3"))
        .unwrap_or_else(|e| panic!("an in-memory backup fails the full verify: {e}"));
    assert_eq!(v.record, Some(record));
    store.close().expect("close");
}

/// One state of Go's reproduced defects, written by the operations alone
/// on a fresh testnet3 store in one write transaction (`build`, which also
/// asserts that the state is there: rows a naive invariant would refuse).
/// The library's check passes it inside the transaction, as the property
/// tests' row checker does; once committed, the read side's checks pass it
/// (chain, records, transaction tables and their decoded part); and
/// `txstore::verify_backup_on_backend` verifies a backup of it.
fn accepted(what: &str, build: impl FnOnce(&TxStore, &mut WriteTx<'_>)) {
    let (store, ts, _) = tx_store("testnet3");
    store
        .update(|tx| {
            build(&ts, tx);
            if let Some((by, why)) = refusal(tx) {
                panic!("{what}: refused by {by:?}: {why}");
            }
            check_rows(tx);
            Ok::<_, purse_store::Error>(())
        })
        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
    check_library_read(&store.read().expect("read"));
    let image = Image::default();
    let info = purse_store::BackupInfo {
        network: "testnet3",
        version: "test",
        time: 0,
    };
    let record = store
        .backup_to_backend(image.clone(), &info)
        .expect("backup");
    let v = purse_udb::txstore::verify_backup_on_backend(image, Some("testnet3"))
        .unwrap_or_else(|e| panic!("{what}: verify_backup refuses it: {e}"));
    assert_eq!(v.record, Some(record), "{what}");
    store.close().expect("close");
}

/// ADR-0018 BK4: every state of the transaction tables that Go's
/// reproduced defects leave (docs/QUIRKS.md; the module documentation of
/// `purse_store::txtables` lists them) passes the consistency check. Each
/// is written by the store's operations, as a wallet would write it, on a
/// store of its own, with the rows that make it asserted, so that the
/// check is held to accept it whether or not a random sequence reaches it:
///
/// - QK-0781: a transaction mined in two blocks and rolled back from the
///   higher, then credited unmined: an outpoint both an unspent output and
///   an unmined credit, the hash both mined and unmined; and, credited at
///   both heights, two credits sharing one unspent row, which the rollback
///   deletes, leaving the lower credit live, naming no spender, with no
///   unspent row, after the unmined copy is removed too;
/// - QK-0100: a zero-value credit unspent by `Rollback` without its unspent
///   row;
/// - QK-0785: an unspent output whose credit is spent, by a ticket of its
///   block and by a regular transaction recorded in a block below;
/// - QK-0101: a debit naming a credit recorded above it that the rollback
///   of that block removed;
/// - QK-0095: a debit that `InsertMinedTx` of an already recorded
///   transaction writes before its duplicate test, its credit spent; and,
///   over QK-0781's second credit of the outpoint, a debit overwritten to
///   spend that second credit while the first still names the spender,
///   and after a rollback of the spender's block, the first credit naming
///   a spender with no debit; and, the first credit's block then
///   disapproved and approved again (QK-0785's mechanism), that credit
///   unspent while naming a spender with no debit, or, the spender
///   recorded below its funder, a spender whose live debit spends the
///   outpoint's credit at another height;
/// - QK-0060: an unmined transaction's input without its unmined-inputs
///   row, by `RemoveUnconfirmed` of a transaction that is not unmined and
///   by `Rollback` returning a spender over another's row; and a
///   commitment's unspent entry re-created by `RemoveUnconfirmed` of the
///   mined vote that redeemed it;
/// - QK-0087 and QK-0780: an unmined coinbase's null outpoint in the
///   unmined-inputs index;
/// - QK-0098: an unpublished mark of a hash with no unmined record;
/// - QK-0056: a ticket's row outliving every record of it;
/// - QK-0736: a disapproved block's regular transaction keeping its mined
///   record with its credit invalidated;
/// - QK-0786: a multisig record unspent while the unmined transaction that
///   `Rollback` returned still spends it;
/// - QK-0787: a multisig record outliving its removed transaction;
/// - QK-0097 and QK-0054: a mined transaction's multisig record reset
///   unmined by `AddMultisigOut`, and a stake-tagged output's record that
///   lost its stake-tree flag when spent;
/// - QK-0079 (with QK-0066): a mined credit without the expiry flag its
///   unmined credit had, classed by the flags it has.
///
/// QK-0789's state (a multisig record marked unmined by a rollback that did
/// not detach its block) is reached by the multisig variant of
/// `a_revalidated_transaction_merges_its_unmined_copy_keeping_the_row_invariants`,
/// whose every step runs the check. QK-0053 and QK-0055 (stale spender
/// bytes) are not representable in the store's rows; QK-0081, QK-0782
/// (invalidated rows of a detached block, deleted by dcr-purse: decided
/// divergence) and QK-0783 (a credit without its record, refused) cannot
/// occur; QK-0734's and QK-0735's states are fixed and left by no
/// operation.
#[test]
fn every_state_of_go_s_reproduced_defects_passes_the_check() {
    use purse_store::{DebitsInvalidated, MultisigOuts, Outpoint, Tickets, TxRecKey};
    let rec = |m: MsgTx| TxRecord::from_msg_tx(m, 1_795_400_000).expect("record");
    let hash_of = |b: &BlockHeader| Hash32(*b.block_hash().as_bytes());
    let credit = |ts: &TxStore, tx: &mut WriteTx<'_>, r: &TxRecord, b: Option<&BlockHeader>, i| {
        let meta = b.map(make_block_meta);
        ts.add_credit(tx, r, meta.as_ref(), i, false, 0)
            .expect("AddCredit");
    };
    let op = |r: &TxRecord, index: u32| Outpoint {
        hash: *r.hash(),
        index,
    };
    let ck = |r: &TxRecord, height: u32, index: u32| CreditKey {
        hash: *r.hash(),
        height,
        index,
    };
    let mined_at = |tx: &mut WriteTx<'_>, r: &TxRecord, height: u32| {
        tx.contains::<TxMined>(&TxRecKey {
            hash: *r.hash(),
            height,
        })
        .expect("contains")
    };
    let to_p2sh = |values: &[i64], tag: Option<u8>, fill: u8| {
        let ms = ms_scripts();
        rec(MsgTx {
            tx_in: vec![TxIn {
                previous_out_point: OutPoint {
                    hash: Hash([fill; 32]),
                    index: 0,
                    tree: 0,
                },
                sequence: u32::MAX,
                ..TxIn::default()
            }],
            tx_out: values
                .iter()
                .map(|v| TxOut {
                    value: *v,
                    version: 0,
                    pk_script: p2sh(tag, &ms[0]),
                })
                .collect(),
            ..MsgTx::default()
        })
    };
    let imported = 0x7fff_ffff;

    accepted(
        "QK-0781: an outpoint unspent and an unmined credit",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let t = rec(paying(&[(Hash([0x11; 32]), 0)], &[5000], 1));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &t, &hash_of(&b1)).expect("b1");
            credit(ts, tx, &t, Some(&b1), 0);
            ts.insert_mined_tx(tx, &t, &hash_of(&b2)).expect("b2");
            ts.rollback(tx, 2).expect("Rollback");
            credit(ts, tx, &t, None, 0);
            assert!(mined_at(tx, &t, 1));
            assert!(tx.contains::<TxUnmined>(t.hash()).expect("contains"));
            assert!(tx.contains::<Unspent>(&op(&t, 0)).expect("contains"));
            assert!(tx.contains::<UnminedCredits>(&op(&t, 0)).expect("contains"));
        },
    );

    accepted(
        "QK-0781: a live credit naming no spender without its unspent row",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let t = rec(paying(&[(Hash([0x11; 32]), 0)], &[5000], 1));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &t, &hash_of(&b1)).expect("b1");
            credit(ts, tx, &t, Some(&b1), 0);
            ts.insert_mined_tx(tx, &t, &hash_of(&b2)).expect("b2");
            credit(ts, tx, &t, Some(&b2), 0);
            assert!(tx.contains::<Credits>(&ck(&t, 1, 0)).expect("contains"));
            assert!(tx.contains::<Credits>(&ck(&t, 2, 0)).expect("contains"));
            let u = tx
                .get::<Unspent>(&op(&t, 0))
                .expect("get")
                .expect("one row");
            assert_eq!(u.height, 2, "one unspent row for the two credits");
            ts.rollback(tx, 2).expect("Rollback");
            ts.remove_unconfirmed(tx, t.msg_tx(), t.hash())
                .expect("RemoveUnconfirmed");
            let c = tx
                .get::<Credits>(&ck(&t, 1, 0))
                .expect("get")
                .expect("live");
            assert_eq!(c.spender, None);
            assert!(!tx.contains::<Unspent>(&op(&t, 0)).expect("contains"));
            assert!(!tx.contains::<TxUnmined>(t.hash()).expect("contains"));
        },
    );

    accepted(
        "QK-0100: a zero-value credit without its unspent row",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let z = rec(paying(&[(Hash([0x12; 32]), 0)], &[0, 500], 2));
            let zh = Hash(z.hash().0);
            let s = rec(paying(&[(zh, 0), (zh, 1)], &[400], 3));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &z, &hash_of(&b1)).expect("z");
            credit(ts, tx, &z, Some(&b1), 0);
            credit(ts, tx, &z, Some(&b1), 1);
            ts.insert_mined_tx(tx, &s, &hash_of(&b2)).expect("s");
            ts.rollback(tx, 2).expect("Rollback");
            let c = tx
                .get::<Credits>(&ck(&z, 1, 0))
                .expect("get")
                .expect("live");
            assert_eq!((c.amount, c.spender), (0, None));
            assert!(!tx.contains::<Unspent>(&op(&z, 0)).expect("contains"));
            assert!(tx.contains::<Unspent>(&op(&z, 1)).expect("contains"));
        },
    );

    accepted(
        "QK-0785: an unspent output a ticket of its block spends",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let mut g2 = BlockGenerator {
                last_hash: g.last_hash,
                last_height: g.last_height,
            };
            let (b2, b2x) = (g.generate(0), g2.generate(1));
            let (a, t) = funded_ticket();
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &a, &hash_of(&b1)).expect("a");
            credit(ts, tx, &a, Some(&b1), 0);
            ts.insert_mined_tx(tx, &t, &hash_of(&b1)).expect("t");
            credit(ts, tx, &t, Some(&b1), 0);
            insert_main_chain_headers(ts, tx, &[&b2]);
            ts.rollback(tx, 2).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b2x]);
            let c = tx
                .get::<Credits>(&ck(&a, 1, 0))
                .expect("get")
                .expect("live");
            assert_eq!(c.spender.map(|s| s.hash), Some(*t.hash()));
            assert!(tx.contains::<Unspent>(&op(&a, 0)).expect("contains"));
        },
    );

    accepted(
        "QK-0785: an unspent output a transaction below it spends",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2, b3) = (g.generate(1), g.generate(1), g.generate(1));
            let mut g2 = BlockGenerator {
                last_hash: g.last_hash,
                last_height: g.last_height,
            };
            let (b4, b4x) = (g.generate(0), g2.generate(1));
            let f = rec(paying(&[(Hash([0x13; 32]), 0)], &[1100], 4));
            let s = rec(paying(&[(Hash(f.hash().0), 0)], &[1000], 5));
            insert_main_chain_headers(ts, tx, &[&b1, &b2, &b3]);
            ts.insert_mined_tx(tx, &f, &hash_of(&b3)).expect("f");
            credit(ts, tx, &f, Some(&b3), 0);
            ts.insert_mined_tx(tx, &s, &hash_of(&b2)).expect("s");
            insert_main_chain_headers(ts, tx, &[&b4]);
            ts.rollback(tx, 4).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b4x]);
            let c = tx
                .get::<Credits>(&ck(&f, 3, 0))
                .expect("get")
                .expect("live");
            assert_eq!(c.spender.map(|s| (s.hash, s.height)), Some((*s.hash(), 2)));
            assert!(tx.contains::<Unspent>(&op(&f, 0)).expect("contains"));
        },
    );

    accepted(
        "QK-0101: a debit of a credit recorded above it, gone",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let y = rec(paying(&[(Hash([0x14; 32]), 0)], &[700], 6));
            let x = rec(paying(&[(Hash(y.hash().0), 0)], &[600], 7));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &y, &hash_of(&b2)).expect("y");
            credit(ts, tx, &y, Some(&b2), 0);
            ts.insert_mined_tx(tx, &x, &hash_of(&b1)).expect("x");
            ts.rollback(tx, 2).expect("Rollback");
            let d = tx
                .get::<Debits>(&ck(&x, 1, 0))
                .expect("get")
                .expect("debit");
            assert_eq!(d.credit, ck(&y, 2, 0));
            assert!(!tx.contains::<Credits>(&d.credit).expect("contains"));
            assert!(
                !tx.contains::<CreditsInvalidated>(&d.credit)
                    .expect("contains")
            );
        },
    );

    accepted(
        "QK-0095: a debit written by InsertMinedTx of a recorded transaction",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let a = rec(paying(&[(Hash([0x1c; 32]), 0)], &[900], 18));
            let x = rec(paying(&[(Hash(a.hash().0), 0)], &[800], 19));
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &x, &hash_of(&b1)).expect("x");
            assert!(!tx.contains::<Debits>(&ck(&x, 1, 0)).expect("contains"));
            ts.insert_mined_tx(tx, &a, &hash_of(&b1)).expect("a");
            credit(ts, tx, &a, Some(&b1), 0);
            ts.insert_mined_tx(tx, &x, &hash_of(&b1))
                .expect("x again, recorded");
            let d = tx
                .get::<Debits>(&ck(&x, 1, 0))
                .expect("get")
                .expect("debit");
            assert_eq!(d.credit, ck(&a, 1, 0));
            assert!(!tx.contains::<Unspent>(&op(&a, 0)).expect("contains"));
        },
    );

    // QK-0095 over QK-0781: the funder recorded and credited again in a
    // higher block (one unspent row, naming the higher credit), then the
    // spender inserted again: its debit, one slot per (transaction, height,
    // input), is overwritten to spend the higher credit, while the lower
    // credit still names the spender. The property seeds 454, 509 and 523
    // reach the first state.
    let funder_twice = |ts: &TxStore,
                        tx: &mut WriteTx<'_>,
                        b1: &BlockHeader,
                        b2: &BlockHeader,
                        spender_at: u32| {
        let f = rec(paying(&[(Hash([0x1d; 32]), 0)], &[900], 20));
        let s = rec(paying(&[(Hash(f.hash().0), 0)], &[800], 21));
        insert_main_chain_headers(ts, tx, &[b1, b2]);
        let sb = if spender_at == 1 { b1 } else { b2 };
        ts.insert_mined_tx(tx, &f, &hash_of(b1)).expect("f");
        credit(ts, tx, &f, Some(b1), 0);
        ts.insert_mined_tx(tx, &s, &hash_of(sb)).expect("s");
        ts.insert_mined_tx(tx, &f, &hash_of(b2)).expect("f again");
        credit(ts, tx, &f, Some(b2), 0);
        assert_eq!(
            tx.get::<Unspent>(&op(&f, 0))
                .expect("get")
                .map(|u| u.height),
            Some(2),
            "the higher credit is the unspent one"
        );
        ts.insert_mined_tx(tx, &s, &hash_of(sb))
            .expect("s again, recorded");
        let d = tx
            .get::<Debits>(&ck(&s, spender_at, 0))
            .expect("get")
            .expect("debit");
        assert_eq!(d.credit, ck(&f, 2, 0), "the debit spends the higher credit");
        let low = tx
            .get::<Credits>(&ck(&f, 1, 0))
            .expect("get")
            .expect("live");
        assert_eq!(
            low.spender.map(|p| (p.hash, p.height, p.input)),
            Some((*s.hash(), spender_at, 0))
        );
        (f, s)
    };
    accepted(
        "QK-0095 over QK-0781: a spent credit whose debit spends its outpoint's other credit",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            funder_twice(ts, tx, &b1, &b2, 1);
        },
    );
    accepted(
        "QK-0095 over QK-0781, rolled back: a spent credit naming a spender with no debit",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let (f, s) = funder_twice(ts, tx, &b1, &b2, 2);
            ts.rollback(tx, 2).expect("Rollback");
            let low = tx
                .get::<Credits>(&ck(&f, 1, 0))
                .expect("get")
                .expect("live");
            assert_eq!(
                low.spender.map(|p| (p.hash, p.height)),
                Some((*s.hash(), 2))
            );
            assert!(!tx.contains::<Debits>(&ck(&s, 2, 0)).expect("contains"));
            assert!(
                !tx.contains::<DebitsInvalidated>(&ck(&s, 2, 0))
                    .expect("contains")
            );
        },
    );

    // The rolled-back state above, then block 1 disapproved and approved
    // again: stake validation puts block 1's credit back with its spender
    // and its unspent row (QK-0785's mechanism, Go's `stakeValidate`), so
    // an unspent output's credit names a spender with no debit. The
    // property seeds 30021, 30197 and 30261 of a sweep biased toward
    // rollbacks and disapprovals reach it.
    let fork = |g: &BlockGenerator| BlockGenerator {
        last_hash: g.last_hash,
        last_height: g.last_height,
    };
    accepted(
        "QK-0095 over QK-0781, rolled back, the funder's block revalidated: an unspent \
         output whose credit names a spender with no debit",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let (mut gd, mut ga) = (fork(&g), fork(&g));
            let (b2, b2d, b2a) = (g.generate(1), gd.generate(0), ga.generate(1));
            let (f, s) = funder_twice(ts, tx, &b1, &b2, 2);
            ts.rollback(tx, 2).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b2d]);
            assert!(!tx.contains::<Unspent>(&op(&f, 0)).expect("contains"));
            ts.rollback(tx, 2).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b2a]);
            let low = tx
                .get::<Credits>(&ck(&f, 1, 0))
                .expect("get")
                .expect("live");
            assert_eq!(
                low.spender.map(|p| (p.hash, p.height)),
                Some((*s.hash(), 2))
            );
            assert_eq!(
                tx.get::<Unspent>(&op(&f, 0))
                    .expect("get")
                    .map(|u| u.height),
                Some(1)
            );
            assert!(!tx.contains::<Debits>(&ck(&s, 2, 0)).expect("contains"));
            assert!(
                !tx.contains::<DebitsInvalidated>(&ck(&s, 2, 0))
                    .expect("contains")
            );
        },
    );
    // The same over a spender recorded below its funder: `s` in block 1
    // spends `f`'s credit of block 2 and, inserted again, the credit of
    // block 3; block 3 rolled back leaves `s`'s debit naming that credit,
    // gone (QK-0101); block 2 disapproved and approved again puts its
    // credit back unspent, still naming `s`, whose live debit spends the
    // outpoint's credit at another height.
    accepted(
        "QK-0095 over QK-0781 below the funder, the funder's block revalidated: an \
         unspent output whose credit's spender debits the outpoint's other credit",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let (mut gd, mut ga) = (fork(&g), fork(&g));
            let (b3, b3d, b3a) = (g.generate(1), gd.generate(0), ga.generate(1));
            let f = rec(paying(&[(Hash([0x1e; 32]), 0)], &[900], 22));
            let s = rec(paying(&[(Hash(f.hash().0), 0)], &[800], 23));
            insert_main_chain_headers(ts, tx, &[&b1, &b2, &b3]);
            ts.insert_mined_tx(tx, &f, &hash_of(&b2)).expect("f");
            credit(ts, tx, &f, Some(&b2), 0);
            ts.insert_mined_tx(tx, &s, &hash_of(&b1)).expect("s");
            ts.insert_mined_tx(tx, &f, &hash_of(&b3)).expect("f again");
            credit(ts, tx, &f, Some(&b3), 0);
            ts.insert_mined_tx(tx, &s, &hash_of(&b1))
                .expect("s again, recorded");
            assert_eq!(
                tx.get::<Debits>(&ck(&s, 1, 0))
                    .expect("get")
                    .map(|d| d.credit),
                Some(ck(&f, 3, 0))
            );
            ts.rollback(tx, 3).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b3d]);
            ts.rollback(tx, 3).expect("Rollback");
            insert_main_chain_headers(ts, tx, &[&b3a]);
            let c = tx
                .get::<Credits>(&ck(&f, 2, 0))
                .expect("get")
                .expect("live");
            assert_eq!(c.spender.map(|p| (p.hash, p.height)), Some((*s.hash(), 1)));
            assert_eq!(
                tx.get::<Unspent>(&op(&f, 0))
                    .expect("get")
                    .map(|u| u.height),
                Some(2)
            );
            assert_eq!(
                tx.get::<Debits>(&ck(&s, 1, 0))
                    .expect("get")
                    .map(|d| d.credit),
                Some(ck(&f, 3, 0))
            );
        },
    );

    accepted(
        "QK-0060: an unmined input's row removed with another transaction",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let a = rec(paying(&[(Hash([0x15; 32]), 0)], &[900], 8));
            let ah = Hash(a.hash().0);
            let u = rec(paying(&[(ah, 0)], &[800], 9));
            let w = rec(paying(&[(ah, 0)], &[700], 10));
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &a, &hash_of(&b1)).expect("a");
            credit(ts, tx, &a, Some(&b1), 0);
            ts.insert_mem_pool_tx(tx, &u).expect("u");
            assert_eq!(
                tx.get::<UnminedInputs>(&op(&a, 0)).expect("get"),
                Some(*u.hash())
            );
            ts.remove_unconfirmed(tx, w.msg_tx(), w.hash())
                .expect("RemoveUnconfirmed of a transaction not recorded");
            assert!(tx.contains::<TxUnmined>(u.hash()).expect("contains"));
            assert!(!tx.contains::<UnminedInputs>(&op(&a, 0)).expect("contains"));
        },
    );

    accepted(
        "QK-0060: an unmined input's row taken by a returned spender",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let a = rec(paying(&[(Hash([0x16; 32]), 0)], &[900], 11));
            let ah = Hash(a.hash().0);
            let x = rec(paying(&[(ah, 0)], &[800], 12));
            let z = rec(paying(&[(ah, 0)], &[700], 13));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &a, &hash_of(&b1)).expect("a");
            credit(ts, tx, &a, Some(&b1), 0);
            ts.insert_mined_tx(tx, &x, &hash_of(&b2)).expect("x");
            ts.insert_mem_pool_tx(tx, &z)
                .expect("z, beside a mined spender");
            ts.rollback(tx, 2).expect("Rollback");
            assert!(tx.contains::<TxUnmined>(z.hash()).expect("contains"));
            assert!(tx.contains::<TxUnmined>(x.hash()).expect("contains"));
            assert_eq!(
                tx.get::<UnminedInputs>(&op(&a, 0)).expect("get"),
                Some(*x.hash())
            );
        },
    );

    accepted(
        "QK-0060: a commitment's unspent entry re-created after its redemption",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let (a, t) = funded_ticket();
            let mut sim = Sim {
                rng: SplitMix64(1),
                recs: Vec::new(),
                tickets: Vec::new(),
                blocks: Vec::new(),
                nonce: 0,
                plan: Vec::new(),
                vote: None,
            };
            let v = sim.build_vote(t.hash(), &hash_of(&b1), 1);
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &a, &hash_of(&b1)).expect("a");
            credit(ts, tx, &a, Some(&b1), 0);
            ts.insert_mined_tx(tx, &t, &hash_of(&b1)).expect("t");
            credit(ts, tx, &t, Some(&b1), 0);
            ts.add_ticket_commitment(tx, &t, 1, 0)
                .expect("AddTicketCommitment");
            insert_main_chain_headers(ts, tx, &[&b2]);
            ts.insert_mined_tx(tx, &v, &hash_of(&b2)).expect("v");
            ts.redeem_ticket_commitments(tx, &v, Some(&make_block_meta(&b2)))
                .expect("RedeemTicketCommitments");
            assert!(
                !tx.contains::<CommitmentsUnspent>(&op(&t, 1))
                    .expect("contains")
            );
            ts.remove_unconfirmed(tx, v.msg_tx(), v.hash())
                .expect("RemoveUnconfirmed of the mined vote");
            assert!(mined_at(tx, &v, 2));
            assert!(
                tx.contains::<CommitmentsUnspent>(&op(&t, 1))
                    .expect("contains")
            );
        },
    );

    accepted(
        "QK-0087, QK-0780: an unmined coinbase's null input",
        |ts, tx| {
            let cb = rec(MsgTx {
                tx_in: vec![TxIn {
                    previous_out_point: OutPoint {
                        hash: Hash::ZERO,
                        index: u32::MAX,
                        tree: 0,
                    },
                    sequence: u32::MAX,
                    signature_script: vec![0x00, 0x00],
                    ..TxIn::default()
                }],
                tx_out: vec![TxOut {
                    value: 5000,
                    version: 0,
                    pk_script: p2pkh(None, 14),
                }],
                ..MsgTx::default()
            });
            ts.insert_mem_pool_tx(tx, &cb).expect("InsertMemPoolTx");
            credit(ts, tx, &cb, None, 0);
            let null = Outpoint {
                hash: Hash32::default(),
                index: u32::MAX,
            };
            assert_eq!(
                tx.get::<UnminedInputs>(&null).expect("get"),
                Some(*cb.hash())
            );
        },
    );

    accepted(
        "QK-0098: a mark of a hash with no unmined record",
        |ts, tx| {
            let stray = Hash32([0x5a; 32]);
            ts.set_published(tx, &stray, false).expect("SetPublished");
            assert!(tx.contains::<Unpublished>(&stray).expect("contains"));
            assert!(!tx.contains::<TxUnmined>(&stray).expect("contains"));
        },
    );

    accepted("QK-0056: a ticket row outliving its records", |ts, tx| {
        let (_, t) = funded_ticket();
        ts.insert_mem_pool_tx(tx, &t).expect("InsertMemPoolTx");
        ts.remove_unconfirmed(tx, t.msg_tx(), t.hash())
            .expect("RemoveUnconfirmed");
        assert!(tx.contains::<Tickets>(t.hash()).expect("contains"));
        assert!(!tx.contains::<TxUnmined>(t.hash()).expect("contains"));
        assert!(tx.is_empty(TableId::TxMined).expect("is_empty"));
    });

    accepted(
        "QK-0736: a disapproved block's record with its credit invalidated",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(0));
            let r = rec(paying(&[(Hash([0x17; 32]), 0)], &[900], 15));
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &r, &hash_of(&b1)).expect("r");
            credit(ts, tx, &r, Some(&b1), 0);
            insert_main_chain_headers(ts, tx, &[&b2]);
            assert!(mined_at(tx, &r, 1));
            assert!(!tx.contains::<Credits>(&ck(&r, 1, 0)).expect("contains"));
            assert!(
                tx.contains::<CreditsInvalidated>(&ck(&r, 1, 0))
                    .expect("contains")
            );
            assert!(!tx.contains::<Unspent>(&op(&r, 0)).expect("contains"));
        },
    );

    accepted(
        "QK-0786: a multisig record unspent while a transaction spends it",
        |ts, tx| {
            import_ms_scripts(tx);
            let mut g = BlockGenerator::new();
            let (b1, b2) = (g.generate(1), g.generate(1));
            let f = to_p2sh(&[3000], None, 0x18);
            let s = rec(paying(&[(Hash(f.hash().0), 0)], &[2000], 16));
            insert_main_chain_headers(ts, tx, &[&b1, &b2]);
            ts.insert_mined_tx(tx, &f, &hash_of(&b1)).expect("f");
            ts.add_credit(tx, &f, Some(&make_block_meta(&b1)), 0, false, imported)
                .expect("AddCredit");
            ts.add_multisig_out(tx, &f, Some(&make_block_meta(&b1)), 0)
                .expect("AddMultisigOut");
            ts.insert_mined_tx(tx, &s, &hash_of(&b2)).expect("s");
            let fop = OutPoint {
                hash: Hash(f.hash().0),
                index: 0,
                tree: 0,
            };
            ts.spend_multisig_out(tx, &fop, s.hash(), 0)
                .expect("SpendMultisigOut");
            ts.rollback(tx, 2).expect("Rollback");
            let m = tx
                .get::<MultisigOuts>(&op(&f, 0))
                .expect("get")
                .expect("row");
            assert_eq!(m.spender, None);
            assert_eq!(
                tx.get::<UnminedInputs>(&op(&f, 0)).expect("get"),
                Some(*s.hash())
            );
        },
    );

    accepted(
        "QK-0787: a multisig record outliving its transaction",
        |ts, tx| {
            import_ms_scripts(tx);
            let u = to_p2sh(&[3000], None, 0x19);
            ts.insert_mem_pool_tx(tx, &u).expect("InsertMemPoolTx");
            ts.add_multisig_out(tx, &u, None, 0)
                .expect("AddMultisigOut");
            ts.remove_unconfirmed(tx, u.msg_tx(), u.hash())
                .expect("RemoveUnconfirmed");
            assert!(tx.contains::<MultisigOuts>(&op(&u, 0)).expect("contains"));
            assert!(!tx.contains::<TxUnmined>(u.hash()).expect("contains"));
        },
    );

    accepted(
        "QK-0097, QK-0054: a mined output's record reset unmined, its tree flag lost",
        |ts, tx| {
            import_ms_scripts(tx);
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let f = to_p2sh(&[3000, 4000], Some(0xba), 0x1a);
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &f, &hash_of(&b1)).expect("f");
            for i in 0..2 {
                ts.add_multisig_out(tx, &f, Some(&make_block_meta(&b1)), i)
                    .expect("AddMultisigOut");
            }
            let m = tx
                .get::<MultisigOuts>(&op(&f, 1))
                .expect("get")
                .expect("row");
            assert!(m.stake_tree && m.block.is_some());
            ts.add_multisig_out(tx, &f, None, 0)
                .expect("AddMultisigOut unmined over the mined record");
            let fop = OutPoint {
                hash: Hash(f.hash().0),
                index: 1,
                tree: 1,
            };
            ts.spend_multisig_out(tx, &fop, &Hash32([0x77; 32]), 0)
                .expect("SpendMultisigOut");
            assert!(mined_at(tx, &f, 1));
            let m = tx
                .get::<MultisigOuts>(&op(&f, 0))
                .expect("get")
                .expect("row");
            assert_eq!(m.block, None, "reset unmined (QK-0097)");
            let m = tx
                .get::<MultisigOuts>(&op(&f, 1))
                .expect("get")
                .expect("row");
            assert!(!m.stake_tree, "tree flag lost (QK-0054)");
        },
    );

    accepted(
        "QK-0079: a mined credit without its unmined credit's expiry flag",
        |ts, tx| {
            let mut g = BlockGenerator::new();
            let b1 = g.generate(1);
            let mut m = paying(&[(Hash([0x1b; 32]), 0)], &[900], 17);
            m.expiry = 500;
            let e = rec(m);
            ts.insert_mem_pool_tx(tx, &e).expect("InsertMemPoolTx");
            credit(ts, tx, &e, None, 0);
            let u = tx
                .get::<UnminedCredits>(&op(&e, 0))
                .expect("get")
                .expect("unmined credit");
            assert!(u.flags.expiry);
            insert_main_chain_headers(ts, tx, &[&b1]);
            ts.insert_mined_tx(tx, &e, &hash_of(&b1)).expect("e");
            let c = tx
                .get::<Credits>(&ck(&e, 1, 0))
                .expect("get")
                .expect("credit");
            assert!(!c.flags.expiry);
            let u = tx
                .get::<Unspent>(&op(&e, 0))
                .expect("get")
                .expect("unspent");
            assert_eq!(u.class, mined_class(&c.flags));
        },
    );
}
