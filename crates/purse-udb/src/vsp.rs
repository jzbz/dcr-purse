// Copyright (c) 2020-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's VSP ticket records (`wallet/udb/vsp.go`; spec 05 sections
//! 3.3, 4.3 and 5.7) over `purse-store`'s `vsp_tickets`, `vsp_fee_status`,
//! `vsp_hosts` and `vsp_pubkeys` tables ([`purse_store::records`]).
//!
//! For each ticket handed to a Voting Service Provider the wallet records
//! the fee transaction's hash, the fee-payment status and the VSP's host;
//! a host is entered once, with the public key given on its first use, and
//! is never changed or removed afterwards. Every function takes the
//! caller's transaction, as Go's take a `walletdb` transaction, and none
//! begins or ends one.
//!
//! # As Go behaves
//!
//! - [`set_vsp_ticket`] gives a new host the next id (1 for the first, in
//!   order of first use) and keeps a known host's id and first public key,
//!   ignoring the public key passed (QK-0122), and ignores the host id in
//!   the record it is given, returning the id it stored where Go writes it
//!   into the caller's record (QK-0122; spec 05 Q23). An empty host is
//!   refused with bbolt's `invalid operation: key required` and a host
//!   longer than 32,768 bytes with `invalid operation: key too large`, the
//!   kinds and texts of the put that fails in Go; Go has written the host
//!   row and the counter by then, which its caller's transaction rolls
//!   back, and dcr-purse writes nothing (QK-0122). The fee hash and status
//!   are stored as given.
//! - [`get_vsp_ticket`] resolves the host and its public key as Go does:
//!   a ticket naming no host (id 0) answers an empty host and no key.
//! - [`get_vsp_tickets_by_fee_status`] answers the tickets of one status
//!   with their host ids but without host or public key, as Go's does. Go
//!   returns a map, in random order (QK-0125); this answers in ticket hash
//!   order (internal bytes, ADR-0012 Rule 3(a)), read from the
//!   `vsp_fee_status` index rather than from every ticket (ADR-0003 R3).
//!   Go's argument is an `int` compared with the stored `uint32`, so a
//!   negative status or one above 2^32 − 1 matches nothing; the argument
//!   here is an `i64` with that rule.
//! - [`get_vsp_host`] answers id 0 with the empty host dcrwallet's upgrade
//!   23 seeds (`wallet/udb/upgrades.go:1575-1585`), which the store does
//!   not keep (ADR-0003 R2), and [`get_vsp_pub_key`] answers an empty host
//!   as missing, as bbolt's `Get` of an empty key does.
//!
//! # Divergences
//!
//! - `GetVSPHost`'s missing-id text formats a pointer to the id with `%v`
//!   (`no VSP host info for id 0xc000…`, `wallet/udb/vsp.go:203`), which
//!   no run reproduces; [`get_vsp_host`] writes the id in decimal
//!   (QK-0123). No caller outside udb reads the text at the pin, and both
//!   oracle runners rewrite it to `<pointer>` before comparing.
//! - `SetVSPHost` and `SetVSPPubKey` are not ported: they write one host
//!   table without the other under any id, which no caller at the pin does
//!   outside `SetVSPTicket`, and the store keeps the two tables each
//!   other's inverse with the last id as the counter (ADR-0003 R2,
//!   decided by the project owner on 2026-11-21; QK-0047). A host is
//!   written only by [`set_vsp_ticket`], through
//!   [`WriteTx::add_vsp_host`].
//! - The public key [`get_vsp_ticket`] returns is a copy; Go's aliases the
//!   database page, invalid once the transaction ends (QK-0124).
//! - Go panics on a malformed stored value or a missing counter (QK-0121);
//!   the store's typed rows cannot hold one, and a row that does not
//!   decode is an `IO` error.

use dcroxide_chainhash::Hash;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Hash32, VspFeeStatus, VspFeeStatusKey, VspHost, VspHostId, VspHosts, VspPubKeys, VspTicketRow,
    VspTickets, WriteTx,
};
use std::ops::Bound;

use crate::txstore::TxRead;

/// `VSPFeeProcessStarted`: the fee payment has started but is not paid.
pub const VSP_FEE_PROCESS_STARTED: u32 = 0;
/// `VSPFeeProcessPaid`: the fee is paid.
pub const VSP_FEE_PROCESS_PAID: u32 = 1;
/// `VSPFeeProcessErrored`: the fee payment failed and is to be made again.
pub const VSP_FEE_PROCESS_ERRORED: u32 = 2;
/// `VSPFeeProcessConfirmed`: the VSP confirmed the fee.
pub const VSP_FEE_PROCESS_CONFIRMED: u32 = 3;

/// A ticket's VSP record (Go's `udb.VSPTicket`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VspTicket {
    /// The fee transaction's hash; all zero when none is known.
    pub fee_hash: Hash32,
    /// The fee status (`VSP_FEE_PROCESS_*`; any value is stored).
    pub fee_tx_status: u32,
    /// The VSP host's id: 0 for no host.
    pub vsp_host_id: u32,
    /// The VSP host, as the caller named it (the VSP's URL); not stored
    /// in the ticket's row but resolved through the host tables.
    pub host: Vec<u8>,
    /// The public key recorded for the host on its first use.
    pub pub_key: Vec<u8>,
}

fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

fn not_exist(text: String) -> AnyError {
    e!(Kind::NotExist, text).into()
}

/// Go's `%v` of a `*[]byte`: `&[104 116 ...]`, the bytes in decimal.
fn go_bytes(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().saturating_mul(4).saturating_add(3));
    s.push_str("&[");
    for (i, byte) in b.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&byte.to_string());
    }
    s.push(']');
    s
}

/// `SetVSPTicket`: records `record`'s fee hash and status for `ticket`,
/// under the id of `record.host`, which is recorded with `record.pub_key`
/// if it is new. `record.vsp_host_id` is ignored. Returns the host id
/// stored (Go writes it into the record).
///
/// # Errors
///
/// An empty host (`invalid operation: key required`) or one longer than
/// 32,768 bytes (`invalid operation: key too large`), kind `Invalid`,
/// before anything is written; the store's.
pub fn set_vsp_ticket(
    tx: &mut WriteTx<'_>,
    ticket: &Hash32,
    record: &VspTicket,
) -> Result<u32, AnyError> {
    let host_id = tx
        .add_vsp_host(&record.host, &record.pub_key)
        .map_err(store_err)?;
    let row = VspTicketRow {
        fee_hash: record.fee_hash,
        fee_status: record.fee_tx_status,
        host_id,
    };
    tx.put_vsp_ticket(ticket, &row).map_err(store_err)?;
    Ok(host_id)
}

/// `DeleteVSPTicket`: removes `ticket`'s record, if any. The host stays.
///
/// # Errors
///
/// The store's.
pub fn delete_vsp_ticket(tx: &mut WriteTx<'_>, ticket: &Hash32) -> Result<(), AnyError> {
    tx.delete_vsp_ticket(ticket).map_err(store_err)?;
    Ok(())
}

/// `GetVSPTicket`: `ticket`'s record with its host and the host's public
/// key resolved (both empty for host id 0).
///
/// # Errors
///
/// No record: kind `NotExist`, `no VSP info for ticket <hash>` (the hash
/// in display order). The store's.
pub fn get_vsp_ticket(
    r: &mut (impl TxRead + ?Sized),
    ticket: &Hash32,
) -> Result<VspTicket, AnyError> {
    let row = r
        .get::<VspTickets>(ticket)
        .map_err(store_err)?
        .ok_or_else(|| not_exist(format!("no VSP info for ticket {}", Hash(ticket.0))))?;
    let mut t = ticket_of(&row);
    if row.host_id != 0
        && let Some(host) = r
            .get::<VspHosts>(&VspHostId(row.host_id))
            .map_err(store_err)?
        && !host.0.is_empty()
    {
        if let Some(p) = r.get::<VspPubKeys>(&host).map_err(store_err)? {
            t.pub_key = p.pubkey;
        }
        t.host = host.0;
    }
    Ok(t)
}

fn ticket_of(row: &VspTicketRow) -> VspTicket {
    VspTicket {
        fee_hash: row.fee_hash,
        fee_tx_status: row.fee_status,
        vsp_host_id: row.host_id,
        host: Vec::new(),
        pub_key: Vec::new(),
    }
}

/// `GetVSPTicketsByFeeStatus`: every ticket whose fee status is
/// `fee_status`, in ticket hash order, each with its host id but without
/// host or public key (as Go's). A status outside `u32` matches nothing.
///
/// # Errors
///
/// An index entry without its ticket row (a store whose record tables are
/// inconsistent, which [`purse_store::ReadTx::check_records`] reports):
/// kind `IO`. The store's.
pub fn get_vsp_tickets_by_fee_status(
    r: &mut (impl TxRead + ?Sized),
    fee_status: i64,
) -> Result<Vec<(Hash32, VspTicket)>, AnyError> {
    let Ok(status) = u32::try_from(fee_status) else {
        return Ok(Vec::new());
    };
    let lo = VspFeeStatusKey {
        status,
        ticket: Hash32([0; 32]),
    };
    let hi = VspFeeStatusKey {
        status,
        ticket: Hash32([0xff; 32]),
    };
    let keys = r
        .keys::<VspFeeStatus>(
            Bound::Included(&lo),
            Bound::Included(&hi),
            false,
            usize::MAX,
        )
        .map_err(store_err)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let row = r
            .get::<VspTickets>(&k.ticket)
            .map_err(store_err)?
            .ok_or_else(|| {
                AnyError::from(e!(
                    Kind::Io,
                    format!(
                        "VSP fee status index names ticket {} with no record",
                        Hash(k.ticket.0)
                    )
                ))
            })?;
        out.push((k.ticket, ticket_of(&row)));
    }
    Ok(out)
}

/// `GetVSPHost`: the host recorded under `id`; id 0 is the empty host
/// dcrwallet's upgrade 23 seeds.
///
/// # Errors
///
/// No such id: kind `NotExist`, `no VSP host info for id <id>` (Go prints
/// a pointer where this prints the id, QK-0123). The store's.
pub fn get_vsp_host(r: &mut (impl TxRead + ?Sized), id: u32) -> Result<Vec<u8>, AnyError> {
    if id == 0 {
        return Ok(Vec::new());
    }
    r.get::<VspHosts>(&VspHostId(id))
        .map_err(store_err)?
        .map(|h| h.0)
        .ok_or_else(|| not_exist(format!("no VSP host info for id {id}")))
}

/// `GetVSPPubKey`: the id and public key recorded for `host`.
///
/// # Errors
///
/// No such host (an empty host included): kind `NotExist`, `no VSP pubkey
/// info for host &[<bytes in decimal>]`, Go's text. The store's.
pub fn get_vsp_pub_key(
    r: &mut (impl TxRead + ?Sized),
    host: &[u8],
) -> Result<(u32, Vec<u8>), AnyError> {
    let row = if host.is_empty() || host.len() > purse_store::MAX_KEY_SIZE {
        None
    } else {
        r.get::<VspPubKeys>(&VspHost(host.to_vec()))
            .map_err(store_err)?
    };
    row.map(|p| (p.host_id, p.pubkey))
        .ok_or_else(|| not_exist(format!("no VSP pubkey info for host {}", go_bytes(host))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use purse_store::{OpenOptions, Store};

    fn store() -> Store {
        OpenOptions::new()
            .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
            .expect("store")
    }

    fn ticket(host: &str, status: u32) -> VspTicket {
        VspTicket {
            fee_hash: Hash32([9; 32]),
            fee_tx_status: status,
            vsp_host_id: 77,
            host: host.as_bytes().to_vec(),
            pub_key: vec![1, 2, 3],
        }
    }

    #[test]
    fn go_bytes_is_gos_pointer_to_slice_text() {
        assert_eq!(go_bytes(b""), "&[]");
        assert_eq!(go_bytes(b"ab"), "&[97 98]");
        assert_eq!(go_bytes(&[0, 255]), "&[0 255]");
    }

    #[test]
    fn hosts_get_ids_in_order_of_first_use_and_keep_their_first_key() {
        let s = store();
        s.update(|tx| {
            assert_eq!(
                set_vsp_ticket(tx, &Hash32([1; 32]), &ticket("a", 0)).expect("a"),
                1
            );
            assert_eq!(
                set_vsp_ticket(tx, &Hash32([2; 32]), &ticket("b", 1)).expect("b"),
                2
            );
            let mut again = ticket("a", 3);
            again.pub_key = vec![7];
            assert_eq!(set_vsp_ticket(tx, &Hash32([3; 32]), &again).expect("a"), 1);
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        let t = get_vsp_ticket(&mut r, &Hash32([3; 32])).expect("ticket");
        assert_eq!(t.host, b"a");
        assert_eq!(t.pub_key, vec![1, 2, 3], "the first key stays");
        assert_eq!(t.vsp_host_id, 1);
        assert_eq!(get_vsp_host(&mut r, 2).expect("host"), b"b");
        assert_eq!(get_vsp_host(&mut r, 0).expect("seeded"), b"");
        assert_eq!(
            get_vsp_host(&mut r, 3).expect_err("missing").to_string(),
            "item does not exist: no VSP host info for id 3"
        );
        assert_eq!(
            get_vsp_pub_key(&mut r, b"b").expect("b"),
            (2, vec![1, 2, 3])
        );
        assert_eq!(
            get_vsp_pub_key(&mut r, b"").expect_err("empty").to_string(),
            "item does not exist: no VSP pubkey info for host &[]"
        );
        r.check_records().expect("consistent");
    }

    #[test]
    fn refused_hosts_write_nothing() {
        let s = store();
        for (host, text) in [
            (Vec::new(), "invalid operation: key required"),
            (vec![b'h'; 32_769], "invalid operation: key too large"),
        ] {
            // The refusal leaves the transaction usable: it commits.
            s.update(|tx| {
                let mut t = ticket("", 0);
                t.host.clone_from(&host);
                let err = set_vsp_ticket(tx, &Hash32([1; 32]), &t).expect_err("refused");
                assert!(err.is_kind(Kind::Invalid));
                assert_eq!(err.to_string(), text);
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
            let tx = s.read().expect("read");
            for t in purse_store::RECORD_TABLES {
                assert_eq!(tx.len(t).expect("len"), 0, "{t:?}");
            }
        }
    }

    #[test]
    fn listings_follow_the_index_and_hold_no_host() {
        let s = store();
        s.update(|tx| {
            set_vsp_ticket(tx, &Hash32([5; 32]), &ticket("a", 2)).expect("5");
            set_vsp_ticket(tx, &Hash32([4; 32]), &ticket("a", 2)).expect("4");
            set_vsp_ticket(tx, &Hash32([6; 32]), &ticket("a", 1)).expect("6");
            // A ticket moved to another status leaves its old one.
            set_vsp_ticket(tx, &Hash32([6; 32]), &ticket("a", 2)).expect("6 again");
            delete_vsp_ticket(tx, &Hash32([5; 32])).expect("delete");
            delete_vsp_ticket(tx, &Hash32([5; 32])).expect("delete again");
            // The two ends of a status's range in the index (both listed),
            // next to tickets of the neighbouring statuses (left out).
            set_vsp_ticket(tx, &Hash32([0; 32]), &ticket("a", 2)).expect("00");
            set_vsp_ticket(tx, &Hash32([0xff; 32]), &ticket("a", 2)).expect("ff");
            set_vsp_ticket(tx, &Hash32([0xfe; 32]), &ticket("a", 1)).expect("fe");
            set_vsp_ticket(tx, &Hash32([1; 32]), &ticket("a", 3)).expect("01");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        let got = get_vsp_tickets_by_fee_status(&mut r, 2).expect("status 2");
        assert_eq!(
            got.iter().map(|(h, _)| h.0[0]).collect::<Vec<_>>(),
            vec![0, 4, 6, 0xff]
        );
        assert!(
            got.iter()
                .all(|(_, t)| t.host.is_empty() && t.pub_key.is_empty())
        );
        assert!(got.iter().all(|(_, t)| t.vsp_host_id == 1));
        // Ticket 6 left status 1 when it moved.
        for (st, only) in [(1, 0xfe), (3, 1)] {
            let got = get_vsp_tickets_by_fee_status(&mut r, st).expect("neighbour");
            assert_eq!(
                got.iter().map(|(h, _)| h.0[0]).collect::<Vec<_>>(),
                vec![only]
            );
        }
        for st in [-1, 1 << 32] {
            assert!(
                get_vsp_tickets_by_fee_status(&mut r, st)
                    .expect("none")
                    .is_empty()
            );
        }
        assert_eq!(
            get_vsp_ticket(&mut r, &Hash32([5; 32]))
                .expect_err("deleted")
                .to_string(),
            format!(
                "item does not exist: no VSP info for ticket {}",
                Hash([5; 32])
            )
        );
    }
}
