// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The consistency check of the transaction tables (ADR-0018 Open point
//! BK4; ADR-0003 Decision items 2, 4 and 5): the indexes and running
//! balances that `purse-udb`'s transaction store keeps in the same write
//! transaction as its records, credits and debits.
//!
//! [`ReadTx::check_tx_tables`] (and [`WriteTx::check_tx_tables`], for a
//! check inside a write transaction) walks every table below once and
//! fails with a kind-`Io` error naming the first invariant that does not
//! hold ([`Error::inconsistency`]); a row that does not decode is a
//! corrupt-row error. [`verify_backup`](crate::verify_backup) runs it after
//! the chain and record checks. The store knows tables and not wallet
//! rules, and does not decode the transactions its records hold: what
//! needs a transaction decoded (every record, mined or unmined, holding a
//! transaction; every credit, unmined credit and debit against the output
//! or input of its record's transaction that it names; an unmined input's
//! spender spending the outpoint; the type of the spender QK-0785 allows;
//! no live credit or debit of a regular transaction in a block marked
//! invalid, the converse of invariant 9) is `purse-udb`'s check
//! (`txstore::check_decoded`,
//! which its `txstore::verify_backup` passes to
//! [`crate::verify_backup_with`]). What neither checks is listed below
//! ("Not checked").
//!
//! # Invariants
//!
//! 1. **History.** Each block row's count of wallet transactions equals
//!    the number of its `history` rows, at positions 0 to count − 1; no
//!    `history` row lies at a height without a block row.
//! 2. **Records.** Every `tx_mined` row lies at a main-chain height, and
//!    the `history` row at its (height, position) names it; `history` and
//!    `tx_mined` have as many rows. With 1, the two are each other's index.
//! 3. **Unmined inputs.** Every `unmined_inputs` row names a transaction
//!    with a `tx_unmined` row (`purse-udb`: one that spends the outpoint,
//!    spec 03 I8 in the direction dcrwallet keeps).
//! 4. **Unmined credits.** Every `unmined_credits` row's transaction has
//!    its `tx_unmined` row, and the row's class is
//!    [`BalanceClass::unmined`] of its flags and of the transaction's
//!    unpublished mark as it now stands.
//! 5. **Unspent outputs.** Every `unspent` row has its live credit at the
//!    row's height, with the same amount, account, flags and script
//!    location, and its class is [`BalanceClass::mined`] of the flags. The
//!    credit names no spender, or a spender whose debit is a live debit of
//!    this credit (QK-0785; `purse-udb`: of a stake transaction, or of a
//!    record at another height, the debits stake invalidation leaves), or
//!    a spender whose debit, live or invalidated, is gone or spends the
//!    outpoint's credit at another height (QK-0095 over QK-0781, then the
//!    credit's block disapproved and approved again, below). An
//!    invalidated debit of this credit, or a debit of another outpoint, is
//!    refused.
//! 6. **Credits and debits have their record.** Every credit and debit,
//!    live or invalidated, has the `tx_mined` row of its transaction at its
//!    height.
//! 7. **Debits name credits.** Every debit, live or invalidated, names a
//!    credit, live or invalidated, and carries that credit's amount,
//!    unless the credit is recorded above the debit and gone: QK-0101's
//!    state, a spender recorded in a block below its funder, whose credit
//!    is gone once the funder's block is rolled back (both dcrwallet and
//!    dcr-purse roll that block back; the spender's own rollback is then
//!    refused).
//! 8. **Spent credits and their debit.** A credit, live or invalidated,
//!    that names a spender whose debit (live or invalidated) exists is a
//!    credit of the outpoint that debit spends. The debit may spend the
//!    outpoint's credit at another height, or be gone (QK-0095 over
//!    QK-0781, below), so the rule is no stronger.
//! 9. **Invalidated rows.** Invalidated credits and debits lie only at
//!    heights whose block row is marked invalid (stake invalidation moves
//!    them there, validation back, and a rollback deletes them with their
//!    block: ADR-0003 Decision item 5, QK-0081). The converse, that a
//!    regular transaction in a block marked invalid has no live credit or
//!    debit, needs the transaction's type and is `purse-udb`'s.
//! 10. **Commitments.** Every `commitments_unspent` row has its commitment.
//! 11. **Balances.** `balances` holds, for each (account, class), the sum
//!     and count of the unspent outputs and unmined credits that no
//!     `unmined_inputs` row names, by their stored class, and for (account,
//!     `Locked`) those of the commitments whose unspent entry lacks the
//!     unmined-spent flag; and no other row (a sum is deleted when its
//!     count reaches zero).
//! 12. **Per-height balances.** `balance_heights` holds the same sums for
//!     the windowed classes ([`BalanceClass::is_windowed`]) by (account,
//!     class, height), and no other row.
//! 13. **Account history.** `acct_history` holds exactly, for every mined
//!     record, one row per account of its live credits and of the credits
//!     (live or invalidated) its live debits spend that name the debit as
//!     their spender, at the record's height and position, naming it.
//!
//! The multisig index (`multisig_unspent` against `multisig_outs`) is
//! invariant 1 of [`crate::records`], which
//! [`ReadTx::check_records`] checks; the chain tables are
//! [`crate::chain`]'s.
//!
//! # States dcrwallet makes that the check accepts
//!
//! dcr-purse reproduces these defects of dcrwallet (ADR-0012 Rule 1), so a
//! store holding them is a store dcr-purse legitimately writes, and none
//! of the rules above refuses one. Each is a rule the check deliberately
//! does not have:
//!
//! - **Mined and unmined at once** (QK-0781, and QK-0734's state as
//!   dcrwallet leaves it): a transaction recorded in two blocks keeps the
//!   lower record when a rollback of the higher one returns it to the
//!   unmined set, so a hash may have `tx_mined` and `tx_unmined` rows, an
//!   outpoint may be both an unspent output and an unmined credit (each
//!   counted in the balances), and two credits of one outpoint at two
//!   heights share one `unspent` row. Hence no rule that a live credit
//!   naming no spender has its unspent row: the rollback of the higher
//!   block deletes the shared row while the lower credit stays, and stays
//!   so after the unmined copy is removed, which no row records; and a
//!   zero-value credit that `Rollback` unspends gets no unspent row
//!   (QK-0100).
//! - **A spent unspent output** (QK-0785): invariant 5's exception.
//! - **A debit without its credit** (QK-0101): invariant 7's exception.
//! - **Unmined inputs** (QK-0060): `RemoveUnconfirmed` deletes the row of
//!   every input of a removed transaction whichever transaction it names,
//!   and `Rollback` registers a returned transaction's inputs over another
//!   unmined spender's rows, so an unmined transaction's inputs need not
//!   have rows naming it; only the direction of invariant 3 holds. The
//!   null outpoint of an unmined coinbase or treasury spend has its row
//!   (QK-0087, QK-0780). `RemoveUnconfirmed` of a vote or revocation
//!   re-creates a commitment's unspent entry its mined redemption deleted,
//!   so no rule ties an unspent entry to the redemptions.
//! - **Unpublished marks** (QK-0098, QK-0058): `SetPublished` marks any
//!   hash, and `Rollback` does not restore a mark, so `unpublished` has no
//!   rule; an unmined credit's class follows the mark as it stands.
//! - **Tickets** (QK-0056): never deleted, so `tickets` has no rule.
//! - **A disapproved block's transactions** (QK-0736): a regular
//!   transaction keeps its mined record with its credits invalidated, so
//!   no rule says a record's credits are live.
//! - **Multisig records** (QK-0786, QK-0787, QK-0789, QK-0097, QK-0054): a
//!   record may be unspent while a transaction spends it, outlive its
//!   transaction, be unmined while its transaction is mined, be reset
//!   unmined by `AddMultisigOut`, and lose its stake-tree flag, so no rule
//!   ties a record to transactions, debits or the output's tree; only the
//!   index rule of [`crate::records`].
//! - **A debit written for a recorded transaction** (QK-0095):
//!   `InsertMinedTx` debits the inputs before its test that the record
//!   exists, so a transaction recorded before its input's credit spends
//!   that credit when it is inserted again. On its own the rows agree. With
//!   QK-0781's second credit of the spent outpoint at another height, the
//!   insert spends that second credit (the outpoint's unspent row names
//!   it) and overwrites the debit, whose key has one slot per (transaction,
//!   height, input), to name it, while the first credit keeps naming the
//!   spender: a credit whose spender's debit spends the outpoint's credit
//!   at another height. A rollback of the block holding that debit deletes
//!   it without unspending the first credit, which is left naming a
//!   spender with no debit. dcrwallet leaves the same rows (its
//!   `InsertMinedTx`, `spendCredit` on the credit `existsUnspent` names and
//!   `putDebit` under `keyDebit`), so invariant 8 refuses neither; it still
//!   refuses a debit spending another outpoint. Either credit, once its
//!   block is disapproved and approved again, gets its unspent row back
//!   with its spender kept (QK-0785's mechanism: dcrwallet's
//!   `stakeValidate` puts the credit back unchanged and its unspent row
//!   unconditionally, and re-spends only what the block's own debits
//!   spend), so invariant 5 accepts the same two spenders under an unspent
//!   row: one with no debit, and one whose debit spends the outpoint's
//!   credit at another height.
//! - **The expiry flag lost when mined** (QK-0079, QK-0066): a mined credit
//!   and its unspent output carry the flags as written, the class following
//!   them (invariant 5 compares the class with the flags the row holds).
//!
//! `purse-udb`'s test
//! `every_state_of_go_s_reproduced_defects_passes_the_check`
//! (`tests/txstore.rs`) writes each of these states by the operations alone
//! and holds this check, `check_decoded` and the verify path to accept it.
//!
//! Two of dcrwallet's states cannot occur in this store and invariants 6
//! and 9 rely on that: dcr-purse deletes a detached block's invalidated
//! credits and debits (ADR-0003 Decision item 5, a decided divergence;
//! QK-0081, QK-0782), and refuses a credit without its record (QK-0783).
//! Two more are not representable in its rows, so no rule concerns them:
//! the stale spender bytes of an unspent credit and of an unspent multisig
//! record (QK-0053, QK-0055); the rows hold a spender only while spent.
//!
//! The auxiliary tables (VSP records, treasury and agenda policies) are not
//! transaction tables and are not part of this check; their invariants are
//! [`crate::records`]', and a consistency check of their own, if they need
//! one, is a later addition.
//!
//! # Not checked
//!
//! Neither this check nor `purse-udb`'s decoded part holds:
//!
//! - a record's key against its transaction's hash (no transaction is
//!   hashed), or a record's received time and position against anything
//!   beyond the history rows;
//! - a credit's flags against its transaction (QK-0079 makes the expiry
//!   flag differ legitimately; the others are not compared either);
//! - that a credit naming a spender has the spender's debit: a deleted
//!   debit row leaves the rows of QK-0095 over QK-0781 after a rollback
//!   (above; also under an unspent row once the credit's block is
//!   validated again), and is refused only where another rule sees it (an
//!   account history row of the debit's record that no longer has a
//!   reason, 13);
//! - a commitment row against its ticket's commitment output, or a
//!   commitment's unspent entry against the redemptions (QK-0060);
//! - the rules the reproduced states above rule out, and the tables they
//!   leave without one (`tickets`, `unpublished`, multisig records beyond
//!   their index).
//!
//! # Cost
//!
//! Every table above is walked once in key order, `tx_mined` once for
//! invariants 2 and 13 together, with point lookups (a B-tree descent each)
//! into the others: O(n log n) in the rows of the transaction tables. No
//! table is collected: the balances are recomputed into one entry per
//! (account, class) and per (account, windowed class, height) holding an
//! output, and the account history is checked record by record against its
//! table, with a final count.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::error::{Error, Result};
use crate::keys::{AcctHistoryKey, CreditKey, Hash32, Height, HistoryKey, Outpoint, TxRecKey};
use crate::schema::{
    AcctHistory, BalanceHeights, Balances, Blocks, Commitments, CommitmentsUnspent, Credits,
    CreditsInvalidated, Debits, DebitsInvalidated, History, Table, TableId, TxMined, TxUnmined,
    UnminedCredits, UnminedInputs, Unpublished, Unspent,
};
use crate::txn::{ReadTx, Rows, WriteTx};
use crate::values::{BalanceClass, BalanceRow, CreditRow, DebitRow, Spender};

impl ReadTx {
    /// Checks every invariant of the transaction tables (module
    /// documentation of [`crate::txtables`]) and fails with a kind-`Io`
    /// error naming the first that does not hold
    /// ([`Error::inconsistency`]); a row that does not decode is a
    /// corrupt-row error. Costs a walk of the transaction tables; for
    /// [`crate::verify_backup`], tests, and any check of a store whose
    /// origin is not trusted.
    pub fn check_tx_tables(&self) -> Result<()> {
        check(self)
    }
}

impl WriteTx<'_> {
    /// [`ReadTx::check_tx_tables`] over this transaction's rows, its own
    /// writes included (opening, and so creating, every table first).
    pub fn check_tx_tables(&mut self) -> Result<()> {
        check(&self.shared()?)
    }
}

fn bad<T>(why: String) -> Result<T> {
    Err(Error::inconsistent(why))
}

/// A hash as dcrd prints it: the bytes reversed, in hex.
fn show(h: &Hash32) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in h.0.iter().rev() {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn show_op(op: &Outpoint) -> String {
    format!("{}:{}", show(&op.hash), op.index)
}

fn show_key(k: &CreditKey) -> String {
    format!("{}:{} at height {}", show(&k.hash), k.index, k.height)
}

fn spender_key(s: &Spender) -> CreditKey {
    CreditKey {
        hash: s.hash,
        height: s.height,
        index: s.input,
    }
}

fn credit_range(hash: Hash32, height: u32) -> (CreditKey, CreditKey) {
    let lo = CreditKey {
        hash,
        height,
        index: 0,
    };
    (
        lo,
        CreditKey {
            index: u32::MAX,
            ..lo
        },
    )
}

fn check<R: Rows>(r: &R) -> Result<()> {
    history(r)?;
    records(r)?;
    let mut sums = Sums::default();
    unmined(r, &mut sums)?;
    unspent(r, &mut sums)?;
    credits::<Credits, R>(r, false)?;
    credits::<CreditsInvalidated, R>(r, true)?;
    debits::<Debits, R>(r, false)?;
    debits::<DebitsInvalidated, R>(r, true)?;
    commitments(r, &mut sums)?;
    sums.compare(r)
}

/// Invariant 1, by one merge of `blocks` and `history`, both in height
/// order.
fn history<R: Rows>(r: &R) -> Result<()> {
    let mut hist = r.all::<History>()?;
    let mut next = hist.next().transpose()?;
    for b in r.all::<Blocks>()? {
        let (h, row) = b?;
        let mut n: u32 = 0;
        while let Some((k, _)) = next {
            if k.height > h.0 {
                break;
            }
            if k.height < h.0 {
                return bad(format!(
                    "a history row at height {} with no block",
                    k.height
                ));
            }
            if k.position != n {
                return bad(format!(
                    "the history of block {} has position {} where {n} is next",
                    h.0, k.position
                ));
            }
            n = n.saturating_add(1);
            next = hist.next().transpose()?;
        }
        if n != row.wallet_txs {
            return bad(format!(
                "block {} counts {} wallet transactions and has {n} history rows",
                h.0, row.wallet_txs
            ));
        }
    }
    if let Some((k, _)) = next {
        return bad(format!(
            "a history row at height {} above the main chain",
            k.height
        ));
    }
    Ok(())
}

/// Invariants 2 and 13, by one walk of `tx_mined`.
fn records<R: Rows>(r: &R) -> Result<()> {
    let mut acct_rows: u64 = 0;
    let mut accounts: Vec<u32> = Vec::new();
    for e in r.all::<TxMined>()? {
        let (k, m) = e?;
        if !r.has::<Blocks>(&Height(k.height))? {
            return bad(format!(
                "record {} at height {}, above the main chain",
                show(&k.hash),
                k.height
            ));
        }
        let at = HistoryKey {
            height: k.height,
            position: m.position,
        };
        if r.row::<History>(&at)? != Some(k.hash) {
            return bad(format!(
                "record {} at height {} position {} is not the history row there",
                show(&k.hash),
                k.height,
                m.position
            ));
        }
        accounts.clear();
        let (lo, hi) = credit_range(k.hash, k.height);
        for c in r.rows::<Credits>(Bound::Included(&lo), Bound::Included(&hi))? {
            accounts.push(c?.1.account);
        }
        for d in r.rows::<Debits>(Bound::Included(&lo), Bound::Included(&hi))? {
            let (dk, d) = d?;
            let me = Spender {
                hash: dk.hash,
                height: dk.height,
                input: dk.index,
            };
            if let Some(c) = credit_of(r, &d.credit)?
                && c.spender == Some(me)
            {
                accounts.push(c.account);
            }
        }
        accounts.sort_unstable();
        accounts.dedup();
        for &account in &accounts {
            let key = AcctHistoryKey {
                account,
                height: k.height,
                position: m.position,
            };
            if r.row::<AcctHistory>(&key)? != Some(k.hash) {
                return bad(format!(
                    "record {} at height {} touches account {account} and has no account \
                     history row naming it",
                    show(&k.hash),
                    k.height
                ));
            }
            acct_rows = acct_rows.saturating_add(1);
        }
    }
    if r.count(TableId::History)? != r.count(TableId::TxMined)? {
        return bad(format!(
            "history holds {} rows for {} records",
            r.count(TableId::History)?,
            r.count(TableId::TxMined)?
        ));
    }
    let have = r.count(TableId::AcctHistory)?;
    if have != acct_rows {
        return bad(format!(
            "acct_history holds {have} rows where the records' credits and debits give {acct_rows}"
        ));
    }
    Ok(())
}

/// The credit at `key`, live or invalidated.
fn credit_of<R: Rows>(r: &R, key: &CreditKey) -> Result<Option<CreditRow>> {
    match r.row::<Credits>(key)? {
        Some(c) => Ok(Some(c)),
        None => r.row::<CreditsInvalidated>(key),
    }
}

/// The debit at `key`, live or invalidated.
fn debit_of<R: Rows>(r: &R, key: &CreditKey) -> Result<Option<DebitRow>> {
    match r.row::<Debits>(key)? {
        Some(d) => Ok(Some(d)),
        None => r.row::<DebitsInvalidated>(key),
    }
}

/// Invariants 3 and 4, and the unmined credits' part of 11.
fn unmined<R: Rows>(r: &R, sums: &mut Sums) -> Result<()> {
    for e in r.all::<UnminedInputs>()? {
        let (op, spender) = e?;
        if !r.has::<TxUnmined>(&spender)? {
            return bad(format!(
                "unmined input {} names {}, which has no unmined record",
                show_op(&op),
                show(&spender)
            ));
        }
    }
    for e in r.all::<UnminedCredits>()? {
        let (op, m) = e?;
        if !r.has::<TxUnmined>(&op.hash)? {
            return bad(format!(
                "unmined credit {} without its unmined record",
                show_op(&op)
            ));
        }
        let want = BalanceClass::unmined(&m.flags, r.has::<Unpublished>(&op.hash)?);
        if m.class != want {
            return bad(format!(
                "unmined credit {} has class {:?} where its flags and mark give {want:?}",
                show_op(&op),
                m.class
            ));
        }
        if !r.has::<UnminedInputs>(&op)? {
            sums.add(m.account, m.class, 0, m.amount);
        }
    }
    Ok(())
}

/// Invariant 5, and the unspent outputs' part of 11 and 12.
fn unspent<R: Rows>(r: &R, sums: &mut Sums) -> Result<()> {
    for e in r.all::<Unspent>()? {
        let (op, u) = e?;
        let ck = CreditKey {
            hash: op.hash,
            height: u.height,
            index: op.index,
        };
        let Some(c) = r.row::<Credits>(&ck)? else {
            return bad(format!(
                "unspent output {} has no live credit at height {}",
                show_op(&op),
                u.height
            ));
        };
        if (u.amount, u.account, u.flags, u.script) != (c.amount, c.account, c.flags, c.script) {
            return bad(format!(
                "unspent output {} differs from its credit",
                show_op(&op)
            ));
        }
        let want = BalanceClass::mined(&c.flags);
        if u.class != want {
            return bad(format!(
                "unspent output {} has class {:?} where its flags give {want:?}",
                show_op(&op),
                u.class
            ));
        }
        if let Some(s) = c.spender {
            spent_unspent(r, &op, &ck, &s)?;
        }
        if !r.has::<UnminedInputs>(&op)? {
            sums.add(u.account, u.class, u.height, u.amount);
        }
    }
    Ok(())
}

/// Invariant 5's rule for an unspent output `op` whose credit `ck` names
/// the spender `s`: the spender's debit is a live debit of `ck` (QK-0785),
/// or is gone, or spends the outpoint's credit at another height (QK-0095
/// over QK-0781, then the credit's block validated again: module
/// documentation). An invalidated debit of `ck` itself, or a debit of
/// another outpoint, is refused.
fn spent_unspent<R: Rows>(r: &R, op: &Outpoint, ck: &CreditKey, s: &Spender) -> Result<()> {
    let dk = spender_key(s);
    let (d, live) = match r.row::<Debits>(&dk)? {
        Some(d) => (d, true),
        None => match r.row::<DebitsInvalidated>(&dk)? {
            Some(d) => (d, false),
            None => return Ok(()),
        },
    };
    let same_outpoint = (d.credit.hash, d.credit.index) == (ck.hash, ck.index);
    if same_outpoint && (live || d.credit.height != ck.height) {
        return Ok(());
    }
    bad(format!(
        "unspent output {} has a credit spent by {}, {} of {}",
        show_op(op),
        show_key(&dk),
        if live {
            "a live debit"
        } else {
            "an invalidated debit"
        },
        show_key(&d.credit)
    ))
}

/// Whether the block row at `height` is marked invalid.
fn invalid_at<R: Rows>(r: &R, height: u32) -> Result<bool> {
    Ok(r.row::<Blocks>(&Height(height))?.is_some_and(|b| b.invalid))
}

/// Invariants 6, 8 and 9 over one credit table.
fn credits<T, R>(r: &R, invalidated: bool) -> Result<()>
where
    T: Table<Key = CreditKey, Value = CreditRow>,
    R: Rows,
{
    let name = T::ID.name();
    for e in r.all::<T>()? {
        let (ck, c) = e?;
        let rec = TxRecKey {
            hash: ck.hash,
            height: ck.height,
        };
        if !r.has::<TxMined>(&rec)? {
            return bad(format!(
                "{name} row {} has no record at its height",
                show_key(&ck)
            ));
        }
        if invalidated && !invalid_at(r, ck.height)? {
            return bad(format!(
                "{name} row {} lies in a block not marked invalid",
                show_key(&ck)
            ));
        }
        // A spender without its debit, or whose debit spends another credit
        // of the same outpoint, is QK-0095's state over QK-0781's (module
        // documentation): only a debit spending another outpoint is refused.
        if let Some(s) = c.spender {
            let dk = spender_key(&s);
            if let Some(d) = debit_of(r, &dk)?
                && (d.credit.hash, d.credit.index) != (ck.hash, ck.index)
            {
                return bad(format!(
                    "{name} row {} names spender {}, whose debit spends {}",
                    show_key(&ck),
                    show_key(&dk),
                    show_key(&d.credit)
                ));
            }
        }
    }
    Ok(())
}

/// Invariants 6, 7 and 9 over one debit table.
fn debits<T, R>(r: &R, invalidated: bool) -> Result<()>
where
    T: Table<Key = CreditKey, Value = DebitRow>,
    R: Rows,
{
    let name = T::ID.name();
    for e in r.all::<T>()? {
        let (dk, d) = e?;
        let rec = TxRecKey {
            hash: dk.hash,
            height: dk.height,
        };
        if !r.has::<TxMined>(&rec)? {
            return bad(format!(
                "{name} row {} has no record at its height",
                show_key(&dk)
            ));
        }
        if invalidated && !invalid_at(r, dk.height)? {
            return bad(format!(
                "{name} row {} lies in a block not marked invalid",
                show_key(&dk)
            ));
        }
        match credit_of(r, &d.credit)? {
            Some(c) if c.amount != d.amount => {
                return bad(format!(
                    "{name} row {} spends {} of credit {}, which holds {}",
                    show_key(&dk),
                    d.amount,
                    show_key(&d.credit),
                    c.amount
                ));
            }
            Some(_) => {}
            None if d.credit.height <= dk.height => {
                return bad(format!(
                    "{name} row {} names credit {}, which is not recorded",
                    show_key(&dk),
                    show_key(&d.credit)
                ));
            }
            // QK-0101: a credit recorded above the debit, gone.
            None => {}
        }
    }
    Ok(())
}

/// Invariant 10, and the `Locked` part of 11.
fn commitments<R: Rows>(r: &R, sums: &mut Sums) -> Result<()> {
    for e in r.all::<CommitmentsUnspent>()? {
        let (op, u) = e?;
        let Some(c) = r.row::<Commitments>(&op)? else {
            return bad(format!(
                "unspent commitment entry {} without its commitment",
                show_op(&op)
            ));
        };
        if !u.unmined_spent {
            sums.add(c.account, BalanceClass::Locked, 0, c.amount);
        }
    }
    Ok(())
}

/// The balances recomputed from the rows (invariants 11 and 12).
#[derive(Default)]
struct Sums {
    by_class: BTreeMap<(u32, BalanceClass), BalanceRow>,
    by_height: BTreeMap<(u32, BalanceClass, u32), BalanceRow>,
}

fn bump(row: &mut BalanceRow, amount: i64) {
    // The running sums wrap as `purse-udb`'s mutators make them.
    row.sum = row.sum.wrapping_add(amount);
    row.count = row.count.wrapping_add(1);
}

impl Sums {
    fn add(&mut self, account: u32, class: BalanceClass, height: u32, amount: i64) {
        bump(self.by_class.entry((account, class)).or_default(), amount);
        if class.is_windowed() {
            bump(
                self.by_height.entry((account, class, height)).or_default(),
                amount,
            );
        }
    }

    fn compare<R: Rows>(mut self, r: &R) -> Result<()> {
        for e in r.all::<Balances>()? {
            let (k, have) = e?;
            match self.by_class.remove(&(k.account, k.class)) {
                Some(want) if want == have => {}
                want => {
                    let want = want.unwrap_or_default();
                    return bad(format!(
                        "the balance of account {} class {:?} is {} in {} outputs where the \
                         rows give {} in {}",
                        k.account, k.class, have.sum, have.count, want.sum, want.count
                    ));
                }
            }
        }
        if let Some(((account, class), want)) = self.by_class.pop_first() {
            return bad(format!(
                "account {account} class {class:?} has no balance row for {} in {} outputs",
                want.sum, want.count
            ));
        }
        for e in r.all::<BalanceHeights>()? {
            let (k, have) = e?;
            match self.by_height.remove(&(k.account, k.class, k.height)) {
                Some(want) if want == have => {}
                want => {
                    let want = want.unwrap_or_default();
                    return bad(format!(
                        "the balance of account {} class {:?} at height {} is {} in {} outputs \
                         where the rows give {} in {}",
                        k.account, k.class, k.height, have.sum, have.count, want.sum, want.count
                    ));
                }
            }
        }
        if let Some(((account, class, height), want)) = self.by_height.pop_first() {
            return bad(format!(
                "account {account} class {class:?} has no balance row at height {height} for {} \
                 in {} outputs",
                want.sum, want.count
            ));
        }
        Ok(())
    }
}
