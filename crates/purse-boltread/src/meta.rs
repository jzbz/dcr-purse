// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The meta page (bbolt `db.go` `meta`, `validate`, `sum64`).

use crate::error::Error;
use crate::page::{PAGE_HEADER_SIZE, le_u32, le_u64};

/// The magic number at the start of every meta (`0xED0CDAED`).
pub const MAGIC: u32 = 0xED0C_DAED;
/// The only file format version bbolt v1.3.12 reads and writes.
pub const VERSION: u32 = 2;
/// The `freelist` value meaning "no freelist page" (`pgidNoFreelist`).
pub const PGID_NO_FREELIST: u64 = u64::MAX;
/// The size of the meta structure that follows the page header.
pub(crate) const META_SIZE: usize = 64;
/// Bytes of the meta covered by the checksum (everything before it).
const CHECKSUM_OFFSET: usize = 56;

/// A decoded meta page (pages 0 and 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    /// Magic number; [`MAGIC`] in a valid meta.
    pub magic: u32,
    /// Format version; [`VERSION`] in a valid meta.
    pub version: u32,
    /// Page size in bytes, as recorded.
    pub page_size: u32,
    /// Flags; always 0 and never read by bbolt.
    pub flags: u32,
    /// Page id of the root bucket's root page.
    pub root: u64,
    /// The root bucket's sequence counter.
    pub sequence: u64,
    /// Page id of the freelist page, or [`PGID_NO_FREELIST`].
    pub freelist: u64,
    /// High-water mark: one past the highest page id allocated.
    pub pgid: u64,
    /// Id of the transaction that wrote this meta.
    pub txid: u64,
    /// Stored FNV-1a 64 checksum of the first 56 bytes.
    pub checksum: u64,
}

/// FNV-1a, 64-bit (`hash/fnv` `New64a`).
pub(crate) fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl Meta {
    /// Decodes the meta of a page starting at `page[0]`, or `None` when the
    /// buffer is too short to hold one.
    pub(crate) fn decode_page(page: &[u8]) -> Option<(Meta, u64)> {
        let m = page.get(PAGE_HEADER_SIZE..PAGE_HEADER_SIZE.checked_add(META_SIZE)?)?;
        let meta = Meta {
            magic: le_u32(m, 0)?,
            version: le_u32(m, 4)?,
            page_size: le_u32(m, 8)?,
            flags: le_u32(m, 12)?,
            root: le_u64(m, 16)?,
            sequence: le_u64(m, 24)?,
            freelist: le_u64(m, 32)?,
            pgid: le_u64(m, 40)?,
            txid: le_u64(m, 48)?,
            checksum: le_u64(m, 56)?,
        };
        let sum = fnv1a64(m.get(..CHECKSUM_OFFSET)?);
        Some((meta, sum))
    }

    /// bbolt's `meta.validate`, given the checksum computed over the meta's
    /// bytes: magic, then version, then checksum.
    pub(crate) fn validate(&self, computed_sum: u64) -> Result<(), Error> {
        if self.magic != MAGIC {
            Err(Error::Invalid)
        } else if self.version != VERSION {
            Err(Error::VersionMismatch)
        } else if self.checksum != computed_sum {
            Err(Error::Checksum)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The meta of a freshly initialised file (spec 01 section 4.3 vectors).
    fn fresh(page_size: u32, txid: u64) -> Vec<u8> {
        let mut p = vec![0u8; 80];
        p[16..20].copy_from_slice(&MAGIC.to_le_bytes());
        p[20..24].copy_from_slice(&VERSION.to_le_bytes());
        p[24..28].copy_from_slice(&page_size.to_le_bytes());
        p[32..40].copy_from_slice(&3u64.to_le_bytes());
        p[48..56].copy_from_slice(&2u64.to_le_bytes());
        p[56..64].copy_from_slice(&4u64.to_le_bytes());
        p[64..72].copy_from_slice(&txid.to_le_bytes());
        p
    }

    #[test]
    fn checksum_vectors() {
        for (ps, txid, want) in [
            (4096, 0, 0x0751_6e11_4689_fdee_u64),
            (4096, 1, 0x264c_351a_5179_480f),
            (16384, 0, 0x90f3_47e3_0ddd_f37e),
            (16384, 1, 0xafee_0eec_18cd_3d9f),
        ] {
            let (m, sum) = Meta::decode_page(&fresh(ps, txid)).expect("decodes");
            assert_eq!(sum, want, "page size {ps} txid {txid}");
            assert_eq!(m.validate(sum).unwrap_err().to_string(), "checksum error");
        }
    }
}
