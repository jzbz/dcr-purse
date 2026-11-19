// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! bbolt's `Bucket.Stats`, computed by the shared walk.

use crate::bucket::Bucket;
use crate::error::Result;
use crate::page::{Item, PAGE_HEADER_SIZE, PageRef};
use crate::walk::{Visitor, walk};

/// Page and key counts for a bucket and every bucket below it, with
/// bbolt's `BucketStats` semantics: `key_n` counts every leaf element
/// (nested-bucket entries included) of the bucket and its descendants, and
/// `depth` is the bucket's tree depth plus the deepest descendant's.
/// Buckets nested inside an inline bucket are not counted, as in bbolt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BucketStats {
    /// Logical branch pages.
    pub branch_page_n: u64,
    /// Physical branch overflow pages.
    pub branch_overflow_n: u64,
    /// Logical leaf pages.
    pub leaf_page_n: u64,
    /// Physical leaf overflow pages.
    pub leaf_overflow_n: u64,
    /// Key/value pairs, nested-bucket entries included.
    pub key_n: u64,
    /// Levels of the tree, nested buckets included.
    pub depth: u64,
    /// Bytes allocated for branch pages.
    pub branch_alloc: u64,
    /// Bytes in use on branch pages.
    pub branch_inuse: u64,
    /// Bytes allocated for leaf pages.
    pub leaf_alloc: u64,
    /// Bytes in use on leaf pages.
    pub leaf_inuse: u64,
    /// Buckets, this one included.
    pub bucket_n: u64,
    /// Inline buckets.
    pub inline_bucket_n: u64,
    /// Bytes used by inline buckets.
    pub inline_bucket_inuse: u64,
}

impl BucketStats {
    /// bbolt's `BucketStats.Add`.
    fn add(&mut self, o: &BucketStats) {
        self.branch_page_n = self.branch_page_n.saturating_add(o.branch_page_n);
        self.branch_overflow_n = self.branch_overflow_n.saturating_add(o.branch_overflow_n);
        self.leaf_page_n = self.leaf_page_n.saturating_add(o.leaf_page_n);
        self.leaf_overflow_n = self.leaf_overflow_n.saturating_add(o.leaf_overflow_n);
        self.key_n = self.key_n.saturating_add(o.key_n);
        self.depth = self.depth.max(o.depth);
        self.branch_alloc = self.branch_alloc.saturating_add(o.branch_alloc);
        self.branch_inuse = self.branch_inuse.saturating_add(o.branch_inuse);
        self.leaf_alloc = self.leaf_alloc.saturating_add(o.leaf_alloc);
        self.leaf_inuse = self.leaf_inuse.saturating_add(o.leaf_inuse);
        self.bucket_n = self.bucket_n.saturating_add(o.bucket_n);
        self.inline_bucket_n = self.inline_bucket_n.saturating_add(o.inline_bucket_n);
        self.inline_bucket_inuse = self
            .inline_bucket_inuse
            .saturating_add(o.inline_bucket_inuse);
    }
}

struct Frame {
    own: BucketStats,
    sub: BucketStats,
    inline: bool,
}

struct StatsVisitor {
    page_size: u64,
    frames: Vec<Frame>,
    result: BucketStats,
}

impl<'db> Visitor<'db> for StatsVisitor {
    fn enter(&mut self, bucket: &Bucket<'db>, _name: Option<&[u8]>) -> Result<()> {
        let inline = bucket.is_inline();
        self.frames.push(Frame {
            own: BucketStats {
                bucket_n: 1,
                inline_bucket_n: u64::from(inline),
                ..BucketStats::default()
            },
            sub: BucketStats::default(),
            inline,
        });
        Ok(())
    }

    fn leave(&mut self) -> Result<()> {
        let Some(Frame { mut own, sub, .. }) = self.frames.pop() else {
            return Ok(());
        };
        own.branch_alloc = own
            .branch_page_n
            .saturating_add(own.branch_overflow_n)
            .saturating_mul(self.page_size);
        own.leaf_alloc = own
            .leaf_page_n
            .saturating_add(own.leaf_overflow_n)
            .saturating_mul(self.page_size);
        own.depth = own.depth.saturating_add(sub.depth);
        own.add(&sub);
        match self.frames.last_mut() {
            Some(parent) => parent.sub.add(&own),
            None => self.result = own,
        }
        Ok(())
    }

    fn page(&mut self, page: &PageRef, depth: usize) -> Result<()> {
        let Some(f) = self.frames.last_mut() else {
            return Ok(());
        };
        let count = usize::from(page.count);
        if page.is_leaf() {
            f.own.key_n = f.own.key_n.saturating_add(count as u64);
            let used = match count.checked_sub(1) {
                Some(last) => page.item_rel_end(&page.leaf_item(last)?),
                None => PAGE_HEADER_SIZE,
            } as u64;
            if f.inline {
                f.own.inline_bucket_inuse = f.own.inline_bucket_inuse.saturating_add(used);
            } else {
                f.own.leaf_page_n = f.own.leaf_page_n.saturating_add(1);
                f.own.leaf_inuse = f.own.leaf_inuse.saturating_add(used);
                f.own.leaf_overflow_n = f
                    .own
                    .leaf_overflow_n
                    .saturating_add(u64::from(page.overflow));
            }
        } else {
            f.own.branch_page_n = f.own.branch_page_n.saturating_add(1);
            // A tree page that is a branch has at least one element.
            let last = page.branch_elem(count.saturating_sub(1))?;
            let used = page.rel_end(&last.key) as u64;
            f.own.branch_inuse = f.own.branch_inuse.saturating_add(used);
            f.own.branch_overflow_n = f
                .own
                .branch_overflow_n
                .saturating_add(u64::from(page.overflow));
        }
        let d = (depth as u64).saturating_add(1);
        if d > f.own.depth {
            f.own.depth = d;
        }
        Ok(())
    }

    fn element(&mut self, item: &Item) -> Result<bool> {
        // bbolt collects sub-bucket stats from the leaves of regular
        // buckets only.
        Ok(item.is_bucket() && self.frames.last().is_some_and(|f| !f.inline))
    }
}

pub(crate) fn bucket_stats(bucket: &Bucket<'_>) -> Result<BucketStats> {
    let mut v = StatsVisitor {
        page_size: u64::from(bucket.db().page_size()),
        frames: Vec::new(),
        result: BucketStats::default(),
    };
    walk(bucket, &mut v)?;
    Ok(v.result)
}
