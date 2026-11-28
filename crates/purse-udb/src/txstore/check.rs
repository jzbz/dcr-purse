// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The part of the transaction tables' consistency check that needs a
//! transaction decoded (ADR-0018 Open point BK4), and the verification of
//! a backup with it.
//!
//! `purse-store` checks every invariant of the transaction tables it can
//! read from its typed rows (`ReadTx::check_tx_tables`, module
//! `purse_store::txtables`, which lists them, the states of dcrwallet's
//! reproduced defects it accepts and what neither part checks). It does
//! not decode the transactions its records hold; these invariants need
//! them decoded:
//!
//! 1. **Credits are outputs.** Every credit, live or invalidated, names an
//!    output of its record's transaction (the `tx_mined` row at its hash
//!    and height): its index is below the number of outputs, its amount is
//!    the output's value, and its script location (offset, length and
//!    script type code) is the one `AddCredit` takes from the output
//!    (`MsgTx.PkScriptLocs` and `pkScriptType`). An unspent output carries
//!    its credit's (the store's invariant 5), so it is held too.
//! 2. **Unmined credits are outputs.** The same for every unmined credit
//!    against its transaction's `tx_unmined` row.
//! 3. **Debits are inputs.** Every debit, live or invalidated, names an
//!    input of its record's transaction, and that input spends the
//!    outpoint (hash and index) of the credit the debit names, whichever
//!    height that credit is at (QK-0781's two credits of one outpoint).
//! 4. **Unmined inputs.** Every `unmined_inputs` row's spender has an input
//!    spending that outpoint (hash and index; the index has no tree, as
//!    dcrwallet's `mi` has none): spec 03 I8 in the direction dcrwallet
//!    keeps (the other direction it breaks, QK-0060; crate `txstore`,
//!    "Rows").
//! 5. **QK-0785's spender.** An unspent output's credit whose spender's
//!    debit is a live debit of that credit (the store's invariant 5) names
//!    one that stake invalidation of the credit's block leaves live: a
//!    debit of a transaction that is not regular, or recorded at another
//!    height than the credit (QK-0785, reproduced). A spender whose debit
//!    is gone or spends the outpoint's credit at another height (QK-0095
//!    over QK-0781, then the credit's block validated again; the store's
//!    module documentation) is not this rule's: no record or type is
//!    asked of it.
//! 6. **Records hold transactions.** Every `tx_mined` and `tx_unmined`
//!    row's bytes decode as a transaction, whether or not a credit, debit
//!    or input row names it (a record without any is legitimate:
//!    `InsertMinedTx` or `InsertMemPoolTx` without `AddCredit`).
//! 7. **A block marked invalid holds no live row of a regular
//!    transaction** (the converse of the store's invariant 9). A record at
//!    a height whose block row is marked invalid, whose transaction is
//!    regular (`stake.DetermineTxType`, as stake invalidation decides), has
//!    no live credit and no live debit: dcrwallet's `stakeInvalidate`
//!    moves them all in the step that sets the mark, `stakeValidate` moves
//!    them back in the step that clears it, and `InsertMinedTx` and
//!    `AddCredit` write only invalidated rows for a regular transaction of
//!    a block so marked (an unmined transaction mined into one is refused,
//!    where dcrwallet panics). A stake transaction's rows stay live.
//!
//! No reproduced defect of dcrwallet breaks 1 to 3, 6 or 7: every credit
//! is built from its output by `AddCredit` (or moved, row unchanged, by
//! `InsertMinedTx` and stake invalidation), every debit is written for
//! input `i` under key (transaction, height, `i`) naming the credit the
//! input's outpoint has unspent, and every record from a transaction the
//! caller handed in. A credit's flags are not compared with the
//! transaction (QK-0079 loses the expiry flag), nor is a record's key
//! with its transaction's hash.
//!
//! [`check_decoded`] checks all seven; [`verify_backup`] is
//! `purse_store::verify_backup` with it, and [`verify_backup_on_backend`]
//! the same on a storage backend. The property tests' row checker
//! (`tests/txstore.rs`, `check_rows`) runs both parts after its own
//! recomputation, after every step of every random sequence and targeted
//! test; the operation-script runner runs them after every operation of
//! every script, and after every member of a group inside the group's
//! transaction; and `every_state_of_go_s_reproduced_defects_passes_the_check`
//! writes each state Go's reproduced defects leave in these tables and
//! holds both parts to accept it.

use std::ops::Bound;
use std::path::Path;

use dcroxide_stake::TxType;
use dcroxide_wire::MsgTx;
use purse_store::{
    Blocks, CreditKey, Credits, CreditsInvalidated, Debits, DebitsInvalidated, Error, Hash32,
    Height, Outpoint, ReadTx, ScriptLoc, StorageBackend, Table, TableId, TxMined, TxRecKey,
    TxUnmined, UnminedCredits, UnminedInputs, Unspent, Verified,
};

use super::credit::script_loc_at;
use super::record::display;
use super::store::TxRead;

/// The rows [`each`] reads per range lookup. `tests/txstore.rs`
/// (`check_decoded_reads_past_the_first_page`) puts more than two pages
/// in each table walked here, a bad row last; keep its count above twice
/// this.
const PAGE: usize = 512;

/// Calls `f` with each row of table `T` in key order, a page at a time.
fn each<T, R, F>(r: &mut R, mut f: F) -> purse_store::Result<()>
where
    T: Table,
    T::Key: Clone,
    R: TxRead + ?Sized,
    F: FnMut(&mut R, T::Key, T::Value) -> purse_store::Result<()>,
{
    let mut lo: Bound<T::Key> = Bound::Unbounded;
    loop {
        let rows = r.rows::<T>(lo.as_ref(), Bound::Unbounded, false, PAGE)?;
        let full = rows.len() == PAGE;
        let mut last = None;
        for (k, v) in rows {
            last = Some(k.clone());
            f(r, k, v)?;
        }
        match last {
            Some(k) if full => lo = Bound::Excluded(k),
            _ => return Ok(()),
        }
    }
}

fn show_op(op: &Outpoint) -> String {
    format!("{}:{}", display(&op.hash), op.index)
}

fn show_key(k: &CreditKey) -> String {
    format!("{}:{} at height {}", display(&k.hash), k.index, k.height)
}

fn bad<T>(why: String) -> purse_store::Result<T> {
    Err(Error::inconsistent(why))
}

fn decode(what: &str, hash: &Hash32, bytes: &[u8]) -> purse_store::Result<MsgTx> {
    MsgTx::from_bytes(bytes)
        .map(|(m, _)| m)
        .map_err(|e| Error::inconsistent(format!("{what} {} does not decode: {e}", display(hash))))
}

/// Checks the invariants of the transaction tables that need a transaction
/// decoded (module documentation) and fails with a kind-`Io` error naming
/// the first that does not hold ([`purse_store::Error::inconsistency`]).
/// `purse-store`'s `check_tx_tables` checks the rest and is meant to run
/// first: a row it refuses (an unmined input naming no unmined record, an
/// unmined credit without its record, a live debit of an unspent output's
/// credit without its record) is refused here too where this check reads
/// it, with the same meaning.
///
/// Costs a walk of `unmined_inputs`, `unspent`, `tx_mined` and
/// `tx_unmined`; one decoding of each record, mined or unmined; four range
/// lookups per mined record (its credits and debits, live and
/// invalidated), and a block lookup for one with a live row; one range
/// lookup per unmined record (its unmined credits); a decoding of the
/// unmined record each `unmined_inputs` row names (the last one decoded is
/// kept for the next row naming the same transaction); and a decoding of
/// the mined record of each spender an unspent output's credit names
/// whose debit is a live debit of it (none outside QK-0785's state). The
/// rows are read a page at a time; nothing is collected beyond one
/// record's rows.
///
/// # Errors
///
/// The first invariant that does not hold; the store's.
pub fn check_decoded<R: TxRead + ?Sized>(r: &mut R) -> purse_store::Result<()> {
    unmined_inputs(r)?;
    unspent_spenders(r)?;
    mined_records(r)?;
    unmined_records(r)
}

/// Invariant 1 for one credit row, of output `index` of `m`, whose script
/// offsets are `locs`; `row` names the row (its table and key) for the
/// message, and is called only on a refusal.
fn output(
    row: &dyn Fn() -> String,
    m: &MsgTx,
    locs: &[usize],
    index: u32,
    amount: i64,
    script: &ScriptLoc,
) -> purse_store::Result<()> {
    let i = usize::try_from(index).unwrap_or(usize::MAX);
    let (Some(out), Some(&offset)) = (m.tx_out.get(i), locs.get(i)) else {
        return bad(format!(
            "{} names output {index} of a transaction with {} outputs",
            row(),
            m.tx_out.len()
        ));
    };
    if amount != out.value {
        return bad(format!(
            "{} holds {amount} where its output holds {}",
            row(),
            out.value
        ));
    }
    let want = script_loc_at(index, out, offset)
        .map_err(|e| Error::inconsistent(format!("{}: {e}", row())))?;
    if *script != want {
        return bad(format!(
            "{} has script location {script:?} where its output's is {want:?}",
            row()
        ));
    }
    Ok(())
}

/// Invariant 3 for one debit row of table `name`: input `k.index` of `m`
/// spends the outpoint of `credit`.
fn input(name: &str, m: &MsgTx, k: &CreditKey, credit: &CreditKey) -> purse_store::Result<()> {
    let i = usize::try_from(k.index).unwrap_or(usize::MAX);
    let Some(input) = m.tx_in.get(i) else {
        return bad(format!(
            "{name} row {} names input {} of a transaction with {} inputs",
            show_key(k),
            k.index,
            m.tx_in.len()
        ));
    };
    let prev = &input.previous_out_point;
    if prev.hash.0 != credit.hash.0 || prev.index != credit.index {
        return bad(format!(
            "{name} row {} names credit {} where its input spends {}:{}",
            show_key(k),
            show_key(credit),
            prev.hash,
            prev.index
        ));
    }
    Ok(())
}

/// The rows of table `T` keyed by the record (`hash`, `height`).
fn of_record<T, R>(
    r: &mut R,
    hash: Hash32,
    height: u32,
) -> purse_store::Result<Vec<(CreditKey, T::Value)>>
where
    T: Table<Key = CreditKey>,
    R: TxRead + ?Sized,
{
    let lo = CreditKey {
        hash,
        height,
        index: 0,
    };
    let hi = CreditKey {
        index: u32::MAX,
        ..lo
    };
    r.rows::<T>(
        Bound::Included(&lo),
        Bound::Included(&hi),
        false,
        usize::MAX,
    )
}

/// Invariants 1, 3, 6 and 7, by one walk of `tx_mined`: each record
/// decoded once, and its credits and debits, live and invalidated,
/// against its transaction.
fn mined_records<R: TxRead + ?Sized>(r: &mut R) -> purse_store::Result<()> {
    each::<TxMined, R, _>(r, |r, k, row| {
        let m = decode("record", &k.hash, &row.tx)?;
        let credits = of_record::<Credits, R>(r, k.hash, k.height)?;
        let icredits = of_record::<CreditsInvalidated, R>(r, k.hash, k.height)?;
        let debits = of_record::<Debits, R>(r, k.hash, k.height)?;
        let idebits = of_record::<DebitsInvalidated, R>(r, k.hash, k.height)?;
        if (!credits.is_empty() || !debits.is_empty())
            && r.get::<Blocks>(&Height(k.height))?
                .is_some_and(|b| b.invalid)
            && dcroxide_stake::determine_tx_type(&m) == TxType::Regular
        {
            return bad(format!(
                "a block marked invalid at height {} holds record {}, a regular transaction, \
                 with {} live credits and {} live debits",
                k.height,
                display(&k.hash),
                credits.len(),
                debits.len()
            ));
        }
        let locs = if credits.is_empty() && icredits.is_empty() {
            Vec::new()
        } else {
            m.pk_script_locs()
        };
        for (name, rows) in [
            (Credits::ID.name(), &credits),
            (CreditsInvalidated::ID.name(), &icredits),
        ] {
            for (ck, c) in rows {
                let row = || format!("{name} row {}", show_key(ck));
                output(&row, &m, &locs, ck.index, c.amount, &c.script)?;
            }
        }
        for (name, rows) in [
            (Debits::ID.name(), &debits),
            (DebitsInvalidated::ID.name(), &idebits),
        ] {
            for (dk, d) in rows {
                input(name, &m, dk, &d.credit)?;
            }
        }
        Ok(())
    })
}

/// Invariants 2 and 6 for unmined records, by one walk of `tx_unmined`:
/// each record decoded once, and its unmined credits (adjacent in
/// `unmined_credits`, keyed by outpoint) against its transaction. An
/// unmined credit without its record (the store's invariant 4) is found
/// by the count, and named by a second walk only then.
fn unmined_records<R: TxRead + ?Sized>(r: &mut R) -> purse_store::Result<()> {
    let mut seen: u64 = 0;
    each::<TxUnmined, R, _>(r, |r, hash, row| {
        let m = decode("unmined record", &hash, &row.tx)?;
        let lo = Outpoint { hash, index: 0 };
        let hi = Outpoint {
            hash,
            index: u32::MAX,
        };
        let credits = r.rows::<UnminedCredits>(
            Bound::Included(&lo),
            Bound::Included(&hi),
            false,
            usize::MAX,
        )?;
        if credits.is_empty() {
            return Ok(());
        }
        let locs = m.pk_script_locs();
        for (op, c) in &credits {
            let row = || format!("unmined credit {}", show_op(op));
            output(&row, &m, &locs, op.index, c.amount, &c.script)?;
        }
        seen = seen.saturating_add(u64::try_from(credits.len()).unwrap_or(u64::MAX));
        Ok(())
    })?;
    let have = r.table_len(TableId::UnminedCredits)?;
    if have == seen {
        return Ok(());
    }
    each::<UnminedCredits, R, _>(r, |r, op, _| {
        if r.contains::<TxUnmined>(&op.hash)? {
            Ok(())
        } else {
            bad(format!(
                "unmined credit {} without its unmined record",
                show_op(&op)
            ))
        }
    })?;
    bad(format!(
        "unmined_credits holds {have} rows where the unmined records have {seen}"
    ))
}

/// Invariant 4, by one walk of `unmined_inputs`.
fn unmined_inputs<R: TxRead + ?Sized>(r: &mut R) -> purse_store::Result<()> {
    let mut last: Option<(Hash32, MsgTx)> = None;
    each::<UnminedInputs, R, _>(r, |r, op, spender| {
        if last.as_ref().is_none_or(|(h, _)| *h != spender) {
            let Some(row) = r.get::<TxUnmined>(&spender)? else {
                return bad(format!(
                    "unmined input {} names {}, which has no unmined record",
                    show_op(&op),
                    display(&spender)
                ));
            };
            last = Some((spender, decode("unmined record", &spender, &row.tx)?));
        }
        let spends = last.as_ref().is_some_and(|(_, m)| {
            m.tx_in.iter().any(|i| {
                i.previous_out_point.hash.0 == op.hash.0 && i.previous_out_point.index == op.index
            })
        });
        if spends {
            Ok(())
        } else {
            bad(format!(
                "unmined input {} names {}, which does not spend it",
                show_op(&op),
                display(&spender)
            ))
        }
    })
}

/// Invariant 5, by one walk of `unspent`.
fn unspent_spenders<R: TxRead + ?Sized>(r: &mut R) -> purse_store::Result<()> {
    each::<Unspent, R, _>(r, |r, op, u| {
        let ck = CreditKey {
            hash: op.hash,
            height: u.height,
            index: op.index,
        };
        let Some(s) = r.get::<Credits>(&ck)?.and_then(|c| c.spender) else {
            return Ok(());
        };
        // Only a live debit of this credit is QK-0785's spender; a spender
        // whose debit is gone or spends the outpoint's credit at another
        // height is QK-0095's over QK-0781's, and anything else the
        // store's invariant 5 refuses.
        let dk = CreditKey {
            hash: s.hash,
            height: s.height,
            index: s.input,
        };
        if r.get::<Debits>(&dk)?.is_none_or(|d| d.credit != ck) {
            return Ok(());
        }
        let rec = TxRecKey {
            hash: s.hash,
            height: s.height,
        };
        let Some(row) = r.get::<TxMined>(&rec)? else {
            return bad(format!(
                "unspent output {} has a credit spent by {}, which has no record at height {}",
                show_op(&op),
                display(&s.hash),
                s.height
            ));
        };
        let m = decode("record", &s.hash, &row.tx)?;
        if s.height != ck.height || dcroxide_stake::determine_tx_type(&m) != TxType::Regular {
            Ok(())
        } else {
            bad(format!(
                "unspent output {} has a credit spent by the regular transaction {} of its own \
                 block, a debit stake invalidation moves (QK-0785 allows only the others)",
                show_op(&op),
                display(&s.hash)
            ))
        }
    })
}

/// Verifies a backup handed in from outside with every check of the
/// transaction tables: `purse_store::verify_backup_with` (its file
/// handling, integrity, record, digest and consistency checks, the store's
/// part of the transaction tables' included) with [`check_decoded`] run
/// on the same read transaction. This is the verification ADR-0018
/// Decision item 5's `verify` and `restore` call (Phase 8).
///
/// # Errors
///
/// `purse_store::verify_backup`'s, and [`check_decoded`]'.
pub fn verify_backup(
    file: &Path,
    scratch: &Path,
    network: Option<&str>,
) -> purse_store::Result<Verified> {
    purse_store::verify_backup_with(file, scratch, network, decoded_of)
}

/// [`verify_backup`]'s checks on a storage backend holding the copy
/// (`purse_store::verify_backup_on_backend_with` with [`check_decoded`]),
/// for tests and fuzzing.
///
/// # Errors
///
/// As [`verify_backup`], without its file handling.
pub fn verify_backup_on_backend(
    backend: impl StorageBackend,
    network: Option<&str>,
) -> purse_store::Result<Verified> {
    purse_store::verify_backup_on_backend_with(backend, network, decoded_of)
}

fn decoded_of(tx: &ReadTx) -> purse_store::Result<()> {
    let mut r: &ReadTx = tx;
    check_decoded(&mut r)
}
