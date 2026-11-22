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

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

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
}

fn pick<'a, T>(rng: &mut SplitMix64, v: &'a [T]) -> Option<&'a T> {
    if v.is_empty() {
        None
    } else {
        v.get(usize::try_from(rng.below(v.len() as u64)).expect("small"))
    }
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
                    }
                }
                v
            }
            "extend.again" => {
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
                v
            }
        }
    }

    fn op(&mut self, name: &str) -> Value {
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
                });
                json!({"op": "tx.raw", "label": label, "tx": tx})
            }
            "store.insert_mem_pool_tx" | "store.remove_unconfirmed" => match self.tx_label() {
                Some(l) => json!({"op": name, "tx": l}),
                None => self.build_regular(),
            },
            "store.insert_mined_tx" => match self.tx_label() {
                Some(l) => {
                    let b = self.block_ref();
                    json!({"op": name, "tx": l, "block": b})
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
                let tip = i64::try_from(self.tip).expect("small");
                let h = if self.below(5) == 0 || self.tip == 0 {
                    [0, -1, tip + 2, tip + 5][usize::try_from(self.below(4)).expect("small")]
                } else {
                    let h = 1 + self.below(self.tip + 1);
                    self.tip = h - 1;
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
    };
    for _ in 0..n {
        let name = TX_OPS[usize::try_from(g.below(TX_OPS.len() as u64)).expect("small")];
        let v = g.op(name);
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
    let rand = hex(&g.rng.bytes(32));
    let mut script = json!({
        "format": "purse-opscript 1",
        "name": format!("txstore generated {id}"),
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
];

/// The kinds a generated group member tolerates: never `Invalid` or `IO`,
/// which dcr-purse returns where Go panics (PARITY.md, `purse-udb`
/// transaction store, item (2)), and a panic is never tolerated.
const TOLERABLE: [&str; 3] = ["Exist", "DoubleSpend", "NotExist"];

/// `generate_txstore`'s script with runs of its operations put in `update`
/// and `view` groups (ADR-0002 section 2), some members tolerating kinds.
/// A group never holds what registers a label (a labelled extension, a
/// built transaction), since a group that ends early or rolls back would
/// leave a later reference to it unresolved, nor a runtime operation.
pub fn group_txstore(rng: &mut SplitMix64, mut script: Value) -> Value {
    let ops = script["ops"].as_array().cloned().unwrap_or_default();
    let groupable = |v: &Value| {
        let name = v["op"].as_str().unwrap_or("");
        (name.starts_with("store.") && v.get("label").is_none()) || name.starts_with("udb.")
    };
    let mut out = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < ops.len() {
        if !groupable(&ops[i]) || rng.below(3) != 0 {
            out.push(ops[i].clone());
            i += 1;
            continue;
        }
        let want = 2 + usize::try_from(rng.below(4)).expect("small");
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
        out.push(json!({"op": kind, "ops": members}));
    }
    script["ops"] = Value::Array(out);
    script
}
