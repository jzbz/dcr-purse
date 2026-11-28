// Copyright (c) 2017-2018 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's agenda vote preferences (`wallet/udb/agendas.go`; spec 05
//! section 3.5, ADR-0001 section 6.20) over `purse-store`'s `agenda_prefs`
//! and `ticket_agenda_prefs` tables.
//!
//! A preference is a choice id for an agenda id of a deployment version,
//! wallet-wide or for one ticket. dcrwallet keys it by the version
//! (big-endian) and the agenda id, in its `agendaprefs` bucket or in the
//! ticket's nested bucket of `ticketsagendaprefs`; `ticket_agenda_prefs`
//! flattens the nested buckets, the ticket hash leading the key. Ids are
//! bytes, as Go's strings are (ADR-0012 Rule 4: no narrowing to UTF-8).
//! Every function takes the caller's transaction.
//!
//! # As Go behaves
//!
//! - The setters store the choice as given, an empty one included, and
//!   wrap every failure in `IO` (`I/O error:: invalid operation: key too
//!   large` for a version ‖ agenda key over bbolt's 32,768 bytes, the one
//!   failure their input can cause). For a ticket the limit is applied to
//!   dcrwallet's key inside the ticket's bucket, not to the flattened key
//!   ([`WriteTx::put_ticket_agenda_pref`]).
//! - The readers answer the stored choice, empty when none is stored, so
//!   an empty choice and no choice read alike.
//!
//! # Divergences
//!
//! - Go's readers return the choice alone; reading the store can fail, so
//!   these return a `Result` (ADR-0012 Rule 2(c)).
//! - `SetTicketAgendaPreference` creates the ticket's nested bucket before
//!   its put, so a put Go refuses can leave an empty bucket inside a
//!   transaction its caller keeps (an `update` group that tolerates the
//!   error); an empty nested bucket holds no preference and the logical
//!   export does not show it (ADR-0001 section 9, N7), and the flattened
//!   table has no bucket to leave.

use purse_errors::{AnyError, Kind, e};
use purse_store::{
    AgendaChoice, AgendaKey, AgendaPrefs, Hash32, TicketAgendaKey, TicketAgendaPrefs, WriteTx,
};

use crate::txstore::TxRead;

fn io(err: purse_store::Error) -> AnyError {
    let inner: AnyError = err.to_purse_error().into();
    e!(Kind::Io, inner).into()
}

fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

/// `SetDefaultAgendaPreference`: the wallet's choice `choice_id` for agenda
/// `agenda_id` of deployment `version`.
///
/// # Errors
///
/// A version ‖ agenda key over 32,768 bytes, or the store's, kind `IO`
/// around the store's error.
pub fn set_default_agenda_preference(
    tx: &mut WriteTx<'_>,
    version: u32,
    agenda_id: &[u8],
    choice_id: &[u8],
) -> Result<(), AnyError> {
    let key = AgendaKey {
        version,
        agenda: agenda_id.to_vec(),
    };
    tx.put::<AgendaPrefs>(&key, &AgendaChoice(choice_id.to_vec()))
        .map_err(io)
}

/// `SetTicketAgendaPreference`: `ticket`'s choice `choice_id` for agenda
/// `agenda_id` of deployment `version`.
///
/// # Errors
///
/// A version ‖ agenda key over 32,768 bytes, or the store's, kind `IO`
/// around the store's error.
pub fn set_ticket_agenda_preference(
    tx: &mut WriteTx<'_>,
    ticket: &Hash32,
    version: u32,
    agenda_id: &[u8],
    choice_id: &[u8],
) -> Result<(), AnyError> {
    let key = TicketAgendaKey {
        ticket: *ticket,
        version,
        agenda: agenda_id.to_vec(),
    };
    tx.put_ticket_agenda_pref(&key, &AgendaChoice(choice_id.to_vec()))
        .map_err(io)
}

/// `DefaultAgendaPreference`: the wallet's choice for agenda `agenda_id`
/// of deployment `version`, empty when none is stored.
///
/// # Errors
///
/// The store's.
pub fn default_agenda_preference(
    r: &mut (impl TxRead + ?Sized),
    version: u32,
    agenda_id: &[u8],
) -> Result<Vec<u8>, AnyError> {
    let key = AgendaKey {
        version,
        agenda: agenda_id.to_vec(),
    };
    Ok(r.get::<AgendaPrefs>(&key)
        .map_err(store_err)?
        .map_or_else(Vec::new, |c| c.0))
}

/// `TicketAgendaPreference`: `ticket`'s choice for agenda `agenda_id` of
/// deployment `version`, empty when none is stored.
///
/// # Errors
///
/// The store's.
pub fn ticket_agenda_preference(
    r: &mut (impl TxRead + ?Sized),
    ticket: &Hash32,
    version: u32,
    agenda_id: &[u8],
) -> Result<Vec<u8>, AnyError> {
    let key = TicketAgendaKey {
        ticket: *ticket,
        version,
        agenda: agenda_id.to_vec(),
    };
    Ok(r.get::<TicketAgendaPrefs>(&key)
        .map_err(store_err)?
        .map_or_else(Vec::new, |c| c.0))
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

    #[test]
    fn choices_are_kept_per_version_agenda_and_ticket() {
        let s = store();
        let t = Hash32([3; 32]);
        s.update(|tx| {
            set_default_agenda_preference(tx, 10, b"treasury", b"yes").expect("default");
            set_default_agenda_preference(tx, 11, b"treasury", b"").expect("empty choice");
            set_ticket_agenda_preference(tx, &t, 10, b"treasury", b"no").expect("ticket");
            set_ticket_agenda_preference(tx, &t, 10, b"", b"abstain").expect("empty agenda");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        assert_eq!(
            default_agenda_preference(&mut r, 10, b"treasury").expect("d"),
            b"yes"
        );
        assert_eq!(
            default_agenda_preference(&mut r, 11, b"treasury").expect("d"),
            b""
        );
        assert_eq!(
            default_agenda_preference(&mut r, 12, b"treasury").expect("d"),
            b""
        );
        assert_eq!(
            ticket_agenda_preference(&mut r, &t, 10, b"treasury").expect("t"),
            b"no"
        );
        assert_eq!(
            ticket_agenda_preference(&mut r, &t, 10, b"").expect("t"),
            b"abstain"
        );
        assert_eq!(
            ticket_agenda_preference(&mut r, &Hash32([4; 32]), 10, b"treasury").expect("t"),
            b""
        );
        assert_eq!(r.len(purse_store::TableId::AgendaPrefs).expect("len"), 2);
    }

    #[test]
    fn the_key_limit_is_gos_for_both_kinds() {
        let s = store();
        let t = Hash32([3; 32]);
        let at_limit = vec![b'a'; 32_764];
        let over = vec![b'a'; 32_765];
        s.update(|tx| {
            for err in [
                set_default_agenda_preference(tx, 1, &over, b"yes").expect_err("over"),
                set_ticket_agenda_preference(tx, &t, 1, &over, b"yes").expect_err("over"),
            ] {
                assert!(err.is_kind(Kind::Io));
                assert_eq!(
                    err.render(purse_errors::DAEMON_SEPARATOR),
                    "I/O error:: invalid operation: key too large"
                );
            }
            set_default_agenda_preference(tx, 1, &at_limit, b"yes").expect("at the limit");
            // The flattened key is 32,800 bytes: over the generic limit,
            // but Go's key is at it.
            set_ticket_agenda_preference(tx, &t, 1, &at_limit, b"no").expect("at the limit");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        assert_eq!(
            ticket_agenda_preference(&mut r, &t, 1, &at_limit).expect("t"),
            b"no"
        );
        assert_eq!(r.len(purse_store::TableId::AgendaPrefs).expect("len"), 1);
        assert_eq!(
            r.len(purse_store::TableId::TicketAgendaPrefs).expect("len"),
            1
        );
    }
}
