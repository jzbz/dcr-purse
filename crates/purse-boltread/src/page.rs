// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Page headers and branch/leaf elements (bbolt `page.go`), decoded from
//! byte buffers with every offset checked.

use std::fmt;
use std::ops::{Deref, Range};
use std::sync::Arc;

use crate::error::{Corruption, Result, corrupt};

/// Size of the page header.
pub(crate) const PAGE_HEADER_SIZE: usize = 16;
/// Size of a branch or leaf element header.
pub(crate) const ELEMENT_SIZE: usize = 16;
/// Size of a nested bucket's header (`root`, `sequence`).
pub(crate) const BUCKET_HEADER_SIZE: usize = 16;

/// Page flag: branch page.
pub const BRANCH_PAGE_FLAG: u16 = 0x01;
/// Page flag: leaf page.
pub const LEAF_PAGE_FLAG: u16 = 0x02;
/// Page flag: meta page.
pub const META_PAGE_FLAG: u16 = 0x04;
/// Page flag: freelist page.
pub const FREELIST_PAGE_FLAG: u16 = 0x10;
/// Leaf element flag: the value is a nested bucket.
pub const BUCKET_LEAF_FLAG: u32 = 0x01;

pub(crate) fn le_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(off..off.checked_add(2)?)?.try_into().ok()?,
    ))
}

pub(crate) fn le_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(off..off.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub(crate) fn le_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(off..off.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// A shared, immutable byte range of a page read from the file: a key, a
/// value, or an inline page. Dereferences to `[u8]`.
#[derive(Clone)]
pub struct Bytes {
    buf: Arc<Vec<u8>>,
    range: Range<usize>,
}

impl Bytes {
    /// The bytes as a slice.
    pub fn as_slice(&self) -> &[u8] {
        self.buf.get(self.range.clone()).unwrap_or_default()
    }

    /// Copies the bytes into a new vector.
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Bytes {}

impl PartialEq<[u8]> for Bytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bytes(")?;
        for b in self.as_slice() {
            write!(f, "{b:02x}")?;
        }
        f.write_str(")")
    }
}

/// One leaf element as a cursor reports it: a key and either a value or a
/// nested bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    key: Bytes,
    value: Bytes,
    flags: u32,
    /// The file page the element lives in (for error context).
    pgid: u64,
}

impl Item {
    /// The key (for a nested bucket, its name).
    pub fn key(&self) -> &[u8] {
        self.key.as_slice()
    }

    /// The value, or `None` for a nested bucket (bbolt's cursors return a
    /// nil value for buckets).
    pub fn value(&self) -> Option<&[u8]> {
        if self.is_bucket() {
            None
        } else {
            Some(self.value.as_slice())
        }
    }

    /// The stored value bytes even for a nested bucket (its bucket header,
    /// followed by the embedded page for an inline bucket).
    pub fn raw_value(&self) -> &[u8] {
        self.value.as_slice()
    }

    /// Whether the element is a nested bucket.
    pub fn is_bucket(&self) -> bool {
        self.flags & BUCKET_LEAF_FLAG != 0
    }

    /// The element's raw flags word.
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// The key as shared bytes.
    pub fn key_bytes(&self) -> &Bytes {
        &self.key
    }

    /// The stored value as shared bytes (see [`Item::raw_value`]).
    pub fn value_bytes(&self) -> &Bytes {
        &self.value
    }

    pub(crate) fn pgid(&self) -> u64 {
        self.pgid
    }
}

/// A decoded branch element.
pub(crate) struct BranchElem {
    pub(crate) key: Range<usize>,
    pub(crate) pgid: u64,
}

/// A logical page (a physical page plus its overflow pages, or the embedded
/// page of an inline bucket) whose header and element array have been
/// checked to fit.
#[derive(Clone)]
pub(crate) struct PageRef {
    buf: Arc<Vec<u8>>,
    start: usize,
    end: usize,
    /// The page id (for an inline page, the file page holding it).
    pub(crate) pgid: u64,
    pub(crate) inline: bool,
    pub(crate) flags: u16,
    pub(crate) count: u16,
    pub(crate) overflow: u32,
}

impl PageRef {
    /// Decodes the header of a page read from the file. The caller has
    /// checked that `buf` is the whole logical page.
    pub(crate) fn file_page(buf: Arc<Vec<u8>>, pgid: u64) -> Result<PageRef> {
        let end = buf.len();
        Self::new(buf, 0, end, pgid, false)
    }

    /// The embedded page of an inline bucket: `range` of `buf` is the
    /// bucket value after its 16-byte header.
    pub(crate) fn inline_page(
        buf: Arc<Vec<u8>>,
        range: Range<usize>,
        pgid: u64,
    ) -> Result<PageRef> {
        let page = Self::new(buf, range.start, range.end, pgid, true)?;
        if page.flags & LEAF_PAGE_FLAG == 0 {
            return Err(corrupt(
                pgid,
                Corruption::InlinePageNotLeaf { flags: page.flags },
            ));
        }
        Ok(page)
    }

    fn new(
        buf: Arc<Vec<u8>>,
        start: usize,
        end: usize,
        pgid: u64,
        inline: bool,
    ) -> Result<PageRef> {
        let bytes = buf.get(start..end).unwrap_or_default();
        let header = (|| Some((le_u16(bytes, 8)?, le_u16(bytes, 10)?, le_u32(bytes, 12)?)))();
        let Some((flags, count, overflow)) = header else {
            return Err(corrupt(
                pgid,
                if inline {
                    Corruption::BucketValueTooShort {
                        len: bytes.len().saturating_add(BUCKET_HEADER_SIZE),
                    }
                } else {
                    Corruption::ElementCountOutOfPage { count: 0 }
                },
            ));
        };
        let page = PageRef {
            buf: buf.clone(),
            start,
            end,
            pgid,
            inline,
            flags,
            count,
            overflow,
        };
        if flags & (BRANCH_PAGE_FLAG | LEAF_PAGE_FLAG) != 0 {
            let need = usize::from(count)
                .checked_mul(ELEMENT_SIZE)
                .and_then(|n| n.checked_add(PAGE_HEADER_SIZE));
            if need.is_none_or(|n| n > bytes.len()) {
                return Err(corrupt(pgid, Corruption::ElementCountOutOfPage { count }));
            }
        }
        Ok(page)
    }

    /// The page bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        self.buf.get(self.start..self.end).unwrap_or_default()
    }

    /// bbolt's `elemRef.isLeaf`: the leaf flag is set. (A page from the
    /// file has been checked to be exactly a branch or a leaf; an inline
    /// page to have the leaf flag.)
    pub(crate) fn is_leaf(&self) -> bool {
        self.flags & LEAF_PAGE_FLAG != 0
    }

    fn out_of_page(&self, index: usize) -> crate::error::Error {
        corrupt(
            self.pgid,
            Corruption::ElementOutOfPage {
                index: u16::try_from(index).unwrap_or(u16::MAX),
            },
        )
    }

    /// The absolute buffer range of `len` bytes at page offset `at`,
    /// checked to lie in the page, and the page offset just past it.
    fn span(&self, index: usize, at: u64, len: u32) -> Result<(Range<usize>, u64)> {
        let page_len = self.end.saturating_sub(self.start) as u64;
        let end = at
            .checked_add(u64::from(len))
            .filter(|&e| e <= page_len)
            .ok_or_else(|| self.out_of_page(index))?;
        // Both fit in usize: they are bounded by the page length.
        let s = usize::try_from(at)
            .ok()
            .and_then(|s| self.start.checked_add(s));
        let e = usize::try_from(end)
            .ok()
            .and_then(|e| self.start.checked_add(e));
        match (s, e) {
            (Some(s), Some(e)) => Ok((s..e, end)),
            _ => Err(self.out_of_page(index)),
        }
    }

    fn header_offset(&self, index: usize) -> Result<usize> {
        if index >= usize::from(self.count) {
            return Err(self.out_of_page(index));
        }
        // In bounds: the element array was checked to fit at construction.
        index
            .checked_mul(ELEMENT_SIZE)
            .and_then(|n| n.checked_add(PAGE_HEADER_SIZE))
            .ok_or_else(|| self.out_of_page(index))
    }

    /// Leaf element `index` (bbolt `leafPageElement`, `key`, `value`).
    pub(crate) fn leaf_item(&self, index: usize) -> Result<Item> {
        let h = self.header_offset(index)?;
        let b = self.bytes();
        let fields = (|| {
            Some((
                le_u32(b, h)?,
                le_u32(b, h.checked_add(4)?)?,
                le_u32(b, h.checked_add(8)?)?,
                le_u32(b, h.checked_add(12)?)?,
            ))
        })();
        let (flags, pos, ksize, vsize) = fields.ok_or_else(|| self.out_of_page(index))?;
        let (key, after) = self.span(index, (h as u64).saturating_add(u64::from(pos)), ksize)?;
        let (value, _) = self.span(index, after, vsize)?;
        Ok(Item {
            key: Bytes {
                buf: self.buf.clone(),
                range: key,
            },
            value: Bytes {
                buf: self.buf.clone(),
                range: value,
            },
            flags,
            pgid: self.pgid,
        })
    }

    /// Branch element `index` (bbolt `branchPageElement`, `key`).
    pub(crate) fn branch_elem(&self, index: usize) -> Result<BranchElem> {
        let h = self.header_offset(index)?;
        let b = self.bytes();
        let fields = (|| {
            Some((
                le_u32(b, h)?,
                le_u32(b, h.checked_add(4)?)?,
                le_u64(b, h.checked_add(8)?)?,
            ))
        })();
        let (pos, ksize, pgid) = fields.ok_or_else(|| self.out_of_page(index))?;
        let (key, _) = self.span(index, (h as u64).saturating_add(u64::from(pos)), ksize)?;
        Ok(BranchElem { key, pgid })
    }

    /// The bytes of an absolute buffer range from [`PageRef::branch_elem`].
    pub(crate) fn slice(&self, r: Range<usize>) -> &[u8] {
        self.buf.get(r).unwrap_or_default()
    }

    /// Offset of `r.end` from the start of the page.
    pub(crate) fn rel_end(&self, r: &Range<usize>) -> usize {
        r.end.saturating_sub(self.start)
    }

    /// The end of `item`'s value relative to the start of this page.
    pub(crate) fn item_rel_end(&self, item: &Item) -> usize {
        item.value.range.end.saturating_sub(self.start)
    }
}

/// Builds the embedded page of an inline bucket from its value bytes.
pub(crate) fn inline_page_of(value: &Bytes, pgid: u64) -> Result<PageRef> {
    let start = value
        .range
        .start
        .checked_add(BUCKET_HEADER_SIZE)
        .filter(|&s| s <= value.range.end)
        .ok_or_else(|| {
            corrupt(
                pgid,
                Corruption::BucketValueTooShort {
                    len: value.range.len(),
                },
            )
        })?;
    PageRef::inline_page(value.buf.clone(), start..value.range.end, pgid)
}
