// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2017 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Unmined transactions (`wallet/udb/txunmined.go`, `kahnsort.go`).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use dcroxide_stake::TxType;
use dcroxide_wire::{MsgTx, NO_EXPIRY_VALUE};
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Empty, Hash32, TableId, Tickets, TxMined, TxRecKey, TxUnmined, TxUnminedRow, UnminedInputs,
    Unpublished, WriteTx,
};

use super::credit::{del_ucredit, del_uinput, put_uinput, set_unpublished};
use super::record::{TxRecord, canonical, display, io};
use super::store::{TxRead, TxStore, guard, store_err};

/// What `PruneUnmined` returns with its error: the hashes removed before
/// the failure, whose removals stay in the caller's transaction (Go's
/// caller commits after an error, QK-0059).
#[derive(Debug)]
pub struct PruneError {
    /// The transactions removed before the failure, in removal order.
    pub removed: Vec<Hash32>,
    /// The failure.
    pub error: AnyError,
}

/// The tables `RemoveUnconfirmed` reads and changes in Go that this crate
/// does not maintain yet: a ticket's commitments, a vote's or revocation's
/// commitment flags, the multisig record of each input (crate
/// documentation, "The next slice").
const REMOVE_TABLES: [TableId; 4] = [
    TableId::Commitments,
    TableId::CommitmentsUnspent,
    TableId::MultisigOuts,
    TableId::MultisigUnspent,
];

/// The bounds of the mined records of `hash`: every height.
fn mined_bounds(hash: &Hash32) -> (TxRecKey, TxRecKey) {
    (
        TxRecKey {
            hash: *hash,
            height: 0,
        },
        TxRecKey {
            hash: *hash,
            height: u32::MAX,
        },
    )
}

/// The last mined record of `hash` (Go's `latestTxRecord`: the highest
/// height).
pub(crate) fn latest_mined(
    r: &mut (impl TxRead + ?Sized),
    hash: &Hash32,
) -> Result<Option<(TxRecKey, purse_store::TxMinedRow)>, AnyError> {
    let (lo, hi) = mined_bounds(hash);
    Ok(
        r.rows::<TxMined>(Bound::Included(&lo), Bound::Included(&hi), true, 1)
            .map_err(store_err)?
            .pop(),
    )
}

/// The height of the last mined record of `hash`, its value not read:
/// Go's `latestTxRecord` where the caller uses only the key or tests the
/// value for nil (`existsTxRecord`-style checks), which copy nothing.
pub(crate) fn latest_mined_height(
    r: &mut (impl TxRead + ?Sized),
    hash: &Hash32,
) -> Result<Option<u32>, AnyError> {
    let (lo, hi) = mined_bounds(hash);
    Ok(
        r.keys::<TxMined>(Bound::Included(&lo), Bound::Included(&hi), true, 1)
            .map_err(store_err)?
            .pop()
            .map(|k| k.height),
    )
}

/// The unmined record of `hash` decoded as Go's `readRawTxRecord` decodes
/// it, a missing row giving Go's text for a value of length 0.
pub(crate) fn read_unmined(
    r: &mut (impl TxRead + ?Sized),
    hash: &Hash32,
) -> Result<TxRecord, AnyError> {
    match r.get::<TxUnmined>(hash).map_err(store_err)? {
        Some(row) => TxRecord::stored(*hash, &row.tx, row.received),
        None => Err(e!(Kind::Io, "tx record len 0").into()),
    }
}

/// One transaction being removed by [`TxStore::remove_unconfirmed`]: Go's
/// recursive call, with the output it is at.
struct Frame {
    tx: MsgTx,
    hash: Hash32,
    out: u32,
    spender_done: bool,
}

impl TxStore {
    /// `InsertMemPoolTx`: records `rec` as unmined, its inputs in the
    /// unmined-inputs index (a vote's stakebase excepted) and a ticket in
    /// the ticket set.
    ///
    /// # Errors
    ///
    /// Go's: the transaction is recorded mined: `Exist`; an input another
    /// unmined transaction spends: `DoubleSpend` (a vote or revocation
    /// double spending an unmined vote on a block other than the tip
    /// removes that vote instead, and one double spending a vote on the tip
    /// is `DoubleSpend` with Go's text). The guard. The store's.
    pub fn insert_mem_pool_tx(&self, tx: &mut WriteTx<'_>, rec: &TxRecord) -> Result<(), AnyError> {
        let hash = *rec.hash();
        if latest_mined_height(tx, &hash)?.is_some() {
            return Err(e!(Kind::Exist, "transaction already exists mined").into());
        }
        if tx.contains::<TxUnmined>(&hash).map_err(store_err)? {
            return Ok(());
        }
        for input in &rec.msg_tx().tx_in {
            let prev = &input.previous_out_point;
            let k = canonical(prev);
            let Some(spender) = tx.get::<UnminedInputs>(&k).map_err(store_err)? else {
                continue;
            };
            if matches!(rec.tx_type(), TxType::SSGen | TxType::SSRtx) {
                let Some(row) = tx.get::<TxUnmined>(&spender).map_err(store_err)? else {
                    // Go slices a missing value and panics.
                    return Err(e!(
                        Kind::Io,
                        format!(
                            "unmined input {prev} names {}, which is not an unmined transaction",
                            display(&spender)
                        )
                    )
                    .into());
                };
                let (spender_tx, _) =
                    MsgTx::from_bytes(&row.tx).map_err(|err| io(AnyError::foreign(err)))?;
                if dcroxide_stake::determine_tx_type(&spender_tx) == TxType::SSGen {
                    let voted = dcroxide_stake::ssgen_block_voted_on(&spender_tx).0;
                    let (tip_hash, _) = self.main_chain_tip(tx)?;
                    if voted.0 == tip_hash.0 {
                        return Err(e!(
                            Kind::DoubleSpend,
                            format!(
                                "vote or revocation {} double spends unmined vote {} on the tip block",
                                display(&hash),
                                display(&spender)
                            )
                        )
                        .into());
                    }
                    self.remove_unconfirmed(tx, &spender_tx, &spender)?;
                    continue;
                }
            }
            return Err(e!(
                Kind::DoubleSpend,
                format!(
                    "{} conflicts with {} by double spending {prev}",
                    display(&hash),
                    display(&spender)
                )
            )
            .into());
        }
        tx.put::<TxUnmined>(
            &hash,
            &TxUnminedRow {
                received: rec.received(),
                tx: rec.serialized().to_vec(),
            },
        )
        .map_err(store_err)?;
        if rec.unpublished() {
            set_unpublished(tx, &hash, true)?;
        }
        let tx_type = dcroxide_stake::determine_tx_type(rec.msg_tx());
        for (i, input) in rec.msg_tx().tx_in.iter().enumerate() {
            if i == 0 && tx_type == TxType::SSGen {
                continue;
            }
            put_uinput(tx, &canonical(&input.previous_out_point), &hash)?;
        }
        if tx_type == TxType::SStx {
            put_ticket(tx, &hash)?;
        }
        Ok(())
    }

    /// `SetPublished`: removes (`published`) or sets the unpublished mark
    /// of `hash`, whether or not an unmined record of it exists, as Go.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn set_published(
        &self,
        tx: &mut WriteTx<'_>,
        hash: &Hash32,
        published: bool,
    ) -> Result<(), AnyError> {
        set_unpublished(tx, hash, !published)
    }

    /// `RemoveUnconfirmed`: removes the unmined transaction `msg_tx`
    /// (`hash`) and every unmined transaction spending its outputs, the
    /// spenders of each output first, in Go's order; the unmined-inputs
    /// rows of each removed transaction's inputs go whichever transaction
    /// they name (QK-0060).
    ///
    /// # Errors
    ///
    /// A spender named by the unmined-inputs index without its record:
    /// `IO`, Go's text. The guard. The store's.
    pub fn remove_unconfirmed(
        &self,
        tx: &mut WriteTx<'_>,
        msg_tx: &MsgTx,
        hash: &Hash32,
    ) -> Result<(), AnyError> {
        guard(tx, "RemoveUnconfirmed", &REMOVE_TABLES)?;
        let mut stack = vec![Frame {
            tx: msg_tx.clone(),
            hash: *hash,
            out: 0,
            spender_done: false,
        }];
        while let Some(top) = stack.last_mut() {
            let n_outs = u32::try_from(top.tx.tx_out.len()).unwrap_or(u32::MAX);
            if top.out < n_outs {
                if !top.spender_done {
                    top.spender_done = true;
                    let k = purse_store::Outpoint {
                        hash: top.hash,
                        index: top.out,
                    };
                    if let Some(spender) = tx.get::<UnminedInputs>(&k).map_err(store_err)? {
                        let rec = read_unmined(tx, &spender)?;
                        stack.push(Frame {
                            tx: rec.into_msg_tx(),
                            hash: spender,
                            out: 0,
                            spender_done: false,
                        });
                    }
                    continue;
                }
                // The output's unmined credit (a ticket's commitment is the
                // next slice's, guarded above).
                let k = purse_store::Outpoint {
                    hash: top.hash,
                    index: top.out,
                };
                del_ucredit(tx, &k)?;
                let Some(top) = stack.last_mut() else {
                    break;
                };
                top.out = top.out.saturating_add(1);
                top.spender_done = false;
                continue;
            }
            let Some(done) = stack.pop() else {
                break;
            };
            for input in &done.tx.tx_in {
                del_uinput(tx, &canonical(&input.previous_out_point))?;
            }
            set_unpublished(tx, &done.hash, false)?;
            tx.delete::<TxUnmined>(&done.hash).map_err(store_err)?;
        }
        Ok(())
    }

    /// `PruneUnmined`: removes the unmined transactions that expired by the
    /// tip, tickets whose price is not `sdiff` (when it is not 0) and votes
    /// on another block than the tip, in key order; returns their hashes.
    ///
    /// # Errors
    ///
    /// An unmined record that does not decode: `IO`, nothing removed. A
    /// removal's error, with the hashes removed before it ([`PruneError`]).
    pub fn prune_unmined(
        &self,
        tx: &mut WriteTx<'_>,
        sdiff: i64,
    ) -> Result<Vec<Hash32>, PruneError> {
        let fail = |error| PruneError {
            removed: Vec::new(),
            error,
        };
        let (tip_hash, tip_height) = self.main_chain_tip(tx).map_err(fail)?;
        let rows = tx
            .rows::<TxUnmined>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(|e| fail(store_err(e)))?;
        let mut remove = Vec::new();
        for (hash, row) in rows {
            let (m, _) =
                MsgTx::from_bytes(&row.tx).map_err(|err| fail(io(AnyError::foreign(err))))?;
            #[allow(
                clippy::cast_sign_loss,
                reason = "Go compares with uint32(tipHeight) (txunmined.go:366)"
            )]
            let expired = m.expiry != NO_EXPIRY_VALUE && m.expiry <= tip_height as u32;
            let prune = if expired {
                true
            } else if dcroxide_stake::is_sstx(&m) {
                sdiff != 0 && m.tx_out.first().map(|o| o.value) != Some(sdiff)
            } else if dcroxide_stake::is_ssgen(&m) {
                dcroxide_stake::ssgen_block_voted_on(&m).0.0 != tip_hash.0
            } else {
                false
            };
            if prune {
                remove.push((hash, m));
            }
        }
        let mut removed = Vec::with_capacity(remove.len());
        for (hash, m) in remove {
            if let Err(error) = self.remove_unconfirmed(tx, &m, &hash) {
                return Err(PruneError { removed, error });
            }
            removed.push(hash);
        }
        Ok(removed)
    }

    /// `UnminedTxs`: every unmined record, marked unpublished as stored,
    /// each after the unmined transactions it spends (Kahn's order, ties
    /// broken by hash in internal byte order, ADR-0012 Rule 3(a); Go breaks
    /// them by map order, QK-0092).
    ///
    /// # Errors
    ///
    /// A record that does not decode: `IO`. The store's.
    pub fn unmined_txs(&self, r: &mut (impl TxRead + ?Sized)) -> Result<Vec<TxRecord>, AnyError> {
        let rows = r
            .rows::<TxUnmined>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        let mut recs = BTreeMap::new();
        for (hash, row) in rows {
            let unpublished = r.get::<Unpublished>(&hash).map_err(store_err)?.is_some();
            let rec = TxRecord::stored(hash, &row.tx, row.received)?.with_unpublished(unpublished);
            recs.insert(hash, rec);
        }
        Ok(dependency_sort(recs))
    }

    /// `UnminedTxHashes`: the unmined transactions' hashes in key order.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn unmined_tx_hashes(
        &self,
        r: &mut (impl TxRead + ?Sized),
    ) -> Result<Vec<Hash32>, AnyError> {
        r.keys::<TxUnmined>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)
    }
}

/// Records a ticket in the ticket set unless it is there (Go's value −1 is
/// never read and not kept, QK-0056).
pub(crate) fn put_ticket(tx: &mut WriteTx<'_>, hash: &Hash32) -> Result<(), AnyError> {
    if !tx.contains::<Tickets>(hash).map_err(store_err)? {
        tx.put::<Tickets>(hash, &Empty).map_err(store_err)?;
    }
    Ok(())
}

/// Kahn's algorithm over the records, an edge from each record to every
/// record spending one of its outputs, the ready set taken in hash order.
fn dependency_sort(mut recs: BTreeMap<Hash32, TxRecord>) -> Vec<TxRecord> {
    let mut children: BTreeMap<Hash32, BTreeSet<Hash32>> = BTreeMap::new();
    let mut in_degree: BTreeMap<Hash32, usize> = recs.keys().map(|h| (*h, 0)).collect();
    for (hash, rec) in &recs {
        let parents: BTreeSet<Hash32> = rec
            .msg_tx()
            .tx_in
            .iter()
            .map(|i| Hash32(i.previous_out_point.hash.0))
            .filter(|p| p != hash && recs.contains_key(p))
            .collect();
        for p in parents {
            if children.entry(p).or_default().insert(*hash)
                && let Some(d) = in_degree.get_mut(hash)
            {
                *d = d.saturating_add(1);
            }
        }
    }
    let mut ready: BTreeSet<Hash32> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(h, _)| *h)
        .collect();
    let mut out = Vec::with_capacity(recs.len());
    while let Some(h) = ready.pop_first() {
        for c in children.remove(&h).unwrap_or_default() {
            if let Some(d) = in_degree.get_mut(&c) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    ready.insert(c);
                }
            }
        }
        if let Some(rec) = recs.remove(&h) {
            out.push(rec);
        }
    }
    // A cycle (impossible between real hashes) leaves records over; they
    // are returned last, in hash order, rather than lost.
    out.extend(recs.into_values());
    out
}
