// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Opening a file: the lock, page-size discovery, meta validation and
//! selection (bbolt `db.go` `Open`, `getPageSize`, `mmap`, `meta`), and the
//! page cache.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::bucket::Bucket;
use crate::error::{Change, Corruption, Error, Result, corrupt};
use crate::meta::{Meta, PGID_NO_FREELIST};
use crate::page::{
    BRANCH_PAGE_FLAG, FREELIST_PAGE_FLAG, LEAF_PAGE_FLAG, META_PAGE_FLAG, PAGE_HEADER_SIZE,
    PageRef, le_u16, le_u32, le_u64,
};
use crate::source::Source;

/// The smallest page size the reader accepts; it also refuses page sizes
/// that are not powers of two.
///
/// This is a deliberate deviation from bbolt, mandated by spec 01 section
/// 4.3: bbolt accepts whatever `Options.PageSize` a program chose (and the
/// size recorded in a valid meta 0, even 0), whereas dcrwallet always
/// creates files with the OS page size (4096, 16384 or 65536 bytes). 1024
/// is where bbolt's own probe for meta page 1 starts.
pub const MIN_PAGE_SIZE: u32 = 1024;

/// The deepest bucket tree the reader follows, in pages from the bucket's
/// root to a leaf; a deeper one is [`Corruption::TreeTooDeep`].
///
/// bbolt has no such limit (it reads a hand-made chain of any length), so
/// this is a deliberate limit of the reader. bbolt's writer keeps at least
/// two children per branch page, so a tree it writes this deep would need
/// more pages than any file can hold.
pub const MAX_TREE_DEPTH: usize = 64;

/// The default page cache budget in bytes.
pub const DEFAULT_CACHE_BYTES: usize = 64 << 20;

/// The default largest logical page (a page with its overflow pages) the
/// reader allocates: 256 MiB.
///
/// bbolt stores a key/value pair larger than a page on overflow pages of one
/// logical page, so the largest page is set by the largest value. dcrwallet
/// stores transactions, headers, filters and small records, none near a
/// megabyte, so a real `wallet.db` stays orders of magnitude below this.
/// The limit exists because a page's overflow count is otherwise bounded
/// only by the file's apparent length, which a sparse file can make
/// enormous without using disk.
pub const DEFAULT_MAX_PAGE_BYTES: usize = 256 << 20;

/// How many bytes of each meta page [`Db::verify_unchanged`] compares: the
/// whole page for every real page size.
const META_CAPTURE: u64 = 64 << 10;

/// The page size bbolt falls back to when neither meta page validates but
/// one could be read: the OS page size of the host, 4096 on x86-64 and on
/// most arm64 Linux, 16384 on Apple silicon. The reader takes 4096 on every
/// host. A file that reaches this fallback fails to open either way; only
/// which error it reports depends on the value: one of at least 8 KiB but
/// under twice the host's page size is too small to bbolt on a host with
/// larger pages, and invalid here. No comparison with Go reaches such a
/// file (the damaged files compared are 256 KiB long, or keep a valid
/// meta 0), so the tests agree with bbolt on every host.
const FALLBACK_PAGE_SIZE: u32 = 4096;

/// How the probe in bbolt's `getPageSize` reads meta 0: one 4 KiB buffer.
const PROBE_LEN: u64 = 0x1000;

/// Options for opening a database.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    lock: bool,
    cache_bytes: usize,
    max_page_bytes: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            lock: true,
            cache_bytes: DEFAULT_CACHE_BYTES,
            max_page_bytes: DEFAULT_MAX_PAGE_BYTES,
        }
    }
}

impl OpenOptions {
    /// The defaults: take the file lock, 64 MiB page cache, 256 MiB pages
    /// at most.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether [`Db::open_with`] takes the file lock (default `true`). An
    /// in-memory [`Source`] is never locked.
    ///
    /// Without the lock nothing keeps dcrwallet from writing the file while
    /// it is read: turn it off only when dcrwallet is known to be stopped
    /// (for example after [`Error::LockUnavailable`] on a filesystem without
    /// locks), and call [`Db::verify_unchanged`] at the end.
    pub fn lock(mut self, lock: bool) -> Self {
        self.lock = lock;
        self
    }

    /// The page cache budget in bytes (default [`DEFAULT_CACHE_BYTES`]; 0
    /// disables caching).
    pub fn cache_bytes(mut self, bytes: usize) -> Self {
        self.cache_bytes = bytes;
        self
    }

    /// The largest logical page the reader allocates (default
    /// [`DEFAULT_MAX_PAGE_BYTES`], whose documentation gives the reasoning).
    /// A larger page is reported as [`Corruption::PageTooLarge`].
    pub fn max_page_bytes(mut self, bytes: usize) -> Self {
        self.max_page_bytes = bytes;
        self
    }
}

/// A bounded cache of logical pages with first-in, first-out eviction:
/// the oldest pages go first until the newcomer fits, so the cache never
/// holds more than its budget. Pages still in use stay alive through their
/// `Arc`s.
struct Cache {
    pages: HashMap<u64, Arc<Vec<u8>>>,
    order: VecDeque<u64>,
    bytes: usize,
    budget: usize,
}

impl Cache {
    fn get(&self, id: u64) -> Option<Arc<Vec<u8>>> {
        self.pages.get(&id).cloned()
    }

    fn insert(&mut self, id: u64, page: Arc<Vec<u8>>) {
        let len = page.len();
        if len > self.budget || self.pages.contains_key(&id) {
            return;
        }
        while self.bytes.saturating_add(len) > self.budget {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(p) = self.pages.remove(&old) {
                self.bytes = self.bytes.saturating_sub(p.len());
            }
        }
        self.bytes = self.bytes.saturating_add(len);
        self.pages.insert(id, page);
        self.order.push_back(id);
    }
}

/// Which pages a read keeps in the page cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keep {
    /// Every page (lookups: `get`, `bucket`, `first`, `last`, `seek`).
    All,
    /// Branch pages only (cursor steps, which read each leaf once in a
    /// scan and would only push useful pages out).
    Branches,
    /// Nothing (whole-tree walks, which read every page exactly once).
    Nothing,
}

/// A read-only bbolt database.
///
/// The snapshot is the one bbolt's read transactions see: the newest meta
/// page that validates. Opening reads only the meta pages; every other page
/// is read on demand with positional reads and kept in a small cache.
///
/// `Db` is `Send` and `Sync`: buckets, cursors and dumps may run on several
/// threads at once over one shared `Db` (the page cache is behind a mutex).
///
/// **Changes after opening.** Pages are read lazily, so the snapshot holds
/// only while nothing writes the file. bbolt writers (dcrwallet) honour the
/// lock: on Unix the shared `flock` keeps them out, on Windows the denied
/// write sharing does. Nothing keeps out a writer that ignores the lock
/// (`cp` over the file, a sync tool, a writer on another NFS client), and
/// with [`OpenOptions::lock(false)`](OpenOptions::lock) nothing is kept out
/// at all. Such a change can surface as [`Error::Io`], as
/// [`Error::Corrupt`], or not at all; call [`Db::verify_unchanged`] after
/// reading everything needed to detect it.
pub struct Db {
    src: Box<dyn Source>,
    len: u64,
    page_size: u32,
    meta: Meta,
    meta_page: u64,
    metas: [(Meta, bool); 2],
    /// The leading bytes of meta pages 0 and 1 as read at open.
    meta_bytes: [Vec<u8>; 2],
    max_page_bytes: usize,
    cache: Mutex<Cache>,
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db")
            .field("len", &self.len)
            .field("page_size", &self.page_size)
            .field("meta_page", &self.meta_page)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// Fails unless `meta` describes a regular file.
#[cfg(any(unix, windows))]
fn regular(meta: std::io::Result<std::fs::Metadata>) -> Result<()> {
    match meta {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(Error::NotRegularFile),
        Err(e) => Err(Error::Open(e)),
    }
}

/// Opens the file read-only, taking a shared lock with one non-blocking
/// attempt when `lock` is set.
#[cfg(unix)]
fn open_file(path: &Path, lock: bool) -> Result<std::fs::File> {
    // Checked before opening so that a FIFO cannot block the open, and
    // again on the handle, which is authoritative.
    regular(std::fs::metadata(path))?;
    let file = std::fs::File::open(path).map_err(Error::Open)?;
    regular(file.metadata())?;
    if lock {
        // bbolt read-write opens (dcrwallet) hold flock(LOCK_EX); a shared
        // flock conflicts with exactly that and not with other read-only
        // readers, which take LOCK_SH. std's lock is flock(2) here.
        match file.try_lock_shared() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Locked),
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::LockUnavailable(e)),
        }
    }
    Ok(file)
}

/// Opens the file read-only; with `lock`, denies write sharing.
///
/// Rust's `File::try_lock_shared` locks bytes [0, 2^64-1) on Windows, which
/// does not overlap the single byte at 2^64-1 that bbolt locks, and locking
/// that byte needs a raw `LockFileEx` call. Instead the file is opened
/// without write sharing: the open then fails while any process (dcrwallet)
/// has the file open for writing, and dcrwallet cannot open it for writing
/// while this handle lives.
#[cfg(windows)]
fn open_file(path: &Path, lock: bool) -> Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    regular(std::fs::metadata(path))?;
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    if lock {
        o.share_mode(FILE_SHARE_READ);
    }
    let file = match o.open(path) {
        Err(e) if lock && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
            return Err(Error::Locked);
        }
        Err(e) => return Err(Error::Open(e)),
        Ok(file) => file,
    };
    regular(file.metadata())?;
    Ok(file)
}

/// bbolt's `meta.validate` on a decoded meta.
fn check(m: &(Meta, u64)) -> Result<()> {
    m.0.validate(m.1)
}

impl Db {
    /// Opens the file at `path` read-only with the default options: a
    /// single non-blocking attempt at the shared file lock (failing with
    /// [`Error::Locked`] while dcrwallet has the file open), then the meta
    /// pages. The file is never written. The path must name a regular file
    /// (symbolic links are followed).
    #[cfg(any(unix, windows))]
    pub fn open(path: impl AsRef<Path>) -> Result<Db> {
        Self::open_with(path, &OpenOptions::default())
    }

    /// Opens the file at `path` read-only with `options`.
    #[cfg(any(unix, windows))]
    pub fn open_with(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Db> {
        let file = open_file(path.as_ref(), options.lock)?;
        // The lock, if taken, lives as long as the file handle, which the
        // Db owns.
        Self::from_source(file, options)
    }

    /// Opens a database held by `src` (a file opened by the caller, or bytes
    /// in memory). No lock is taken.
    pub fn from_source(src: impl Source + 'static, options: &OpenOptions) -> Result<Db> {
        let src: Box<dyn Source> = Box::new(src);
        let len = src.size()?;
        if len == 0 {
            return Err(Error::EmptyFile);
        }
        let page_size = page_size_probe(src.as_ref(), len)?;
        if len < u64::from(page_size).saturating_mul(2) {
            return Err(Error::FileTooSmall { len, page_size });
        }
        if page_size < MIN_PAGE_SIZE || !page_size.is_power_of_two() {
            return Err(Error::UnsupportedPageSize(page_size));
        }
        // Both meta pages lie in the file: len >= 2 * page_size.
        let meta_bytes = read_meta_pages(src.as_ref(), page_size)?;
        let decode = |b: &[u8]| Meta::decode_page(b).ok_or(Error::Invalid);
        let (m0, m1) = (decode(&meta_bytes[0])?, decode(&meta_bytes[1])?);
        let (v0, v1) = (check(&m0), check(&m1));
        if let (Err(e), Err(_)) = (&v0, &v1) {
            // bbolt returns meta 0's error when both fail.
            return Err(match e {
                Error::VersionMismatch => Error::VersionMismatch,
                Error::Checksum => Error::Checksum,
                _ => Error::Invalid,
            });
        }
        let metas = [(m0.0, v0.is_ok()), (m1.0, v1.is_ok())];
        // bbolt's `db.meta()`: the meta with the larger txid if it
        // validates (meta 0 on a tie), otherwise the other one.
        let (a, b) = if m1.0.txid > m0.0.txid {
            (1, 0)
        } else {
            (0, 1)
        };
        let meta_page = if metas[a].1 { a } else { b };
        Ok(Db {
            src,
            len,
            page_size,
            meta: metas[meta_page].0,
            meta_page: meta_page as u64,
            metas,
            meta_bytes,
            max_page_bytes: options.max_page_bytes,
            cache: Mutex::new(Cache {
                pages: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
                budget: options.cache_bytes,
            }),
        })
    }

    /// Checks that the file still has the length and the meta pages it had
    /// at open: re-reads the size and the raw bytes of both meta pages and
    /// fails with [`Error::Changed`] if anything differs. Every bbolt commit
    /// rewrites a meta page, so this detects a commit made since the open
    /// (by a writer that ignored or bypassed the lock); a caller runs it
    /// after reading everything it needs.
    pub fn verify_unchanged(&self) -> Result<()> {
        let now = self.src.size()?;
        if now != self.len {
            return Err(Error::Changed {
                what: Change::Length { was: self.len, now },
            });
        }
        let pages = read_meta_pages(self.src.as_ref(), self.page_size)?;
        for (n, (was, now)) in self.meta_bytes.iter().zip(pages.iter()).enumerate() {
            if was != now {
                return Err(Error::Changed {
                    what: Change::MetaPage(n as u64),
                });
            }
        }
        Ok(())
    }

    /// The page size in use (from the meta page that bbolt's probe found).
    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    /// The file length in bytes, as found at open.
    pub fn file_len(&self) -> u64 {
        self.len
    }

    /// The number of whole pages in the file.
    pub fn file_pages(&self) -> u64 {
        self.len
            .checked_div(u64::from(self.page_size))
            .unwrap_or_default()
    }

    /// The meta in use: the snapshot every read sees.
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    /// Which meta page (0 or 1) is in use.
    pub fn meta_page(&self) -> u64 {
        self.meta_page
    }

    /// Both meta pages as decoded, each with whether it validates.
    pub fn metas(&self) -> [(Meta, bool); 2] {
        self.metas
    }

    /// The root bucket, whose entries are the top-level buckets.
    pub fn root(&self) -> Bucket<'_> {
        Bucket::root(self, self.meta.root, self.meta.sequence)
    }

    /// The top-level bucket `name`, if it exists.
    pub fn bucket(&self, name: &[u8]) -> Result<Option<Bucket<'_>>> {
        self.root().bucket(name)
    }

    /// The bucket reached by following `path` from the root, if it exists
    /// (an empty path is the root bucket).
    pub fn bucket_path<P: AsRef<[u8]>>(&self, path: &[P]) -> Result<Option<Bucket<'_>>> {
        let mut b = self.root();
        for name in path {
            match b.bucket(name.as_ref())? {
                Some(child) => b = child,
                None => return Ok(None),
            }
        }
        Ok(Some(b))
    }

    /// Reads the freelist page of the snapshot: the sorted ids it lists, or
    /// `None` when the meta records no freelist page. bbolt does not load the
    /// freelist for a read-only open and nothing else in this crate needs
    /// it; this is for verification reports. Like bbolt, the ids are not
    /// checked against the high-water mark.
    pub fn freelist(&self) -> Result<Option<Vec<u64>>> {
        let id = self.meta.freelist;
        if id == PGID_NO_FREELIST {
            return Ok(None);
        }
        let page = self.read_page(id, false, Keep::Nothing)?;
        if page.flags & FREELIST_PAGE_FLAG == 0 {
            return Err(corrupt(
                id,
                Corruption::UnexpectedPageType { flags: page.flags },
            ));
        }
        let b = page.bytes();
        let (count, start) = if page.count == 0xFFFF {
            let n = le_u64(b, PAGE_HEADER_SIZE)
                .ok_or_else(|| corrupt(id, Corruption::FreelistOutOfPage { count: u64::MAX }))?;
            (n, PAGE_HEADER_SIZE.saturating_add(8))
        } else {
            (u64::from(page.count), PAGE_HEADER_SIZE)
        };
        let too_big = || corrupt(id, Corruption::FreelistOutOfPage { count });
        let n = usize::try_from(count).map_err(|_| too_big())?;
        let body = n
            .checked_mul(8)
            .and_then(|len| b.get(start..start.checked_add(len)?))
            .ok_or_else(too_big)?;
        let mut ids: Vec<u64> = body
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_le_bytes(*c))
            .collect();
        ids.sort_unstable();
        Ok(Some(ids))
    }

    /// Reads logical page `id` (with its overflow pages). With `check`, the
    /// header must name `id` and carry exactly one page type, as bbolt's
    /// `Tx.page` asserts. `keep` says whether a page read from the file is
    /// kept in the cache.
    pub(crate) fn read_page(&self, id: u64, check: bool, keep: Keep) -> Result<PageRef> {
        if let Some(buf) = self.cache.lock().ok().and_then(|c| c.get(id)) {
            return self.decode(buf, id, check);
        }
        let ps = u64::from(self.page_size);
        let file_pages = self.file_pages();
        let oob = |end_page: u64| {
            corrupt(
                id,
                Corruption::PageOutOfBounds {
                    end_page,
                    file_pages,
                },
            )
        };
        let first_end = id.checked_add(1).ok_or_else(|| oob(u64::MAX))?;
        if first_end > file_pages {
            return Err(oob(first_end));
        }
        // id < file_pages, so these products are below the file length.
        let offset = id.saturating_mul(ps);
        let mut header = [0u8; PAGE_HEADER_SIZE];
        self.src.read_exact_at(&mut header, offset)?;
        let overflow = le_u32(&header, 12).unwrap_or_default();
        let end_page = first_end
            .checked_add(u64::from(overflow))
            .ok_or_else(|| oob(u64::MAX))?;
        if end_page > file_pages {
            return Err(oob(end_page));
        }
        // Bounded by the file length and by the page limit.
        let bytes = end_page.saturating_sub(id).saturating_mul(ps);
        let max = self.max_page_bytes as u64;
        if bytes > max {
            return Err(corrupt(id, Corruption::PageTooLarge { bytes, max }));
        }
        let total = usize::try_from(bytes).map_err(|_| oob(end_page))?;
        let mut buf = Vec::new();
        // A failed allocation (a huge overflow count in a sparse file) is an
        // error, not an abort.
        buf.try_reserve_exact(total)
            .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::OutOfMemory, e)))?;
        buf.resize(total, 0);
        self.src.read_exact_at(&mut buf, offset)?;
        let buf = Arc::new(buf);
        let page = self.decode(buf.clone(), id, check)?;
        let wanted = match keep {
            Keep::All => true,
            Keep::Branches => page.flags == BRANCH_PAGE_FLAG,
            Keep::Nothing => false,
        };
        if wanted && let Ok(mut c) = self.cache.lock() {
            c.insert(id, buf);
        }
        Ok(page)
    }

    fn decode(&self, buf: Arc<Vec<u8>>, id: u64, check: bool) -> Result<PageRef> {
        if check {
            let found = le_u64(&buf, 0).unwrap_or_default();
            if found != id {
                return Err(corrupt(id, Corruption::PageIdMismatch { found }));
            }
            let flags = le_u16(&buf, 8).unwrap_or_default();
            if !matches!(
                flags,
                BRANCH_PAGE_FLAG | LEAF_PAGE_FLAG | META_PAGE_FLAG | FREELIST_PAGE_FLAG
            ) {
                return Err(corrupt(id, Corruption::UnexpectedPageType { flags }));
            }
        }
        PageRef::file_page(buf, id)
    }

    /// Reads a page of a bucket tree: checked as by `Tx.page`, and then
    /// required to be a branch or a leaf (bbolt panics in `Cursor.search`
    /// on anything else, and misreads it elsewhere), with a non-empty
    /// branch.
    pub(crate) fn tree_page(&self, id: u64, keep: Keep) -> Result<PageRef> {
        let page = self.read_page(id, true, keep)?;
        match page.flags {
            LEAF_PAGE_FLAG => Ok(page),
            BRANCH_PAGE_FLAG if page.count > 0 => Ok(page),
            BRANCH_PAGE_FLAG => Err(corrupt(id, Corruption::EmptyBranch)),
            flags => Err(corrupt(id, Corruption::UnexpectedPageType { flags })),
        }
    }
}

/// Reads `len` bytes at `pos`, or as many as the file has, into a zeroed
/// buffer of `PROBE_LEN` bytes; returns the buffer and the byte count.
fn read_probe(src: &dyn Source, file_len: u64, pos: u64) -> Result<(Vec<u8>, u64)> {
    let n = file_len.saturating_sub(pos).min(PROBE_LEN);
    let mut buf = vec![0u8; PROBE_LEN as usize];
    // n <= PROBE_LEN.
    let dst = buf.get_mut(..n as usize).unwrap_or_default();
    src.read_exact_at(dst, pos)?;
    Ok((buf, n))
}

/// bbolt's `getPageSize`: meta 0's page size if meta 0 validates; else the
/// page size of the first valid meta found at 1 KiB << i for i in 0..=14;
/// else the OS page size if either probe could read its bytes; else
/// `ErrInvalid`.
fn page_size_probe(src: &dyn Source, len: u64) -> Result<u32> {
    let mut can_read = false;
    let (buf, n) = read_probe(src, len, 0)?;
    if n == PROBE_LEN {
        can_read = true;
        if let Some(m) = Meta::decode_page(&buf)
            && check(&m).is_ok()
        {
            return Ok(m.0.page_size);
        }
    }
    for i in 0..=14u32 {
        let pos = 1024u64 << i;
        // bbolt: `if pos >= fileSize-1024 { break }` in int64.
        if len.checked_sub(1024).is_none_or(|limit| pos >= limit) {
            break;
        }
        // pos < len - 1024: the read is full, or short ending at EOF,
        // which bbolt accepts.
        let (buf, _) = read_probe(src, len, pos)?;
        can_read = true;
        if let Some(m) = Meta::decode_page(&buf)
            && check(&m).is_ok()
        {
            return Ok(m.0.page_size);
        }
    }
    if can_read {
        Ok(FALLBACK_PAGE_SIZE)
    } else {
        Err(Error::Invalid)
    }
}

/// The leading bytes (up to 64 KiB) of meta pages 0 and 1. The caller has
/// checked that the file holds two pages.
fn read_meta_pages(src: &dyn Source, page_size: u32) -> Result<[Vec<u8>; 2]> {
    let ps = u64::from(page_size);
    // At most 64 KiB, so the cast is lossless.
    let n = ps.min(META_CAPTURE) as usize;
    let mut out = [vec![0u8; n], vec![0u8; n]];
    for (i, buf) in out.iter_mut().enumerate() {
        src.read_exact_at(buf, ps.saturating_mul(i as u64))?;
    }
    Ok(out)
}
