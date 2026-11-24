// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The queries over unspent outputs that need no input selection:
//! `ExistsUTXO` (`txquery.go:295-307`), `UnspentOutputs`, `UnspentOutput`,
//! `ForEachUnspentOutpoint` and `IsUnspentOutpoint` (`txmined.go:2463-2650`,
//! with `outputCreditInfo`, `:2244-2348`), and `PreviousPkScripts`
//! (`txquery.go:540-638`).
//!
//! An unspent row carries what Go reads from the credit it names (amount,
//! flags and script location, ADR-0003 Decision item 2, the row equal to
//! its credit's: `check_credit_rows` in `tests/txstore.rs`), so a mined
//! output costs the record read Go makes and no credit read; the block hash
//! Go's unspent value carries is the main-chain block's at the row's height,
//! read once per height. An unmined credit's script is cut from the unmined
//! record at the location the credit records, where Go decodes the whole
//! transaction: the same bytes (the location was computed from that
//! transaction, whose prefix the record holds verbatim), no decoding.

use std::collections::BTreeMap;
use std::ops::Bound;

use dcroxide_chainhash::Hash;
use dcroxide_wire::{OutPoint, TX_TREE_REGULAR, TX_TREE_STAKE};
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    CreditKey, Credits, Debits, Hash32, Outpoint, OutputTag, ScriptLoc, TxMined, TxMinedRow,
    TxRecKey, TxUnmined, UnminedCreditRow, UnminedCredits, UnminedInputs, Unpublished, Unspent,
    UnspentRow,
};

use super::chain::block_at;
use super::record::{Block, BlockMeta, Credit, TxRecord, ZERO_TIME, canonical, display};
use super::store::{TxRead, TxStore, store_err, walk};

/// Go's `scriptLocNotStored`: a legacy credit without a script location,
/// whose script is read by decoding the whole transaction. This store
/// never writes one (an output script never starts at offset 0), but the
/// reader keeps Go's branch.
const SCRIPT_LOC_NOT_STORED: u32 = 0;

/// The tree Go gives an outpoint from its credit's tag: stake for any tag
/// but `opNonstake`.
fn tree_of(tag: OutputTag) -> i8 {
    if tag == OutputTag::NonStake {
        TX_TREE_REGULAR
    } else {
        TX_TREE_STAKE
    }
}

fn wire_outpoint(op: &Outpoint, tree: i8) -> OutPoint {
    OutPoint {
        hash: Hash(op.hash.0),
        index: op.index,
        tree,
    }
}

/// `hash:index`, as dcrd's `OutPoint.String` prints an outpoint.
fn op_display(op: &Outpoint) -> String {
    format!("{}:{}", display(&op.hash), op.index)
}

fn hex(b: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len().saturating_mul(2));
    for x in b {
        s.push(char::from(DIGITS[usize::from(x >> 4)]));
        s.push(char::from(DIGITS[usize::from(x & 0x0f)]));
    }
    s
}

/// Go's 72-byte credit key (hash ‖ height ‖ block hash ‖ index, big
/// endian), as its error texts print it.
fn go_credit_key(k: &CreditKey, block: &Hash32) -> String {
    let mut b = Vec::with_capacity(72);
    b.extend_from_slice(&k.hash.0);
    b.extend_from_slice(&k.height.to_be_bytes());
    b.extend_from_slice(&block.0);
    b.extend_from_slice(&k.index.to_be_bytes());
    hex(&b)
}

/// `fetchRawTxRecordPkScript` (`txdb.go:606-647`) over the record bytes
/// `tx` of `hash` (`None`: no record, Go's nil value): the script at `loc`,
/// with Go's bounds check, whose offsets count the 8-byte received time
/// that prefixes Go's value; a location not stored decodes the transaction
/// for output `index`.
fn script_at(
    tx: Option<&[u8]>,
    loc: &ScriptLoc,
    hash: &Hash32,
    index: u32,
) -> Result<Vec<u8>, AnyError> {
    if loc.offset == SCRIPT_LOC_NOT_STORED {
        let Some(tx) = tx else {
            return Err(e!(Kind::Io, "tx record len 0").into());
        };
        let rec = TxRecord::stored(*hash, tx, 0)?;
        let out = usize::try_from(index)
            .ok()
            .and_then(|i| rec.msg_tx().tx_out.get(i));
        return match out {
            Some(o) => Ok(o.pk_script.clone()),
            None => Err(e!(Kind::Io, "missing transaction output for credit index").into()),
        };
    }
    // Go's value is received (8 bytes) ‖ transaction.
    let v_len = tx.map_or(0, |t| t.len()).saturating_add(8);
    let loc_int = u64::from(loc.offset).saturating_add(8);
    let end = loc_int.saturating_add(u64::from(loc.len));
    let v_len = u64::try_from(v_len).unwrap_or(u64::MAX);
    // Go: scrLocInt > len(v)-1 || scrLocInt+scrLenInt > len(v).
    let (Some(tx), false) = (tx, loc_int >= v_len || end > v_len) else {
        return Err(e!(
            Kind::Io,
            format!("invalid script offset {loc_int} for tx {}", display(hash))
        )
        .into());
    };
    let start = usize::try_from(loc.offset).unwrap_or(usize::MAX);
    let stop = start.saturating_add(usize::from(loc.len));
    tx.get(start..stop)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| e!(Kind::Io, "script location outside the record").into())
}

/// The hashes of the main-chain blocks a query has read, by height: Go's
/// unspent value carries the block hash, this store's row the height.
#[derive(Default)]
struct BlockHashes(BTreeMap<u32, Hash32>);

impl BlockHashes {
    fn at(&mut self, r: &mut (impl TxRead + ?Sized), height: u32) -> Result<Hash32, AnyError> {
        if let Some(h) = self.0.get(&height) {
            return Ok(*h);
        }
        let Some(b) = block_at(r, height)? else {
            return Err(e!(
                Kind::Io,
                format!("missing block record for height {height}")
            )
            .into());
        };
        self.0.insert(height, b.hash);
        Ok(b.hash)
    }
}

/// The reads one query shares across outputs: the block hashes, and the
/// last mined record read, which the next output of the same transaction
/// (the next unspent key) reuses.
#[derive(Default)]
struct Reads {
    blocks: BlockHashes,
    record: Option<(TxRecKey, Option<TxMinedRow>)>,
}

impl Reads {
    fn record(
        &mut self,
        r: &mut (impl TxRead + ?Sized),
        key: TxRecKey,
    ) -> Result<Option<&TxMinedRow>, AnyError> {
        if self.record.as_ref().is_none_or(|(k, _)| *k != key) {
            let row = r.get::<TxMined>(&key).map_err(store_err)?;
            self.record = Some((key, row));
        }
        Ok(self.record.as_ref().and_then(|(_, row)| row.as_ref()))
    }
}

/// `outputCreditInfo`'s unmined branch: the credit of `op` from its
/// unmined credit `m` and its unmined record.
fn unmined_credit(
    r: &mut (impl TxRead + ?Sized),
    op: &Outpoint,
    m: &UnminedCreditRow,
) -> Result<Credit, AnyError> {
    let Some(row) = r.get::<TxUnmined>(&op.hash).map_err(store_err)? else {
        return Err(e!(Kind::Io, "unmined len 0").into());
    };
    let start = usize::try_from(m.script.offset).unwrap_or(usize::MAX);
    let Some(script) = row
        .tx
        .get(start..start.saturating_add(usize::from(m.script.len)))
    else {
        return Err(e!(
            Kind::Io,
            format!("no output {} for tx {}", op.index, display(&op.hash))
        )
        .into());
    };
    Ok(Credit {
        outpoint: wire_outpoint(op, tree_of(m.flags.tag)),
        block: BlockMeta::unmined(),
        amount: m.amount,
        pk_script: script.to_vec(),
        received: row.received,
        from_coinbase: false,
        has_expiry: m.flags.expiry,
    })
}

/// `outputCreditInfo`: the credit of `op`, from its unmined credit when
/// one is recorded (Go looks there first, even for an output it found
/// unspent), else from its unspent row `mined`.
fn output_credit(
    r: &mut (impl TxRead + ?Sized),
    op: &Outpoint,
    mined: Option<&UnspentRow>,
    reads: &mut Reads,
) -> Result<Credit, AnyError> {
    if let Some(m) = r.get::<UnminedCredits>(op).map_err(store_err)? {
        return unmined_credit(r, op, &m);
    }
    let Some(u) = mined else {
        return Err(e!(
            Kind::Io,
            format!("missing credit for outpoint {}", op_display(op))
        )
        .into());
    };
    let block = reads.blocks.at(r, u.height)?;
    let key = TxRecKey {
        hash: op.hash,
        height: u.height,
    };
    // Go reads the record's received time without checking that it exists
    // (a panic on a missing record, ADR-0012 Rule 2: missing stored state).
    let Some(rec) = reads.record(r, key)? else {
        return Err(e!(
            Kind::Io,
            format!(
                "missing transaction record for credit {} at height {}",
                op_display(op),
                u.height
            )
        )
        .into());
    };
    let received = rec.received;
    let pk_script = script_at(Some(&rec.tx), &u.script, &op.hash, op.index)?;
    Ok(Credit {
        outpoint: wire_outpoint(op, tree_of(u.flags.tag)),
        block: BlockMeta {
            block: Block {
                hash: block,
                height: i32::try_from(u.height).unwrap_or(i32::MAX),
            },
            // Go leaves the credit's block time and vote bits unset.
            time: ZERO_TIME,
            vote_bits: 0,
        },
        amount: u.amount,
        pk_script,
        received,
        from_coinbase: u.flags.coinbase,
        has_expiry: u.flags.expiry,
    })
}

impl TxStore {
    /// `ExistsUTXO`: whether `op` is an unspent output (spent by an unmined
    /// transaction or not) or an unmined credit.
    ///
    /// # Errors
    ///
    /// The store's (Go's returns none, ADR-0012 Rule 2(c)).
    pub fn exists_utxo(
        &self,
        r: &mut (impl TxRead + ?Sized),
        op: &OutPoint,
    ) -> Result<bool, AnyError> {
        let k = canonical(op);
        if r.contains::<Unspent>(&k).map_err(store_err)? {
            return Ok(true);
        }
        r.contains::<UnminedCredits>(&k).map_err(store_err)
    }

    /// `UnspentOutputs`: the unspent outputs no unmined transaction spends,
    /// in outpoint order, then the unmined credits no unmined transaction
    /// spends whose transaction is not unpublished, in outpoint order.
    ///
    /// A mined output's block is its hash and height, with Go's zero time
    /// and vote bits 0, as Go leaves them; an unmined one's has height -1
    /// and no hash. An unspent output that also has an unmined credit is
    /// given as the unmined credit, as Go's `outputCreditInfo` looks there
    /// first.
    ///
    /// # Errors
    ///
    /// A record, block row or unmined record missing, or a script location
    /// outside its record: `IO`, Go's text where Go has one. The store's.
    pub fn unspent_outputs(&self, r: &mut (impl TxRead + ?Sized)) -> Result<Vec<Credit>, AnyError> {
        let mut out = Vec::new();
        let mut reads = Reads::default();
        walk::<Unspent, _, _>(r, Bound::Unbounded, Bound::Unbounded, false, |r, op, u| {
            if !r.contains::<UnminedInputs>(&op).map_err(store_err)? {
                out.push(output_credit(r, &op, Some(&u), &mut reads)?);
            }
            Ok(false)
        })?;
        walk::<UnminedCredits, _, _>(r, Bound::Unbounded, Bound::Unbounded, false, |r, op, m| {
            if !r.contains::<UnminedInputs>(&op).map_err(store_err)?
                && !r.contains::<Unpublished>(&op.hash).map_err(store_err)?
            {
                out.push(unmined_credit(r, &op, &m)?);
            }
            Ok(false)
        })?;
        Ok(out)
    }

    /// `UnspentOutput`: the credit of `op` when it is an unmined credit and
    /// `include_mempool` is set, or an unspent output (spent by an unmined
    /// transaction or not; the credit then comes from an unmined credit of
    /// `op` if there is one, as in [`TxStore::unspent_outputs`]).
    ///
    /// # Errors
    ///
    /// Neither: `NotExist`, Go's text. As [`TxStore::unspent_outputs`].
    pub fn unspent_output(
        &self,
        r: &mut (impl TxRead + ?Sized),
        op: &OutPoint,
        include_mempool: bool,
    ) -> Result<Credit, AnyError> {
        let k = canonical(op);
        if include_mempool && let Some(m) = r.get::<UnminedCredits>(&k).map_err(store_err)? {
            return unmined_credit(r, &k, &m);
        }
        if let Some(u) = r.get::<Unspent>(&k).map_err(store_err)? {
            return output_credit(r, &k, Some(&u), &mut Reads::default());
        }
        Err(e!(
            Kind::NotExist,
            format!("no unspent output {}", op_display(&k))
        )
        .into())
    }

    /// `ForEachUnspentOutpoint`: calls `f` with each outpoint
    /// [`TxStore::unspent_outputs`] would give, with its tree, in the same
    /// order, reading no record; an error from `f` ends the walk.
    ///
    /// # Errors
    ///
    /// `f`'s. The store's.
    pub fn for_each_unspent_outpoint(
        &self,
        r: &mut (impl TxRead + ?Sized),
        mut f: impl FnMut(&OutPoint) -> Result<(), AnyError>,
    ) -> Result<(), AnyError> {
        walk::<Unspent, _, _>(r, Bound::Unbounded, Bound::Unbounded, false, |r, op, u| {
            if !r.contains::<UnminedInputs>(&op).map_err(store_err)? {
                f(&wire_outpoint(&op, tree_of(u.flags.tag)))?;
            }
            Ok(false)
        })?;
        walk::<UnminedCredits, _, _>(r, Bound::Unbounded, Bound::Unbounded, false, |r, op, m| {
            if !r.contains::<UnminedInputs>(&op).map_err(store_err)?
                && !r.contains::<Unpublished>(&op.hash).map_err(store_err)?
            {
                f(&wire_outpoint(&op, tree_of(m.flags.tag)))?;
            }
            Ok(false)
        })?;
        Ok(())
    }

    /// `IsUnspentOutpoint`, as Go answers (QK-0088, reproduced; spec 04
    /// D-1): false when `op`'s transaction is unpublished, else whether no
    /// unmined transaction spends `op`. Go tests that the unspent table
    /// exists, which it always does, where it means to look the outpoint
    /// up, so any other outpoint, recorded or not, is answered true.
    ///
    /// # Errors
    ///
    /// The store's (Go's returns none, ADR-0012 Rule 2(c)).
    pub fn is_unspent_outpoint(
        &self,
        r: &mut (impl TxRead + ?Sized),
        op: &OutPoint,
    ) -> Result<bool, AnyError> {
        let k = canonical(op);
        if r.contains::<Unpublished>(&k.hash).map_err(store_err)? {
            return Ok(false);
        }
        Ok(!r.contains::<UnminedInputs>(&k).map_err(store_err)?)
    }

    /// `PreviousPkScripts`, as Go answers (QK-0089, reproduced; spec 04
    /// D-1): with a block, the scripts of the credits the record's debits
    /// in that block spend, in input order (none for a block that is not
    /// the main-chain block at its height, whose records Go keys apart).
    ///
    /// Without a block, Go walks the inputs (an input spending an unmined
    /// credit fails with `IO`, a short record key; one spending an unspent
    /// output adds its script) and then dereferences the missing block: the
    /// port gives the same `IO` error and, where Go panics, an `Invalid`
    /// error (ADR-0012 Rule 2), so it never answers without a block.
    ///
    /// # Errors
    ///
    /// As above. A debit's credit missing, or a script location outside its
    /// record: `IO`, Go's text. The store's.
    pub fn previous_pk_scripts(
        &self,
        r: &mut (impl TxRead + ?Sized),
        rec: &TxRecord,
        block: Option<&Block>,
    ) -> Result<Vec<Vec<u8>>, AnyError> {
        let mut scripts = Vec::new();
        let Some(block) = block else {
            for input in &rec.msg_tx().tx_in {
                let prev = canonical(&input.previous_out_point);
                if r.contains::<TxUnmined>(&prev.hash).map_err(store_err)? {
                    if !r.contains::<UnminedCredits>(&prev).map_err(store_err)? {
                        continue;
                    }
                    // Go passes the 32-byte hash where its record reader
                    // wants a 68-byte record key.
                    return Err(e!(Kind::Io, "tx record key len 32").into());
                }
                let Some(u) = r.get::<Unspent>(&prev).map_err(store_err)? else {
                    continue;
                };
                let ck = CreditKey {
                    hash: prev.hash,
                    height: u.height,
                    index: prev.index,
                };
                scripts.push(credit_script(r, &ck, "value")?);
            }
            return Err(e!(
                Kind::Invalid,
                "PreviousPkScripts: no block, where Go dereferences a nil block and panics"
            )
            .into());
        };
        let Ok(height) = u32::try_from(block.height) else {
            return Ok(scripts);
        };
        if block_at(r, height)?.map(|b| b.hash) != Some(block.hash) {
            return Ok(scripts);
        }
        let lo = CreditKey {
            hash: *rec.hash(),
            height,
            index: 0,
        };
        let hi = CreditKey {
            index: u32::MAX,
            ..lo
        };
        let debits = r
            .rows::<Debits>(
                Bound::Included(&lo),
                Bound::Included(&hi),
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
        for (_, d) in debits {
            scripts.push(credit_script(r, &d.credit, "val")?);
        }
        Ok(scripts)
    }
}

/// The script of the mined credit `ck`, read as `PreviousPkScripts` reads
/// it: the credit, then its record; `what` is the word Go's two "missing
/// credit" texts differ by.
fn credit_script(
    r: &mut (impl TxRead + ?Sized),
    ck: &CreditKey,
    what: &str,
) -> Result<Vec<u8>, AnyError> {
    let Some(c) = r.get::<Credits>(ck).map_err(store_err)? else {
        let block = block_at(r, ck.height)?.map(|b| b.hash).unwrap_or_default();
        return Err(e!(
            Kind::Io,
            format!(
                "missing credit {what} for key {}",
                go_credit_key(ck, &block)
            )
        )
        .into());
    };
    let row = r
        .get::<TxMined>(&TxRecKey {
            hash: ck.hash,
            height: ck.height,
        })
        .map_err(store_err)?;
    script_at(
        row.as_ref().map(|t| t.tx.as_slice()),
        &c.script,
        &ck.hash,
        ck.index,
    )
}
