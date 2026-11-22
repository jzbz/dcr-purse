// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Queries over the transaction records (`wallet/udb/txquery.go`; the
//! ticket queries of `txmined.go:2351-2355`, `2725-2850`).

use std::collections::BTreeMap;
use std::ops::Bound;

use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Blocks, Hash32, Height, History, HistoryKey, TableId, Tickets, TxMined, TxMinedRow, TxRecKey,
    TxUnmined, UnminedInputs,
};

use super::chain::block_at;
use super::record::{Block, BlockMeta, Ticket, TicketDetails, TxDetails, TxRecord, display};
use super::store::{TxRead, TxStore, guard, store_err, walk};
use super::unmined::{latest_mined, latest_mined_height};

/// The tables the details of a record read in Go: its credits and debits
/// (mined), or the unspent and unmined credits its inputs spend (unmined).
const DETAILS_TABLES: [TableId; 4] = [
    TableId::Credits,
    TableId::Debits,
    TableId::Unspent,
    TableId::UnminedCredits,
];

/// Go's `int32(^uint32(0) >> 1)`: a negative range bound.
const MAX_HEIGHT: i32 = i32::MAX;

impl TxStore {
    /// The details of a mined record (`minedTxDetails`): the block's hash
    /// and time from its main-chain row, vote bits 0, as Go leaves them.
    fn mined_details(
        &self,
        r: &mut (impl TxRead + ?Sized),
        key: &TxRecKey,
        row: &TxMinedRow,
    ) -> Result<TxDetails, AnyError> {
        let record = TxRecord::stored(key.hash, &row.tx, row.received)?;
        let Some(b) = block_at(r, key.height)? else {
            return Err(e!(
                Kind::Io,
                format!("missing block record for height {}", key.height)
            )
            .into());
        };
        Ok(TxDetails {
            record,
            block: BlockMeta {
                block: Block {
                    hash: b.hash,
                    height: i32::try_from(key.height).unwrap_or(MAX_HEIGHT),
                },
                time: b.time,
                vote_bits: 0,
            },
        })
    }

    /// `TxDetails`: the unmined record of `hash`, or its last mined one.
    ///
    /// # Errors
    ///
    /// No record: `NotExist`. A record that does not decode: `IO`. The
    /// guard. The store's.
    pub fn tx_details(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<TxDetails, AnyError> {
        guard(r, "TxDetails", &DETAILS_TABLES)?;
        if let Some(row) = r.get::<TxUnmined>(hash).map_err(store_err)? {
            return Ok(TxDetails {
                record: TxRecord::stored(*hash, &row.tx, row.received)?,
                block: BlockMeta::unmined(),
            });
        }
        match latest_mined(r, hash)? {
            Some((k, row)) => self.mined_details(r, &k, &row),
            None => Err(e!(Kind::NotExist).into()),
        }
    }

    /// `UniqueTxDetails`: the record of `hash` in `block` (unmined when
    /// `None`), or `None`.
    ///
    /// The record's key holds the height, not the block hash Go's key
    /// holds, so the main-chain block at that height is read to compare
    /// its hash: one read per call.
    ///
    /// # Errors
    ///
    /// A record that does not decode: `IO`. The guard. The store's.
    pub fn unique_tx_details(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
        block: Option<&Block>,
    ) -> Result<Option<TxDetails>, AnyError> {
        guard(r, "UniqueTxDetails", &DETAILS_TABLES)?;
        let Some(block) = block else {
            return match r.get::<TxUnmined>(hash).map_err(store_err)? {
                Some(row) => Ok(Some(TxDetails {
                    record: TxRecord::stored(*hash, &row.tx, row.received)?,
                    block: BlockMeta::unmined(),
                })),
                None => Ok(None),
            };
        };
        let Ok(height) = u32::try_from(block.height) else {
            return Ok(None);
        };
        if block_at(r, height)?.map(|b| b.hash) != Some(block.hash) {
            return Ok(None);
        }
        let key = TxRecKey {
            hash: *hash,
            height,
        };
        match r.get::<TxMined>(&key).map_err(store_err)? {
            Some(row) => self.mined_details(r, &key, &row).map(Some),
            None => Ok(None),
        }
    }

    /// `Tx`: the decoded transaction of `hash`, unmined or last mined.
    ///
    /// # Errors
    ///
    /// No record: `NotExist`, Go's text. A record that does not decode:
    /// `IO`. The store's.
    pub fn tx(&self, r: &mut (impl TxRead + ?Sized), hash: &Hash32) -> Result<MsgTx, AnyError> {
        if let Some(row) = r.get::<TxUnmined>(hash).map_err(store_err)? {
            return Ok(TxRecord::stored(*hash, &row.tx, row.received)?.into_msg_tx());
        }
        match latest_mined(r, hash)? {
            Some((_, row)) => Ok(TxRecord::stored(*hash, &row.tx, row.received)?.into_msg_tx()),
            None => Err(e!(Kind::NotExist, format!("tx {} not found", display(hash))).into()),
        }
    }

    /// `ExistsTxMinedOrUnmined`: whether `hash` is recorded mined, and
    /// whether unmined (never both: an unmined record answers first).
    ///
    /// # Errors
    ///
    /// The store's (Go's returns none, ADR-0012 Rule 2(c)).
    pub fn exists_tx_mined_or_unmined(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<(bool, bool), AnyError> {
        if r.contains::<TxUnmined>(hash).map_err(store_err)? {
            return Ok((false, true));
        }
        Ok((latest_mined_height(r, hash)?.is_some(), false))
    }

    /// `ExistsTx`.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn exists_tx(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<bool, AnyError> {
        let (m, u) = self.exists_tx_mined_or_unmined(r, hash)?;
        Ok(m || u)
    }

    /// `TxBlockHeight`: -1 when unmined, else the last mined record's
    /// height.
    ///
    /// # Errors
    ///
    /// No record: `NotExist`, Go's text. The store's.
    pub fn tx_block_height(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<i32, AnyError> {
        if r.contains::<TxUnmined>(hash).map_err(store_err)? {
            return Ok(-1);
        }
        match latest_mined_height(r, hash)? {
            Some(height) => Ok(i32::try_from(height).unwrap_or(MAX_HEIGHT)),
            None => Err(e!(Kind::NotExist, format!("no transaction {}", display(hash))).into()),
        }
    }

    /// The heights Go's block iterator walks from `begin` to `end`
    /// (`rangeBlockTransactions`, `RangeBlocks`), lowest and highest, and
    /// whether downwards: negative bounds are the highest height; upwards
    /// when `begin < end`, from the first block at or above `begin`;
    /// downwards otherwise, from the last block at or below `begin`.
    fn walk_bounds(begin: i32, end: i32) -> (u32, u32, bool) {
        let begin = if begin < 0 { MAX_HEIGHT } else { begin }.unsigned_abs();
        let end = if end < 0 { MAX_HEIGHT } else { end }.unsigned_abs();
        if begin < end {
            (begin, end, false)
        } else {
            (end, begin, true)
        }
    }

    /// `RangeTransactions`: calls `f` with the details of each main-chain
    /// block's records from `begin` to `end` (see `RangeBlocks` for the
    /// walk), one call per block with records, and with every unmined
    /// record in one call first when `begin` is negative, or last when
    /// `end` is; `f` returns `true` to stop. Takes no context (ADR-0008).
    ///
    /// The walk reads the `history` rows of the range, page by page, and
    /// the block row of each block that has any, so a block without wallet
    /// transactions costs no read at all, where Go's cursor reads its block
    /// record; a walk `f` stops reads about as many rows as it visits
    /// (`store::walk`).
    ///
    /// # Errors
    ///
    /// `f`'s. A record or block row missing, or a record not decoding:
    /// `IO`. The guard. The store's.
    pub fn range_transactions(
        &self,
        r: &mut (impl TxRead + ?Sized),
        begin: i32,
        end: i32,
        mut f: impl FnMut(&[TxDetails]) -> Result<bool, AnyError>,
    ) -> Result<(), AnyError> {
        guard(r, "RangeTransactions", &DETAILS_TABLES)?;
        let mut added_unmined = false;
        if begin < 0 {
            if self.range_unmined(r, &mut f)? {
                return Ok(());
            }
            added_unmined = true;
        }
        let (lo, hi, rev) = Self::walk_bounds(begin, end);
        // The records of the block being read: its height and hashes, in
        // the order the walk meets them.
        let mut pending: Option<(u32, Vec<Hash32>)> = None;
        let stopped = walk::<History, _, _>(
            r,
            Bound::Included(HistoryKey {
                height: lo,
                position: 0,
            }),
            Bound::Included(HistoryKey {
                height: hi,
                position: u32::MAX,
            }),
            rev,
            |r, k, txh| match &mut pending {
                Some((h, txs)) if *h == k.height => {
                    txs.push(txh);
                    Ok(false)
                }
                _ => {
                    let done = match pending.replace((k.height, vec![txh])) {
                        Some((h, txs)) => Self::block_details(r, h, txs, rev, &mut f)?,
                        None => false,
                    };
                    Ok(done)
                }
            },
        )?;
        if stopped {
            return Ok(());
        }
        if let Some((h, txs)) = pending.take()
            && Self::block_details(r, h, txs, rev, &mut f)?
        {
            return Ok(());
        }
        if !added_unmined && end < 0 {
            self.range_unmined(r, &mut f)?;
        }
        Ok(())
    }

    /// Calls `f` with the details of the records `txs` of the main-chain
    /// block at `h`, met in reverse position order when `rev`.
    fn block_details(
        r: &mut (impl TxRead + ?Sized),
        h: u32,
        mut txs: Vec<Hash32>,
        rev: bool,
        f: &mut impl FnMut(&[TxDetails]) -> Result<bool, AnyError>,
    ) -> Result<bool, AnyError> {
        if rev {
            txs.reverse();
        }
        let Some(b) = block_at(r, h)? else {
            return Err(e!(Kind::Io, format!("missing block record for height {h}")).into());
        };
        let mut details = Vec::with_capacity(txs.len());
        for txh in txs {
            let key = TxRecKey {
                hash: txh,
                height: h,
            };
            let Some(row) = r.get::<TxMined>(&key).map_err(store_err)? else {
                return Err(e!(
                    Kind::Io,
                    format!("missing transaction {} for block {h}", display(&txh))
                )
                .into());
            };
            details.push(TxDetails {
                record: TxRecord::stored(txh, &row.tx, row.received)?,
                block: BlockMeta {
                    block: Block {
                        hash: b.hash,
                        height: i32::try_from(h).unwrap_or(MAX_HEIGHT),
                    },
                    time: b.time,
                    vote_bits: 0,
                },
            });
        }
        f(&details)
    }

    /// `rangeUnminedTransactions`: every unmined record in key order, in
    /// one call, when there is one.
    fn range_unmined(
        &self,
        r: &mut (impl TxRead + ?Sized),
        f: &mut impl FnMut(&[TxDetails]) -> Result<bool, AnyError>,
    ) -> Result<bool, AnyError> {
        let rows = r
            .rows::<TxUnmined>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        if rows.is_empty() {
            return Ok(false);
        }
        let mut details = Vec::with_capacity(rows.len());
        for (hash, row) in rows {
            details.push(TxDetails {
                record: TxRecord::stored(hash, &row.tx, row.received)?,
                block: BlockMeta::unmined(),
            });
        }
        f(&details)
    }

    /// `RangeBlocks`: calls `f` with each main-chain block from `begin` to
    /// `end`; `f` returns `true` to stop.
    ///
    /// # Errors
    ///
    /// `f`'s. The store's.
    pub fn range_blocks(
        &self,
        r: &mut (impl TxRead + ?Sized),
        begin: i32,
        end: i32,
        mut f: impl FnMut(&Block) -> Result<bool, AnyError>,
    ) -> Result<(), AnyError> {
        let (lo, hi, rev) = Self::walk_bounds(begin, end);
        walk::<Blocks, _, _>(
            r,
            Bound::Included(Height(lo)),
            Bound::Included(Height(hi)),
            rev,
            |_, h, b| {
                f(&Block {
                    hash: b.hash,
                    height: i32::try_from(h.0).unwrap_or(MAX_HEIGHT),
                })
            },
        )?;
        Ok(())
    }

    /// `OwnTicket`: whether `hash` is in the ticket set.
    ///
    /// # Errors
    ///
    /// The store's (Go's returns none).
    pub fn own_ticket(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<bool, AnyError> {
        r.contains::<Tickets>(hash).map_err(store_err)
    }

    /// `IterateTickets` with `TicketIterator.Next`: every ticket of the
    /// ticket set that is recorded, mined (its last record) or unmined, in
    /// hash order, with the unmined transaction spending its output 0.
    ///
    /// A mined record's key holds the height, not the block hash Go's key
    /// holds, so the block's hash comes from its main-chain row. Each
    /// height's hash is kept once read, so the walk reads one block row per
    /// distinct block holding a mined ticket, where Go reads none; tickets
    /// come in hash order, not block order, so keeping only the last block
    /// read would save little.
    ///
    /// # Errors
    ///
    /// A record that does not decode, or a block row missing: `IO`. The
    /// guard. The store's.
    pub fn iterate_tickets(&self, r: &mut (impl TxRead + ?Sized)) -> Result<Vec<Ticket>, AnyError> {
        guard(r, "TicketIterator.Next", &[TableId::Credits])?;
        let hashes = r
            .keys::<Tickets>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        let mut out = Vec::with_capacity(hashes.len());
        // The hashes of the blocks read, by height.
        let mut blocks: BTreeMap<u32, Hash32> = BTreeMap::new();
        for hash in hashes {
            if let Some((k, row)) = latest_mined(r, &hash)? {
                let block_hash = match blocks.get(&k.height) {
                    Some(b) => *b,
                    None => {
                        let Some(b) = block_at(r, k.height)? else {
                            return Err(e!(
                                Kind::Io,
                                format!("missing block record for height {}", k.height)
                            )
                            .into());
                        };
                        blocks.insert(k.height, b.hash);
                        b.hash
                    }
                };
                let spender = self.output0_spender(r, &hash)?;
                out.push(Ticket {
                    record: TxRecord::stored(hash, &row.tx, row.received)?,
                    block: Block {
                        hash: block_hash,
                        height: i32::try_from(k.height).unwrap_or(MAX_HEIGHT),
                    },
                    spender,
                });
            } else if let Some(row) = r.get::<TxUnmined>(&hash).map_err(store_err)? {
                out.push(Ticket {
                    record: TxRecord::stored(hash, &row.tx, row.received)?,
                    block: Block {
                        hash: Hash32::default(),
                        height: -1,
                    },
                    spender: Hash32::default(),
                });
            }
        }
        Ok(out)
    }

    /// The unmined spender of output 0 of `hash`, zero when none.
    fn output0_spender(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<Hash32, AnyError> {
        let k = purse_store::Outpoint {
            hash: *hash,
            index: 0,
        };
        Ok(r.get::<UnminedInputs>(&k)
            .map_err(store_err)?
            .unwrap_or_default())
    }

    /// `TicketDetails`: for a ticket's details, the ticket and the details
    /// of the transaction spending its output 0 (`None` for a transaction
    /// that is not a ticket).
    ///
    /// # Errors
    ///
    /// The spender's details' errors other than `NotExist`. The guard. The
    /// store's.
    pub fn ticket_details(
        &self,
        r: &mut (impl TxRead + ?Sized),
        details: &TxDetails,
    ) -> Result<Option<TicketDetails>, AnyError> {
        if !dcroxide_stake::is_sstx(details.record.msg_tx()) {
            return Ok(None);
        }
        guard(r, "TicketDetails", &[TableId::Credits])?;
        let spender_hash = self.output0_spender(r, details.record.hash())?;
        let spender = match self.tx_details(r, &spender_hash) {
            Ok(d) => Some(d),
            Err(err) if err.is_kind(Kind::NotExist) => None,
            Err(err) => return Err(err),
        };
        Ok(Some(TicketDetails {
            ticket: details.clone(),
            spender,
        }))
    }

    /// `UnspentOutputCount`: the number of unspent outputs (the length of
    /// `unspent`, empty in the first slice).
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn unspent_output_count(&self, r: &mut (impl TxRead + ?Sized)) -> Result<u64, AnyError> {
        r.table_len(TableId::Unspent).map_err(store_err)
    }
}
