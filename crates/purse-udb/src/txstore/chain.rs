// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The main chain, headers and committed filters (`txmined.go:181-622`,
//! `884-1010`, `cfilters.go`).

use std::ops::Bound;

use dcroxide_gcs::FilterV2;
use dcroxide_gcs::blockcf2;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    BirthState, BlockRow, Blocks, CFilterRow, ChainBlock, Hash32, HeaderBytes, Height, TableId,
    WriteTx,
};

use super::record::{
    BlockMeta, display, header_height, header_merkle_root, header_time, header_vote_bits,
};
use super::store::{TxRead, TxStore, guard, store_err, walk_keys};

/// A main-chain committed filter (`BlockCFilter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockCFilter {
    /// The block hash.
    pub hash: Hash32,
    /// The filter's key: the first 16 bytes of the header's merkle root.
    pub key: [u8; 16],
    /// The serialized version 2 filter.
    pub filter: Vec<u8>,
}

fn not_exist(msg: String) -> AnyError {
    e!(Kind::NotExist, msg).into()
}

fn invalid(msg: impl Into<String>) -> AnyError {
    e!(Kind::Invalid, msg.into()).into()
}

/// Go's `fetchRawBlockHeader` refusal.
fn no_header() -> AnyError {
    e!(Kind::NotExist, "block header").into()
}

/// The main-chain height of a Go `int32` height: none for a negative one,
/// which Go's `keyBlockRecord(uint32(h))` puts above every stored block.
fn stored_height(h: i32) -> Option<u32> {
    u32::try_from(h).ok()
}

/// The header of `hash`, in the main chain or detached (Go's `h` bucket).
pub(crate) fn header_of(
    r: &mut (impl TxRead + ?Sized),
    hash: &Hash32,
) -> Result<Option<HeaderBytes>, AnyError> {
    match r.get::<purse_store::BlockByHash>(hash).map_err(store_err)? {
        Some(h) => r.get::<purse_store::Headers>(&h).map_err(store_err),
        None => r
            .get::<purse_store::HeadersDetached>(hash)
            .map_err(store_err),
    }
}

/// The main-chain block row at `height`.
pub(crate) fn block_at(
    r: &mut (impl TxRead + ?Sized),
    height: u32,
) -> Result<Option<BlockRow>, AnyError> {
    r.get::<Blocks>(&Height(height)).map_err(store_err)
}

/// The tip row; its absence is stored state Go cannot read (its
/// `MainChainTip` panics on the missing header).
pub(crate) fn tip(r: &mut (impl TxRead + ?Sized)) -> Result<purse_store::Tip, AnyError> {
    r.tip_row()
        .map_err(store_err)?
        .ok_or_else(|| e!(Kind::Io, "no main chain tip is recorded").into())
}

/// The birthday as `extend_main_chain` and `missing_cfilters_height` read
/// it (ADR-0019 decision item 1 with FB10, FB11 (i) and FB12 (a); the
/// addendum "2026-11-22 (later)", items 1, 4 and 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Birthday {
    /// A resolved birthday (the `birth` row's height), a birthday pending
    /// from a height H (H, the row's height), or no `birth` row (0, FB10).
    Height(u32),
    /// A birthday pending from a time T (Unix seconds) only: there is no
    /// birthday height, and a block lies below the birthday while its own
    /// timestamp is not after T.
    PendingTime(i64),
    /// A birthday pending from both a height H and a time T, which
    /// dcrwallet resolves at whichever comes first (`chainntfns.go:186-195`).
    /// ADR-0019 states no rule for it: a block is taken as below it only
    /// when it lies below both (`extend_main_chain`), and the walk starts
    /// at H (`missing_cfilters_height`).
    PendingBoth(u32, i64),
}

impl Birthday {
    /// Whether a block at `height` whose header's timestamp is `time` may
    /// be connected without its committed filter (rule 4 as FB11 (i)
    /// widens it): below the birthday height, or, under a pending time
    /// birthday, with a timestamp not after T. With no `birth` row (height
    /// 0) no block may.
    fn takes_no_filter(self, height: u32, time: i64) -> bool {
        match self {
            Birthday::Height(h) => height < h,
            Birthday::PendingTime(t) => time <= t,
            Birthday::PendingBoth(h, t) => height < h && time <= t,
        }
    }
}

/// The `birth` row read as ADR-0019's birthday ([`Birthday`]).
fn birthday(r: &mut (impl TxRead + ?Sized)) -> Result<Birthday, AnyError> {
    Ok(match r.birth_row().map_err(store_err)? {
        None => Birthday::Height(0),
        Some(b) => match (b.set_from_height, b.set_from_time) {
            (_, false) => Birthday::Height(b.height),
            (false, true) => Birthday::PendingTime(b.time),
            (true, true) => Birthday::PendingBoth(b.height, b.time),
        },
    })
}

/// The tables stake (in)validation reads and moves rows between in Go.
const STAKE_VALIDATION_TABLES: [TableId; 6] = [
    TableId::Credits,
    TableId::CreditsInvalidated,
    TableId::Unspent,
    TableId::Debits,
    TableId::DebitsInvalidated,
    TableId::Balances,
];

impl TxStore {
    /// `MainChainTip`: the tip block's hash and height.
    ///
    /// # Errors
    ///
    /// No tip is recorded (Go panics): kind `IO`. The store's.
    pub fn main_chain_tip(
        &self,
        r: &mut (impl TxRead + ?Sized),
    ) -> Result<(Hash32, i32), AnyError> {
        let t = tip(r)?;
        Ok((t.hash, i32::try_from(t.height).unwrap_or(i32::MAX)))
    }

    /// `ExtendMainChain`: connects the block `hash` with `header` above the
    /// tip, with its committed filter `filter` (the serialized version 2
    /// filter), and stake validates or invalidates the tip by the header's
    /// vote bit.
    ///
    /// A block without its filter is accepted below the birthday height
    /// and refused at or above it (ADR-0019 FB4); while a birthday set from
    /// a time T is pending, it is accepted when the header's own timestamp
    /// is not after T and refused otherwise (FB11 (i)). Go always takes
    /// one.
    ///
    /// # Errors
    ///
    /// Go's, in Go's order: a header stating height 0 (or 2^31 and more):
    /// `Invalid`, `block 0 cannot be added`; `have_cfilters` unset:
    /// `Invalid`; a header not naming the tip as its parent: `Exist`,
    /// `block already recorded in main chain`, when the main-chain block at
    /// the header's height is this one, `Invalid` otherwise; the wrong
    /// height: `Invalid`. A missing filter at or above the birthday height:
    /// `Invalid`. The stake (in)validation of a tip with credit rows: `Bug`
    /// (the guard). The store's.
    pub fn extend_main_chain(
        &self,
        tx: &mut WriteTx<'_>,
        header: &HeaderBytes,
        hash: &Hash32,
        filter: Option<&[u8]>,
    ) -> Result<(), AnyError> {
        let height = header_height(header);
        if height < 1 {
            return Err(invalid("block 0 cannot be added"));
        }
        if !tx.have_cfilters_row().map_err(store_err)? {
            return Err(invalid(
                "main chain may not be extended without first saving all previous cfilters",
            ));
        }
        let t = tip(tx)?;
        if header.prev_block() != t.hash {
            let at = stored_height(height).map_or(Ok(None), |h| block_at(tx, h))?;
            if at.is_some_and(|b| b.hash == *hash) {
                return Err(e!(Kind::Exist, purse_store::IN_MAIN_CHAIN_TEXT).into());
            }
            return Err(invalid("not direct child of current tip"));
        }
        if i64::from(t.height).checked_add(1) != Some(i64::from(height)) {
            return Err(invalid("invalid height for next block"));
        }
        let new_height = height.unsigned_abs();
        if filter.is_none() && !birthday(tx)?.takes_no_filter(new_height, header_time(header)) {
            return Err(invalid(
                "a block at or above the birthday height needs its committed filter",
            ));
        }
        let approves = header_vote_bits(header) & 1 != 0;
        // Go stake (in)validates the tip before it connects the block; the
        // tip's row is written after the connection here, so that the
        // store's refusals of the block (a hash already in the main chain
        // at another height, a filter over bbolt's value limit) come before
        // any write. The rows written are the same.
        let validity = self.stake_validity(tx, t.height, !approves)?;
        let mut key = [0u8; 16];
        key.copy_from_slice(&header_merkle_root(header)[..16]);
        let row = BlockRow {
            hash: *hash,
            time: header_time(header),
            vote_bits: header_vote_bits(header),
            invalid: false,
            wallet_txs: 0,
        };
        let f = filter.map(|f| CFilterRow {
            key,
            filter: f.to_vec(),
        });
        tx.connect_block(
            new_height,
            ChainBlock {
                row: &row,
                header,
                filter: f.as_ref(),
            },
        )
        .map_err(store_err)?;
        if let Some((height, row)) = validity {
            tx.put::<Blocks>(&Height(height), &row).map_err(store_err)?;
        }
        Ok(())
    }

    /// `stakeValidate` and `stakeInvalidate` (`txmined.go:631-885`): the
    /// row of the block at `height` marked invalidated or not, to be written
    /// by the caller, or `None` when the mark does not change. Moving its
    /// regular transactions' credits and debits is the credit slice's; here
    /// the guard stands where Go reads them.
    fn stake_validity(
        &self,
        tx: &mut WriteTx<'_>,
        height: u32,
        invalid: bool,
    ) -> Result<Option<(u32, BlockRow)>, AnyError> {
        let Some(mut row) = block_at(tx, height)? else {
            return Err(e!(
                Kind::Io,
                format!("missing block record for height {height}")
            )
            .into());
        };
        if row.invalid == invalid {
            return Ok(None);
        }
        if row.wallet_txs != 0 {
            guard(
                tx,
                if invalid {
                    "stakeInvalidate"
                } else {
                    "stakeValidate"
                },
                &STAKE_VALIDATION_TABLES,
            )?;
        }
        row.invalid = invalid;
        Ok(Some((height, row)))
    }

    /// `ProcessedTxsBlockMarker`: the marker's hash, zero when absent.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn processed_txs_block_marker(
        &self,
        r: &mut (impl TxRead + ?Sized),
    ) -> Result<Hash32, AnyError> {
        Ok(r.processed_row().map_err(store_err)?.unwrap_or_default())
    }

    /// `UpdateProcessedTxsBlockMarker`: moves the marker to the main-chain
    /// block `hash` when it is higher than the marker's main-chain fork
    /// point.
    ///
    /// # Errors
    ///
    /// Go's: a marker (or an ancestor) without a header: `NotExist`,
    /// `block header`; `hash` not in the main chain: `Invalid`. The
    /// store's.
    pub fn update_processed_txs_block_marker(
        &self,
        tx: &mut WriteTx<'_>,
        hash: &Hash32,
    ) -> Result<(), AnyError> {
        let mut prev = self.processed_txs_block_marker(tx)?;
        while !self.block_in_main_chain(tx, &prev)?.0 {
            prev = header_of(tx, &prev)?.ok_or_else(no_header)?.prev_block();
        }
        let prev_header = header_of(tx, &prev)?.ok_or_else(no_header)?;
        if !self.block_in_main_chain(tx, hash)?.0 {
            return Err(invalid(format!(
                "{} is not a main chain block",
                display(hash)
            )));
        }
        let header = header_of(tx, hash)?.ok_or_else(no_header)?;
        if header.height() > prev_header.height() {
            tx.set_processed_marker(hash).map_err(store_err)?;
        }
        Ok(())
    }

    /// `IsMissingMainChainCFilters`: whether `have_cfilters` is unset.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn is_missing_main_chain_cfilters(
        &self,
        r: &mut (impl TxRead + ?Sized),
    ) -> Result<bool, AnyError> {
        Ok(!r.have_cfilters_row().map_err(store_err)?)
    }

    /// `MissingCFiltersHeight`: the lowest main-chain height at or above
    /// the birthday height whose block has no filter (ADR-0019 FB9 (a):
    /// Go walks from the genesis block). The block heights are read page
    /// by page (`store::walk_keys`), so the walk stops near the first
    /// missing filter, as Go's cursor does, and neither a block row nor a
    /// filter is copied or decoded: a filter's presence is all the test
    /// reads, as Go's `fetchRawCFilter2` hands back a slice of the page.
    /// A filter row too short to hold its key, which Go's walk passes over
    /// (its `IO` error is not `NotExist`), counts as present here too.
    /// While a birthday set from a time alone is pending there is no
    /// birthday height, and nothing is walked (ADR-0019 FB12 (a)); a row
    /// pending from both a height H and a time is walked from H.
    ///
    /// # Errors
    ///
    /// No filter is missing, or a birthday set from a time alone is
    /// pending: `NotExist`, as Go when none is missing. The store's.
    pub fn missing_cfilters_height(&self, r: &mut (impl TxRead + ?Sized)) -> Result<i32, AnyError> {
        let from = match birthday(r)? {
            Birthday::Height(h) | Birthday::PendingBoth(h, _) => h,
            // FB12 (a): the `have_cfilters` row's 1 speaks of the birthday
            // once resolved.
            Birthday::PendingTime(_) => return Err(e!(Kind::NotExist).into()),
        };
        let mut missing = None;
        walk_keys::<Blocks, _, _>(
            r,
            Bound::Included(Height(from)),
            Bound::Unbounded,
            false,
            |r, h| {
                if r.contains::<purse_store::CFilters>(&h).map_err(store_err)? {
                    return Ok(false);
                }
                missing = Some(h.0);
                Ok(true)
            },
        )?;
        match missing {
            Some(h) => Ok(i32::try_from(h).unwrap_or(i32::MAX)),
            None => Err(e!(Kind::NotExist).into()),
        }
    }

    /// `InsertMissingCFilters`: records the filters of the main-chain
    /// blocks `hashes`, in chain order from a block whose parent has its
    /// filter, and sets `have_cfilters` when the last is the tip.
    ///
    /// # Errors
    ///
    /// Go's: `have_cfilters` set (always, in a store this crate creates):
    /// `Invalid`; slices of different lengths: `Invalid`; a block without a
    /// header: `NotExist`; blocks out of order or after a missing filter:
    /// `Invalid`, nothing written (Go has written the filters before the
    /// refused block). The store's.
    pub fn insert_missing_cfilters(
        &self,
        tx: &mut WriteTx<'_>,
        hashes: &[Hash32],
        filters: &[Vec<u8>],
    ) -> Result<(), AnyError> {
        if tx.have_cfilters_row().map_err(store_err)? {
            return Err(invalid(
                "all cfilters for main chain blocks are already recorded",
            ));
        }
        if hashes.len() != filters.len() {
            return Err(invalid("slices must have equal len"));
        }
        let Some(last) = hashes.last() else {
            return Ok(());
        };
        // Go writes each filter after checking its block, so a refusal
        // leaves the filters before it written; here every block is
        // checked first, in Go's order and with Go's errors, and nothing is
        // written on a refusal. No check reads a filter this call writes:
        // the first block's parent filter is read before Go has written
        // anything either, and the others are checked against the list.
        let mut keys = Vec::with_capacity(hashes.len());
        for (i, hash) in hashes.iter().enumerate() {
            let mut ok = i == 0 && *hash == *self.genesis_hash();
            // QK-0093: the genesis block's key stays zero.
            let mut key = [0u8; 16];
            if !ok {
                let Some(header) = header_of(tx, hash)? else {
                    return Err(not_exist(format!(
                        "missing header for block {}",
                        display(hash)
                    )));
                };
                key.copy_from_slice(&header_merkle_root(&header)[..16]);
                let parent = header.prev_block();
                ok = match i.checked_sub(1) {
                    None => self.cfilter_row(tx, &parent)?.is_some(),
                    Some(p) => hashes.get(p) == Some(&parent),
                };
            }
            if !ok {
                return Err(invalid(
                    "block hashes are not ordered or previous cfilters are missing",
                ));
            }
            keys.push(key);
        }
        for ((hash, f), key) in hashes.iter().zip(filters).zip(keys) {
            self.put_cfilter(
                tx,
                hash,
                &CFilterRow {
                    key,
                    filter: f.clone(),
                },
            )?;
        }
        let t = tip(tx)?;
        if t.hash == *last {
            tx.set_have_cfilters(true).map_err(store_err)?;
        }
        Ok(())
    }

    /// The filter of `hash`, main chain or detached.
    fn cfilter_row(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<Option<CFilterRow>, AnyError> {
        match r.get::<purse_store::BlockByHash>(hash).map_err(store_err)? {
            Some(h) => r.get::<purse_store::CFilters>(&h).map_err(store_err),
            None => r
                .get::<purse_store::CFiltersDetached>(hash)
                .map_err(store_err),
        }
    }

    /// Go's `putRawCFilter` keyed by hash: the main chain's row by height,
    /// or a detached block's, which needs its detached header.
    fn put_cfilter(
        &self,
        tx: &mut WriteTx<'_>,
        hash: &Hash32,
        f: &CFilterRow,
    ) -> Result<(), AnyError> {
        match tx.block_height(hash).map_err(store_err)? {
            Some(h) => tx.put_cfilter_at(h, f).map_err(store_err),
            None => {
                let header = tx
                    .get::<purse_store::HeadersDetached>(hash)
                    .map_err(store_err)?
                    .ok_or_else(no_header)?;
                tx.insert_detached(hash, &header, Some(f))
                    .map_err(store_err)
            }
        }
    }

    /// `GetMainChainCFilters`: up to `count` main-chain filters from the
    /// height of `start`'s header (after it unless `inclusive`). A
    /// detached start block's height is used as Go uses it (QK-0751).
    ///
    /// # Errors
    ///
    /// Go's: no header for `start`: `NotExist`; a main-chain block without
    /// its filter: `NotExist`. The store's.
    pub fn get_main_chain_cfilters(
        &self,
        r: &mut (impl TxRead + ?Sized),
        start: &Hash32,
        inclusive: bool,
        count: usize,
    ) -> Result<Vec<BlockCFilter>, AnyError> {
        let Some(header) = header_of(r, start)? else {
            return Err(not_exist(format!(
                "starting block {} not found",
                display(start)
            )));
        };
        let mut height = header.height();
        if !inclusive {
            height = height.wrapping_add(1);
        }
        let mut out = Vec::new();
        while out.len() < count {
            let Some(row) = block_at(r, height)? else {
                break;
            };
            let Some(f) = r
                .get::<purse_store::CFilters>(&Height(height))
                .map_err(store_err)?
            else {
                return Err(not_exist(format!(
                    "no cfilter saved for block {}",
                    display(&row.hash)
                )));
            };
            out.push(BlockCFilter {
                hash: row.hash,
                key: f.key,
                filter: f.filter,
            });
            height = height.wrapping_add(1);
        }
        Ok(out)
    }

    /// `CFilterV2`: the key and filter of the block `hash`, main chain or
    /// detached.
    ///
    /// # Errors
    ///
    /// No filter: `NotExist`, Go's text. The store's.
    pub fn cfilter_v2(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<([u8; 16], Vec<u8>), AnyError> {
        match self.cfilter_row(r, hash)? {
            Some(f) => Ok((f.key, f.filter)),
            None => Err(not_exist(format!(
                "no cfilter saved for block {}",
                display(hash)
            ))),
        }
    }

    /// `ImportCFiltersV2`: records `filters` for the main-chain blocks from
    /// `start` up, each keyed by its header's merkle root.
    ///
    /// # Errors
    ///
    /// Go's: more filters than blocks: `NotExist`, `block height of <h>
    /// unknown`; a filter `gcs.FromBytesV2` refuses: its error (kind
    /// `Other`); nothing is written (Go has written the filters before the
    /// failing one, and its callers abandon the transaction). The store's.
    pub fn import_cfilters_v2(
        &self,
        tx: &mut WriteTx<'_>,
        start: i32,
        filters: &[Vec<u8>],
    ) -> Result<(), AnyError> {
        let mut blocks = match stored_height(start) {
            Some(h) => tx
                .rows::<Blocks>(
                    Bound::Included(&Height(h)),
                    Bound::Unbounded,
                    false,
                    filters.len(),
                )
                .map_err(store_err)?
                .into_iter(),
            None => Vec::new().into_iter(),
        };
        // Every filter is checked, in Go's order and with Go's errors,
        // before the first is written (Go writes each after its checks).
        let mut rows = Vec::with_capacity(filters.len());
        let mut i: i32 = 0;
        for fd in filters {
            let Some((h, row)) = blocks.next() else {
                return Err(not_exist(format!(
                    "block height of {} unknown",
                    start.wrapping_add(i)
                )));
            };
            FilterV2::from_bytes(blockcf2::B, blockcf2::M, fd)
                .map_err(|err| AnyError::new(err.to_string()))?;
            let header = header_of(tx, &row.hash)?.ok_or_else(no_header)?;
            let mut key = [0u8; 16];
            key.copy_from_slice(&header_merkle_root(&header)[..16]);
            rows.push((h.0, key));
            i = i.wrapping_add(1);
        }
        for ((height, key), fd) in rows.into_iter().zip(filters) {
            tx.put_cfilter_at(
                height,
                &CFilterRow {
                    key,
                    filter: fd.clone(),
                },
            )
            .map_err(store_err)?;
        }
        Ok(())
    }

    /// `GetMainChainBlockHashForHeight`.
    ///
    /// # Errors
    ///
    /// No main-chain block at `height`: `NotExist`, Go's text. The store's.
    pub fn get_main_chain_block_hash_for_height(
        &self,
        r: &mut (impl TxRead + ?Sized),
        height: i32,
    ) -> Result<Hash32, AnyError> {
        match stored_height(height).map_or(Ok(None), |h| block_at(r, h))? {
            Some(b) => Ok(b.hash),
            None => Err(not_exist(format!(
                "no block at height {height} in main chain"
            ))),
        }
    }

    /// `GetSerializedBlockHeader` and `GetBlockHeader`: the header of the
    /// block `hash`, main chain or detached.
    ///
    /// # Errors
    ///
    /// No header: `NotExist`, `block header`. The store's.
    pub fn get_block_header(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<HeaderBytes, AnyError> {
        header_of(r, hash)?.ok_or_else(no_header)
    }

    /// `GetBlockHeaderTime`.
    ///
    /// # Errors
    ///
    /// No header: `NotExist`, `block header`. The store's.
    pub fn get_block_header_time(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<i64, AnyError> {
        Ok(header_time(&self.get_block_header(r, hash)?))
    }

    /// `BlockInMainChain`: whether `hash` is a main-chain block, and if so
    /// whether it is stake invalidated.
    ///
    /// # Errors
    ///
    /// The store's (Go's returns no error, ADR-0012 Rule 2(c)).
    pub fn block_in_main_chain(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<(bool, bool), AnyError> {
        let Some(h) = r.get::<purse_store::BlockByHash>(hash).map_err(store_err)? else {
            return Ok((false, false));
        };
        Ok(match block_at(r, h.0)? {
            Some(b) if b.hash == *hash => (true, b.invalid),
            _ => (false, false),
        })
    }

    /// `GetBlockMetaForHash`.
    ///
    /// # Errors
    ///
    /// No header: `NotExist`, Go's text. The store's.
    pub fn get_block_meta_for_hash(
        &self,
        r: &mut (impl TxRead + ?Sized),
        hash: &Hash32,
    ) -> Result<BlockMeta, AnyError> {
        match header_of(r, hash)? {
            Some(header) => Ok(BlockMeta::from_header(hash, &header)),
            None => Err(not_exist(format!(
                "block {} header not found",
                display(hash)
            ))),
        }
    }

    /// `GetMainChainBlockHashes`: up to `count` main-chain block hashes
    /// from `start` (after it unless `inclusive`).
    ///
    /// # Errors
    ///
    /// Go's: no header for `start`: `NotExist`; `start` not in the main
    /// chain: `Invalid` (also where no main-chain block stands at the
    /// header's height, where Go panics, QK-0086). The store's.
    pub fn get_main_chain_block_hashes(
        &self,
        r: &mut (impl TxRead + ?Sized),
        start: &Hash32,
        inclusive: bool,
        count: usize,
    ) -> Result<Vec<Hash32>, AnyError> {
        let Some(header) = header_of(r, start)? else {
            return Err(not_exist(format!(
                "starting block {} not found",
                display(start)
            )));
        };
        let mut height = header.height();
        if block_at(r, height)?.map(|b| b.hash) != Some(*start) {
            return Err(invalid(format!(
                "block {} not in main chain",
                display(start)
            )));
        }
        if !inclusive {
            height = height.wrapping_add(1);
        }
        let mut out = Vec::new();
        while out.len() < count {
            let Some(b) = block_at(r, height)? else {
                break;
            };
            out.push(b.hash);
            height = height.wrapping_add(1);
        }
        Ok(out)
    }
}

/// `SetBirthState`: writes the birthday state.
///
/// # Errors
///
/// The store's.
pub fn set_birth_state(tx: &mut WriteTx<'_>, state: &BirthState) -> Result<(), AnyError> {
    tx.set_birth_state(state).map_err(store_err)
}

/// `BirthState`: the birthday state, `None` when none is set.
///
/// # Errors
///
/// The store's.
pub fn birth_state(r: &mut (impl TxRead + ?Sized)) -> Result<Option<BirthState>, AnyError> {
    r.birth_row().map_err(store_err)
}

/// `ExtractBlockHeaderParentHash`.
pub fn extract_block_header_parent_hash(header: &HeaderBytes) -> Hash32 {
    header.prev_block()
}

/// `ExtractBlockHeaderHeight` (and `RawBlockHeader.Height`).
pub fn extract_block_header_height(header: &HeaderBytes) -> i32 {
    header_height(header)
}

/// `ExtractBlockHeaderTime`.
pub fn extract_block_header_time(header: &HeaderBytes) -> i64 {
    header_time(header)
}
