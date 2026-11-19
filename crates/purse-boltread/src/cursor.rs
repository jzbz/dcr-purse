// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The cursor (bbolt `cursor.go`), step for step: a stack of (page, index)
//! references from the bucket's root page to a leaf.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::bucket::Bucket;
use crate::db::{Keep, MAX_TREE_DEPTH};
use crate::error::{Corruption, Result, corrupt};
use crate::page::{Bytes, Item, PageRef};

/// bbolt's `elemRef`. The index is signed because bbolt's is: `Last` on an
/// empty leaf leaves it at -1.
#[derive(Clone)]
struct ElemRef {
    page: PageRef,
    index: i64,
}

impl ElemRef {
    fn count(&self) -> i64 {
        i64::from(self.page.count)
    }
}

/// What bbolt's `prev` did.
enum PrevOutcome {
    /// Moved back: the element reached (`None` when it is an empty leaf or
    /// the cursor was never positioned).
    Moved(Option<Item>),
    /// Was at the start: repositioned on the first element, which is given.
    Reset(Option<Item>),
}

/// An ordered cursor over one bucket, reproducing bbolt's positioning rules
/// exactly:
///
/// - [`Cursor::seek`] finds the first key `>=` the target; when there is
///   none it returns `None` and stays one past the end, so that a following
///   [`Cursor::prev`] returns the last entry;
/// - [`Cursor::next`] at the end returns `None` and stays on the last entry;
/// - [`Cursor::prev`] at the start returns `None` and repositions on the
///   first entry;
/// - nested buckets are reported with no value ([`Item::value`] is `None`).
///
/// Unlike bbolt it checks what it walks: a descent that reaches a page
/// already on its path from the root is [`Corruption::Cycle`], and a `next`
/// (`prev`) that does not return a key strictly greater (smaller) than the
/// one the cursor was on is [`Corruption::KeysOutOfOrder`]. Every file bbolt
/// writes passes both checks; they make iteration terminate on a page graph
/// with shared children, where bbolt would repeat entries.
///
/// Every move can fail on a corrupt file; after an error the position is
/// unspecified until the next `first`, `last` or `seek`.
#[derive(Clone)]
pub struct Cursor<'db> {
    bucket: Bucket<'db>,
    stack: Vec<ElemRef>,
    /// What descents keep in the page cache: everything for positioning
    /// moves, branch pages only while stepping.
    keep: Keep,
    /// The key of the element the cursor is on, when it is on one.
    pos: Option<Bytes>,
}

impl std::fmt::Debug for Cursor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cursor")
            .field("bucket", &self.bucket)
            .field("depth", &self.stack.len())
            .finish_non_exhaustive()
    }
}

/// bbolt's `sort.Search`: the smallest index in `0..n` for which `pred`
/// holds, assuming it is monotonic (and probing exactly as Go does when it
/// is not, which a corrupt page can make it).
fn search<F>(n: usize, mut pred: F) -> Result<usize>
where
    F: FnMut(usize) -> Result<bool>,
{
    let (mut i, mut j) = (0usize, n);
    while i < j {
        let h = i.saturating_add(j.saturating_sub(i) / 2);
        if pred(h)? {
            j = h;
        } else {
            i = h.saturating_add(1);
        }
    }
    Ok(i)
}

fn index_of(i: usize) -> i64 {
    i64::try_from(i).unwrap_or(i64::MAX)
}

fn key_of(item: &Option<Item>) -> Option<Bytes> {
    item.as_ref().map(|i| i.key_bytes().clone())
}

impl<'db> Cursor<'db> {
    pub(crate) fn new(bucket: Bucket<'db>) -> Self {
        Cursor {
            bucket,
            stack: Vec::new(),
            keep: Keep::All,
            pos: None,
        }
    }

    /// The bucket this cursor walks.
    pub fn bucket(&self) -> &Bucket<'db> {
        &self.bucket
    }

    /// Moves to the first entry and returns it, or `None` for an empty
    /// bucket.
    pub fn first(&mut self) -> Result<Option<Item>> {
        let item = self.first_inner()?;
        self.pos = key_of(&item);
        Ok(item)
    }

    /// Moves to the last entry and returns it, or `None` for an empty
    /// bucket.
    pub fn last(&mut self) -> Result<Option<Item>> {
        self.stack.clear();
        let root = self.bucket.root_page();
        self.descend(root, true)?;
        self.go_to_last()?;
        let mut skipped = EmptyLeaves::default();
        while self.stack.len() > 1 && self.top_count() == 0 {
            self.skip_empty(&mut skipped)?;
            self.prev_inner()?;
        }
        let item = if self.stack.is_empty() {
            None
        } else {
            self.key_value()?
        };
        self.pos = key_of(&item);
        Ok(item)
    }

    /// Moves to the next entry and returns it; at the end returns `None`
    /// and stays where it was.
    #[allow(clippy::should_implement_trait)] // bbolt's name; fallible, so not Iterator::next
    pub fn next(&mut self) -> Result<Option<Item>> {
        self.keep = Keep::Branches;
        let item = self.next_inner();
        self.keep = Keep::All;
        let item = item?;
        if let Some(it) = &item {
            self.check_order(it, Ordering::Greater)?;
            self.pos = Some(it.key_bytes().clone());
        }
        Ok(item)
    }

    /// Moves to the previous entry and returns it; at the start returns
    /// `None`, leaving the cursor on the first entry.
    pub fn prev(&mut self) -> Result<Option<Item>> {
        self.keep = Keep::Branches;
        let outcome = self.prev_inner();
        self.keep = Keep::All;
        match outcome? {
            PrevOutcome::Reset(first) => {
                self.pos = key_of(&first);
                Ok(None)
            }
            PrevOutcome::Moved(Some(it)) => {
                self.check_order(&it, Ordering::Less)?;
                self.pos = Some(it.key_bytes().clone());
                Ok(Some(it))
            }
            PrevOutcome::Moved(None) => {
                // On an empty leaf (bbolt's quirk) or never positioned.
                self.pos = None;
                Ok(None)
            }
        }
    }

    /// Moves to the first entry whose key is `>=` `key` and returns it, or
    /// `None` when every key is smaller.
    pub fn seek(&mut self, key: &[u8]) -> Result<Option<Item>> {
        let mut item = self.seek_internal(key)?;
        if let Some(top) = self.stack.last()
            && top.index >= top.count()
        {
            item = self.next_inner()?;
        }
        self.pos = key_of(&item);
        Ok(item)
    }

    /// Fails unless `item`'s key compares `want` to the current position.
    fn check_order(&self, item: &Item, want: Ordering) -> Result<()> {
        match &self.pos {
            Some(p) if item.key().cmp(p.as_slice()) != want => {
                Err(corrupt(item.pgid(), Corruption::KeysOutOfOrder))
            }
            _ => Ok(()),
        }
    }

    /// bbolt's unexported `seek`, used by `Get` and `Bucket`: descends to
    /// the leaf and reports the element at the found index, without moving
    /// on to the next leaf when the index is past the end.
    pub(crate) fn seek_internal(&mut self, key: &[u8]) -> Result<Option<Item>> {
        self.stack.clear();
        let mut pgid = self.bucket.root_page();
        loop {
            self.descend(pgid, false)?;
            let Some(top) = self.stack.last_mut() else {
                return Ok(None);
            };
            let page = &top.page;
            let count = usize::from(page.count);
            if page.is_leaf() {
                // nsearch
                let i = search(count, |i| {
                    Ok(page.leaf_item(i)?.key().cmp(key) != Ordering::Less)
                })?;
                top.index = index_of(i);
                return self.key_value();
            }
            // searchPage
            let mut exact = false;
            let mut i = search(count, |i| {
                let e = page.branch_elem(i)?;
                let ord = page.slice(e.key).cmp(key);
                if ord == Ordering::Equal {
                    exact = true;
                }
                Ok(ord != Ordering::Less)
            })?;
            if !exact && i > 0 {
                i = i.saturating_sub(1);
            }
            pgid = page.branch_elem(i)?.pgid;
            top.index = index_of(i);
        }
    }

    /// Reads page `pgid` and pushes it, positioned at its last element when
    /// `at_last` (bbolt's `count() - 1`, -1 for an empty leaf), else at 0.
    /// The page id is checked against the path from the root before the
    /// page is read, so a cycle costs no read.
    fn descend(&mut self, pgid: u64, at_last: bool) -> Result<()> {
        if self.stack.len() >= MAX_TREE_DEPTH {
            return Err(corrupt(pgid, Corruption::TreeTooDeep));
        }
        if !self.bucket.is_inline() && self.stack.iter().any(|e| e.page.pgid == pgid) {
            return Err(corrupt(pgid, Corruption::Cycle { pgid }));
        }
        let page = self.bucket.page_node(pgid, self.keep)?;
        let index = if at_last {
            i64::from(page.count).saturating_sub(1)
        } else {
            0
        };
        self.stack.push(ElemRef { page, index });
        Ok(())
    }

    fn top_count(&self) -> i64 {
        self.stack.last().map_or(0, ElemRef::count)
    }

    /// Records the empty leaf on top of the stack as skipped, failing if it
    /// was skipped before in this move (a shared or cyclic page graph), or
    /// if more leaves were skipped than the file has pages.
    fn skip_empty(&self, skipped: &mut EmptyLeaves) -> Result<()> {
        let pgid = self.stack.last().map_or(0, |e| e.page.pgid);
        if !skipped.0.insert(pgid) || skipped.0.len() as u64 > self.bucket.db().file_pages() {
            return Err(corrupt(pgid, Corruption::TooManyEmptyLeaves));
        }
        Ok(())
    }

    /// The child page id of branch reference `e` at its index.
    fn child_of(e: &ElemRef) -> Result<u64> {
        let i = usize::try_from(e.index).map_err(|_| {
            corrupt(
                e.page.pgid,
                Corruption::ElementOutOfPage { index: u16::MAX },
            )
        })?;
        Ok(e.page.branch_elem(i)?.pgid)
    }

    /// bbolt's `goToFirstElementOnTheStack`.
    fn go_to_first(&mut self) -> Result<()> {
        loop {
            let Some(top) = self.stack.last() else {
                return Ok(());
            };
            if top.page.is_leaf() {
                return Ok(());
            }
            let pgid = Self::child_of(top)?;
            self.descend(pgid, false)?;
        }
    }

    /// bbolt's `last`: descend along the last index.
    fn go_to_last(&mut self) -> Result<()> {
        loop {
            let Some(top) = self.stack.last() else {
                return Ok(());
            };
            if top.page.is_leaf() {
                return Ok(());
            }
            let pgid = Self::child_of(top)?;
            self.descend(pgid, true)?;
        }
    }

    /// bbolt's `first`.
    fn first_inner(&mut self) -> Result<Option<Item>> {
        self.stack.clear();
        let root = self.bucket.root_page();
        self.descend(root, false)?;
        self.go_to_first()?;
        if self.top_count() == 0 {
            self.next_inner()?;
        }
        self.key_value()
    }

    /// bbolt's `next`.
    fn next_inner(&mut self) -> Result<Option<Item>> {
        let mut skipped = EmptyLeaves::default();
        loop {
            let mut level = None;
            for (i, e) in self.stack.iter_mut().enumerate().rev() {
                if e.index < e.count().saturating_sub(1) {
                    e.index = e.index.saturating_add(1);
                    level = Some(i);
                    break;
                }
            }
            // At the end: stay on the last element.
            let Some(i) = level else {
                return Ok(None);
            };
            self.stack.truncate(i.saturating_add(1));
            self.go_to_first()?;
            if self.top_count() == 0 {
                self.skip_empty(&mut skipped)?;
                continue;
            }
            return self.key_value();
        }
    }

    /// bbolt's `prev`.
    fn prev_inner(&mut self) -> Result<PrevOutcome> {
        let mut i = self.stack.len();
        while i > 0 {
            i = i.saturating_sub(1);
            let Some(e) = self.stack.get_mut(i) else {
                break;
            };
            if e.index > 0 {
                e.index = e.index.saturating_sub(1);
                break;
            }
            // At the beginning: stay on the first element.
            if self.stack.len() == 1 {
                return Ok(PrevOutcome::Reset(self.first_inner()?));
            }
            self.stack.truncate(i);
        }
        if self.stack.is_empty() {
            return Ok(PrevOutcome::Moved(None));
        }
        self.go_to_last()?;
        Ok(PrevOutcome::Moved(self.key_value()?))
    }

    /// bbolt's `keyValue`: the element at the top reference, if the index
    /// is on one.
    fn key_value(&self) -> Result<Option<Item>> {
        let Some(top) = self.stack.last() else {
            return Ok(None);
        };
        if top.count() == 0 || top.index >= top.count() || !top.page.is_leaf() {
            return Ok(None);
        }
        let Ok(i) = usize::try_from(top.index) else {
            return Ok(None);
        };
        top.page.leaf_item(i).map(Some)
    }
}

/// The empty leaves one cursor move has skipped.
#[derive(Default)]
struct EmptyLeaves(HashSet<u64>);

/// A forward iterator over a bucket's entries, in key order.
#[derive(Debug)]
pub struct Iter<'db> {
    cursor: Cursor<'db>,
    started: bool,
    done: bool,
}

impl<'db> Iter<'db> {
    pub(crate) fn new(cursor: Cursor<'db>) -> Self {
        Iter {
            cursor,
            started: false,
            done: false,
        }
    }
}

impl Iterator for Iter<'_> {
    type Item = Result<Item>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let r = if self.started {
            self.cursor.next()
        } else {
            self.started = true;
            self.cursor.first()
        };
        match r {
            Ok(Some(item)) => Some(Ok(item)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}
