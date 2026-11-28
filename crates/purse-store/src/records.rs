// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The records ADR-0003 Decision item 2 has Phase 2 add: multisig outputs,
//! VSP tickets and hosts, treasury and agenda voting policies (spec 05's
//! auxiliary buckets), and the operations that keep their indexes.
//!
//! # Layout
//!
//! | table | key | value | dcrwallet bucket and access path |
//! |---|---|---|---|
//! | [`MultisigOuts`] | [`Outpoint`] | [`MultisigOutRow`] | `ms`: `GetMultisigOutput`, `AddMultisigOut`, `SpendMultisigOut` |
//! | [`MultisigUnspent`] | [`MultisigUnspentKey`] (script HASH160 ‖ outpoint) | [`Empty`] | `mu`: `UnspentMultisigCreditsForAddress` |
//! | [`VspTickets`] | ticket [`Hash32`] | [`VspTicketRow`] | `vsp`: `GetVSPTicket`, `SetVSPTicket`, `DeleteVSPTicket` |
//! | [`VspFeeStatus`] | [`VspFeeStatusKey`] (status ‖ ticket) | [`Empty`] | none (an index): `GetVSPTicketsByFeeStatus` |
//! | [`VspHosts`] | [`VspHostId`] | [`VspHost`] | `vsphost`: `GetVSPHost`, `GetVSPTicket` |
//! | [`VspPubKeys`] | [`VspHost`] | [`VspPubKeyRow`] | `vsppubkey`: `GetVSPPubKey`, `SetVSPTicket` |
//! | [`TSpendPolicies`] | tspend [`Hash32`] | [`TreasuryVote`] | `tspendpolicy` |
//! | [`TreasuryKeyPolicies`] | [`PiKey`] | [`TreasuryVote`] | `treasurypolicy` |
//! | [`VspTSpendPolicies`] | [`VspTSpendKey`] | [`TreasuryVote`] | `vsptspendpolicy` |
//! | [`VspTreasuryKeyPolicies`] | [`VspTreasuryKey`] | [`TreasuryVote`] | `vsptreasurypolicy` |
//! | [`AgendaPrefs`] | [`AgendaKey`] | [`AgendaChoice`] | `agendaprefs` |
//! | [`TicketAgendaPrefs`] | [`TicketAgendaKey`] | [`AgendaChoice`] | `ticketsagendaprefs` (nested buckets flattened) |
//!
//! Each key type's documentation names the dcrwallet code its bytes follow
//! and whether its order reaches a client. Only one does: the unspent
//! multisig outputs of a script, which `redeemmultisigouts` lists in the
//! order of dcrwallet's `mu` walk (raw transaction hash, then big-endian
//! index); `multisig_unspent` keys them by the script first and the
//! outpoint second, so one prefix walk returns exactly that sequence
//! (`tests/conformance.rs` holds it against bbolt). The other readers are
//! lookups by key, or return a Go map, whose order is random (QK-0125;
//! the four policy maps; `GetVSPTicketsByFeeStatus`).
//!
//! dcrwallet's `vsphostindex` counter row is not kept: hosts are never
//! deleted and every host id is the last one plus one, so the last key of
//! `vsp_hosts` is the counter (as the length of `unspent` is `KeyN`;
//! ADR-0003, addendum "Schema version 1 as built"). Nor is the reserved
//! host id 0 with an empty host that dcrwallet's upgrade 23 seeds:
//! `GetVSPTicket` treats an absent host and an empty one alike
//! (`wallet/udb/vsp.go:112-123`).
//!
//! Imported keys and scripts, and voting accounts, need no table of their
//! own: an imported key or script is an [`AddrRow::Imported`](crate::AddrRow)
//! row of `addr_by_hash`, listed under its account by `addr_by_acct_sha`
//! (whose prefix walk of the imported account 0x7FFFFFFF is dcrwallet's
//! `ForEachAccountAddress` order, which `getaddressesbyaccount "imported"`
//! returns); a voting account is an `accounts` row of its own type whose
//! key material carries the Argon2id parameters (`purse-udb`'s
//! `AccountKeys`).
//!
//! # Invariants
//!
//! The operations below hold these, and [`ReadTx::check_records`] checks
//! every one:
//!
//! 1. `multisig_unspent` holds exactly one entry per unspent
//!    `multisig_outs` row, under that row's script hash, and nothing else.
//! 2. `vsp_fee_status` holds exactly one entry per `vsp_tickets` row,
//!    under that row's status, and nothing else.
//! 3. `vsp_hosts` and `vsp_pubkeys` are each other's inverse: every host id
//!    maps to a host whose public-key row names that id, and the reverse;
//!    no id is 0 and no host is empty.
//! 4. Every `vsp_tickets` row's host id is 0 or a key of `vsp_hosts`.
//!
//! The policy and agenda tables have no invariant beyond their encodings
//! and are written with the generic [`WriteTx::put`] and
//! [`WriteTx::delete`]: dcrwallet's "no policy" (0) and "no choice" are
//! absent rows ([`TreasuryVote`] cannot hold 0). One exception:
//! [`WriteTx::put_ticket_agenda_pref`] writes `ticket_agenda_prefs` with
//! bbolt's key limit applied to dcrwallet's nested key (version ‖ agenda
//! id) rather than to the flattened one, which is 32 bytes longer. The
//! generic writes reach the indexed tables too, for migrations and tests,
//! unchecked.
//!
//! # Operations
//!
//! As the chain operations do, each checks what it can refuse before it
//! writes (kind `Invalid`), so a refused call leaves the transaction
//! usable; an engine error part-way (kind `Io`) is the caller's to
//! abandon.
//!
//! [`TSpendPolicies`]: crate::TSpendPolicies
//! [`TreasuryKeyPolicies`]: crate::TreasuryKeyPolicies
//! [`VspTSpendPolicies`]: crate::VspTSpendPolicies
//! [`VspTreasuryKeyPolicies`]: crate::VspTreasuryKeyPolicies
//! [`AgendaPrefs`]: crate::AgendaPrefs
//! [`TicketAgendaPrefs`]: crate::TicketAgendaPrefs
//! [`PiKey`]: crate::PiKey
//! [`VspTSpendKey`]: crate::VspTSpendKey
//! [`VspTreasuryKey`]: crate::VspTreasuryKey
//! [`AgendaKey`]: crate::AgendaKey
//! [`TicketAgendaKey`]: crate::TicketAgendaKey
//! [`TreasuryVote`]: crate::TreasuryVote
//! [`AgendaChoice`]: crate::AgendaChoice

use crate::codec::Codec;
use crate::error::{Error, Result};
use crate::keys::{
    Hash32, Hash160, MultisigUnspentKey, Outpoint, TicketAgendaKey, VspFeeStatusKey, VspHost,
    VspHostId,
};
use crate::schema::{
    MultisigOuts, MultisigUnspent, Table, TableId, TicketAgendaPrefs, VspFeeStatus, VspHosts,
    VspPubKeys, VspTickets,
};
use crate::txn::{MAX_KEY_SIZE, ReadTx, WriteReader, WriteTx};
use crate::values::{AgendaChoice, Empty, MultisigOutRow, VspPubKeyRow, VspTicketRow};

/// The text of [`WriteTx::add_vsp_host`]'s refusal once every host id is
/// taken (dcrwallet's counter wraps to 0 there and overwrites the reserved
/// row, `wallet/udb/vsp.go:55-57`; four billion hosts cannot be reached).
pub const VSP_HOST_IDS_EXHAUSTED_TEXT: &str = "no VSP host id left";

/// The text of [`WriteTx::put_vsp_ticket`]'s refusal of a host id with no
/// host.
pub const UNKNOWN_VSP_HOST_TEXT: &str = "VSP ticket names an unknown host id";

fn unspent_key(op: &Outpoint, row: &MultisigOutRow) -> MultisigUnspentKey {
    MultisigUnspentKey {
        script_hash: row.script_hash,
        outpoint: *op,
    }
}

fn status_key(ticket: &Hash32, row: &VspTicketRow) -> VspFeeStatusKey {
    VspFeeStatusKey {
        status: row.fee_status,
        ticket: *ticket,
    }
}

/// bbolt's refusals of a host as a key (`bbolt@v1.3.12/bucket.go:286-289`),
/// with the texts [`WriteTx::put`] gives them.
fn check_host_key(host: &[u8]) -> Result<()> {
    if host.is_empty() {
        Err(Error::invalid("key required"))
    } else if host.len() > MAX_KEY_SIZE {
        Err(Error::invalid("key too large"))
    } else {
        Ok(())
    }
}

macro_rules! record_readers {
    ($($m:ident)?) => {
        /// The id of the VSP host `host` (`None`: never recorded).
        /// dcrwallet's `GetVSPPubKey(host).ID`.
        pub fn vsp_host_id(&$($m)? self, host: &[u8]) -> Result<Option<u32>> {
            if host.is_empty() || host.len() > MAX_KEY_SIZE {
                return Ok(None);
            }
            Ok(self.get::<VspPubKeys>(&VspHost(host.to_vec()))?.map(|r| r.host_id))
        }

        /// The tickets whose VSP fee status is `status`, in ticket hash
        /// order: one prefix walk of `vsp_fee_status`
        /// (`GetVSPTicketsByFeeStatus`, whose Go map has no order).
        pub fn vsp_tickets_with_fee_status(&$($m)? self, status: u32) -> Result<Vec<Hash32>> {
            self.prefix::<VspFeeStatus>(&status.to_be_bytes())?
                .map(|r| r.map(|(k, _)| k.ticket))
                .collect()
        }

        /// The unspent multisig outputs paying the script `script_hash`,
        /// in dcrwallet's order (its `mu` walk, filtered by script,
        /// `UnspentMultisigCreditsForAddress`): one prefix walk of
        /// `multisig_unspent`.
        pub fn unspent_multisig_for(&$($m)? self, script_hash: &Hash160) -> Result<Vec<Outpoint>> {
            self.prefix::<MultisigUnspent>(&script_hash.0)?
                .map(|r| r.map(|(k, _)| k.outpoint))
                .collect()
        }
    };
}

/// The record tables' check, defined for a read transaction and for a
/// write transaction's shared-borrow reads (`WriteTx::check_records`).
macro_rules! records_check {
    () => {
        /// Checks every invariant of the record tables (module documentation
        /// of [`crate::records`]) by a walk of the indexed ones, and fails
        /// with a kind-`Io` error naming the first that does not hold
        /// ([`Error::inconsistency`]); a row that does not decode is a
        /// corrupt-row error. Costs time linear in those tables; for tests,
        /// and for a check of a store whose origin is not trusted.
        pub fn check_records(&self) -> Result<()> {
            let bad = |why: String| Err(Error::inconsistent(why));
            let mut unspent = 0u64;
            for r in self.iter::<MultisigOuts>()? {
                let (op, row) = r?;
                if row.spender.is_none() {
                    unspent = unspent.saturating_add(1);
                    if !self.contains::<MultisigUnspent>(&unspent_key(&op, &row))? {
                        return bad("an unspent multisig output without its index entry".to_owned());
                    }
                }
            }
            if self.len(TableId::MultisigUnspent)? != unspent {
                return bad(format!(
                    "multisig_unspent holds {} entries for {unspent} unspent multisig outputs",
                    self.len(TableId::MultisigUnspent)?
                ));
            }
            for r in self.iter::<VspTickets>()? {
                let (ticket, row) = r?;
                if !self.contains::<VspFeeStatus>(&status_key(&ticket, &row))? {
                    return bad("a VSP ticket without its fee status entry".to_owned());
                }
                if row.host_id != 0 && !self.contains::<VspHosts>(&VspHostId(row.host_id))? {
                    return bad(format!(
                        "a VSP ticket names host id {} with no host",
                        row.host_id
                    ));
                }
            }
            if self.len(TableId::VspFeeStatus)? != self.len(TableId::VspTickets)? {
                return bad("vsp_fee_status and vsp_tickets differ in length".to_owned());
            }
            if self.len(TableId::VspHosts)? != self.len(TableId::VspPubKeys)? {
                return bad("vsp_hosts and vsp_pubkeys differ in length".to_owned());
            }
            for r in self.iter::<VspHosts>()? {
                let (id, host) = r?;
                if id.0 == 0 || host.0.is_empty() {
                    return bad("a VSP host with id 0 or an empty host".to_owned());
                }
                match self.get::<VspPubKeys>(&host)? {
                    Some(p) if p.host_id == id.0 => {}
                    _ => {
                        return bad(format!(
                            "VSP host id {} has no public-key row naming it",
                            id.0
                        ));
                    }
                }
            }
            Ok(())
        }
    };
}

impl ReadTx {
    record_readers!();
    records_check!();
}

impl WriteReader<'_, '_> {
    records_check!();
}

impl WriteTx<'_> {
    record_readers!(mut);

    /// [`ReadTx::check_records`] over this transaction's rows, its own
    /// writes included (opening, and so creating, every table first): a
    /// check of a state before it is committed, for tests.
    pub fn check_records(&mut self) -> Result<()> {
        self.checked_reader()?.check_records()
    }

    /// Writes the multisig output at `op` (dcrwallet's
    /// `putMultisigOutRawValues`), keeping `multisig_unspent`: the entry
    /// of the row it replaces goes, and the new row's goes in when it is
    /// unspent (`putMultisigOutUS`, `deleteMultisigOutUS`). Mining,
    /// spending, unspending and unmining an output are each one call with
    /// the changed row.
    pub fn put_multisig_out(&mut self, op: &Outpoint, row: &MultisigOutRow) -> Result<()> {
        if let Some(old) = self.get::<MultisigOuts>(op)?
            && old.spender.is_none()
        {
            self.delete::<MultisigUnspent>(&unspent_key(op, &old))?;
        }
        self.put::<MultisigOuts>(op, row)?;
        if row.spender.is_none() {
            self.put::<MultisigUnspent>(&unspent_key(op, row), &Empty)?;
        }
        Ok(())
    }

    /// Removes the multisig output at `op` and its `multisig_unspent`
    /// entry, returning the row. (dcrwallet never removes one; this is
    /// for a rescan's rebuild and for tests.)
    pub fn delete_multisig_out(&mut self, op: &Outpoint) -> Result<Option<MultisigOutRow>> {
        let old = self.take::<MultisigOuts>(op)?;
        if let Some(o) = &old
            && o.spender.is_none()
        {
            self.delete::<MultisigUnspent>(&unspent_key(op, o))?;
        }
        Ok(old)
    }

    /// The id of the VSP host `host`, recording it first if it is new:
    /// the host part of dcrwallet's `SetVSPTicket`
    /// (`wallet/udb/vsp.go:47-82`). A known host keeps its id and the
    /// public key recorded on its first use, `pubkey` being ignored, as in
    /// dcrwallet (QK-0122). A new host gets the highest id given so far
    /// plus one (1 for the first), and is written to `vsp_hosts` and
    /// `vsp_pubkeys` together.
    ///
    /// Refused before anything is written: an empty host, as bbolt refuses
    /// its key (`invalid operation: key required`, where dcrwallet has
    /// written the host row and the counter by then and its caller
    /// abandons the transaction, QK-0122); a host longer than
    /// [`MAX_KEY_SIZE`] (`invalid operation: key too large`); a new host
    /// when id `u32::MAX` is taken ([`VSP_HOST_IDS_EXHAUSTED_TEXT`]).
    pub fn add_vsp_host(&mut self, host: &[u8], pubkey: &[u8]) -> Result<u32> {
        check_host_key(host)?;
        if let Some(id) = self.vsp_host_id(host)? {
            return Ok(id);
        }
        let last = self.last::<VspHosts>()?.map_or(0, |(id, _)| id.0);
        let id = last
            .checked_add(1)
            .ok_or_else(|| Error::invalid(VSP_HOST_IDS_EXHAUSTED_TEXT))?;
        let host = VspHost(host.to_vec());
        let row = VspPubKeyRow {
            host_id: id,
            pubkey: pubkey.to_vec(),
        };
        crate::txn::check_value_len(Some(row.encode().len()))?;
        self.put::<VspHosts>(&VspHostId(id), &host)?;
        self.put::<VspPubKeys>(&host, &row)?;
        Ok(id)
    }

    /// Writes a ticket's VSP record (the record part of dcrwallet's
    /// `SetVSPTicket`), keeping `vsp_fee_status`. Refused before anything
    /// is written: a host id other than 0 that `vsp_hosts` does not hold
    /// ([`UNKNOWN_VSP_HOST_TEXT`]); dcrwallet's caller always passes the id
    /// [`WriteTx::add_vsp_host`] returns.
    pub fn put_vsp_ticket(&mut self, ticket: &Hash32, row: &VspTicketRow) -> Result<()> {
        if row.host_id != 0 && !self.contains::<VspHosts>(&VspHostId(row.host_id))? {
            return Err(Error::invalid(UNKNOWN_VSP_HOST_TEXT));
        }
        if let Some(old) = self.get::<VspTickets>(ticket)? {
            self.delete::<VspFeeStatus>(&status_key(ticket, &old))?;
        }
        self.put::<VspTickets>(ticket, row)?;
        self.put::<VspFeeStatus>(&status_key(ticket, row), &Empty)
    }

    /// Removes a ticket's VSP record and its fee-status entry; whether
    /// there was one (dcrwallet's `DeleteVSPTicket`, which does not fail
    /// on a missing ticket).
    pub fn delete_vsp_ticket(&mut self, ticket: &Hash32) -> Result<bool> {
        match self.take::<VspTickets>(ticket)? {
            Some(old) => {
                self.delete::<VspFeeStatus>(&status_key(ticket, &old))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Writes a ticket's agenda choice (the put of dcrwallet's
    /// `setTicketPreference`, `wallet/udb/agendas.go:38-44`), with bbolt's
    /// limits applied to dcrwallet's key, not to this table's.
    ///
    /// dcrwallet keys the choice by version ‖ agenda id inside the
    /// ticket's nested bucket; the flattened key here is 32 bytes longer,
    /// so the generic [`WriteTx::put`] would refuse an agenda id of 32,733
    /// to 32,764 bytes that dcrwallet stores (a flattened key of 32,769 to
    /// 32,800 bytes). This write refuses exactly
    /// what dcrwallet's put refuses: a version ‖ agenda key longer than
    /// [`MAX_KEY_SIZE`] (`invalid operation: key too large`; the key is
    /// never empty) and a value longer than
    /// [`MAX_VALUE_SIZE`](crate::MAX_VALUE_SIZE). Nothing is written then.
    pub fn put_ticket_agenda_pref(
        &mut self,
        key: &TicketAgendaKey,
        choice: &AgendaChoice,
    ) -> Result<()> {
        if key.agenda.len().saturating_add(4) > MAX_KEY_SIZE {
            return Err(Error::invalid("key too large"));
        }
        crate::txn::check_value_len(Some(choice.0.len()))?;
        let k = key.encode();
        self.table_mut(TicketAgendaPrefs::ID)?
            .insert(k.as_slice(), choice.0.as_slice())?;
        Ok(())
    }
}
