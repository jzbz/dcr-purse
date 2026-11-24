// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Balances: `AccountBalance` and `AccountBalances` (`balanceFullScan`,
//! `txmined.go:3356-3583`) over the running sums of `balances` and
//! `balance_heights` (ADR-0003 Decision item 4), and `UnspentTickets`
//! (`txmined.go:2652-2722`).

use std::ops::Bound;

use dcroxide_wire::MsgTx;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    BalanceClass, BalanceHeightKey, BalanceHeights, BalanceKey, Balances as BalanceSums, Hash32,
    Outpoint, Tickets, TxUnmined, UnminedInputs, Unspent,
};

use super::record::{display, io};
use super::store::{TxRead, TxStore, store_err};
use super::unmined::latest_mined_height;

/// An account's balances (dcrwallet's `Balances`), in atoms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Balances {
    /// The account.
    pub account: u32,
    /// Coinbase outputs not yet mature.
    pub immature_coinbase_rewards: i64,
    /// Vote and revocation outputs not yet mature, and unmined ones.
    pub immature_stake_generation: i64,
    /// Ticket commitments not spent by an unmined vote or revocation.
    pub locked_by_tickets: i64,
    /// Spendable at the confirmation depth asked for.
    pub spendable: i64,
    /// Everything counted.
    pub total: i64,
    /// Ticket outputs.
    pub voting_authority: i64,
    /// Unmined outputs not spendable at the depth asked for.
    pub unconfirmed: i64,
}

impl TxStore {
    /// The sum of the outputs of (`account`, `class`) mined above
    /// `threshold`, read from the high end of `balance_heights`, the side
    /// of the depth window that is short for every depth a wallet asks
    /// for (the recent blocks).
    fn sum_above(
        r: &mut (impl TxRead + ?Sized),
        account: u32,
        class: BalanceClass,
        threshold: i64,
    ) -> Result<i64, AnyError> {
        // Every height is above a negative threshold, none above u32::MAX.
        let lo = match u32::try_from(threshold) {
            Ok(t) => match t.checked_add(1) {
                Some(l) => l,
                None => return Ok(0),
            },
            Err(_) if threshold < 0 => 0,
            Err(_) => return Ok(0),
        };
        let lo = BalanceHeightKey {
            account,
            class,
            height: lo,
        };
        let hi = BalanceHeightKey {
            account,
            class,
            height: u32::MAX,
        };
        let mut sum = 0i64;
        for (_, row) in r
            .rows::<BalanceHeights>(
                Bound::Included(&lo),
                Bound::Included(&hi),
                false,
                usize::MAX,
            )
            .map_err(store_err)?
        {
            sum = sum.wrapping_add(row.sum);
        }
        Ok(sum)
    }

    /// `AccountBalances`: the balances at `min_conf` confirmations of every
    /// account with an output counted (an unspent output or unmined credit
    /// no unmined transaction spends), in account order (Go returns a map),
    /// with the tip as the sync height.
    ///
    /// Each class's sum is read whole from `balances`; for the mined
    /// classes the outputs too recent for the depth or the maturity asked
    /// are summed from `balance_heights` and moved to the immature
    /// categories, as `balanceFullScan` decides per output.
    ///
    /// # Errors
    ///
    /// No tip: `IO`. The store's.
    pub fn account_balances(
        &self,
        r: &mut (impl TxRead + ?Sized),
        min_conf: i32,
    ) -> Result<Vec<Balances>, AnyError> {
        let (_, tip) = self.main_chain_tip(r)?;
        let tip = i64::from(tip);
        let rows = r
            .rows::<BalanceSums>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        let mut out: Vec<Balances> = Vec::new();
        for (BalanceKey { account, class }, row) in rows {
            if out.last().is_none_or(|b| b.account != account) {
                out.push(Balances {
                    account,
                    ..Balances::default()
                });
            }
            let sum = row.sum;
            // The mined outputs above a height are immature, or not yet
            // confirmed: confirmed(minConf, h, tip) holds for h at most
            // tip + 1 - minConf; a coinbase, vote or revocation output is
            // mature for h at most tip - CoinbaseMaturity, ticket change
            // for h at most tip - SStxChangeMaturity.
            let recent = |r: &mut _, threshold: i64| Self::sum_above(r, account, class, threshold);
            let mut b = out.last().copied().unwrap_or_default();
            match class {
                BalanceClass::MinedRegular => {
                    let unconfirmed = if min_conf <= 0 {
                        0
                    } else {
                        recent(r, tip.wrapping_add(1).wrapping_sub(i64::from(min_conf)))?
                    };
                    b.spendable = b.spendable.wrapping_add(sum.wrapping_sub(unconfirmed));
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::MinedCoinbase => {
                    let immature = recent(r, tip.wrapping_sub(i64::from(self.coinbase_maturity)))?;
                    b.spendable = b.spendable.wrapping_add(sum.wrapping_sub(immature));
                    b.immature_coinbase_rewards =
                        b.immature_coinbase_rewards.wrapping_add(immature);
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::MinedStakeGen => {
                    let immature = recent(r, tip.wrapping_sub(i64::from(self.coinbase_maturity)))?;
                    b.spendable = b.spendable.wrapping_add(sum.wrapping_sub(immature));
                    b.immature_stake_generation =
                        b.immature_stake_generation.wrapping_add(immature);
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::MinedTicketChange => {
                    let immature =
                        recent(r, tip.wrapping_sub(i64::from(self.sstx_change_maturity)))?;
                    b.spendable = b.spendable.wrapping_add(sum.wrapping_sub(immature));
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::VotingAuthority => {
                    b.voting_authority = b.voting_authority.wrapping_add(sum);
                }
                BalanceClass::UnminedRegularPublished => {
                    if min_conf == 0 {
                        b.spendable = b.spendable.wrapping_add(sum);
                    } else {
                        b.unconfirmed = b.unconfirmed.wrapping_add(sum);
                    }
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::UnminedRegularUnpublished => {
                    b.unconfirmed = b.unconfirmed.wrapping_add(sum);
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::UnminedCoinbasePublished => {
                    if min_conf == 0 {
                        b.spendable = b.spendable.wrapping_add(sum);
                    }
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::UnminedCoinbaseUnpublished | BalanceClass::UnminedTicketChange => {
                    b.total = b.total.wrapping_add(sum);
                }
                BalanceClass::UnminedStakeGen => {
                    b.immature_stake_generation = b.immature_stake_generation.wrapping_add(sum);
                    b.total = b.total.wrapping_add(sum);
                }
                // Go counts an unmined treasury-generated output in no
                // category, but its account appears.
                BalanceClass::UnminedTreasuryGen => {}
                BalanceClass::Locked => {
                    b.locked_by_tickets = b.locked_by_tickets.wrapping_add(sum);
                    b.total = b.total.wrapping_add(sum);
                }
            }
            if let Some(last) = out.last_mut() {
                *last = b;
            }
        }
        Ok(out)
    }

    /// `AccountBalance`: [`TxStore::account_balances`] of `account`, zero
    /// balances when it has no output counted.
    ///
    /// # Errors
    ///
    /// As [`TxStore::account_balances`].
    pub fn account_balance(
        &self,
        r: &mut (impl TxRead + ?Sized),
        min_conf: i32,
        account: u32,
    ) -> Result<Balances, AnyError> {
        Ok(self
            .account_balances(r, min_conf)?
            .into_iter()
            .find(|b| b.account == account)
            .unwrap_or(Balances {
                account,
                ..Balances::default()
            }))
    }

    /// `UnspentTickets`: the tickets of the ticket set whose output 0 is an
    /// unspent output, in hash order, except those an unmined transaction
    /// spends, unless that is a vote on a block other than the tip; without
    /// `include_immature`, only those mined at least `TicketMaturity + 1`
    /// blocks below `sync_height`, the difference taken in 32 bits with
    /// Go's wrap, so a `sync_height` more than 2^31 below the ticket's
    /// height counts it as mature (QK-0784, reproduced). An unmined ticket
    /// has no unspent output and is never returned (QK-0090).
    ///
    /// # Errors
    ///
    /// An unmined spender that does not decode: `IO`. An unmined-inputs
    /// entry naming a spender with no unmined record, where Go slices the
    /// missing value and panics (QK-0048; ADR-0012 Rule 2(a)): `IO`. No
    /// tip: `IO`. The store's.
    pub fn unspent_tickets(
        &self,
        r: &mut (impl TxRead + ?Sized),
        sync_height: i32,
        include_immature: bool,
    ) -> Result<Vec<Hash32>, AnyError> {
        let (tip_hash, _) = self.main_chain_tip(r)?;
        let hashes = r
            .keys::<Tickets>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?;
        let mut out = Vec::new();
        for hash in hashes {
            let op = Outpoint { hash, index: 0 };
            if !r.contains::<Unspent>(&op).map_err(store_err)? {
                continue;
            }
            if let Some(spender) = r.get::<UnminedInputs>(&op).map_err(store_err)? {
                // Go's `serializedSpender == nil` test is dead: its
                // `extractRawUnminedTx` slices the missing value first and
                // panics (QK-0048), so missing stored state is `IO` here,
                // as in `InsertMemPoolTx`.
                let Some(row) = r.get::<TxUnmined>(&spender).map_err(store_err)? else {
                    return Err(e!(
                        Kind::Io,
                        format!(
                            "unmined input {}:0 names {}, which is not an unmined transaction",
                            display(&hash),
                            display(&spender)
                        )
                    )
                    .into());
                };
                let (spender_tx, _) =
                    MsgTx::from_bytes(&row.tx).map_err(|err| io(AnyError::foreign(err)))?;
                if dcroxide_stake::is_ssgen(&spender_tx)
                    && dcroxide_stake::ssgen_block_voted_on(&spender_tx).0.0 != tip_hash.0
                {
                    out.push(hash);
                }
                continue;
            }
            if !include_immature {
                let Some(height) = latest_mined_height(r, &hash)? else {
                    continue;
                };
                if !ticket_matured(self.ticket_maturity, height, sync_height) {
                    continue;
                }
            }
            out.push(hash);
        }
        Ok(out)
    }
}

/// `ticketMatured` (`txmined.go:2962-2967`): whether a ticket mined at
/// `tx_height` has matured at `cur_height`. Go reads the stored height as
/// an `int32` (`readRawTxRecordBlockHeight`) and subtracts in `int32`,
/// which wraps: a `cur_height` more than 2^31 below the ticket's height
/// gives a large positive difference and counts the ticket as mature
/// (QK-0784, reproduced under ADR-0012 Rule 1). The only caller in
/// dcrwallet passes the tip height and asks for immature tickets too
/// (`wallet/wallet.go:1178`), so the wrap is reached only through the
/// store's API.
fn ticket_matured(ticket_maturity: i32, tx_height: u32, cur_height: i32) -> bool {
    let tx_height = tx_height.cast_signed();
    tx_height >= 0 && cur_height.wrapping_sub(tx_height) > ticket_maturity
}

#[cfg(test)]
mod tests {
    use super::ticket_matured;

    /// Go's `ticketMatured` at its boundary and at the `int32` wrap
    /// (QK-0784), with simnet's `TicketMaturity` of 16.
    #[test]
    fn ticket_matured_is_gos() {
        for (tx_height, cur_height, want) in [
            (25, 41, false),
            (25, 42, true),
            (0, 17, true),
            (0, 16, false),
            (25, i32::MAX, true),
            // 25 - 2^31: the difference is i32::MIN, no wrap.
            (25, -2_147_483_623, false),
            // One lower: the difference wraps to i32::MAX.
            (25, -2_147_483_624, true),
            (25, i32::MIN, true),
            (2, -2_147_483_647, true),
            // A stored height of 2^31 or more is negative as Go reads it.
            (0x8000_0000, i32::MAX, false),
            (u32::MAX, 100, false),
        ] {
            assert_eq!(
                ticket_matured(16, tx_height, cur_height),
                want,
                "{tx_height} at {cur_height}"
            );
        }
    }
}
