// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The `update` and `view` groups of the Rust runner (ADR-0002 section 2,
//! "Transactions"; `tools/oracle/cmd/oprunner/script.go`, `group`), for
//! members that are transaction store operations or its fixtures.
//!
//! Go runs a group's members in one `walletdb.Update` or `View`; a
//! member's error ends the group and rolls the transaction back unless the
//! member's `tolerate` names its kind, in which case what the member wrote
//! before its error stays, and the group's record holds its members'
//! records. The Rust runner runs each member as the top level does, each
//! in its own transaction, but an `update` group's member's transaction is
//! committed even when the member fails, and an `update` group that ends
//! on an untolerated error puts the store file back as it was before the
//! group (and the block labels with it). That is one transaction's outcome
//! for these members, since none reads a count that a transaction's own
//! writes would change (`UnspentOutputCount` reads the `unspent` table,
//! which the first slice never writes); and a member that writes before
//! failing shows in the export where Go's does not, or the reverse. The
//! two operations whose failures write in Go and not here
//! (`ImportCFiltersV2`, `InsertMissingCFilters`; crate `txstore`, "A
//! failed operation writes nothing") are never given `tolerate` by the
//! generated scripts. Address manager operations are not accepted in a
//! group yet: their memory follows the store's (QK-0733), which a file put
//! back would bypass.
//!
//! Go never tolerates a panic: a member that panics ends its group, and
//! the transaction rolls back. Where Go panics the Rust operation returns
//! an error (PARITY.md, `purse-udb` transaction store, item (2)), whose
//! kind a `tolerate` list can name, and the Rust group would go on where
//! Go's stops. So a member whose operation returns such an error may not
//! tolerate the kinds it takes ([`PANIC_STAND_INS`]); the runner refuses
//! the script as a script fault, as it refuses one Go would run
//! differently in any other way it cannot reproduce. The refusal covers
//! the genuine `Invalid` of `Rollback` to height 0 too: the runner cannot
//! tell before the member runs which of the two it will meet.

use std::sync::atomic::Ordering;

use purse_errors::{AnyError, Kind};
use purse_store::Store;
use purse_testutil::serde_json::{Map, Value, json};

use super::{Args, KIND_NAMES, OpResult, Runner, error_value, fault, txops};

/// The transaction store operations that need a read-write transaction
/// (Go's `writeTx` kind); a `view` group refuses them.
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

/// The operations that return an error where Go panics, with the kinds
/// of those errors (PARITY.md, `purse-udb` transaction store, item (2);
/// crate `txstore`, "Panics and errors"): `Rollback` to a height two or
/// more above the tip or below 0, `GetMainChainBlockHashes` from and
/// `InsertMinedTx` into a block whose header is above the tip or
/// detached, and `InsertMinedTx` of an unmined transaction into a
/// stake-invalidated block (`Invalid`); `AddCredit` of a regular
/// transaction whose block has no header or no block row at the header's
/// height, or without a block for a transaction not recorded unmined
/// (`Invalid`); `PreviousPkScripts` without a block, which Go ends by
/// dereferencing the missing block (`Invalid`); `MainChainTip` with no
/// tip, `InsertMemPoolTx` of a vote or revocation whose double-spent input
/// names a missing record, `UnspentOutputs`, `UnspentOutput` and
/// `RandomUTXO` with an output's record missing, `SelectInputs` with an
/// unmined credit's record missing, and `UnspentTickets` with an
/// unmined-inputs entry naming a spender with no unmined record (`IO`). `IO` is listed for the first three as
/// well, for the missing or corrupt stored state at which Go's reads
/// panic.
const PANIC_STAND_INS: &[(&str, &[Kind])] = &[
    ("store.rollback", &[Kind::Invalid, Kind::Io]),
    (
        "store.get_main_chain_block_hashes",
        &[Kind::Invalid, Kind::Io],
    ),
    ("store.insert_mined_tx", &[Kind::Invalid, Kind::Io]),
    ("store.main_chain_tip", &[Kind::Io]),
    ("store.insert_mem_pool_tx", &[Kind::Io]),
    ("store.add_credit", &[Kind::Invalid, Kind::Io]),
    ("store.previous_pk_scripts", &[Kind::Invalid]),
    ("store.unspent_outputs", &[Kind::Io]),
    ("store.unspent_output", &[Kind::Io]),
    ("store.unspent_tickets", &[Kind::Io]),
    ("store.random_utxo", &[Kind::Io]),
    ("store.select_inputs", &[Kind::Io]),
];

/// A script fault when `op` may not tolerate one of `kinds`: a kind of an
/// error that `op` returns where Go panics, which Go's group never
/// tolerates ([`PANIC_STAND_INS`]).
fn check_tolerable(op: &str, kinds: &[Kind]) -> Result<(), String> {
    let Some((_, panics)) = PANIC_STAND_INS.iter().find(|(name, _)| *name == op) else {
        return Ok(());
    };
    match kinds.iter().find(|k| panics.contains(k)) {
        Some(k) => Err(fault(format!(
            "{op}: tolerate: {} is the kind of an error dcr-purse returns where Go panics, \
             and Go never tolerates a panic (PARITY.md, purse-udb transaction store, item (2))",
            KIND_NAMES[usize::from(k.code())]
        ))),
        None => Ok(()),
    }
}

/// Go's `tolerateKinds`: the member's `tolerate` list, absent or a
/// non-empty list of kind names other than `Other`.
fn tolerate_kinds(op: &str, m: &Map<String, Value>) -> Result<Vec<Kind>, String> {
    let Some(v) = m.get("tolerate") else {
        return Ok(Vec::new());
    };
    let names = v
        .as_array()
        .ok_or_else(|| fault(format!("{op}: tolerate must be a list")))?;
    if names.is_empty() {
        return Err(fault(format!("{op}: tolerate is empty")));
    }
    names
        .iter()
        .map(|n| {
            let n = n.as_str().unwrap_or("");
            KIND_NAMES
                .iter()
                .position(|k| *k == n)
                .filter(|i| *i > 0)
                .and_then(|i| i64::try_from(i).ok())
                .and_then(Kind::from_code)
                .ok_or_else(|| {
                    fault(format!(
                        "{op}: tolerate: {n:?} is not a dcrwallet error kind other than Other"
                    ))
                })
        })
        .collect()
}

impl Runner {
    /// The store file closed, copied to `to` (or from `from` back over
    /// it), and opened again; the manager is left as it is.
    fn copy_store(&mut self, from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
        if let Some(s) = self.store.take() {
            s.close().map_err(|e| fault(format!("close store: {e}")))?;
        }
        std::fs::copy(from, to).map_err(|e| fault(format!("copy store: {e}")))?;
        self.store = Some(Store::open(&self.path).map_err(|e| fault(format!("open store: {e}")))?);
        Ok(())
    }

    /// Go's `group`: the members' records, and the group's result (`null`,
    /// or the error that ended it).
    pub(super) fn group(&mut self, a: &Args<'_>) -> Result<(OpResult, Vec<Value>), String> {
        a.check(&["ops"])?;
        let view = a.op == "view";
        let members = a
            .value("ops")?
            .as_array()
            .ok_or_else(|| fault(format!("{}: ops must be a list", a.op)))?
            .clone();
        let snapshot = self.path.with_extension("group");
        if !view {
            let path = self.path.clone();
            self.copy_store(&path, &snapshot)?;
        }
        let blocks = self.blocks.clone();
        let mut records = Vec::with_capacity(members.len());
        let mut ending: Option<AnyError> = None;
        for (j, raw) in members.iter().enumerate() {
            let m = raw
                .as_object()
                .ok_or_else(|| fault("an operation must be an object"))?;
            let name = m
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| fault("an operation needs an op name"))?;
            if !txops::handles(name) {
                return Err(fault(format!(
                    "{name} in {}: the Rust runner takes only transaction store operations in a group",
                    a.op
                )));
            }
            if view && CHANGING.contains(&name) {
                return Err(fault(format!(
                    "{name} needs a read-write transaction, not a view"
                )));
            }
            let tolerate = tolerate_kinds(name, m)?;
            check_tolerable(name, &tolerate)?;
            let mut member = m.clone();
            member.remove("tolerate");
            self.in_group = !view;
            let before = self.drawn.load(Ordering::Relaxed);
            let ran = self.op(&Value::Object(member));
            self.in_group = false;
            let drawn = self.drawn.load(Ordering::Relaxed).saturating_sub(before);
            let (name, r) = ran?;
            let mut rec = Map::new();
            rec.insert("i".to_owned(), json!(j));
            rec.insert("op".to_owned(), json!(name));
            // Go's members carry `drawn` too; only the store's random
            // operations draw among the members this runner takes.
            if drawn != 0 {
                rec.insert("drawn".to_owned(), json!(drawn));
            }
            match r {
                Ok(v) => {
                    rec.insert("ok".to_owned(), v);
                    records.push(Value::Object(rec));
                }
                Err(e) => {
                    rec.insert("err".to_owned(), error_value(&e));
                    let tolerated = tolerate.iter().any(|k| e.is_kind(*k));
                    if tolerated {
                        rec.insert("tolerated".to_owned(), json!(true));
                    }
                    records.push(Value::Object(rec));
                    if !tolerated {
                        ending = Some(e);
                        break;
                    }
                }
            }
        }
        if !view {
            if ending.is_some() {
                // The transaction rolled back: so do the labels its members
                // registered.
                let path = self.path.clone();
                self.copy_store(&snapshot, &path)?;
                self.blocks = blocks;
            }
            std::fs::remove_file(&snapshot).map_err(|e| fault(format!("remove snapshot: {e}")))?;
        }
        Ok((
            match ending {
                Some(e) => Err(e),
                None => Ok(Value::Null),
            },
            records,
        ))
    }
}
