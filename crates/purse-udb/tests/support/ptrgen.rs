// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Generated operation scripts shaped like dcrwallet's
//! `processTransactionRecord` (`wallet/chainntfns.go` at the pin), which
//! runs in one `walletdb.Update` per transaction: `InsertMinedTx` (or
//! `InsertMemPoolTx`); for a multisig input, `Address` of each key's
//! address, `markUsedAddress` and `ImportScript` of the redeem script;
//! for each output, `Address` of what it pays, `AddCredit` (or
//! `AddTicketCommitment`) with that address's account and branch, and
//! `markUsedAddress`, which is `MarkUsed`, `AccountProperties` and
//! `SyncAccountToAddrIndex` to the last used index plus the gap limit
//! (`wallet/addresses.go`); `RedeemScript` of a P2SH output before its
//! `AddMultisigOut`.
//!
//! `generate_ptr` draws a credit script (`txgen::generate_txstore_credits`)
//! or a commitment and multisig script (`txgen::generate_txstore_cms`),
//! half the time each, from the same stream, and then, with draws of its
//! own after the base script's:
//!
//! - puts a prologue before it: `Unlock`, `NewAccount` of account 1 (and
//!   of account 2 half the time), `ImportXpubAccount` (account 2^31), now
//!   and then imported private keys and a `SyncAccountToAddrIndex` of
//!   account 0, and in one script in six `ConvertToWatchingOnly`, all at
//!   the top level, so every account exists in the committed store before
//!   any group reads it;
//! - makes some of the outputs the built transactions pay to account 0's
//!   derived addresses pay account 1 or 2's, the imported xpub account's,
//!   or the P2PKH address of an import key, so credits land in several
//!   accounts and in the imported account;
//! - in every `update` group that holds an `AddCredit`,
//!   `AddTicketCommitment` or `AddMultisigOut`, puts the address manager
//!   members `processTransactionRecord` makes around each (above): each
//!   `Address` and `MarkUsed` tolerating `NotExist` three times in four,
//!   as Go skips a missing address (otherwise the group rolls back, as
//!   `processTransactionRecord`'s caller abandons its transaction on an
//!   error); the credit's account and change flag taken from the address
//!   half the time; now and then a multisig input's members and an
//!   `ImportPrivateKey`, `ImportPublicKey` or `ImportScript` member; and
//!   in one group in five a last member that looks up an address no
//!   wallet holds without tolerating `NotExist`, standing for any error
//!   on which `processTransactionRecord`'s caller abandons the
//!   transaction, so that the address manager's writes before it roll
//!   back;
//! - ends with `AccountProperties` and `AccountBalance` of every account,
//!   the address walks of accounts 0 and 1, and `ForEachActiveAddress`.
//!
//! What it never draws, by construction:
//!
//! - an account created, renamed, or given a passphrase in a group, and
//!   no `UpgradeToSLIP0044CoinType` in one: after a rolled-back group Go's
//!   manager answers from what the group put in its memory and dcr-purse's
//!   from the store (QK-0733, QK-0790, QK-0791; `tests/groups.rs` pins
//!   those), and the address manager members drawn here keep nothing in
//!   Go's memory (`MarkUsed`, `SyncAccountToAddrIndex` and the imports
//!   write only the store; `Address`, `AccountProperties` and
//!   `RedeemScript` of accounts the committed store holds cache what the
//!   store holds);
//! - `ChangePassphrase` or `ConvertToWatchingOnly` in a group (refused by
//!   the Rust runner, `groups::ITS_OWN_TRANSACTION`);
//! - a `SyncAccountToAddrIndex` to a hardened index or of an account that
//!   may not exist, and `MarkUsed` only of what the scripts pay and of
//!   import keys' addresses (Go's `MarkUsedChildIndex` panics for an
//!   account without a variables bucket; a recorded chain row's account
//!   always has one, and `MarkUsed` of an imported row marks nothing);
//! - an address label registered in a group (a group that rolls back
//!   would leave a later reference to it unresolved);
//! - a count of the `unspent` table in an `update` group: the base
//!   scripts' groups hold no counter, and the members added here are no
//!   counter either, so the sequence of QK-0003 (crate `txstore`, choice
//!   15, pending owner) never arises (`counts_after_unspent_writes` is
//!   empty for every script; checked by
//!   `process_transaction_record_scripts_avoid_the_pinned_sequences`).
//!
//! The base scripts' rules (QK-0082, QK-0081, QK-0101, QK-0783, QK-0734,
//! QK-0781) stand: the members added here change no store operation, and
//! a group they end early rolls back what the base rules already allow to
//! be rolled back.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

use std::collections::HashMap;

use dcroxide_txscript::stdaddr;
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex};
use purse_udb::addrmgr::ChainParams;

use super::generator::{import_key, import_pub_key};
use super::txgen::{generate_txstore_cms, generate_txstore_credits};

/// The reserved account of imported keys and scripts.
const IMPORTED: u32 = 0x7FFF_FFFF;
/// The account `ImportXpubAccount` makes in a fresh wallet.
const XPUB: u32 = 0x8000_0000;

/// The address manager operations a generated script puts in groups.
pub const PTR_MEMBERS: &[&str] = &[
    "mgr.address",
    "mgr.mark_used",
    "mgr.account_properties",
    "mgr.sync_account_to_addr_index",
    "mgr.redeem_script",
    "mgr.import_private_key",
    "mgr.import_public_key",
    "mgr.import_script",
];

/// What an output pays, as far as the generator knows.
#[derive(Clone)]
enum Payee {
    /// A derived address of a BIP0044 account.
    Derived { acct: u32, branch: u64, index: u64 },
    /// An imported key's P2PKH address (encoded), or an imported script
    /// (a reference), both of the imported account.
    Imported(Value),
}

impl Payee {
    fn addr(&self) -> Value {
        match self {
            Payee::Derived {
                acct,
                branch,
                index,
            } => json!({"acct": acct, "branch": branch, "index": index}),
            Payee::Imported(v) => v.clone(),
        }
    }

    fn account(&self) -> u32 {
        match self {
            Payee::Derived { acct, .. } => *acct,
            Payee::Imported(_) => IMPORTED,
        }
    }

    fn internal(&self) -> bool {
        matches!(self, Payee::Derived { branch: 1, .. })
    }
}

struct Ptr<'r> {
    rng: &'r mut SplitMix64,
    chain: ChainParams,
    /// The BIP0044 accounts other than 0 the prologue creates.
    accounts: Vec<u32>,
    /// For each built transaction, what each output pays.
    payees: HashMap<String, Vec<Option<Payee>>>,
}

impl Ptr<'_> {
    fn below(&mut self, n: u64) -> u64 {
        self.rng.below(n)
    }

    /// `kinds`, each kept with probability 3/4 (none, then no list).
    fn tolerate(&mut self, m: &mut Value, kinds: &[&str]) {
        let kept: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|_| self.below(4) != 0)
            .collect();
        if !kept.is_empty() {
            m["tolerate"] = json!(kept);
        }
    }

    fn p2pkh(&self, pk: &[u8]) -> String {
        stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&stdaddr::hash160(pk), &self.chain)
            .expect("p2pkh")
            .encode()
    }

    /// An import key's P2PKH address.
    fn imported_addr(&mut self) -> Value {
        let pk = import_pub_key(&import_key(self.rng));
        json!(self.p2pkh(&pk))
    }

    /// The payee an output's address reference becomes: account 0's
    /// derived address as drawn half the time, else another account's or
    /// an import key's.
    fn repay(&mut self, pay: &Value) -> (Value, Option<Payee>) {
        let derived = pay.get("acct").and_then(Value::as_u64) == Some(0);
        if !derived {
            // A reference to an imported script (`ms1`, `ms2`, `s1`).
            let payee = pay.get("ref").map(|_| Payee::Imported(pay.clone()));
            return (pay.clone(), payee);
        }
        let branch = pay["branch"].as_u64().unwrap_or(0);
        let index = pay["index"].as_u64().unwrap_or(0);
        let acct = match self.below(12) {
            0..=5 => 0,
            6 | 7 => 1,
            8 => *self.accounts.last().unwrap_or(&1),
            9 => XPUB,
            10 => {
                let a = self.imported_addr();
                return (a.clone(), Some(Payee::Imported(a)));
            }
            _ => 0,
        };
        let p = Payee::Derived {
            acct,
            branch,
            index,
        };
        (p.addr(), Some(p))
    }

    /// `tx.build` with its outputs' payees redrawn, noting them.
    fn build(&mut self, mut op: Value) -> Value {
        let label = op["label"].as_str().unwrap_or("").to_owned();
        let mut payees = Vec::new();
        if let Some(outputs) = op["outputs"].as_array_mut() {
            for o in outputs {
                if let Some(pay) = o.get("pay").cloned() {
                    let (a, p) = self.repay(&pay);
                    o["pay"] = a;
                    payees.push(p);
                } else if let Some(pay) = o.get("commitment").and_then(|c| c.get("pay")).cloned() {
                    let (a, p) = self.repay(&pay);
                    o["commitment"]["pay"] = a;
                    payees.push(p);
                } else {
                    payees.push(None);
                }
            }
        }
        self.payees.insert(label, payees);
        op
    }

    fn payee(&self, m: &Value) -> Option<Payee> {
        let label = m["tx"].as_str()?;
        let index = usize::try_from(m["index"].as_u64()?).ok()?;
        self.payees.get(label)?.get(index)?.clone()
    }

    fn lookup(&mut self, op: &str, addr: Value) -> Value {
        let mut m = json!({"op": op, "addr": addr});
        self.tolerate(&mut m, &["NotExist"]);
        m
    }

    /// `markUsedAddress`'s and `processTransactionRecord`'s members around
    /// a credit or commitment `m` of `p`.
    fn record(&mut self, m: &Value, p: &Payee, out: &mut Vec<Value>) {
        if self.below(4) != 0 {
            out.push(self.lookup("mgr.address", p.addr()));
        }
        let mut m = m.clone();
        if self.below(2) == 0 {
            m["account"] = json!(p.account());
            if m["op"] == "store.add_credit" {
                if p.internal() {
                    m["change"] = json!(true);
                } else if let Some(o) = m.as_object_mut() {
                    o.remove("change");
                }
            }
        }
        out.push(m);
        if self.below(4) != 0 {
            out.push(self.lookup("mgr.mark_used", p.addr()));
        }
        if let Payee::Derived {
            acct,
            branch,
            index,
        } = *p
            && self.below(3) != 0
        {
            if self.below(2) == 0 {
                out.push(json!({"op": "mgr.account_properties", "account": acct}));
            }
            let gap = [0, 1, 2, 5, 20][usize::try_from(self.below(5)).expect("small")];
            out.push(json!({"op": "mgr.sync_account_to_addr_index",
                            "account": acct, "index": index + gap, "branch": branch}));
        }
    }

    /// A multisig input's members: `Address` and `MarkUsed` of a key's
    /// address, and `ImportScript` of a 1-of-2 script of that key.
    fn multisig_input(&mut self, out: &mut Vec<Value>) {
        let k1 = import_pub_key(&import_key(self.rng));
        let k2 = import_pub_key(&import_key(self.rng));
        let a = json!(self.p2pkh(&k1));
        out.push(self.lookup("mgr.address", a.clone()));
        out.push(self.lookup("mgr.mark_used", a));
        let mut m = json!({"op": "mgr.import_script",
                           "script": format!("5121{}21{}52ae", hex(&k1), hex(&k2))});
        self.tolerate(&mut m, &["Exist"]);
        out.push(m);
    }

    /// An import member.
    fn import(&mut self) -> Value {
        let k = import_key(self.rng);
        let mut m = match self.below(3) {
            0 => json!({"op": "mgr.import_private_key", "key": hex(&k)}),
            1 => json!({"op": "mgr.import_public_key", "pubkey": hex(&import_pub_key(&k))}),
            _ => json!({"op": "mgr.import_script",
                        "script": format!("5121{}51ae", hex(&import_pub_key(&k)))}),
        };
        let kinds: &[&str] = match m["op"].as_str() {
            Some("mgr.import_private_key") => &["Exist", "Locked"],
            Some("mgr.import_public_key") => &["Exist", "Invalid"],
            _ => &["Exist"],
        };
        self.tolerate(&mut m, kinds);
        m
    }

    /// An `update` group's members with `processTransactionRecord`'s
    /// address manager members put in (module documentation).
    fn group(&mut self, members: &[Value]) -> Vec<Value> {
        let mut out = Vec::with_capacity(members.len() * 3);
        for (j, m) in members.iter().enumerate() {
            match m["op"].as_str().unwrap_or("") {
                "store.add_credit" | "store.add_ticket_commitment" => match self.payee(m) {
                    Some(p) => self.record(m, &p, &mut out),
                    None => out.push(m.clone()),
                },
                "store.add_multisig_out" => {
                    if let Some(p @ Payee::Imported(_)) = self.payee(m)
                        && self.below(2) == 0
                    {
                        // Go logs a failed lookup and goes on.
                        out.push(self.lookup("mgr.redeem_script", p.addr()));
                    }
                    out.push(m.clone());
                }
                _ => out.push(m.clone()),
            }
            if j == 0 {
                if self.below(4) == 0 {
                    self.multisig_input(&mut out);
                }
                if self.below(5) == 0 {
                    let m = self.import();
                    out.push(m);
                }
            }
        }
        if self.below(5) == 0 {
            // A lookup of an address no wallet holds, not tolerated: it
            // ends the group, standing for any error on which
            // `processTransactionRecord`'s caller abandons the
            // transaction, so the members' writes roll back.
            let h = self.rng.bytes(20);
            let a = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&h, &self.chain)
                .expect("p2pkh")
                .encode();
            out.push(json!({"op": "mgr.address", "addr": a}));
        }
        out
    }

    fn prologue(&mut self) -> Vec<Value> {
        let mut ops = vec![
            json!({"op": "mgr.unlock", "pass": "private"}),
            json!({"op": "mgr.new_account", "name": "one"}),
        ];
        self.accounts.push(1);
        if self.below(2) == 0 {
            ops.push(json!({"op": "mgr.new_account", "name": "two"}));
            self.accounts.push(2);
        }
        ops.push(json!({
            "op": "mgr.import_xpub_account", "name": "watch",
            "xpub": {"seed": "000102030405060708090a0b0c0d0e0f",
                     "path": ["44'", "1'", format!("{}'", self.below(3))]},
        }));
        for _ in 0..self.below(3) {
            // The same key drawn twice is refused with `Exist`.
            let k = import_key(self.rng);
            ops.push(json!({"op": "mgr.import_private_key", "key": hex(&k)}));
        }
        if self.below(2) == 0 {
            ops.push(json!({"op": "mgr.sync_account_to_addr_index", "account": 0,
                            "index": self.below(8), "branch": self.below(2)}));
        }
        if self.below(6) == 0 {
            ops.push(json!({"op": "mgr.convert_to_watching_only"}));
        }
        ops
    }

    fn epilogue(&mut self) -> Vec<Value> {
        let mut accounts = vec![0];
        accounts.extend(self.accounts.iter().copied());
        accounts.extend([XPUB, IMPORTED]);
        let mut ops = Vec::new();
        for a in accounts {
            if a != IMPORTED {
                ops.push(json!({"op": "mgr.account_properties", "account": a}));
            }
            ops.push(json!({"op": "store.account_balance", "minconf": 0, "account": a}));
        }
        for a in [0, 1] {
            ops.push(json!({"op": "mgr.for_each_account_address", "account": a}));
        }
        ops.push(json!({"op": "mgr.for_each_active_address"}));
        ops
    }
}

/// Whether `op` is an `update` group holding a record of a transaction's
/// output.
fn records(op: &Value) -> bool {
    op["op"] == "update"
        && op["ops"].as_array().is_some_and(|m| {
            m.iter().any(|x| {
                matches!(
                    x["op"].as_str(),
                    Some(
                        "store.add_credit"
                            | "store.add_ticket_commitment"
                            | "store.add_multisig_out"
                    )
                )
            })
        })
}

/// A generated script shaped like `processTransactionRecord` (module
/// documentation), on `network_override` or a drawn network.
pub fn generate_ptr(
    rng: &mut SplitMix64,
    id: usize,
    network_override: Option<&'static str>,
) -> Value {
    let cms = rng.below(2) == 0;
    let mut script = if cms {
        generate_txstore_cms(rng, id, network_override)
    } else {
        generate_txstore_credits(rng, id, network_override)
    };
    let network = script["setup"]["network"]
        .as_str()
        .expect("network")
        .to_owned();
    let mut g = Ptr {
        rng,
        chain: ChainParams::by_name(&network).expect("network"),
        accounts: Vec::new(),
        payees: HashMap::new(),
    };
    let mut ops = g.prologue();
    for op in script["ops"].as_array().cloned().unwrap_or_default() {
        if op["op"] == "tx.build" {
            ops.push(g.build(op));
        } else if records(&op) {
            let members = op["ops"].as_array().cloned().unwrap_or_default();
            ops.push(json!({"op": "update", "ops": g.group(&members)}));
        } else {
            ops.push(op);
        }
    }
    ops.extend(g.epilogue());
    script["ops"] = Value::Array(ops);
    script["name"] = json!(format!(
        "ptr {}generated {id}",
        if cms { "commitment and multisig " } else { "" }
    ));
    script
}
