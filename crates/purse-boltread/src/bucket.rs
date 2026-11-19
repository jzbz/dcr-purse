// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Buckets: lookup of keys and nested buckets (bbolt `bucket.go`, read
//! path).

use crate::cursor::{Cursor, Iter};
use crate::db::{Db, Keep};
use crate::error::{Corruption, Result, corrupt};
use crate::page::{BUCKET_HEADER_SIZE, Bytes, Item, PageRef, inline_page_of, le_u64};
use crate::stats::BucketStats;

/// A bucket of the snapshot: the root bucket or a nested one.
#[derive(Clone)]
pub struct Bucket<'db> {
    db: &'db Db,
    root: u64,
    sequence: u64,
    /// The embedded page of an inline bucket (`root == 0`).
    inline: Option<PageRef>,
    /// The page holding this bucket's header (for error context).
    holder: u64,
}

impl std::fmt::Debug for Bucket<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bucket")
            .field("root", &self.root)
            .field("sequence", &self.sequence)
            .field("inline", &self.inline.is_some())
            .finish_non_exhaustive()
    }
}

impl<'db> Bucket<'db> {
    pub(crate) fn root(db: &'db Db, root: u64, sequence: u64) -> Self {
        Bucket {
            db,
            root,
            sequence,
            inline: None,
            holder: root,
        }
    }

    /// Opens the nested bucket an element describes (bbolt `openBucket`).
    /// The element must carry the bucket flag; the value must hold the
    /// 16-byte header and, for an inline bucket, an embedded leaf page.
    pub(crate) fn open_child(&self, item: &Item) -> Result<Bucket<'db>> {
        let v = item.raw_value();
        let pgid = item.pgid();
        let short = || corrupt(pgid, Corruption::BucketValueTooShort { len: v.len() });
        if v.len() < BUCKET_HEADER_SIZE {
            return Err(short());
        }
        let root = le_u64(v, 0).ok_or_else(short)?;
        let sequence = le_u64(v, 8).ok_or_else(short)?;
        let inline = if root == 0 {
            Some(inline_page_of(item.value_bytes(), pgid)?)
        } else {
            None
        };
        Ok(Bucket {
            db: self.db,
            root,
            sequence,
            inline,
            holder: pgid,
        })
    }

    /// The database this bucket belongs to.
    pub fn db(&self) -> &'db Db {
        self.db
    }

    /// The page id of the bucket's root page; 0 for an inline bucket.
    pub fn root_page(&self) -> u64 {
        self.root
    }

    /// The bucket's sequence counter (`NextSequence`).
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Whether the bucket is stored inline in its parent's leaf.
    pub fn is_inline(&self) -> bool {
        self.root == 0
    }

    /// A cursor over the bucket, not yet positioned.
    pub fn cursor(&self) -> Cursor<'db> {
        Cursor::new(self.clone())
    }

    /// Iterates over the bucket's entries in key order (`First`, then
    /// `Next`). Stops after the first error.
    pub fn iter(&self) -> Iter<'db> {
        Iter::new(self.cursor())
    }

    /// The value stored under `key`, or `None` if there is none or `key`
    /// names a nested bucket (bbolt `Get`).
    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let item = self.cursor().seek_internal(key)?;
        Ok(match item {
            Some(item) if !item.is_bucket() && item.key() == key => {
                Some(item.value_bytes().clone())
            }
            _ => None,
        })
    }

    /// The nested bucket `name`, or `None` if there is none or `name` is a
    /// plain key (bbolt `Bucket`).
    pub fn bucket(&self, name: &[u8]) -> Result<Option<Bucket<'db>>> {
        match self.cursor().seek_internal(name)? {
            Some(item) if item.is_bucket() && item.key() == name => {
                self.open_child(&item).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// bbolt's `Bucket.Stats` for this bucket and everything below it.
    pub fn stats(&self) -> Result<BucketStats> {
        crate::stats::bucket_stats(self)
    }

    /// bbolt's `pageNode` for a bucket read from the file: the embedded
    /// page of an inline bucket (which has no other pages), else the tree
    /// page `id` (kept in the page cache as `keep` says).
    pub(crate) fn page_node(&self, id: u64, keep: Keep) -> Result<PageRef> {
        if self.root == 0 {
            if id != 0 {
                return Err(corrupt(
                    self.holder,
                    Corruption::InlinePageAccess { pgid: id },
                ));
            }
            return self
                .inline
                .clone()
                .ok_or_else(|| corrupt(self.holder, Corruption::InlineRootBucket));
        }
        self.db.tree_page(id, keep)
    }
}
