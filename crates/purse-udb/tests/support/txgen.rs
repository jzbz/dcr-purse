// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Generated operation scripts for the transaction store's first slice
//! (`generate_txstore`): random chains (disapproving vote bits included),
//! regular, coinbase, ticket, vote and revocation transactions built by
//! `tx.build` over wallet and foreign outpoints (so chains of spends,
//! double spends and conflicting votes arise), inserted unmined, mined into
//! the tip or lower blocks, removed, pruned, published, rolled back to
//! every height including invalid ones, with reopens, clock moves, and
//! every query of the slice (headers, filters and the birthday included),
//! on all four networks.
//!
//! A script never faults: blocks are labelled only by extensions that
//! cannot be refused (a child of the tip with the default height), and
//! every transaction label is one `tx.build` made, which cannot fail.
//!
//! `generate_txstore_credits` draws the same operations and, with them,
//! the credit slice's ([`CREDIT_OPS`]): `AddCredit` (unmined, and mined in
//! an `update` group right after the `InsertMinedTx` that records the
//! transaction in that block, so the credit's block always holds the
//! record and is the main-chain block at its height: QK-0783, where Go
//! writes a credit dcr-purse refuses, is never reached), the UTXO queries,
//! `PreviousPkScripts`, balances and `UnspentTickets` (its sync height
//! drawn at or below the tip, at Go's `ticketMatured` boundary above a
//! height at or below the tip, or rarely at an `int32` extreme, QK-0784).
//! It avoids the sequences on which dcr-purse departs from Go, under a
//! rule or a decision (QK-0082, fixed under ADR-0012 Rule 7; QK-0081 with
//! QK-0782, ADR-0003 Decision item 5; QK-0101 and QK-0783, whose refusals
//! the project owner kept on 2026-11-24): a
//! transaction spending a built transaction's output is mined only into
//! the tip, and never while a transaction it spends may be in that same
//! block (the same-block spend chain whose disapproval Go records wrongly,
//! QK-0082, fixed; a spender mined below the transaction it spends, the
//! QK-0101 refusals); only a transaction spending no built output is
//! mined into lower blocks; and no detached block is connected again
//! (`extend.again` is drawn as a plain extension: Go's invalidated rows of
//! a detached block, which dcr-purse deletes, QK-0081, would come back
//! with it, QK-0782). The draws of `generate_txstore` are unchanged, so
//! its frozen vectors and recorded campaigns stand.
//!
//! `generate_txstore_cms` draws the credit scripts' operations and, with
//! them, the commitment and multisig slice's ([`CMS_OPS`]): the script
//! first imports two multisig scripts and `OP_TRUE` as scripts
//! (`mgr.import_script`, labels `ms1`, `ms2`, `s1`); transactions pay them
//! by P2SH (one under a stake tag); `cms.mined` is an `update` group of an
//! `InsertMinedTx` (under the credit scripts' mining rule) followed by the
//! `AddCredit`s, `AddMultisigOut`s, `AddTicketCommitment`s or
//! `RedeemTicketCommitments` the wallet's `processTransactionRecord` would
//! make for it in that block, `cms.unmined` the same without a block; and
//! the four writing operations and two queries are drawn alone too, at any
//! index, over any transaction, block or outpoint. Every block such an
//! operation names is a stored header's, never a meta of the zero hash, so
//! the one divergence of the slice (PARITY.md, `purse-udb` transaction
//! store, item (20)) is never reached. The draws of the other two
//! generators are unchanged.
//!
//! Every generator avoids, by construction, the sequence in which
//! dcr-purse fixes QK-0734 (ADR-0012 Rule 7; PARITY.md, `purse-udb`
//! transaction store, item (23)): a transaction is mined again only once
//! no record of an earlier `InsertMinedTx` of it can remain (each record's
//! height is bounded below by the tip's for a mining into the tip, by 1 for
//! a labelled block, and a valid `Rollback` to a height at or below that
//! bound removes it); until then the draw becomes an `InsertMemPoolTx`,
//! with the records of a mined group made unmined, so the draws are the
//! same. A transaction mined in a block that is later disapproved, mined
//! again in a later block that is rolled back, and then approved with the
//! first block, is mined and unmined at once in Go and mined only in
//! dcr-purse; the rule keeps a transaction from being mined twice at all
//! (QK-0781, which has its own oracle test, so no longer arises in
//! generated scripts). Found by the selection slice's campaigns
//! (`0x7a3e0301`, `0x7a3e0302`, credit group scripts 17 and others, a
//! transaction mined into the genesis block and again into a block
//! disapproving it); before the rule, the scripts of the earlier slices
//! did reach it.
//!
//! `generate_txstore_select` draws the credit scripts' operations and,
//! with them, input selection ([`SELECT_OPS`]): `RandomUTXO` and
//! `SelectInputs` (one to three increasing targets, now and then 0, -1 or
//! `i64::MAX`, an account other than 0 a quarter of the time, an ignore
//! list of one or two outpoints, a tree of 1 now and then, a quarter of
//! the time), each with its own `rand` key, which pins the stream on both
//! sides, or, for `select.again`, continuing the stream of the random
//! operation drawn just before it; `Spender` over credited and other
//! outpoints; and more mined credits. Every script ends with three
//! selections of every output. It avoids the credit scripts' divergences
//! by the same construction. The draws of the other generators are
//! unchanged.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

use std::collections::HashMap;

use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex};

/// The operations a generated transaction store script draws from (a name
/// listed twice is drawn twice as often).
pub const TX_OPS: &[&str] = &[
    "store.extend_main_chain",
    "store.extend_main_chain",
    "store.extend_main_chain",
    "extend.vote0",
    "extend.odd",
    "extend.again",
    "build.regular",
    "build.regular",
    "build.regular",
    "build.coinbase",
    "build.ticket",
    "build.ticket",
    "build.vote",
    "build.vote",
    "build.revocation",
    "tx.raw",
    "store.insert_mem_pool_tx",
    "store.insert_mem_pool_tx",
    "store.insert_mem_pool_tx",
    "store.insert_mem_pool_tx",
    "store.insert_mined_tx",
    "store.insert_mined_tx",
    "store.insert_mined_tx",
    "store.remove_unconfirmed",
    "store.set_published",
    "store.prune_unmined",
    "store.rollback",
    "store.tx_details",
    "store.unique_tx_details",
    "store.tx",
    "store.exists_tx",
    "store.exists_tx_mined_or_unmined",
    "store.tx_block_height",
    "store.unmined_txs",
    "store.unmined_tx_hashes",
    "store.range_transactions",
    "store.range_blocks",
    "store.iterate_tickets",
    "store.ticket_details",
    "store.own_ticket",
    "store.main_chain_tip",
    "store.block_in_main_chain",
    "store.get_block_meta_for_hash",
    "store.get_main_chain_block_hashes",
    "store.get_main_chain_cfilters",
    "store.get_main_chain_block_hash_for_height",
    "store.cfilter_v2",
    "store.update_processed_txs_block_marker",
    "store.processed_txs_block_marker",
    "udb.set_birth_state",
    "store.import_cfilters_v2",
    "store.missing_cfilters_height",
    "store.is_missing_main_chain_cfilters",
    "store.insert_missing_cfilters",
    "store.get_block_header",
    "store.get_serialized_block_header",
    "store.get_block_header_time",
    "udb.extract_block_header",
    "udb.birth_state",
    "store.unspent_output_count",
    "rt.reopen",
    "rt.clock_advance",
];

/// The operations a credit script ([`generate_txstore_credits`]) draws
/// besides [`TX_OPS`] (`extend.vote0` again, for more disapproved blocks):
/// `credit.mined` is an `update` group of an `InsertMinedTx` and
/// `AddCredit`s of the transaction in that block, `credit.unmined` an
/// `update` group of `AddCredit`s without a block (after an
/// `InsertMemPoolTx`, half the time).
pub const CREDIT_OPS: &[&str] = &[
    "credit.mined",
    "credit.mined",
    "credit.mined",
    "credit.mined",
    "credit.mined",
    "credit.unmined",
    "credit.unmined",
    "credit.unmined",
    "store.add_credit",
    "store.exists_utxo",
    "store.is_unspent_outpoint",
    "store.unspent_outputs",
    "store.unspent_output",
    "store.unspent_output",
    "store.for_each_unspent_outpoint",
    "store.previous_pk_scripts",
    "store.previous_pk_scripts",
    "store.account_balance",
    "store.account_balances",
    "store.unspent_tickets",
    "extend.vote0",
];

/// The operations a commitment and multisig script
/// ([`generate_txstore_cms`]) draws besides [`TX_OPS`] and [`CREDIT_OPS`].
pub const CMS_OPS: &[&str] = &[
    "build.p2sh",
    "build.p2sh",
    "build.p2sh",
    "build.p2sh",
    "build.ticket",
    "build.ticket",
    "build.vote",
    "build.vote",
    "build.revocation",
    "cms.mined",
    "cms.mined",
    "cms.mined",
    "cms.mined",
    "cms.mined",
    "cms.unmined",
    "cms.unmined",
    "cms.unmined",
    "store.add_ticket_commitment",
    "store.add_ticket_commitment",
    "store.redeem_ticket_commitments",
    "store.redeem_ticket_commitments",
    "store.add_multisig_out",
    "store.add_multisig_out",
    "store.add_multisig_out",
    "store.spend_multisig_out",
    "store.spend_multisig_out",
    "store.spend_multisig_out",
    "store.spend_multisig_out",
    "store.get_multisig_output",
    "store.get_multisig_output",
    "store.unspent_multisig_credits_for_address",
    "store.unspent_multisig_credits_for_address",
    "store.remove_unconfirmed",
    "store.rollback",
];

/// The operations a selection script ([`generate_txstore_select`]) draws
/// besides [`TX_OPS`] and [`CREDIT_OPS`]: the random selections, each with
/// its own `rand` key, `select.again` one continuing the stream of the
/// random operation just drawn (or a keyed one when the last operation is
/// not random), `Spender`, and more mined credits to select from.
pub const SELECT_OPS: &[&str] = &[
    "store.random_utxo",
    "store.random_utxo",
    "store.random_utxo",
    "store.select_inputs",
    "store.select_inputs",
    "store.select_inputs",
    "store.select_inputs",
    "select.again",
    "select.again",
    "store.spender",
    "store.spender",
    "store.spender",
    "credit.mined",
    "credit.mined",
    "credit.mined",
    "build.regular",
];

/// The scripts a commitment and multisig script imports: a 1-of-2 and a
/// 2-of-3 multisig script of fixed keys, and `OP_TRUE`.
fn cms_scripts() -> [(&'static str, String); 3] {
    let k1 = format!("02{}", "11".repeat(32));
    let k2 = format!("03{}", "22".repeat(32));
    let k3 = format!("02{}", "33".repeat(32));
    [
        ("ms1", format!("5121{k1}21{k2}52ae")),
        ("ms2", format!("5221{k1}21{k2}21{k3}53ae")),
        ("s1", "51".to_owned()),
    ]
}

/// The networks, by how often a script draws them.
const NETWORKS: [(&str, u64); 4] = [
    ("simnet", 70),
    ("regnet", 10),
    ("testnet3", 10),
    ("mainnet", 10),
];

/// Foreign transactions whose outputs the built transactions spend.
const FOREIGN: [&str; 4] = [
    "1111111111111111111111111111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333333333333333333333333333",
    "4444444444444444444444444444444444444444444444444444444444444444",
];

/// A built transaction the generator remembers.
struct Built {
    label: String,
    outputs: u32,
    ticket: bool,
    /// The labels of the built transactions it spends.
    prevs: Vec<String>,
}

fn pick<'a, T>(rng: &mut SplitMix64, v: &'a [T]) -> Option<&'a T> {
    if v.is_empty() {
        None
    } else {
        v.get(usize::try_from(rng.below(v.len() as u64)).expect("small"))
    }
}

/// The `TicketMaturity` of `network`.
fn ticket_maturity(network: &str) -> u64 {
    let params = match network {
        "mainnet" => dcroxide_chaincfg::mainnet_params(),
        "testnet3" => dcroxide_chaincfg::testnet3_params(),
        "regnet" => dcroxide_chaincfg::regnet_params(),
        _ => dcroxide_chaincfg::simnet_params(),
    };
    u64::from(params.ticket_maturity)
}

fn network(rng: &mut SplitMix64) -> &'static str {
    let total: u64 = NETWORKS.iter().map(|n| n.1).sum();
    let mut x = rng.below(total);
    for (name, weight) in NETWORKS {
        if x < weight {
            return name;
        }
        x -= weight;
    }
    "simnet"
}

struct Gen<'r> {
    rng: &'r mut SplitMix64,
    txs: Vec<Built>,
    blocks: Vec<String>,
    ops: Vec<Value>,
    costly: bool,
    reopens: usize,
    /// The tip height as far as the generator can tell (extensions it
    /// knows succeed, rollbacks it knows are valid).
    tip: u64,
    /// A credit script ([`generate_txstore_credits`]).
    credits: bool,
    /// The extensions known to have connected a block (credit scripts).
    ext: u64,
    /// For each transaction an `InsertMinedTx` was drawn for, `ext` at
    /// that draw: while it is unchanged the transaction may be in the tip.
    mined_at: HashMap<String, u64>,
    /// A rollback was drawn since the last extension known to connect a
    /// block: any transaction drawn mined may be in the tip.
    rolled_back: bool,
    /// The outputs an `AddCredit` was drawn for (credit scripts).
    credited: Vec<(String, u32)>,
    /// The network's `TicketMaturity`.
    ticket_maturity: u64,
    /// The P2SH outputs of each transaction built by `build.p2sh`
    /// (commitment and multisig scripts).
    p2sh: HashMap<String, Vec<u32>>,
    /// The outputs an `AddMultisigOut` was drawn for.
    multisig: Vec<(String, u32)>,
    /// A selection script ([`generate_txstore_select`]).
    select: bool,
    /// The built transactions' outputs that later built transactions
    /// spend, in the order drawn (`Spender`'s outpoints).
    spent: Vec<(String, u32)>,
    /// For each transaction an `InsertMinedTx` was drawn for, the lowest
    /// height a record of it may stand at (the tip's for the tip, 1 for a
    /// labelled block): while one may remain, it is not mined again
    /// (QK-0734, module documentation).
    mined_floor: HashMap<String, u64>,
}

impl Gen<'_> {
    fn below(&mut self, n: u64) -> u64 {
        self.rng.below(n)
    }

    fn addr(&mut self) -> Value {
        json!({"acct": 0, "branch": self.below(2), "index": self.below(12)})
    }

    fn block_ref(&mut self) -> Value {
        if self.below(4) == 0 || self.blocks.is_empty() {
            return json!("tip");
        }
        let b = pick(self.rng, &self.blocks).cloned().unwrap_or_default();
        json!(b)
    }

    fn tx_label(&mut self) -> Option<String> {
        pick(self.rng, &self.txs).map(|b| b.label.clone())
    }

    /// A hash argument: a transaction label, or a foreign hash.
    fn hash_ref(&mut self) -> Value {
        match self.tx_label() {
            Some(l) if self.below(6) != 0 => json!(l),
            _ => json!(FOREIGN[usize::try_from(self.below(4)).expect("small")]),
        }
    }

    /// An outpoint to spend: a built transaction's output (often), or a
    /// foreign one.
    fn prev(&mut self) -> Value {
        if self.below(3) != 0
            && let Some(b) = pick(self.rng, &self.txs)
        {
            let (label, n) = (b.label.clone(), b.outputs.max(1));
            return json!({"tx": label, "index": self.below(u64::from(n))});
        }
        let f = FOREIGN[usize::try_from(self.below(4)).expect("small")];
        json!({"tx": f, "index": self.below(4)})
    }

    fn new_label(&mut self, kind: &str) -> String {
        format!("{kind}{}", self.txs.len())
    }

    fn build_regular(&mut self) -> Value {
        let label = self.new_label("t");
        let n_in = 1 + self.below(3);
        let inputs: Vec<Value> = (0..n_in)
            .map(|_| json!({"prev": self.prev(), "value": 1000 + self.below(9000)}))
            .collect();
        let n_out = 1 + self.below(3);
        let mut outputs = Vec::new();
        for _ in 0..n_out {
            if self.below(5) == 0 {
                outputs.push(json!({"value": self.below(500), "script": "76a914000102030405060708090a0b0c0d0e0f1011121388ac"}));
            } else {
                outputs.push(json!({"value": 1 + self.below(5000), "pay": self.addr()}));
            }
        }
        let mut v = json!({"op": "tx.build", "label": label, "inputs": inputs, "outputs": outputs});
        if self.below(5) == 0 {
            v["expiry"] = json!(1 + self.below(8));
        }
        if self.below(6) == 0 {
            v["received"] = json!(1_795_300_000 + i64::try_from(self.below(1000)).expect("small"));
        }
        self.txs.push(Built {
            label,
            outputs: u32::try_from(n_out).expect("small"),
            ticket: false,
            prevs: Vec::new(),
        });
        v
    }

    fn build_coinbase(&mut self) -> Value {
        let label = self.new_label("cb");
        let v = json!({
            "op": "tx.build", "label": label,
            "inputs": [{"prev": "coinbase", "value": 50_000, "script": hex(&self.rng.bytes(4))}],
            "outputs": [{"value": 50_000, "pay": self.addr()}],
        });
        self.txs.push(Built {
            label,
            outputs: 1,
            ticket: false,
            prevs: Vec::new(),
        });
        v
    }

    fn build_ticket(&mut self) -> Value {
        let label = self.new_label("tk");
        let price = [1000, 2000][usize::try_from(self.below(2)).expect("small")];
        let v = json!({
            "op": "tx.build", "label": label,
            "inputs": [{"prev": self.prev(), "value": price + 100}],
            "outputs": [
                {"value": price, "pay": self.addr(), "tag": "sstx"},
                {"value": 0, "commitment": {"pay": self.addr(), "amount": price + 100}},
                {"value": 0, "pay": self.addr(), "tag": "sstxchange"},
            ],
        });
        self.txs.push(Built {
            label,
            outputs: 3,
            ticket: true,
            prevs: Vec::new(),
        });
        v
    }

    fn ticket_label(&mut self) -> Option<String> {
        let tickets: Vec<String> = self
            .txs
            .iter()
            .filter(|b| b.ticket)
            .map(|b| b.label.clone())
            .collect();
        pick(self.rng, &tickets).cloned()
    }

    fn build_vote(&mut self) -> Value {
        let Some(t) = self.ticket_label() else {
            return self.build_ticket();
        };
        let label = self.new_label("v");
        let block = self.block_ref();
        let v = json!({
            "op": "tx.build", "label": label,
            "inputs": [
                {"prev": "stakebase", "value": 300},
                {"prev": {"tx": t, "index": 0, "tree": 1}, "value": 1000},
            ],
            "outputs": [
                {"value": 0, "voteblock": block},
                {"value": 0, "votebits": "0100"},
                {"value": 1300, "pay": self.addr(), "tag": "ssgen"},
            ],
        });
        self.txs.push(Built {
            label,
            outputs: 3,
            ticket: false,
            prevs: Vec::new(),
        });
        v
    }

    fn build_revocation(&mut self) -> Value {
        let Some(t) = self.ticket_label() else {
            return self.build_ticket();
        };
        let label = self.new_label("r");
        let v = json!({
            "op": "tx.build", "label": label, "version": 2,
            "inputs": [{"prev": {"tx": t, "index": 0, "tree": 1}, "value": 1000}],
            "outputs": [{"value": 1000, "pay": self.addr(), "tag": "ssrtx"}],
        });
        self.txs.push(Built {
            label,
            outputs: 1,
            ticket: false,
            prevs: Vec::new(),
        });
        v
    }

    fn extend(&mut self, kind: &str) -> Value {
        match kind {
            "extend.odd" => {
                // Refused or not, never labelled.
                let mut v = json!({"op": "store.extend_main_chain"});
                match self.below(3) {
                    0 => v["parent"] = self.block_ref(),
                    1 => v["height"] = json!(self.below(12)),
                    _ => {
                        // A child of the tip: never refused.
                        v["time"] =
                            json!(1_795_300_000 + i64::try_from(self.below(9)).expect("small"));
                        self.tip += 1;
                        self.connected();
                    }
                }
                v
            }
            "extend.again" if !self.credits => {
                let b = self.block_ref();
                json!({"op": "store.extend_main_chain", "again": b})
            }
            _ => {
                let label = format!("b{}", self.blocks.len());
                let mut v = json!({"op": "store.extend_main_chain", "label": label});
                if kind == "extend.vote0" {
                    v["votebits"] = json!(0);
                } else if self.below(4) == 0 {
                    v["votebits"] = json!(1 + 2 * self.below(4));
                }
                let mut filter = Vec::new();
                for _ in 0..self.below(3) {
                    if let Some(l) = self.tx_label() {
                        filter.push(json!(l));
                    }
                }
                v["filter"] = json!(filter);
                self.blocks.push(label);
                self.tip += 1;
                self.connected();
                v
            }
        }
    }

    /// An extension known to connect a block.
    fn connected(&mut self) {
        self.ext += 1;
        self.rolled_back = false;
    }

    fn built(&self, label: &str) -> Option<&Built> {
        self.txs.iter().find(|b| b.label == label)
    }

    /// The block a credit script may mine `label` into, noting the draw,
    /// or `None`: a transaction spending no built output anywhere; one
    /// spending a built output only into the tip, and not while a
    /// transaction it spends may be in the tip (module documentation).
    /// Whether `label` may still have a mined record, so that mining it
    /// again could leave it mined and unmined at once (QK-0781), the state
    /// in which a later approval of the first block reaches QK-0734's fix.
    fn remined(&self, label: &str) -> bool {
        self.mined_floor.contains_key(label)
    }

    /// Records an `InsertMinedTx` of `label` into `block` (a label, or the
    /// tip).
    fn note_mined(&mut self, label: &str, block: &Value) {
        let floor = if block == "tip" { self.tip } else { 1 };
        let f = self.mined_floor.entry(label.to_owned()).or_insert(floor);
        *f = (*f).min(floor);
    }

    /// `ops` (an `InsertMinedTx` of `label` into `block` and the records of
    /// that block) as drawn, or, when `label` may still have a mined
    /// record, with the `InsertMinedTx` made an `InsertMemPoolTx` and the
    /// records unmined: the draws are the same either way.
    fn mined_group(&mut self, label: &str, block: &Value, mut ops: Vec<Value>) -> Value {
        if self.remined(label) {
            ops[0] = json!({"op": "store.insert_mem_pool_tx", "tx": label});
            for o in ops.iter_mut().skip(1) {
                if let Some(m) = o.as_object_mut() {
                    m.remove("block");
                }
            }
        } else {
            self.note_mined(label, block);
        }
        json!({"op": "update", "ops": ops})
    }

    fn mined_block(&mut self, label: &str) -> Option<Value> {
        let prevs = self
            .built(label)
            .map(|b| b.prevs.clone())
            .unwrap_or_default();
        let block = if prevs.is_empty() {
            self.block_ref()
        } else if prevs.iter().any(|p| {
            self.mined_at
                .get(p)
                .is_some_and(|e| *e == self.ext || self.rolled_back)
        }) {
            return None;
        } else {
            json!("tip")
        };
        self.mined_at.insert(label.to_owned(), self.ext);
        Some(block)
    }

    /// An outpoint argument: a built transaction's output (or one past its
    /// last), or a foreign one.
    fn outpoint(&mut self) -> Value {
        if self.below(2) == 0
            && let Some((label, index)) = pick(self.rng, &self.credited).cloned()
        {
            return json!({"tx": label, "index": index});
        }
        if self.below(4) != 0
            && let Some(b) = pick(self.rng, &self.txs)
        {
            let (label, n) = (b.label.clone(), b.outputs);
            return json!({"tx": label, "index": self.below(u64::from(n) + 1)});
        }
        let f = FOREIGN[usize::try_from(self.below(4)).expect("small")];
        json!({"tx": f, "index": self.below(4)})
    }

    /// A sync height for `UnspentTickets`: half the time one at or below
    /// the tip; else one exactly `TicketMaturity` or `TicketMaturity + 1`
    /// above a height at or below the tip, which for a ticket mined there
    /// is the last immature or the first mature height of Go's
    /// `ticketMatured`; rarely an `int32` extreme, where Go's subtraction
    /// wraps (QK-0784).
    fn sync_height(&mut self) -> i64 {
        match self.below(16) {
            0 => [
                i64::from(i32::MIN),
                i64::from(i32::MIN) + 1,
                i64::from(i32::MAX),
            ][usize::try_from(self.below(3)).expect("small")],
            1..=7 => {
                let h = self.below(self.tip + 1);
                i64::try_from(h + self.ticket_maturity + self.below(2)).expect("small")
            }
            _ => i64::try_from(self.below(self.tip + 4)).expect("small"),
        }
    }

    fn minconf(&mut self) -> i64 {
        [-1, 0, 1, 1, 2, 6][usize::try_from(self.below(6)).expect("small")]
    }

    /// The `AddCredit`s of `label`'s outputs (each with probability 2/3,
    /// at least one; rarely an index past the last), with `block`.
    fn credits_of(&mut self, label: &str, block: Option<&Value>) -> Vec<Value> {
        let n = self.built(label).map_or(1, |b| b.outputs);
        let mut out = Vec::new();
        for i in 0..n {
            if self.below(3) != 0 || (i + 1 == n && out.is_empty()) {
                out.push(self.credit(label, i, block));
            }
        }
        if self.below(12) == 0 {
            out.push(self.credit(label, n, block));
        }
        out
    }

    fn credit(&mut self, label: &str, index: u32, block: Option<&Value>) -> Value {
        let account = if self.below(4) == 0 { self.below(3) } else { 0 };
        let mut v =
            json!({"op": "store.add_credit", "tx": label, "index": index, "account": account});
        if self.below(3) == 0 {
            v["change"] = json!(true);
        }
        if let Some(b) = block {
            v["block"] = b.clone();
        }
        self.credited.push((label.to_owned(), index));
        v
    }

    /// A P2SH address reference of an imported script, or now and then a
    /// derived P2PKH address (refused by `UnspentMultisigCreditsForAddress`).
    fn ms_addr(&mut self) -> Value {
        match self.below(8) {
            0 => self.addr(),
            1 => json!({"ref": "s1"}),
            2..=4 => json!({"ref": "ms2"}),
            _ => json!({"ref": "ms1"}),
        }
    }

    fn build_p2sh(&mut self) -> Value {
        let label = self.new_label("p");
        let n_in = 1 + self.below(2);
        let inputs: Vec<Value> = (0..n_in)
            .map(|_| json!({"prev": self.prev(), "value": 1000 + self.below(9000)}))
            .collect();
        let n_out = 1 + self.below(4);
        let mut outputs = Vec::new();
        let mut p2sh = Vec::new();
        for i in 0..n_out {
            let value = 1 + self.below(5000);
            let i = u32::try_from(i).expect("small");
            match self.below(8) {
                0 => outputs.push(json!({"value": value, "pay": self.addr()})),
                1 => {
                    p2sh.push(i);
                    outputs
                        .push(json!({"value": value, "pay": {"ref": "ms1"}, "tag": "sstxchange"}));
                }
                _ => {
                    p2sh.push(i);
                    let a = self.ms_addr();
                    let a = if a.get("ref").is_some() {
                        a
                    } else {
                        json!({"ref": "ms1"})
                    };
                    outputs.push(json!({"value": value, "pay": a}));
                }
            }
        }
        self.p2sh.insert(label.clone(), p2sh);
        self.txs.push(Built {
            label: label.clone(),
            outputs: u32::try_from(n_out).expect("small"),
            ticket: false,
            prevs: Vec::new(),
        });
        json!({"op": "tx.build", "label": label, "inputs": inputs, "outputs": outputs})
    }

    /// A transaction label: one of `prefix`'s kind (by label) two times in
    /// three when there is one, else any.
    fn label_like(&mut self, prefixes: &[&str]) -> Option<String> {
        let like: Vec<String> = self
            .txs
            .iter()
            .filter(|b| prefixes.iter().any(|p| b.label.starts_with(p)))
            .map(|b| b.label.clone())
            .collect();
        if !like.is_empty() && self.below(3) != 0 {
            return pick(self.rng, &like).cloned();
        }
        self.tx_label()
    }

    fn add_multisig(&mut self, label: &str, index: u32, block: Option<&Value>) -> Value {
        let mut v = json!({"op": "store.add_multisig_out", "tx": label, "index": index});
        if let Some(b) = block {
            v["block"] = b.clone();
        }
        self.multisig.push((label.to_owned(), index));
        v
    }

    /// What `processTransactionRecord` records for `label` beyond the
    /// transaction (`block` given for a mined one): its credits, the
    /// multisig records of its P2SH outputs, a ticket's commitment, a vote's
    /// or revocation's redemption.
    fn records_of(&mut self, label: &str, block: Option<&Value>) -> Vec<Value> {
        let mut ops = self.credits_of(label, block);
        for i in self.p2sh.get(label).cloned().unwrap_or_default() {
            if self.below(4) != 0 {
                ops.push(self.add_multisig(label, i, block));
            }
        }
        if self.built(label).is_some_and(|b| b.ticket) {
            ops.push(json!({"op": "store.add_ticket_commitment", "tx": label, "index": 1, "account": self.below(3)}));
        }
        if label.starts_with('v') || label.starts_with('r') {
            let mut v = json!({"op": "store.redeem_ticket_commitments", "tx": label});
            if let Some(b) = block {
                v["block"] = b.clone();
            }
            ops.push(v);
        }
        ops
    }

    /// A multisig outpoint argument: one an `AddMultisigOut` was drawn for
    /// (often), or any outpoint.
    fn ms_outpoint(&mut self) -> Value {
        if self.below(4) != 0
            && let Some((label, index)) = pick(self.rng, &self.multisig).cloned()
        {
            return json!({"tx": label, "index": index});
        }
        self.outpoint()
    }

    /// An operation of [`CMS_OPS`].
    fn cms_op(&mut self, name: &str) -> Value {
        match name {
            "build.p2sh" => self.build_p2sh(),
            "cms.mined" => {
                let Some(l) = self.label_like(&["p", "tk", "v", "r"]) else {
                    return self.build_p2sh();
                };
                let Some(block) = self.mined_block(&l) else {
                    return json!({"op": "store.insert_mem_pool_tx", "tx": l});
                };
                let mut ops = vec![json!({"op": "store.insert_mined_tx", "tx": l, "block": block})];
                ops.extend(self.records_of(&l, Some(&block)));
                self.mined_group(&l, &block, ops)
            }
            "cms.unmined" => {
                let Some(l) = self.label_like(&["p", "tk", "v", "r"]) else {
                    return self.build_p2sh();
                };
                let mut ops = Vec::new();
                if self.below(4) != 0 {
                    ops.push(json!({"op": "store.insert_mem_pool_tx", "tx": l}));
                }
                ops.extend(self.records_of(&l, None));
                json!({"op": "update", "ops": ops})
            }
            "store.add_ticket_commitment" => match self.label_like(&["tk"]) {
                Some(l) => {
                    let n = self.built(&l).map_or(1, |b| b.outputs);
                    let index = if self.below(2) == 0 {
                        1
                    } else {
                        self.below(u64::from(n) + 1)
                    };
                    json!({"op": name, "tx": l, "index": index, "account": self.below(3)})
                }
                None => self.build_ticket(),
            },
            "store.redeem_ticket_commitments" => match self.label_like(&["v", "r"]) {
                Some(l) => {
                    let mut v = json!({"op": name, "tx": l});
                    if self.below(2) == 0 {
                        v["block"] = self.block_ref();
                    }
                    v
                }
                None => self.build_vote(),
            },
            "store.add_multisig_out" => match self.label_like(&["p"]) {
                Some(l) => {
                    let n = self.built(&l).map_or(1, |b| b.outputs);
                    let index = u32::try_from(self.below(u64::from(n) + 1)).expect("small");
                    let block = (self.below(2) == 0).then(|| self.block_ref());
                    self.add_multisig(&l, index, block.as_ref())
                }
                None => self.build_p2sh(),
            },
            "store.spend_multisig_out" => {
                let op = self.ms_outpoint();
                let spender = self.hash_ref();
                json!({"op": name, "outpoint": op, "spender": spender, "index": self.below(2)})
            }
            "store.get_multisig_output" => {
                let op = self.ms_outpoint();
                json!({"op": name, "outpoint": op})
            }
            "store.unspent_multisig_credits_for_address" => {
                let a = self.ms_addr();
                json!({"op": name, "addr": a})
            }
            // The others are the first slice's, drawn more often here.
            other => self.op(other),
        }
    }

    /// A random store operation with a fresh `rand` key.
    fn random_op(&mut self, name: &str) -> Value {
        let mut v = if name == "store.random_utxo" {
            let m = self.minconf();
            let sh = self.sync_height();
            json!({"op": name, "minconf": m, "syncheight": sh})
        } else {
            let m = self.minconf();
            let sh = self.sync_height();
            let n = 1 + self.below(3);
            let mut t = 0i64;
            let mut targets = Vec::new();
            for _ in 0..n {
                match self.below(10) {
                    0 => targets.push(json!(0)),
                    1 => targets.push(json!(-1)),
                    2 => targets.push(json!(i64::MAX)),
                    _ => {
                        t += i64::try_from(1 + self.below(9000)).expect("small");
                        targets.push(json!(t));
                    }
                }
            }
            let acct = if self.below(4) == 0 { self.below(3) } else { 0 };
            let mut v = json!({"op": name, "account": acct, "minconf": m, "syncheight": sh, "targets": targets});
            if self.below(4) == 0 {
                let k = 1 + self.below(2);
                let ignore: Vec<Value> = (0..k)
                    .map(|_| {
                        let mut op = self.outpoint();
                        if self.below(3) == 0 {
                            op["tree"] = json!(1);
                        }
                        op
                    })
                    .collect();
                v["ignore"] = json!(ignore);
            }
            v
        };
        v["rand"] = json!(hex(&self.rng.bytes(32)));
        v
    }

    /// An operation of [`SELECT_OPS`].
    fn select_op(&mut self, name: &str) -> Value {
        match name {
            "store.random_utxo" | "store.select_inputs" => self.random_op(name),
            "select.again" => {
                let last = self.ops.last().and_then(|v| v["op"].as_str()).unwrap_or("");
                if last != "store.random_utxo" && last != "store.select_inputs" {
                    return self.random_op("store.random_utxo");
                }
                let kind = if self.below(2) == 0 {
                    "store.random_utxo"
                } else {
                    "store.select_inputs"
                };
                let mut v = self.random_op(kind);
                if let Some(m) = v.as_object_mut() {
                    m.remove("rand");
                }
                v
            }
            "store.spender" => {
                // An output a built transaction spends, credited when one
                // is, two times in three.
                let credited: Vec<(String, u32)> = self
                    .spent
                    .iter()
                    .filter(|o| self.credited.contains(o))
                    .cloned()
                    .collect();
                let from = if credited.is_empty() {
                    self.spent.clone()
                } else {
                    credited
                };
                if self.below(3) != 0
                    && let Some((label, index)) = pick(self.rng, &from).cloned()
                {
                    return json!({"op": name, "outpoint": {"tx": label, "index": index}});
                }
                let op = self.outpoint();
                json!({"op": name, "outpoint": op})
            }
            other => self.op(other),
        }
    }

    /// An operation of [`CREDIT_OPS`].
    fn credit_op(&mut self, name: &str) -> Value {
        match name {
            "credit.mined" => {
                let Some(l) = self.tx_label() else {
                    return self.build_regular();
                };
                let Some(block) = self.mined_block(&l) else {
                    return json!({"op": "store.insert_mem_pool_tx", "tx": l});
                };
                let mut ops = vec![json!({"op": "store.insert_mined_tx", "tx": l, "block": block})];
                ops.extend(self.credits_of(&l, Some(&block)));
                self.mined_group(&l, &block, ops)
            }
            "credit.unmined" => {
                let Some(l) = self.tx_label() else {
                    return self.build_regular();
                };
                let mut ops = Vec::new();
                if self.below(2) == 0 {
                    ops.push(json!({"op": "store.insert_mem_pool_tx", "tx": l}));
                }
                ops.extend(self.credits_of(&l, None));
                json!({"op": "update", "ops": ops})
            }
            "store.add_credit" => match self.tx_label() {
                Some(l) => {
                    let n = self.built(&l).map_or(1, |b| b.outputs);
                    let i = u32::try_from(self.below(u64::from(n))).expect("small");
                    self.credit(&l, i, None)
                }
                None => self.build_regular(),
            },
            "store.exists_utxo" | "store.is_unspent_outpoint" => {
                let op = self.outpoint();
                json!({"op": name, "outpoint": op})
            }
            "store.unspent_output" => {
                let op = self.outpoint();
                json!({"op": name, "outpoint": op, "include_mempool": self.below(2) == 0})
            }
            "store.previous_pk_scripts" => match self.tx_label() {
                Some(l) => {
                    let mut v = json!({"op": name, "tx": l});
                    if self.below(4) != 0 {
                        v["block"] = self.block_ref();
                    }
                    v
                }
                None => self.build_regular(),
            },
            "store.account_balance" => {
                let m = self.minconf();
                json!({"op": name, "minconf": m, "account": self.below(3)})
            }
            "store.account_balances" => {
                let m = self.minconf();
                json!({"op": name, "minconf": m})
            }
            "store.unspent_tickets" => {
                let sh = self.sync_height();
                json!({"op": name, "syncheight": sh, "include_immature": self.below(2) == 0})
            }
            other => json!({"op": other}),
        }
    }

    fn op(&mut self, name: &str) -> Value {
        if self.credits && !TX_OPS.contains(&name) && CREDIT_OPS.contains(&name) {
            return self.credit_op(name);
        }
        if self.select && !TX_OPS.contains(&name) && SELECT_OPS.contains(&name) {
            return self.select_op(name);
        }
        if self.credits && !TX_OPS.contains(&name) {
            return self.cms_op(name);
        }
        match name {
            "store.extend_main_chain" | "extend.vote0" | "extend.odd" | "extend.again" => {
                self.extend(name)
            }
            "build.regular" => self.build_regular(),
            "build.coinbase" => self.build_coinbase(),
            "build.ticket" => self.build_ticket(),
            "build.vote" => self.build_vote(),
            "build.revocation" => self.build_revocation(),
            "tx.raw" => {
                // A transaction with trailing bytes: the store keeps them,
                // the readers drop them.
                let label = self.new_label("raw");
                let f = FOREIGN[usize::try_from(self.below(4)).expect("small")];
                let idx = format!("{:02x}", self.below(200) + 10);
                let tx = format!(
                    "0100000001{f}{idx}00000000ffffffff0101000000000000000000016a000000000000000001020000000000000000000000ffffffff00{}",
                    if self.below(2) == 0 { "" } else { "beef" }
                );
                self.txs.push(Built {
                    label: label.clone(),
                    outputs: 1,
                    ticket: false,
                    prevs: Vec::new(),
                });
                json!({"op": "tx.raw", "label": label, "tx": tx})
            }
            "store.insert_mem_pool_tx" | "store.remove_unconfirmed" => match self.tx_label() {
                Some(l) => json!({"op": name, "tx": l}),
                None => self.build_regular(),
            },
            "store.insert_mined_tx" => match self.tx_label() {
                Some(l) if self.credits => match self.mined_block(&l) {
                    Some(b) if !self.remined(&l) => {
                        self.note_mined(&l, &b);
                        json!({"op": name, "tx": l, "block": b})
                    }
                    _ => json!({"op": "store.insert_mem_pool_tx", "tx": l}),
                },
                Some(l) => {
                    let b = self.block_ref();
                    if self.remined(&l) {
                        json!({"op": "store.insert_mem_pool_tx", "tx": l})
                    } else {
                        self.note_mined(&l, &b);
                        json!({"op": name, "tx": l, "block": b})
                    }
                }
                None => self.build_regular(),
            },
            "store.set_published" => {
                let h = self.hash_ref();
                json!({"op": name, "tx": h, "published": self.below(2) == 0})
            }
            "store.prune_unmined" => {
                let sdiff = [0, 1000, 2000][usize::try_from(self.below(3)).expect("small")];
                json!({"op": name, "sdiff": sdiff})
            }
            "store.rollback" => {
                // Mostly a valid height (1 to one above the tip); sometimes
                // 0, -1 or two or more above the tip (Go: Invalid or panic).
                self.rolled_back = true;
                let tip = i64::try_from(self.tip).expect("small");
                let h = if self.below(5) == 0 || self.tip == 0 {
                    [0, -1, tip + 2, tip + 5][usize::try_from(self.below(4)).expect("small")]
                } else {
                    let h = 1 + self.below(self.tip + 1);
                    self.tip = h - 1;
                    // Every record at or above h is gone.
                    self.mined_floor.retain(|_, f| *f < h);
                    i64::try_from(h).expect("small")
                };
                json!({"op": name, "height": h})
            }
            "store.tx_details"
            | "store.tx"
            | "store.exists_tx"
            | "store.exists_tx_mined_or_unmined"
            | "store.tx_block_height"
            | "store.ticket_details" => {
                let h = self.hash_ref();
                json!({"op": name, "tx": h})
            }
            "store.own_ticket" => {
                let h = self.hash_ref();
                json!({"op": name, "ticket": h})
            }
            "store.unique_tx_details" => {
                let h = self.hash_ref();
                let mut v = json!({"op": name, "tx": h});
                if self.below(3) != 0 {
                    v["block"] = self.block_ref();
                }
                v
            }
            "store.range_transactions" | "store.range_blocks" => {
                let b = i64::try_from(self.below(10)).expect("small") - 2;
                let e = i64::try_from(self.below(10)).expect("small") - 2;
                let mut v = json!({"op": name, "begin": b, "end": e});
                if self.below(3) == 0 {
                    v["stop_after"] = json!(self.below(3));
                }
                v
            }
            "store.block_in_main_chain"
            | "store.get_block_meta_for_hash"
            | "store.cfilter_v2"
            | "store.get_block_header"
            | "store.get_serialized_block_header"
            | "store.get_block_header_time"
            | "udb.extract_block_header" => {
                let b = self.block_ref();
                json!({"op": name, "block": b})
            }
            "store.update_processed_txs_block_marker" => {
                let b = self.block_ref();
                json!({"op": name, "block": b})
            }
            "store.get_main_chain_block_hashes" | "store.get_main_chain_cfilters" => {
                let b = self.block_ref();
                json!({"op": name, "start": b, "inclusive": self.below(2) == 0, "count": self.below(5)})
            }
            "store.get_main_chain_block_hash_for_height" => {
                json!({"op": name, "height": i64::try_from(self.below(12)).expect("small") - 1})
            }
            "udb.set_birth_state" => {
                let mut v = json!({"op": name, "height": self.below(4), "time": 1_795_300_000});
                if self.below(2) == 0 {
                    v["block"] = self.block_ref();
                }
                v["from_height"] = json!(self.below(3) == 0);
                v["from_time"] = json!(self.below(3) == 0);
                v
            }
            "store.insert_missing_cfilters" => {
                // Refused by both: have_cfilters is always set.
                let n = self.below(3);
                let blocks: Vec<Value> = (0..n).map(|_| self.block_ref()).collect();
                json!({"op": name, "blocks": blocks})
            }
            "store.import_cfilters_v2" => {
                let n = self.below(3);
                let filters: Vec<Value> = (0..n).map(|_| json!("00")).collect();
                json!({"op": name, "start_height": i64::try_from(self.below(8)).expect("small") - 1, "filters": filters})
            }
            "rt.reopen" => {
                if self.costly && self.reopens >= 1 {
                    return json!({"op": "store.main_chain_tip"});
                }
                self.reopens += 1;
                json!({"op": name})
            }
            "rt.clock_advance" => json!({"op": name, "seconds": self.below(600)}),
            other => json!({"op": other}),
        }
    }
}

/// A generated transaction store script.
pub fn generate_txstore(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    generate(rng, id, network_override, false, false)
}

/// A generated transaction store script drawing the credit slice's
/// operations too (module documentation).
pub fn generate_txstore_credits(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    generate(rng, id, network_override, true, false)
}

/// A generated transaction store script drawing the credit slice's and the
/// commitment and multisig slice's operations too (module documentation).
pub fn generate_txstore_cms(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    generate(rng, id, network_override, true, true)
}

/// A generated transaction store script drawing the credit slice's
/// operations and input selection, `RandomUTXO` and `Spender` (module
/// documentation).
pub fn generate_txstore_select(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    generate_kind(rng, id, network_override, true, false, true)
}

fn generate(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
    credits: bool,
    cms: bool,
) -> Value {
    generate_kind(rng, id, network_override, credits, cms, false)
}

fn generate_kind(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
    credits: bool,
    cms: bool,
    select: bool,
) -> Value {
    let network = network_override.unwrap_or_else(|| network(rng));
    let seed = rng.bytes(32);
    let digest = rng.below(4) == 0;
    let n = 20 + rng.below(50);
    let mut g = Gen {
        rng,
        txs: Vec::new(),
        blocks: Vec::new(),
        ops: Vec::new(),
        costly: network != "simnet",
        reopens: 0,
        tip: 0,
        credits,
        ext: 0,
        mined_at: HashMap::new(),
        rolled_back: false,
        credited: Vec::new(),
        ticket_maturity: ticket_maturity(network),
        p2sh: HashMap::new(),
        multisig: Vec::new(),
        select,
        spent: Vec::new(),
        mined_floor: HashMap::new(),
    };
    if cms {
        for (label, script) in cms_scripts() {
            g.ops
                .push(json!({"op": "mgr.import_script", "script": script, "label": label}));
        }
    }
    let mut lists: Vec<&[&str]> = vec![TX_OPS];
    if credits {
        lists.push(CREDIT_OPS);
    }
    if cms {
        lists.push(CMS_OPS);
    }
    if select {
        lists.push(SELECT_OPS);
    }
    let draws: usize = lists.iter().map(|l| l.len()).sum();
    for _ in 0..n {
        let mut i = usize::try_from(g.below(draws as u64)).expect("small");
        let mut name = "";
        for l in &lists {
            if i < l.len() {
                name = l[i];
                break;
            }
            i -= l.len();
        }
        let v = g.op(name);
        if v["op"] == "tx.build" {
            for i in v["inputs"].as_array().into_iter().flatten() {
                if let (Some(t), Some(x)) = (i["prev"]["tx"].as_str(), i["prev"]["index"].as_u64())
                    && t.len() != 64
                {
                    g.spent
                        .push((t.to_owned(), u32::try_from(x).expect("small")));
                }
            }
        }
        if v["op"] == "tx.build"
            && let Some(last) = g.txs.last_mut()
        {
            last.prevs = v["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["prev"]["tx"].as_str())
                .filter(|t| t.len() != 64)
                .map(str::to_owned)
                .collect();
        }
        g.ops.push(v);
    }
    // Every script ends with the queries over everything it recorded.
    for q in [
        "store.unmined_txs",
        "store.iterate_tickets",
        "store.main_chain_tip",
    ] {
        g.ops.push(json!({"op": q}));
    }
    g.ops
        .push(json!({"op": "store.range_transactions", "begin": 0, "end": -1}));
    if credits {
        for v in [
            json!({"op": "store.unspent_outputs"}),
            json!({"op": "store.for_each_unspent_outpoint"}),
            json!({"op": "store.account_balances", "minconf": 0}),
            json!({"op": "store.account_balances", "minconf": 1}),
            json!({"op": "store.unspent_tickets", "syncheight": g.tip, "include_immature": true}),
        ] {
            g.ops.push(v);
        }
    }
    if select {
        for (minconf, account) in [(0, 0), (1, 0), (1, 1)] {
            let rand = hex(&g.rng.bytes(32));
            g.ops.push(
                json!({"op": "store.select_inputs", "account": account, "minconf": minconf,
                "syncheight": g.tip, "targets": [0], "rand": rand}),
            );
        }
    }
    if cms {
        for a in ["ms1", "ms2"] {
            g.ops.push(
                json!({"op": "store.unspent_multisig_credits_for_address", "addr": {"ref": a}}),
            );
        }
        let recorded = g.multisig.clone();
        for (label, index) in recorded.iter().take(6) {
            g.ops.push(json!({"op": "store.get_multisig_output", "outpoint": {"tx": label, "index": index}}));
        }
    }
    let rand = hex(&g.rng.bytes(32));
    let mut script = json!({
        "format": "purse-opscript 1",
        "name": format!(
            "txstore {}generated {id}",
            match (credits, cms, select) {
                (_, _, true) => "selection ",
                (_, true, false) => "commitment and multisig ",
                (true, false, false) => "credit ",
                (false, false, false) => "",
            }
        ),
        "setup": {
            "network": network,
            "seed": hex(&seed),
            "pubpass": "public",
            "privpass": "private",
            "rand": rand,
            "clock": 1_795_300_000,
        },
        "ops": g.ops,
    });
    if digest {
        script["export"] = json!({"chain": "digest"});
    }
    script
}

/// The operations a generated script runs in a group only as a member of
/// an `update` group (Go's `writeTx` kind).
const CHANGING: &[&str] = &[
    "store.extend_main_chain",
    "store.update_processed_txs_block_marker",
    "udb.set_birth_state",
    "store.insert_missing_cfilters",
    "store.import_cfilters_v2",
    "store.rollback",
    "store.insert_mined_tx",
    "store.insert_mem_pool_tx",
    "store.set_published",
    "store.remove_unconfirmed",
    "store.prune_unmined",
    "store.add_credit",
    "store.add_ticket_commitment",
    "store.redeem_ticket_commitments",
    "store.add_multisig_out",
    "store.spend_multisig_out",
];

/// The kinds a generated group member tolerates: never `Invalid` or `IO`,
/// which dcr-purse returns where Go panics (PARITY.md, `purse-udb`
/// transaction store, item (2)), and a panic is never tolerated.
const TOLERABLE: [&str; 3] = ["Exist", "DoubleSpend", "NotExist"];

/// The operations that may write Go's `unspent` bucket in a script
/// (`putUnspent`, `putRawUnspent` and `deleteRawUnspent` are reached from
/// `ExtendMainChain` and `Rollback` through stake validation and
/// invalidation, from `InsertMinedTx` through `moveMinedTx` and its spends,
/// from `AddCredit` with a block, and from `Rollback`; txmined.go at the
/// pin).
pub const UNSPENT_WRITERS: &[&str] = &[
    "store.extend_main_chain",
    "store.insert_mined_tx",
    "store.add_credit",
    "store.rollback",
];

/// The operations whose answer depends on the number of entries of the
/// `unspent` bucket, which Go reads with bbolt's `KeyN`, a count of the
/// committed pages that leaves out the transaction's own writes (QK-0003):
/// `UnspentOutputCount`, and `MakeInputSource`, whose count bounds the
/// random phase's tries and so the bytes drawn.
pub const UNSPENT_COUNTERS: &[&str] = &["store.unspent_output_count", "store.select_inputs"];

/// Whether `op` (a top-level operation) is `AddCredit` or a group holding
/// one.
fn credits(op: &Value) -> bool {
    op["op"] == "store.add_credit"
        || op["ops"]
            .as_array()
            .is_some_and(|m| m.iter().any(|x| x["op"] == "store.add_credit"))
}

/// The places in `script` where an `update` group counts the `unspent`
/// bucket after a member that may write it ([`UNSPENT_WRITERS`],
/// [`UNSPENT_COUNTERS`]), once an `AddCredit` has come before the counting
/// member (until then the bucket is empty on both sides, and no member
/// writes it): `(index of the group, index of the counting member)`. Go
/// answers such a count from the bucket as the group's transaction began,
/// the Rust runner, and dcr-purse's store, with the group's own writes
/// counted (QK-0003; PARITY.md, `purse-udb` transaction store, item (26)).
pub fn counts_after_unspent_writes(script: &Value) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut credited = false;
    for (i, op) in script["ops"].as_array().into_iter().flatten().enumerate() {
        if op["op"] == "update" {
            let mut wrote = false;
            for (j, m) in op["ops"].as_array().into_iter().flatten().enumerate() {
                let name = m["op"].as_str().unwrap_or("");
                if wrote && credited && UNSPENT_COUNTERS.contains(&name) {
                    out.push((i, j));
                }
                wrote |= UNSPENT_WRITERS.contains(&name);
                credited |= name == "store.add_credit";
            }
        }
        credited |= credits(op);
    }
    out
}

/// `generate_txstore`'s script with runs of its operations put in `update`
/// and `view` groups (ADR-0002 section 2), some members tolerating kinds.
/// A group never holds what registers a label (a labelled extension, a
/// built transaction), since a group that ends early or rolls back would
/// leave a later reference to it unresolved, nor a runtime operation.
///
/// Nor does an `update` group count the `unspent` bucket after a member
/// that may write it once the script has credited an output
/// ([`counts_after_unspent_writes`]): such a run is cut into consecutive
/// `update` groups before each counting member, with the draws unchanged,
/// so a script that never meets the sequence is the one drawn before the
/// rule (PARITY.md, `purse-udb` transaction store, item (26); crate
/// `txstore`, choice 15; pinned by
/// `a_count_after_an_unspent_write_in_a_group_is_gos_committed_count`).
pub fn group_txstore(rng: &mut SplitMix64, mut script: Value) -> Value {
    let ops = script["ops"].as_array().cloned().unwrap_or_default();
    let groupable = |v: &Value| {
        let name = v["op"].as_str().unwrap_or("");
        (name.starts_with("store.") && v.get("label").is_none()) || name.starts_with("udb.")
    };
    // The index of the first operation that is or holds an `AddCredit`.
    let first_credit = ops.iter().position(credits).unwrap_or(usize::MAX);
    let mut out = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < ops.len() {
        if !groupable(&ops[i]) || rng.below(3) != 0 {
            out.push(ops[i].clone());
            i += 1;
            continue;
        }
        let want = 2 + usize::try_from(rng.below(4)).expect("small");
        let start = i;
        let mut members = Vec::new();
        while i < ops.len() && members.len() < want && groupable(&ops[i]) {
            let mut m = ops[i].clone();
            // Go writes before these two refuse, dcr-purse does not
            // (PARITY.md, `purse-udb` transaction store, item (9)).
            let writes_before_failing = matches!(
                m["op"].as_str(),
                Some("store.import_cfilters_v2" | "store.insert_missing_cfilters")
            );
            if rng.below(2) == 0 && !writes_before_failing {
                let kinds: Vec<&str> = TOLERABLE
                    .iter()
                    .copied()
                    .filter(|_| rng.below(2) == 0)
                    .collect();
                if !kinds.is_empty() {
                    m["tolerate"] = json!(kinds);
                }
            }
            members.push(m);
            i += 1;
        }
        let queries = members
            .iter()
            .all(|m| !CHANGING.contains(&m["op"].as_str().unwrap_or("")));
        let kind = if queries && rng.below(2) == 0 {
            "view"
        } else {
            "update"
        };
        if kind == "view" {
            out.push(json!({"op": kind, "ops": members}));
            continue;
        }
        // Cut before a member counting `unspent` after a member that may
        // write it, once an output has been credited (the doc above).
        let mut part: Vec<Value> = Vec::new();
        let mut wrote = false;
        for (k, m) in members.into_iter().enumerate() {
            let name = m["op"].as_str().unwrap_or("").to_owned();
            if wrote && first_credit < start + k && UNSPENT_COUNTERS.contains(&name.as_str()) {
                out.push(json!({"op": "update", "ops": std::mem::take(&mut part)}));
                wrote = false;
            }
            wrote |= UNSPENT_WRITERS.contains(&name.as_str());
            part.push(m);
        }
        out.push(json!({"op": "update", "ops": part}));
    }
    script["ops"] = Value::Array(out);
    script
}
