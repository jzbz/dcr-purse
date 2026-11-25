// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The transaction store's operations of the Rust runner, with Go's
//! arguments and result shapes (`tools/oracle/cmd/oprunner/ops_store.go`,
//! `build.go`, `refs.go`): the fixtures `tx.build` and `tx.raw`, the header
//! and filter builder of `store.extend_main_chain`, and every transaction
//! store operation ported so far.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counts of output kinds"
)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use dcroxide_crypto::rand::Prng;
use purse_dcrutil::Amount;
use purse_snacl::{RandomSource, RandomSourceError};

use dcroxide_gcs::{FilterV2, blockcf2};
use dcroxide_stake::TxType;
use dcroxide_txscript::stdaddr::Address;
use dcroxide_wire::{
    BlockHeader, MAX_TX_IN_SEQUENCE_NUM, MsgTx, NULL_BLOCK_HEIGHT, NULL_BLOCK_INDEX, OutPoint,
    TxIn, TxOut,
};
use purse_errors::AnyError;
use purse_store::{BirthState, Hash32, HeaderBytes};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{hex, unhex};
use purse_udb::txstore::{
    Balances, Block, BlockMeta, Credit, MultisigOut, TxDetails, TxRead, TxRecord,
};
use sha2::{Digest, Sha256};

use super::{Args, OpResult, Runner, fault, resolve_err};

/// Whether `name` is one of this module's operations.
pub fn handles(name: &str) -> bool {
    super::SUPPORTED.contains(&name)
        && (name.starts_with("store.")
            || name.starts_with("tx.")
            || matches!(
                name,
                "udb.set_birth_state" | "udb.birth_state" | "udb.extract_block_header"
            ))
}

/// A hash in display order (`chainhash.Hash.String`).
pub fn hs(h: &Hash32) -> String {
    let mut r = h.0;
    r.reverse();
    hex(&r)
}

/// A hash from its display form.
fn from_display(s: &str) -> Option<Hash32> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut b: [u8; 32] = unhex(s).try_into().ok()?;
    b.reverse();
    Some(Hash32(b))
}

/// Go's `txTypeName`.
fn type_name(t: TxType) -> &'static str {
    match t {
        TxType::Regular => "regular",
        TxType::SStx => "ticket",
        TxType::SSGen => "vote",
        TxType::SSRtx => "revocation",
        TxType::TAdd => "tadd",
        TxType::TSpend => "tspend",
        TxType::TreasuryBase => "treasurybase",
    }
}

/// Go's `txRecordValue` for a record a query returned (its `SerializedTx`
/// is nil, so the decoded transaction is serialized again).
fn record_value(rec: &TxRecord) -> Value {
    json!({
        "hash": hs(rec.hash()),
        "tx": hex(&rec.msg_tx().serialize()),
        "received~c": rec.received(),
        "type": type_name(rec.tx_type()),
        "unpublished": rec.unpublished(),
    })
}

fn block_value(b: &Block) -> Value {
    json!({"hash": hs(&b.hash), "height": b.height})
}

fn meta_value(b: &BlockMeta) -> Value {
    json!({
        "hash": hs(&b.block.hash), "height": b.block.height,
        "time": b.time, "votebits": b.vote_bits,
    })
}

/// Go's `txDetailsValue`.
fn details_value(d: &TxDetails) -> Value {
    let mut v = record_value(&d.record);
    v["block"] = meta_value(&d.block);
    v["height"] = json!(d.height());
    v["credits"] = Value::Array(
        d.credits
            .iter()
            .map(|c| {
                json!({
                    "index": c.index, "amount": c.amount, "spent": c.spent,
                    "change": c.change, "opcode": c.opcode, "coinbase": c.coinbase,
                    "expiry": c.expiry,
                })
            })
            .collect(),
    );
    v["debits"] = Value::Array(
        d.debits
            .iter()
            .map(|x| json!({"index": x.index, "amount": x.amount}))
            .collect(),
    );
    v
}

/// The transaction store's random stream: dcrd's generator, every byte it
/// hands out added to the runner's `drawn`, as Go counts its stream's.
struct Stream<'s> {
    prng: &'s mut Prng,
    drawn: &'s AtomicU64,
}

impl RandomSource for Stream<'_> {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        self.prng.read(buf);
        self.drawn.fetch_add(
            u64::try_from(buf.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        Ok(())
    }
}

/// Go's `outpointValue`.
fn outpoint_value(op: &OutPoint) -> Value {
    json!({"hash": op.hash.to_string(), "index": op.index, "tree": op.tree})
}

/// Go's `creditValue`.
fn credit_value(c: &Credit) -> Value {
    json!({
        "outpoint": outpoint_value(&c.outpoint),
        "block": meta_value(&c.block),
        "amount": c.amount,
        "pkscript": hex(&c.pk_script),
        "received~c": c.received,
        "coinbase": c.from_coinbase,
        "expiry": c.has_expiry,
    })
}

/// Go's `get_multisig_output` result: the spender only while the output is
/// spent, as `wallet/multisig.go` reads it (QK-0055's stale byte is not
/// kept; ADR-0002 section 4).
fn multisig_value(m: &MultisigOut) -> Value {
    let mut v = json!({
        "outpoint": outpoint_value(&m.outpoint), "tree": m.tree,
        "scripthash": hex(&m.script_hash.0), "m": m.m, "n": m.n,
        "tx": hs(&m.tx_hash), "block": hs(&m.block_hash),
        "height": m.block_height, "amount": m.amount, "spent": m.spender.is_some(),
    });
    if let Some(s) = m.spender {
        v["spentby"] = json!(hs(&s.hash));
        v["spentbyindex"] = json!(s.input);
    }
    v
}

/// Go's `balancesValue`.
fn balances_value(b: &Balances) -> Value {
    json!({
        "account": b.account,
        "immaturecoinbaserewards": b.immature_coinbase_rewards,
        "immaturestakegeneration": b.immature_stake_generation,
        "lockedbytickets": b.locked_by_tickets,
        "spendable": b.spendable,
        "total": b.total,
        "votingauthority": b.voting_authority,
        "unconfirmed": b.unconfirmed,
    })
}

/// Go's `detHash`: SHA-256 of `<kind>/<height>/<counter>`.
fn det_hash(kind: &str, height: u32, counter: u32) -> dcroxide_chainhash::Hash {
    let d = Sha256::digest(format!("{kind}/{height}/{counter}").as_bytes());
    let mut b = [0u8; 32];
    b.copy_from_slice(&d);
    dcroxide_chainhash::Hash(b)
}

/// A block reference: `"tip"`, or a hash or label (Go's `blockHash`).
enum BlockRef {
    Tip,
    Hash(Hash32),
}

/// A block meta argument (Go's `argBlockMeta`).
enum MetaRef {
    None,
    Given(BlockMeta),
    Header(BlockRef),
}

impl Runner {
    /// Go's `hashRef`: 64 hex digits in display order, a transaction label
    /// or a block label.
    fn hash_ref(&self, v: &Value, what: &str) -> Result<Hash32, String> {
        let s = v
            .as_str()
            .ok_or_else(|| fault(format!("{what} must be a string")))?;
        if let Some(h) = from_display(s) {
            return Ok(h);
        }
        if let Some(rec) = self.txs.get(s) {
            return Ok(*rec.hash());
        }
        if let Some(h) = self.blocks.get(s) {
            return Ok(*h);
        }
        Err(fault(format!("{what}: unknown label {s:?}")))
    }

    fn arg_hash(&self, a: &Args<'_>, key: &str) -> Result<Hash32, String> {
        self.hash_ref(a.value(key)?, &format!("{} {key}", a.op))
    }

    fn block_ref(&self, v: &Value, what: &str) -> Result<BlockRef, String> {
        if v.as_str() == Some("tip") {
            return Ok(BlockRef::Tip);
        }
        self.hash_ref(v, what).map(BlockRef::Hash)
    }

    fn arg_block(&self, a: &Args<'_>, key: &str) -> Result<BlockRef, String> {
        self.block_ref(a.value(key)?, &format!("{} {key}", a.op))
    }

    fn resolve_block(
        &self,
        r: &mut (impl TxRead + ?Sized),
        b: &BlockRef,
    ) -> Result<Hash32, AnyError> {
        match b {
            BlockRef::Tip => Ok(self.txstore.main_chain_tip(r)?.0),
            BlockRef::Hash(h) => Ok(*h),
        }
    }

    /// Go's `blockMeta`: the meta of the stored header.
    fn block_meta(
        &self,
        r: &mut (impl TxRead + ?Sized),
        h: &Hash32,
        what: &str,
    ) -> Result<BlockMeta, AnyError> {
        let header = self
            .txstore
            .get_block_header(r, h)
            .map_err(|e| resolve_err(what, "GetBlockHeader", e))?;
        Ok(BlockMeta::from_header(h, &header))
    }

    fn arg_meta(&self, a: &Args<'_>, key: &str) -> Result<MetaRef, String> {
        let Some(v) = a.map.get(key) else {
            return Ok(MetaRef::None);
        };
        if v.is_null() {
            return Ok(MetaRef::None);
        }
        let what = format!("{} {key}", a.op);
        if let Some(o) = v.as_object() {
            let bad = || {
                fault(format!(
                    "{what}: a block meta is {{hash, height, time, votebits}}"
                ))
            };
            if o.len() != 4 {
                return Err(bad());
            }
            let hash = self.hash_ref(o.get("hash").ok_or_else(bad)?, &what)?;
            let height = o
                .get("height")
                .and_then(Value::as_i64)
                .and_then(|h| i32::try_from(h).ok())
                .ok_or_else(bad)?;
            let time = o.get("time").and_then(Value::as_i64).ok_or_else(bad)?;
            let vote_bits = o
                .get("votebits")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(bad)?;
            return Ok(MetaRef::Given(BlockMeta {
                block: Block { hash, height },
                time,
                vote_bits,
            }));
        }
        Ok(MetaRef::Header(self.block_ref(v, &what)?))
    }

    fn resolve_meta(
        &self,
        r: &mut (impl TxRead + ?Sized),
        m: &MetaRef,
        what: &str,
    ) -> Result<Option<BlockMeta>, AnyError> {
        match m {
            MetaRef::None => Ok(None),
            MetaRef::Given(b) => Ok(Some(*b)),
            MetaRef::Header(b) => {
                let h = self.resolve_block(r, b)?;
                self.block_meta(r, &h, what).map(Some)
            }
        }
    }

    /// Go's `argOutpoint`: `{tx, index, tree?}`, `tx` a hash reference.
    fn arg_outpoint(&self, a: &Args<'_>, key: &str) -> Result<OutPoint, String> {
        let what = format!("{} {key}", a.op);
        self.outpoint_ref(a.value(key)?, &what)
    }

    /// Go's `outpoint`: an outpoint reference `{tx, index, tree?}`.
    fn outpoint_ref(&self, v: &Value, what: &str) -> Result<OutPoint, String> {
        let what = what.to_owned();
        let bad = || fault(format!("{what}: an outpoint is {{tx, index, tree?}}"));
        let o = v.as_object().ok_or_else(bad)?;
        if o.keys()
            .any(|k| !["tx", "index", "tree"].contains(&k.as_str()))
        {
            return Err(bad());
        }
        let h = self.hash_ref(o.get("tx").ok_or_else(bad)?, &what)?;
        let index = match o.get("index") {
            None => 0,
            Some(v) => v
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(bad)?,
        };
        let tree = match o.get("tree") {
            None => 0,
            Some(v) => v
                .as_i64()
                .and_then(|v| i8::try_from(v).ok())
                .ok_or_else(bad)?,
        };
        Ok(OutPoint {
            hash: dcroxide_chainhash::Hash(h.0),
            index,
            tree,
        })
    }

    /// The stream a random store operation draws from: dcrd's generator
    /// keyed by its `rand` argument (Go's `streamKey`, which restarts the
    /// injected stream under the key), or, without one, the stream the last
    /// random store operation left, when nothing has drawn since.
    fn take_stream(&mut self, a: &Args<'_>) -> Result<Prng, String> {
        if a.has("rand") {
            let k: [u8; 32] = a
                .hex("rand")?
                .try_into()
                .map_err(|_| fault(format!("{}: rand must be 32 bytes of hex", a.op)))?;
            self.stream = None;
            return Ok(Prng::from_seed(k));
        }
        match self.stream.take() {
            Some(p) if self.drawn.load(Ordering::Relaxed) == self.stream_mark => Ok(p),
            _ => Err(fault(format!(
                "{}: without rand, the Rust runner draws what Go draws only right after \
                 another random store operation, with nothing drawn in between",
                a.op
            ))),
        }
    }

    /// Keeps the stream a random store operation leaves for the next.
    fn put_stream(&mut self, p: Prng) {
        self.stream = Some(p);
        self.stream_mark = self.drawn.load(Ordering::Relaxed);
    }

    fn arg_tx(&self, a: &Args<'_>, key: &str) -> Result<TxRecord, String> {
        let s = a.str(key)?;
        self.txs
            .get(s)
            .cloned()
            .ok_or_else(|| fault(format!("{}: unknown transaction label {s:?}", a.op)))
    }

    fn opt_u32(a: &Args<'_>, key: &str, def: u32) -> Result<u32, String> {
        if a.has(key) { a.u32(key) } else { Ok(def) }
    }

    fn i32_arg(a: &Args<'_>, key: &str) -> Result<i32, String> {
        let v = a.i64(key)?;
        i32::try_from(v).map_err(|_| fault(format!("{}: argument {key:?} out of range", a.op)))
    }

    fn bool_arg(a: &Args<'_>, key: &str) -> Result<bool, String> {
        a.value(key)?
            .as_bool()
            .ok_or_else(|| fault(format!("{}: {key} must be a boolean", a.op)))
    }

    fn opt_bool(a: &Args<'_>, key: &str) -> Result<bool, String> {
        if a.has(key) {
            Self::bool_arg(a, key)
        } else {
            Ok(false)
        }
    }

    fn count_arg(a: &Args<'_>) -> Result<usize, String> {
        let n = a.i64("count")?;
        if !(0..=100_000).contains(&n) {
            return Err(fault(format!("{}: argument \"count\" out of range", a.op)));
        }
        Ok(usize::try_from(n).unwrap_or(0))
    }

    fn hex_list(a: &Args<'_>, key: &str) -> Result<Vec<Vec<u8>>, String> {
        a.value(key)?
            .as_array()
            .ok_or_else(|| fault(format!("{}: {key} must be a list", a.op)))?
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .filter(|s| s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| fault(format!("{}: {key}: not hex", a.op)))?;
                Ok(unhex(s))
            })
            .collect()
    }

    /// Runs one transaction store operation.
    pub(super) fn tx_op(&mut self, a: &Args<'_>) -> Result<OpResult, String> {
        let ok_null = |r: Result<(), AnyError>| r.map(|()| Value::Null);
        let store = self.txstore.clone();
        let st = &store;
        Ok(match a.op {
            "tx.build" => return self.tx_build(a),
            "tx.raw" => {
                a.check(&["label", "tx", "received"])?;
                let label = a.str("label")?.to_owned();
                if self.txs.contains_key(&label) {
                    return Err(fault(format!("tx.raw: label {label:?} already used")));
                }
                let b = a.hex("tx")?;
                let received = if a.has("received") {
                    a.i64("received")?
                } else {
                    self.clock
                };
                TxRecord::new(&b, received).map(|rec| {
                    let v = json!({"hash": hs(rec.hash()), "type": type_name(rec.tx_type())});
                    self.txs.insert(label, rec);
                    v
                })
            }
            "store.main_chain_tip" => {
                a.check(&[])?;
                self.view(|r| st.main_chain_tip(r))?
                    .map(|(h, height)| json!({"hash": hs(&h), "height": height}))
            }
            "store.extend_main_chain" => return self.extend(a),
            "store.processed_txs_block_marker" => {
                a.check(&[])?;
                self.view(|r| st.processed_txs_block_marker(r))?
                    .map(|h| json!(hs(&h)))
            }
            "store.update_processed_txs_block_marker" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                ok_null(self.update(|tx| {
                    let h = self.resolve_block(tx, &b)?;
                    st.update_processed_txs_block_marker(tx, &h)
                })?)
            }
            "udb.set_birth_state" => {
                a.check(&["block", "height", "time", "from_height", "from_time"])?;
                let b = if a.has("block") {
                    Some(self.arg_block(a, "block")?)
                } else {
                    None
                };
                let height = Self::opt_u32(a, "height", 0)?;
                let time = if a.has("time") { a.i64("time")? } else { 0 };
                let from_height = Self::opt_bool(a, "from_height")?;
                let from_time = Self::opt_bool(a, "from_time")?;
                ok_null(self.update(|tx| {
                    let hash = match &b {
                        Some(b) => self.resolve_block(tx, b)?,
                        None => Hash32::default(),
                    };
                    purse_udb::txstore::set_birth_state(
                        tx,
                        &BirthState {
                            set_from_height: from_height,
                            set_from_time: from_time,
                            hash,
                            height,
                            time,
                        },
                    )
                })?)
            }
            "udb.birth_state" => {
                a.check(&[])?;
                self.view(purse_udb::txstore::birth_state)?
                    .map(|b| match b {
                        None => Value::Null,
                        Some(b) => json!({
                            "hash": hs(&b.hash), "height": b.height, "time": b.time,
                            "from_height": b.set_from_height, "from_time": b.set_from_time,
                        }),
                    })
            }
            "store.is_missing_main_chain_cfilters" => {
                a.check(&[])?;
                self.view(|r| st.is_missing_main_chain_cfilters(r))?
                    .map(|v| json!(v))
            }
            "store.missing_cfilters_height" => {
                a.check(&[])?;
                self.view(|r| st.missing_cfilters_height(r))?
                    .map(|v| json!(v))
            }
            "store.insert_missing_cfilters" => {
                a.check(&["blocks"])?;
                let refs = a
                    .value("blocks")?
                    .as_array()
                    .ok_or_else(|| fault(format!("{}: blocks must be a list", a.op)))?
                    .iter()
                    .map(|v| self.block_ref(v, &format!("{} blocks", a.op)))
                    .collect::<Result<Vec<_>, String>>()?;
                let what = format!("{} blocks", a.op);
                ok_null(self.update(|tx| {
                    let mut hashes = Vec::new();
                    let mut filters = Vec::new();
                    for b in &refs {
                        let h = self.resolve_block(tx, b)?;
                        let header = st
                            .get_block_header(tx, &h)
                            .map_err(|e| resolve_err(&what, "GetBlockHeader", e))?;
                        let mut merkle = [0u8; 32];
                        merkle.copy_from_slice(&header.0[36..68]);
                        let key = blockcf2::key(&dcroxide_chainhash::Hash(merkle));
                        let f = FilterV2::new(blockcf2::B, blockcf2::M, key, &[])
                            .map_err(|e| AnyError::new(e.to_string()))?;
                        hashes.push(h);
                        filters.push(f.bytes().to_vec());
                    }
                    st.insert_missing_cfilters(tx, &hashes, &filters)
                })?)
            }
            "store.get_main_chain_cfilters" => {
                a.check(&["start", "inclusive", "count"])?;
                let b = self.arg_block(a, "start")?;
                let incl = Self::bool_arg(a, "inclusive")?;
                let n = Self::count_arg(a)?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_main_chain_cfilters(r, &h, incl, n)
                })?
                .map(|fs| {
                    Value::Array(
                        fs.iter()
                            .map(|f| {
                                json!({"block": hs(&f.hash), "key": hex(&f.key), "filter": hex(&f.filter)})
                            })
                            .collect(),
                    )
                })
            }
            "store.cfilter_v2" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.cfilter_v2(r, &h)
                })?
                .map(|(k, f)| json!({"key": hex(&k), "filter": hex(&f)}))
            }
            "store.import_cfilters_v2" => {
                a.check(&["start_height", "filters"])?;
                let start = Self::i32_arg(a, "start_height")?;
                let data = Self::hex_list(a, "filters")?;
                ok_null(self.update(|tx| st.import_cfilters_v2(tx, start, &data))?)
            }
            "udb.extract_block_header" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_block_header(r, &h)
                })?
                .map(|h: HeaderBytes| {
                    json!({
                        "parent": hex(&purse_udb::txstore::extract_block_header_parent_hash(&h).0),
                        "height": purse_udb::txstore::extract_block_header_height(&h),
                        "time": purse_udb::txstore::extract_block_header_time(&h),
                        "raw_height": purse_udb::txstore::extract_block_header_height(&h),
                    })
                })
            }
            "store.get_main_chain_block_hash_for_height" => {
                a.check(&["height"])?;
                let h = Self::i32_arg(a, "height")?;
                self.view(|r| st.get_main_chain_block_hash_for_height(r, h))?
                    .map(|h| json!(hs(&h)))
            }
            "store.get_serialized_block_header" | "store.get_block_header" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_block_header(r, &h)
                })?
                .map(|h| json!(hex(&h.0)))
            }
            "store.get_block_header_time" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_block_header_time(r, &h)
                })?
                .map(|t| json!(t))
            }
            "store.block_in_main_chain" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.block_in_main_chain(r, &h)
                })?
                .map(|(m, i)| json!({"main": m, "invalidated": i}))
            }
            "store.get_block_meta_for_hash" => {
                a.check(&["block"])?;
                let b = self.arg_block(a, "block")?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_block_meta_for_hash(r, &h)
                })?
                .map(|m| meta_value(&m))
            }
            "store.get_main_chain_block_hashes" => {
                a.check(&["start", "inclusive", "count"])?;
                let b = self.arg_block(a, "start")?;
                let incl = Self::bool_arg(a, "inclusive")?;
                let n = Self::count_arg(a)?;
                self.view(|r| {
                    let h = self.resolve_block(r, &b)?;
                    st.get_main_chain_block_hashes(r, &h, incl, n)
                })?
                .map(|hs_| Value::Array(hs_.iter().map(|h| json!(hs(h))).collect()))
            }
            "store.rollback" => {
                a.check(&["height"])?;
                let h = Self::i32_arg(a, "height")?;
                self.update(|tx| st.rollback(tx, h))?.map(|blocks| {
                    // Go's result is a map: sorted by block hash.
                    let mut v: Vec<(String, Value)> = blocks
                        .iter()
                        .map(|b| {
                            let txs: Vec<Value> = b
                                .txs
                                .iter()
                                .map(|t| json!(t.tx_hash().to_string()))
                                .collect();
                            let h = hs(&b.block.hash);
                            (h.clone(), json!({"block": h, "txs": txs}))
                        })
                        .collect();
                    v.sort_by(|x, y| x.0.cmp(&y.0));
                    Value::Array(v.into_iter().map(|(_, x)| x).collect())
                })
            }
            "store.insert_mined_tx" => {
                a.check(&["tx", "block"])?;
                let rec = self.arg_tx(a, "tx")?;
                let b = self.arg_block(a, "block")?;
                ok_null(self.update(|tx| {
                    let h = self.resolve_block(tx, &b)?;
                    st.insert_mined_tx(tx, &rec, &h)
                })?)
            }
            "store.insert_mem_pool_tx" => {
                a.check(&["tx"])?;
                let rec = self.arg_tx(a, "tx")?;
                ok_null(self.update(|tx| st.insert_mem_pool_tx(tx, &rec))?)
            }
            "store.set_published" => {
                a.check(&["tx", "published"])?;
                let h = self.arg_hash(a, "tx")?;
                let p = Self::bool_arg(a, "published")?;
                ok_null(self.update(|tx| st.set_published(tx, &h, p))?)
            }
            "store.remove_unconfirmed" => {
                a.check(&["tx"])?;
                let rec = self.arg_tx(a, "tx")?;
                ok_null(self.update(|tx| st.remove_unconfirmed(tx, rec.msg_tx(), rec.hash()))?)
            }
            "store.prune_unmined" => {
                a.check(&["sdiff"])?;
                let sdiff = a.i64("sdiff")?;
                self.update(|tx| st.prune_unmined(tx, sdiff).map_err(|e| e.error))?
                    .map(|hs_| Value::Array(hs_.iter().map(|h| json!(hs(h))).collect()))
            }
            "store.add_credit" => {
                a.check(&["tx", "index", "change", "account", "block"])?;
                let rec = self.arg_tx(a, "tx")?;
                let index = a.u32("index")?;
                let change = Self::opt_bool(a, "change")?;
                let account = a.u32("account")?;
                let m = self.arg_meta(a, "block")?;
                let what = format!("{} block", a.op);
                ok_null(self.update(|tx| {
                    // Resolved last, as Go's runner: its failure is the
                    // operation's result.
                    let block = self.resolve_meta(tx, &m, &what)?;
                    st.add_credit(tx, &rec, block.as_ref(), index, change, account)
                })?)
            }
            "store.account_balance" => {
                a.check(&["minconf", "account"])?;
                let minconf = Self::i32_arg(a, "minconf")?;
                let account = a.u32("account")?;
                self.view(|r| st.account_balance(r, minconf, account))?
                    .map(|b| balances_value(&b))
            }
            "store.account_balances" => {
                a.check(&["minconf"])?;
                let minconf = Self::i32_arg(a, "minconf")?;
                self.view(|r| st.account_balances(r, minconf))?.map(|bs| {
                    Value::Array(
                        bs.iter()
                            .map(|b| {
                                let mut v = balances_value(b);
                                v["key"] = json!(b.account);
                                v
                            })
                            .collect(),
                    )
                })
            }
            "store.unspent_tickets" => {
                a.check(&["syncheight", "include_immature"])?;
                let sh = Self::i32_arg(a, "syncheight")?;
                let imm = Self::bool_arg(a, "include_immature")?;
                self.view(|r| st.unspent_tickets(r, sh, imm))?
                    .map(|hs_| Value::Array(hs_.iter().map(|h| json!(hs(h))).collect()))
            }
            "store.exists_utxo" => {
                a.check(&["outpoint"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                self.view(|r| st.exists_utxo(r, &op))?.map(|v| json!(v))
            }
            "store.is_unspent_outpoint" => {
                a.check(&["outpoint"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                self.view(|r| st.is_unspent_outpoint(r, &op))?
                    .map(|v| json!(v))
            }
            "store.unspent_outputs" => {
                a.check(&[])?;
                self.view(|r| st.unspent_outputs(r))?
                    .map(|cs| Value::Array(cs.iter().map(credit_value).collect()))
            }
            "store.unspent_output" => {
                a.check(&["outpoint", "include_mempool"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                let mempool = Self::bool_arg(a, "include_mempool")?;
                self.view(|r| st.unspent_output(r, &op, mempool))?
                    .map(|c| credit_value(&c))
            }
            "store.for_each_unspent_outpoint" => {
                a.check(&[])?;
                let mut out = Vec::new();
                let r = self.view(|r| {
                    st.for_each_unspent_outpoint(r, |op| {
                        out.push(outpoint_value(op));
                        Ok(())
                    })
                })?;
                r.map(|()| Value::Array(out))
            }
            "store.previous_pk_scripts" => {
                a.check(&["tx", "block"])?;
                let rec = self.arg_tx(a, "tx")?;
                let m = self.arg_meta(a, "block")?;
                let what = format!("{} block", a.op);
                self.view(|r| {
                    let meta = self.resolve_meta(r, &m, &what)?;
                    st.previous_pk_scripts(r, &rec, meta.as_ref().map(|m| &m.block))
                })?
                .map(|ss| Value::Array(ss.iter().map(|s| json!(hex(s))).collect()))
            }
            "store.unspent_output_count" => {
                a.check(&[])?;
                self.view(|r| st.unspent_output_count(r))?.map(|n| json!(n))
            }
            "store.own_ticket" => {
                a.check(&["ticket"])?;
                let h = self.arg_hash(a, "ticket")?;
                self.view(|r| st.own_ticket(r, &h))?.map(|v| json!(v))
            }
            "store.iterate_tickets" => {
                a.check(&[])?;
                self.view(|r| st.iterate_tickets(r))?.map(|ts| {
                    Value::Array(
                        ts.iter()
                            .map(|t| {
                                let mut v = record_value(&t.record);
                                v["block"] = block_value(&t.block);
                                v["spender"] = json!(hs(&t.spender));
                                v
                            })
                            .collect(),
                    )
                })
            }
            "store.tx_details" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| st.tx_details(r, &h))?
                    .map(|d| details_value(&d))
            }
            "store.unique_tx_details" => {
                a.check(&["tx", "block"])?;
                let h = self.arg_hash(a, "tx")?;
                let m = self.arg_meta(a, "block")?;
                let what = format!("{} block", a.op);
                self.view(|r| {
                    let meta = self.resolve_meta(r, &m, &what)?;
                    st.unique_tx_details(r, &h, meta.as_ref().map(|m| &m.block))
                })?
                .map(|d| d.as_ref().map_or(Value::Null, details_value))
            }
            "store.ticket_details" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| {
                    let d = st.tx_details(r, &h)?;
                    st.ticket_details(r, &d)
                })?
                .map(|t| match t {
                    None => Value::Null,
                    Some(t) => json!({
                        "ticket": details_value(&t.ticket),
                        "spender": t.spender.as_ref().map_or(Value::Null, details_value),
                    }),
                })
            }
            "store.tx" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| st.tx(r, &h))?
                    .map(|t| json!(hex(&t.serialize())))
            }
            "store.exists_tx" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| st.exists_tx(r, &h))?.map(|v| json!(v))
            }
            "store.exists_tx_mined_or_unmined" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| st.exists_tx_mined_or_unmined(r, &h))?
                    .map(|(m, u)| json!({"mined": m, "unmined": u}))
            }
            "store.tx_block_height" => {
                a.check(&["tx"])?;
                let h = self.arg_hash(a, "tx")?;
                self.view(|r| st.tx_block_height(r, &h))?.map(|v| json!(v))
            }
            "store.range_transactions" => {
                a.check(&["begin", "end", "stop_after"])?;
                let (begin, end) = (Self::i32_arg(a, "begin")?, Self::i32_arg(a, "end")?);
                let stop = if a.has("stop_after") {
                    Self::i32_arg(a, "stop_after")?
                } else {
                    -1
                };
                let mut batches = Vec::new();
                let r = self.view(|r| {
                    st.range_transactions(r, begin, end, |ds| {
                        batches.push(Value::Array(ds.iter().map(details_value).collect()));
                        Ok(stop >= 0
                            && i64::try_from(batches.len()).unwrap_or(i64::MAX) >= i64::from(stop))
                    })
                })?;
                // Go returns the batches with the error, and the record
                // shows the error alone.
                r.map(|()| Value::Array(batches))
            }
            "store.range_blocks" => {
                a.check(&["begin", "end", "stop_after"])?;
                let (begin, end) = (Self::i32_arg(a, "begin")?, Self::i32_arg(a, "end")?);
                let stop = if a.has("stop_after") {
                    Self::i32_arg(a, "stop_after")?
                } else {
                    -1
                };
                let mut out = Vec::new();
                let r = self.view(|r| {
                    st.range_blocks(r, begin, end, |b| {
                        out.push(block_value(b));
                        Ok(stop >= 0
                            && i64::try_from(out.len()).unwrap_or(i64::MAX) >= i64::from(stop))
                    })
                })?;
                r.map(|()| Value::Array(out))
            }
            "store.unmined_txs" => {
                a.check(&[])?;
                self.view(|r| st.unmined_txs(r))?.map(|recs| {
                    let pos: HashMap<Hash32, usize> = recs
                        .iter()
                        .enumerate()
                        .map(|(i, r)| (*r.hash(), i))
                        .collect();
                    let mut dep_ok = true;
                    for (i, rec) in recs.iter().enumerate() {
                        for input in &rec.msg_tx().tx_in {
                            let p = Hash32(input.previous_out_point.hash.0);
                            if pos.get(&p).is_some_and(|j| *j > i) {
                                dep_ok = false;
                            }
                        }
                    }
                    let mut sorted: Vec<&TxRecord> = recs.iter().collect();
                    sorted.sort_by_key(|r| hs(r.hash()));
                    json!({
                        "txs": sorted.into_iter().map(record_value).collect::<Vec<_>>(),
                        "dependency_order": dep_ok,
                    })
                })
            }
            "store.unmined_tx_hashes" => {
                a.check(&[])?;
                self.view(|r| st.unmined_tx_hashes(r))?
                    .map(|hs_| Value::Array(hs_.iter().map(|h| json!(hs(h))).collect()))
            }
            "store.add_ticket_commitment" => {
                a.check(&["tx", "index", "account"])?;
                let rec = self.arg_tx(a, "tx")?;
                let index = a.u32("index")?;
                let account = a.u32("account")?;
                ok_null(self.update(|tx| st.add_ticket_commitment(tx, &rec, index, account))?)
            }
            "store.redeem_ticket_commitments" => {
                a.check(&["tx", "block"])?;
                let rec = self.arg_tx(a, "tx")?;
                let m = self.arg_meta(a, "block")?;
                let what = format!("{} block", a.op);
                ok_null(self.update(|tx| {
                    let block = self.resolve_meta(tx, &m, &what)?;
                    st.redeem_ticket_commitments(tx, &rec, block.as_ref())
                })?)
            }
            "store.add_multisig_out" => {
                a.check(&["tx", "index", "block"])?;
                let rec = self.arg_tx(a, "tx")?;
                let index = a.u32("index")?;
                let m = self.arg_meta(a, "block")?;
                let what = format!("{} block", a.op);
                ok_null(self.update(|tx| {
                    let block = self.resolve_meta(tx, &m, &what)?;
                    st.add_multisig_out(tx, &rec, block.as_ref(), index)
                })?)
            }
            "store.spend_multisig_out" => {
                a.check(&["outpoint", "spender", "index"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                let spender = self.arg_hash(a, "spender")?;
                let index = a.u32("index")?;
                ok_null(self.update(|tx| st.spend_multisig_out(tx, &op, &spender, index))?)
            }
            "store.get_multisig_output" => {
                a.check(&["outpoint"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                self.view(|r| st.get_multisig_output(r, &op))?
                    .map(|m| multisig_value(&m))
            }
            "store.unspent_multisig_credits_for_address" => {
                a.check(&["addr"])?;
                let aref = self.addr_ref(a, "addr")?;
                let what = format!("{} addr", a.op);
                self.view(|r| {
                    let addr = self.resolve(r, aref, &what)?;
                    st.unspent_multisig_credits_for_address(r, &addr)
                })?
                .map(|ms| {
                    Value::Array(
                        ms.iter()
                            .map(|m| {
                                json!({
                                    "outpoint": outpoint_value(&m.outpoint),
                                    "scripthash": hex(&m.script_hash.0),
                                    "script": hex(&m.ms_script),
                                    "m": m.m, "n": m.n, "amount": m.amount,
                                })
                            })
                            .collect(),
                    )
                })
            }
            "store.random_utxo" => {
                a.check(&["minconf", "syncheight", "rand"])?;
                let min_conf = Self::i32_arg(a, "minconf")?;
                let sync = Self::i32_arg(a, "syncheight")?;
                let mut prng = self.take_stream(a)?;
                let drawn = std::sync::Arc::clone(&self.drawn);
                let r = self.view(|r| {
                    let mut s = Stream {
                        prng: &mut prng,
                        drawn: &drawn,
                    };
                    st.random_utxo(r, &mut s, min_conf, sync)
                });
                self.put_stream(prng);
                r?.map(|c| json!({"credit~r": c.as_ref().map(credit_value)}))
            }
            "store.select_inputs" => {
                a.check(&[
                    "account",
                    "minconf",
                    "syncheight",
                    "targets",
                    "ignore",
                    "rand",
                ])?;
                let account = a.u32("account")?;
                let min_conf = Self::i32_arg(a, "minconf")?;
                let sync = Self::i32_arg(a, "syncheight")?;
                let targets: Vec<i64> = a
                    .value("targets")?
                    .as_array()
                    .ok_or_else(|| fault(format!("{}: targets must be a list", a.op)))?
                    .iter()
                    .map(|v| {
                        v.as_i64()
                            .ok_or_else(|| fault(format!("{}: a target is an integer", a.op)))
                    })
                    .collect::<Result<_, _>>()?;
                // Go's set is keyed by the whole wire outpoint, tree included.
                let ignore: Option<HashSet<(Hash32, u32, i8)>> = if a.has("ignore") {
                    let what = format!("{} ignore", a.op);
                    let list = a
                        .value("ignore")?
                        .as_array()
                        .ok_or_else(|| fault(format!("{what}: must be a list")))?;
                    let mut set = HashSet::new();
                    for v in list {
                        let op = self.outpoint_ref(v, &what)?;
                        set.insert((Hash32(op.hash.0), op.index, op.tree));
                    }
                    Some(set)
                } else {
                    None
                };
                let mut prng = self.take_stream(a)?;
                let drawn = std::sync::Arc::clone(&self.drawn);
                let r = self.view(|r| {
                    let skip = |op: &OutPoint| {
                        ignore
                            .as_ref()
                            .is_some_and(|s| s.contains(&(Hash32(op.hash.0), op.index, op.tree)))
                    };
                    let ignore_fn: Option<&dyn Fn(&OutPoint) -> bool> =
                        if ignore.is_some() { Some(&skip) } else { None };
                    let mut s = Stream {
                        prng: &mut prng,
                        drawn: &drawn,
                    };
                    let mut src =
                        st.make_input_source(r, &mut s, account, min_conf, sync, ignore_fn)?;
                    let mut out = Vec::with_capacity(targets.len());
                    for t in &targets {
                        let d = src.select_inputs(Amount(*t))?;
                        let inputs: Vec<Value> = d
                            .inputs
                            .iter()
                            .zip(&d.scripts)
                            .zip(&d.redeem_script_sizes)
                            .map(|((input, script), size)| {
                                json!({
                                    "outpoint": outpoint_value(&input.previous_out_point),
                                    "value": input.value_in,
                                    "script": hex(script),
                                    "redeemsize": size,
                                })
                            })
                            .collect();
                        out.push(json!({"target": t, "amount": d.amount.0, "inputs": inputs}));
                    }
                    Ok(out)
                });
                self.put_stream(prng);
                r?.map(|out| json!({"selections~r": out}))
            }
            "store.spender" => {
                a.check(&["outpoint"])?;
                let op = self.arg_outpoint(a, "outpoint")?;
                self.view(|r| st.spender(r, &op))?
                    .map(|(tx, index)| json!({"tx": tx.tx_hash().to_string(), "index": index}))
            }
            other => return Err(fault(format!("txops: {other} is not handled"))),
        })
    }

    /// `tx.build` (Go's `opTxBuild`).
    fn tx_build(&mut self, a: &Args<'_>) -> Result<OpResult, String> {
        a.check(&[
            "label", "version", "locktime", "expiry", "inputs", "outputs", "received",
        ])?;
        let label = a.str("label")?.to_owned();
        if self.txs.contains_key(&label) {
            return Err(fault(format!("tx.build: label {label:?} already used")));
        }
        let version = Self::opt_u32(a, "version", 1)?;
        let mut m = MsgTx {
            version: u16::try_from(version & 0xffff).unwrap_or(0),
            lock_time: Self::opt_u32(a, "locktime", 0)?,
            expiry: Self::opt_u32(a, "expiry", 0)?,
            ..MsgTx::default()
        };
        let empty = Vec::new();
        let ins = match a.map.get("inputs") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| fault("tx.build: inputs must be a list"))?,
            None => &empty,
        };
        let outs = match a.map.get("outputs") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| fault("tx.build: outputs must be a list"))?,
            None => &empty,
        };
        let received = if a.has("received") {
            a.i64("received")?
        } else {
            self.clock
        };
        for (i, inp) in ins.iter().enumerate() {
            let what = format!("tx.build {label} input {i}");
            let o = inp
                .as_object()
                .ok_or_else(|| fault(format!("{what}: an input is an object")))?;
            for k in o.keys() {
                if !["prev", "value", "sequence", "script", "tree"].contains(&k.as_str()) {
                    return Err(fault(format!("{what}: unknown field {k:?}")));
                }
            }
            let prev = o
                .get("prev")
                .ok_or_else(|| fault(format!("{what}: prev")))?;
            let mut op = match prev.as_str() {
                Some("coinbase" | "stakebase") => OutPoint {
                    hash: dcroxide_chainhash::Hash::ZERO,
                    index: u32::MAX,
                    tree: 0,
                },
                _ => {
                    let po = prev.as_object().ok_or_else(|| {
                        fault(format!("{what}: an outpoint is {{tx, index, tree?}}"))
                    })?;
                    let h = self.hash_ref(
                        po.get("tx")
                            .ok_or_else(|| fault(format!("{what}: outpoint tx")))?,
                        &what,
                    )?;
                    OutPoint {
                        hash: dcroxide_chainhash::Hash(h.0),
                        index: po
                            .get("index")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                            .unwrap_or(0),
                        tree: po
                            .get("tree")
                            .and_then(Value::as_i64)
                            .and_then(|v| i8::try_from(v).ok())
                            .unwrap_or(0),
                    }
                }
            };
            if let Some(t) = o.get("tree") {
                op.tree = t
                    .as_i64()
                    .and_then(|v| i8::try_from(v).ok())
                    .ok_or_else(|| fault(format!("{what}: tree")))?;
            }
            let script = match o.get("script") {
                None => Vec::new(),
                Some(s) => {
                    let s = s.as_str().unwrap_or("!");
                    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(fault(format!("{what}: script is not hex")));
                    }
                    unhex(s)
                }
            };
            m.tx_in.push(TxIn {
                previous_out_point: op,
                sequence: match o.get("sequence") {
                    None => MAX_TX_IN_SEQUENCE_NUM,
                    Some(v) => v
                        .as_u64()
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or_else(|| fault(format!("{what}: sequence")))?,
                },
                value_in: o.get("value").and_then(Value::as_i64).unwrap_or(0),
                block_height: NULL_BLOCK_HEIGHT,
                block_index: NULL_BLOCK_INDEX,
                signature_script: script,
            });
        }
        let outs = outs.clone();
        let label2 = label.clone();
        let built = self.view(|r| {
            for (i, out) in outs.iter().enumerate() {
                let what = format!("tx.build {label2} output {i}");
                m.tx_out.push(self.build_output(r, out, &what)?);
            }
            TxRecord::from_msg_tx(m.clone(), received)
        });
        let built = match built {
            Ok(Ok(rec)) => Ok(rec),
            Ok(Err(e)) => match script_fault(&e) {
                Some(f) => return Err(fault(f)),
                None => Err(e),
            },
            Err(fault) => return Err(fault),
        };
        Ok(built.map(|rec| {
            let v = json!({
                "hash": hs(rec.hash()),
                "type": type_name(rec.tx_type()),
                "tx": hex(rec.serialized()),
            });
            self.txs.insert(label, rec);
            v
        }))
    }

    /// Go's `buildOutput`. Script faults surface as `AnyError`s tagged
    /// `script fault:`, which [`Runner::tx_build`]'s caller turns back into
    /// faults.
    fn build_output(
        &self,
        r: &mut purse_store::ReadTx,
        o: &Value,
        what: &str,
    ) -> Result<TxOut, AnyError> {
        let sf = |msg: String| AnyError::new(format!("script fault: {msg}"));
        let o = o
            .as_object()
            .ok_or_else(|| sf(format!("{what}: an output is an object")))?;
        let value = o.get("value").and_then(Value::as_i64).unwrap_or(0);
        let mut n = 0;
        let mut script = Vec::new();
        let addr = |v: &Value, r: &mut purse_store::ReadTx| -> Result<Address, AnyError> {
            let a = Args {
                op: what,
                map: &purse_testutil::serde_json::Map::from_iter([("a".to_owned(), v.clone())]),
            };
            let aref = self.addr_ref(&a, "a").map_err(&sf)?;
            self.resolve(r, aref, what)
        };
        if let Some(p) = o.get("pay") {
            n += 1;
            let ad = addr(p, r)?;
            let (_, mut s) = ad.payment_script();
            if let Some(tag) = o.get("tag").and_then(Value::as_str) {
                let op = match tag {
                    "sstx" => 0xba,
                    "ssgen" => 0xbb,
                    "ssrtx" => 0xbc,
                    "sstxchange" => 0xbd,
                    "tgen" => 0xc3,
                    _ => return Err(sf(format!("{what}: unknown tag {tag:?}"))),
                };
                s.insert(0, op);
            }
            script = s;
        } else if o.contains_key("tag") {
            return Err(sf(format!("{what}: tag without pay")));
        }
        if let Some(s) = o.get("script") {
            n += 1;
            let s = s.as_str().unwrap_or("!");
            if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(sf(format!("{what}: script is not hex")));
            }
            script = unhex(s);
        }
        if let Some(c) = o.get("commitment") {
            n += 1;
            let c = c
                .as_object()
                .ok_or_else(|| sf(format!("{what}: commitment")))?;
            let ad = addr(c.get("pay").unwrap_or(&Value::Null), r)?;
            let (h160, p2sh) = match &ad {
                Address::PubKeyHashEcdsaSecp256k1V0 { hash, .. } => (*hash, false),
                Address::ScriptHashV0 { hash, .. } => (*hash, true),
                _ => {
                    return Err(sf(format!(
                        "{what}: commitment address must be P2PKH or P2SH"
                    )));
                }
            };
            let amount = c.get("amount").and_then(Value::as_i64).unwrap_or(0);
            let mut payload = h160.to_vec();
            let mut amt = amount.to_le_bytes();
            if p2sh {
                amt[7] |= 0x80;
            }
            payload.extend_from_slice(&amt);
            let limits = c
                .get("limits")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
                .unwrap_or(0x5800);
            payload.extend_from_slice(&limits.to_le_bytes());
            script = push_script(&payload);
        }
        if let Some(vb) = o.get("voteblock") {
            n += 1;
            let b = self.block_ref(vb, what).map_err(&sf)?;
            let h = self.resolve_block(r, &b)?;
            let meta = self.block_meta(r, &h, what)?;
            let mut payload = h.0.to_vec();
            #[allow(clippy::cast_sign_loss, reason = "Go's uint32(meta.Height)")]
            payload.extend_from_slice(&(meta.block.height as u32).to_le_bytes());
            script = push_script(&payload);
        }
        if let Some(v) = o.get("votebits") {
            n += 1;
            let s = v.as_str().unwrap_or("!");
            if s.len() % 2 != 0
                || !s.bytes().all(|b| b.is_ascii_hexdigit())
                || s.len() < 4
                || s.len() > 150
            {
                return Err(sf(format!("{what}: votebits must be 2 to 75 bytes of hex")));
            }
            script = push_script(&unhex(s));
        }
        if let Some(v) = o.get("opreturn") {
            n += 1;
            let s = v.as_str().unwrap_or("!");
            if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) || s.len() > 150 {
                return Err(sf(format!(
                    "{what}: opreturn must be at most 75 bytes of hex"
                )));
            }
            script = push_script(&unhex(s));
        }
        if n != 1 {
            return Err(sf(format!(
                "{what}: an output needs exactly one of pay, script, commitment, voteblock, votebits, opreturn"
            )));
        }
        Ok(TxOut {
            value,
            version: 0,
            pk_script: script,
        })
    }

    /// `store.extend_main_chain`, with Go's header and filter builder
    /// (`buildHeader`) or the stored header and filter of `again`.
    fn extend(&mut self, a: &Args<'_>) -> Result<OpResult, String> {
        let store = self.txstore.clone();
        if a.has("again") {
            a.check(&["again"])?;
            let b = self.arg_block(a, "again")?;
            let what = format!("{} again", a.op);
            return self.update(|tx| {
                let h = self.resolve_block(tx, &b)?;
                let header = store
                    .get_block_header(tx, &h)
                    .map_err(|e| resolve_err(&what, "GetSerializedBlockHeader", e))?;
                let (_, f) = store
                    .cfilter_v2(tx, &h)
                    .map_err(|e| resolve_err(&what, "CFilterV2", e))?;
                let hash = Hash32(*header_hash(&header).as_bytes());
                store.extend_main_chain(tx, &header, &hash, Some(&f))?;
                Ok(json!({"hash": hs(&hash), "height": header.height()}))
            });
        }
        a.check(&[
            "label",
            "parent",
            "height",
            "votebits",
            "stakeversion",
            "time",
            "filter",
        ])?;
        let label = if a.has("label") {
            a.str("label")?.to_owned()
        } else {
            String::new()
        };
        if !label.is_empty() && self.blocks.contains_key(&label) {
            return Err(fault(format!(
                "{}: block label {label:?} already used",
                a.op
            )));
        }
        let parent = if a.has("parent") {
            Some(self.arg_block(a, "parent")?)
        } else {
            None
        };
        let explicit_height = if a.has("height") {
            Some(a.u32("height")?)
        } else {
            None
        };
        let vote_bits = Self::opt_u32(a, "votebits", 1)?;
        let stake_version = Self::opt_u32(a, "stakeversion", 0)?;
        let time = if a.has("time") {
            Some(a.i64("time")?)
        } else {
            None
        };
        let mut data: Vec<Vec<u8>> = Vec::new();
        if let Some(v) = a.map.get("filter") {
            for l in v
                .as_array()
                .ok_or_else(|| fault(format!("{}: filter must be a list", a.op)))?
            {
                let l = l.as_str().unwrap_or_default();
                let rec = self
                    .txs
                    .get(l)
                    .ok_or_else(|| fault(format!("{}: unknown transaction label {l:?}", a.op)))?;
                for o in &rec.msg_tx().tx_out {
                    data.push(o.pk_script.clone());
                }
            }
        }
        self.nonce = self.nonce.wrapping_add(1);
        let nonce = self.nonce;
        let chain_time = self.chain_time;
        let bits = self.params.pow_limit_bits;
        let what = format!("{} parent", a.op);
        let r = self.update(|tx| {
            let (mut parent_hash, mut height) = store.main_chain_tip(tx)?;
            if let Some(p) = &parent {
                parent_hash = self.resolve_block(tx, p)?;
                let ph = store
                    .get_block_header(tx, &parent_hash)
                    .map_err(|e| resolve_err(&what, "GetBlockHeader", e))?;
                height = purse_udb::txstore::extract_block_header_height(&ph);
            }
            #[allow(clippy::cast_sign_loss, reason = "Go's uint32(height + 1)")]
            let h32 = explicit_height.unwrap_or(height.wrapping_add(1) as u32);
            let ts =
                time.unwrap_or_else(|| chain_time.wrapping_add(i64::from(h32).wrapping_mul(300)));
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "Go's time.Unix(ts, 0) stored as a uint32"
            )]
            let header = BlockHeader {
                version: 7,
                prev_block: dcroxide_chainhash::Hash(parent_hash.0),
                merkle_root: det_hash("merkle", h32, nonce),
                stake_root: det_hash("stake", h32, nonce),
                vote_bits: u16::try_from(vote_bits & 0xffff).unwrap_or(0),
                final_state: [0; 6],
                voters: 0,
                fresh_stake: 0,
                revocations: 0,
                pool_size: 0,
                bits,
                sbits: 0,
                height: h32,
                size: 0,
                timestamp: ts as u32,
                nonce,
                extra_data: [0; 32],
                stake_version,
            };
            let refs: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
            let f = FilterV2::new(
                blockcf2::B,
                blockcf2::M,
                blockcf2::key(&header.merkle_root),
                &refs,
            )
            .map_err(|e| AnyError::new(e.to_string()))?;
            let hash = Hash32(*header.block_hash().as_bytes());
            let hb = HeaderBytes(header.serialize());
            store.extend_main_chain(tx, &hb, &hash, Some(f.bytes()))?;
            Ok((hash, h32))
        })?;
        Ok(r.map(|(hash, h32)| {
            if !label.is_empty() {
                self.blocks.insert(label, hash);
            }
            json!({"hash": hs(&hash), "height": h32})
        }))
    }
}

/// The hash of a serialized header.
fn header_hash(h: &HeaderBytes) -> dcroxide_chainhash::Hash {
    match BlockHeader::from_bytes(&h.0) {
        Ok((b, _)) => b.block_hash(),
        Err(_) => dcroxide_chainhash::Hash::ZERO,
    }
}

/// `OP_RETURN` and a minimal push of `b` (at most 75 bytes).
fn push_script(b: &[u8]) -> Vec<u8> {
    let mut s = vec![0x6a, u8::try_from(b.len()).unwrap_or(0)];
    s.extend_from_slice(b);
    s
}

/// An `AnyError` the output builder tagged as a script fault.
pub fn script_fault(e: &AnyError) -> Option<String> {
    let t = e.render(purse_errors::DAEMON_SEPARATOR);
    t.strip_prefix("script fault: ").map(str::to_owned)
}
