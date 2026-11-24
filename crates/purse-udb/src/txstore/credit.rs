// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Credits and debits: `AddCredit` (`txmined.go:1384-1558`), the values a
//! credit is built from (`getStakeOpCode`, `pkScriptType`,
//! `MsgTx.PkScriptLocs`), and the mutators through which every unspent
//! output, unmined credit, unmined-input entry and unpublished mark is
//! written, so that `balances`, `balance_heights` and `acct_history`
//! change in the same transaction as the rows (ADR-0003 Decision item 4).

use std::ops::Bound;

use dcroxide_stake::TxType;
use dcroxide_txscript::stdscript::{ScriptType, determine_script_type};
use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    AcctHistory, AcctHistoryKey, BalanceClass, BalanceHeightKey, BalanceHeights, BalanceKey,
    BalanceRow, Balances, CreditFlags, CreditKey, CreditRow, Credits, CreditsInvalidated, Debits,
    Hash32, Outpoint, OutputTag, ScriptLoc, TxMined, TxRecKey, TxUnmined, UnminedCreditRow,
    UnminedCredits, UnminedInputs, Unpublished, UnpublishedFlag, Unspent, UnspentRow, WriteTx,
};

use super::chain::{block_at, header_of};
use super::record::{BlockMeta, TxRecord, display};
use super::store::{TxRead, TxStore, store_err};

/// `opNonstake` (`OP_NOP10`), the tag of an output without a stake opcode.
pub const OP_NONSTAKE: u8 = 0xb9;

/// The opcode value dcrwallet stores and returns for a tag
/// (`fetchRawCreditTagOpCode`, `txdb.go:743-766`).
pub fn tag_opcode(tag: OutputTag) -> u8 {
    match tag {
        OutputTag::NonStake => OP_NONSTAKE,
        OutputTag::Ticket => 0xba,
        OutputTag::Vote => 0xbb,
        OutputTag::Revocation => 0xbc,
        OutputTag::TicketChange => 0xbd,
        OutputTag::TreasuryGen => 0xc3,
    }
}

/// `getStakeOpCode` (`txmined.go:1436-1454`): the stake tag of a script
/// type; treasury adds are not tagged.
fn stake_tag(st: ScriptType) -> OutputTag {
    use ScriptType as S;
    match st {
        S::StakeSubmissionPubKeyHash | S::StakeSubmissionScriptHash => OutputTag::Ticket,
        S::StakeGenPubKeyHash | S::StakeGenScriptHash => OutputTag::Vote,
        S::StakeRevocationPubKeyHash | S::StakeRevocationScriptHash => OutputTag::Revocation,
        S::StakeChangePubKeyHash | S::StakeChangeScriptHash => OutputTag::TicketChange,
        S::TreasuryGenPubKeyHash | S::TreasuryGenScriptHash => OutputTag::TreasuryGen,
        _ => OutputTag::NonStake,
    }
}

/// `pkScriptType` (`txmined.go:1456-1480`): dcrwallet's stored script type
/// code (`scriptTypeUnspecified` 1 to `scriptTypeSP2SH` 8, `txdb.go:72-112`).
fn script_type_code(st: ScriptType) -> u8 {
    use ScriptType as S;
    match st {
        S::PubKeyHashEcdsaSecp256k1 => 2,
        S::PubKeyEcdsaSecp256k1 => 3,
        S::PubKeyHashEd25519 | S::PubKeyHashSchnorrSecp256k1 => 4,
        S::PubKeyEd25519 | S::PubKeySchnorrSecp256k1 => 5,
        S::ScriptHash => 6,
        S::StakeSubmissionPubKeyHash
        | S::StakeGenPubKeyHash
        | S::StakeRevocationPubKeyHash
        | S::StakeChangePubKeyHash
        | S::TreasuryGenPubKeyHash => 7,
        S::StakeSubmissionScriptHash
        | S::StakeGenScriptHash
        | S::StakeRevocationScriptHash
        | S::StakeChangeScriptHash
        | S::TreasuryGenScriptHash => 8,
        _ => 1,
    }
}

/// `compat.IsEitherCoinBaseTx`: a coinbase before or after the treasury
/// agenda (treasurybases and treasury spends included, QK-0087).
pub(crate) fn is_either_coinbase(tx: &MsgTx) -> bool {
    dcroxide_standalone::is_coin_base_tx(tx, false)
        || dcroxide_standalone::is_coin_base_tx(tx, true)
}

/// What a credit records of output `index` of `msg`: its amount, its flags
/// (`change` as given) and its script's location and type.
pub(crate) struct NewCredit {
    pub(crate) amount: i64,
    pub(crate) flags: CreditFlags,
    pub(crate) script: ScriptLoc,
}

/// The script location of output `index` of `msg`: the offset
/// `MsgTx.PkScriptLocs` gives (dcroxide's `pk_script_locs_iter`, which
/// allocates nothing), the length and dcrwallet's script type code.
///
/// # Errors
///
/// No such output: `Invalid`. A script longer than 65,535 bytes, which the
/// store's script length (u16, ADR-0003) cannot hold and Go records:
/// `Invalid` (PARITY.md, `purse-udb`).
pub(crate) fn script_loc(msg: &MsgTx, index: u32) -> Result<ScriptLoc, AnyError> {
    let i = usize::try_from(index).unwrap_or(usize::MAX);
    let (Some(out), Some(offset)) = (msg.tx_out.get(i), msg.pk_script_locs_iter().nth(i)) else {
        return Err(e!(
            Kind::Invalid,
            "transaction output index for credit does not exist"
        )
        .into());
    };
    let len = u16::try_from(out.pk_script.len()).map_err(|_| {
        AnyError::from(e!(
            Kind::Invalid,
            format!(
                "output {index} script of {} bytes exceeds the store's script length limit of 65535",
                out.pk_script.len()
            )
        ))
    })?;
    let offset = u32::try_from(offset)
        .map_err(|_| AnyError::from(e!(Kind::Invalid, "script offset exceeds 2^32")))?;
    Ok(ScriptLoc {
        offset,
        len,
        class: script_type_code(determine_script_type(out.version, &out.pk_script)),
    })
}

/// The credit `addCredit` builds for output `index` of `msg`.
pub(crate) fn new_credit(msg: &MsgTx, index: u32, change: bool) -> Result<NewCredit, AnyError> {
    let script = script_loc(msg, index)?;
    let out = &msg.tx_out[usize::try_from(index).unwrap_or(usize::MAX)];
    Ok(NewCredit {
        amount: out.value,
        flags: CreditFlags {
            change,
            tag: stake_tag(determine_script_type(out.version, &out.pk_script)),
            coinbase: is_either_coinbase(msg),
            expiry: msg.expiry != 0,
        },
        script,
    })
}

/// The balance class of a mined unspent output (`balanceFullScan`'s first
/// pass, `txmined.go:3410-3449`).
pub fn mined_class(flags: &CreditFlags) -> BalanceClass {
    match flags.tag {
        OutputTag::NonStake | OutputTag::TreasuryGen => {
            if flags.coinbase {
                BalanceClass::MinedCoinbase
            } else {
                BalanceClass::MinedRegular
            }
        }
        OutputTag::Ticket => BalanceClass::VotingAuthority,
        OutputTag::Vote | OutputTag::Revocation => BalanceClass::MinedStakeGen,
        OutputTag::TicketChange => BalanceClass::MinedTicketChange,
    }
}

/// The balance class of an unmined credit (`balanceFullScan`'s second
/// pass, `txmined.go:3486-3508`), which depends on the transaction's
/// unpublished mark.
pub fn unmined_class(flags: &CreditFlags, unpublished: bool) -> BalanceClass {
    match flags.tag {
        OutputTag::NonStake => match (flags.coinbase, unpublished) {
            (false, false) => BalanceClass::UnminedRegularPublished,
            (false, true) => BalanceClass::UnminedRegularUnpublished,
            (true, false) => BalanceClass::UnminedCoinbasePublished,
            (true, true) => BalanceClass::UnminedCoinbaseUnpublished,
        },
        OutputTag::Ticket => BalanceClass::VotingAuthority,
        OutputTag::Vote | OutputTag::Revocation => BalanceClass::UnminedStakeGen,
        OutputTag::TicketChange => BalanceClass::UnminedTicketChange,
        OutputTag::TreasuryGen => BalanceClass::UnminedTreasuryGen,
    }
}

/// The unspent row of a mined credit at `height`.
pub(crate) fn unspent_row(height: u32, credit: &CreditRow) -> UnspentRow {
    UnspentRow {
        height,
        account: credit.account,
        amount: credit.amount,
        flags: credit.flags,
        class: mined_class(&credit.flags),
        script: credit.script,
    }
}

// ---------------------------------------------------------------- balances

/// Adds (`add`) or removes one output of `amount` to the running sums of
/// (`account`, `class`), and of (`account`, `class`, `height`) for a mined
/// class. A sum whose count reaches zero is deleted; a nonzero sum with no
/// output left can only come from a mutator bypassed, and is an error.
fn bump(
    tx: &mut WriteTx<'_>,
    account: u32,
    class: BalanceClass,
    height: u32,
    amount: i64,
    add: bool,
) -> Result<(), AnyError> {
    fn one<T>(tx: &mut WriteTx<'_>, key: &T::Key, amount: i64, add: bool) -> Result<(), AnyError>
    where
        T: purse_store::Table<Value = BalanceRow>,
    {
        let mut row = tx
            .get::<T>(key)
            .map_err(store_err)?
            .unwrap_or(BalanceRow { sum: 0, count: 0 });
        if add {
            row.sum = row.sum.wrapping_add(amount);
            row.count = row.count.wrapping_add(1);
        } else {
            row.sum = row.sum.wrapping_sub(amount);
            row.count = row.count.wrapping_sub(1);
        }
        if row.count == 0 {
            if row.sum != 0 {
                return Err(e!(
                    Kind::Bug,
                    format!("balance sum {} left with no output", row.sum)
                )
                .into());
            }
            tx.delete::<T>(key).map_err(store_err)?;
        } else if row.count < 0 {
            return Err(e!(Kind::Bug, "balance count below zero").into());
        } else {
            tx.put::<T>(key, &row).map_err(store_err)?;
        }
        Ok(())
    }
    one::<Balances>(tx, &BalanceKey { account, class }, amount, add)?;
    if class.is_windowed() {
        one::<BalanceHeights>(
            tx,
            &BalanceHeightKey {
                account,
                class,
                height,
            },
            amount,
            add,
        )?;
    }
    Ok(())
}

fn bump_unspent(tx: &mut WriteTx<'_>, u: &UnspentRow, add: bool) -> Result<(), AnyError> {
    bump(tx, u.account, u.class, u.height, u.amount, add)
}

fn bump_ucredit(tx: &mut WriteTx<'_>, m: &UnminedCreditRow, add: bool) -> Result<(), AnyError> {
    bump(tx, m.account, m.class, 0, m.amount, add)
}

/// Writes the unspent output `op` (Go's `putUnspent` / `putRawUnspent`),
/// moving the balance sums from the row it replaces, if any; an output an
/// unmined transaction spends counts in no sum.
pub(crate) fn put_unspent(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    row: &UnspentRow,
) -> Result<(), AnyError> {
    let excluded = tx.contains::<UnminedInputs>(op).map_err(store_err)?;
    if !excluded && let Some(old) = tx.get::<Unspent>(op).map_err(store_err)? {
        bump_unspent(tx, &old, false)?;
    }
    tx.put::<Unspent>(op, row).map_err(store_err)?;
    if !excluded {
        bump_unspent(tx, row, true)?;
    }
    Ok(())
}

/// Deletes the unspent output `op` (`deleteRawUnspent`; a missing row is
/// no error), returning it.
pub(crate) fn del_unspent(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
) -> Result<Option<UnspentRow>, AnyError> {
    let Some(old) = tx.get::<Unspent>(op).map_err(store_err)? else {
        return Ok(None);
    };
    del_unspent_row(tx, op, &old)?;
    Ok(Some(old))
}

/// Deletes the unspent output `op` whose row the caller has just read as
/// `old` (the debit path, which reads the row to find the credit), so the
/// row is not read a second time.
pub(crate) fn del_unspent_row(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    old: &UnspentRow,
) -> Result<(), AnyError> {
    tx.delete::<Unspent>(op).map_err(store_err)?;
    if !tx.contains::<UnminedInputs>(op).map_err(store_err)? {
        bump_unspent(tx, old, false)?;
    }
    Ok(())
}

/// Writes the unmined credit `op` (`putRawUnminedCredit`), its class taken
/// from the transaction's unpublished mark.
pub(crate) fn put_ucredit(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    amount: i64,
    flags: CreditFlags,
    account: u32,
    script: ScriptLoc,
) -> Result<(), AnyError> {
    let unpublished = tx.contains::<Unpublished>(&op.hash).map_err(store_err)?;
    let row = UnminedCreditRow {
        amount,
        flags,
        account,
        class: unmined_class(&flags, unpublished),
        script,
    };
    let excluded = tx.contains::<UnminedInputs>(op).map_err(store_err)?;
    if !excluded && let Some(old) = tx.get::<UnminedCredits>(op).map_err(store_err)? {
        bump_ucredit(tx, &old, false)?;
    }
    tx.put::<UnminedCredits>(op, &row).map_err(store_err)?;
    if !excluded {
        bump_ucredit(tx, &row, true)?;
    }
    Ok(())
}

/// Deletes the unmined credit `op` (`deleteRawUnminedCredit`; a missing
/// row is no error), returning it.
pub(crate) fn del_ucredit(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
) -> Result<Option<UnminedCreditRow>, AnyError> {
    let Some(old) = tx.get::<UnminedCredits>(op).map_err(store_err)? else {
        return Ok(None);
    };
    tx.delete::<UnminedCredits>(op).map_err(store_err)?;
    if !tx.contains::<UnminedInputs>(op).map_err(store_err)? {
        bump_ucredit(tx, &old, false)?;
    }
    Ok(Some(old))
}

/// Records `spender` as the unmined transaction spending `op`
/// (`putRawUnminedInput`, which overwrites): the outpoint's unspent output
/// and unmined credit leave the balance sums when it had no spender.
pub(crate) fn put_uinput(
    tx: &mut WriteTx<'_>,
    op: &Outpoint,
    spender: &Hash32,
) -> Result<(), AnyError> {
    if !tx.contains::<UnminedInputs>(op).map_err(store_err)? {
        if let Some(u) = tx.get::<Unspent>(op).map_err(store_err)? {
            bump_unspent(tx, &u, false)?;
        }
        if let Some(m) = tx.get::<UnminedCredits>(op).map_err(store_err)? {
            bump_ucredit(tx, &m, false)?;
        }
    }
    tx.put::<UnminedInputs>(op, spender).map_err(store_err)
}

/// Deletes the unmined-input entry of `op` (`deleteRawUnminedInput`; a
/// missing row is no error): the outpoint's unspent output and unmined
/// credit return to the balance sums.
pub(crate) fn del_uinput(tx: &mut WriteTx<'_>, op: &Outpoint) -> Result<(), AnyError> {
    if !tx.delete::<UnminedInputs>(op).map_err(store_err)? {
        return Ok(());
    }
    if let Some(u) = tx.get::<Unspent>(op).map_err(store_err)? {
        bump_unspent(tx, &u, true)?;
    }
    if let Some(m) = tx.get::<UnminedCredits>(op).map_err(store_err)? {
        bump_ucredit(tx, &m, true)?;
    }
    Ok(())
}

/// The bounds of every unmined credit of `hash`.
fn ucredit_bounds(hash: &Hash32) -> (Outpoint, Outpoint) {
    (
        Outpoint {
            hash: *hash,
            index: 0,
        },
        Outpoint {
            hash: *hash,
            index: u32::MAX,
        },
    )
}

/// Sets (`putUnpublished`) or clears (`deleteUnpublished`) the unpublished
/// mark of `hash`. When the mark changes, the unmined credits of `hash`
/// change class (published and unpublished non-stake outputs count
/// differently, `txmined.go:3488-3494`).
pub(crate) fn set_unpublished(
    tx: &mut WriteTx<'_>,
    hash: &Hash32,
    unpublished: bool,
) -> Result<(), AnyError> {
    let was = tx.contains::<Unpublished>(hash).map_err(store_err)?;
    if was == unpublished {
        return Ok(());
    }
    if unpublished {
        tx.put::<Unpublished>(hash, &UnpublishedFlag)
            .map_err(store_err)?;
    } else {
        tx.delete::<Unpublished>(hash).map_err(store_err)?;
    }
    let (lo, hi) = ucredit_bounds(hash);
    let rows = tx
        .rows::<UnminedCredits>(
            Bound::Included(&lo),
            Bound::Included(&hi),
            false,
            usize::MAX,
        )
        .map_err(store_err)?;
    for (op, old) in rows {
        let class = unmined_class(&old.flags, unpublished);
        if class == old.class {
            continue;
        }
        let new = UnminedCreditRow { class, ..old };
        if !tx.contains::<UnminedInputs>(&op).map_err(store_err)? {
            bump_ucredit(tx, &old, false)?;
            bump_ucredit(tx, &new, true)?;
        }
        tx.put::<UnminedCredits>(&op, &new).map_err(store_err)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- acct_history

/// The accounts a mined record touches: the owners of its live credits and
/// of the credits its live debits spend (found in `credits`, else in
/// `credits_invalidated`, and counted only while the credit names that
/// debit as its spender; a debit whose credit is in neither table, or
/// names another spender, names no account). Sorted, without repeats.
pub(crate) fn record_accounts(
    r: &mut (impl TxRead + ?Sized),
    hash: &Hash32,
    height: u32,
) -> Result<Vec<u32>, AnyError> {
    let lo = CreditKey {
        hash: *hash,
        height,
        index: 0,
    };
    let hi = CreditKey {
        hash: *hash,
        height,
        index: u32::MAX,
    };
    let mut out = Vec::new();
    for (_, c) in r
        .rows::<Credits>(
            Bound::Included(&lo),
            Bound::Included(&hi),
            false,
            usize::MAX,
        )
        .map_err(store_err)?
    {
        out.push(c.account);
    }
    for (k, d) in r
        .rows::<Debits>(
            Bound::Included(&lo),
            Bound::Included(&hi),
            false,
            usize::MAX,
        )
        .map_err(store_err)?
    {
        let c = match r.get::<Credits>(&d.credit).map_err(store_err)? {
            Some(c) => Some(c),
            None => r.get::<CreditsInvalidated>(&d.credit).map_err(store_err)?,
        };
        let me = purse_store::Spender {
            hash: k.hash,
            height: k.height,
            input: k.index,
        };
        if let Some(c) = c.filter(|c| c.spender == Some(me)) {
            out.push(c.account);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// The mined records an operation changes the credits or debits of, with
/// their accounts and positions before the change: `acct_history` is
/// brought up to date for each when the operation ends ([`AcctTracker::flush`]).
#[derive(Default)]
pub(crate) struct AcctTracker {
    entries: Vec<(TxRecKey, Option<u32>, Vec<u32>)>,
}

impl AcctTracker {
    /// Notes the record (`hash`, `height`) before its credits or debits
    /// change (once per record and operation).
    pub(crate) fn touch(
        &mut self,
        tx: &mut WriteTx<'_>,
        hash: &Hash32,
        height: u32,
    ) -> Result<(), AnyError> {
        let key = TxRecKey {
            hash: *hash,
            height,
        };
        if self.entries.iter().any(|(k, _, _)| *k == key) {
            return Ok(());
        }
        let pos = record_position(tx, &key)?;
        let before = if pos.is_some() {
            record_accounts(tx, hash, height)?
        } else {
            Vec::new()
        };
        self.entries.push((key, pos, before));
        Ok(())
    }

    /// Rewrites the `acct_history` rows of every record noted.
    pub(crate) fn flush(self, tx: &mut WriteTx<'_>) -> Result<(), AnyError> {
        for (key, pos_before, before) in self.entries {
            if let Some(position) = pos_before {
                for account in &before {
                    tx.delete::<AcctHistory>(&AcctHistoryKey {
                        account: *account,
                        height: key.height,
                        position,
                    })
                    .map_err(store_err)?;
                }
            }
            let Some(position) = record_position(tx, &key)? else {
                continue;
            };
            for account in record_accounts(tx, &key.hash, key.height)? {
                tx.put::<AcctHistory>(
                    &AcctHistoryKey {
                        account,
                        height: key.height,
                        position,
                    },
                    &key.hash,
                )
                .map_err(store_err)?;
            }
        }
        Ok(())
    }
}

/// The position of the mined record `key` in its block, if it exists.
fn record_position(tx: &mut WriteTx<'_>, key: &TxRecKey) -> Result<Option<u32>, AnyError> {
    Ok(tx
        .get::<TxMined>(key)
        .map_err(store_err)?
        .map(|r| r.position))
}

// ---------------------------------------------------------------- AddCredit

impl TxStore {
    /// `AddCredit`: records output `index` of `rec` as a credit of
    /// `account`, `change` marking an internal-branch address. With a block
    /// (the transaction recorded mined in it), a regular transaction in a
    /// stake-invalidated block gets an invalidated credit, rewritten if one
    /// is there (spec 04 Q-25, as Go); otherwise a mined credit and
    /// its unspent output are written unless the credit exists. Without a
    /// block, an unmined credit is written unless it exists.
    ///
    /// # Errors
    ///
    /// Go's: no output `index`: `Invalid`. Where Go panics (a regular
    /// transaction's block with no header or no block row at the header's
    /// height; no unmined record of `rec` without a block): `Invalid`,
    /// nothing written. Where Go writes a credit no query reaches
    /// consistently, which the store's credit key (no block hash) cannot
    /// hold: a block that is not the main-chain block at its height, or no
    /// mined record of `rec` in it: `Invalid` (PARITY.md, `purse-udb`). A
    /// script longer than 65,535 bytes: `Invalid`. The store's.
    pub fn add_credit(
        &self,
        tx: &mut WriteTx<'_>,
        rec: &TxRecord,
        block: Option<&BlockMeta>,
        index: u32,
        change: bool,
        account: u32,
    ) -> Result<(), AnyError> {
        let msg = rec.msg_tx();
        if usize::try_from(index).map_or(true, |i| i >= msg.tx_out.len()) {
            return Err(e!(
                Kind::Invalid,
                "transaction output index for credit does not exist"
            )
            .into());
        }
        let hash = *rec.hash();
        let Some(block) = block else {
            if !tx.contains::<TxUnmined>(&hash).map_err(store_err)? {
                return Err(e!(
                    Kind::Invalid,
                    "attempted to add credit for unmined tx, but unmined tx with same hash does not exist"
                )
                .into());
            }
            let op = Outpoint { hash, index };
            if tx.contains::<UnminedCredits>(&op).map_err(store_err)? {
                return Ok(());
            }
            let c = new_credit(msg, index, change)?;
            return put_ucredit(tx, &op, c.amount, c.flags, account, c.script);
        };
        let regular = rec.tx_type() == TxType::Regular;
        // Go reads the header and the block row at its height only for a
        // regular transaction, and panics without them.
        if regular {
            let Some(header) = header_of(tx, &block.block.hash)? else {
                return Err(e!(
                    Kind::Invalid,
                    format!(
                        "AddCredit: no header for block {}, where Go panics",
                        display(&block.block.hash)
                    )
                )
                .into());
            };
            if block_at(tx, header.height())?.is_none() {
                return Err(e!(
                    Kind::Invalid,
                    format!(
                        "AddCredit: no block record at height {}, where Go panics",
                        header.height()
                    )
                )
                .into());
            }
        }
        let height = match u32::try_from(block.block.height) {
            Ok(h) => match block_at(tx, h)? {
                Some(row) if row.hash == block.block.hash => Some((h, row)),
                _ => None,
            },
            Err(_) => None,
        };
        let Some((height, row)) = height else {
            return Err(e!(
                Kind::Invalid,
                format!(
                    "credits must be added to main chain blocks: block {} is not the main chain block at height {}",
                    display(&block.block.hash),
                    block.block.height
                )
            )
            .into());
        };
        let rec_key = TxRecKey { hash, height };
        if !tx.contains::<TxMined>(&rec_key).map_err(store_err)? {
            return Err(e!(
                Kind::Invalid,
                format!(
                    "no mined record of transaction {} in block {}",
                    display(&hash),
                    display(&block.block.hash)
                )
            )
            .into());
        }
        let key = CreditKey {
            hash,
            height,
            index,
        };
        let c = new_credit(msg, index, change)?;
        let credit = CreditRow {
            amount: c.amount,
            flags: c.flags,
            account,
            script: c.script,
            spender: None,
        };
        if regular && row.invalid {
            // Go overwrites; the record spending the credit overwritten, if
            // any, loses the account its debit named.
            let mut acct = AcctTracker::default();
            if let Some(s) = tx
                .get::<CreditsInvalidated>(&key)
                .map_err(store_err)?
                .and_then(|c| c.spender)
            {
                acct.touch(tx, &s.hash, s.height)?;
            }
            tx.put::<CreditsInvalidated>(&key, &credit)
                .map_err(store_err)?;
            return acct.flush(tx);
        }
        if tx.contains::<Credits>(&key).map_err(store_err)? {
            return Ok(());
        }
        let mut acct = AcctTracker::default();
        acct.touch(tx, &hash, height)?;
        tx.put::<Credits>(&key, &credit).map_err(store_err)?;
        put_unspent(tx, &Outpoint { hash, index }, &unspent_row(height, &credit))?;
        acct.flush(tx)
    }
}

#[cfg(test)]
mod tests {
    use dcroxide_txscript::stdscript::ScriptType;
    use dcroxide_wire::{MsgTx, TxIn, TxOut};
    use purse_store::{BalanceClass, CreditFlags, OutputTag};

    use super::{
        mined_class, new_credit, script_loc, script_type_code, stake_tag, tag_opcode, unmined_class,
    };

    fn p2pkh(tag: Option<u8>) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(tag);
        s.extend_from_slice(&[0x76, 0xa9, 0x14]);
        s.extend_from_slice(&[7; 20]);
        s.extend_from_slice(&[0x88, 0xac]);
        s
    }

    /// The stored tag's three-bit code is the opcode less `OP_NOP10`, and 7
    /// for `OP_TGEN` (`txdb.go:743-766`), as the export renders `optag`.
    #[test]
    fn tags_and_opcodes_agree_with_dcrwallet() {
        for (tag, op) in [
            (OutputTag::NonStake, 0xb9u8),
            (OutputTag::Ticket, 0xba),
            (OutputTag::Vote, 0xbb),
            (OutputTag::Revocation, 0xbc),
            (OutputTag::TicketChange, 0xbd),
        ] {
            assert_eq!(tag_opcode(tag), op);
            assert_eq!(u16::from(tag as u8), u16::from(op) - 0xb9);
        }
        assert_eq!(tag_opcode(OutputTag::TreasuryGen), 0xc3);
        assert_eq!(OutputTag::TreasuryGen as u8, 7);
        assert_eq!(stake_tag(ScriptType::TreasuryAdd), OutputTag::NonStake);
        assert_eq!(script_type_code(ScriptType::NonStandard), 1);
        assert_eq!(script_type_code(ScriptType::MultiSig), 1);
        assert_eq!(script_type_code(ScriptType::PubKeyHashEcdsaSecp256k1), 2);
        assert_eq!(script_type_code(ScriptType::StakeGenScriptHash), 8);
    }

    /// Every flag combination has the class `balanceFullScan` counts it in.
    #[test]
    fn classes_follow_balance_full_scan() {
        let f = |tag, coinbase| CreditFlags {
            change: false,
            tag,
            coinbase,
            expiry: false,
        };
        assert_eq!(
            mined_class(&f(OutputTag::TreasuryGen, false)),
            BalanceClass::MinedRegular
        );
        assert_eq!(
            mined_class(&f(OutputTag::NonStake, true)),
            BalanceClass::MinedCoinbase
        );
        assert_eq!(
            mined_class(&f(OutputTag::Revocation, true)),
            BalanceClass::MinedStakeGen
        );
        assert_eq!(
            unmined_class(&f(OutputTag::NonStake, true), true),
            BalanceClass::UnminedCoinbaseUnpublished
        );
        assert_eq!(
            unmined_class(&f(OutputTag::Ticket, false), true),
            BalanceClass::VotingAuthority
        );
        assert_eq!(
            unmined_class(&f(OutputTag::TreasuryGen, false), false),
            BalanceClass::UnminedTreasuryGen
        );
        for tag in [
            OutputTag::NonStake,
            OutputTag::Ticket,
            OutputTag::Vote,
            OutputTag::Revocation,
            OutputTag::TicketChange,
            OutputTag::TreasuryGen,
        ] {
            for cb in [false, true] {
                let m = mined_class(&f(tag, cb));
                assert!(m.is_windowed() || m == BalanceClass::VotingAuthority);
                for unpublished in [false, true] {
                    let u = unmined_class(&f(tag, cb), unpublished);
                    assert!(!u.is_windowed());
                }
            }
        }
    }

    /// A credit's location is dcroxide's `pk_script_locs` entry and the
    /// script's length; a script the store's u16 length cannot hold is
    /// refused, as is an output that does not exist.
    #[test]
    fn script_locations_come_from_pk_script_locs() {
        let m = MsgTx {
            tx_in: vec![TxIn::default()],
            tx_out: vec![
                TxOut {
                    value: 5,
                    version: 0,
                    pk_script: p2pkh(None),
                },
                TxOut {
                    value: 6,
                    version: 0,
                    pk_script: p2pkh(Some(0xbb)),
                },
            ],
            expiry: 9,
            ..MsgTx::default()
        };
        let locs = m.pk_script_locs();
        let bytes = m.serialize();
        for (i, loc) in locs.iter().enumerate() {
            let s = script_loc(&m, u32::try_from(i).expect("small")).expect("loc");
            assert_eq!(s.offset as usize, *loc);
            let end = loc + usize::from(s.len);
            assert_eq!(&bytes[*loc..end], m.tx_out[i].pk_script.as_slice());
        }
        let c = new_credit(&m, 1, true).expect("credit");
        assert_eq!(c.amount, 6);
        assert_eq!(c.flags.tag, OutputTag::Vote);
        assert!(c.flags.change && c.flags.expiry && !c.flags.coinbase);
        assert_eq!(c.script.class, 7);
        assert!(script_loc(&m, 2).is_err());
        let long = MsgTx {
            tx_out: vec![TxOut {
                value: 1,
                version: 0,
                pk_script: vec![0x6a; 65_536],
            }],
            ..MsgTx::default()
        };
        let e = script_loc(&long, 0).expect_err("too long");
        assert!(e.is_kind(purse_errors::Kind::Invalid));
    }
}
