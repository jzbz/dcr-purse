// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The Rust runner of operation scripts (`purse-opscript 1`, ADR-0002),
//! over [`purse_udb::Manager`] and `purse-store`.
//!
//! It reads the scripts Go's `tools/oracle/cmd/oprunner` reads and writes
//! the same per-operation records (ADR-0002 sections 3 and 4) and, at the
//! end, the logical export of its store. It implements the address
//! manager operations ported so far ([`SUPPORTED`]) and refuses any other
//! operation as a script fault, so no script can pass by skipping one.
//!
//! Each operation runs in its own transaction, as Go's top-level
//! operations do: a read transaction for Go's `readTx` operations, a write
//! transaction committed on success and rolled back on an error for its
//! `writeTx` ones. A conversion and a passphrase change run in the write
//! transaction the manager begins itself, committed and rolled back the
//! same way. Address arguments take Go's reference forms (an encoded
//! address, `{"ref": label}`, `{"acct", "branch", "index"}`, `{"p2sh":
//! hex}`), resolved inside the operation's transaction with Go's texts.
//!
//! Its randomness comes from a SplitMix64 stream keyed by `setup.rand`:
//! reproducible from run to run, but not Go's stream (the draw orders of
//! the two sides are not aligned yet, ADR-0002 open item 2), so the
//! differential compares exports under the masked profile.
//!
//! The transaction store's random operations (`store.random_utxo`,
//! `store.select_inputs`) are the exception: they draw from dcrd's
//! generator keyed by their `rand` argument, which restarts Go's stream
//! under the same key (ADR-0002 section 2), so their `~r` results and
//! their `drawn` are Go's exactly. Without `rand` such an operation
//! continues the stream of the last one, which the runner allows only when
//! nothing has drawn since (no address manager, `rt.*` or wallet operation
//! in between); otherwise the script is refused as a script fault, since
//! the runner cannot know where Go's stream stands.
//!
//! `rt.checkpoint` writes the export of the store as it stands, as Go's
//! runner writes its snapshot's (`<name>.pe` beside the store), and the
//! run returns each one with the final export ([`Run::checkpoints`]).
//!
//! A top-level record carries `drawn`, as Go's do, when the operation read
//! bytes from the stream: the bytes every source the runner gave the
//! manager handed out while it ran. The general comparison removes it
//! (`compare::comparable`), as it does Go's; tests read it where a draw, or
//! its absence, is the point (a refusal that must come before any draw).
//! Group members carry it as Go's do.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dcroxide_txscript::stdaddr::{self, hash160};
use purse_dcrutil::SignatureType;
use purse_errors::AnyError;
use purse_snacl::{Limits, RandomSourceError};
use purse_store::{Store, WriteTx};
use purse_testutil::serde_json::{Map, Value, json};
use purse_testutil::{SplitMix64, hex, unhex};
use purse_udb::Manager;
use purse_udb::addrmgr::{
    self, Address, BoxedRandom, ChainParams, ExtendedKey, ManagedAddress, StoreRead, Wif,
};
use purse_udb::export::{ExportOptions, export};
use purse_udb::txstore::{TxRecord, TxStore};

pub mod auxops;
mod groups;
mod txops;

/// The operations this runner implements, every one with Go's arguments
/// and result shape.
pub const SUPPORTED: &[&str] = &[
    "udb.coin_types",
    "udb.hd_keys_from_seed",
    "udb.validate_account_name",
    "udb.is_imported_voting",
    "mgr.watching_only",
    "mgr.is_locked",
    "mgr.chain_params",
    "mgr.coin_type",
    "mgr.coin_type_priv_key",
    "mgr.upgrade_to_slip0044_coin_type",
    "mgr.encrypt",
    "mgr.decrypt",
    "mgr.lock",
    "mgr.unlock",
    "mgr.unlocked_with_passphrase",
    "mgr.unlock_account",
    "mgr.lock_account",
    "mgr.set_account_passphrase",
    "mgr.change_passphrase",
    "mgr.convert_to_watching_only",
    "mgr.last_account",
    "mgr.last_imported_account",
    "mgr.account_name",
    "mgr.for_each_account",
    "mgr.account_properties",
    "mgr.account_has_passphrase",
    "mgr.account_extended_pub_key",
    "mgr.account_extended_priv_key",
    "mgr.account_branch_extended_pub_key",
    "mgr.lookup_account",
    "mgr.new_account",
    "mgr.rename_account",
    "mgr.import_xpub_account",
    "mgr.import_voting_account",
    "mgr.address",
    "mgr.addr_account",
    "mgr.exists_hash160",
    "mgr.import_private_key",
    "mgr.import_public_key",
    "mgr.import_script",
    "mgr.private_key",
    "mgr.have_private_key",
    "mgr.redeem_script",
    "mgr.sync_account_to_addr_index",
    "mgr.record_derived_address",
    "mgr.mark_used",
    "mgr.mark_used_child_index",
    "mgr.mark_returned_child_index",
    "mgr.for_each_account_address",
    "mgr.for_each_active_account_address",
    "mgr.for_each_active_address",
    "rt.reopen",
    "rt.clock_set",
    "rt.clock_advance",
    "rt.checkpoint",
    "update",
    "view",
    "tx.build",
    "tx.raw",
    "store.main_chain_tip",
    "store.extend_main_chain",
    "store.processed_txs_block_marker",
    "store.update_processed_txs_block_marker",
    "udb.set_birth_state",
    "udb.birth_state",
    "store.is_missing_main_chain_cfilters",
    "store.missing_cfilters_height",
    "store.insert_missing_cfilters",
    "store.get_main_chain_cfilters",
    "store.cfilter_v2",
    "store.import_cfilters_v2",
    "udb.extract_block_header",
    "store.get_main_chain_block_hash_for_height",
    "store.get_serialized_block_header",
    "store.get_block_header_time",
    "store.get_block_header",
    "store.block_in_main_chain",
    "store.get_block_meta_for_hash",
    "store.get_main_chain_block_hashes",
    "store.rollback",
    "store.insert_mined_tx",
    "store.insert_mem_pool_tx",
    "store.set_published",
    "store.remove_unconfirmed",
    "store.prune_unmined",
    "store.add_credit",
    "store.account_balance",
    "store.account_balances",
    "store.unspent_tickets",
    "store.exists_utxo",
    "store.is_unspent_outpoint",
    "store.unspent_outputs",
    "store.unspent_output",
    "store.for_each_unspent_outpoint",
    "store.previous_pk_scripts",
    "store.unspent_output_count",
    "store.own_ticket",
    "store.iterate_tickets",
    "store.tx_details",
    "store.unique_tx_details",
    "store.ticket_details",
    "store.tx",
    "store.exists_tx",
    "store.exists_tx_mined_or_unmined",
    "store.tx_block_height",
    "store.range_transactions",
    "store.range_blocks",
    "store.unmined_txs",
    "store.unmined_tx_hashes",
    "store.add_ticket_commitment",
    "store.redeem_ticket_commitments",
    "store.add_multisig_out",
    "store.spend_multisig_out",
    "store.get_multisig_output",
    "store.unspent_multisig_credits_for_address",
    "store.random_utxo",
    "store.select_inputs",
    "store.spender",
    "udb.set_vsp_ticket",
    "udb.delete_vsp_ticket",
    "udb.get_vsp_ticket",
    "udb.get_vsp_tickets_by_fee_status",
    "udb.get_vsp_host",
    "udb.get_vsp_pub_key",
    "udb.set_tspend_policy",
    "udb.set_vsp_tspend_policy",
    "udb.tspend_policy",
    "udb.vsp_tspend_policy",
    "udb.tspend_policies",
    "udb.vsp_tspend_policies",
    "udb.set_treasury_key_policy",
    "udb.set_vsp_treasury_key_policy",
    "udb.treasury_key_policy",
    "udb.vsp_treasury_key_policy",
    "udb.treasury_key_policies",
    "udb.vsp_treasury_key_policies",
    "udb.set_default_agenda_preference",
    "udb.set_ticket_agenda_preference",
    "udb.default_agenda_preference",
    "udb.ticket_agenda_preference",
];

/// The `@exporter` meta of this runner's exports.
pub const EXPORTER: &str = "crates/purse-udb/tests/support/runner.rs";

/// The stem of Go's run database (`script.go`, `newRunner`: `dbPath` is
/// `<dir>/wallet.db`), the one checkpoint name Go refuses by its file
/// layout: its snapshot `<name>.db` would be the database itself.
pub const GO_DATABASE_STEM: &str = "wallet";

/// What a run produced.
#[derive(Debug)]
pub struct Run {
    /// One record per top-level operation.
    pub results: Vec<Value>,
    /// The final logical export.
    pub export: String,
    /// The logical export of each `rt.checkpoint`, in script order: its
    /// name and its text (also written as `<name>.pe` in the run's
    /// directory, as Go's runner writes it).
    pub checkpoints: Vec<(String, String)>,
}

/// Go's kind names, by `purse_errors::Kind` code.
const KIND_NAMES: [&str; 21] = [
    "Other",
    "Bug",
    "Invalid",
    "Permission",
    "IO",
    "Exist",
    "NotExist",
    "Encoding",
    "Crypto",
    "Locked",
    "Passphrase",
    "Seed",
    "WatchingOnly",
    "InsufficientBalance",
    "ScriptFailure",
    "Policy",
    "Consensus",
    "DoubleSpend",
    "Protocol",
    "NoPeers",
    "Deployment",
];

/// The canonical encoding of an error (ADR-0002 section 3): its text with
/// the daemon's separator and the kind `errors.As` finds.
pub fn error_value(err: &AnyError) -> Value {
    let kind = err
        .as_kind()
        .map_or("Other", |k| KIND_NAMES[usize::from(k.code())]);
    json!({"kind": kind, "text": err.render(purse_errors::DAEMON_SEPARATOR)})
}

/// The runner's state.
struct Runner {
    path: PathBuf,
    store: Option<Store>,
    mgr: Option<Manager>,
    chain: ChainParams,
    pub_pass: Vec<u8>,
    /// The private passphrase the export's `[privcheck]` uses: none for a
    /// wallet created from an extended key, as Go's runner passes none.
    priv_pass: Option<Vec<u8>>,
    /// The passphrases of accounts with their own, by account (Go's
    /// `runner.acctPass`): set by a successful `SetAccountPassphrase` (an
    /// empty one removes the entry) or `ImportVotingAccount`, cleared by a
    /// conversion; the export's `[privcheck]` unlocks those accounts with
    /// them.
    acct_pass: BTreeMap<u32, Vec<u8>>,
    /// `setup.ncpu`: the processor count new Argon2id parameters record.
    ncpu: usize,
    rng: SplitMix64,
    /// The bytes the manager's sources have handed out (`drawn`).
    drawn: Arc<AtomicU64>,
    clock: i64,
    /// The addresses of labelled imports (Go's `runner.addrs`).
    addrs: HashMap<String, Address>,
    /// The transaction store.
    txstore: TxStore,
    /// The network's parameters (the headers' proof-of-work bits).
    params: dcroxide_chaincfg::Params,
    /// Records made by `tx.build` and `tx.raw`, by label (Go's
    /// `runner.txs`).
    txs: HashMap<String, TxRecord>,
    /// Blocks connected by `store.extend_main_chain`, by label (Go's
    /// `runner.blocks`).
    blocks: HashMap<String, purse_store::Hash32>,
    /// `setup.chain_time` (default `setup.clock`): the time of height 0 in
    /// built headers.
    chain_time: i64,
    /// The number of headers built so far (Go's `runner.nonce`).
    nonce: u32,
    /// The export's chain mode (`export.chain` of the script).
    digest: bool,
    /// The members' records of the group the last operation was.
    group_ops: Option<Vec<Value>>,
    /// The stream the transaction store's random selections draw from:
    /// dcrd's generator keyed by the last `rand` argument, as Go's
    /// injected stream restarts under it, with the value of `drawn` after
    /// the last random store operation (`stream_mark`). The address
    /// manager draws from a stream of its own, so the runner continues
    /// this one only while nothing else has drawn (`txops.rs`).
    stream: Option<dcroxide_crypto::rand::Prng>,
    stream_mark: u64,
    /// Whether an `update` group's member is running.
    in_group: bool,
    /// The exports of the checkpoints so far, by name (`Run::checkpoints`).
    checkpoints: Vec<(String, String)>,
}

/// An address argument before its resolution: ready, or the P2PKH address
/// of a derived key, which needs the operation's transaction.
enum AddrRef {
    Ready(Address),
    Derived { acct: u32, branch: u32, index: u32 },
}

/// Go's `managedAddressValue` (`tools/oracle/cmd/oprunner/refs.go`).
fn managed_value(m: &ManagedAddress) -> Value {
    let mut v = json!({
        "addr": m.address().encode(),
        "account": m.account(),
        "addrhash": hex(&m.addr_hash()),
        "imported": m.imported(),
        "internal": m.internal(),
        "multisig": m.multisig(),
    });
    match m {
        ManagedAddress::PubKey { pub_key, index, .. } => {
            v["kind"] = json!("pubkey");
            v["pubkey"] = json!(hex(pub_key));
            v["index"] = json!(index);
        }
        ManagedAddress::Script { script, .. } => {
            v["kind"] = json!("script");
            v["scriptversion"] = json!(0);
            v["script"] = json!(hex(script));
        }
    }
    v
}

/// Go's `resolveErr`: `<what>: <entry>: <udb's text>`, keeping udb's kind.
fn resolve_err(what: &str, entry: &str, err: AnyError) -> AnyError {
    let text = format!(
        "{what}: {entry}: {}",
        err.render(purse_errors::DAEMON_SEPARATOR)
    );
    AnyError::wrap(text, [err])
}

/// A seeded random source for the manager, adding the bytes it hands out
/// to `drawn`.
fn source(rng: &mut SplitMix64, drawn: &Arc<AtomicU64>) -> BoxedRandom {
    let mut r = SplitMix64(rng.next_u64());
    let drawn = Arc::clone(drawn);
    Box::new(move |buf: &mut [u8]| -> Result<(), RandomSourceError> {
        r.fill(buf);
        drawn.fetch_add(
            u64::try_from(buf.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        Ok(())
    })
}

/// An error of the runner's own: a script fault (ADR-0002 section 2) or a
/// store failure, never an operation result.
fn fault(msg: impl Into<String>) -> String {
    msg.into()
}

/// The arguments of one operation, checked against what it reads.
struct Args<'a> {
    op: &'a str,
    map: &'a Map<String, Value>,
}

impl Args<'_> {
    fn check(&self, allowed: &[&str]) -> Result<(), String> {
        for k in self.map.keys() {
            if k != "op" && !allowed.contains(&k.as_str()) {
                return Err(fault(format!("{}: unknown argument {k:?}", self.op)));
            }
        }
        Ok(())
    }

    fn value(&self, name: &str) -> Result<&Value, String> {
        self.map
            .get(name)
            .ok_or_else(|| fault(format!("{}: missing argument {name:?}", self.op)))
    }

    fn str(&self, name: &str) -> Result<&str, String> {
        self.value(name)?
            .as_str()
            .ok_or_else(|| fault(format!("{}: {name} must be a string", self.op)))
    }

    fn u32(&self, name: &str) -> Result<u32, String> {
        self.value(name)?
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| fault(format!("{}: {name} must be a uint32", self.op)))
    }

    fn i64(&self, name: &str) -> Result<i64, String> {
        self.value(name)?
            .as_i64()
            .ok_or_else(|| fault(format!("{}: {name} must be an integer", self.op)))
    }

    fn hex(&self, name: &str) -> Result<Vec<u8>, String> {
        let s = self.str(name)?;
        if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(fault(format!("{}: {name} must be hex", self.op)));
        }
        Ok(unhex(s))
    }

    fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
}

/// Go's `keySpec`: an encoded extended key, or `{"seed", "path"}` (a
/// master key from the seed, then each child, `'` marking a hardened
/// index); neutered unless `private`.
fn key_spec(v: &Value, chain: &ChainParams, private: bool) -> Result<ExtendedKey, String> {
    let key = if let Some(s) = v.as_str() {
        ExtendedKey::new_key_from_string(s, &chain.hd)
            .map_err(|e| fault(format!("extended key: {e}")))?
    } else {
        let o = v.as_object().ok_or_else(|| fault("bad key spec"))?;
        for k in o.keys() {
            if k != "seed" && k != "path" {
                return Err(fault(format!("bad key spec: unknown field {k:?}")));
            }
        }
        let seed = o
            .get("seed")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("key spec seed is not hex"))?;
        let mut k = ExtendedKey::new_master(&unhex(seed), &chain.hd)
            .map_err(|e| fault(format!("key spec: NewMaster: {e}")))?;
        for p in o
            .get("path")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let p = p.as_str().ok_or_else(|| fault("key spec path element"))?;
            let hard = p.ends_with('\'');
            let n: u32 = p
                .trim_end_matches('\'')
                .parse()
                .ok()
                .filter(|n| *n < 0x8000_0000)
                .ok_or_else(|| fault(format!("key spec path element {p:?}")))?;
            let idx = if hard { n | 0x8000_0000 } else { n };
            k = k
                .child(idx)
                .map_err(|e| fault(format!("key spec child {p:?}: {e}")))?;
        }
        k
    };
    Ok(if private { key } else { key.neuter() })
}

type OpResult = Result<Value, AnyError>;

impl Runner {
    fn store(&self) -> &Store {
        self.store.as_ref().expect("store open")
    }

    fn mgr(&self) -> &Manager {
        self.mgr.as_ref().expect("manager open")
    }

    /// Runs `f` on a read transaction, as a Go caller's `walletdb.View`.
    fn view<T>(
        &self,
        f: impl FnOnce(&mut purse_store::ReadTx) -> Result<T, AnyError>,
    ) -> Result<Result<T, AnyError>, String> {
        let mut tx = self
            .store()
            .read()
            .map_err(|e| fault(format!("read: {e}")))?;
        let r = f(&mut tx);
        tx.close().map_err(|e| fault(format!("close: {e}")))?;
        Ok(r)
    }

    /// Runs `f` in a write transaction, committed when `f` succeeds and
    /// rolled back when it fails, as Go's `walletdb.Update`; inside an
    /// `update` group, committed either way, as Go's group transaction
    /// keeps what a member wrote before its error (`groups.rs`).
    fn update<T>(
        &self,
        f: impl FnOnce(&mut WriteTx<'_>) -> Result<T, AnyError>,
    ) -> Result<Result<T, AnyError>, String> {
        if self.in_group {
            let r: Result<Result<T, AnyError>, purse_store::Error> =
                self.store().update(|tx| Ok(f(tx)));
            return r.map_err(|e| fault(format!("update: {e}")));
        }
        let r: Result<T, RunErr> = self.store().update(|tx| f(tx).map_err(RunErr::Udb));
        match r {
            Ok(v) => Ok(Ok(v)),
            Err(RunErr::Udb(e)) => Ok(Err(e)),
            Err(RunErr::Store(e)) => Err(fault(format!("update: {e}"))),
        }
    }

    /// Go's `address`: an address argument, as far as it resolves without
    /// a transaction.
    fn addr_ref(&self, a: &Args<'_>, key: &str) -> Result<AddrRef, String> {
        let what = format!("{} {key}", a.op);
        let v = a.value(key)?;
        if let Some(s) = v.as_str() {
            return stdaddr::decode_address(s, &self.chain)
                .map(AddrRef::Ready)
                .map_err(|e| fault(format!("{what}: {e}")));
        }
        let o = v
            .as_object()
            .ok_or_else(|| fault(format!("{what}: bad address reference {v}")))?;
        for k in o.keys() {
            if !["ref", "acct", "branch", "index", "p2sh"].contains(&k.as_str()) {
                return Err(fault(format!("{what}: bad address reference: {k:?}")));
            }
        }
        let u = |k: &str| -> Option<u32> { o.get(k)?.as_u64().and_then(|v| u32::try_from(v).ok()) };
        if let Some(r) = o.get("ref") {
            let r = r.as_str().ok_or_else(|| fault(format!("{what}: ref")))?;
            return self
                .addrs
                .get(r)
                .cloned()
                .map(AddrRef::Ready)
                .ok_or_else(|| fault(format!("{what}: unknown address label {r:?}")));
        }
        if let (Some(acct), Some(branch), Some(index)) = (u("acct"), u("branch"), u("index")) {
            return Ok(AddrRef::Derived {
                acct,
                branch,
                index,
            });
        }
        if let Some(p) = o.get("p2sh").and_then(Value::as_str) {
            if p.len() % 2 != 0 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(fault(format!("{what}: p2sh is not hex")));
            }
            return stdaddr::new_address_script_hash_v0(&unhex(p), &self.chain)
                .map(AddrRef::Ready)
                .map_err(|e| fault(format!("{what}: {e}")));
        }
        Err(fault(format!("{what}: bad address reference {v}")))
    }

    /// Resolves an address argument in the operation's transaction.
    fn resolve<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        r: AddrRef,
        what: &str,
    ) -> Result<Address, AnyError> {
        match r {
            AddrRef::Ready(a) => Ok(a),
            AddrRef::Derived {
                acct,
                branch,
                index,
            } => {
                let xpub = self
                    .mgr()
                    .account_branch_extended_pub_key(tx, acct, branch)
                    .map_err(|e| resolve_err(what, "AccountBranchExtendedPubKey", e))?;
                let child = xpub.child(index).map_err(|e| {
                    resolve_err(what, &format!("Child({index})"), AnyError::foreign(e))
                })?;
                stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                    &hash160(child.serialized_pub_key()),
                    &self.chain,
                )
                .map_err(AnyError::foreign)
            }
        }
    }

    /// Go's `addrLabel`: an import's optional label, checked before the
    /// call.
    fn label(&self, a: &Args<'_>) -> Result<Option<String>, String> {
        if !a.has("label") {
            return Ok(None);
        }
        let l = a.str("label")?.to_owned();
        if l.is_empty() {
            return Ok(None);
        }
        if self.addrs.contains_key(&l) {
            return Err(fault(format!("{}: address label {l:?} already used", a.op)));
        }
        Ok(Some(l))
    }

    fn reopen(&mut self) -> Result<OpResult, String> {
        drop(self.mgr.take());
        if let Some(s) = self.store.take() {
            s.close().map_err(|e| fault(format!("close store: {e}")))?;
        }
        self.store = Some(Store::open(&self.path).map_err(|e| fault(format!("open store: {e}")))?);
        let rng = source(&mut self.rng, &self.drawn);
        let opened =
            self.view(|tx| Manager::open(tx, &self.chain, &self.pub_pass, rng, Limits::DEFAULT))?;
        Ok(opened.map(|m| {
            self.mgr = Some(m.with_num_cpu(self.ncpu));
            Value::Null
        }))
    }

    fn op(&mut self, raw: &Value) -> Result<(String, OpResult), String> {
        let map = raw
            .as_object()
            .ok_or_else(|| fault("an operation must be an object"))?;
        let name = map
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| fault("an operation needs an op name"))?
            .to_owned();
        let a = Args { op: &name, map };
        // Go's address manager, runtime and wallet operations may draw from
        // the injected stream; the runner's draw from another, so the store's
        // stream no longer stands where Go's does.
        if ["mgr.", "rt.", "wallet."]
            .iter()
            .any(|p| name.starts_with(p))
        {
            self.stream = None;
        }
        let account_arg = |a: &Args<'_>| -> Result<u32, String> {
            a.check(&["account"])?;
            a.u32("account")
        };
        let ok_null = |r: Result<(), AnyError>| r.map(|()| Value::Null);
        let key = |r: Result<ExtendedKey, AnyError>| r.map(|k| json!(k.string().as_str()));
        let result: OpResult = match name.as_str() {
            "udb.coin_types" => {
                a.check(&[])?;
                let (legacy, slip) = addrmgr::coin_types(&self.chain);
                Ok(json!({"legacy": legacy, "slip0044": slip}))
            }
            "udb.hd_keys_from_seed" => {
                a.check(&["seed"])?;
                let seed = a.hex("seed")?;
                addrmgr::hd_keys_from_seed(&seed, &self.chain).map(|k| {
                    json!({
                        "cointype_legacy": k.coin_type_legacy.string().as_str(),
                        "cointype_slip0044": k.coin_type_slip0044.string().as_str(),
                        "account0_legacy": k.account0_legacy.string().as_str(),
                        "account0_slip0044": k.account0_slip0044.string().as_str(),
                    })
                })
            }
            "udb.validate_account_name" => {
                a.check(&["name"])?;
                ok_null(addrmgr::validate_account_name(a.str("name")?.as_bytes()))
            }
            "udb.is_imported_voting" => {
                a.check(&["type"])?;
                let t = a
                    .u32("type")
                    .ok()
                    .and_then(|t| u8::try_from(t).ok())
                    .ok_or_else(|| fault("udb.is_imported_voting: type must be 0..255"))?;
                Ok(json!(addrmgr::is_imported_voting(t)))
            }
            "mgr.watching_only" => {
                a.check(&[])?;
                Ok(json!(self.mgr().watching_only()))
            }
            "mgr.is_locked" => {
                a.check(&[])?;
                Ok(json!(self.mgr().is_locked()))
            }
            "mgr.chain_params" => {
                a.check(&[])?;
                Ok(json!(self.mgr().chain_params().name))
            }
            "mgr.lock" => {
                a.check(&[])?;
                ok_null(self.mgr().lock())
            }
            "mgr.unlock" => {
                a.check(&["pass"])?;
                let p = a.str("pass")?.as_bytes().to_vec();
                // Go runs it in a View, for the bucket it never reads.
                self.view(|_| self.mgr().unlock(&p))?.map(|()| Value::Null)
            }
            "mgr.convert_to_watching_only" => {
                a.check(&[])?;
                // The one method that commits its own transaction (it
                // changes the manager's memory only after the commit). As
                // Go's runner, a converted wallet's exports get no private
                // passphrase.
                let r = self.mgr().convert_to_watching_only(self.store());
                if r.is_ok() {
                    self.priv_pass = None;
                    self.acct_pass.clear();
                }
                ok_null(r)
            }
            "mgr.change_passphrase" => {
                a.check(&["old", "new", "private"])?;
                let old = a.str("old")?.as_bytes().to_vec();
                let new = a.str("new")?.as_bytes().to_vec();
                let private = a
                    .value("private")?
                    .as_bool()
                    .ok_or_else(|| fault(format!("{}: private must be a boolean", a.op)))?;
                // Commits its own transaction, as a conversion does. As
                // Go's runner, a successful change replaces the passphrase
                // later reopens and the export use.
                let r = self
                    .mgr()
                    .change_passphrase(self.store(), &old, &new, private);
                if r.is_ok() {
                    if private {
                        self.priv_pass = Some(new);
                    } else {
                        self.pub_pass = new;
                    }
                }
                ok_null(r)
            }
            "mgr.unlock_account" => {
                a.check(&["account", "pass"])?;
                let (acct, p) = (a.u32("account")?, a.str("pass")?.as_bytes().to_vec());
                ok_null(self.view(|tx| self.mgr().unlock_account(tx, acct, &p))?)
            }
            "mgr.lock_account" => {
                let acct = account_arg(&a)?;
                ok_null(self.view(|tx| self.mgr().lock_account(tx, acct))?)
            }
            "mgr.set_account_passphrase" => {
                a.check(&["account", "pass"])?;
                let (acct, p) = (a.u32("account")?, a.str("pass")?.as_bytes().to_vec());
                let r = self.update(|tx| self.mgr().set_account_passphrase(tx, acct, &p))?;
                if r.is_ok() {
                    if p.is_empty() {
                        self.acct_pass.remove(&acct);
                    } else {
                        self.acct_pass.insert(acct, p);
                    }
                }
                ok_null(r)
            }
            "mgr.unlocked_with_passphrase" => {
                a.check(&["pass"])?;
                ok_null(
                    self.mgr()
                        .unlocked_with_passphrase(a.str("pass")?.as_bytes()),
                )
            }
            "mgr.coin_type" => {
                a.check(&[])?;
                self.view(|tx| self.mgr().coin_type(tx))?.map(|v| json!(v))
            }
            "mgr.coin_type_priv_key" => {
                a.check(&[])?;
                key(self.view(|tx| self.mgr().coin_type_priv_key(tx))?)
            }
            "mgr.upgrade_to_slip0044_coin_type" => {
                a.check(&[])?;
                // `null`, as for Go's call, unless account 0's own
                // passphrase was dropped (QK-0767): `oprunner` reports that
                // from Go's state before the call.
                self.update(|tx| self.mgr().upgrade_to_slip0044_coin_type(tx))?
                    .map(|dropped| {
                        if dropped {
                            json!({"passphrase_dropped": true})
                        } else {
                            Value::Null
                        }
                    })
            }
            "mgr.encrypt" | "mgr.decrypt" => {
                a.check(&["key", "data"])?;
                let private = match a.str("key")? {
                    "private" => true,
                    "public" => false,
                    _ => return Err(fault(format!("{name}: key must be private or public"))),
                };
                let data = a.hex("data")?;
                let m = self.mgr();
                if name == "mgr.encrypt" {
                    let r = if private {
                        m.encrypt_private(&data)
                    } else {
                        m.encrypt_public(&data)
                    };
                    r.map(|s| json!({"sealed~r": hex(&s)}))
                } else {
                    let r = if private {
                        m.decrypt_private(&data)
                    } else {
                        m.decrypt_public(&data)
                    };
                    r.map(|p| json!(hex(p.as_bytes())))
                }
            }
            "mgr.last_account" => {
                a.check(&[])?;
                self.view(|tx| self.mgr().last_account(tx))?
                    .map(|v| json!(v))
            }
            "mgr.last_imported_account" => {
                a.check(&[])?;
                self.view(|tx| self.mgr().last_imported_account(tx))?
                    .map(|v| json!(v))
            }
            "mgr.account_name" => {
                let acct = account_arg(&a)?;
                self.view(|tx| self.mgr().account_name(tx, acct))?
                    .map(|n| json!(String::from_utf8_lossy(&n)))
            }
            "mgr.for_each_account" => {
                a.check(&[])?;
                self.view(|tx| self.mgr().for_each_account(tx))?
                    .map(|v| json!(v))
            }
            "mgr.account_properties" => {
                let acct = account_arg(&a)?;
                self.view(|tx| self.mgr().account_properties(tx, acct))?
                    .map(|p| {
                        json!({
                            "account": p.account_number,
                            "name": String::from_utf8_lossy(&p.account_name),
                            "type": p.account_type,
                            "extused": p.last_used_external_index,
                            "intused": p.last_used_internal_index,
                            "extret": p.last_returned_external_index,
                            "intret": p.last_returned_internal_index,
                            "importedkeys": p.imported_key_count,
                            "encrypted": p.account_encrypted,
                            "unlocked": p.account_unlocked,
                        })
                    })
            }
            "mgr.account_has_passphrase" => {
                let acct = account_arg(&a)?;
                self.view(|tx| Ok(self.mgr().account_has_passphrase(tx, acct)))?
                    .map(|(has, unlocked)| json!({"has": has, "unlocked": unlocked}))
            }
            "mgr.account_extended_pub_key" => {
                let acct = account_arg(&a)?;
                key(self.view(|tx| self.mgr().account_extended_pub_key(tx, acct))?)
            }
            "mgr.account_extended_priv_key" => {
                let acct = account_arg(&a)?;
                key(self.view(|tx| self.mgr().account_extended_priv_key(tx, acct))?)
            }
            "mgr.account_branch_extended_pub_key" => {
                a.check(&["account", "branch"])?;
                let (acct, branch) = (a.u32("account")?, a.u32("branch")?);
                key(self.view(|tx| self.mgr().account_branch_extended_pub_key(tx, acct, branch))?)
            }
            "mgr.lookup_account" => {
                a.check(&["name"])?;
                let name = a.str("name")?.as_bytes().to_vec();
                self.view(|tx| self.mgr().lookup_account(tx, &name))?
                    .map(|v| json!(v))
            }
            "mgr.new_account" => {
                a.check(&["name"])?;
                let name = a.str("name")?.as_bytes().to_vec();
                self.update(|tx| self.mgr().new_account(tx, &name))?
                    .map(|v| json!(v))
            }
            "mgr.rename_account" => {
                a.check(&["account", "name"])?;
                let (acct, name) = (a.u32("account")?, a.str("name")?.as_bytes().to_vec());
                ok_null(self.update(|tx| self.mgr().rename_account(tx, acct, &name))?)
            }
            "mgr.import_xpub_account" => {
                a.check(&["name", "xpub"])?;
                let name = a.str("name")?.as_bytes().to_vec();
                let xpub = key_spec(a.value("xpub")?, &self.chain, false)?;
                ok_null(self.update(|tx| self.mgr().import_xpub_account(tx, &name, &xpub))?)
            }
            "mgr.import_voting_account" => {
                a.check(&["name", "pass", "xpriv"])?;
                let name = a.str("name")?.as_bytes().to_vec();
                let p = a.str("pass")?.as_bytes().to_vec();
                let key = key_spec(a.value("xpriv")?, &self.chain, true)?;
                let r = self.update(|tx| self.mgr().import_voting_account(tx, &key, &p, &name))?;
                if let Ok(acct) = r
                    && !p.is_empty()
                {
                    self.acct_pass.insert(acct, p);
                }
                r.map(|v| json!(v))
            }
            "mgr.address" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                self.view(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().address(tx, &addr)
                })?
                .map(|m| managed_value(&m))
            }
            "mgr.addr_account" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                self.view(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().addr_account(tx, &addr)
                })?
                .map(|v| json!(v))
            }
            "mgr.exists_hash160" => {
                a.check(&["hash160"])?;
                let h = a.hex("hash160")?;
                self.view(|tx| self.mgr().exists_hash160(tx, &h))?
                    .map(|v| json!(v))
            }
            "mgr.import_private_key" => {
                a.check(&["key", "label"])?;
                let key = a.hex("key")?;
                if key.len() != 32 {
                    return Err(fault(format!("{name}: key must be 32 bytes")));
                }
                let wif = Wif::new(
                    &key,
                    self.chain.ids.private_key,
                    SignatureType::ECDSA_SECP256K1,
                )
                .map_err(|e| fault(format!("{name}: NewWIF: {e}")))?;
                let label = self.label(&a)?;
                let r = self.update(|tx| self.mgr().import_private_key(tx, &wif))?;
                self.labelled(label, r)
            }
            "mgr.import_public_key" => {
                a.check(&["pubkey", "label"])?;
                let pk = a.hex("pubkey")?;
                let label = self.label(&a)?;
                let r = self.update(|tx| self.mgr().import_public_key(tx, &pk))?;
                self.labelled(label, r)
            }
            "mgr.import_script" => {
                a.check(&["script", "label"])?;
                let script = a.hex("script")?;
                let label = self.label(&a)?;
                let r = self.update(|tx| self.mgr().import_script(tx, &script))?;
                self.labelled(label, r)
            }
            "mgr.private_key" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                self.view(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().private_key(tx, &addr)
                })?
                .map(|k| {
                    json!({
                        "key": hex(&k.serialize()[..]),
                        "pubkey": hex(&k.public_point().serialize_compressed()),
                    })
                })
            }
            "mgr.have_private_key" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                self.view(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().have_private_key(tx, &addr)
                })?
                .map(|v| json!(v))
            }
            "mgr.redeem_script" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                self.view(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().redeem_script(tx, &addr)
                })?
                .map(|s| json!(hex(&s)))
            }
            "mgr.sync_account_to_addr_index" => {
                a.check(&["account", "index", "branch"])?;
                let (acct, idx, branch) = (a.u32("account")?, a.u32("index")?, a.u32("branch")?);
                ok_null(
                    self.update(|tx| self.mgr().sync_account_to_addr_index(tx, acct, idx, branch))?,
                )
            }
            "mgr.record_derived_address" => {
                a.check(&["account", "branch", "child", "pubkey"])?;
                let (acct, branch, child) = (a.u32("account")?, a.u32("branch")?, a.u32("child")?);
                let pk = if a.has("pubkey") {
                    Some(a.hex("pubkey")?)
                } else {
                    None
                };
                ok_null(self.update(|tx| {
                    let pk = match pk {
                        Some(pk) => pk,
                        // The public key of the derived child, as Go's
                        // runner derives it in the same transaction.
                        None => {
                            let xpub = self
                                .mgr()
                                .account_branch_extended_pub_key(tx, acct, branch)?;
                            let k = xpub.child(child).map_err(AnyError::foreign)?;
                            k.serialized_pub_key().to_vec()
                        }
                    };
                    self.mgr()
                        .record_derived_address(tx, acct, branch, child, &pk)
                })?)
            }
            "mgr.mark_used" => {
                a.check(&["addr"])?;
                let r = self.addr_ref(&a, "addr")?;
                let what = format!("{name} addr");
                ok_null(self.update(|tx| {
                    let addr = self.resolve(tx, r, &what)?;
                    self.mgr().mark_used(tx, &addr)
                })?)
            }
            "mgr.mark_used_child_index" | "mgr.mark_returned_child_index" => {
                a.check(&["account", "branch", "child"])?;
                let (acct, branch, child) = (a.u32("account")?, a.u32("branch")?, a.u32("child")?);
                let used = name == "mgr.mark_used_child_index";
                ok_null(self.update(|tx| {
                    if used {
                        self.mgr().mark_used_child_index(tx, acct, branch, child)
                    } else {
                        self.mgr()
                            .mark_returned_child_index(tx, acct, branch, child)
                    }
                })?)
            }
            "mgr.for_each_account_address" | "mgr.for_each_active_account_address" => {
                let acct = account_arg(&a)?;
                let active = name == "mgr.for_each_active_account_address";
                self.view(|tx| {
                    let mut out = Vec::new();
                    let f = |m: ManagedAddress| {
                        out.push(managed_value(&m));
                        Ok(())
                    };
                    if active {
                        self.mgr().for_each_active_account_address(tx, acct, f)?;
                    } else {
                        self.mgr().for_each_account_address(tx, acct, f)?;
                    }
                    Ok(Value::Array(out))
                })?
            }
            "mgr.for_each_active_address" => {
                a.check(&[])?;
                self.view(|tx| {
                    let mut out = Vec::new();
                    self.mgr().for_each_active_address(tx, |addr| {
                        out.push(json!(addr.encode()));
                        Ok(())
                    })?;
                    Ok(Value::Array(out))
                })?
            }
            "rt.reopen" => {
                a.check(&[])?;
                self.reopen()?
            }
            "rt.clock_set" => {
                a.check(&["unix"])?;
                self.clock = a.i64("unix")?;
                Ok(Value::Null)
            }
            "rt.clock_advance" => {
                a.check(&["seconds"])?;
                self.clock = self.clock.saturating_add(a.i64("seconds")?);
                Ok(Value::Null)
            }
            "rt.checkpoint" => {
                a.check(&["name"])?;
                self.checkpoint(a.str("name")?)?
            }
            "update" | "view" => {
                let (r, records) = self.group(&a)?;
                self.group_ops = Some(records);
                r
            }
            other if txops::handles(other) => self.tx_op(&a)?,
            other if auxops::handles(other) => self.aux_op(&a)?,
            other => {
                return Err(fault(format!(
                    "operation {other:?} is not ported to purse-udb yet"
                )));
            }
        };
        Ok((name, result))
    }
}

impl Runner {
    /// The logical export of the store as it stands, with the passphrases
    /// Go's runner would pass (`tools/oracle/cmd/oprunner/script.go`,
    /// `exportNow`): `what` names it in a failure, which is the runner's
    /// fault, never an operation result.
    fn export_now(&self, what: &str) -> Result<String, String> {
        let acct_pass: Vec<(u32, &[u8])> = self
            .acct_pass
            .iter()
            .map(|(a, p)| (*a, p.as_slice()))
            .collect();
        let opts = ExportOptions {
            pub_pass: Some(&self.pub_pass),
            priv_pass: self.priv_pass.as_deref(),
            acct_pass: &acct_pass,
            exporter: EXPORTER,
            limits: Limits::DEFAULT,
            digest: self.digest,
        };
        let tx = self
            .store()
            .read()
            .map_err(|e| fault(format!("read: {e}")))?;
        let text = export(&tx, &self.chain, &opts).map_err(|e| {
            fault(format!(
                "{what}: {}",
                e.render(purse_errors::DAEMON_SEPARATOR)
            ))
        })?;
        drop(tx);
        Ok(text)
    }

    /// Go's `rt.checkpoint` (`script.go`, `top`): the export of the store
    /// as it stands, written as `<name>.pe` beside the store and kept for
    /// [`Run::checkpoints`], under Go's naming rules (not `final`, not
    /// empty, no `/`, `\` or `.`, not repeated; otherwise a script fault)
    /// and one name Go refuses through its file layout rather than its
    /// name check: `wallet`. Go's run keeps its database at
    /// `<dir>/wallet.db` and `exportNow` creates the snapshot
    /// `<dir>/<name>.db` with `O_EXCL`, so a checkpoint named `wallet`
    /// fails Go's run with "file exists" (an I/O error, not a script
    /// fault); this runner's store is `wallet.redb` and it writes no
    /// snapshot, so it refuses the name explicitly. No other file is in
    /// Go's run directory while a script runs (`results.jsonl` is
    /// written after the last operation and is not a `.db`).
    /// The record is Go's ExportSummary (ADR-0002 section 4) computed from
    /// this runner's export, whose digests cover what Go's producer writes
    /// and this one does not; the comparison reads its file name and
    /// section names and compares the exports themselves
    /// (`compare::compare_checkpoints`). Neither side draws: Go's exporter
    /// opens its snapshot with the real generator, never the injected one.
    fn checkpoint(&mut self, name: &str) -> Result<OpResult, String> {
        if name == "final"
            || name.is_empty()
            || name.contains(['/', '\\', '.'])
            || self.checkpoints.iter().any(|(n, _)| n == name)
        {
            return Err(fault(format!("bad or repeated checkpoint name {name:?}")));
        }
        if name == GO_DATABASE_STEM {
            return Err(fault(format!(
                "checkpoint name {name:?}: Go's snapshot {name}.db would be its own database"
            )));
        }
        let text = self.export_now(&format!("logical export {name:?}"))?;
        let file = format!("{name}.pe");
        let dir = self
            .path
            .parent()
            .ok_or_else(|| fault("no run directory"))?;
        std::fs::write(dir.join(&file), &text).map_err(|e| fault(format!("write {file}: {e}")))?;
        let ex = purse_export_check::parse(&text)
            .map_err(|e| fault(format!("{file} does not parse: {e}")))?;
        let sections: Vec<Value> = ex
            .sections
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "records": s.records.len(),
                    "sha256~cr": s.digest(purse_export_check::Profile::EXACT),
                    "msha256~c": s.digest(purse_export_check::Profile::MASKED),
                })
            })
            .collect();
        let file_sha256: String = {
            use sha2::{Digest, Sha256};
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let summary = json!({
            "file": file,
            "sha256~cr": ex.sha256,
            "msha256~c": ex.msha256,
            "file_sha256~cr": file_sha256,
            "lines": text.lines().count(),
            "sections": sections,
        });
        self.checkpoints.push((name.to_owned(), text));
        Ok(Ok(summary))
    }

    /// Go's `labelAddress`: a successful import's address under its label.
    fn labelled(&mut self, label: Option<String>, r: Result<ManagedAddress, AnyError>) -> OpResult {
        r.map(|m| {
            if let Some(l) = label {
                self.addrs.insert(l, m.address().clone());
            }
            managed_value(&m)
        })
    }
}

fn network(name: &str) -> Result<ChainParams, String> {
    ChainParams::by_name(name).ok_or_else(|| fault(format!("unknown network {name:?}")))
}

/// Runs `script` with its store in `dir` (created if missing).
///
/// # Errors
///
/// A script fault, an operation not yet ported, a setup error (as Go, a
/// setup error fails the run) or a store failure.
pub fn run(script: &Value, dir: &Path) -> Result<Run, String> {
    purse_errors::set_separator(purse_errors::DAEMON_SEPARATOR);
    let s = script
        .as_object()
        .ok_or_else(|| fault("a script is an object"))?;
    if s.get("format").and_then(Value::as_str) != Some("purse-opscript 1") {
        return Err(fault("not a purse-opscript 1 script"));
    }
    for k in s.keys() {
        if !["format", "name", "setup", "export", "ops"].contains(&k.as_str()) {
            return Err(fault(format!("unknown script member {k:?}")));
        }
    }
    let setup = s
        .get("setup")
        .and_then(Value::as_object)
        .ok_or_else(|| fault("missing setup"))?;
    for k in setup.keys() {
        if ![
            "network",
            "seed",
            "xpub",
            "pubpass",
            "privpass",
            "rand",
            "clock",
            "ncpu",
            "chain_time",
        ]
        .contains(&k.as_str())
        {
            return Err(fault(format!("setup member {k:?} is not supported yet")));
        }
    }
    let get_str = |k: &str| -> Result<&str, String> {
        setup
            .get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| fault(format!("setup.{k} must be a string")))
    };
    let chain = network(get_str("network")?)?;
    let txstore = TxStore::new(&chain).map_err(|e| fault(format!("TxStore::new: {e:?}")))?;
    let params = match chain.name {
        "mainnet" => dcroxide_chaincfg::mainnet_params(),
        "testnet3" => dcroxide_chaincfg::testnet3_params(),
        "simnet" => dcroxide_chaincfg::simnet_params(),
        _ => dcroxide_chaincfg::regnet_params(),
    };
    // The script's export options: the chain mode is read; the others
    // shape only Go's export and are ignored here, as before.
    let digest = match s.get("export") {
        None => false,
        Some(x) => {
            let o = x
                .as_object()
                .ok_or_else(|| fault("export must be an object"))?;
            match o.get("chain").and_then(Value::as_str) {
                None | Some("full") => false,
                Some("digest") => true,
                Some(m) => return Err(fault(format!("export.chain {m:?}"))),
            }
        }
    };
    // Go's setup: exactly one of a seed (Initialize) and an extended key
    // (InitializeWatchOnly): a key spec, neutered, or {"raw": string}
    // passed unparsed.
    let wallet_key = match (setup.get("seed"), setup.get("xpub")) {
        (Some(_), None) => WalletKey::Seed(unhex(get_str("seed")?)),
        (None, Some(x)) => match x.as_object().and_then(|o| o.get("raw")) {
            Some(raw) if x.as_object().is_some_and(|o| o.len() == 1) => WalletKey::Xpub(
                raw.as_str()
                    .ok_or_else(|| fault("setup.xpub.raw must be a string"))?
                    .as_bytes()
                    .to_vec(),
            ),
            _ => WalletKey::Xpub(key_spec(x, &chain, false)?.string().as_bytes().to_vec()),
        },
        _ => return Err(fault("setup needs exactly one of seed and xpub")),
    };
    let pub_pass = get_str("pubpass")?.as_bytes().to_vec();
    let priv_pass = get_str("privpass")?.as_bytes().to_vec();
    let rand_key = unhex(get_str("rand")?);
    let clock = setup
        .get("clock")
        .and_then(Value::as_i64)
        .ok_or_else(|| fault("setup.clock must be an integer"))?;
    let chain_time = match setup.get("chain_time") {
        None => clock,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| fault("setup.chain_time must be an integer"))?,
    };
    // Go's setup.ncpu: 1 unless given, within 1 to 1024.
    let ncpu = match setup.get("ncpu") {
        None => 1,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=1024).contains(n))
            .ok_or_else(|| fault("setup.ncpu out of range"))?,
    };
    let mut state = 0u64;
    for chunk in rand_key.chunks(8) {
        let mut b = [0u8; 8];
        b[..chunk.len()].copy_from_slice(chunk);
        state ^= u64::from_le_bytes(b);
        state = SplitMix64(state).next_u64();
    }
    let mut rng = SplitMix64(state);

    std::fs::create_dir_all(dir).map_err(|e| fault(format!("create {}: {e}", dir.display())))?;
    let path = dir.join(purse_store::STORE_FILE_NAME);
    let store = Store::create(&path).map_err(|e| fault(format!("create store: {e}")))?;
    let drawn = Arc::new(AtomicU64::new(0));
    let mut create_rng = source(&mut rng, &drawn);
    let created: Result<(), RunErr> = store.update(|tx: &mut WriteTx<'_>| {
        match &wallet_key {
            WalletKey::Seed(seed) => Manager::create(
                tx,
                &chain,
                seed,
                &pub_pass,
                &priv_pass,
                &mut *create_rng,
                &Limits::DEFAULT,
            ),
            WalletKey::Xpub(key) => Manager::create_watch_only(
                tx,
                &chain,
                key,
                &pub_pass,
                &mut *create_rng,
                &Limits::DEFAULT,
            ),
        }
        .map_err(RunErr::Udb)?;
        // Go's Initialize creates the transaction store in the same
        // transaction.
        txstore.create(tx).map_err(RunErr::Udb)
    });
    match created {
        Ok(()) => {}
        Err(RunErr::Udb(e)) => {
            return Err(fault(format!(
                "setup: {}",
                e.render(purse_errors::DAEMON_SEPARATOR)
            )));
        }
        Err(RunErr::Store(e)) => return Err(fault(format!("setup: store: {e}"))),
    }
    let mut runner = Runner {
        path,
        store: Some(store),
        mgr: None,
        chain,
        pub_pass,
        priv_pass: match wallet_key {
            WalletKey::Seed(_) => Some(priv_pass),
            WalletKey::Xpub(_) => None,
        },
        acct_pass: BTreeMap::new(),
        ncpu,
        rng,
        drawn,
        clock,
        addrs: HashMap::new(),
        txstore,
        params,
        txs: HashMap::new(),
        blocks: HashMap::new(),
        chain_time,
        nonce: 0,
        digest,
        group_ops: None,
        in_group: false,
        stream: None,
        stream_mark: 0,
        checkpoints: Vec::new(),
    };
    match runner.reopen()? {
        Ok(_) => {}
        Err(e) => {
            return Err(fault(format!(
                "setup: open: {}",
                e.render(purse_errors::DAEMON_SEPARATOR)
            )));
        }
    }

    let ops = s
        .get("ops")
        .and_then(Value::as_array)
        .ok_or_else(|| fault("missing ops"))?;
    let mut results = Vec::with_capacity(ops.len());
    for (i, raw) in ops.iter().enumerate() {
        let before = runner.drawn.load(Ordering::Relaxed);
        let (name, r) = runner.op(raw).map_err(|e| format!("op {i}: {e}"))?;
        let drawn = runner.drawn.load(Ordering::Relaxed).saturating_sub(before);
        let mut rec = Map::new();
        rec.insert("i".to_owned(), json!(i));
        rec.insert("op".to_owned(), json!(name));
        match r {
            Ok(v) => rec.insert("ok".to_owned(), v),
            Err(e) => rec.insert("err".to_owned(), error_value(&e)),
        };
        if drawn != 0 {
            rec.insert("drawn".to_owned(), json!(drawn));
        }
        if let Some(members) = runner.group_ops.take() {
            rec.insert("ops".to_owned(), Value::Array(members));
        }
        results.push(Value::Object(rec));
    }

    let text = runner.export_now("final export")?;
    drop(runner.mgr.take());
    if let Some(s) = runner.store.take() {
        s.close().map_err(|e| fault(format!("close store: {e}")))?;
    }
    Ok(Run {
        results,
        export: text,
        checkpoints: runner.checkpoints,
    })
}

/// What the setup creates the wallet from.
enum WalletKey {
    /// A seed: `Initialize`.
    Seed(Vec<u8>),
    /// An extended key's string: `InitializeWatchOnly`.
    Xpub(Vec<u8>),
}

/// The error of the creating transaction.
enum RunErr {
    Store(purse_store::Error),
    Udb(AnyError),
}

impl From<purse_store::Error> for RunErr {
    fn from(e: purse_store::Error) -> Self {
        RunErr::Store(e)
    }
}
