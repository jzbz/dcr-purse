// Copyright (c) 2020-2021 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's treasury vote policies (`wallet/udb/treasury.go`; spec 05
//! sections 3.4, 4.4 and 5.8) over `purse-store`'s `tspend_policies`,
//! `treasury_key_policies`, `vsp_tspend_policies` and
//! `vsp_treasury_key_policies` tables.
//!
//! A policy says how to vote on treasury spends: by the spend's hash or by
//! the Politeia ("pi") key that signed it, for the wallet's own tickets or
//! for one VSP customer's ticket. A policy is one byte, Go's
//! `stake.TreasuryVoteT` (1 yes, 2 no); 0 (`TreasuryVoteInvalid`,
//! abstain) is no policy, kept as the row's absence, so a setter given 0
//! deletes. Setters store any other byte, as Go's do (QK-0127; spec 05
//! open question 14): only the wallet layer limits a policy to 1 or 2
//! (`wallet/wallet.go:669-674`). Every function takes the caller's
//! transaction.
//!
//! # As Go behaves
//!
//! - A delete never fails on a missing key, an empty one included; a put
//!   of an empty pi key fails with bbolt's `invalid operation: key
//!   required`, and of a key longer than 32,768 bytes (a pi key, or a
//!   ticket and pi key together) with `invalid operation: key too large`,
//!   kind `Invalid`, returned as the store gives it, as Go returns its
//!   put's error unwrapped; a delete's error is wrapped in `IO`, as Go
//!   wraps it (QK-0127; spec 05 Q33).
//! - The single readers answer 0 for no policy.
//! - The listings answer every policy. Go returns maps, in random order;
//!   these answer in key order (hashes in internal byte order, pi keys
//!   bytewise; ADR-0012 Rule 3(a)), read from the table, where Go looks
//!   every key up a second time (spec 05 Q32), which changes nothing for
//!   keys its setters write.
//!
//! # Divergences
//!
//! - Go's single readers fail with `IO` `invalid length N for tspend vote
//!   policy` (or `treasury key policy`) for a stored value that is not one
//!   byte, and its listings skip a stored 0. For a key of the wrong length
//!   (spec 05 section 5.8), `TSpendPolicies` and, for a key of 32 bytes or
//!   more, `VSPTSpendPolicies` look the policy up again under the key
//!   zero-padded or truncated to its length, so the row is usually left
//!   out (it is listed under the rebuilt key when that key holds a
//!   policy); a per-ticket key shorter than 32 bytes panics, in
//!   `VSPTSpendPolicies` on `k[32:]` and in `VSPTreasuryKeyPolicies` on
//!   `k[chainhash.HashSize:]` (wallet/udb/treasury.go:134, :249). No
//!   setter at the pin writes such a value or key, and neither the store's
//!   typed keys nor its value type ([`purse_store::TreasuryVote`], exactly
//!   one non-zero byte) can hold one, so a row that does not decode is an
//!   `IO` error of the store's.
//! - Go's listings return the map filled so far together with an error;
//!   these return the error alone.

use purse_errors::{AnyError, Kind, e};
use purse_store::{
    Hash32, PiKey, TSpendPolicies, Table, TreasuryKeyPolicies, TreasuryVote, VspTSpendKey,
    VspTSpendPolicies, VspTreasuryKey, VspTreasuryKeyPolicies, WriteTx,
};
use std::ops::Bound;

use crate::txstore::TxRead;

/// `stake.TreasuryVoteInvalid`: no policy (abstain).
pub const TREASURY_VOTE_INVALID: u8 = 0;
/// `stake.TreasuryVoteYes`.
pub const TREASURY_VOTE_YES: u8 = 1;
/// `stake.TreasuryVoteNo`.
pub const TREASURY_VOTE_NO: u8 = 2;

fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

/// A setter's body: 0 deletes `key` (an error wrapped in `IO`), any other
/// byte is written (the store's error as it is).
fn set<T: Table<Value = TreasuryVote>>(
    tx: &mut WriteTx<'_>,
    key: &T::Key,
    policy: u8,
) -> Result<(), AnyError> {
    match TreasuryVote::new(policy) {
        None => tx
            .delete::<T>(key)
            .map(drop)
            .map_err(|err| e!(Kind::Io, store_err(err)).into()),
        Some(v) => tx.put::<T>(key, &v).map_err(store_err),
    }
}

/// A single reader's body: the policy at `key`, 0 for none.
fn get<T: Table<Value = TreasuryVote>>(
    r: &mut (impl TxRead + ?Sized),
    key: &T::Key,
) -> Result<u8, AnyError> {
    Ok(r.get::<T>(key)
        .map_err(store_err)?
        .map_or(TREASURY_VOTE_INVALID, TreasuryVote::code))
}

/// A listing's body: every row of `T`, in key order.
fn all<T: Table<Value = TreasuryVote>>(
    r: &mut (impl TxRead + ?Sized),
) -> Result<Vec<(T::Key, u8)>, AnyError> {
    Ok(
        r.rows::<T>(Bound::Unbounded, Bound::Unbounded, false, usize::MAX)
            .map_err(store_err)?
            .into_iter()
            .map(|(k, v)| (k, v.code()))
            .collect(),
    )
}

/// `SetTSpendPolicy`: the policy for the treasury spend `tspend`; 0
/// removes it.
///
/// # Errors
///
/// The store's (kind `IO` around a delete's).
pub fn set_tspend_policy(
    tx: &mut WriteTx<'_>,
    tspend: &Hash32,
    policy: u8,
) -> Result<(), AnyError> {
    set::<TSpendPolicies>(tx, tspend, policy)
}

/// `SetVSPTSpendPolicy`: the policy of VSP customer ticket `ticket` for
/// the treasury spend `tspend`; 0 removes it.
///
/// # Errors
///
/// The store's (kind `IO` around a delete's).
pub fn set_vsp_tspend_policy(
    tx: &mut WriteTx<'_>,
    ticket: &Hash32,
    tspend: &Hash32,
    policy: u8,
) -> Result<(), AnyError> {
    let key = VspTSpendKey {
        ticket: *ticket,
        tspend: *tspend,
    };
    set::<VspTSpendPolicies>(tx, &key, policy)
}

/// `TSpendPolicy`: the policy for `tspend`, 0 for none.
///
/// # Errors
///
/// The store's.
pub fn tspend_policy(r: &mut (impl TxRead + ?Sized), tspend: &Hash32) -> Result<u8, AnyError> {
    get::<TSpendPolicies>(r, tspend)
}

/// `VSPTSpendPolicy`: the policy of `ticket` for `tspend`, 0 for none.
///
/// # Errors
///
/// The store's.
pub fn vsp_tspend_policy(
    r: &mut (impl TxRead + ?Sized),
    ticket: &Hash32,
    tspend: &Hash32,
) -> Result<u8, AnyError> {
    let key = VspTSpendKey {
        ticket: *ticket,
        tspend: *tspend,
    };
    get::<VspTSpendPolicies>(r, &key)
}

/// `TSpendPolicies`: every tspend policy, by tspend hash (internal byte
/// order).
///
/// # Errors
///
/// The store's.
pub fn tspend_policies(r: &mut (impl TxRead + ?Sized)) -> Result<Vec<(Hash32, u8)>, AnyError> {
    all::<TSpendPolicies>(r)
}

/// `VSPTSpendPolicies`: every per-ticket tspend policy, by ticket, then
/// tspend hash (internal byte order).
///
/// # Errors
///
/// The store's.
pub fn vsp_tspend_policies(
    r: &mut (impl TxRead + ?Sized),
) -> Result<Vec<(VspTSpendKey, u8)>, AnyError> {
    all::<VspTSpendPolicies>(r)
}

/// `SetTreasuryKeyPolicy`: the policy for spends signed by pi key `pikey`
/// (its bytes as given); 0 removes it.
///
/// # Errors
///
/// An empty pi key (`invalid operation: key required`) or one longer than
/// 32,768 bytes (`invalid operation: key too large`), kind `Invalid`,
/// unless the policy is 0. The store's (kind `IO` around a delete's).
pub fn set_treasury_key_policy(
    tx: &mut WriteTx<'_>,
    pikey: &[u8],
    policy: u8,
) -> Result<(), AnyError> {
    set::<TreasuryKeyPolicies>(tx, &PiKey(pikey.to_vec()), policy)
}

/// `SetVSPTreasuryKeyPolicy`: the policy of `ticket` for spends signed by
/// `pikey`; 0 removes it.
///
/// # Errors
///
/// A pi key longer than 32,736 bytes (the key, ticket and pi key, is then
/// over bbolt's limit: `invalid operation: key too large`, kind
/// `Invalid`), unless the policy is 0. The store's (kind `IO` around a
/// delete's).
pub fn set_vsp_treasury_key_policy(
    tx: &mut WriteTx<'_>,
    ticket: &Hash32,
    pikey: &[u8],
    policy: u8,
) -> Result<(), AnyError> {
    let key = VspTreasuryKey {
        ticket: *ticket,
        pikey: pikey.to_vec(),
    };
    set::<VspTreasuryKeyPolicies>(tx, &key, policy)
}

/// `TreasuryKeyPolicy`: the policy for `pikey`, 0 for none.
///
/// # Errors
///
/// The store's.
pub fn treasury_key_policy(r: &mut (impl TxRead + ?Sized), pikey: &[u8]) -> Result<u8, AnyError> {
    get::<TreasuryKeyPolicies>(r, &PiKey(pikey.to_vec()))
}

/// `VSPTreasuryKeyPolicy`: the policy of `ticket` for `pikey`, 0 for
/// none.
///
/// # Errors
///
/// The store's.
pub fn vsp_treasury_key_policy(
    r: &mut (impl TxRead + ?Sized),
    ticket: &Hash32,
    pikey: &[u8],
) -> Result<u8, AnyError> {
    let key = VspTreasuryKey {
        ticket: *ticket,
        pikey: pikey.to_vec(),
    };
    get::<VspTreasuryKeyPolicies>(r, &key)
}

/// `TreasuryKeyPolicies`: every pi key policy, by pi key bytes.
///
/// # Errors
///
/// The store's.
pub fn treasury_key_policies(
    r: &mut (impl TxRead + ?Sized),
) -> Result<Vec<(Vec<u8>, u8)>, AnyError> {
    Ok(all::<TreasuryKeyPolicies>(r)?
        .into_iter()
        .map(|(k, v)| (k.0, v))
        .collect())
}

/// `VSPTreasuryKeyPolicies`: every per-ticket pi key policy, by ticket
/// (internal byte order), then pi key bytes.
///
/// # Errors
///
/// The store's.
pub fn vsp_treasury_key_policies(
    r: &mut (impl TxRead + ?Sized),
) -> Result<Vec<(VspTreasuryKey, u8)>, AnyError> {
    all::<VspTreasuryKeyPolicies>(r)
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
    fn zero_deletes_and_any_other_byte_is_kept() {
        let s = store();
        let (t, h) = (Hash32([1; 32]), Hash32([2; 32]));
        s.update(|tx| {
            set_tspend_policy(tx, &h, 1).expect("yes");
            set_vsp_tspend_policy(tx, &t, &h, 255).expect("255");
            set_treasury_key_policy(tx, &[2; 33], 2).expect("no");
            set_vsp_treasury_key_policy(tx, &t, b"", 3).expect("ticket alone");
            set_tspend_policy(tx, &t, 1).expect("second");
            set_tspend_policy(tx, &t, 0).expect("deleted");
            set_tspend_policy(tx, &t, 0).expect("deleted again");
            set_treasury_key_policy(tx, b"", 0).expect("an empty key deletes");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        assert_eq!(tspend_policy(&mut r, &h).expect("h"), 1);
        assert_eq!(tspend_policy(&mut r, &t).expect("t"), 0);
        assert_eq!(vsp_tspend_policy(&mut r, &t, &h).expect("vsp"), 255);
        assert_eq!(vsp_tspend_policy(&mut r, &h, &t).expect("swapped"), 0);
        assert_eq!(treasury_key_policy(&mut r, &[2; 33]).expect("pi"), 2);
        assert_eq!(
            vsp_treasury_key_policy(&mut r, &t, b"").expect("empty pi"),
            3
        );
        assert_eq!(tspend_policies(&mut r).expect("all"), vec![(h, 1)]);
        assert_eq!(
            treasury_key_policies(&mut r).expect("all"),
            vec![(vec![2; 33], 2)]
        );
        assert_eq!(vsp_treasury_key_policies(&mut r).expect("all").len(), 1);
        assert_eq!(vsp_tspend_policies(&mut r).expect("all").len(), 1);
    }

    #[test]
    fn refused_keys_give_bbolts_texts_and_write_nothing() {
        let s = store();
        s.update(|tx| {
            for (err, text) in [
                (
                    set_treasury_key_policy(tx, b"", 1).expect_err("empty"),
                    "invalid operation: key required",
                ),
                (
                    set_treasury_key_policy(tx, &[1; 32_769], 1).expect_err("long"),
                    "invalid operation: key too large",
                ),
                (
                    set_vsp_treasury_key_policy(tx, &Hash32([1; 32]), &[1; 32_737], 1)
                        .expect_err("long"),
                    "invalid operation: key too large",
                ),
            ] {
                assert!(err.is_kind(Kind::Invalid));
                assert_eq!(err.to_string(), text);
            }
            set_vsp_treasury_key_policy(tx, &Hash32([1; 32]), &[1; 32_736], 1)
                .expect("at the limit");
            set_treasury_key_policy(tx, &[1; 32_769], 0).expect("a long key deletes");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
        let mut r = s.read().expect("read");
        assert!(treasury_key_policies(&mut r).expect("none").is_empty());
        assert_eq!(vsp_treasury_key_policies(&mut r).expect("one").len(), 1);
    }
}
