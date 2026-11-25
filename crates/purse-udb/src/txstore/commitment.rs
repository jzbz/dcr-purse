// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Ticket commitments: `AddTicketCommitment`, `RedeemTicketCommitments`,
//! `originalTicketInfo`, `replaceTicketCommitmentUnminedSpent` and
//! `removeUnspentTicketCommitments` (`txmined.go:1560-1710`), the deletion
//! `RemoveUnconfirmed` makes of an unmined ticket's commitments
//! (`txunmined.go:211-233`), and the `Locked` balance mutator: every write
//! of a `commitments_unspent` entry and every deletion of a commitment goes
//! through [`put_unspent_commitment`] or [`del_unspent_commitment`], which
//! move the running sum of (account, `Locked`) in `balances` in the same
//! transaction (ADR-0003 Decision item 4), counting a commitment while its
//! unspent entry exists without the unmined-spent flag, as
//! `balanceFullScan`'s last pass does (`txmined.go:3517-3544`).

use std::ops::Bound;

use dcroxide_stake::TxType;
use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    BalanceClass, CommitmentRow, CommitmentUnspentRow, Commitments, CommitmentsUnspent, Hash32,
    Outpoint, WriteTx,
};

use super::credit::bump;
use super::record::{BlockMeta, TxRecord};
use super::store::{TxRead, TxStore, store_err};

/// `originalTicketInfo` (`txmined.go:1612-1626`): the ticket a vote or
/// revocation spends and the bound of the ticket's output indexes that may
/// hold a commitment (votes have a stakebase input and two outputs a
/// ticket's commitments do not pay; the bound is an upper one, as Go's,
/// since a vote may carry other trailing outputs). `tx_type` is a vote's or
/// a revocation's, the type of `msg`.
///
/// Go computes the bound in `int` and converts it to `uint32`; the
/// arithmetic here wraps the same way, and a transaction of either type
/// has the input it reads (`IsSSGen` and `IsSSRtx` check the inputs), so
/// the missing input that would make Go panic gives the zero hash here.
pub(crate) fn original_ticket_info(tx_type: TxType, msg: &MsgTx) -> (Hash32, u32) {
    let outs = u32::try_from(msg.tx_out.len()).unwrap_or(u32::MAX);
    let (input, count) = if tx_type == TxType::SSGen {
        (1, outs.wrapping_sub(2).wrapping_mul(2).wrapping_add(1))
    } else {
        (0, outs.wrapping_mul(2).wrapping_add(1))
    };
    let hash = msg
        .tx_in
        .get(input)
        .map_or_else(Hash32::default, |i| Hash32(i.previous_out_point.hash.0));
    (hash, count)
}

/// The commitments of `ticket` at the odd indexes below `count`, in index
/// order: Go's loop over `i = 1, 3, 5, … < count` with one lookup per
/// candidate, read here as one range of the ticket's commitments.
fn ticket_commitments(
    r: &mut (impl TxRead + ?Sized),
    ticket: &Hash32,
    count: u32,
) -> Result<Vec<(Outpoint, CommitmentRow)>, AnyError> {
    if count <= 1 {
        return Ok(Vec::new());
    }
    let lo = Outpoint {
        hash: *ticket,
        index: 1,
    };
    let hi = Outpoint {
        hash: *ticket,
        index: count,
    };
    let mut rows = r
        .rows::<Commitments>(
            Bound::Included(&lo),
            Bound::Excluded(&hi),
            false,
            usize::MAX,
        )
        .map_err(store_err)?;
    rows.retain(|(k, _)| k.index % 2 == 1);
    Ok(rows)
}

/// Writes the unspent entry of the commitment `op` (`putRawUnspentTicketCommitment`,
/// which overwrites) with `unmined_spent`, moving the `Locked` sum of the
/// commitment's account when the entry starts or stops counting.
pub(crate) fn put_unspent_commitment(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    c: &CommitmentRow,
    unmined_spent: bool,
) -> Result<(), AnyError> {
    let was = tx
        .get::<CommitmentsUnspent>(op)
        .map_err(store_err)?
        .is_some_and(|u| !u.unmined_spent);
    tx.put::<CommitmentsUnspent>(op, &CommitmentUnspentRow { unmined_spent })
        .map_err(store_err)?;
    if was == unmined_spent {
        // It counted and no longer does, or the reverse.
        bump(
            tx,
            c.account,
            BalanceClass::Locked,
            0,
            c.amount,
            !unmined_spent,
        )?;
    }
    Ok(())
}

/// Deletes the unspent entry of the commitment `op`
/// (`deleteRawUnspentTicketCommitment`; a missing entry is no error),
/// moving the `Locked` sum when it counted.
pub(crate) fn del_unspent_commitment(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    c: &CommitmentRow,
) -> Result<(), AnyError> {
    let Some(old) = tx.get::<CommitmentsUnspent>(op).map_err(store_err)? else {
        return Ok(());
    };
    tx.delete::<CommitmentsUnspent>(op).map_err(store_err)?;
    if !old.unmined_spent {
        bump(tx, c.account, BalanceClass::Locked, 0, c.amount, false)?;
    }
    Ok(())
}

/// `replaceTicketCommitmentUnminedSpent` (`txmined.go:1657-1686`): every
/// commitment the wallet tracks of the ticket the vote or revocation `msg`
/// spends gets its unspent entry written with `value`, re-created if it
/// had been deleted (QK-0060, reproduced).
pub(crate) fn replace_unmined_spent(
    tx: &mut WriteTx<'_>,
    tx_type: TxType,
    msg: &MsgTx,
    value: bool,
) -> Result<(), AnyError> {
    let (ticket, count) = original_ticket_info(tx_type, msg);
    for (op, c) in ticket_commitments(tx, &ticket, count)? {
        put_unspent_commitment(tx, &op, &c, value)?;
    }
    Ok(())
}

/// `removeUnspentTicketCommitments` (`txmined.go:1629-1655`): the unspent
/// entries of the commitments of the ticket the vote or revocation `msg`
/// spends are deleted; the commitments stay.
fn remove_unspent(tx: &mut WriteTx<'_>, tx_type: TxType, msg: &MsgTx) -> Result<(), AnyError> {
    let (ticket, count) = original_ticket_info(tx_type, msg);
    for (op, c) in ticket_commitments(tx, &ticket, count)? {
        del_unspent_commitment(tx, &op, &c)?;
    }
    Ok(())
}

/// `RemoveUnconfirmed`'s step for output `op` of an unmined ticket
/// (`txunmined.go:211-233`): its commitment, if the wallet tracks one, is
/// deleted with its unspent entry.
pub(crate) fn delete_commitment(tx: &mut WriteTx<'_>, op: &Outpoint) -> Result<(), AnyError> {
    let Some(c) = tx.get::<Commitments>(op).map_err(store_err)? else {
        return Ok(());
    };
    tx.delete::<Commitments>(op).map_err(store_err)?;
    del_unspent_commitment(tx, op, &c)
}

impl TxStore {
    /// `AddTicketCommitment`: records output `index` of the ticket `rec` as
    /// a commitment of `account`, with an unspent entry, unless the wallet
    /// already tracks it (in which case nothing is checked or written, as
    /// in Go: an unspent entry is added only at the first sight of the
    /// commitment). The amount is the one the commitment script carries.
    ///
    /// # Errors
    ///
    /// Go's, in Go's order: no output `index`: `Invalid` "index should be
    /// of an existing output"; an even index: `Invalid` "index should be of
    /// a ticket commitment"; `rec` not a ticket: `Invalid` "transaction
    /// record should be of a ticket"; a script too short for a commitment,
    /// which no ticket has (`IsSStx` checks its commitment outputs): the
    /// stake rule's error, unwrapped (kind `Other`). The store's.
    pub fn add_ticket_commitment(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        index: u32,
        account: u32,
    ) -> Result<(), AnyError> {
        let op = Outpoint {
            hash: *rec.hash(),
            index,
        };
        if tx.contains::<Commitments>(&op).map_err(store_err)? {
            return Ok(());
        }
        let Some(out) = usize::try_from(index)
            .ok()
            .and_then(|i| rec.msg_tx().tx_out.get(i))
        else {
            return Err(e!(Kind::Invalid, "index should be of an existing output").into());
        };
        if index % 2 != 1 {
            return Err(e!(Kind::Invalid, "index should be of a ticket commitment").into());
        }
        if rec.tx_type() != TxType::SStx {
            return Err(e!(Kind::Invalid, "transaction record should be of a ticket").into());
        }
        let amount = dcroxide_stake::amount_from_sstx_pk_scr_commitment(&out.pk_script)
            .map_err(AnyError::foreign)?;
        let c = CommitmentRow { amount, account };
        tx.put::<Commitments>(&op, &c).map_err(store_err)?;
        put_unspent_commitment(tx, &op, &c, false)
    }

    /// `RedeemTicketCommitments`: for the vote or revocation `rec`, unmined
    /// (no `block`), the commitments the wallet tracks of the ticket it
    /// spends are marked spent by an unmined transaction, re-creating an
    /// unspent entry that was deleted; mined, their unspent entries are
    /// deleted. Only whether `block` is given matters, as in Go.
    ///
    /// # Errors
    ///
    /// `rec` neither a vote nor a revocation: `Invalid` "rec must be a vote
    /// or revocation tx". The store's.
    pub fn redeem_ticket_commitments(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        block: Option<&BlockMeta>,
    ) -> Result<(), AnyError> {
        let tx_type = rec.tx_type();
        if !matches!(tx_type, TxType::SSGen | TxType::SSRtx) {
            return Err(e!(Kind::Invalid, "rec must be a vote or revocation tx").into());
        }
        if block.is_none() {
            return replace_unmined_spent(tx, tx_type, rec.msg_tx(), true);
        }
        remove_unspent(tx, tx_type, rec.msg_tx())
    }
}

#[cfg(test)]
mod tests {
    use dcroxide_chainhash::Hash;
    use dcroxide_stake::TxType;
    use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut};
    use purse_store::Hash32;

    use super::original_ticket_info;

    fn tx(inputs: &[[u8; 32]], outs: usize) -> MsgTx {
        MsgTx {
            tx_in: inputs
                .iter()
                .map(|h| TxIn {
                    previous_out_point: OutPoint {
                        hash: Hash(*h),
                        index: 0,
                        tree: 1,
                    },
                    ..TxIn::default()
                })
                .collect(),
            tx_out: vec![TxOut::default(); outs],
            ..MsgTx::default()
        }
    }

    /// Go's `originalTicketInfo`: a vote's ticket is its second input and
    /// its bound `(outputs - 2) * 2 + 1`; a revocation's is its first input
    /// and `outputs * 2 + 1`; the conversion to `uint32` wraps.
    #[test]
    fn original_ticket_info_is_gos() {
        let vote = tx(&[[0; 32], [7; 32]], 5);
        assert_eq!(
            original_ticket_info(TxType::SSGen, &vote),
            (Hash32([7; 32]), 7)
        );
        let revocation = tx(&[[9; 32]], 3);
        assert_eq!(
            original_ticket_info(TxType::SSRtx, &revocation),
            (Hash32([9; 32]), 7)
        );
        // (1 - 2) * 2 + 1 = -1 in Go's int, 0xFFFFFFFF as a uint32.
        let short = tx(&[[0; 32], [7; 32]], 1);
        assert_eq!(original_ticket_info(TxType::SSGen, &short).1, u32::MAX);
        // A missing input reads as the zero hash.
        assert_eq!(
            original_ticket_info(TxType::SSRtx, &MsgTx::default()),
            (Hash32::default(), 1)
        );
    }
}
