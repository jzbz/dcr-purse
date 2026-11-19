// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The reader's error type.

use std::fmt;
use std::io;

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong opening or reading a bbolt file.
///
/// One group mirrors the errors bbolt's own `Open` returns
/// ([`Error::Invalid`], [`Error::VersionMismatch`], [`Error::Checksum`],
/// [`Error::FileTooSmall`]). [`Error::Corrupt`] covers every structural
/// defect that bbolt does not check for and would answer with a panic, a
/// fault or a read of unrelated bytes. [`Error::class`] sorts every variant
/// into the handful of cases a caller has to tell apart.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file could not be opened: it does not exist, permission was
    /// denied, and so on.
    Open(io::Error),
    /// The path names something other than a regular file (a directory, a
    /// FIFO, a socket, a device; symbolic links are followed).
    NotRegularFile,
    /// Another process holds the database file's lock: dcrwallet (or any
    /// other bbolt program opened read-write) has the file open. Stop it and
    /// try again.
    Locked,
    /// The lock could not be attempted at all: the filesystem or platform
    /// does not support it (`ENOLCK`, `EOPNOTSUPP`, `ENOSYS`, ...). Opening
    /// with [`OpenOptions::lock(false)`](crate::OpenOptions::lock) skips the
    /// lock, which is safe only once it is confirmed by other means that
    /// dcrwallet is stopped and stays stopped.
    LockUnavailable(io::Error),
    /// Reading the file failed after it was opened (or a page buffer could
    /// not be allocated). A file that shrinks underneath the reader shows up
    /// here; see [`Db`](crate::Db) on changes after opening.
    Io(io::Error),
    /// The file is empty. bbolt would initialise it as a new database when
    /// opened read-write; a read-only open cannot.
    EmptyFile,
    /// No meta page is usable and meta page 0 has the wrong magic number:
    /// not a bbolt file (bbolt's `ErrInvalid`, "invalid database"). Also
    /// returned, as by bbolt, when neither meta page can even be located.
    Invalid,
    /// No meta page is usable and meta page 0 has a format version other
    /// than 2 (bbolt's `ErrVersionMismatch`).
    VersionMismatch,
    /// No meta page is usable and meta page 0 fails its checksum (bbolt's
    /// `ErrChecksum`).
    Checksum,
    /// The file is shorter than two pages ("file size too small").
    FileTooSmall {
        /// The file length in bytes.
        len: u64,
        /// The page size in use.
        page_size: u32,
    },
    /// The meta page declares a page size this reader refuses: not a power
    /// of two, or smaller than [`MIN_PAGE_SIZE`](crate::MIN_PAGE_SIZE).
    UnsupportedPageSize(u32),
    /// A structural defect in the page graph.
    Corrupt {
        /// The page the defect was found on; for an inline bucket, the page
        /// whose leaf holds the bucket's value.
        page: u64,
        /// What is wrong.
        detail: Corruption,
    },
    /// [`Db::verify_unchanged`](crate::Db::verify_unchanged) found the file
    /// different from what was read at open: something wrote to it while
    /// it was being read, so what was read may be inconsistent.
    Changed {
        /// What differs.
        what: Change,
    },
}

/// What [`Error::Changed`] found different.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// The file length.
    Length {
        /// The length at open.
        was: u64,
        /// The length now.
        now: u64,
    },
    /// The bytes of a meta page.
    MetaPage(u64),
}

/// The cases a caller tells apart, from [`Error::class`].
///
/// | Class | Variants | Meaning |
/// | --- | --- | --- |
/// | `CannotOpen` | `Open`, `NotRegularFile` | the path cannot be opened as a file |
/// | `CannotLock` | `LockUnavailable` | the lock cannot be attempted here |
/// | `Locked` | `Locked` | dcrwallet (or another writer) has the file open |
/// | `NotBolt` | `EmptyFile`, `Invalid` | not a bbolt database |
/// | `Unsupported` | `VersionMismatch`, `UnsupportedPageSize` | a bbolt-like file this reader does not read |
/// | `Corrupt` | `Checksum`, `FileTooSmall`, `Corrupt` | a damaged or hostile bbolt file |
/// | `Io` | `Io` | reading failed after the open |
/// | `Changed` | `Changed` | the file was written while being read |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// The path cannot be opened as a file.
    CannotOpen,
    /// The lock cannot be attempted on this filesystem or platform.
    CannotLock,
    /// Another process holds the lock.
    Locked,
    /// Not a bbolt database.
    NotBolt,
    /// A bbolt-like file in a form this reader does not read.
    Unsupported,
    /// A damaged or hostile bbolt file.
    Corrupt,
    /// A read failed after the open.
    Io,
    /// The file changed while being read.
    Changed,
}

impl Error {
    /// The class of this error (see [`ErrorClass`] for the mapping).
    pub fn class(&self) -> ErrorClass {
        match self {
            Error::Open(_) | Error::NotRegularFile => ErrorClass::CannotOpen,
            Error::LockUnavailable(_) => ErrorClass::CannotLock,
            Error::Locked => ErrorClass::Locked,
            Error::EmptyFile | Error::Invalid => ErrorClass::NotBolt,
            Error::VersionMismatch | Error::UnsupportedPageSize(_) => ErrorClass::Unsupported,
            Error::Checksum | Error::FileTooSmall { .. } | Error::Corrupt { .. } => {
                ErrorClass::Corrupt
            }
            Error::Io(_) => ErrorClass::Io,
            Error::Changed { .. } => ErrorClass::Changed,
        }
    }
}

/// The structural defects [`Error::Corrupt`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Corruption {
    /// A page, or the overflow pages that continue it, lies (partly) beyond
    /// the end of the file.
    PageOutOfBounds {
        /// One past the last page the reference needs.
        end_page: u64,
        /// The number of whole pages in the file.
        file_pages: u64,
    },
    /// A logical page (with its overflow pages) is larger than
    /// [`OpenOptions::max_page_bytes`](crate::OpenOptions::max_page_bytes).
    PageTooLarge {
        /// The logical page's size in bytes.
        bytes: u64,
        /// The limit.
        max: u64,
    },
    /// The page header names a different page id (bbolt: assertion panic).
    PageIdMismatch {
        /// The id in the header.
        found: u64,
    },
    /// The page has flags other than exactly one page type (bbolt:
    /// assertion panic), or a type that does not belong where it was
    /// reached (a meta or freelist page inside a bucket tree, a
    /// non-freelist page as the freelist).
    UnexpectedPageType {
        /// The page header's flags.
        flags: u16,
    },
    /// The element array (16 bytes per element) does not fit in the page.
    ElementCountOutOfPage {
        /// The element count from the header.
        count: u16,
    },
    /// An element's key or value lies outside its page.
    ElementOutOfPage {
        /// The element index.
        index: u16,
    },
    /// A branch page with no elements.
    EmptyBranch,
    /// A bucket's tree is deeper than [`MAX_TREE_DEPTH`](crate::MAX_TREE_DEPTH)
    /// levels.
    TreeTooDeep,
    /// A cursor descending the tree reached a page already on its path from
    /// the root: a cycle.
    Cycle {
        /// The page reached again.
        pgid: u64,
    },
    /// A page reached twice while walking the whole tree (a cycle, or a page
    /// shared between two parents; bbolt's consistency check calls this
    /// "multiple references").
    PageReferencedTwice {
        /// The page reached again.
        pgid: u64,
    },
    /// A cursor step returned a key not strictly after (for `next`) or
    /// before (for `prev`) the key it moved from: keys out of order, or a
    /// page shared between two parents.
    KeysOutOfOrder,
    /// A cursor step skipped the same empty leaf twice, or more empty leaves
    /// than the file has pages.
    TooManyEmptyLeaves,
    /// A nested-bucket value shorter than the 16-byte bucket header.
    BucketValueTooShort {
        /// The value length.
        len: usize,
    },
    /// An inline bucket whose embedded page is not a leaf.
    InlinePageNotLeaf {
        /// The embedded page header's flags.
        flags: u16,
    },
    /// An inline bucket asked for a page other than its embedded one
    /// (bbolt: panic).
    InlinePageAccess {
        /// The page id asked for.
        pgid: u64,
    },
    /// The root bucket claims to be inline (its root page id is 0).
    InlineRootBucket,
    /// The freelist page's id list does not fit in the page.
    FreelistOutOfPage {
        /// The id count the page declares.
        count: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Open(e) => write!(f, "cannot open database file: {e}"),
            Error::NotRegularFile => f.write_str("database path is not a regular file"),
            Error::Locked => f.write_str(
                "database file is locked by another process; dcrwallet must be stopped first",
            ),
            Error::LockUnavailable(e) => write!(
                f,
                "cannot lock the database file on this filesystem ({e}); open without the lock \
                 only after making sure dcrwallet is stopped"
            ),
            Error::Io(e) => write!(f, "read error: {e}"),
            Error::EmptyFile => f.write_str("database file is empty"),
            Error::Invalid => f.write_str("invalid database"),
            Error::VersionMismatch => f.write_str("version mismatch"),
            Error::Checksum => f.write_str("checksum error"),
            Error::FileTooSmall { len, page_size } => write!(
                f,
                "file size too small ({len} bytes, page size {page_size})"
            ),
            Error::UnsupportedPageSize(n) => write!(f, "unsupported page size {n}"),
            Error::Corrupt { page, detail } => write!(f, "corrupt database: page {page}: {detail}"),
            Error::Changed { what } => match what {
                Change::Length { was, now } => write!(
                    f,
                    "database file changed while being read: length {was} is now {now}"
                ),
                Change::MetaPage(n) => write!(
                    f,
                    "database file changed while being read: meta page {n} differs"
                ),
            },
        }
    }
}

impl fmt::Display for Corruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Corruption::PageOutOfBounds {
                end_page,
                file_pages,
            } => write!(
                f,
                "reference needs pages up to {end_page} but the file has {file_pages}"
            ),
            Corruption::PageTooLarge { bytes, max } => {
                write!(
                    f,
                    "logical page of {bytes} bytes exceeds the {max}-byte limit"
                )
            }
            Corruption::PageIdMismatch { found } => {
                write!(f, "page header identifies as page {found}")
            }
            Corruption::UnexpectedPageType { flags } => {
                write!(f, "unexpected page type/flags {flags:#06x}")
            }
            Corruption::ElementCountOutOfPage { count } => {
                write!(f, "{count} element headers do not fit in the page")
            }
            Corruption::ElementOutOfPage { index } => {
                write!(f, "element {index} points outside the page")
            }
            Corruption::EmptyBranch => f.write_str("branch page has no elements"),
            Corruption::TreeTooDeep => write!(
                f,
                "bucket tree deeper than {} levels",
                crate::MAX_TREE_DEPTH
            ),
            Corruption::Cycle { pgid } => write!(f, "cycle: page {pgid} is its own ancestor"),
            Corruption::PageReferencedTwice { pgid } => {
                write!(f, "page {pgid} is referenced twice")
            }
            Corruption::KeysOutOfOrder => f.write_str("keys out of order"),
            Corruption::TooManyEmptyLeaves => {
                f.write_str("cursor skipped the same empty leaf twice")
            }
            Corruption::BucketValueTooShort { len } => {
                write!(
                    f,
                    "nested bucket value is {len} bytes, shorter than its header"
                )
            }
            Corruption::InlinePageNotLeaf { flags } => {
                write!(f, "inline bucket page has flags {flags:#06x}, not a leaf")
            }
            Corruption::InlinePageAccess { pgid } => {
                write!(f, "inline bucket refers to page {pgid}")
            }
            Corruption::InlineRootBucket => f.write_str("root bucket has root page 0"),
            Corruption::FreelistOutOfPage { count } => {
                write!(f, "freelist of {count} ids does not fit in its page")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) | Error::Open(e) | Error::LockUnavailable(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Shorthand for an [`Error::Corrupt`].
pub(crate) fn corrupt(page: u64, detail: Corruption) -> Error {
    Error::Corrupt { page, detail }
}
