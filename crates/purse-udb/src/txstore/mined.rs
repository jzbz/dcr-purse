// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Mined transactions: `InsertMinedTx` with `moveMinedTx` and
//! `removeDoubleSpends`, `Rollback`, and the credit and debit moves of
//! stake (in)validation (`txmined.go:631-885`, `1053-1375`, `1874-2240`;
//! `txunmined.go:149-173`).

use std::collections::BTreeMap;
use std::ops::Bound;

use dcroxide_stake::TxType;
use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Blocks, CreditFlags, CreditKey, CreditRow, Credits, CreditsInvalidated, DebitRow, Debits,
    DebitsInvalidated, Hash32, Height, History, HistoryKey, Outpoint, Spender, TxMined, TxMinedRow,
    TxRecKey, TxUnmined, TxUnminedRow, UnminedInputs, Unspent, UnspentRow, WriteTx,
};

use super::chain::{block_at, header_of, tip};
use super::commitment::replace_unmined_spent;
use super::credit::{
    AcctTracker, del_ucredit, del_uinput, del_unspent, del_unspent_row, is_either_coinbase,
    put_ucredit, put_uinput, put_unspent, script_loc, set_unpublished, unspent_row,
};
use super::multisig::{any_multisig, mark_mined, mark_spent, mark_unmined, mark_unspent};
use super::record::{Block, TxRecord, canonical, display};
use super::store::{TxRead, TxStore, store_err};
use super::unmined::{put_ticket, read_unmined};

/// The transactions `Rollback` removed from one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolledBack {
    /// The block they were mined in.
    pub block: Block,
    /// Its transactions, in the order they were recorded.
    pub txs: Vec<MsgTx>,
}

/// `spendCredit` (`txdb.go:942-955`): marks the credit `key` spent by
/// input `input` of the mined record (`hash`, `height`) and returns it.
///
/// # Errors
///
/// No such credit, where Go writes a zero-amount spent credit (QK-0101,
/// reachable only through state no operation writes here): `IO`.
fn spend_credit(
    tx: &mut WriteTx<'_>,
    acct: &mut AcctTracker,
    key: &CreditKey,
    spender: Spender,
) -> Result<CreditRow, AnyError> {
    let Some(mut c) = tx.get::<Credits>(key).map_err(store_err)? else {
        return Err(e!(
            Kind::Io,
            format!(
                "missing credit {}:{} at height {}, spent by {}",
                display(&key.hash),
                key.index,
                key.height,
                display(&spender.hash)
            )
        )
        .into());
    };
    if let Some(old) = c.spender {
        acct.touch(tx, &old.hash, old.height)?;
    }
    acct.touch(tx, &spender.hash, spender.height)?;
    c.spender = Some(spender);
    tx.put::<Credits>(key, &c).map_err(store_err)?;
    Ok(c)
}

/// `unspendRawCredit` (`txdb.go:959-976`): marks the credit `key` unspent
/// and returns it, or `None` when no credit has the key.
fn unspend_credit(
    tx: &mut WriteTx<'_>,
    acct: &mut AcctTracker,
    key: &CreditKey,
) -> Result<Option<CreditRow>, AnyError> {
    let Some(mut c) = tx.get::<Credits>(key).map_err(store_err)? else {
        return Ok(None);
    };
    if let Some(old) = c.spender {
        acct.touch(tx, &old.hash, old.height)?;
    }
    c.spender = None;
    tx.put::<Credits>(key, &c).map_err(store_err)?;
    Ok(Some(c))
}

/// Records the debit of input `input` of the mined record (`hash`,
/// `height`) spending the unspent output `prev`, whose row the caller has
/// read as `u` (`InsertMinedTx`'s and `moveMinedTx`'s live branch): the
/// credit marked spent, the debit written with the credit's amount (as
/// Go's `spendCredit` returns it), the unspent output deleted without a
/// second read of its row.
fn debit_unspent(
    tx: &mut WriteTx<'_>,
    acct: &mut AcctTracker,
    prev: &Outpoint,
    u: &UnspentRow,
    hash: &Hash32,
    height: u32,
    input: u32,
) -> Result<(), AnyError> {
    let ck = CreditKey {
        hash: prev.hash,
        height: u.height,
        index: prev.index,
    };
    let c = spend_credit(
        tx,
        acct,
        &ck,
        Spender {
            hash: *hash,
            height,
            input,
        },
    )?;
    tx.put::<Debits>(
        &CreditKey {
            hash: *hash,
            height,
            index: input,
        },
        &DebitRow {
            amount: c.amount,
            credit: ck,
        },
    )
    .map_err(store_err)?;
    del_unspent_row(tx, prev, u)
}

impl TxStore {
    /// `InsertMinedTx`: records `rec` as mined in the main-chain block
    /// `block_hash`. Each input spending an unspent wallet output is
    /// debited (in a stake-invalidated block, a regular transaction's
    /// debits are recorded invalidated and spend nothing). A transaction
    /// recorded unmined is moved into the block, its record keeping the
    /// unmined record's received time and bytes, as Go copies the stored
    /// value, and its unmined credits becoming mined ones; otherwise the
    /// unmined transactions it double spends are removed with their
    /// spenders, and the record is written. A ticket is added to the
    /// ticket set.
    ///
    /// # Errors
    ///
    /// Go's: no header for `block_hash`: `Invalid`; a block not in the
    /// main chain: `Invalid` (also where no main-chain block stands at the
    /// header's height, where Go panics, QK-0086); an unmined regular
    /// transaction into a stake-invalidated block: `Invalid`, with Go's
    /// panic text (Go does not implement it), nothing written. A spent
    /// output's credit missing (QK-0101): `IO`. The store's.
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
        let hash = *rec.hash();
        let key = TxRecKey { hash, height };
        let exists = tx.contains::<TxMined>(&key).map_err(store_err)?;
        let unmined = if exists {
            None
        } else {
            tx.get::<TxUnmined>(&hash).map_err(store_err)?
        };
        // Go writes the debits of the input loop and then panics; the
        // refusal comes first here, so that it writes nothing.
        if unmined.is_some() && invalidated {
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
        let mut acct = AcctTracker::default();
        acct.touch(tx, &hash, height)?;
        for (i, input) in rec.msg_tx().tx_in.iter().enumerate() {
            let prev = canonical(&input.previous_out_point);
            let Some(u) = tx.get::<Unspent>(&prev).map_err(store_err)? else {
                continue;
            };
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            if invalidated {
                let ck = CreditKey {
                    hash: prev.hash,
                    height: u.height,
                    index: prev.index,
                };
                let Some(c) = tx.get::<Credits>(&ck).map_err(store_err)? else {
                    return Err(e!(Kind::Io, "credit len 0").into());
                };
                tx.put::<DebitsInvalidated>(
                    &CreditKey {
                        hash,
                        height,
                        index: i,
                    },
                    &DebitRow {
                        amount: c.amount,
                        credit: ck,
                    },
                )
                .map_err(store_err)?;
            } else {
                debit_unspent(tx, &mut acct, &prev, &u, &hash, height, i)?;
            }
        }
        if exists {
            return acct.flush(tx);
        }
        if tx_type == TxType::SStx {
            put_ticket(tx, &hash)?;
        }
        if let Some(unmined) = unmined {
            self.move_mined_tx(tx, &mut acct, rec, &key, unmined)?;
            return acct.flush(tx);
        }
        self.remove_double_spends(tx, rec)?;
        append_to_block(tx, &key, rec.received(), rec.serialized().to_vec())?;
        acct.flush(tx)
    }

    /// `moveMinedTx`: the unmined record of `rec` moved into its block,
    /// with its unmined credits.
    fn move_mined_tx(
        &self,
        tx: &mut WriteTx<'_>,
        acct: &mut AcctTracker,
        rec: &TxRecord,
        key: &TxRecKey,
        unmined: TxUnminedRow,
    ) -> Result<(), AnyError> {
        append_to_block(tx, key, unmined.received, unmined.tx)?;
        for (i, input) in rec.msg_tx().tx_in.iter().enumerate() {
            let prev = canonical(&input.previous_out_point);
            let u = tx.get::<Unspent>(&prev).map_err(store_err)?;
            del_uinput(tx, &prev)?;
            // Dead in Go (QK-0080): `InsertMinedTx` has consumed every
            // unspent output the transaction spends.
            if let Some(u) = u {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                debit_unspent(tx, acct, &prev, &u, &key.hash, key.height, i)?;
            }
        }
        let n = u32::try_from(rec.msg_tx().tx_out.len()).unwrap_or(u32::MAX);
        for index in 0..n {
            let op = Outpoint {
                hash: key.hash,
                index,
            };
            let Some(m) = del_ucredit(tx, &op)? else {
                continue;
            };
            // Go rebuilds the credit without its expiry flag (QK-0079).
            let credit = CreditRow {
                amount: m.amount,
                flags: CreditFlags {
                    expiry: false,
                    ..m.flags
                },
                account: m.account,
                script: m.script,
                spender: None,
            };
            tx.put::<Credits>(
                &CreditKey {
                    hash: key.hash,
                    height: key.height,
                    index,
                },
                &credit,
            )
            .map_err(store_err)?;
            put_unspent(tx, &op, &unspent_row(key.height, &credit))?;
        }
        set_unpublished(tx, &key.hash, false)?;
        tx.delete::<TxUnmined>(&key.hash).map_err(store_err)?;
        Ok(())
    }

    /// `removeDoubleSpends`: every unmined transaction spending an output
    /// `rec` spends is removed with its spenders.
    fn remove_double_spends(&self, tx: &mut WriteTx<'_>, rec: &TxRecord) -> Result<(), AnyError> {
        for input in &rec.msg_tx().tx_in {
            let k = canonical(&input.previous_out_point);
            if let Some(spender) = tx
                .get::<purse_store::UnminedInputs>(&k)
                .map_err(store_err)?
            {
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
    /// spender's rows, as Go), the credits their debits spent restored to
    /// the unspent set (a multisig record of the restored output marked
    /// unspent), and their credits made unmined credits (a multisig record
    /// of each marked unmined, whatever block it names, as Go's is:
    /// QK-0789); a rolled-back vote or revocation marks the
    /// commitments it redeemed as spent by an unmined transaction again.
    /// Coinbases are dropped with their credits (a multisig record of each
    /// is marked unmined and kept, as Go keeps it), and afterwards the
    /// unmined transactions spending a dropped coinbase credit are removed.
    /// The
    /// detached blocks' invalidated credits and debits are deleted, which
    /// Go leaves (ADR-0003 Decision item 5; QK-0081). The blocks' headers
    /// and filters are kept as detached ones. Returns each block's
    /// transactions in record order, highest block first, for the blocks
    /// that had any (Go's map has no entry for the others).
    ///
    /// # Errors
    ///
    /// Go's: height 0: `Invalid`. A height below 0 or two or more above
    /// the tip, where Go panics (after removing every block for a negative
    /// height): `Invalid`, nothing written. A recorded transaction that
    /// does not decode, or a record, block row or debited credit missing:
    /// `IO`. The store's.
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
        // One emptiness test of the multisig records, which most wallets
        // never hold, instead of one lookup per credit output and restored
        // debit.
        let ms = any_multisig(tx)?;
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
        rollback_check(tx, &blocks)?;
        let mut acct = AcctTracker::default();
        let mut coinbase_credits: Vec<Outpoint> = Vec::new();
        let mut out = Vec::with_capacity(blocks.len());
        // Highest block first, each block's records in position order.
        for (h, history) in blocks.into_iter().rev() {
            let Some(row) = block_at(tx, h)? else {
                return Err(e!(Kind::Io, format!("missing block record for height {h}")).into());
            };
            // The credits removed from this block, which a later record of
            // the same block may have debited (Go's `removedCredits`).
            let mut removed: BTreeMap<CreditKey, CreditRow> = BTreeMap::new();
            let mut txs = Vec::with_capacity(history.len());
            for (hk, txh) in history {
                let key = TxRecKey {
                    hash: txh,
                    height: h,
                };
                let Some(mined) = tx.get::<TxMined>(&key).map_err(store_err)? else {
                    return Err(e!(Kind::Io, "tx record len 0").into());
                };
                acct.touch(tx, &txh, h)?;
                let rec = TxRecord::stored(txh, &mined.tx, mined.received)?;
                tx.delete::<TxMined>(&key).map_err(store_err)?;
                tx.delete::<History>(&hk).map_err(store_err)?;
                let tx_type = rec.tx_type();
                let msg = rec.into_msg_tx();
                let n_out = u32::try_from(msg.tx_out.len()).unwrap_or(u32::MAX);
                if is_either_coinbase(&msg) {
                    for index in 0..n_out {
                        let ck = CreditKey {
                            hash: txh,
                            height: h,
                            index,
                        };
                        let Some(c) = tx.get::<Credits>(&ck).map_err(store_err)? else {
                            continue;
                        };
                        let op = Outpoint { hash: txh, index };
                        coinbase_credits.push(op);
                        del_unspent(tx, &op)?;
                        if let Some(s) = c.spender {
                            acct.touch(tx, &s.hash, s.height)?;
                        }
                        tx.delete::<Credits>(&ck).map_err(store_err)?;
                        removed.insert(ck, c);
                        if ms {
                            mark_unmined(tx, &op)?;
                        }
                    }
                    delete_invalidated(tx, &mut acct, &txh, h, &msg)?;
                    txs.push(msg);
                    continue;
                }
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
                    let prev = canonical(&input.previous_out_point);
                    put_uinput(tx, &prev, &txh)?;
                    let dk = CreditKey {
                        hash: txh,
                        height: h,
                        index: u32::try_from(i).unwrap_or(u32::MAX),
                    };
                    let Some(d) = tx.get::<Debits>(&dk).map_err(store_err)? else {
                        continue;
                    };
                    // `rollback_check` found the credit of every debit.
                    let restored = unspend_credit(tx, &mut acct, &d.credit)?;
                    tx.delete::<Debits>(&dk).map_err(store_err)?;
                    // Go restores nothing when the credit's amount is 0,
                    // which is also its answer for a credit already removed
                    // (QK-0100).
                    let Some(c) = restored.filter(|c| c.amount != 0) else {
                        continue;
                    };
                    put_unspent(tx, &prev, &unspent_row(d.credit.height, &c))?;
                    if ms {
                        mark_unspent(tx, &prev)?;
                    }
                }
                for index in 0..n_out {
                    let ck = CreditKey {
                        hash: txh,
                        height: h,
                        index,
                    };
                    let Some(c) = tx.get::<Credits>(&ck).map_err(store_err)? else {
                        continue;
                    };
                    let script = script_loc(&msg, index)?;
                    let op = Outpoint { hash: txh, index };
                    put_ucredit(tx, &op, c.amount, c.flags, c.account, script)?;
                    if let Some(s) = c.spender {
                        acct.touch(tx, &s.hash, s.height)?;
                    }
                    tx.delete::<Credits>(&ck).map_err(store_err)?;
                    removed.insert(ck, c);
                    del_unspent(tx, &op)?;
                    if ms {
                        mark_unmined(tx, &op)?;
                    }
                }
                if matches!(tx_type, TxType::SSGen | TxType::SSRtx) {
                    replace_unmined_spent(tx, tx_type, &msg, true)?;
                }
                delete_invalidated(tx, &mut acct, &txh, h, &msg)?;
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
        for op in coinbase_credits {
            if let Some(spender) = tx
                .get::<purse_store::UnminedInputs>(&op)
                .map_err(store_err)?
            {
                let rec = read_unmined(tx, &spender)?;
                self.remove_unconfirmed(tx, rec.msg_tx(), &spender)?;
            }
        }
        acct.flush(tx)?;
        Ok(out)
    }

    /// The refusal `stake_move` would meet validating the block at
    /// `height`, made before `ExtendMainChain` writes anything: an
    /// invalidated debit of one of its regular transactions whose credit is
    /// neither a live credit nor an invalidated credit of an earlier regular
    /// transaction of the block (which validation makes live first). Go
    /// writes a zero-amount spent credit there (QK-0101); the state comes
    /// only from a spender recorded in a block below its funding
    /// transaction's.
    ///
    /// # Errors
    ///
    /// As described: `IO`. A record that does not decode: `IO`. The store's.
    pub(crate) fn stake_validate_check(
        &self,
        tx: &mut WriteTx<'_>,
        height: u32,
    ) -> Result<(), AnyError> {
        let history = tx
            .rows::<History>(
                Bound::Included(&HistoryKey {
                    height,
                    position: 0,
                }),
                Bound::Included(&HistoryKey {
                    height,
                    position: u32::MAX,
                }),
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
        let mut moved: std::collections::BTreeSet<CreditKey> = std::collections::BTreeSet::new();
        for (_, txh) in history {
            let lo = CreditKey {
                hash: txh,
                height,
                index: 0,
            };
            let hi = CreditKey {
                index: u32::MAX,
                ..lo
            };
            let debits = tx
                .rows::<DebitsInvalidated>(
                    Bound::Included(&lo),
                    Bound::Included(&hi),
                    false,
                    usize::MAX,
                )
                .map_err(store_err)?;
            let credits = TxRead::keys::<CreditsInvalidated>(
                tx,
                Bound::Included(&lo),
                Bound::Included(&hi),
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
            if debits.is_empty() && credits.is_empty() {
                continue;
            }
            let Some(mined) = tx
                .get::<TxMined>(&TxRecKey { hash: txh, height })
                .map_err(store_err)?
            else {
                continue;
            };
            if TxRecord::stored(txh, &mined.tx, mined.received)?.tx_type() != TxType::Regular {
                continue;
            }
            for (_, d) in debits {
                if !moved.contains(&d.credit)
                    && !tx.contains::<Credits>(&d.credit).map_err(store_err)?
                {
                    return Err(e!(
                        Kind::Io,
                        format!(
                            "stake validation: invalidated debit of {} spends credit {}:{} at height {}, which is not recorded",
                            display(&txh),
                            display(&d.credit.hash),
                            d.credit.index,
                            d.credit.height
                        )
                    )
                    .into());
                }
            }
            moved.extend(credits);
        }
        Ok(())
    }

    /// The credit and debit moves of `stakeInvalidate` (`invalidate`) or
    /// `stakeValidate` for the regular transactions recorded in the block
    /// at `height` (`txmined.go:631-885`), in record order; the block row's
    /// flag is the caller's.
    ///
    /// A debit of an invalidated transaction restores the unspent output
    /// of the credit it spent only while that credit is a live credit:
    /// Go restores it in every case, and an earlier transaction of the
    /// same block whose spent credit was moved to the invalidated table
    /// leaves an unspent entry with no credit (QK-0082, fixed under
    /// ADR-0012 Rule 7; PARITY.md, `purse-udb`). A revalidated record whose
    /// transaction is also unmined absorbs its unmined copy
    /// (`merge_unmined_copy`; QK-0734, fixed under Rule 7).
    ///
    /// # Errors
    ///
    /// A record missing or not decoding: `IO`, Go's text. A credit an
    /// invalidated debit names missing at validation (QK-0101): `IO`. The
    /// store's.
    pub(crate) fn stake_move(
        &self,
        tx: &mut WriteTx<'_>,
        height: u32,
        block_hash: &Hash32,
        invalidate: bool,
    ) -> Result<(), AnyError> {
        let history = tx
            .rows::<History>(
                Bound::Included(&HistoryKey {
                    height,
                    position: 0,
                }),
                Bound::Included(&HistoryKey {
                    height,
                    position: u32::MAX,
                }),
                false,
                usize::MAX,
            )
            .map_err(store_err)?;
        if history.is_empty() {
            return Ok(());
        }
        let mut acct = AcctTracker::default();
        for (_, txh) in history {
            let key = TxRecKey { hash: txh, height };
            let Some(mined) = tx.get::<TxMined>(&key).map_err(store_err)? else {
                return Err(e!(
                    Kind::Io,
                    format!(
                        "missing transaction record for tx {} block {}",
                        display(&txh),
                        display(block_hash)
                    )
                )
                .into());
            };
            let rec = TxRecord::stored(txh, &mined.tx, mined.received)?;
            if rec.tx_type() != TxType::Regular {
                continue;
            }
            acct.touch(tx, &txh, height)?;
            let msg = rec.msg_tx();
            let n_out = u32::try_from(msg.tx_out.len()).unwrap_or(u32::MAX);
            for index in 0..n_out {
                let ck = CreditKey {
                    hash: txh,
                    height,
                    index,
                };
                let op = Outpoint { hash: txh, index };
                if invalidate {
                    let Some(c) = tx.get::<Credits>(&ck).map_err(store_err)? else {
                        continue;
                    };
                    tx.delete::<Credits>(&ck).map_err(store_err)?;
                    tx.put::<CreditsInvalidated>(&ck, &c).map_err(store_err)?;
                    del_unspent(tx, &op)?;
                } else {
                    let Some(c) = tx.get::<CreditsInvalidated>(&ck).map_err(store_err)? else {
                        continue;
                    };
                    tx.delete::<CreditsInvalidated>(&ck).map_err(store_err)?;
                    tx.put::<Credits>(&ck, &c).map_err(store_err)?;
                    // Go's unconditional `putUnspent`, a spent credit
                    // included: the debit of a regular spender in this block,
                    // moved back by its own loop below, deletes the entry
                    // again; the live debit of a stake spender, or of a
                    // spender in another block, leaves it (QK-0785,
                    // reproduced, decided 2026-11-24).
                    put_unspent(tx, &op, &unspent_row(height, &c))?;
                }
            }
            for (i, input) in msg.tx_in.iter().enumerate() {
                let dk = CreditKey {
                    hash: txh,
                    height,
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                };
                let prev = canonical(&input.previous_out_point);
                if invalidate {
                    let Some(d) = tx.get::<Debits>(&dk).map_err(store_err)? else {
                        continue;
                    };
                    let restored = unspend_credit(tx, &mut acct, &d.credit)?;
                    tx.delete::<Debits>(&dk).map_err(store_err)?;
                    tx.put::<DebitsInvalidated>(&dk, &d).map_err(store_err)?;
                    if let Some(c) = restored {
                        put_unspent(tx, &prev, &unspent_row(d.credit.height, &c))?;
                    }
                } else {
                    let Some(d) = tx.get::<DebitsInvalidated>(&dk).map_err(store_err)? else {
                        continue;
                    };
                    spend_credit(
                        tx,
                        &mut acct,
                        &d.credit,
                        Spender {
                            hash: txh,
                            height,
                            input: dk.index,
                        },
                    )?;
                    del_unspent(tx, &prev)?;
                    tx.delete::<DebitsInvalidated>(&dk).map_err(store_err)?;
                    tx.put::<Debits>(&dk, &d).map_err(store_err)?;
                }
            }
            if !invalidate {
                merge_unmined_copy(tx, &mut acct, &key, block_hash, msg)?;
            }
        }
        acct.flush(tx)
    }
}

/// QK-0734's fix (ADR-0012 Rule 7; decided, project owner, 2026-11-24):
/// when stake validation revalidates the record `key` of a transaction that
/// is also unmined (mined again in a later block, which a `Rollback` then
/// returned to the unmined set), the unmined copy is merged into the
/// revalidated record as `moveMinedTx` merges an unmined transaction into
/// the block it is mined in, each row through the balance mutators. Go keeps
/// both, so each output is an unspent output and an unmined credit at once,
/// listed twice by `UnspentOutputs` and counted twice by the balances
/// (PARITY.md, `purse-udb`, item (23)). QK-0781's path to the same rows
/// (a transaction recorded mined in two blocks, the higher one rolled
/// back, the output then credited without a block) passes through no
/// revalidation and stays reproduced.
///
/// - Each input: its unmined-inputs entry deleted when it names this
///   transaction (an entry naming another transaction is that
///   transaction's, a live double spend's, and stays, where `moveMinedTx`
///   deletes it whichever transaction it names: decided, project owner,
///   2026-11-25, crate `txstore`, choice 14; Go merges nothing and keeps
///   it too); then, when the record holds no debit for the
///   input and the output it spends is an unspent wallet credit naming no
///   spender, the credit is spent and the debit written at the record's
///   block, as `moveMinedTx` debits it. That output became a credit only
///   after the record's first mining (a later `AddCredit`), so the
///   revalidated record has no invalidated debit to restore; without the
///   debit the output would be listed and selected as unspent while the
///   mined record spends it.
/// - Each output: an unmined credit the record also holds as a credit is
///   deleted (the duplicate); one the record does not hold, credited only by
///   the later mining that the `Rollback` returned to the unmined set, is
///   moved into a mined credit and an unspent output at the record's block,
///   without its expiry flag, as `moveMinedTx` moves it (QK-0079).
/// - The multisig records (QK-0789; the re-marking kept, a divergence from
///   Go: decided, project owner, 2026-11-25, crate `txstore`, choice 13):
///   an output's record that is unmined, as the `Rollback` of the later
///   block left it whatever block it named, is marked mined in the
///   revalidated block, as `AddMultisigOut` with that block would; an
///   input's record that is unspent, as the `Rollback` left it when it
///   restored the later mining's debit, is marked spent by the input, as
///   `SpendMultisigOut` would (the stake-tree flag cleared, QK-0054). Go
///   leaves both as the `Rollback` left them.
/// - The unpublished mark and the unmined record are deleted.
///
/// The debit and the moved credit can see only the rows `moveMinedTx`
/// would: a mined spender of a moved credit recorded while it was unmined
/// has no debit of it, as in Go after `InsertMinedTx` of an unmined
/// transaction.
fn merge_unmined_copy(
    tx: &mut WriteTx<'_>,
    acct: &mut AcctTracker,
    key: &TxRecKey,
    block_hash: &Hash32,
    msg: &MsgTx,
) -> Result<(), AnyError> {
    let hash = key.hash;
    if !tx.contains::<TxUnmined>(&hash).map_err(store_err)? {
        return Ok(());
    }
    let ms = any_multisig(tx)?;
    for (i, input) in msg.tx_in.iter().enumerate() {
        let prev = canonical(&input.previous_out_point);
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        if tx.get::<UnminedInputs>(&prev).map_err(store_err)? == Some(hash) {
            del_uinput(tx, &prev)?;
        }
        let dk = CreditKey {
            hash,
            height: key.height,
            index: i,
        };
        if !tx.contains::<Debits>(&dk).map_err(store_err)?
            && let Some(u) = tx.get::<Unspent>(&prev).map_err(store_err)?
        {
            let ck = CreditKey {
                hash: prev.hash,
                height: u.height,
                index: prev.index,
            };
            let unspent = tx
                .get::<Credits>(&ck)
                .map_err(store_err)?
                .is_some_and(|c| c.spender.is_none());
            if unspent {
                debit_unspent(tx, acct, &prev, &u, &hash, key.height, i)?;
            }
        }
        if ms {
            mark_spent(tx, &prev, &hash, i)?;
        }
    }
    let n_out = u32::try_from(msg.tx_out.len()).unwrap_or(u32::MAX);
    for index in 0..n_out {
        let op = Outpoint { hash, index };
        if let Some(m) = del_ucredit(tx, &op)? {
            let ck = CreditKey {
                hash,
                height: key.height,
                index,
            };
            // Stake validation has moved every invalidated credit of the
            // record back to `credits`, so a credit the record holds is
            // there.
            if !tx.contains::<Credits>(&ck).map_err(store_err)? {
                let credit = CreditRow {
                    amount: m.amount,
                    flags: CreditFlags {
                        expiry: false,
                        ..m.flags
                    },
                    account: m.account,
                    script: m.script,
                    spender: None,
                };
                tx.put::<Credits>(&ck, &credit).map_err(store_err)?;
                put_unspent(tx, &op, &unspent_row(key.height, &credit))?;
            }
        }
        if ms {
            mark_mined(tx, &op, block_hash, key.height)?;
        }
    }
    set_unpublished(tx, &hash, false)?;
    tx.delete::<TxUnmined>(&hash).map_err(store_err)?;
    Ok(())
}

/// `Rollback`'s one refusal found as it goes in Go, made before any write:
/// a debit of a record being detached whose credit is neither a live
/// credit nor one an earlier record of the same block removed (Go's
/// `removedCredits`): Go's `IO` "missing credit" after its earlier writes,
/// which its caller abandons. A credit removed by a higher block is gone by
/// the time the debit is met, as in Go, whose walk goes from the tip down.
/// Two states reach it, neither on a consensus-valid chain: a spender
/// recorded in a block below its funding transaction's, whose block was
/// rolled back first (QK-0101); and a live debit of a stake transaction
/// spending a regular transaction's credit of its own block, which stake
/// invalidation of that block moved to the invalidated table while the
/// stake debit stayed live (QK-0785; Go refuses alike until the block is
/// approved again). The text's key is dcrwallet's 72-byte credit key, its
/// block hash the main-chain block now at the credit's height (zero when
/// none), which is the one Go's key holds while that block is connected:
/// always in the second state, and in the first when one `Rollback`
/// covers both blocks.
fn rollback_check(
    tx: &mut WriteTx<'_>,
    blocks: &[(u32, Vec<(HistoryKey, Hash32)>)],
) -> Result<(), AnyError> {
    let mut seen: std::collections::BTreeSet<TxRecKey> = std::collections::BTreeSet::new();
    for (h, history) in blocks.iter().rev() {
        for (_, txh) in history {
            let lo = CreditKey {
                hash: *txh,
                height: *h,
                index: 0,
            };
            let hi = CreditKey {
                index: u32::MAX,
                ..lo
            };
            let debits = tx
                .rows::<Debits>(
                    Bound::Included(&lo),
                    Bound::Included(&hi),
                    false,
                    usize::MAX,
                )
                .map_err(store_err)?;
            for (_, d) in debits {
                let owner = TxRecKey {
                    hash: d.credit.hash,
                    height: d.credit.height,
                };
                let gone = seen.contains(&owner) && owner.height != *h;
                if gone || !tx.contains::<Credits>(&d.credit).map_err(store_err)? {
                    let block = block_at(tx, d.credit.height)?
                        .map(|b| b.hash)
                        .unwrap_or_default();
                    let mut key = Vec::with_capacity(72);
                    key.extend_from_slice(&d.credit.hash.0);
                    key.extend_from_slice(&d.credit.height.to_be_bytes());
                    key.extend_from_slice(&block.0);
                    key.extend_from_slice(&d.credit.index.to_be_bytes());
                    let key: String = key.iter().map(|b| format!("{b:02x}")).collect();
                    return Err(e!(
                        Kind::Io,
                        format!(
                            "missing credit {}:{}, key {key}, spent by {}",
                            display(&d.credit.hash),
                            d.credit.index,
                            display(txh)
                        )
                    )
                    .into());
                }
            }
            seen.insert(TxRecKey {
                hash: *txh,
                height: *h,
            });
        }
    }
    Ok(())
}

/// Deletes the invalidated credits and debits of the detached record
/// (`hash`, `height`) (ADR-0003 Decision item 5: Go leaves them, QK-0081),
/// noting for `acct_history` the record spending each deleted credit.
fn delete_invalidated(
    tx: &mut WriteTx<'_>,
    acct: &mut AcctTracker,
    hash: &Hash32,
    height: u32,
    msg: &MsgTx,
) -> Result<(), AnyError> {
    let n_out = u32::try_from(msg.tx_out.len()).unwrap_or(u32::MAX);
    for index in 0..n_out {
        let ck = CreditKey {
            hash: *hash,
            height,
            index,
        };
        if let Some(c) = tx.get::<CreditsInvalidated>(&ck).map_err(store_err)? {
            if let Some(s) = c.spender {
                acct.touch(tx, &s.hash, s.height)?;
            }
            tx.delete::<CreditsInvalidated>(&ck).map_err(store_err)?;
        }
    }
    let n_in = u32::try_from(msg.tx_in.len()).unwrap_or(u32::MAX);
    for index in 0..n_in {
        tx.delete::<DebitsInvalidated>(&CreditKey {
            hash: *hash,
            height,
            index,
        })
        .map_err(store_err)?;
    }
    Ok(())
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
