// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The canonical text dump of a database's bucket tree.
//!
//! The format is shared with the Go oracle (`tools/oracle/cmd/boltdump`),
//! whose dump of the same file must be byte-for-byte identical. One line
//! per entry, each ending in `\n`, in cursor order, depth first: a nested
//! bucket's own line is followed immediately by the lines of its contents,
//! before the next entry of its parent. Fields are separated by one space;
//! byte strings are lower-case hex (an empty string is empty):
//!
//! ```text
//! b <path> <name> <sequence>
//! k <path> <key> <value>
//! ```
//!
//! `<path>` is the path of the bucket holding the entry: `/` for the root
//! bucket, otherwise `/` followed by the hex names of the buckets from the
//! top level down, joined by `/` (for example `/6d657461/6b6579`). A `b`
//! line is a nested bucket with that name, `<sequence>` its sequence
//! counter in decimal; a `k` line is a key/value pair. Whether a bucket is
//! stored inline and which pages hold what are not part of the dump. The
//! digest is the SHA-256 of the whole dump, and the entry count its number
//! of lines.

use std::io::{BufWriter, Write};

use sha2::{Digest, Sha256};

use crate::bucket::Bucket;
use crate::db::Db;
use crate::error::Result;
use crate::page::{Item, PageRef};
use crate::walk::{Visitor, walk};

/// What a dump wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DumpSummary {
    /// The number of lines (entries).
    pub entries: u64,
    /// The number of bytes.
    pub bytes: u64,
    /// The SHA-256 of the dump.
    pub sha256: [u8; 32],
}

impl DumpSummary {
    /// The digest in lower-case hex.
    pub fn sha256_hex(&self) -> String {
        let mut s = Vec::with_capacity(64);
        push_hex(&mut s, &self.sha256);
        String::from_utf8(s).unwrap_or_default()
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn push_hex(out: &mut Vec<u8>, bytes: &[u8]) {
    out.reserve(bytes.len().saturating_mul(2));
    for &b in bytes {
        out.push(HEX[usize::from(b >> 4)]);
        out.push(HEX[usize::from(b & 0x0f)]);
    }
}

struct DumpVisitor<'w> {
    out: BufWriter<&'w mut dyn Write>,
    hasher: Sha256,
    entries: u64,
    bytes: u64,
    /// Path of the innermost open bucket. Memory stays linear in the
    /// nesting depth: each level appends to it and `lens` remembers where
    /// to cut it back to.
    path: Vec<u8>,
    lens: Vec<usize>,
    line: Vec<u8>,
}

impl DumpVisitor<'_> {
    fn emit(&mut self) -> Result<()> {
        self.line.push(b'\n');
        self.hasher.update(&self.line);
        self.out.write_all(&self.line)?;
        self.entries = self.entries.saturating_add(1);
        self.bytes = self.bytes.saturating_add(self.line.len() as u64);
        Ok(())
    }

    fn start_line(&mut self, kind: u8) -> Vec<u8> {
        let mut line = std::mem::take(&mut self.line);
        line.clear();
        line.push(kind);
        line.push(b' ');
        line.extend_from_slice(&self.path);
        line.push(b' ');
        line
    }
}

impl<'db> Visitor<'db> for DumpVisitor<'_> {
    fn enter(&mut self, bucket: &Bucket<'db>, name: Option<&[u8]>) -> Result<()> {
        let Some(name) = name else {
            self.path.clear();
            self.path.push(b'/');
            self.lens.push(1);
            return Ok(());
        };
        let mut line = self.start_line(b'b');
        push_hex(&mut line, name);
        line.push(b' ');
        line.extend_from_slice(bucket.sequence().to_string().as_bytes());
        self.line = line;
        self.emit()?;
        self.lens.push(self.path.len());
        if self.path.len() > 1 {
            self.path.push(b'/');
        }
        push_hex(&mut self.path, name);
        Ok(())
    }

    fn leave(&mut self) -> Result<()> {
        if let Some(len) = self.lens.pop() {
            self.path.truncate(len);
        }
        Ok(())
    }

    fn page(&mut self, _page: &PageRef, _depth: usize) -> Result<()> {
        Ok(())
    }

    fn element(&mut self, item: &Item) -> Result<bool> {
        if item.is_bucket() {
            // The line is written on entering the bucket.
            return Ok(true);
        }
        let mut line = self.start_line(b'k');
        push_hex(&mut line, item.key());
        line.push(b' ');
        push_hex(&mut line, item.raw_value());
        self.line = line;
        self.emit()?;
        Ok(false)
    }
}

/// Writes the canonical dump of `db` (format in the module documentation)
/// to `out` and returns its entry count and digest.
pub fn dump(db: &Db, out: &mut dyn Write) -> Result<DumpSummary> {
    dump_bucket(&db.root(), out)
}

/// Writes the canonical dump of `bucket` and everything below it, with
/// paths relative to the bucket (it is written as if it were the root).
pub fn dump_bucket(bucket: &Bucket<'_>, out: &mut dyn Write) -> Result<DumpSummary> {
    let mut v = DumpVisitor {
        out: BufWriter::with_capacity(1 << 20, out),
        hasher: Sha256::new(),
        entries: 0,
        bytes: 0,
        path: b"/".to_vec(),
        lens: Vec::new(),
        line: Vec::new(),
    };
    walk(bucket, &mut v)?;
    v.out.flush()?;
    Ok(DumpSummary {
        entries: v.entries,
        bytes: v.bytes,
        sha256: v.hasher.finalize().into(),
    })
}
