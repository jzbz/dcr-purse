// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Read and write transactions: typed access to every table, raw access by
//! table id, ordered iteration and the meta rows.

use std::cell::OnceCell;
use std::marker::PhantomData;
use std::ops::Bound;

use redb::{ReadOnlyTable, ReadableTable, ReadableTableMetadata as _, TableDefinition};

use crate::codec::{Codec, DecodeError};
use crate::cursor::{ReadCursor, TypedCursor, WriteCursor};
use crate::error::{Error, Result};
use crate::keys::{Hash32, MetaKey};
use crate::schema::{TABLE_COUNT, Table, TableId};
use crate::values::{BirthState, Tip};
use crate::wallet::WalletRecord;

/// The engine's key and value type for every table: raw bytes, so that key
/// order is byte order, as in bbolt.
pub(crate) type Raw = &'static [u8];

/// The engine's definition of a table.
pub(crate) const fn definition(id: TableId) -> TableDefinition<'static, Raw, Raw> {
    TableDefinition::new(id.name())
}

/// A copied key and value.
pub type RawEntry = (Vec<u8>, Vec<u8>);

/// The longest key a table takes: bbolt's `MaxKeySize`
/// (`bbolt@v1.3.12/bucket.go:10-11`).
pub const MAX_KEY_SIZE: usize = 32768;

/// The longest value a table takes: bbolt's `MaxValueSize`, 2^31 - 2
/// (`bbolt@v1.3.12/bucket.go:13-14`).
pub const MAX_VALUE_SIZE: usize = (1 << 31) - 2;

/// bbolt's checks on a `Put`, in its order (`bbolt@v1.3.12/bucket.go:286-292`).
fn check_entry(key: &[u8], value: &[u8]) -> Result<()> {
    if key.is_empty() {
        Err(Error::invalid("key required"))
    } else if key.len() > MAX_KEY_SIZE {
        Err(Error::invalid("key too large"))
    } else {
        check_value_len(Some(value.len()))
    }
}

/// bbolt's value-size check on a `Put` alone, with its kind and text, for
/// a value of `len` bytes (`None`: a length past `usize`). An operation
/// that makes several writes calls it before the first, so that a value
/// the put would refuse is refused before anything is written.
pub(crate) fn check_value_len(len: Option<usize>) -> Result<()> {
    match len {
        Some(n) if n <= MAX_VALUE_SIZE => Ok(()),
        _ => Err(Error::invalid("value too large")),
    }
}

pub(crate) fn decode<C: Codec>(table: TableId, bytes: &[u8]) -> Result<C> {
    C::decode(bytes).map_err(|e| Error::corrupt(table, e))
}

/// The bounds of every key that starts with `prefix`.
pub(crate) fn prefix_bounds(prefix: &[u8]) -> (Bound<Vec<u8>>, Bound<Vec<u8>>) {
    // The first key after every key with the prefix: the prefix with its
    // last byte below 0xff incremented and what follows dropped.
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if let Some(next) = last.checked_add(1) {
            end.push(next);
            return (Bound::Included(prefix.to_vec()), Bound::Excluded(end));
        }
    }
    (Bound::Included(prefix.to_vec()), Bound::Unbounded)
}

fn as_slice(b: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match b {
        Bound::Included(v) => Bound::Included(v.as_slice()),
        Bound::Excluded(v) => Bound::Excluded(v.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn encode_bound<C: Codec>(b: Bound<&C>) -> Bound<Vec<u8>> {
    match b {
        Bound::Included(k) => Bound::Included(k.encode().as_ref().to_vec()),
        Bound::Excluded(k) => Bound::Excluded(k.encode().as_ref().to_vec()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// Typed entries of a table in key order, from either end.
pub struct Iter<'a, T: Table> {
    inner: redb::Range<'a, Raw, Raw>,
    _table: PhantomData<T>,
}

impl<T: Table> Iter<'_, T> {
    fn item(
        r: std::result::Result<
            (redb::AccessGuard<'_, Raw>, redb::AccessGuard<'_, Raw>),
            redb::StorageError,
        >,
    ) -> Result<(T::Key, T::Value)> {
        let (k, v) = r?;
        Ok((decode(T::ID, k.value())?, decode(T::ID, v.value())?))
    }
}

impl<T: Table> Iterator for Iter<'_, T> {
    type Item = Result<(T::Key, T::Value)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(Self::item)
    }
}

impl<T: Table> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(Self::item)
    }
}

/// Raw entries of a table in key order, from either end.
pub struct RawIter<'a> {
    inner: redb::Range<'a, Raw, Raw>,
}

fn raw_item(
    r: std::result::Result<
        (redb::AccessGuard<'_, Raw>, redb::AccessGuard<'_, Raw>),
        redb::StorageError,
    >,
) -> Result<RawEntry> {
    let (k, v) = r?;
    Ok((k.value().to_vec(), v.value().to_vec()))
}

impl Iterator for RawIter<'_> {
    type Item = Result<RawEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(raw_item)
    }
}

impl DoubleEndedIterator for RawIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(raw_item)
    }
}

// Reads shared by both transaction types, over any table the engine can
// read.

fn get_raw_in<R: ReadableTable<Raw, Raw>>(t: &R, key: &[u8]) -> Result<Option<Vec<u8>>> {
    Ok(t.get(key)?.map(|g| g.value().to_vec()))
}

fn get_in<T: Table, R: ReadableTable<Raw, Raw>>(t: &R, key: &T::Key) -> Result<Option<T::Value>> {
    match t.get(key.encode().as_ref())? {
        Some(g) => Ok(Some(decode(T::ID, g.value())?)),
        None => Ok(None),
    }
}

fn range_in<'a, R: ReadableTable<Raw, Raw>>(
    t: &'a R,
    lo: &Bound<Vec<u8>>,
    hi: &Bound<Vec<u8>>,
) -> Result<redb::Range<'a, Raw, Raw>> {
    Ok(t.range((as_slice(lo), as_slice(hi)))?)
}

fn meta_raw<R: ReadableTable<Raw, Raw>>(t: &R, key: MetaKey) -> Result<Option<Vec<u8>>> {
    get_raw_in(t, key.name())
}

pub(crate) fn meta_u32(v: &[u8], what: &'static str) -> Result<u32> {
    let a: [u8; 4] = v
        .try_into()
        .map_err(|_| Error::corrupt(TableId::Meta, DecodeError::len(what, v.len())))?;
    Ok(u32::from_be_bytes(a))
}

macro_rules! meta_readers {
    () => {
        /// The schema version (`None`: no version row).
        pub fn schema_version(&self) -> Result<Option<u32>> {
            let t = self.table(TableId::Meta)?;
            meta_raw(t, MetaKey::Version)?
                .map(|v| meta_u32(&v, "schema version"))
                .transpose()
        }

        /// The main chain tip (`None`: none recorded yet).
        pub fn tip(&self) -> Result<Option<Tip>> {
            let t = self.table(TableId::Meta)?;
            meta_raw(t, MetaKey::Tip)?
                .map(|v| decode(TableId::Meta, &v))
                .transpose()
        }

        /// The wallet's birthday state (`None`: none set).
        pub fn birth_state(&self) -> Result<Option<BirthState>> {
            let t = self.table(TableId::Meta)?;
            meta_raw(t, MetaKey::Birth)?
                .map(|v| decode(TableId::Meta, &v))
                .transpose()
        }

        /// Whether every main-chain block has its committed filter
        /// (`false` when the row is absent, as dcrwallet's
        /// `IsMissingMainChainCFilters` reads it).
        pub fn have_cfilters(&self) -> Result<bool> {
            let t = self.table(TableId::Meta)?;
            Ok(meta_raw(t, MetaKey::HaveCFilters)?
                .map(|v| decode::<bool>(TableId::Meta, &v))
                .transpose()?
                .unwrap_or(false))
        }

        /// The `wallet` row `R` (`None`: absent).
        pub fn wallet_row<R: WalletRecord>(&self) -> Result<Option<R::Value>> {
            let t = self.table(TableId::Wallet)?;
            get_raw_in(t, R::KEY.name())?
                .map(|v| decode(TableId::Wallet, &v))
                .transpose()
        }

        /// The processed-transactions block marker (`None`: unset).
        pub fn processed_marker(&self) -> Result<Option<Hash32>> {
            let t = self.table(TableId::Meta)?;
            meta_raw(t, MetaKey::ProcessedMarker)?
                .map(|v| decode(TableId::Meta, &v))
                .transpose()
        }
    };
}

macro_rules! typed_readers {
    () => {
        /// The value at `key` in table `T`.
        pub fn get<T: Table>(&self, key: &T::Key) -> Result<Option<T::Value>> {
            get_in::<T, _>(self.table(T::ID)?, key)
        }

        /// Whether table `T` has `key`.
        pub fn contains<T: Table>(&self, key: &T::Key) -> Result<bool> {
            Ok(self.table(T::ID)?.get(key.encode().as_ref())?.is_some())
        }

        /// The raw value at `key` in table `table`.
        pub fn get_raw(&self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>> {
            get_raw_in(self.table(table)?, key)
        }

        /// The number of entries in table `table`, which redb keeps in the
        /// table's root, so the count costs no walk.
        pub fn len(&self, table: TableId) -> Result<u64> {
            Ok(self.table(table)?.len()?)
        }

        /// Whether table `table` is empty.
        pub fn is_empty(&self, table: TableId) -> Result<bool> {
            Ok(self.table(table)?.is_empty()?)
        }

        /// The first entry of table `T` in key order.
        pub fn first<T: Table>(&self) -> Result<Option<(T::Key, T::Value)>> {
            match self.table(T::ID)?.first()? {
                Some((k, v)) => Ok(Some((decode(T::ID, k.value())?, decode(T::ID, v.value())?))),
                None => Ok(None),
            }
        }

        /// The last entry of table `T` in key order.
        pub fn last<T: Table>(&self) -> Result<Option<(T::Key, T::Value)>> {
            match self.table(T::ID)?.last()? {
                Some((k, v)) => Ok(Some((decode(T::ID, k.value())?, decode(T::ID, v.value())?))),
                None => Ok(None),
            }
        }
    };
}

/// A read transaction: a consistent snapshot of the store, which never
/// waits for a writer and which writers never wait for (ADR-0008,
/// "Constraints on others").
///
/// Tables are opened on first use and kept for the transaction's life. A
/// long-lived read transaction keeps the pages of its snapshot from reuse,
/// so the file grows while it lives; hold one only as long as needed.
pub struct ReadTx {
    txn: redb::ReadTransaction,
    tables: [OnceCell<ReadOnlyTable<Raw, Raw>>; TABLE_COUNT],
}

impl ReadTx {
    pub(crate) fn new(txn: redb::ReadTransaction) -> ReadTx {
        ReadTx {
            txn,
            tables: std::array::from_fn(|_| OnceCell::new()),
        }
    }

    fn table(&self, id: TableId) -> Result<&ReadOnlyTable<Raw, Raw>> {
        let cell = self
            .tables
            .get(id.index())
            .ok_or_else(|| Error::invalid("table id out of range"))?;
        if let Some(t) = cell.get() {
            return Ok(t);
        }
        let t = self.txn.open_table(definition(id))?;
        Ok(cell.get_or_init(|| t))
    }

    typed_readers!();
    meta_readers!();

    /// The number of `unspent` keys: dcrwallet's `KeyN` of `u`
    /// (`UnspentOutputCount`, `wallet/udb/txmined.go:2353`), from the
    /// table's length.
    pub fn unspent_count(&self) -> Result<u64> {
        self.len(TableId::Unspent)
    }

    /// Entries of table `T` with keys in `(lo, hi)`, in key order.
    pub fn range<T: Table>(&self, lo: Bound<&T::Key>, hi: Bound<&T::Key>) -> Result<Iter<'_, T>> {
        let (lo, hi) = (encode_bound(lo), encode_bound(hi));
        Ok(Iter {
            inner: range_in(self.table(T::ID)?, &lo, &hi)?,
            _table: PhantomData,
        })
    }

    /// Every entry of table `T`, in key order.
    pub fn iter<T: Table>(&self) -> Result<Iter<'_, T>> {
        self.range::<T>(Bound::Unbounded, Bound::Unbounded)
    }

    /// Entries of table `T` whose encoded key starts with `prefix`, in key
    /// order (for example one account's rows of `acct_history`).
    pub fn prefix<T: Table>(&self, prefix: &[u8]) -> Result<Iter<'_, T>> {
        let (lo, hi) = prefix_bounds(prefix);
        Ok(Iter {
            inner: range_in(self.table(T::ID)?, &lo, &hi)?,
            _table: PhantomData,
        })
    }

    /// Raw entries of table `table` with keys in `(lo, hi)`, in key order.
    pub fn range_raw(
        &self,
        table: TableId,
        lo: Bound<&[u8]>,
        hi: Bound<&[u8]>,
    ) -> Result<RawIter<'_>> {
        let t = self.table(table)?;
        Ok(RawIter {
            inner: t.range((lo, hi))?,
        })
    }

    /// A cursor over table `T` that decodes its entries.
    pub fn typed_cursor<T: Table>(&self) -> Result<TypedCursor<ReadCursor<'_>, T>> {
        Ok(TypedCursor::new(self.cursor(T::ID)?))
    }

    /// Raw entries of table `table` whose key starts with `prefix`, in key
    /// order.
    pub fn prefix_raw(&self, table: TableId, prefix: &[u8]) -> Result<RawIter<'_>> {
        let (lo, hi) = prefix_bounds(prefix);
        Ok(RawIter {
            inner: range_in(self.table(table)?, &lo, &hi)?,
        })
    }

    /// A cursor over table `table` with bbolt's positioning semantics
    /// (spec 04 §5.1).
    pub fn cursor(&self, table: TableId) -> Result<ReadCursor<'_>> {
        Ok(ReadCursor::new(self.table(table)?))
    }

    /// Ends the transaction, reporting an error the engine hit closing it.
    pub fn close(self) -> Result<()> {
        let ReadTx { txn, tables } = self;
        drop(tables);
        txn.close()?;
        Ok(())
    }
}

/// A write transaction, handed to the closure of
/// [`Store::update`](crate::Store::update) and committed when it returns
/// `Ok`.
///
/// Tables are opened on first use. Every write is visible to later reads
/// of the same transaction.
pub struct WriteTx<'t> {
    txn: &'t redb::WriteTransaction,
    tables: [Option<redb::Table<'t, Raw, Raw>>; TABLE_COUNT],
}

impl<'t> WriteTx<'t> {
    pub(crate) fn new(txn: &'t redb::WriteTransaction) -> WriteTx<'t> {
        WriteTx {
            txn,
            tables: std::array::from_fn(|_| None),
        }
    }

    /// Opens every table, creating those that do not exist.
    pub(crate) fn open_all(&mut self) -> Result<()> {
        for &id in TableId::ALL {
            self.table_mut(id)?;
        }
        Ok(())
    }

    pub(crate) fn table_mut(&mut self, id: TableId) -> Result<&mut redb::Table<'t, Raw, Raw>> {
        let txn = self.txn;
        let slot = self
            .tables
            .get_mut(id.index())
            .ok_or_else(|| Error::invalid("table id out of range"))?;
        if slot.is_none() {
            *slot = Some(txn.open_table(definition(id))?);
        }
        slot.as_mut()
            .ok_or_else(|| Error::invalid("table slot empty after open"))
    }

    /// Reads need the table open too; a shared borrow suffices once it is.
    fn table(&self, id: TableId) -> Result<&redb::Table<'t, Raw, Raw>> {
        self.tables
            .get(id.index())
            .and_then(Option::as_ref)
            .ok_or_else(|| Error::invalid("table not open"))
    }

    /// Opens `id` so that the shared-borrow readers can use it.
    fn ensure(&mut self, id: TableId) -> Result<&Self> {
        self.table_mut(id)?;
        Ok(self)
    }

    /// The value at `key` in table `T`.
    pub fn get<T: Table>(&mut self, key: &T::Key) -> Result<Option<T::Value>> {
        self.ensure(T::ID)?.reader().get::<T>(key)
    }

    /// Whether table `T` has `key`.
    pub fn contains<T: Table>(&mut self, key: &T::Key) -> Result<bool> {
        self.ensure(T::ID)?.reader().contains::<T>(key)
    }

    /// The raw value at `key` in table `table`.
    pub fn get_raw(&mut self, table: TableId, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.ensure(table)?.reader().get_raw(table, key)
    }

    /// The number of entries in table `table`, counting this transaction's
    /// writes. (bbolt's `KeyN` in a write transaction does not count them,
    /// spec 04 §5.1 item 6, QK-0003; dcrwallet's one call in a write
    /// transaction, `txToMultisig`'s, comes before anything there writes
    /// to `unspent`, so the counts agree: docs/PARITY.md, `purse-store`.)
    pub fn len(&mut self, table: TableId) -> Result<u64> {
        self.ensure(table)?.reader().len(table)
    }

    /// Whether table `table` is empty.
    pub fn is_empty(&mut self, table: TableId) -> Result<bool> {
        self.ensure(table)?.reader().is_empty(table)
    }

    /// The first entry of table `T` in key order.
    pub fn first<T: Table>(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        self.ensure(T::ID)?.reader().first::<T>()
    }

    /// The last entry of table `T` in key order.
    pub fn last<T: Table>(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        self.ensure(T::ID)?.reader().last::<T>()
    }

    /// The schema version (`None`: no version row).
    pub fn schema_version(&mut self) -> Result<Option<u32>> {
        self.ensure(TableId::Meta)?.reader().schema_version()
    }

    /// The main chain tip.
    pub fn tip(&mut self) -> Result<Option<Tip>> {
        self.ensure(TableId::Meta)?.reader().tip()
    }

    /// The number of `unspent` keys, counting this transaction's writes
    /// (as [`WriteTx::len`]).
    pub fn unspent_count(&mut self) -> Result<u64> {
        self.len(TableId::Unspent)
    }

    /// The wallet's birthday state.
    pub fn birth_state(&mut self) -> Result<Option<BirthState>> {
        self.ensure(TableId::Meta)?.reader().birth_state()
    }

    /// Whether every main-chain block has its committed filter.
    pub fn have_cfilters(&mut self) -> Result<bool> {
        self.ensure(TableId::Meta)?.reader().have_cfilters()
    }

    /// The `wallet` row `R`.
    pub fn wallet_row<R: WalletRecord>(&mut self) -> Result<Option<R::Value>> {
        self.ensure(TableId::Wallet)?.reader().wallet_row::<R>()
    }

    /// Writes the `wallet` row `R`.
    pub fn put_wallet_row<R: WalletRecord>(&mut self, value: &R::Value) -> Result<()> {
        let v = value.encode();
        self.put_raw(TableId::Wallet, R::KEY.name(), v.as_ref())
    }

    /// Removes the `wallet` row `R`; whether it was there.
    pub fn delete_wallet_row<R: WalletRecord>(&mut self) -> Result<bool> {
        self.delete_raw(TableId::Wallet, R::KEY.name())
    }

    /// The processed-transactions block marker.
    pub fn processed_marker(&mut self) -> Result<Option<Hash32>> {
        self.ensure(TableId::Meta)?.reader().processed_marker()
    }

    fn reader(&self) -> WriteReader<'_, 't> {
        WriteReader(self)
    }

    /// Writes `value` at `key` in table `T`, replacing what was there.
    ///
    /// bbolt's limits apply, with dcrwallet's kind and text: an empty key
    /// is refused as `invalid operation: key required`, a key longer than
    /// [`MAX_KEY_SIZE`] as `invalid operation: key too large`, and a value
    /// longer than [`MAX_VALUE_SIZE`] as `invalid operation: value too
    /// large` (`bbolt@v1.3.12/bucket.go:281-313`, through `convertErr`,
    /// spec 01 §5.8). Nothing is written then, and the transaction stays
    /// usable.
    pub fn put<T: Table>(&mut self, key: &T::Key, value: &T::Value) -> Result<()> {
        let k = key.encode();
        let v = value.encode();
        self.put_raw(T::ID, k.as_ref(), v.as_ref())
    }

    /// Removes `key` from table `T`; whether it was there.
    pub fn delete<T: Table>(&mut self, key: &T::Key) -> Result<bool> {
        let k = key.encode();
        Ok(self.table_mut(T::ID)?.remove(k.as_ref())?.is_some())
    }

    /// Removes `key` from table `T`, returning the value it held.
    pub(crate) fn take<T: Table>(&mut self, key: &T::Key) -> Result<Option<T::Value>> {
        let k = key.encode();
        match self.table_mut(T::ID)?.remove(k.as_ref())? {
            Some(g) => Ok(Some(decode(T::ID, g.value())?)),
            None => Ok(None),
        }
    }

    /// Writes a raw entry. The bytes are not checked against the table's
    /// types; this is for migrations and tests. bbolt's limits apply as in
    /// [`WriteTx::put`].
    pub fn put_raw(&mut self, table: TableId, key: &[u8], value: &[u8]) -> Result<()> {
        check_entry(key, value)?;
        self.table_mut(table)?.insert(key, value)?;
        Ok(())
    }

    /// Removes a raw key; whether it was there.
    pub fn delete_raw(&mut self, table: TableId, key: &[u8]) -> Result<bool> {
        Ok(self.table_mut(table)?.remove(key)?.is_some())
    }

    /// Entries of table `T` with keys in `(lo, hi)`, in key order. The
    /// iterator borrows the transaction; collect what a loop will change.
    pub fn range<T: Table>(
        &mut self,
        lo: Bound<&T::Key>,
        hi: Bound<&T::Key>,
    ) -> Result<Iter<'_, T>> {
        let (lo, hi) = (encode_bound(lo), encode_bound(hi));
        let t = self.table_mut(T::ID)?;
        Ok(Iter {
            inner: range_in(&*t, &lo, &hi)?,
            _table: PhantomData,
        })
    }

    /// Every entry of table `T`, in key order.
    pub fn iter<T: Table>(&mut self) -> Result<Iter<'_, T>> {
        self.range::<T>(Bound::Unbounded, Bound::Unbounded)
    }

    /// Entries of table `T` whose encoded key starts with `prefix`.
    pub fn prefix<T: Table>(&mut self, prefix: &[u8]) -> Result<Iter<'_, T>> {
        let (lo, hi) = prefix_bounds(prefix);
        let t = self.table_mut(T::ID)?;
        Ok(Iter {
            inner: range_in(&*t, &lo, &hi)?,
            _table: PhantomData,
        })
    }

    /// Raw entries of table `table` with keys in `(lo, hi)`, in key order.
    pub fn range_raw(
        &mut self,
        table: TableId,
        lo: Bound<&[u8]>,
        hi: Bound<&[u8]>,
    ) -> Result<RawIter<'_>> {
        let t = self.table_mut(table)?;
        Ok(RawIter {
            inner: t.range((lo, hi))?,
        })
    }

    /// Raw entries of table `table` whose key starts with `prefix`, in key
    /// order.
    pub fn prefix_raw(&mut self, table: TableId, prefix: &[u8]) -> Result<RawIter<'_>> {
        let (lo, hi) = prefix_bounds(prefix);
        let t = self.table_mut(table)?;
        Ok(RawIter {
            inner: range_in(&*t, &lo, &hi)?,
        })
    }

    /// A cursor over table `table` with bbolt's positioning semantics that
    /// may delete the entry it is on and keep moving, and between whose
    /// moves the transaction may be written ([`WriteCursor::tx`]).
    pub fn cursor(&mut self, table: TableId) -> Result<WriteCursor<'_, 't>> {
        self.table_mut(table)?;
        Ok(WriteCursor::new(self, table))
    }

    /// A cursor over table `T` that decodes its entries; its raw
    /// [`WriteCursor`] deletes and reaches the transaction.
    pub fn typed_cursor<T: Table>(&mut self) -> Result<TypedCursor<WriteCursor<'_, 't>, T>> {
        Ok(TypedCursor::new(self.cursor(T::ID)?))
    }

    /// Sets the schema version. Only the store's create and migration paths
    /// call it.
    pub(crate) fn set_schema_version(&mut self, v: u32) -> Result<()> {
        self.put_raw(TableId::Meta, MetaKey::Version.name(), &v.to_be_bytes())
    }

    /// Records the main chain tip. The chain methods ([`crate::chain`])
    /// keep this row equal to the last main-chain block; writing it
    /// directly is for migrations and tests.
    pub fn set_tip(&mut self, tip: &Tip) -> Result<()> {
        self.put_raw(TableId::Meta, MetaKey::Tip.name(), &tip.encode())
    }

    /// Records the wallet's birthday state.
    pub fn set_birth_state(&mut self, state: &BirthState) -> Result<()> {
        self.put_raw(TableId::Meta, MetaKey::Birth.name(), &state.encode())
    }

    /// Records whether every main-chain block has its committed filter.
    pub fn set_have_cfilters(&mut self, have: bool) -> Result<()> {
        self.put_raw(TableId::Meta, MetaKey::HaveCFilters.name(), &have.encode())
    }

    /// Records the processed-transactions block marker.
    pub fn set_processed_marker(&mut self, hash: &Hash32) -> Result<()> {
        self.put_raw(TableId::Meta, MetaKey::ProcessedMarker.name(), &hash.0)
    }
}

/// Shared-borrow reads over a write transaction's open tables.
struct WriteReader<'a, 't>(&'a WriteTx<'t>);

impl<'a, 't> WriteReader<'a, 't> {
    fn table(&self, id: TableId) -> Result<&'a redb::Table<'t, Raw, Raw>> {
        self.0.table(id)
    }

    typed_readers!();
    meta_readers!();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value check alone draws the line where [`WriteTx::put`] does,
    /// with its text, so a chain operation's early refusal matches the
    /// put's.
    #[test]
    fn the_value_check_alone_is_the_puts() {
        assert!(check_value_len(Some(0)).is_ok());
        assert!(check_value_len(Some(MAX_VALUE_SIZE)).is_ok());
        for len in [MAX_VALUE_SIZE.checked_add(1), Some(usize::MAX), None] {
            let e = check_value_len(len).unwrap_err();
            assert_eq!(e.to_string(), "invalid operation: value too large");
        }
        assert_eq!(
            check_entry(b"k", &[]).map_err(|e| e.to_string()),
            check_value_len(Some(0)).map_err(|e| e.to_string())
        );
    }
}
