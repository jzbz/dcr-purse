// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Cursors with bbolt v1.3.12's positioning semantics (spec 04 §5.1;
//! spec 01 §5.7), on which dcrwallet's iteration orders and the
//! distribution of its random input selection depend.
//!
//! What a client can observe and the cursors reproduce:
//!
//! - `seek(k)` returns the first entry with key `>= k`, or `None`;
//! - `prev` right after a `seek` that returned `None` returns the last
//!   entry ("previous of end is last");
//! - `next` from the last entry and `prev` from the first return `None`,
//!   and bbolt leaves its cursor on that last (first) entry, so a step
//!   back the other way returns its neighbour, not the entry itself;
//! - `next` and `prev` on a cursor never positioned return `None`.
//!
//! [`ReadCursor`] runs on redb's gap cursor ([`redb::ReadableTable::lower_bound`])
//! and a mode that records where bbolt's cursor would stand. [`WriteCursor`]
//! keeps the key it stands on and seeks again on every move, so that the
//! transaction can be written between moves (redb does not allow a table to
//! change under a live engine cursor).
//!
//! What a [`WriteCursor`] returns when the transaction is written between
//! its moves is documented on the type.

use std::marker::PhantomData;
use std::ops::Bound;

use redb::{ReadOnlyTable, ReadableTable};

use crate::codec::Codec as _;
use crate::error::Result;
use crate::schema::{Table, TableId};
use crate::txn::{Raw, RawEntry, WriteTx, decode};

/// Where bbolt's cursor stands, relative to redb's gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Never positioned.
    Unpositioned,
    /// On an entry; the gap is just after it.
    After,
    /// On an entry; the gap is just before it.
    Before,
    /// `seek` (or `first`/`last` on an empty table) found nothing; the gap
    /// is at the end.
    EndSeek,
    /// `next` ran off the end: bbolt stays on the last entry; the gap is at
    /// the end.
    EndNext,
    /// `prev` ran off the start: bbolt stays on the first entry; the gap is
    /// at the start.
    Start,
}

fn copy(e: Option<(redb::AccessGuard<'_, Raw>, redb::AccessGuard<'_, Raw>)>) -> Option<RawEntry> {
    e.map(|(k, v)| (k.value().to_vec(), v.value().to_vec()))
}

/// A read-only cursor over one table of a [`ReadTx`](crate::ReadTx).
pub struct ReadCursor<'a> {
    table: &'a ReadOnlyTable<Raw, Raw>,
    c: Option<redb::Cursor<'a, Raw, Raw>>,
    mode: Mode,
}

impl<'a> ReadCursor<'a> {
    pub(crate) fn new(table: &'a ReadOnlyTable<Raw, Raw>) -> Self {
        ReadCursor {
            table,
            c: None,
            mode: Mode::Unpositioned,
        }
    }

    fn landed(
        &mut self,
        c: redb::Cursor<'a, Raw, Raw>,
        e: Option<RawEntry>,
        on: Mode,
    ) -> Option<RawEntry> {
        self.mode = if e.is_some() { on } else { Mode::EndSeek };
        self.c = Some(c);
        e
    }

    /// `First`: the first entry.
    pub fn first(&mut self) -> Result<Option<RawEntry>> {
        let mut c = self.table.lower_bound(Bound::<&[u8]>::Unbounded)?;
        let e = copy(c.next()?);
        Ok(self.landed(c, e, Mode::After))
    }

    /// `Last`: the last entry.
    pub fn last(&mut self) -> Result<Option<RawEntry>> {
        let mut c = self.table.upper_bound(Bound::<&[u8]>::Unbounded)?;
        let e = copy(c.prev()?);
        Ok(self.landed(c, e, Mode::Before))
    }

    /// `Seek`: the first entry with key `>= key`.
    pub fn seek(&mut self, key: &[u8]) -> Result<Option<RawEntry>> {
        let mut c = self.table.lower_bound(Bound::Included(key))?;
        let e = copy(c.next()?);
        Ok(self.landed(c, e, Mode::After))
    }

    /// `Next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<RawEntry>> {
        let Some(c) = self.c.as_mut() else {
            return Ok(None);
        };
        let e = match self.mode {
            Mode::After => copy(c.next()?),
            Mode::Before => {
                c.next()?;
                copy(c.next()?)
            }
            Mode::Start => {
                if c.next()?.is_none() {
                    return Ok(None);
                }
                copy(c.next()?)
            }
            Mode::EndSeek | Mode::EndNext | Mode::Unpositioned => return Ok(None),
        };
        self.mode = if e.is_some() {
            Mode::After
        } else {
            Mode::EndNext
        };
        Ok(e)
    }

    /// `Prev`.
    pub fn prev(&mut self) -> Result<Option<RawEntry>> {
        let Some(c) = self.c.as_mut() else {
            return Ok(None);
        };
        let e = match self.mode {
            Mode::After | Mode::EndNext => {
                if c.prev()?.is_none() {
                    self.mode = Mode::Start;
                    return Ok(None);
                }
                copy(c.prev()?)
            }
            Mode::Before | Mode::EndSeek => copy(c.prev()?),
            Mode::Start | Mode::Unpositioned => return Ok(None),
        };
        self.mode = if e.is_some() {
            Mode::Before
        } else {
            Mode::Start
        };
        Ok(e)
    }
}

/// Where a [`WriteCursor`] stands.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pos {
    Unpositioned,
    /// On the entry with this key (which may since have been deleted).
    On(Vec<u8>),
    /// `seek` found nothing.
    EndSeek,
    /// `next` ran off the end, from the entry with this key, which the
    /// cursor still stands on (bbolt stays on the last entry).
    EndNext(Vec<u8>),
    /// `prev` ran off the start; the cursor stands on the entry with this
    /// key, the table's first when it did (bbolt repositions on the first
    /// entry).
    Start(Vec<u8>),
    /// `prev` ran off the start of an empty table: the cursor stands on
    /// nothing. `next` returns `None` and `delete` removes nothing; `prev`
    /// runs off the start again, repositioning on the table's first entry
    /// as it then is.
    Nowhere,
}

/// A cursor over one table of a [`WriteTx`] that can delete the entry it
/// stands on, and between whose moves the transaction can be written
/// through [`WriteCursor::tx`].
///
/// # Writes between moves
///
/// A write cursor reads its table as it is at each move. `first`, `last`
/// and `seek` answer as on a fresh cursor. `next` returns the first entry
/// whose key is above the key the cursor stands on and `prev` the last
/// entry below it, whether or not that key is still in the table. After
/// `next` ran off the end from key `k` the cursor still stands on `k`, so a
/// later `next` returns an entry inserted above `k` since. Every `prev` that
/// finds nothing has run off the start, and the cursor then stands on the
/// table's first entry as the table is at that `prev` (bbolt's `Prev`
/// repositions there with a fresh descent each time), or on nothing if the
/// table is empty; a further `prev` returns `None` and repositions again,
/// on what is the first entry by then. A cursor standing on nothing
/// returns `None` from `next`. `delete` removes the entry the cursor
/// stands on. After a `seek` that found nothing, `prev` returns the last
/// entry. So an entry deleted behind or under the cursor is never returned
/// again, an entry inserted ahead of it is returned when the cursor
/// reaches it, and no entry is skipped or repeated.
///
/// With no write to the cursor's table since its last `first`, `last` or
/// `seek`, or since a `prev` that ran off the start while no leaf of the
/// table had been emptied in the transaction (bbolt's `Prev` then calls its
/// `first`, a fresh descent, so it positions the cursor as `first` does),
/// this is bbolt's cursor, and writes to other tables never change it;
/// `tests/conformance.rs` holds both against bbolt itself through the
/// `boltcursor` oracle. Two exceptions remain even then, both in a
/// transaction that deleted from the table: bbolt's `Prev` into a leaf
/// emptied earlier in the transaction returns `None` (QK-0747), and after
/// its `Next` ran off the end over emptied leaves bbolt's cursor stands on
/// the last, empty leaf, so its `Delete` removes nothing and its `Prev`
/// returns the entry it ran off from again (QK-0749); this cursor stays on
/// the entry it ran off from, deletes it, and steps below it. After a write
/// to the cursor's own table bbolt's answer depends on its page layout: a
/// cursor that reached a leaf before the leaf was first written in the
/// transaction keeps reading the leaf's committed copy, and one that
/// reached it afterwards reads the live copy, where a delete at or before
/// its index makes the next `Next` skip an entry and an insert before it
/// makes it repeat one (spec 01 §10 Q9, bolt#620; QK-0004). bbolt's `Last`
/// on a table of several leaves emptied in the transaction never returns
/// (QK-0750); this cursor's returns `None`. dcrwallet relies on none of this:
/// it never calls a cursor's `Delete`, collects keys before it changes the
/// bucket it walks, and where it does not (upgrade 17, QK-0119) overwrites
/// only the entry it stands on, where bbolt and this rule agree. The rule
/// is the store's deliberate divergence (docs/PARITY.md).
pub struct WriteCursor<'a, 't> {
    tx: &'a mut WriteTx<'t>,
    table: TableId,
    pos: Pos,
}

impl<'a, 't> WriteCursor<'a, 't> {
    pub(crate) fn new(tx: &'a mut WriteTx<'t>, table: TableId) -> Self {
        WriteCursor {
            tx,
            table,
            pos: Pos::Unpositioned,
        }
    }

    /// The first entry after `lo`.
    fn after(&mut self, lo: Bound<&[u8]>) -> Result<Option<RawEntry>> {
        let t = self.tx.table_mut(self.table)?;
        let mut c = t.lower_bound(lo)?;
        Ok(copy(c.next()?))
    }

    /// The last entry before `hi`.
    fn before(&mut self, hi: Bound<&[u8]>) -> Result<Option<RawEntry>> {
        let t = self.tx.table_mut(self.table)?;
        let mut c = t.upper_bound(hi)?;
        Ok(copy(c.prev()?))
    }

    fn land(&mut self, e: Option<RawEntry>, none: Pos) -> Option<RawEntry> {
        self.pos = match &e {
            Some((k, _)) => Pos::On(k.clone()),
            None => none,
        };
        e
    }

    /// `First`.
    pub fn first(&mut self) -> Result<Option<RawEntry>> {
        let e = self.after(Bound::Unbounded)?;
        Ok(self.land(e, Pos::EndSeek))
    }

    /// `Last`.
    pub fn last(&mut self) -> Result<Option<RawEntry>> {
        let e = self.before(Bound::Unbounded)?;
        Ok(self.land(e, Pos::EndSeek))
    }

    /// `Seek`: the first entry with key `>= key`.
    pub fn seek(&mut self, key: &[u8]) -> Result<Option<RawEntry>> {
        let e = self.after(Bound::Included(key))?;
        Ok(self.land(e, Pos::EndSeek))
    }

    /// `Next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<RawEntry>> {
        let from = match &self.pos {
            // From `EndNext(k)` too: the cursor still stands on `k`, and an
            // entry put above it since is the next one.
            Pos::On(k) | Pos::Start(k) | Pos::EndNext(k) => k.clone(),
            Pos::Unpositioned | Pos::EndSeek | Pos::Nowhere => return Ok(None),
        };
        let e = self.after(Bound::Excluded(&from))?;
        Ok(self.land(e, Pos::EndNext(from)))
    }

    /// `Prev`.
    pub fn prev(&mut self) -> Result<Option<RawEntry>> {
        let e = match &self.pos {
            Pos::Unpositioned => return Ok(None),
            Pos::On(k) | Pos::EndNext(k) => {
                let from = k.clone();
                self.before(Bound::Excluded(&from))?
            }
            Pos::EndSeek => self.before(Bound::Unbounded)?,
            // Already off the start: bbolt's cursor stands at index 0 of
            // every level, so its `Prev` runs off the start again.
            Pos::Start(_) | Pos::Nowhere => None,
        };
        if e.is_none() {
            // Off the start: bbolt's `Prev` calls its `first`, a fresh
            // descent, and stands on the first entry as the table now is
            // (bbolt@v1.3.12 cursor.go:248-277, 41-58).
            self.pos = match self.after(Bound::Unbounded)? {
                Some((first, _)) => Pos::Start(first),
                None => Pos::Nowhere,
            };
            return Ok(None);
        }
        Ok(self.land(e, Pos::EndSeek))
    }

    /// `Delete`: removes the entry the cursor stands on, without moving the
    /// cursor, and returns whether it was still there. As in bbolt, a
    /// cursor that ran off the end with `next` stands on the entry it ran
    /// off from (the last), and one that ran off the start with `prev` on
    /// the table's first entry as it was at that `prev`, so `delete`
    /// removes that entry (bbolt
    /// removes nothing after a `Next` that ran off over leaves emptied in
    /// the transaction, QK-0749); after a `seek` that found nothing, or
    /// `first` or `last` on an empty table, nothing is removed.
    /// On a cursor never positioned nothing is removed either, where bbolt
    /// panics with an index out of range (spec 01 §5.7; dcrwallet never
    /// calls a cursor's `Delete`).
    pub fn delete(&mut self) -> Result<bool> {
        let (Pos::On(k) | Pos::EndNext(k) | Pos::Start(k)) = &self.pos else {
            return Ok(false);
        };
        let k = k.clone();
        self.tx.delete_raw(self.table, &k)
    }

    /// The transaction under the cursor. Anything may be written through
    /// it between moves, in this cursor's table or another; the next move
    /// reads the table as it then is.
    pub fn tx(&mut self) -> &mut WriteTx<'t> {
        self.tx
    }

    /// The table the cursor walks.
    pub fn table(&self) -> TableId {
        self.table
    }
}

/// The moves both cursors share, over raw entries; what [`TypedCursor`]
/// builds on.
pub trait RawCursor {
    /// `First`: the first entry.
    fn first(&mut self) -> Result<Option<RawEntry>>;
    /// `Last`: the last entry.
    fn last(&mut self) -> Result<Option<RawEntry>>;
    /// `Next`.
    fn next(&mut self) -> Result<Option<RawEntry>>;
    /// `Prev`.
    fn prev(&mut self) -> Result<Option<RawEntry>>;
    /// `Seek`: the first entry with key `>= key`.
    fn seek(&mut self, key: &[u8]) -> Result<Option<RawEntry>>;
}

macro_rules! raw_cursor {
    ($t:ty) => {
        impl RawCursor for $t {
            fn first(&mut self) -> Result<Option<RawEntry>> {
                <$t>::first(self)
            }
            fn last(&mut self) -> Result<Option<RawEntry>> {
                <$t>::last(self)
            }
            fn next(&mut self) -> Result<Option<RawEntry>> {
                <$t>::next(self)
            }
            fn prev(&mut self) -> Result<Option<RawEntry>> {
                <$t>::prev(self)
            }
            fn seek(&mut self, key: &[u8]) -> Result<Option<RawEntry>> {
                <$t>::seek(self, key)
            }
        }
    };
}

raw_cursor!(ReadCursor<'_>);
raw_cursor!(WriteCursor<'_, '_>);

/// A cursor over table `T` that decodes what it returns: the same moves as
/// the raw cursor under it, which [`TypedCursor::raw`] reaches (for a
/// [`WriteCursor`], to delete or to write between moves).
pub struct TypedCursor<C, T: Table> {
    raw: C,
    _table: PhantomData<T>,
}

impl<C: RawCursor, T: Table> TypedCursor<C, T> {
    pub(crate) fn new(raw: C) -> Self {
        TypedCursor {
            raw,
            _table: PhantomData,
        }
    }

    fn typed(e: Option<RawEntry>) -> Result<Option<(T::Key, T::Value)>> {
        e.map(|(k, v)| Ok((decode(T::ID, &k)?, decode(T::ID, &v)?)))
            .transpose()
    }

    /// `First`.
    pub fn first(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.first()?)
    }

    /// `Last`.
    pub fn last(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.last()?)
    }

    /// `Next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.next()?)
    }

    /// `Prev`.
    pub fn prev(&mut self) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.prev()?)
    }

    /// `Seek`: the first entry with key `>= key`.
    pub fn seek(&mut self, key: &T::Key) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.seek(key.encode().as_ref())?)
    }

    /// `Seek` to raw bytes that need not be a whole key, as dcrwallet seeks
    /// the unspent outputs to a random 32-byte transaction hash (spec 04
    /// §5.19).
    pub fn seek_bytes(&mut self, key: &[u8]) -> Result<Option<(T::Key, T::Value)>> {
        Self::typed(self.raw.seek(key)?)
    }

    /// The raw cursor under this one.
    pub fn raw(&mut self) -> &mut C {
        &mut self.raw
    }

    /// The raw cursor, giving this one up.
    pub fn into_raw(self) -> C {
        self.raw
    }
}
