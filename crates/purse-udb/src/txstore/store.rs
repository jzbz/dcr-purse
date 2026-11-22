// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The [`TxStore`] value, the read access its queries take ([`TxRead`]),
//! store creation (`createStore` and the rows of upgrades 10, 11, 15 and
//! 26) and the guard of the later slices' tables.

use std::ops::Bound;

use dcroxide_gcs::blockcf2::{self, PrevScripter};
use dcroxide_wire::OutPoint;
use purse_errors::{AnyError, Kind, e};
use purse_store::{
    BirthState, BlockRow, CFilterRow, ChainBlock, Hash32, HeaderBytes, ReadTx, Table, TableId, Tip,
    WriteTx,
};

use super::record::{ZERO_TIME, header_time, header_vote_bits};
use crate::addrmgr::ChainParams;

/// Read access to the transaction store's rows: a read transaction or a
/// write transaction (the address manager's `StoreRead` pattern, widened to
/// the ranges and chain markers the queries read).
pub trait TxRead {
    /// The value at `key` of table `T`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>>;

    /// Whether table `T` has `key`, its value neither copied nor decoded
    /// (Go's `Get(k) != nil`, which copies nothing either).
    ///
    /// # Errors
    ///
    /// The store's.
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool>;

    /// At most `limit` rows of table `T` with keys between `lo` and `hi`,
    /// in key order, or from the high end when `rev`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>>;

    /// At most `limit` keys of table `T` between `lo` and `hi`, in key
    /// order, or from the high end when `rev`, the values neither copied
    /// nor decoded (a Go cursor loop that reads only `k`).
    ///
    /// # Errors
    ///
    /// The store's.
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>>;

    /// Whether `table` holds no row.
    ///
    /// # Errors
    ///
    /// The store's.
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool>;

    /// The number of rows of `table`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64>;

    /// The `tip` meta row.
    ///
    /// # Errors
    ///
    /// The store's.
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>>;

    /// The `birth` meta row.
    ///
    /// # Errors
    ///
    /// The store's.
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>>;

    /// The `have_cfilters` meta row (absent reads as false).
    ///
    /// # Errors
    ///
    /// The store's.
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool>;

    /// The `processed` meta row.
    ///
    /// # Errors
    ///
    /// The store's.
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>>;
}

/// The first `limit` items of `it`, from the back when `rev`.
fn take<I>(
    it: impl DoubleEndedIterator<Item = purse_store::Result<I>>,
    rev: bool,
    limit: usize,
) -> purse_store::Result<Vec<I>> {
    if rev {
        it.rev().take(limit).collect()
    } else {
        it.take(limit).collect()
    }
}

/// The size of the first page [`walk`] reads.
const WALK_FIRST_PAGE: usize = 1;

/// The largest page [`walk`] reads: pages double from [`WALK_FIRST_PAGE`]
/// up to this size.
const WALK_LAST_PAGE: usize = 512;

/// Calls `f` with each row of table `T` between `lo` and `hi`, in key
/// order, or from the high end when `rev`, until `f` returns `true`; the
/// result says whether it did.
///
/// This is the cursor walk of Go's block iterator: the rows are read a
/// page at a time, the pages doubling from one row to [`WALK_LAST_PAGE`],
/// so a walk that `f` stops after visiting `n` rows has read at most
/// `max(2n - 1, n + 511)` rows, whatever the range holds, where Go's cursor
/// reads `n`; a full walk of `n` rows makes about `n / 512` range lookups
/// more than Go's one. `f` gets the reader back, since a page is read
/// before it is visited, so it can read other rows, as Go's loops do
/// between cursor steps (an iterator held across `f` cannot:
/// [`purse_store::WriteTx`]'s range borrows the transaction mutably).
///
/// # Errors
///
/// `f`'s. The store's.
pub(crate) fn walk<T, R, F>(
    r: &mut R,
    lo: Bound<T::Key>,
    hi: Bound<T::Key>,
    rev: bool,
    mut f: F,
) -> Result<bool, AnyError>
where
    T: Table,
    T::Key: Clone,
    R: TxRead + ?Sized,
    F: FnMut(&mut R, T::Key, T::Value) -> Result<bool, AnyError>,
{
    pages(
        r,
        lo,
        hi,
        rev,
        |r, lo, hi, n| r.rows::<T>(lo, hi, rev, n),
        |(k, _)| k.clone(),
        |r, (k, v)| f(r, k, v),
    )
}

/// [`walk`] over the keys alone ([`TxRead::keys`]): `f` gets each key of
/// table `T` between `lo` and `hi`, and no value is copied or decoded, as
/// in Go's cursor loops that read only `k` (`MissingCFiltersHeight` reads
/// the block record's value only for the hash, which the height keying
/// the filters here makes unneeded).
///
/// # Errors
///
/// `f`'s. The store's.
pub(crate) fn walk_keys<T, R, F>(
    r: &mut R,
    lo: Bound<T::Key>,
    hi: Bound<T::Key>,
    rev: bool,
    f: F,
) -> Result<bool, AnyError>
where
    T: Table,
    T::Key: Clone,
    R: TxRead + ?Sized,
    F: FnMut(&mut R, T::Key) -> Result<bool, AnyError>,
{
    pages(
        r,
        lo,
        hi,
        rev,
        |r, lo, hi, n| r.keys::<T>(lo, hi, rev, n),
        T::Key::clone,
        f,
    )
}

/// The paging of [`walk`] and [`walk_keys`]: `read` gets the reader, the
/// bounds left and a page size, `key` the key of an item, so the next page
/// starts past the last.
fn pages<K, I, R>(
    r: &mut R,
    lo: Bound<K>,
    hi: Bound<K>,
    rev: bool,
    mut read: impl FnMut(&mut R, Bound<&K>, Bound<&K>, usize) -> purse_store::Result<Vec<I>>,
    key: impl Fn(&I) -> K,
    mut f: impl FnMut(&mut R, I) -> Result<bool, AnyError>,
) -> Result<bool, AnyError>
where
    R: ?Sized,
{
    let (mut lo, mut hi) = (lo, hi);
    let mut page = WALK_FIRST_PAGE;
    loop {
        let items = read(r, lo.as_ref(), hi.as_ref(), page).map_err(store_err)?;
        let full = items.len() == page;
        let mut last = None;
        for item in items {
            last = Some(key(&item));
            if f(r, item)? {
                return Ok(true);
            }
        }
        match last {
            Some(k) if full => {
                if rev {
                    hi = Bound::Excluded(k);
                } else {
                    lo = Bound::Excluded(k);
                }
            }
            _ => return Ok(false),
        }
        page = page.saturating_mul(2).min(WALK_LAST_PAGE);
    }
}

impl TxRead for ReadTx {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        ReadTx::get::<T>(self, key)
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        ReadTx::contains::<T>(self, key)
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        take(ReadTx::range::<T>(self, lo, hi)?, rev, limit)
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        take(ReadTx::keys::<T>(self, lo, hi)?, rev, limit)
    }
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool> {
        ReadTx::is_empty(self, table)
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        ReadTx::len(self, table)
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        ReadTx::tip(self)
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        ReadTx::birth_state(self)
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        ReadTx::have_cfilters(self)
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        ReadTx::processed_marker(self)
    }
}

impl TxRead for &ReadTx {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        ReadTx::get::<T>(self, key)
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        ReadTx::contains::<T>(self, key)
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        take(ReadTx::range::<T>(self, lo, hi)?, rev, limit)
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        take(ReadTx::keys::<T>(self, lo, hi)?, rev, limit)
    }
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool> {
        ReadTx::is_empty(self, table)
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        ReadTx::len(self, table)
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        ReadTx::tip(self)
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        ReadTx::birth_state(self)
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        ReadTx::have_cfilters(self)
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        ReadTx::processed_marker(self)
    }
}

impl TxRead for WriteTx<'_> {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        WriteTx::get::<T>(self, key)
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        WriteTx::contains::<T>(self, key)
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        take(WriteTx::range::<T>(self, lo, hi)?, rev, limit)
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        take(WriteTx::keys::<T>(self, lo, hi)?, rev, limit)
    }
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool> {
        WriteTx::is_empty(self, table)
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        WriteTx::len(self, table)
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        WriteTx::tip(self)
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        WriteTx::birth_state(self)
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        WriteTx::have_cfilters(self)
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        WriteTx::processed_marker(self)
    }
}

impl<S: TxRead + ?Sized> TxRead for &mut S {
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        (**self).get::<T>(key)
    }
    fn contains<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<bool> {
        (**self).contains::<T>(key)
    }
    fn rows<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        (**self).rows::<T>(lo, hi, rev, limit)
    }
    fn keys<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
        rev: bool,
        limit: usize,
    ) -> purse_store::Result<Vec<T::Key>> {
        (**self).keys::<T>(lo, hi, rev, limit)
    }
    fn table_is_empty(&mut self, table: TableId) -> purse_store::Result<bool> {
        (**self).table_is_empty(table)
    }
    fn table_len(&mut self, table: TableId) -> purse_store::Result<u64> {
        (**self).table_len(table)
    }
    fn tip_row(&mut self) -> purse_store::Result<Option<Tip>> {
        (**self).tip_row()
    }
    fn birth_row(&mut self) -> purse_store::Result<Option<BirthState>> {
        (**self).birth_row()
    }
    fn have_cfilters_row(&mut self) -> purse_store::Result<bool> {
        (**self).have_cfilters_row()
    }
    fn processed_row(&mut self) -> purse_store::Result<Option<Hash32>> {
        (**self).processed_row()
    }
}

/// The tables the later slices fill (module documentation, "Credit-dependent
/// steps"). No operation of this slice writes them.
pub const LATER_TABLES: [TableId; 13] = [
    TableId::Credits,
    TableId::CreditsInvalidated,
    TableId::Unspent,
    TableId::UnminedCredits,
    TableId::Debits,
    TableId::DebitsInvalidated,
    TableId::Commitments,
    TableId::CommitmentsUnspent,
    TableId::MultisigOuts,
    TableId::MultisigUnspent,
    TableId::Balances,
    TableId::BalanceHeights,
    TableId::AcctHistory,
];

/// The guard of the credit-dependent steps: an error of kind `Bug` naming
/// `step` when any of `tables` holds a row, which only a write outside
/// this module can have put there. One emptiness check per table, where
/// Go makes one lookup per input or output; both find nothing in every
/// store the first slice writes.
pub(crate) fn guard(
    r: &mut (impl TxRead + ?Sized),
    step: &str,
    tables: &[TableId],
) -> Result<(), AnyError> {
    for t in tables {
        if !r.table_is_empty(*t).map_err(store_err)? {
            return Err(e!(
                Kind::Bug,
                format!(
                    "{step}: table {} holds rows, which this version of purse-udb does not maintain",
                    t.name()
                )
            )
            .into());
        }
    }
    Ok(())
}

/// A store error in the wallet-wide model (the address manager's
/// `store_err`).
pub(crate) fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

/// What Go's `Store` holds that the first slice reads: the network's
/// genesis block, its header and its committed filter.
#[derive(Clone, Debug)]
pub struct TxStore {
    genesis_hash: Hash32,
    genesis_header: HeaderBytes,
    genesis_filter: CFilterRow,
}

/// `genesisPrevScripter` (`upgrades.go:1271-1277`): the genesis block
/// spends no previous output.
struct NoPrevScripts;

impl PrevScripter for NoPrevScripts {
    fn prev_script(&self, _out: &OutPoint) -> Option<(u16, &[u8])> {
        None
    }
}

impl TxStore {
    /// The store of `chain`'s network, as dcrwallet's `Open` makes the
    /// `Store` value.
    ///
    /// # Errors
    ///
    /// The network is not one of the four, or the genesis block's filter
    /// cannot be built (never, for the four networks).
    pub fn new(chain: &ChainParams) -> Result<TxStore, AnyError> {
        let params = match chain.name {
            "mainnet" => dcroxide_chaincfg::mainnet_params(),
            "testnet3" => dcroxide_chaincfg::testnet3_params(),
            "simnet" => dcroxide_chaincfg::simnet_params(),
            "regnet" => dcroxide_chaincfg::regnet_params(),
            other => {
                return Err(e!(Kind::Invalid, format!("unknown network {other:?}")).into());
            }
        };
        let block = &params.genesis_block;
        let filter = blockcf2::regular(block, &NoPrevScripts)
            .map_err(|err| AnyError::new(format!("genesis block filter: {err:?}")))?;
        Ok(TxStore {
            genesis_hash: Hash32(*params.genesis_hash.as_bytes()),
            genesis_header: HeaderBytes(block.header.serialize()),
            genesis_filter: CFilterRow {
                key: blockcf2::key(&block.header.merkle_root),
                filter: filter.bytes().to_vec(),
            },
        })
    }

    /// The network's genesis block hash.
    pub fn genesis_hash(&self) -> &Hash32 {
        &self.genesis_hash
    }

    /// Creates the transaction store's rows in `tx`, in the final form
    /// dcrwallet's `createStore` and its upgrades 10, 11, 15 and 26 leave
    /// them: the genesis block as the main chain (its row, header and
    /// committed filter, and the tip), the processed-transactions marker
    /// at the genesis block, `have_cfilters` set, and the birthday at the
    /// genesis block with no flag and Go's zero time (QK-0114). The
    /// write-only creation date and the mined balance are not kept
    /// (ADR-0003, addendum "Schema version 1 as built").
    ///
    /// # Errors
    ///
    /// A main chain is already recorded: kind `IO`, Go's text for a
    /// namespace that is not empty. The store's.
    pub fn create(&self, tx: &mut WriteTx<'_>) -> Result<(), AnyError> {
        if tx.tip().map_err(store_err)?.is_some() {
            return Err(e!(Kind::Io, "bucket is not empty").into());
        }
        let row = BlockRow {
            hash: self.genesis_hash,
            time: header_time(&self.genesis_header),
            vote_bits: header_vote_bits(&self.genesis_header),
            invalid: false,
            wallet_txs: 0,
        };
        tx.connect_block(
            0,
            ChainBlock {
                row: &row,
                header: &self.genesis_header,
                filter: Some(&self.genesis_filter),
            },
        )
        .map_err(store_err)?;
        tx.set_processed_marker(&self.genesis_hash)
            .map_err(store_err)?;
        tx.set_have_cfilters(true).map_err(store_err)?;
        tx.set_birth_state(&BirthState {
            set_from_height: false,
            set_from_time: false,
            hash: self.genesis_hash,
            height: 0,
            time: ZERO_TIME,
        })
        .map_err(store_err)?;
        Ok(())
    }
}
