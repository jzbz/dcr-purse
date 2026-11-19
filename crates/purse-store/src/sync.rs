// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Relaxed durability, confined by the API to initial sync's
//! header-and-filter batches and the rescan marker.
//!
//! The owner's decision (ADR-0003 Open point 4, ADR-0014 Open point 2,
//! brief §4 principle 6): during initial sync, header-and-filter batches
//! and the rescan marker may commit with `Durability::None`, provided a
//! durable commit follows at least every 50 batches and at the end of the
//! sync, and every write whose effect a client or a peer can see is
//! durable. A crash then loses at most the batches since the last durable
//! commit, which the sync fetches again; each commit stays atomic, so the
//! store reopens at a transaction boundary.
//!
//! The rule is held by the types and by the store:
//!
//! - a [`SyncBatch`] writes only `headers`, `cfilters`, `blocks`,
//!   `block_by_hash` and the tip and processed-marker rows of `meta`, and
//!   in `blocks` only rows that carry no wallet state: it refuses to write
//!   a block row with wallet transactions or the stake-invalidation flag,
//!   and to overwrite or remove one that has either (those changes are a
//!   client-visible write and go through
//!   [`Store::update`](crate::Store::update)). So nothing a wallet
//!   transaction, an address or a balance lives in can be committed
//!   without durability;
//! - the count of commits made without durability since the last durable
//!   one is the store's, shared by every [`InitialSync`] of the handle and
//!   reset by every durable commit, [`Store::update`](crate::Store::update)'s
//!   included. Each commit decides its durability from the count and
//!   updates it under one lock that no other commit of the handle can
//!   enter in between (redb's writer lock alone ends inside the commit,
//!   before the count could be updated), so however many syncs run, on
//!   however many threads, at most [`MAX_DURABLE_INTERVAL`]` - 1`
//!   consecutive commits are non-durable;
//! - [`InitialSync::finish`] (or, on the error path, dropping it) commits
//!   durably once more, and a durable commit makes every earlier
//!   non-durable one durable with it. It skips that commit when the count
//!   is 0, which, the count being exact, means the last commit of the
//!   handle was durable and so covers every batch of this sync.

use crate::error::{Error, Result};
use crate::keys::{Hash32, Height};
use crate::schema::{BlockByHash, Blocks, CFilters, Headers, Table};
use crate::store::{CommitMode, Store};
use crate::txn::WriteTx;
use crate::values::{BlockRow, CFilterRow, HeaderBytes, Tip};

/// The largest number of consecutive commits whose last is durable: at
/// least one durable commit every 50 (ADR-0003 Open point 4, as decided),
/// counted over every [`InitialSync`] of a store handle together. At 2,000
/// headers a batch a crash then fetches at most 98,000 headers and their
/// filters again.
pub const MAX_DURABLE_INTERVAL: u32 = 50;

/// Initial sync's relaxed commits. Created by
/// [`Store::initial_sync`](crate::Store::initial_sync).
///
/// Any number may be alive on one store; they share the store's count of
/// non-durable commits (module documentation), so the bound holds for all
/// of them together.
pub struct InitialSync<'s> {
    store: &'s Store,
    interval: u32,
    finished: bool,
}

impl<'s> InitialSync<'s> {
    pub(crate) fn new(store: &'s Store) -> InitialSync<'s> {
        InitialSync {
            store,
            interval: MAX_DURABLE_INTERVAL,
            finished: false,
        }
    }

    /// Makes this sync's batch durable whenever the store's count of
    /// non-durable commits has reached `interval - 1`, instead of `49`.
    /// `interval` must be between 1 (every batch durable) and
    /// [`MAX_DURABLE_INTERVAL`].
    pub fn with_interval(mut self, interval: u32) -> Result<Self> {
        if interval == 0 || interval > MAX_DURABLE_INTERVAL {
            return Err(Error::invalid("durable interval out of range 1..=50"));
        }
        self.interval = interval;
        Ok(self)
    }

    /// Commits made without durability since the last durable commit of
    /// the store, by this sync and every other one of the same handle.
    pub fn pending(&self) -> u32 {
        self.store.relaxed_pending()
    }

    /// Runs `f` in one write transaction restricted to header-and-filter
    /// writes and commits it: durably when the store has made `interval -
    /// 1` non-durable commits since its last durable one, without
    /// durability otherwise. Returns `f`'s value and whether the commit was
    /// durable. Errors are those of [`Store::update`](crate::Store::update),
    /// [`Error::commit_outcome_unknown`] included.
    pub fn batch<R, E: From<Error>>(
        &mut self,
        f: impl FnOnce(&mut SyncBatch<'_, '_>) -> Result<R, E>,
    ) -> Result<(R, bool), E> {
        let mode = CommitMode::Relaxed {
            interval: self.interval,
        };
        self.store.update_with(mode, |tx| {
            let mut b = SyncBatch { tx };
            f(&mut b)
        })
    }

    /// Ends initial sync with a durable commit, which makes every batch
    /// before it durable.
    pub fn finish(mut self) -> Result<()> {
        self.finished = true;
        self.flush()
    }

    fn flush(&mut self) -> Result<()> {
        if self.store.relaxed_pending() == 0 {
            return Ok(());
        }
        self.store.update(|_| Ok::<_, Error>(()))
    }
}

impl Drop for InitialSync<'_> {
    /// Without [`InitialSync::finish`] (a sync that stopped on an error),
    /// commits durably once more so that the batches that did commit are
    /// kept; an error here cannot be reported and leaves them to be fetched
    /// again after a crash.
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.flush();
        }
    }
}

/// A write transaction of initial sync, restricted to the tables a
/// header-and-filter batch writes. Reads of those tables see the batch's
/// own writes.
///
/// It has no general `put`: a wallet row cannot be written without
/// durability.
///
/// ```compile_fail
/// use purse_store::{CreditKey, CreditRow, Credits, Error, Store};
///
/// fn relaxed_credit(store: &Store) {
///     let mut sync = store.initial_sync();
///     sync.batch(|b| b.put::<Credits>(&CreditKey::default(), &CreditRow::default()))
///         .unwrap();
/// }
/// ```
pub struct SyncBatch<'a, 't> {
    tx: &'a mut WriteTx<'t>,
}

impl SyncBatch<'_, '_> {
    /// Stores a block header under its hash.
    pub fn put_header(&mut self, hash: &Hash32, header: &HeaderBytes) -> Result<()> {
        self.tx.put::<Headers>(hash, header)
    }

    /// Stores a block's committed filter under its hash.
    pub fn put_cfilter(&mut self, hash: &Hash32, filter: &CFilterRow) -> Result<()> {
        self.tx.put::<CFilters>(hash, filter)
    }

    /// Records a main-chain block at `height`, with its hash index,
    /// replacing a row that carries no wallet state (a header-only
    /// reorganisation); the replaced block's hash index goes with it.
    ///
    /// Refused with an `Invalid` error, before anything is written, when
    /// `row` or the row it would replace has wallet transactions or the
    /// stake-invalidation flag: such a row is wallet state, written
    /// durably through [`Store::update`](crate::Store::update).
    pub fn put_block(&mut self, height: u32, row: &BlockRow) -> Result<()> {
        if carries_wallet_state(row) {
            return Err(Error::invalid(
                "a block row with wallet transactions or invalidated is not a header batch's",
            ));
        }
        if let Some(old) = self.tx.get::<Blocks>(&Height(height))? {
            if carries_wallet_state(&old) {
                return Err(Error::invalid(
                    "a header batch cannot replace a block row with wallet transactions or invalidated",
                ));
            }
            if old.hash != row.hash && self.block_height(&old.hash)? == Some(height) {
                self.tx.delete::<BlockByHash>(&old.hash)?;
            }
        }
        self.tx.put::<Blocks>(&Height(height), row)?;
        self.tx.put::<BlockByHash>(&row.hash, &Height(height))
    }

    /// Removes the main-chain block at `height` and its hash index (a
    /// header-only reorganisation). The header and filter stay, as
    /// dcrwallet keeps detached headers (ADR-0003 Open point 5). Returns
    /// the removed row.
    ///
    /// Refused with an `Invalid` error, before anything is removed, when
    /// the row has wallet transactions or the stake-invalidation flag:
    /// detaching such a block changes what a client sees and goes through
    /// [`Store::update`](crate::Store::update).
    pub fn remove_block(&mut self, height: u32) -> Result<Option<BlockRow>> {
        let Some(row) = self.tx.get::<Blocks>(&Height(height))? else {
            return Ok(None);
        };
        if carries_wallet_state(&row) {
            return Err(Error::invalid(
                "a header batch cannot remove a block row with wallet transactions or invalidated",
            ));
        }
        self.tx.delete::<Blocks>(&Height(height))?;
        if self.block_height(&row.hash)? == Some(height) {
            self.tx.delete::<BlockByHash>(&row.hash)?;
        }
        Ok(Some(row))
    }

    /// The header stored under `hash`.
    pub fn header(&mut self, hash: &Hash32) -> Result<Option<HeaderBytes>> {
        self.tx.get::<Headers>(hash)
    }

    /// The main-chain block at `height`.
    pub fn block(&mut self, height: u32) -> Result<Option<BlockRow>> {
        self.tx.get::<Blocks>(&Height(height))
    }

    /// The main-chain height of the block `hash`.
    pub fn block_height(&mut self, hash: &Hash32) -> Result<Option<u32>> {
        Ok(self.tx.get::<BlockByHash>(hash)?.map(|h| h.0))
    }

    /// Records the main chain tip.
    pub fn set_tip(&mut self, tip: &Tip) -> Result<()> {
        self.tx.set_tip(tip)
    }

    /// The main chain tip.
    pub fn tip(&mut self) -> Result<Option<Tip>> {
        self.tx.tip()
    }

    /// Advances the processed-transactions block marker (the rescan
    /// marker).
    pub fn set_processed_marker(&mut self, hash: &Hash32) -> Result<()> {
        self.tx.set_processed_marker(hash)
    }
}

/// Whether a block row holds wallet state a header batch must not change
/// without durability: wallet transactions mined in the block, or the
/// stake invalidation of its regular tree.
fn carries_wallet_state(row: &BlockRow) -> bool {
    row.wallet_txs != 0 || row.invalid
}

// The tables above are exactly `schema::SYNC_TABLES`; this keeps the two in
// step at compile time.
const _: () = {
    let ids = crate::schema::SYNC_TABLES;
    assert!(ids.len() == 4);
    assert!(ids[0] as usize == Headers::ID as usize);
    assert!(ids[1] as usize == CFilters::ID as usize);
    assert!(ids[2] as usize == Blocks::ID as usize);
    assert!(ids[3] as usize == BlockByHash::ID as usize);
};
