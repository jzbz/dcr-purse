// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A whole-tree walk without recursion, shared by the dump and the stats.
//!
//! The walk visits pages depth first in branch order (bbolt's
//! `forEachPage`), the elements of each leaf in order, and descends into a
//! nested bucket right after reaching its element, so the elements come in
//! cursor order. Every page of the file is visited at most once: a page (or
//! one of its overflow pages) reached a second time is reported as
//! corruption, which rules out cycles and keeps the walk linear in the file
//! size.

use std::collections::BTreeMap;

use crate::bucket::Bucket;
use crate::db::{Keep, MAX_TREE_DEPTH};
use crate::error::{Corruption, Result, corrupt};
use crate::page::{Item, PageRef};

/// What the walk reports.
pub(crate) trait Visitor<'db> {
    /// A bucket is entered: the root (`name` is `None`) or a nested one.
    fn enter(&mut self, bucket: &Bucket<'db>, name: Option<&[u8]>) -> Result<()>;
    /// The bucket entered last is finished.
    fn leave(&mut self) -> Result<()>;
    /// A page of the current bucket at tree depth `depth` (0 for its root).
    fn page(&mut self, page: &PageRef, depth: usize) -> Result<()>;
    /// A leaf element of the current bucket. For a nested bucket, the return
    /// value says whether to descend into it.
    fn element(&mut self, item: &Item) -> Result<bool>;
}

/// The page ranges reached so far, as disjoint `first -> last` intervals
/// (memory grows with the pages actually visited, not with the file size
/// or with overflow counts).
struct Visited {
    ranges: BTreeMap<u64, u64>,
}

impl Visited {
    fn new() -> Self {
        Visited {
            ranges: BTreeMap::new(),
        }
    }

    /// Marks `page` and its overflow pages, failing if any was marked.
    fn mark(&mut self, page: &PageRef) -> Result<()> {
        let first = page.pgid;
        let last = first.saturating_add(u64::from(page.overflow));
        // The interval starting at or before `last` with the greatest start
        // is the only one that can overlap [first, last].
        if let Some((&start, &end)) = self.ranges.range(..=last).next_back()
            && end >= first
        {
            return Err(corrupt(
                first,
                Corruption::PageReferencedTwice {
                    pgid: start.max(first),
                },
            ));
        }
        self.ranges.insert(first, last);
        Ok(())
    }
}

struct Frame<'db> {
    bucket: Bucket<'db>,
    /// The page stack with the next index to visit on each page.
    stack: Vec<(PageRef, usize)>,
    started: bool,
}

enum Step<'db> {
    Enter(Bucket<'db>, Item),
    Leave,
    Continue,
}

/// Walks `bucket` and everything below it.
pub(crate) fn walk<'db, V: Visitor<'db>>(bucket: &Bucket<'db>, v: &mut V) -> Result<()> {
    let mut visited = Visited::new();
    let mut load = |b: &Bucket<'db>, pgid: u64| -> Result<PageRef> {
        let page = b.page_node(pgid, Keep::Nothing)?;
        if !page.inline {
            visited.mark(&page)?;
        }
        Ok(page)
    };
    let mut frames = vec![Frame {
        bucket: bucket.clone(),
        stack: Vec::new(),
        started: false,
    }];
    v.enter(bucket, None)?;
    while let Some(frame) = frames.last_mut() {
        let step = if !frame.started {
            frame.started = true;
            let page = load(&frame.bucket, frame.bucket.root_page())?;
            v.page(&page, 0)?;
            frame.stack.push((page, 0));
            Step::Continue
        } else if let Some((page, next)) = frame.stack.last_mut() {
            let i = *next;
            if i >= usize::from(page.count) {
                frame.stack.pop();
                Step::Continue
            } else if page.is_leaf() {
                *next = i.saturating_add(1);
                let item = page.leaf_item(i)?;
                if v.element(&item)? && item.is_bucket() {
                    let child = frame.bucket.open_child(&item)?;
                    Step::Enter(child, item)
                } else {
                    Step::Continue
                }
            } else {
                *next = i.saturating_add(1);
                let pgid = page.branch_elem(i)?.pgid;
                let depth = frame.stack.len();
                if depth >= MAX_TREE_DEPTH {
                    return Err(corrupt(pgid, Corruption::TreeTooDeep));
                }
                let child = load(&frame.bucket, pgid)?;
                v.page(&child, depth)?;
                frame.stack.push((child, 0));
                Step::Continue
            }
        } else {
            Step::Leave
        };
        match step {
            Step::Continue => {}
            Step::Leave => {
                v.leave()?;
                frames.pop();
            }
            Step::Enter(child, item) => {
                v.enter(&child, Some(item.key()))?;
                frames.push(Frame {
                    bucket: child,
                    stack: Vec::new(),
                    started: false,
                });
            }
        }
    }
    Ok(())
}
