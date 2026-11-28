// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The `update` and `view` groups of the Rust runner (ADR-0002 section 2,
//! "Transactions"; `tools/oracle/cmd/oprunner/script.go`, `group`).
//!
//! Go runs a group's members in one `walletdb.Update` or `View`; a
//! member's error ends the group and rolls the transaction back unless the
//! member's `tolerate` names its kind, in which case what the member wrote
//! before its error stays, and the group's record holds its members'
//! records. The Rust runner does the same over `purse-store`: an `update`
//! group is one write transaction ([`Store::update`]), committed when no
//! member's error ends the group and rolled back when one does, and a
//! `view` group one read transaction; every member reads and writes
//! through the group's transaction ([`Cx`]), as ADR-0002 section 6 says.
//! Members are transaction store, auxiliary record and address manager
//! operations and the fixtures, as in Go (any operation but `rt.reopen`,
//! `rt.checkpoint`, a group and a wallet session's operations).
//!
//! Address manager operations take the caller's transaction, as Go's do,
//! and the manager caches only what it reads from a committed snapshot
//! (`purse_udb::addrmgr`, `StoreRead::is_snapshot`), so after a group that
//! rolled back its memory agrees with the store (QK-0733, not reproduced:
//! ADR-0002, decision by the project owner of 2026-11-19, item 2). Go's
//! manager changes its memory inside the transaction and keeps the change
//! after a rollback; where that shows in a result the two runners differ
//! by decision, and `tests/groups.rs` pins each such case on both sides
//! (PARITY.md, `purse-udb`, "Groups"). The address manager members of
//! dcrwallet's `processTransactionRecord` (`Address`, `MarkUsed`,
//! `AccountProperties`, `SyncAccountToAddrIndex`, `RedeemScript`, the
//! imports) write only the store, or cache only what the committed store
//! holds, so no such case arises from them: the generated scripts of
//! `tests/support/ptrgen.rs` run them in groups that commit and in groups
//! that roll back, every record and the export equal to Go's. Two methods
//! are refused as members, as a script fault: `ChangePassphrase` and
//! `ConvertToWatchingOnly`, which in dcr-purse take the store and commit a
//! write transaction of their own ([`ITS_OWN_TRANSACTION`]; QK-0733), so
//! they cannot share a group's. The runner's bookkeeping that follows the
//! store (block and address labels, the account passphrases and the
//! wallet's passphrases) is put back when an `update` group rolls back, as
//! Go's runner puts its own back (`save`, `restore`); transaction labels,
//! the header counter, the clock and the random streams are not, as in Go.
//!
//! The store's consistency checks (`super::consistent`, as
//! `txstore::verify_backup` runs them; ADR-0018 BK4) run after every
//! member, on the group's transaction ([`consistent_in`]): an `update`
//! group's write transaction is checked with every write of the group so
//! far, before it commits or rolls back (`purse-store`'s `WriteTx`
//! checks and `txstore::check_decoded` through the same [`Rtx`] the
//! members read), so a state that only a member saw, which a rollback
//! then removes, is still held to the check; the member whose untolerated
//! error ends the group is checked too. A failing check is a fault of the
//! run naming the member. The runner checks the store once more after the
//! group, on a read transaction, whether the group committed or rolled
//! back, as after any top-level operation. Under the emulation this
//! replaced, each member committed on its own and only the state after
//! the group was checked. `tests/groups.rs` edits a group's transaction
//! after a member, and the store after a group, and shows each refused
//! there (`a_corrupted_member_state_inside_a_group_is_caught`,
//! `a_group_is_checked_after_it_commits_or_rolls_back`).
//!
//! Inside an `update` group a count of the `unspent` table includes the
//! group's own writes, as `purse-store` counts a write transaction's
//! writes, where Go's `UnspentOutputCount` and `MakeInputSource` read
//! bbolt's `KeyN`, which counts the committed pages only (QK-0003): a
//! member counting after a member that wrote the table (`AddCredit` with
//! a block, `InsertMinedTx`, `Rollback`, `ExtendMainChain`) answers with
//! the group's writes counted in dcr-purse and with the count as the
//! group began in Go
//! (`a_count_after_an_unspent_write_in_a_group_is_gos_committed_count`,
//! `tests/guards.rs`; PARITY.md, `purse-udb` transaction store, item
//! (26); the generated group scripts never make the sequence,
//! `txgen::group_txstore`). Running the group in one transaction leaves
//! dcr-purse's answers there as they were when each member committed on
//! its own: the count read the same rows either way.
//!
//! A member whose operation fails after writing part of what it writes
//! keeps that part in the group's transaction when its error is tolerated,
//! as in Go; the two operations whose failures write in Go and not in
//! dcr-purse (`ImportCFiltersV2`, `InsertMissingCFilters`; crate
//! `txstore`, "A failed operation writes nothing") are never given
//! `tolerate` by the generated scripts, and `SetVSPTicket`'s partial write
//! is refused ([`PARTIAL_WRITES`]).
//!
//! Go never tolerates a panic: a member that panics ends its group, and
//! the transaction rolls back. Where Go panics the Rust operation returns
//! an error (ADR-0012 Rule 2), whose kind a `tolerate` list can name, and
//! the comparison accepts any error against Go's `Panic`, so a Rust group
//! tolerating it would go on and commit where Go's stops and rolls back,
//! with every record compared equal. So a member whose operation returns
//! such an error may not tolerate the kinds it takes: the transaction
//! store's (PARITY.md, `purse-udb` transaction store, item (2)) and the
//! address manager's `MarkUsed`, `MarkUsedChildIndex` and
//! `MarkReturnedChildIndex` on an account with no row (`IO`, QK-0039)
//! always ([`PANIC_STAND_INS`]), and the address manager's `PrivateKey`
//! of an imported key (`Crypto`, QK-0763) while the wallet was converted
//! in this process and not reopened since, when Go's private crypto key
//! is nil ([`LIVE_CONVERSION_STAND_INS`]). The runner refuses the script
//! as a script fault, as it refuses one Go would run differently in any
//! other way it cannot reproduce. The refusal covers the genuine errors
//! of those kinds too (the `Invalid` of `Rollback` to height 0, an `IO`
//! of a store that fails): the runner cannot tell before the member runs
//! which it will meet. The address manager's other errors where Go
//! panics need no refusal: `SetAccountPassphrase` and
//! `ImportVotingAccount` at 256 processors or more return Argon2id's
//! refusal with kind `Other` (QK-0630), which no `tolerate` list can
//! name, so the group ends on both sides; an account row whose key
//! material does not decode (`IO`, QK-0042) is a state no operation
//! writes. Where Go panics and dcr-purse succeeds, `ImportVotingAccount`
//! after a conversion in this process (QK-0765), the comparison itself
//! fails at the member, as at the top level, and a group goes on in
//! dcr-purse where Go's rolls back (`tests/groups.rs` pins it).

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::sync::atomic::Ordering;

use purse_errors::{AnyError, Kind};
use purse_store::{BirthState, Hash32, ReadTx, Table, TableId, Tip, WalletRecord, WriteTx};
use purse_testutil::serde_json::{Map, Value, json};
use purse_udb::addrmgr::{Address, StoreRead};
use purse_udb::txstore::TxRead;

use super::{Args, KIND_NAMES, OpResult, Runner, error_value, fault};

/// The transaction an operation runs in: its own (`Top`), or its group's.
pub(super) enum Cx<'g, 't> {
    /// A top-level operation: its own transaction, as Go's runner gives it.
    Top,
    /// A member of a `view` group: the group's read transaction.
    View(&'g mut ReadTx),
    /// A member of an `update` group: the group's write transaction.
    Update(&'g mut WriteTx<'t>),
}

/// The transaction a reading operation reads: a read transaction (its own
/// or a `view` group's) or an `update` group's write transaction, whose
/// writes so far it sees. Both the address manager's reads
/// ([`StoreRead`]) and the transaction store's ([`TxRead`]) take it.
pub(super) enum Rtx<'a, 't> {
    /// A read transaction: a committed snapshot.
    Read(&'a mut ReadTx),
    /// A write transaction, which may still roll back.
    Write(&'a mut WriteTx<'t>),
}

/// Calls the same method on whichever transaction an [`Rtx`] holds.
macro_rules! either {
    ($rtx:expr, $t:ident => $e:expr) => {
        match $rtx {
            Rtx::Read($t) => $e,
            Rtx::Write($t) => $e,
        }
    };
}

impl StoreRead for Rtx<'_, '_> {
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>> {
        either!(self, t => StoreRead::wallet_row::<R>(&mut **t))
    }
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        either!(self, t => StoreRead::get::<T>(&mut **t, key))
    }
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>> {
        either!(self, t => StoreRead::account_numbers(&mut **t))
    }
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64> {
        either!(self, t => StoreRead::count_prefix::<T>(&mut **t, prefix))
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        either!(self, t => StoreRead::table_len(&mut **t, table))
    }
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        either!(self, t => StoreRead::rows_with_prefix::<T>(&mut **t, prefix))
    }
    fn is_snapshot(&self) -> bool {
        either!(self, t => StoreRead::is_snapshot(&**t))
    }
}

impl TxRead for Rtx<'_, '_> {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        either!(self, t => TxRead::get::<T>(&mut **t, key))
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        either!(self, t => TxRead::contains::<T>(&mut **t, key))
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        either!(self, t => TxRead::rows::<T>(&mut **t, lo, hi, rev, limit))
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        either!(self, t => TxRead::keys::<T>(&mut **t, lo, hi, rev, limit))
    }
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool> {
        either!(self, t => TxRead::table_is_empty(&mut **t, table))
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        either!(self, t => TxRead::table_len(&mut **t, table))
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        either!(self, t => TxRead::tip_row(&mut **t))
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        either!(self, t => TxRead::birth_row(&mut **t))
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        either!(self, t => TxRead::have_cfilters_row(&mut **t))
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        either!(self, t => TxRead::processed_row(&mut **t))
    }
}

/// The address manager methods that begin and commit a write transaction
/// of their own in dcr-purse (they take the store, where Go's take the
/// caller's bucket: QK-0733, "For `ConvertToWatchingOnly`" and "For
/// `ChangePassphrase`"), so they cannot be members of a group: the
/// group's write transaction is open while its members run, and
/// `purse-store` runs write transactions one at a time. The runner
/// refuses such a member as a script fault (PARITY.md, `purse-udb`,
/// "Groups"); Go's runner runs it in the group's transaction.
pub const ITS_OWN_TRANSACTION: &[&str] = &["mgr.change_passphrase", "mgr.convert_to_watching_only"];

/// The top-level operations Go's runner does not take as members
/// (`script.go`, `top`: they are not in its operation table, so a member
/// naming one is an unknown operation).
const TOP_LEVEL_ONLY: &[&str] = &["update", "view", "rt.reopen", "rt.checkpoint"];

/// What ends an `update` group's transaction other than a commit.
enum Abort {
    /// A member's untolerated error: the group rolls back.
    Ended(AnyError),
    /// A script fault or a runner failure: the run ends.
    Fault(String),
    /// The store's failure.
    Store(purse_store::Error),
}

impl From<purse_store::Error> for Abort {
    fn from(e: purse_store::Error) -> Self {
        Abort::Store(e)
    }
}

/// The runner's bookkeeping that follows the store (Go's `bookkeeping`,
/// `script.go`): saved before a group and put back when it rolls back.
struct Bookkeeping {
    pub_pass: Vec<u8>,
    priv_pass: Option<Vec<u8>>,
    acct_pass: BTreeMap<u32, Vec<u8>>,
    blocks: HashMap<String, Hash32>,
    addrs: HashMap<String, Address>,
}

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
/// panic. And the address manager's `MarkUsed`, `MarkUsedChildIndex` and
/// `MarkReturnedChildIndex` on an account with no row, which fail with
/// `I/O error: no account <n>` where Go dereferences the missing bucket
/// (`IO`; QK-0039, PARITY.md, `purse-udb`).
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
    ("mgr.mark_used", &[Kind::Io]),
    ("mgr.mark_used_child_index", &[Kind::Io]),
    ("mgr.mark_returned_child_index", &[Kind::Io]),
];

/// The operations that return an error where Go panics only while the
/// wallet was converted to watching-only in this process and not
/// reopened since, with the kinds of those errors: `PrivateKey` of an
/// imported key, which fails with the error Go returns after a reopen
/// (`Crypto`, `... missing nonce`) where Go's converted manager
/// dereferences its nil private crypto key (QK-0763, PARITY.md,
/// `purse-udb`). After a reopen Go returns that error too, and a member
/// may tolerate it.
const LIVE_CONVERSION_STAND_INS: &[(&str, &[Kind])] = &[("mgr.private_key", &[Kind::Crypto])];

/// A script fault when `op` may not tolerate one of `kinds`: a kind of an
/// error that `op` returns where Go panics, which Go's group never
/// tolerates ([`PANIC_STAND_INS`], and [`LIVE_CONVERSION_STAND_INS`] when
/// `converted_live`, the wallet converted in this process and not
/// reopened since).
fn check_tolerable(op: &str, kinds: &[Kind], converted_live: bool) -> Result<(), String> {
    let always = PANIC_STAND_INS.iter().map(|e| (e, ""));
    let live = LIVE_CONVERSION_STAND_INS
        .iter()
        .filter(|_| converted_live)
        .map(|e| (e, ", after a conversion before a reopen"));
    for ((_, panics), when) in always.chain(live).filter(|((name, _), _)| *name == op) {
        if let Some(k) = kinds.iter().find(|k| panics.contains(k)) {
            return Err(fault(format!(
                "{op}: tolerate: {} is the kind of an error dcr-purse returns where Go \
                 panics{when}, and Go never tolerates a panic (PARITY.md, purse-udb, \
                 Groups; transaction store, item (2))",
                KIND_NAMES[usize::from(k.code())]
            )));
        }
    }
    Ok(())
}

/// The operations whose Go refusal comes after Go has written part of
/// what they write, with the kinds of those refusals: `SetVSPTicket` of an
/// empty host or one over bbolt's key limit has written the host row and
/// raised the counter when its public-key put fails (QK-0122), a state
/// the store's host tables cannot hold (ADR-0003 R2), where dcr-purse's
/// refuses before writing. Go's caller abandons that transaction; a group
/// that tolerated the error would keep it.
const PARTIAL_WRITES: &[(&str, &[Kind])] = &[("udb.set_vsp_ticket", &[Kind::Invalid])];

/// A script fault when `op` may not tolerate one of `kinds`: a kind of a
/// refusal Go makes after writing part of the operation
/// ([`PARTIAL_WRITES`]).
fn check_whole(op: &str, kinds: &[Kind]) -> Result<(), String> {
    let Some((_, partial)) = PARTIAL_WRITES.iter().find(|(name, _)| *name == op) else {
        return Ok(());
    };
    match kinds.iter().find(|k| partial.contains(k)) {
        Some(k) => Err(fault(format!(
            "{op}: tolerate: Go refuses with {} after writing part of the operation, \
             which dcr-purse's store cannot hold (QK-0122)",
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
    /// Go's `save`.
    fn save(&self) -> Bookkeeping {
        Bookkeeping {
            pub_pass: self.pub_pass.clone(),
            priv_pass: self.priv_pass.clone(),
            acct_pass: self.acct_pass.clone(),
            blocks: self.blocks.clone(),
            addrs: self.addrs.clone(),
        }
    }

    /// Go's `restore`.
    fn restore(&mut self, b: Bookkeeping) {
        self.pub_pass = b.pub_pass;
        self.priv_pass = b.priv_pass;
        self.acct_pass = b.acct_pass;
        self.blocks = b.blocks;
        self.addrs = b.addrs;
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
        let saved = self.save();
        let store = self.store.take().ok_or_else(|| fault("no store open"))?;
        let mut records = Vec::with_capacity(members.len());
        let ran = if view {
            match store.read() {
                Ok(mut tx) => {
                    let r = self.members(&mut Cx::View(&mut tx), a.op, &members, &mut records);
                    tx.close().map_err(|e| fault(format!("close: {e}"))).and(r)
                }
                Err(e) => Err(fault(format!("read: {e}"))),
            }
        } else {
            let r: Result<(), Abort> = store.update(|tx| {
                match self.members(&mut Cx::Update(tx), a.op, &members, &mut records) {
                    Ok(None) => Ok(()),
                    Ok(Some(e)) => Err(Abort::Ended(e)),
                    Err(f) => Err(Abort::Fault(f)),
                }
            });
            match r {
                Ok(()) => Ok(None),
                Err(Abort::Ended(e)) => Ok(Some(e)),
                Err(Abort::Fault(f)) => Err(f),
                Err(Abort::Store(e)) => Err(fault(format!("update: {e}"))),
            }
        };
        self.store = Some(store);
        let ending = ran?;
        if ending.is_some() {
            // The transaction rolled back: so does the bookkeeping its
            // members registered.
            self.restore(saved);
        }
        Ok((
            match ending {
                Some(e) => Err(e),
                None => Ok(Value::Null),
            },
            records,
        ))
    }

    /// Runs a group's members in its transaction `cx`, appending their
    /// records: the error of the member that ended the group, or `None`
    /// when every member succeeded or had its error tolerated.
    fn members(
        &mut self,
        cx: &mut Cx<'_, '_>,
        group: &str,
        members: &[Value],
        records: &mut Vec<Value>,
    ) -> Result<Option<AnyError>, String> {
        let view = matches!(cx, Cx::View(_));
        for (j, raw) in members.iter().enumerate() {
            let m = raw
                .as_object()
                .ok_or_else(|| fault("an operation must be an object"))?;
            let name = m
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| fault("an operation needs an op name"))?;
            if TOP_LEVEL_ONLY.contains(&name) {
                return Err(fault(format!("unknown op {name:?} in {group}")));
            }
            if ITS_OWN_TRANSACTION.contains(&name) {
                return Err(fault(format!(
                    "{name} in {group}: dcr-purse's method commits a write transaction of its \
                     own and cannot run in the group's (QK-0733; PARITY.md, purse-udb, Groups)"
                )));
            }
            if view && (CHANGING.contains(&name) || super::auxops::AUX_CHANGING.contains(&name)) {
                return Err(fault(format!(
                    "{name} needs a read-write transaction, not a view"
                )));
            }
            let tolerate = tolerate_kinds(name, m)?;
            check_tolerable(name, &tolerate, self.converted_live)?;
            check_whole(name, &tolerate)?;
            let mut member = m.clone();
            member.remove("tolerate");
            let before = self.drawn.load(Ordering::Relaxed);
            let ran = self.op(cx, &Value::Object(member));
            let drawn = self.drawn.load(Ordering::Relaxed).saturating_sub(before);
            let (name, r) = ran?;
            let mut rec = Map::new();
            rec.insert("i".to_owned(), json!(j));
            rec.insert("op".to_owned(), json!(name));
            // Go's members carry `drawn` too.
            if drawn != 0 {
                rec.insert("drawn".to_owned(), json!(drawn));
            }
            let ending = match r {
                Ok(v) => {
                    rec.insert("ok".to_owned(), v);
                    records.push(Value::Object(rec));
                    None
                }
                Err(e) => {
                    rec.insert("err".to_owned(), error_value(&e));
                    let tolerated = tolerate.iter().any(|k| e.is_kind(*k));
                    if tolerated {
                        rec.insert("tolerated".to_owned(), json!(true));
                    }
                    records.push(Value::Object(rec));
                    (!tolerated).then_some(e)
                }
            };
            self.corrupt_member(cx, j)?;
            consistent_in(cx).map_err(|e| {
                format!(
                    "{group} member {j} ({name}): the group's transaction fails the \
                     consistency checks: {e}"
                )
            })?;
            if let Some(e) = ending {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    /// A test's edit of the group's write transaction after member `j`
    /// ([`super::Corruption`]).
    fn corrupt_member(&self, cx: &mut Cx<'_, '_>, j: usize) -> Result<(), String> {
        let Some(c) = self.corrupt else {
            return Ok(());
        };
        if c.op != self.op_index || c.member != Some(j) {
            return Ok(());
        }
        match cx {
            Cx::Update(tx) => {
                (c.edit)(tx).map_err(|e| fault(format!("member {j}: the test's edit: {e}")))
            }
            Cx::View(_) | Cx::Top => Err(fault("the test's edit needs an update group")),
        }
    }
}

/// The store's consistency checks ([`super::consistent`], as
/// `txstore::verify_backup` runs them) on a member's transaction: a
/// `view` group's read transaction, or an `update` group's write
/// transaction with every write of the group so far, before it commits or
/// rolls back (`purse-store`'s `WriteTx::check_rows`, `check_chain`,
/// `check_records` and `check_tx_tables`, and `txstore::check_decoded`
/// through the same [`Rtx`] the members read).
fn consistent_in(cx: &mut Cx<'_, '_>) -> purse_store::Result<()> {
    match cx {
        Cx::Top => Ok(()),
        Cx::View(tx) => super::consistent(tx),
        Cx::Update(tx) => {
            tx.check_rows()?;
            tx.check_chain()?;
            tx.check_records()?;
            tx.check_tx_tables()?;
            purse_udb::txstore::check_decoded(&mut Rtx::Write(tx))
        }
    }
}
