// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Mined transactions: `InsertMinedTx` with `moveMinedTx` and
//! `removeDoubleSpends`, and `Rollback` (`txmined.go:1053-1375`,
//! `1874-2240`; `txunmined.go:149-173`).

use std::ops::Bound;

use dcroxide_stake::TxType;
use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Blocks, Hash32, Height, History, HistoryKey, TableId, TxMined, TxMinedRow, TxRecKey, TxUnmined,
    TxUnminedRow, UnminedInputs, Unpublished, WriteTx,
};

use super::chain::{block_at, header_of, tip};
use super::record::{Block, TxRecord, canonical, display};
use super::store::{TxRead, TxStore, guard, store_err};
use super::unmined::{put_ticket, read_unmined};

/// The tables `InsertMinedTx` and `moveMinedTx` read and change in Go
/// beside the transaction records: each input's unspent entry, the
/// debits, the credits it spends, each output's unmined credit.
const INSERT_MINED_TABLES: [TableId; 6] = [
    TableId::Unspent,
    TableId::Credits,
    TableId::Debits,
    TableId::DebitsInvalidated,
    TableId::UnminedCredits,
    TableId::Balances,
];

/// The tables `Rollback` reads and changes in Go beside the transaction
/// records and the chain: each output's credit and unspent entry, each
/// input's debit, the multisig records, a vote's or revocation's
/// commitment flags.
const ROLLBACK_TABLES: [TableId; 9] = [
    TableId::Credits,
    TableId::Unspent,
    TableId::UnminedCredits,
    TableId::Debits,
    TableId::MultisigOuts,
    TableId::MultisigUnspent,
    TableId::Commitments,
    TableId::CommitmentsUnspent,
    TableId::Balances,
];

/// The transactions `Rollback` removed from one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolledBack {
    /// The block they were mined in.
    pub block: Block,
    /// Its transactions, in the order they were recorded.
    pub txs: Vec<MsgTx>,
}

/// `compat.IsEitherCoinBaseTx`: a coinbase before or after the treasury
/// agenda.
fn is_either_coinbase(tx: &MsgTx) -> bool {
    dcroxide_standalone::is_coin_base_tx(tx, false)
        || dcroxide_standalone::is_coin_base_tx(tx, true)
}

impl TxStore {
    /// `InsertMinedTx`: records `rec` as mined in the main-chain block
    /// `block_hash`. A transaction recorded unmined is moved into the
    /// block, its record keeping the unmined record's received time and
    /// bytes, as Go copies the stored value; otherwise the unmined
    /// transactions it double spends are removed with their spenders, and
    /// the record is written. A ticket is added to the ticket set.
    ///
    /// # Errors
    ///
    /// Go's: no header for `block_hash`: `Invalid`; a block not in the
    /// main chain: `Invalid` (also where no main-chain block stands at the
    /// header's height, where Go panics, QK-0086); an unmined regular
    /// transaction into a stake-invalidated block: `Invalid`, with Go's
    /// panic text (Go does not implement it). The guard. The store's.
    pub fn insert_mined_tx(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        block_hash: &Hash32,
    ) -> Result<(), AnyError> {
        let Some(header) = header_of(tx, block_hash)? else {
            return Err(e!(Kind::Invalid, "block header must be recorded").into());
        };
        let height = header.height();
        let row = match block_at(tx, height)? {
            Some(row) if row.hash == *block_hash => row,
            _ => {
                return Err(e!(
                    Kind::Invalid,
                    "mined transactions must be added to main chain blocks"
                )
                .into());
            }
        };
        let tx_type = dcroxide_stake::determine_tx_type(rec.msg_tx());
        let invalidated = tx_type == TxType::Regular && row.invalid;
        // Each input's unspent credit and its debit: the credit slice's.
        if !rec.msg_tx().tx_in.is_empty() {
            guard(tx, "InsertMinedTx", &INSERT_MINED_TABLES)?;
        }
        let hash = *rec.hash();
        let key = TxRecKey { hash, height };
        if tx.contains::<TxMined>(&key).map_err(store_err)? {
            return Ok(());
        }
        // Go records a ticket before it moves or removes anything; here the
        // ticket row is written after the refusals and guards those steps
        // make, so that a refusal or a guard stop writes nothing (the same
        // rows on success: neither step touches the ticket set).
        if let Some(unmined) = tx.get::<TxUnmined>(&hash).map_err(store_err)? {
            if invalidated {
                return Err(e!(
                    Kind::Invalid,
                    format!(
                        "unimplemented: moveMinedTx called on a stake-invalidated tx: block {} height {} tx {}",
                        display(block_hash),
                        height,
                        display(&hash)
                    )
                )
                .into());
            }
            // Each output's unmined credit becoming a mined one: the credit
            // slice's (guarded above for a transaction with inputs).
            guard(tx, "moveMinedTx", &[TableId::UnminedCredits])?;
            if tx_type == TxType::SStx {
                put_ticket(tx, &hash)?;
            }
            return self.move_mined_tx(tx, rec, &key, unmined);
        }
        self.remove_double_spends(tx, rec)?;
        if tx_type == TxType::SStx {
            put_ticket(tx, &hash)?;
        }
        append_to_block(tx, &key, rec.received(), rec.serialized().to_vec())
    }

    /// `moveMinedTx`: the unmined record of `rec` moved into its block (the
    /// caller has made the guard of its unmined credits).
    fn move_mined_tx(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        key: &TxRecKey,
        unmined: TxUnminedRow,
    ) -> Result<(), AnyError> {
        append_to_block(tx, key, unmined.received, unmined.tx)?;
        for input in &rec.msg_tx().tx_in {
            tx.delete::<UnminedInputs>(&canonical(&input.previous_out_point))
                .map_err(store_err)?;
        }
        tx.delete::<Unpublished>(&key.hash).map_err(store_err)?;
        tx.delete::<TxUnmined>(&key.hash).map_err(store_err)?;
        Ok(())
    }

    /// `removeDoubleSpends`: every unmined transaction spending an output
    /// `rec` spends is removed with its spenders.
    fn remove_double_spends(&self, tx: &mut WriteTx<'_>, rec: &TxRecord) -> Result<(), AnyError> {
        for input in &rec.msg_tx().tx_in {
            let k = canonical(&input.previous_out_point);
            if let Some(spender) = tx.get::<UnminedInputs>(&k).map_err(store_err)? {
                let ds = read_unmined(tx, &spender)?;
                self.remove_unconfirmed(tx, ds.msg_tx(), &spender)?;
            }
        }
        Ok(())
    }

    /// `Rollback`: detaches every main-chain block at `height` and above,
    /// the tip first. Each block's transactions are moved back to the
    /// unmined set with their record bytes, their inputs registered in the
    /// unmined-inputs index (a vote's stakebase excepted, over another
    /// spender's rows, as Go), coinbases dropped; the blocks' headers and
    /// filters are kept as detached ones. Returns each block's
    /// transactions in record order, highest block first, for the blocks
    /// that had any (Go's map has no entry for the others).
    ///
    /// # Errors
    ///
    /// Go's: height 0: `Invalid`. A height below 0 or two or more above
    /// the tip, where Go panics (after removing every block for a negative
    /// height): `Invalid`, nothing written. A recorded transaction that
    /// does not decode, or a record or block row missing: `IO`. The guard.
    /// The store's.
    pub fn rollback(&self, tx: &mut WriteTx<'_>, height: i32) -> Result<Vec<RolledBack>, AnyError> {
        if height == 0 {
            return Err(e!(Kind::Invalid, "cannot rollback the genesis block").into());
        }
        let t = tip(tx)?;
        let from = match u32::try_from(height) {
            Ok(h) if h <= t.height.saturating_add(1) => h,
            _ => {
                return Err(e!(
                    Kind::Invalid,
                    format!(
                        "cannot rollback to height {height}: the main chain tip is at height {}",
                        t.height
                    )
                )
                .into());
            }
        };
        if from > t.height {
            return Ok(Vec::new());
        }
        guard(tx, "Rollback", &ROLLBACK_TABLES)?;
        // The records of the blocks from `from` up, read in one range of
        // `history` (a block without wallet transactions has no row, so
        // costs no read), grouped by block.
        let history = tx
            .rows::<History>(
                Bound::Included(&HistoryKey {
                    height: from,
                    position: 0,
                }),
                Bound::Unbounded,
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
        let mut blocks: Vec<(u32, Vec<(HistoryKey, Hash32)>)> = Vec::new();
        for (hk, txh) in history {
            match blocks.last_mut() {
                Some((h, rows)) if *h == hk.height => rows.push((hk, txh)),
                _ => blocks.push((hk.height, vec![(hk, txh)])),
            }
        }
        let mut out = Vec::with_capacity(blocks.len());
        // Highest block first, each block's records in position order.
        for (h, history) in blocks.into_iter().rev() {
            let Some(row) = block_at(tx, h)? else {
                return Err(e!(Kind::Io, format!("missing block record for height {h}")).into());
            };
            let mut txs = Vec::with_capacity(history.len());
            for (hk, txh) in history {
                let key = TxRecKey {
                    hash: txh,
                    height: h,
                };
                let Some(mined) = tx.get::<TxMined>(&key).map_err(store_err)? else {
                    return Err(e!(Kind::Io, "tx record len 0").into());
                };
                let rec = TxRecord::stored(txh, &mined.tx, mined.received)?;
                tx.delete::<TxMined>(&key).map_err(store_err)?;
                tx.delete::<History>(&hk).map_err(store_err)?;
                let tx_type = rec.tx_type();
                let msg = rec.into_msg_tx();
                if !is_either_coinbase(&msg) {
                    tx.put::<TxUnmined>(
                        &txh,
                        &TxUnminedRow {
                            received: mined.received,
                            tx: mined.tx,
                        },
                    )
                    .map_err(store_err)?;
                    for (i, input) in msg.tx_in.iter().enumerate() {
                        if i == 0 && tx_type == TxType::SSGen {
                            continue;
                        }
                        tx.put::<UnminedInputs>(&canonical(&input.previous_out_point), &txh)
                            .map_err(store_err)?;
                    }
                }
                txs.push(msg);
            }
            // Go's map has an entry only for a block with transactions,
            // which every group here is.
            out.push(RolledBack {
                block: Block {
                    hash: row.hash,
                    height: i32::try_from(h).unwrap_or(i32::MAX),
                },
                txs,
            });
        }
        tx.disconnect_from(from).map_err(store_err)?;
        Ok(out)
    }
}

/// Appends a record to its block: the `history` row at the block's next
/// position, the block row's count, and the `tx_mined` row (Go's
/// `appendRawBlockRecord` and `putTxRecord`).
fn append_to_block(
    tx: &mut WriteTx<'_>,
    key: &TxRecKey,
    received: i64,
    bytes: Vec<u8>,
) -> Result<(), AnyError> {
    let Some(mut row) = block_at(tx, key.height)? else {
        return Err(e!(Kind::Io, "block record len 0").into());
    };
    let position = row.wallet_txs;
    row.wallet_txs = position
        .checked_add(1)
        .ok_or_else(|| AnyError::from(e!(Kind::Io, "block record holds 2^32 transactions")))?;
    tx.put::<Blocks>(&Height(key.height), &row)
        .map_err(store_err)?;
    tx.put::<History>(
        &HistoryKey {
            height: key.height,
            position,
        },
        &key.hash,
    )
    .map_err(store_err)?;
    tx.put::<TxMined>(
        key,
        &TxMinedRow {
            position,
            received,
            tx: bytes,
        },
    )
    .map_err(store_err)
}
