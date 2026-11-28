// Copyright (c) 2017-2024 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `Initialize` and `InitializeWatchOnly`
//! (`wallet/udb/initialize.go`): a new wallet's address manager and
//! transaction store, created in one write transaction of the store.
//!
//! Go's two functions create the `waddrmgr` and `wtxmgr` top-level buckets,
//! run `createAddressManager` (or `createWatchOnly`) and then
//! `createStore` in one `walletdb.Update`, write the initial database
//! version (1), commit, and run the 25 upgrades in one later write
//! transaction (`Upgrade`, `wallet/udb/upgrades.go`). dcr-purse writes the
//! upgraded state directly ([`Manager::create`],
//! [`Manager::create_watch_only`], [`TxStore::create`]; ADR-0013), so the
//! whole creation is the one transaction here, and the store holds either
//! the complete new wallet or what it held before. Which one is known from
//! the error, except after a commit whose outcome the store cannot know
//! ([`CreationStoreError::commit_outcome_unknown`]), when the store must be
//! reopened and read to find out.
//!
//! # As Go behaves
//!
//! - An existing wallet is refused first, before the arguments are looked
//!   at and before anything is drawn, with the error of Go's first
//!   `CreateTopLevelBucket`: kind `IO` wrapping bbolt's `Exist` error,
//!   rendered `I/O error:: item already exists: bucket already exists`
//!   with the daemon's separator. Go refuses whenever the `waddrmgr` or the
//!   `wtxmgr` bucket exists; here, whenever the store holds any row of the
//!   address manager (the `wallet` or `accounts` table) or a main chain tip
//!   of the transaction store, which every creation writes. The address
//!   manager's own `address manager already exists`, which Go's
//!   `createAddressManager` and `createWatchOnly` check, is unreachable
//!   through `Initialize` in Go and here.
//! - Then the address manager's errors, in Go's order and with Go's texts
//!   ([`Manager::create`], [`Manager::create_watch_only`]), and only then
//!   the transaction store's. Randomness is drawn by the address manager
//!   alone, in Go's order and sizes; the transaction store draws nothing.
//!
//! # Not as Go behaves
//!
//! - Go commits its transaction before the upgrades run, so an upgrade
//!   that fails rolls back all 25 and leaves Go's database created at
//!   version 1. There are no upgrades here and no such state.
//! - A store error is returned with the store's kind and text, as Go's
//!   `walletdb.Update` returns its driver's error, but its leaf is a
//!   [`CreationStoreError`] holding the store's typed error rather than a
//!   text, so that a commit whose outcome is unknown can be told apart
//!   ([`creation_outcome_unknown`]); the rendered error is the same.
//! - An extended private key passed to [`initialize_watch_only`] is refused
//!   (QK-0762, decided by the project owner on 2026-11-22), as
//!   [`Manager::create_watch_only`] refuses it.

use std::fmt;

use purse_errors::{AnyError, Kind, Op, e};
use purse_snacl::{Limits, RandomSource};
use purse_store::{Store, TableId, WriteTx};

use crate::addrmgr::{ChainParams, Manager};
use crate::txstore::TxStore;

/// The text of bbolt's `ErrBucketExists`, which Go's `CreateTopLevelBucket`
/// returns for a database `Initialize` has already created.
pub const BUCKET_EXISTS_TEXT: &str = "bucket already exists";

/// The store's error from a creation's write transaction, kept whole as
/// the leaf of the error [`initialize`] and [`initialize_watch_only`]
/// return (find it with [`AnyError::find_foreign`], or ask
/// [`creation_outcome_unknown`]).
///
/// It renders as the store's cause alone, after the kind the enclosing
/// level writes, so the returned error renders exactly as
/// `purse_store::Error::to_purse_error` would render it.
#[derive(Debug)]
pub struct CreationStoreError {
    error: purse_store::Error,
    cause: String,
}

impl CreationStoreError {
    fn new(error: purse_store::Error) -> CreationStoreError {
        // `to_purse_error` holds the store's cause as a text leaf under
        // the kind: that text, and nothing else, is this leaf's.
        let cause = error
            .to_purse_error()
            .wrapped()
            .map(|leaf| leaf.to_string())
            .unwrap_or_default();
        CreationStoreError { error, cause }
    }

    /// The store's error.
    pub fn store_error(&self) -> &purse_store::Error {
        &self.error
    }

    /// Whether the creation's commit failed after its writes may have
    /// reached storage, so that the store may hold the whole new wallet
    /// (`purse_store::Error::commit_outcome_unknown`). The store then
    /// refuses every further write transaction until it is reopened.
    pub fn commit_outcome_unknown(&self) -> bool {
        self.error.commit_outcome_unknown()
    }

    /// The error [`initialize`] returns for it: the store's kind, no op,
    /// and this leaf.
    fn into_any(self) -> AnyError {
        let kind = self.error.kind();
        purse_errors::Error::from_parts(Op::default(), kind, Some(AnyError::foreign(self))).into()
    }
}

impl fmt::Display for CreationStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.cause)
    }
}

impl std::error::Error for CreationStoreError {}

/// Whether `err`, an error of [`initialize`] or [`initialize_watch_only`],
/// is a store error after which the creation may have been committed in
/// full ([`CreationStoreError::commit_outcome_unknown`]): the caller must
/// reopen the store and read whether the wallet is there. False for every
/// other error, after which the store holds what it held before.
pub fn creation_outcome_unknown(err: &AnyError) -> bool {
    err.find_foreign::<CreationStoreError>()
        .is_some_and(CreationStoreError::commit_outcome_unknown)
}

/// The error of the creation's write transaction.
enum InitErr {
    Store(purse_store::Error),
    Udb(AnyError),
}

impl From<purse_store::Error> for InitErr {
    fn from(e: purse_store::Error) -> Self {
        InitErr::Store(e)
    }
}

/// Go's `Initialize`: creates a wallet from `seed` in `store`, the address
/// manager ([`Manager::create`]) and the transaction store
/// ([`TxStore::create`]) in one write transaction, committed durably.
///
/// `rng` is drawn from as [`Manager::create`] draws; `limits` bounds the
/// network's scrypt cost. Open the wallet afterwards with
/// [`Manager::open`] and [`TxStore::new`], as Go's caller opens it with
/// `udb.Open`.
///
/// # Errors
///
/// In Go's order: a store already holding a wallet, kind `IO` wrapping
/// `Exist` [`BUCKET_EXISTS_TEXT`]; then [`Manager::create`]'s (an empty
/// private passphrase, a seed of the wrong length or an unusable one,
/// snacl's for a failing random source or a scrypt cost over `limits`);
/// then the transaction store's; the store's, the commit's included, each
/// with the store's kind and text and a [`CreationStoreError`] as its
/// leaf. On every error nothing is written, except that a commit whose
/// outcome the store cannot know may have been made in full: then
/// [`creation_outcome_unknown`] is true of the error, the store refuses
/// every further write transaction, and the caller must close and reopen
/// it and read whether the wallet is there (it finds either no wallet or
/// the whole of it) before creating again.
pub fn initialize<R: RandomSource + ?Sized>(
    store: &Store,
    chain: &ChainParams,
    seed: &[u8],
    pub_pass: &[u8],
    priv_pass: &[u8],
    rng: &mut R,
    limits: &Limits,
) -> Result<(), AnyError> {
    create(store, chain, |tx| {
        Manager::create(tx, chain, seed, pub_pass, priv_pass, rng, limits)
    })
}

/// Go's `InitializeWatchOnly`: creates a watching-only wallet in `store`
/// from `hd_pub_key`, an account's extended public key as a string (passed
/// as bytes, unparsed, as Go passes its string), the address manager
/// ([`Manager::create_watch_only`]) and the transaction store
/// ([`TxStore::create`]) in one write transaction, committed durably.
///
/// # Errors
///
/// As [`initialize`], with [`Manager::create_watch_only`]'s errors in place
/// of [`Manager::create`]'s: a key that does not parse (hdkeychain's error,
/// unwrapped), dcr-purse's refusal of an extended private key (`Invalid`
/// `extended key must be an xpub`, QK-0762), an unusable branch child,
/// snacl's.
pub fn initialize_watch_only<R: RandomSource + ?Sized>(
    store: &Store,
    chain: &ChainParams,
    hd_pub_key: &[u8],
    pub_pass: &[u8],
    rng: &mut R,
    limits: &Limits,
) -> Result<(), AnyError> {
    create(store, chain, |tx| {
        Manager::create_watch_only(tx, chain, hd_pub_key, pub_pass, rng, limits)
    })
}

/// The body both creations share: Go's bucket check, the address manager
/// part `manager`, then the transaction store, in one transaction.
fn create(
    store: &Store,
    chain: &ChainParams,
    manager: impl FnOnce(&mut WriteTx<'_>) -> Result<(), AnyError>,
) -> Result<(), AnyError> {
    let created = store.update(|tx| -> Result<(), InitErr> {
        if wallet_exists(tx)? {
            return Err(InitErr::Udb(
                e!(Kind::Io, e!(Kind::Exist, BUCKET_EXISTS_TEXT)).into(),
            ));
        }
        manager(tx).map_err(InitErr::Udb)?;
        // Built after the address manager's part, so that its one possible
        // error (a network other than the four, which
        // `ChainParams::by_name` never gives) cannot come before the
        // manager's.
        let txstore = TxStore::new(chain).map_err(InitErr::Udb)?;
        txstore.create(tx).map_err(InitErr::Udb)
    });
    match created {
        Ok(()) => Ok(()),
        Err(InitErr::Udb(err)) => Err(err),
        Err(InitErr::Store(err)) => Err(CreationStoreError::new(err).into_any()),
    }
}

/// Whether `tx` already holds a wallet: Go's `waddrmgr` bucket (any row of
/// the address manager) or its `wtxmgr` bucket (a main chain tip, which
/// [`TxStore::create`] writes and nothing removes).
fn wallet_exists(tx: &mut WriteTx<'_>) -> purse_store::Result<bool> {
    Ok(!tx.is_empty(TableId::Wallet)? || !tx.is_empty(TableId::Accounts)? || tx.tip()?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use purse_snacl::{OsRandom, RandomSourceError};

    fn fresh() -> Store {
        purse_store::OpenOptions::new()
            .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
            .expect("store")
    }

    /// The text and kind Go's `Initialize` gives for a database it created
    /// (pinned on the Go side by the oracle's
    /// `TestInitializeOfACreatedDatabase`), for either creation, before
    /// anything is drawn.
    #[test]
    fn an_existing_wallet_is_refused_with_gos_text() {
        let chain = ChainParams::by_name("simnet").expect("simnet");
        let store = fresh();
        initialize(
            &store,
            &chain,
            &[1; 32],
            b"public",
            b"private",
            &mut OsRandom,
            &Limits::DEFAULT,
        )
        .expect("create");
        let mut drawn = 0usize;
        let mut counting = |buf: &mut [u8]| -> Result<(), RandomSourceError> {
            drawn = drawn.saturating_add(buf.len());
            buf.fill(0);
            Ok(())
        };
        // Arguments the address manager would refuse are not looked at.
        for err in [
            initialize(
                &store,
                &chain,
                &[1; 3],
                b"public",
                b"",
                &mut counting,
                &Limits::DEFAULT,
            ),
            initialize_watch_only(
                &store,
                &chain,
                b"xyz",
                b"public",
                &mut counting,
                &Limits::DEFAULT,
            ),
        ] {
            let err = err.expect_err("refused");
            assert!(err.is_kind(Kind::Io), "{err}");
            assert_eq!(
                err.render(purse_errors::DAEMON_SEPARATOR),
                "I/O error:: item already exists: bucket already exists"
            );
        }
        assert_eq!(drawn, 0);
    }
}
