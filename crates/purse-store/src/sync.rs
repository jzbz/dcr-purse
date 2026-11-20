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
//! - a [`SyncBatch`] writes only the chain tables ([`crate::chain`]:
//!   `blocks`, `headers`, `cfilters`, `block_by_hash` and the two detached
//!   tables) and the tip and processed-marker rows of `meta`, through the
//!   chain operations alone, which keep the chain's invariants; and in
//!   `blocks` only rows that carry no wallet state: it refuses to connect
//!   a block row with wallet transactions or the stake-invalidation flag,
//!   and to detach one that has either (those changes are a
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

use crate::chain::{Caller, ChainBlock};
use crate::error::{Error, Result};
use crate::keys::Hash32;
use crate::schema::{
    BlockByHash, Blocks, CFilters, CFiltersDetached, Headers, HeadersDetached, Table,
};
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

/// A write transaction of initial sync, restricted to the chain operations
/// of a header-and-filter batch ([`crate::chain`]). Reads see the batch's
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
    /// [`WriteTx::connect_block`]; also refused with kind `Invalid` when
    /// the row has wallet transactions or the stake-invalidation flag:
    /// such a row is wallet state, written durably through
    /// [`Store::update`](crate::Store::update).
    pub fn connect_block(&mut self, height: u32, block: ChainBlock<'_>) -> Result<()> {
        self.tx
            .connect_blocks_as(Caller::HeaderBatch, height, &[block])
    }

    /// [`WriteTx::connect_blocks`], refused as
    /// [`SyncBatch::connect_block`] is.
    pub fn connect_blocks(&mut self, first_height: u32, blocks: &[ChainBlock<'_>]) -> Result<()> {
        self.tx
            .connect_blocks_as(Caller::HeaderBatch, first_height, blocks)
    }

    /// [`WriteTx::promote_block`], refused as
    /// [`SyncBatch::connect_block`] is.
    pub fn promote_block(&mut self, height: u32, row: &BlockRow) -> Result<()> {
        self.tx.promote_block_as(Caller::HeaderBatch, height, row)
    }

    /// [`WriteTx::disconnect_from`] (a header-only reorganisation); also
    /// refused with kind `Invalid`, before anything is detached, when a
    /// block it would detach has wallet transactions or the
    /// stake-invalidation flag: detaching such a block changes what a
    /// client sees and goes through [`Store::update`](crate::Store::update).
    pub fn disconnect_from(&mut self, height: u32) -> Result<Vec<(u32, BlockRow)>> {
        self.tx.disconnect_from_as(Caller::HeaderBatch, height)
    }

    /// [`WriteTx::reorganize`], refused as [`SyncBatch::connect_block`]
    /// and [`SyncBatch::disconnect_from`] are.
    pub fn reorganize(
        &mut self,
        fork_height: u32,
        blocks: &[ChainBlock<'_>],
    ) -> Result<Vec<(u32, BlockRow)>> {
        self.tx
            .reorganize_as(Caller::HeaderBatch, fork_height, blocks)
    }

    /// [`WriteTx::insert_detached`].
    pub fn insert_detached(
        &mut self,
        hash: &Hash32,
        header: &HeaderBytes,
        filter: Option<&CFilterRow>,
    ) -> Result<()> {
        self.tx.insert_detached(hash, header, filter)
    }

    /// [`WriteTx::put_cfilter_at`].
    pub fn put_cfilter_at(&mut self, height: u32, filter: &CFilterRow) -> Result<()> {
        self.tx.put_cfilter_at(height, filter)
    }

    /// The header of the block `hash`, in the main chain or detached.
    pub fn header(&mut self, hash: &Hash32) -> Result<Option<HeaderBytes>> {
        self.tx.header(hash)
    }

    /// The main-chain header at `height`.
    pub fn header_at(&mut self, height: u32) -> Result<Option<HeaderBytes>> {
        self.tx.header_at(height)
    }

    /// The committed filter of the block `hash`, in the main chain or
    /// detached.
    pub fn cfilter(&mut self, hash: &Hash32) -> Result<Option<CFilterRow>> {
        self.tx.cfilter(hash)
    }

    /// The main-chain block at `height`.
    pub fn block(&mut self, height: u32) -> Result<Option<BlockRow>> {
        self.tx.block(height)
    }

    /// The main-chain height of the block `hash`.
    pub fn block_height(&mut self, hash: &Hash32) -> Result<Option<u32>> {
        self.tx.block_height(hash)
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

// The tables the chain operations write are exactly `schema::SYNC_TABLES`;
// this keeps the two in step at compile time.
const _: () = {
    let ids = crate::schema::SYNC_TABLES;
    assert!(ids.len() == 6);
    assert!(ids[0] as usize == Headers::ID as usize);
    assert!(ids[1] as usize == Blocks::ID as usize);
    assert!(ids[2] as usize == BlockByHash::ID as usize);
    assert!(ids[3] as usize == CFilters::ID as usize);
    assert!(ids[4] as usize == HeadersDetached::ID as usize);
    assert!(ids[5] as usize == CFiltersDetached::ID as usize);
};
