// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The chain tables: main-chain blocks, headers and committed filters
//! keyed by height, detached ones keyed by hash, and the operations that
//! keep the two apart.
//!
//! # Layout
//!
//! | table | key | value | holds |
//! |---|---|---|---|
//! | [`Blocks`] | height | [`BlockRow`] | the main chain's block records |
//! | [`Headers`] | height | [`HeaderBytes`] | the main chain's headers |
//! | [`CFilters`] | height | [`CFilterRow`] | the main chain's committed filters |
//! | [`BlockByHash`] | block hash | height | the hash-to-height index of the three above |
//! | [`HeadersDetached`] | block hash | [`HeaderBytes`] | headers of blocks not in the main chain |
//! | [`CFiltersDetached`] | block hash | [`CFilterRow`] | their filters |
//!
//! Main-chain rows are keyed by height, big-endian, so the rows of a batch
//! of new blocks append at the right edge of each tree and a run of
//! filters is one range scan; keyed by hash, every batch landed on random
//! leaves and a 2,000-block commit wrote 3.5 times the bytes (ADR-0003,
//! addendum "Phase 2 measurements"; ADR-0014 Open point 6, decided by the
//! project owner on 2026-11-19 for schema version 1). dcrwallet keeps
//! every header it has ever connected in its `h` bucket by hash and never
//! prunes it, and `getblockheader` returns a detached block's header from
//! there (`internal/rpc/jsonrpc/methods.go:1222`, `wallet/wallet.go:821-829`);
//! the detached tables keep that, by hash, beside the main chain (ADR-0003
//! Open point 5). A lookup by hash ([`ReadTx::header`], [`ReadTx::cfilter`])
//! goes through `block_by_hash` to the height, and to the detached table
//! when the block is not in the main chain: two lookups for a main-chain
//! block instead of dcrwallet's one.
//!
//! # Invariants
//!
//! The chain methods below hold these, and [`ReadTx::check_chain`] checks
//! every one of them:
//!
//! 1. `blocks` holds exactly the heights `0..=n` of a main chain of `n + 1`
//!    blocks, or nothing; there are no gaps.
//! 2. `headers` holds exactly the heights of `blocks`; the header at
//!    height `h` states height `h` and, above the genesis block, names
//!    the hash of `blocks[h - 1]` as its previous block. (The store does
//!    not hash headers; that a block's hash is its header's is the
//!    caller's to ensure.)
//! 3. `cfilters` holds only heights of `blocks`. A main-chain block may
//!    lack its filter (dcrwallet's `have_cfilters` unset, filters being
//!    added by `put_cfilter_at`).
//! 4. `block_by_hash` maps the hash of `blocks[h]` to `h` for every `h`,
//!    and holds nothing else.
//! 5. The `tip` meta row is the height and hash of the last row of
//!    `blocks`, and is absent when `blocks` is empty.
//! 6. No hash in `headers_detached` or `cfilters_detached` is in
//!    `block_by_hash`: a block's header and filter are either the main
//!    chain's, by height, or detached, by hash, never both.
//! 7. Every hash in `cfilters_detached` is in `headers_detached`.
//!
//! The generic `put` and `delete` of [`WriteTx`] reach these tables too,
//! for migrations and tests; a write made that way is not checked.
//!
//! # Operations
//!
//! Each operation checks everything it can refuse before it writes,
//! bbolt's value limit on each filter it is given included
//! ([`MAX_VALUE_SIZE`](crate::MAX_VALUE_SIZE), the one limit of
//! [`WriteTx::put`] a chain row can reach), so a refused call (kind
//! `Invalid` or `Exist`) writes nothing and the transaction stays usable;
//! once checked, its writes are made in the one write transaction, so a
//! reorganisation is atomic with whatever else the transaction commits.
//! An engine error part-way through the writes (kind `Io`) may leave some
//! of them in the transaction, which the caller then abandons by returning
//! the error, as [`Store::update`](crate::Store::update) does.
//!
//! - [`WriteTx::connect_blocks`] (and [`WriteTx::connect_block`]) append
//!   blocks directly above the tip, or from the genesis block on an empty
//!   chain. A block whose header or filter was detached takes them back
//!   (the detached rows are removed; a filter passed replaces the detached
//!   one).
//! - [`WriteTx::promote_block`] connects a detached block from its stored
//!   header and filter.
//! - [`WriteTx::disconnect_from`] detaches every block at and above a
//!   height (dcrwallet's `Rollback`, `wallet/udb/txmined.go:1874`):
//!   each block's row and index go, and its header and filter move to the
//!   detached tables. The genesis block is never detached.
//! - [`WriteTx::reorganize`] detaches every block above a fork point and
//!   connects a new branch there, in one call.
//! - [`WriteTx::insert_detached`] records a header (and filter) of a block
//!   not in the main chain; [`WriteTx::put_cfilter_at`] adds a main-chain
//!   block's missing filter.
//!
//! Every one of them keeps the `tip` row. They move chain rows only: the
//! wallet's rows of a detached block (transactions, credits, balances) are
//! the caller's, in the same transaction. [`SyncBatch`](crate::SyncBatch)
//! offers the same operations under relaxed durability, refusing any that
//! would write, replace or remove a block row carrying wallet state.

use std::collections::BTreeMap;
use std::ops::{Bound, RangeBounds};

use crate::error::{Error, Result};
use crate::keys::{Hash32, Height};
use crate::schema::{
    BlockByHash, Blocks, CFilters, CFiltersDetached, Headers, HeadersDetached, TableId,
};
use crate::txn::{Iter, ReadTx, WriteTx, check_value_len};
use crate::values::{BlockRow, CFilterRow, HeaderBytes, Tip};

/// dcrwallet's text for a block already in the main chain
/// (`ExtendMainChain`, `wallet/udb/txmined.go:220-222`, kind `Exist`).
pub const IN_MAIN_CHAIN_TEXT: &str = "block already recorded in main chain";

/// dcrwallet's text for a header stating height 0 given to connect above
/// the genesis block, or stating a height of 2^31 or more, which
/// dcrwallet reads as a negative `int32` (`ExtendMainChain`,
/// `wallet/udb/txmined.go:204-206`, kind `Invalid`).
pub const GENESIS_AGAIN_TEXT: &str = "block 0 cannot be added";

/// A block to connect to the main chain: its record, its header and, when
/// the caller has it, its committed filter.
#[derive(Clone, Copy, Debug)]
pub struct ChainBlock<'a> {
    /// The block's record; its `hash` is the block's hash.
    pub row: &'a BlockRow,
    /// The block's header.
    pub header: &'a HeaderBytes,
    /// The block's committed filter. `None` keeps a detached filter of the
    /// same block, if there is one, and otherwise records none.
    pub filter: Option<&'a CFilterRow>,
}

/// Whether a block row holds wallet state a header batch must not change
/// without durability: wallet transactions mined in the block, or the
/// stake invalidation of its regular tree.
pub(crate) fn carries_wallet_state(row: &BlockRow) -> bool {
    row.wallet_txs != 0 || row.invalid
}

/// Who asks for a chain change: a durable write transaction, or a relaxed
/// header batch, which may not touch a row with wallet state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
    Durable,
    HeaderBatch,
}

fn height_bounds(r: &impl RangeBounds<u32>) -> (Bound<Height>, Bound<Height>) {
    let f = |b: Bound<&u32>| match b {
        Bound::Included(h) => Bound::Included(Height(*h)),
        Bound::Excluded(h) => Bound::Excluded(Height(*h)),
        Bound::Unbounded => Bound::Unbounded,
    };
    (f(r.start_bound()), f(r.end_bound()))
}

macro_rules! chain_readers {
    ($($m:ident)?) => {
        /// The main-chain block record at `height`.
        pub fn block(&$($m)? self, height: u32) -> Result<Option<BlockRow>> {
            self.get::<Blocks>(&Height(height))
        }

        /// The main-chain height of the block `hash` (`None`: not in the
        /// main chain).
        pub fn block_height(&$($m)? self, hash: &Hash32) -> Result<Option<u32>> {
            Ok(self.get::<BlockByHash>(hash)?.map(|h| h.0))
        }

        /// The main-chain header at `height`.
        pub fn header_at(&$($m)? self, height: u32) -> Result<Option<HeaderBytes>> {
            self.get::<Headers>(&Height(height))
        }

        /// The header of the block `hash`, in the main chain or detached
        /// (dcrwallet's `GetBlockHeader`, which reads its `h` bucket of
        /// every header ever connected).
        pub fn header(&$($m)? self, hash: &Hash32) -> Result<Option<HeaderBytes>> {
            match self.get::<BlockByHash>(hash)? {
                Some(h) => self.get::<Headers>(&h),
                None => self.get::<HeadersDetached>(hash),
            }
        }

        /// The committed filter of the main-chain block at `height`.
        pub fn cfilter_at(&$($m)? self, height: u32) -> Result<Option<CFilterRow>> {
            self.get::<CFilters>(&Height(height))
        }

        /// The committed filter of the block `hash`, in the main chain or
        /// detached (dcrwallet's `CFilterV2`).
        pub fn cfilter(&$($m)? self, hash: &Hash32) -> Result<Option<CFilterRow>> {
            match self.get::<BlockByHash>(hash)? {
                Some(h) => self.get::<CFilters>(&h),
                None => self.get::<CFiltersDetached>(hash),
            }
        }

        /// Main-chain block records at heights in `heights`, in height
        /// order, from either end.
        pub fn blocks_in(&$($m)? self, heights: impl RangeBounds<u32>) -> Result<Iter<'_, Blocks>> {
            let (lo, hi) = height_bounds(&heights);
            self.range::<Blocks>(lo.as_ref(), hi.as_ref())
        }

        /// Main-chain headers at heights in `heights`, in height order.
        pub fn headers_in(&$($m)? self, heights: impl RangeBounds<u32>) -> Result<Iter<'_, Headers>> {
            let (lo, hi) = height_bounds(&heights);
            self.range::<Headers>(lo.as_ref(), hi.as_ref())
        }

        /// Main-chain committed filters at heights in `heights`, in height
        /// order: one range scan (`GetMainChainCFilters`). Heights whose
        /// block lacks its filter are absent.
        pub fn cfilters_in(&$($m)? self, heights: impl RangeBounds<u32>) -> Result<Iter<'_, CFilters>> {
            let (lo, hi) = height_bounds(&heights);
            self.range::<CFilters>(lo.as_ref(), hi.as_ref())
        }
    };
}

impl ReadTx {
    chain_readers!();

    /// Checks every invariant of the chain tables (module documentation
    /// of [`crate::chain`]) by a walk of all six, and fails with a kind-`Io`
    /// error naming the first that does not hold
    /// ([`Error::inconsistency`]). Costs time linear in the chain; for
    /// tests, and for a check after a crash.
    pub fn check_chain(&self) -> Result<()> {
        let bad = |why: String| Err(Error::inconsistent(why));
        let n = self.len(TableId::Blocks)?;
        match (self.first::<Blocks>()?, self.last::<Blocks>()?) {
            (Some((first, _)), Some((last, row))) => {
                if first.0 != 0 || u64::from(last.0) != n.wrapping_sub(1) {
                    return bad(format!(
                        "blocks hold {n} rows from height {} to {}, not 0 to {}",
                        first.0,
                        last.0,
                        n.wrapping_sub(1)
                    ));
                }
                let want = Tip {
                    height: last.0,
                    hash: row.hash,
                };
                if self.tip()? != Some(want) {
                    return bad(format!("the tip row is not the last block ({})", last.0));
                }
            }
            _ => {
                if self.tip()?.is_some() {
                    return bad("a tip row with no main chain".to_owned());
                }
            }
        }
        if self.len(TableId::Headers)? != n {
            return bad("headers and blocks differ in length".to_owned());
        }
        if self.len(TableId::BlockByHash)? != n {
            return bad("block_by_hash and blocks differ in length".to_owned());
        }
        if let Some((h, _)) = self.last::<CFilters>()?
            && u64::from(h.0) >= n
        {
            return bad(format!("a filter at height {} above the main chain", h.0));
        }
        let mut prev: Option<Hash32> = None;
        for r in self.iter::<Blocks>()? {
            let (h, row) = r?;
            let Some(header) = self.get::<Headers>(&h)? else {
                return bad(format!("no header at height {}", h.0));
            };
            if header.height() != h.0 {
                return bad(format!(
                    "the header at height {} states height {}",
                    h.0,
                    header.height()
                ));
            }
            if let Some(p) = prev
                && header.prev_block() != p
            {
                return bad(format!(
                    "the header at height {} does not name the block below",
                    h.0
                ));
            }
            if self.get::<BlockByHash>(&row.hash)? != Some(h) {
                return bad(format!(
                    "block_by_hash does not map the block at height {} to it",
                    h.0
                ));
            }
            prev = Some(row.hash);
        }
        for r in self.iter::<HeadersDetached>()? {
            let (hash, _) = r?;
            if self.contains::<BlockByHash>(&hash)? {
                return bad("a detached header of a main-chain block".to_owned());
            }
        }
        for r in self.iter::<CFiltersDetached>()? {
            let (hash, _) = r?;
            if !self.contains::<HeadersDetached>(&hash)? {
                return bad("a detached filter without its detached header".to_owned());
            }
        }
        Ok(())
    }
}

/// dcrwallet's first two refusals of a block given to connect
/// (`ExtendMainChain`, `wallet/udb/txmined.go:204-222`), which read the
/// block's header and not the height the caller gives: a header stating
/// height 0 when the block does not start an empty chain (`starts_chain`),
/// or a height of 2^31 or more, which dcrwallet's `int32(header.Height)`
/// makes negative and so below 1, whatever the chain; and a block already
/// in the main chain at the height its header states. The main chain is
/// what stays of the stored one (`kept`, the block's height there) and the
/// blocks of the same call checked before this one (`earlier`, at their
/// heights).
fn refuse_known(
    b: &ChainBlock<'_>,
    kept: Option<u32>,
    earlier: &BTreeMap<Hash32, u32>,
    starts_chain: bool,
) -> Result<()> {
    let stated = b.header.height();
    if i32::try_from(stated).is_err() || (stated == 0 && !starts_chain) {
        return Err(Error::invalid(GENESIS_AGAIN_TEXT));
    }
    let at = kept.or_else(|| earlier.get(&b.row.hash).copied());
    if at == Some(stated) {
        return Err(Error::duplicate(IN_MAIN_CHAIN_TEXT));
    }
    Ok(())
}

/// The refusal of a block row with wallet state by a header batch.
const NOT_A_HEADER_BATCH_ROW: &str =
    "a block row with wallet transactions or invalidated is not a header batch's";

impl WriteTx<'_> {
    chain_readers!(mut);

    /// Connects `block` at `height`, directly above the tip (at 0 on an
    /// empty chain), and makes it the tip. A detached header and filter of
    /// the block are removed, the filter kept when `block` carries none.
    ///
    /// The refusals come in dcrwallet's order (`ExtendMainChain`,
    /// `wallet/udb/txmined.go:204-226`), which looks at the header before
    /// the height it is given:
    ///
    /// 1. a header stating height 0 on a chain that already has its genesis
    ///    block, or stating a height of 2^31 or more on any chain (dcrwallet
    ///    reads the height as an `int32`): kind `Invalid`, dcrwallet's text
    ///    ([`GENESIS_AGAIN_TEXT`]). The operations therefore never put a
    ///    block above height 2^31 - 1;
    /// 2. a block already in the main chain at the height its header
    ///    states, whatever `height` is: kind `Exist`, dcrwallet's text
    ///    ([`IN_MAIN_CHAIN_TEXT`]). This is the refusal a block delivered a
    ///    second time gets, and dcrwallet's caller ignores it
    ///    (`wallet/chainntfns.go:45-49`);
    /// 3. in a header batch ([`SyncBatch`](crate::SyncBatch)), a block row
    ///    with wallet state: kind `Invalid`;
    /// 4. `height` not the next height, a header not stating `height`, or
    ///    one not naming the tip as its previous block: kind `Invalid`;
    /// 5. a block hash already in the main chain at another height than
    ///    its header states (a header that is not the block's): kind
    ///    `Invalid`. dcrwallet would record such a block a second time;
    /// 6. a filter whose row is longer than bbolt's value limit
    ///    ([`MAX_VALUE_SIZE`](crate::MAX_VALUE_SIZE)): kind `Invalid`,
    ///    `value too large`, as [`WriteTx::put`] refuses it, but before any
    ///    row of the call is written.
    pub fn connect_block(&mut self, height: u32, block: ChainBlock<'_>) -> Result<()> {
        self.connect_blocks(height, &[block])
    }

    /// Connects `blocks` at `first_height` and the heights after it, as
    /// [`WriteTx::connect_block`] does each, refusing all of them, before
    /// anything is written, when any would be refused. Each block's header
    /// must name the block before it in `blocks`. The blocks are checked in
    /// order, as dcrwallet's `ExtendMainChain` would be called for each, so
    /// a block given twice is refused with kind `Exist` at its second copy
    /// when that copy's header states the height of the first.
    pub fn connect_blocks(&mut self, first_height: u32, blocks: &[ChainBlock<'_>]) -> Result<()> {
        self.connect_blocks_as(Caller::Durable, first_height, blocks)
    }

    pub(crate) fn connect_blocks_as(
        &mut self,
        caller: Caller,
        first_height: u32,
        blocks: &[ChainBlock<'_>],
    ) -> Result<()> {
        let keep = self.last::<Blocks>()?.map(|(tip, _)| tip.0);
        // dcrwallet's refusals of the header come before the height given.
        if let Some(b) = blocks.first() {
            let kept = self.kept_height(keep, &b.row.hash)?;
            refuse_known(b, kept, &BTreeMap::new(), keep.is_none())?;
            if caller == Caller::HeaderBatch && carries_wallet_state(b.row) {
                return Err(Error::invalid(NOT_A_HEADER_BATCH_ROW));
            }
        }
        match keep {
            Some(tip) => {
                if tip.checked_add(1) != Some(first_height) {
                    return Err(Error::invalid(
                        "a block connects only at the height above the tip",
                    ));
                }
            }
            None => {
                if first_height != 0 {
                    return Err(Error::invalid(
                        "an empty main chain starts with a block at height 0",
                    ));
                }
            }
        }
        self.change_chain(caller, keep, blocks).map(drop)
    }

    /// Connects the detached block `row.hash` at `height` from its stored
    /// header and filter, as [`WriteTx::connect_block`] does. Refused with
    /// kind `Invalid` when no detached header of the block is stored, and
    /// as `connect_block` refuses.
    pub fn promote_block(&mut self, height: u32, row: &BlockRow) -> Result<()> {
        self.promote_block_as(Caller::Durable, height, row)
    }

    pub(crate) fn promote_block_as(
        &mut self,
        caller: Caller,
        height: u32,
        row: &BlockRow,
    ) -> Result<()> {
        let Some(header) = self.get::<HeadersDetached>(&row.hash)? else {
            return Err(Error::invalid("no detached header of the block to promote"));
        };
        let block = ChainBlock {
            row,
            header: &header,
            filter: None,
        };
        self.connect_blocks_as(caller, height, &[block])
    }

    /// Detaches every main-chain block at `height` and above, the tip
    /// first, as dcrwallet's `Rollback(height)` removes its block records:
    /// each block's row and hash index go, and its header and filter move
    /// to the detached tables. The block at `height - 1` becomes the tip.
    /// Returns the detached rows, highest first; none when nothing stands
    /// at `height`. Refused with kind `Invalid` for height 0: the genesis
    /// block is never detached.
    pub fn disconnect_from(&mut self, height: u32) -> Result<Vec<(u32, BlockRow)>> {
        self.disconnect_from_as(Caller::Durable, height)
    }

    pub(crate) fn disconnect_from_as(
        &mut self,
        caller: Caller,
        height: u32,
    ) -> Result<Vec<(u32, BlockRow)>> {
        let Some(keep) = height.checked_sub(1) else {
            return Err(Error::invalid("the genesis block cannot be detached"));
        };
        match self.last::<Blocks>()? {
            Some((tip, _)) if tip.0 >= height => self.change_chain(caller, Some(keep), &[]),
            _ => Ok(Vec::new()),
        }
    }

    /// Detaches every main-chain block above `fork_height`, as
    /// [`WriteTx::disconnect_from`] does, and connects `blocks` from
    /// `fork_height + 1`, as [`WriteTx::connect_blocks`] does, checking
    /// everything first: refused, with nothing written, when `fork_height`
    /// is above the tip or any block would be refused against the chain as
    /// it stands after the detach. A block of the detached branch may be in
    /// `blocks` again; one at or below `fork_height`, at the height its
    /// header states, is refused with kind `Exist` and
    /// [`IN_MAIN_CHAIN_TEXT`], as dcrwallet's `ExtendMainChain` refuses it
    /// after its `Rollback`. Returns the detached rows, highest first.
    pub fn reorganize(
        &mut self,
        fork_height: u32,
        blocks: &[ChainBlock<'_>],
    ) -> Result<Vec<(u32, BlockRow)>> {
        self.reorganize_as(Caller::Durable, fork_height, blocks)
    }

    pub(crate) fn reorganize_as(
        &mut self,
        caller: Caller,
        fork_height: u32,
        blocks: &[ChainBlock<'_>],
    ) -> Result<Vec<(u32, BlockRow)>> {
        self.change_chain(caller, Some(fork_height), blocks)
    }

    /// Records the header, and the filter when given, of the block `hash`,
    /// which is not in the main chain (a block not yet connected),
    /// replacing a detached header or filter stored for it. Refused with
    /// kind `Exist` and [`IN_MAIN_CHAIN_TEXT`] when the block is in the
    /// main chain, and with kind `Invalid`, `value too large`, before the
    /// header is written, when the filter's row is longer than bbolt's
    /// value limit ([`MAX_VALUE_SIZE`](crate::MAX_VALUE_SIZE)).
    pub fn insert_detached(
        &mut self,
        hash: &Hash32,
        header: &HeaderBytes,
        filter: Option<&CFilterRow>,
    ) -> Result<()> {
        if self.contains::<BlockByHash>(hash)? {
            return Err(Error::duplicate(IN_MAIN_CHAIN_TEXT));
        }
        if let Some(f) = filter {
            check_value_len(f.encoded_len())?;
        }
        self.put::<HeadersDetached>(hash, header)?;
        if let Some(f) = filter {
            self.put::<CFiltersDetached>(hash, f)?;
        }
        Ok(())
    }

    /// Records the committed filter of the main-chain block at `height`
    /// (dcrwallet's `InsertMissingCFilters` and `ImportCFiltersV2`, which
    /// key it by the block's hash). Refused with kind `Invalid` when no
    /// main-chain block stands at `height`.
    pub fn put_cfilter_at(&mut self, height: u32, filter: &CFilterRow) -> Result<()> {
        if !self.contains::<Blocks>(&Height(height))? {
            return Err(Error::invalid("no main-chain block at the filter's height"));
        }
        self.put::<CFilters>(&Height(height), filter)
    }

    /// The main-chain height of `hash` when it stays in the main chain, at
    /// or below `keep` (nothing stays when `None`).
    fn kept_height(&mut self, keep: Option<u32>, hash: &Hash32) -> Result<Option<u32>> {
        Ok(self
            .block_height(hash)?
            .filter(|h| keep.is_some_and(|k| *h <= k)))
    }

    /// Keeps the main chain up to `keep` (nothing when `None`), detaching
    /// what stands above it, and connects `new` above it. Everything is
    /// checked before anything is written.
    fn change_chain(
        &mut self,
        caller: Caller,
        keep: Option<u32>,
        new: &[ChainBlock<'_>],
    ) -> Result<Vec<(u32, BlockRow)>> {
        // What stays, and what goes.
        let tip = self.last::<Blocks>()?;
        let below = match (keep, tip) {
            (None, None) => None,
            (None, Some(_)) => {
                return Err(Error::invalid("the genesis block cannot be detached"));
            }
            (Some(_), None) => {
                return Err(Error::invalid("the fork point is above the tip"));
            }
            (Some(k), Some((t, ref row))) => {
                if k > t.0 {
                    return Err(Error::invalid("the fork point is above the tip"));
                }
                if k == t.0 {
                    Some((k, row.hash))
                } else {
                    let row = self
                        .get::<Blocks>(&Height(k))?
                        .ok_or_else(|| Error::inconsistent(format!("no block at height {k}")))?;
                    Some((k, row.hash))
                }
            }
        };
        let detach: Vec<(u32, BlockRow)> = match below {
            Some((k, _)) => {
                let mut v = self
                    .blocks_in((Bound::Excluded(k), Bound::Unbounded))?
                    .map(|r| r.map(|(h, row)| (h.0, row)))
                    .collect::<Result<Vec<_>>>()?;
                v.reverse();
                v
            }
            None => Vec::new(),
        };

        // Checks: each block of `new` in order, as dcrwallet's
        // `ExtendMainChain` would be called for each, then what a header
        // batch may not detach.
        //
        // The height of `new[0]`; none above `u32::MAX`, which matters only
        // when there is a block to put there.
        let first_height = match below {
            Some((k, _)) => k.checked_add(1),
            None => Some(0),
        };
        let first_height = match (first_height, new.is_empty()) {
            (Some(h), _) => h,
            (None, true) => u32::MAX,
            (None, false) => return Err(Error::invalid("no height above the fork point")),
        };
        let mut prev = below.map(|(_, hash)| hash);
        let mut height = first_height;
        // The blocks of `new` checked so far, at the heights they go in at.
        let mut seen = BTreeMap::new();
        for (i, b) in new.iter().enumerate() {
            if i > 0 {
                height = height
                    .checked_add(1)
                    .ok_or_else(|| Error::invalid("no height above the last block"))?;
            }
            let kept = self.kept_height(keep, &b.row.hash)?;
            refuse_known(b, kept, &seen, keep.is_none() && i == 0)?;
            if caller == Caller::HeaderBatch && carries_wallet_state(b.row) {
                return Err(Error::invalid(NOT_A_HEADER_BATCH_ROW));
            }
            if b.header.height() != height {
                return Err(Error::invalid(
                    "the header does not state the height it is connected at",
                ));
            }
            if let Some(p) = prev
                && b.header.prev_block() != p
            {
                return Err(Error::invalid(
                    "the header does not name the block below as its previous block",
                ));
            }
            // A block of the branch that goes may come back; one that stays,
            // or one earlier in `new`, may not be connected a second time.
            // At the height its header states it was refused above; at
            // another, the header is not the block's.
            if seen.insert(b.row.hash, height).is_some() || kept.is_some() {
                return Err(Error::invalid(
                    "the block is already in the main chain at another height than its header states",
                ));
            }
            // The one put limit a chain row can reach: the other rows are of
            // fixed length, and a detached filter taken back was put once.
            if let Some(f) = b.filter {
                check_value_len(f.encoded_len())?;
            }
            prev = Some(b.row.hash);
        }
        if caller == Caller::HeaderBatch && detach.iter().any(|(_, row)| carries_wallet_state(row))
        {
            return Err(Error::invalid(
                "a header batch cannot detach a block row with wallet transactions or invalidated",
            ));
        }

        // Detach, the tip first.
        for (h, row) in &detach {
            let key = Height(*h);
            let header = self.take::<Headers>(&key)?;
            let filter = self.take::<CFilters>(&key)?;
            self.delete::<Blocks>(&key)?;
            if self.get::<BlockByHash>(&row.hash)? == Some(key) {
                self.delete::<BlockByHash>(&row.hash)?;
            }
            if let Some(header) = header {
                self.put::<HeadersDetached>(&row.hash, &header)?;
            }
            if let Some(filter) = filter {
                self.put::<CFiltersDetached>(&row.hash, &filter)?;
            }
        }

        // Connect.
        let mut height = first_height;
        for b in new {
            let key = Height(height);
            self.delete::<HeadersDetached>(&b.row.hash)?;
            let detached_filter = self.take::<CFiltersDetached>(&b.row.hash)?;
            self.put::<Blocks>(&key, b.row)?;
            self.put::<Headers>(&key, b.header)?;
            self.put::<BlockByHash>(&b.row.hash, &key)?;
            if let Some(f) = b.filter.or(detached_filter.as_ref()) {
                self.put::<CFilters>(&key, f)?;
            }
            height = height.wrapping_add(1);
        }

        // The tip.
        if let Some(last) = new.last() {
            self.set_tip(&Tip {
                height: height.wrapping_sub(1),
                hash: last.row.hash,
            })?;
        } else if let (Some((k, hash)), false) = (below, detach.is_empty()) {
            self.set_tip(&Tip { height: k, hash })?;
        }
        Ok(detach)
    }
}
