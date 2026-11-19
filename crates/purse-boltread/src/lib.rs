// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A read-only reader for bbolt v1.3.12 database files, the format of
//! dcrwallet's `wallet.db`.
//!
//! It is a test utility: dcr-purse's tests read Go-written fixture databases
//! with it. The wallet itself reads no dcrwallet database, so this crate is
//! not part of it and is not published.
//!
//! The reader never writes and never memory-maps: pages are read with
//! positional reads from a [`Source`] (a file, or bytes in memory) into a
//! small cache. Opening a file takes a shared lock with a single
//! non-blocking attempt, so it fails promptly with [`Error::Locked`] while
//! dcrwallet has the database open instead of waiting as bbolt does.
//!
//! Meta page validation and selection, the page-size probe, and cursor
//! positioning follow bbolt exactly. Where bbolt trusts the file and would
//! panic, fault or read unrelated memory on a malformed one, the reader
//! returns [`Error::Corrupt`] instead: every offset, count, length and page
//! id is checked against its page and the file, a page is never larger than
//! the file nor than [`OpenOptions::max_page_bytes`], and cycles, shared
//! pages, keys out of order and absurd depths are detected.
//!
//! The snapshot read is consistent only while nothing writes the file. The
//! lock keeps out dcrwallet and every other bbolt writer, but not a writer
//! that ignores it (`cp` over the file, a sync tool, a writer on another NFS
//! client), and nothing at all when opened with
//! [`OpenOptions::lock(false)`](OpenOptions::lock). Call
//! [`Db::verify_unchanged`] after reading to detect such a write.
//! [`Error::class`] tells the failures a caller must distinguish apart:
//! not a bbolt file, corrupt, locked, cannot open, cannot lock.
//!
//! ```no_run
//! # fn main() -> Result<(), purse_boltread::Error> {
//! let db = purse_boltread::Db::open("wallet.db")?;
//! if let Some(meta) = db.bucket(b"meta")? {
//!     let version = meta.get(b"ver")?;
//!     # let _ = version;
//! }
//! let summary = purse_boltread::dump(&db, &mut std::io::sink())?;
//! println!("{} entries, sha256 {}", summary.entries, summary.sha256_hex());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod bucket;
mod cursor;
mod db;
mod dump;
mod error;
mod meta;
mod page;
mod source;
mod stats;
mod walk;

pub use bucket::Bucket;
pub use cursor::{Cursor, Iter};
pub use db::{
    DEFAULT_CACHE_BYTES, DEFAULT_MAX_PAGE_BYTES, Db, MAX_TREE_DEPTH, MIN_PAGE_SIZE, OpenOptions,
};
pub use dump::{DumpSummary, dump, dump_bucket};
pub use error::{Change, Corruption, Error, ErrorClass, Result};
pub use meta::{MAGIC, Meta, PGID_NO_FREELIST, VERSION};
pub use page::{
    BRANCH_PAGE_FLAG, BUCKET_LEAF_FLAG, Bytes, FREELIST_PAGE_FLAG, Item, LEAF_PAGE_FLAG,
    META_PAGE_FLAG,
};
pub use source::Source;
pub use stats::BucketStats;
