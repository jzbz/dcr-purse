// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Helpers shared by the integration tests.

#![allow(dead_code)]
// each test binary uses a subset
// Test code building files at fixed offsets; an overflow here is a test
// bug and panics loudly in debug builds.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use purse_boltread::{BucketStats, Db, Item, OpenOptions};
use purse_testutil::serde_json::{self, Value, json};

/// This crate's `tests/data` directory.
pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// A fresh, empty scratch directory under cargo's target tmpdir.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-boltread")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Decompresses a gzip file with the pure-Rust backend.
pub fn gunzip(path: &Path) -> Vec<u8> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(f)
        .read_to_end(&mut out)
        .unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
    out
}

/// The upstream dcrwallet fixtures: (name, compressed path), sorted.
pub fn upstream_fixtures() -> Vec<(String, PathBuf)> {
    let dir = data_dir().join("upstream");
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .expect("read upstream dir")
        .filter_map(|e| {
            let p = e.ok()?.path();
            let name = p.file_name()?.to_str()?.to_owned();
            name.ends_with(".db.gz").then_some((name, p))
        })
        .collect();
    v.sort();
    v
}

/// The frozen corruption-test fixture's bytes.
pub fn frozen() -> Vec<u8> {
    std::fs::read(data_dir().join("frozen-4k.db")).expect("read frozen-4k.db")
}

/// The bytes of a fixture named as in `dump-digests.txt`: a path under
/// `tests/data` (gunzipped when it ends in `.gz`), or `frozen-4k.db@meta0`,
/// the frozen fixture with meta page 1 invalidated.
pub fn fixture_bytes(name: &str) -> Vec<u8> {
    if name == "frozen-4k.db@meta0" {
        let mut b = frozen();
        b[4096 + 16] ^= 0xff;
        return b;
    }
    let src = data_dir().join(name);
    if name.ends_with(".gz") {
        gunzip(&src)
    } else {
        file_bytes(&src)
    }
}

/// One line of `dump-digests.txt`.
#[derive(Debug, Clone)]
pub struct Digest {
    pub file: String,
    pub txid: u64,
    pub entries: u64,
    pub sha256: String,
}

/// The Go oracle's digests of the fixtures, from `tests/data/dump-digests.txt`.
pub fn recorded_digests() -> Vec<Digest> {
    let text = std::fs::read_to_string(data_dir().join("dump-digests.txt")).expect("digests");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 4, "bad digest line {l:?}");
            Digest {
                file: f[0].to_owned(),
                txid: f[1].parse().expect("txid"),
                entries: f[2].parse().expect("entries"),
                sha256: f[3].to_owned(),
            }
        })
        .collect()
}

/// The recorded digest of fixture `name`.
pub fn recorded_digest(name: &str) -> String {
    recorded_digests()
        .into_iter()
        .find(|d| d.file == name)
        .unwrap_or_else(|| panic!("no recorded digest for {name}"))
        .sha256
}

/// The Go oracle's `BucketStats` of every bucket of every fixture, from
/// `tests/data/bucket-stats.json`: file -> dump-format path -> stats.
pub fn recorded_stats() -> BTreeMap<String, BTreeMap<String, Value>> {
    let text = std::fs::read_to_string(data_dir().join("bucket-stats.json")).expect("stats");
    serde_json::from_str(&text).expect("bucket-stats.json")
}

/// A bucket path in the dump's notation: `/` or `/<hex>/<hex>...`.
pub fn path_string(path: &[Vec<u8>]) -> String {
    let parts: Vec<String> = path.iter().map(|p| purse_testutil::hex(p)).collect();
    format!("/{}", parts.join("/"))
}

/// Bucket paths (name components) of every bucket below and including the
/// root, sorted.
pub fn bucket_paths(db: &Db) -> Vec<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut stack = vec![Vec::new()];
    while let Some(path) = stack.pop() {
        let b = db.bucket_path(&path).expect("lookup").expect("exists");
        for item in b.iter() {
            let item = item.expect("item");
            if item.is_bucket() {
                let mut p = path.clone();
                p.push(item.key().to_vec());
                stack.push(p);
            }
        }
        out.push(path);
    }
    out.sort();
    out
}

/// `BucketStats` in the oracle's JSON shape.
pub fn stats_json(s: &BucketStats) -> Value {
    json!({
        "branch_page_n": s.branch_page_n,
        "branch_overflow_n": s.branch_overflow_n,
        "leaf_page_n": s.leaf_page_n,
        "leaf_overflow_n": s.leaf_overflow_n,
        "key_n": s.key_n,
        "depth": s.depth,
        "branch_alloc": s.branch_alloc,
        "branch_inuse": s.branch_inuse,
        "leaf_alloc": s.leaf_alloc,
        "leaf_inuse": s.leaf_inuse,
        "bucket_n": s.bucket_n,
        "inline_bucket_n": s.inline_bucket_n,
        "inline_bucket_inuse": s.inline_bucket_inuse,
    })
}

/// Opens in-memory bytes.
pub fn open_bytes(bytes: Vec<u8>) -> purse_boltread::Result<Db> {
    Db::from_source(bytes, &OpenOptions::default())
}

/// An item as the oracle's cursor command reports it.
pub fn item_json(item: &Option<Item>) -> Value {
    match item {
        None => json!({"key": null, "value": null, "bucket": false}),
        Some(it) => json!({
            "key": purse_testutil::hex(it.key()),
            "value": it.value().map(purse_testutil::hex),
            "bucket": it.is_bucket(),
        }),
    }
}

/// A file's bytes.
pub fn file_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Recomputes the checksum of the meta on page `page` (page size `ps`).
pub fn fix_checksum(b: &mut [u8], page: usize, ps: usize) {
    let m = page * ps + 16;
    let sum = fnv1a64(&b[m..m + 56]);
    b[m + 56..m + 64].copy_from_slice(&sum.to_le_bytes());
}

/// A page header plus meta, as bbolt writes them.
pub fn meta_page(
    id: u64,
    page_size: u32,
    root: u64,
    freelist: u64,
    pgid: u64,
    txid: u64,
) -> Vec<u8> {
    let mut p = vec![0u8; 80];
    p[0..8].copy_from_slice(&id.to_le_bytes());
    p[8..10].copy_from_slice(&0x04u16.to_le_bytes());
    p[16..20].copy_from_slice(&0xED0C_DAEDu32.to_le_bytes());
    p[20..24].copy_from_slice(&2u32.to_le_bytes());
    p[24..28].copy_from_slice(&page_size.to_le_bytes());
    p[32..40].copy_from_slice(&root.to_le_bytes());
    p[48..56].copy_from_slice(&freelist.to_le_bytes());
    p[56..64].copy_from_slice(&pgid.to_le_bytes());
    p[64..72].copy_from_slice(&txid.to_le_bytes());
    let sum = fnv1a64(&p[16..72]);
    p[72..80].copy_from_slice(&sum.to_le_bytes());
    p
}

/// A database built page by page, in the layout bbolt writes.
pub struct Synth {
    pub ps: usize,
    /// Logical pages by id (whole pages, overflow included).
    pub pages: BTreeMap<u64, Vec<u8>>,
    pub root: u64,
    pub freelist: u64,
    pub txid: u64,
    /// Overwrite meta 0's checksum so that only meta 1 validates.
    pub break_meta0: bool,
}

impl Synth {
    pub fn new(ps: usize, root: u64) -> Self {
        Synth {
            ps,
            pages: BTreeMap::new(),
            root,
            freelist: u64::MAX,
            txid: 5,
            break_meta0: false,
        }
    }

    fn header(&self, id: u64, flags: u16, count: u16, body: &[u8]) -> Vec<u8> {
        let len = 16 + body.len();
        let n = len.div_ceil(self.ps).max(1);
        let mut p = vec![0u8; n * self.ps];
        p[0..8].copy_from_slice(&id.to_le_bytes());
        p[8..10].copy_from_slice(&flags.to_le_bytes());
        p[10..12].copy_from_slice(&count.to_le_bytes());
        p[12..16].copy_from_slice(&((n - 1) as u32).to_le_bytes());
        p[16..len].copy_from_slice(body);
        p
    }

    /// A leaf page of plain key/value pairs.
    pub fn leaf(&mut self, id: u64, items: &[(Vec<u8>, Vec<u8>)]) {
        let n = items.len();
        let mut elems = Vec::new();
        let mut data = Vec::new();
        for (i, (k, v)) in items.iter().enumerate() {
            let pos = 16 * (n - i) + data.len();
            elems.extend_from_slice(&0u32.to_le_bytes());
            elems.extend_from_slice(&(pos as u32).to_le_bytes());
            elems.extend_from_slice(&(k.len() as u32).to_le_bytes());
            elems.extend_from_slice(&(v.len() as u32).to_le_bytes());
            data.extend_from_slice(k);
            data.extend_from_slice(v);
        }
        elems.extend_from_slice(&data);
        let page = self.header(id, 0x02, n as u16, &elems);
        self.pages.insert(id, page);
    }

    /// A branch page.
    pub fn branch(&mut self, id: u64, children: &[(Vec<u8>, u64)]) {
        let n = children.len();
        let mut elems = Vec::new();
        let mut data = Vec::new();
        for (i, (k, child)) in children.iter().enumerate() {
            let pos = 16 * (n - i) + data.len();
            elems.extend_from_slice(&(pos as u32).to_le_bytes());
            elems.extend_from_slice(&(k.len() as u32).to_le_bytes());
            elems.extend_from_slice(&child.to_le_bytes());
            data.extend_from_slice(k);
        }
        elems.extend_from_slice(&data);
        let page = self.header(id, 0x01, n as u16, &elems);
        self.pages.insert(id, page);
    }

    /// A freelist page listing `ids` (with the 0xFFFF count escape when
    /// there are that many).
    pub fn freelist_page(&mut self, id: u64, ids: &[u64]) {
        let mut body = Vec::new();
        let count = if ids.len() >= 0xFFFF {
            body.extend_from_slice(&(ids.len() as u64).to_le_bytes());
            0xFFFF
        } else {
            ids.len() as u16
        };
        for i in ids {
            body.extend_from_slice(&i.to_le_bytes());
        }
        let page = self.header(id, 0x10, count, &body);
        self.pages.insert(id, page);
        self.freelist = id;
    }

    /// One past the last page.
    pub fn pgid(&self) -> u64 {
        self.pages
            .iter()
            .map(|(id, p)| id + (p.len() / self.ps) as u64)
            .max()
            .unwrap_or(2)
            .max(2)
    }

    fn metas(&self) -> [Vec<u8>; 2] {
        let pgid = self.pgid();
        let ps = self.ps as u32;
        let m0 = meta_page(0, ps, self.root, self.freelist, pgid, self.txid);
        let mut m1 = meta_page(1, ps, self.root, self.freelist, pgid, self.txid - 1);
        if self.break_meta0 {
            // Meta 1 then carries the snapshot.
            m1 = meta_page(1, ps, self.root, self.freelist, pgid, self.txid + 1);
        }
        let mut m0 = m0;
        if self.break_meta0 {
            m0[72] ^= 0xff;
        }
        [m0, m1]
    }

    /// The file's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.pgid() as usize * self.ps];
        for (i, m) in self.metas().iter().enumerate() {
            out[i * self.ps..i * self.ps + 80].copy_from_slice(m);
        }
        for (id, p) in &self.pages {
            let at = *id as usize * self.ps;
            out[at..at + p.len()].copy_from_slice(p);
        }
        out
    }

    /// Writes the file sparsely (only the pages are written).
    pub fn write(&self, path: &Path) {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::File::create(path).expect("create");
        f.set_len(self.pgid() * self.ps as u64).expect("set_len");
        for (i, m) in self.metas().iter().enumerate() {
            f.seek(SeekFrom::Start((i * self.ps) as u64)).expect("seek");
            f.write_all(m).expect("write");
        }
        for (id, p) in &self.pages {
            f.seek(SeekFrom::Start(id * self.ps as u64)).expect("seek");
            f.write_all(p).expect("write");
        }
    }
}

/// Opens `path` for a test that has the Go oracle open the same file while
/// the reader holds it.
///
/// On Unix that is the default open: the shared lock lets Go's read-only
/// opens in.  On Windows the default open denies write sharing, and bbolt
/// asks for write access even to read, so Go could not open the file at
/// all; there the lock is left off.  The lock itself has tests of its own.
pub fn open_beside_go(path: &Path) -> purse_boltread::Result<Db> {
    Db::open_with(path, &OpenOptions::new().lock(cfg!(unix)))
}

/// How long one case may run.  A case takes milliseconds; the margin is for
/// a slow, loaded CI runner in a debug build.
const CASE_DEADLINE: Duration = Duration::from_secs(30);

/// Aborts the process when one case runs past its deadline: a removed guard
/// in the reader then fails the suite at once instead of hanging it.
///
/// Nothing is timed until the first [`Watchdog::case`], so a test's setup,
/// which may build the Go oracle from cold, is not held to a case's deadline.
pub struct Watchdog {
    state: Arc<Mutex<Option<(String, Instant)>>>,
    stop: Arc<AtomicBool>,
}

impl Watchdog {
    pub fn new(test: &str) -> Self {
        let state: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        let test = test.to_owned();
        std::thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                let current = s
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|e| e.into_inner().clone());
                if let Some((name, since)) = current
                    && since.elapsed() > CASE_DEADLINE
                {
                    eprintln!(
                        "watchdog: {test}: case {name:?} ran over {} s; aborting",
                        CASE_DEADLINE.as_secs()
                    );
                    std::process::abort();
                }
            }
        });
        Watchdog { state, stop }
    }

    /// Starts the deadline of a new case.
    pub fn case(&self, name: &str) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some((name.to_owned(), Instant::now()));
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
