// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Tests that need no Go toolchain: the checked-in fixtures against the Go
//! oracle's recorded dumps and bucket stats, the file lock, opening
//! non-files, the error classes, change detection, threads, and the read
//! API.

// Test code; counters and offsets cannot overflow here.
#![allow(clippy::arithmetic_side_effects)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use purse_boltread::{Db, Error, ErrorClass, Item, OpenOptions, Source, dump};

#[test]
fn fixtures_match_recorded_go_dumps_and_stats() {
    let dir = common::scratch("fixtures-recorded");
    let digests = common::recorded_digests();
    let stats = common::recorded_stats();
    // Every upstream fixture, both frozen ones, and the frozen fixture's
    // older snapshot.
    assert_eq!(digests.len(), common::upstream_fixtures().len() + 3);
    assert_eq!(stats.len(), common::upstream_fixtures().len() + 2);
    for d in &digests {
        let bytes = common::fixture_bytes(&d.file);
        let path = dir.join(d.file.replace(['/', '@'], "_"));
        std::fs::write(&path, &bytes).expect("write fixture copy");
        let db = Db::open(&path).unwrap_or_else(|e| panic!("{}: {e}", d.file));
        assert_eq!(db.meta().txid, d.txid, "{}: txid", d.file);
        assert!(db.metas()[db.meta_page() as usize].1);
        if d.file.ends_with("@meta0") {
            assert_eq!(db.meta_page(), 0);
        }
        let mut out = Vec::new();
        let s = dump(&db, &mut out).unwrap_or_else(|e| panic!("{}: {e}", d.file));
        assert_eq!(s.entries, d.entries, "{}: entries", d.file);
        assert_eq!(s.sha256_hex(), d.sha256, "{}: digest", d.file);
        assert_eq!(s.bytes, out.len() as u64);
        if let Some(want) = stats.get(&d.file) {
            let paths = common::bucket_paths(&db);
            let names: BTreeSet<String> = paths.iter().map(|p| common::path_string(p)).collect();
            let recorded: BTreeSet<String> = want.keys().cloned().collect();
            assert_eq!(names, recorded, "{}: bucket paths", d.file);
            for p in &paths {
                let b = db.bucket_path(p).expect("lookup").expect("exists");
                let got = common::stats_json(&b.stats().expect("stats"));
                assert_eq!(
                    &got,
                    &want[&common::path_string(p)],
                    "{}: stats of {p:?}",
                    d.file
                );
            }
        }
        db.verify_unchanged().expect("unchanged");
        drop(db);
        assert_eq!(common::file_bytes(&path), bytes, "{}: file changed", d.file);
    }
}

/// The frozen fixtures hold what the corruption tests rely on.
#[test]
fn frozen_fixtures_have_the_structures_tests_need() {
    let db = common::open_bytes(common::frozen()).expect("open");
    let root = db.root().stats().expect("stats");
    assert!(root.inline_bucket_n > 0, "{root:?}");
    assert!(root.leaf_overflow_n > 0, "{root:?}");
    assert!(root.branch_page_n > 1, "{root:?}");
    let deep = common::open_bytes(common::fixture_bytes("frozen-deep-4k.db")).expect("open");
    let mut deepest = 0;
    for p in common::bucket_paths(&deep) {
        let s = deep
            .bucket_path(&p)
            .expect("lookup")
            .expect("exists")
            .stats()
            .expect("stats");
        if s.bucket_n == 1 {
            deepest = deepest.max(s.depth);
        }
    }
    assert!(deepest >= 4, "deepest bucket tree has {deepest} levels");
}

#[cfg(unix)]
#[test]
fn lock_held_by_another_handle_fails_promptly() {
    use std::fs::File;
    let dir = common::scratch("lock-local");
    let path = dir.join("frozen.db");
    std::fs::write(&path, common::frozen()).expect("write");
    // A separate open file description, as another process would have.
    let holder = File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    holder.try_lock().expect("exclusive lock");
    let t = Instant::now();
    let err = Db::open(&path).expect_err("must fail while locked");
    assert!(matches!(err, Error::Locked), "got {err}");
    assert_eq!(err.class(), ErrorClass::Locked);
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "took {:?}",
        t.elapsed()
    );
    // Without locking it opens (the option exists for callers that hold
    // the lock themselves).
    Db::open_with(&path, &OpenOptions::new().lock(false)).expect("unlocked open");
    holder.unlock().expect("unlock");
    // Shared readers coexist.
    let a = Db::open(&path).expect("open after release");
    let b = Db::open(&path).expect("second reader");
    assert_eq!(a.meta(), b.meta());
    // A writer (exclusive) cannot get in while a reader holds the lock.
    let writer = File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    assert!(writer.try_lock().is_err(), "reader must block writers");
    drop((a, b));
    writer.try_lock().expect("lock after readers closed");
}

/// Windows: the reader denies write sharing instead of locking.
#[cfg(windows)]
#[test]
fn lock_held_by_another_handle_fails_promptly() {
    let dir = common::scratch("lock-local");
    let path = dir.join("frozen.db");
    std::fs::write(&path, common::frozen()).expect("write");
    // A handle open for writing, as dcrwallet's is.
    let holder = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    let t = Instant::now();
    let err = Db::open(&path).expect_err("must fail while a writer has the file open");
    assert!(matches!(err, Error::Locked), "got {err}");
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "took {:?}",
        t.elapsed()
    );
    Db::open_with(&path, &OpenOptions::new().lock(false)).expect("unlocked open");
    drop(holder);
    let a = Db::open(&path).expect("open after release");
    let b = Db::open(&path).expect("second reader");
    assert_eq!(a.meta(), b.meta());
    // A writer cannot open the file while a reader has it.
    let e = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect_err("reader must keep writers out");
    assert_eq!(e.raw_os_error(), Some(32), "{e}");
    drop((a, b));
    std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .expect("writer after readers closed");
}

fn collect(items: impl Iterator<Item = purse_boltread::Result<Item>>) -> Vec<Item> {
    items.map(|r| r.expect("item")).collect()
}

/// Walks every bucket of the frozen fixtures and checks the cursor, get and
/// bucket lookups against one another.
#[test]
fn api_is_self_consistent_on_frozen_fixtures() {
    for name in ["frozen-4k.db", "frozen-deep-4k.db"] {
        let db = common::open_bytes(common::fixture_bytes(name)).expect("open");
        assert_eq!(db.page_size(), 4096);
        assert_eq!(db.meta_page(), 1, "{name}: newest meta is page 1");
        check_api(&db, name);
    }
}

fn check_api(db: &Db, name: &str) {
    let mut stack = vec![(Vec::<Vec<u8>>::new(), db.root())];
    while let Some((path, b)) = stack.pop() {
        let fwd = collect(b.iter());
        let mut c = b.cursor();
        let mut back = Vec::new();
        let mut it = c.last().expect("last");
        while let Some(item) = it {
            back.push(item);
            it = c.prev().expect("prev");
        }
        back.reverse();
        assert_eq!(fwd, back, "{name}: bucket {path:?}");
        for w in fwd.windows(2) {
            assert!(w[0].key() < w[1].key());
        }
        let mut map = BTreeMap::new();
        for item in &fwd {
            map.insert(item.key().to_vec(), item.clone());
            let got = b.get(item.key()).expect("get");
            assert_eq!(got.as_deref(), item.value());
            let child = b.bucket(item.key()).expect("bucket");
            assert_eq!(child.is_some(), item.is_bucket());
            if let Some(child) = child {
                let mut p = path.clone();
                p.push(item.key().to_vec());
                let again = db.bucket_path(&p).expect("path").expect("exists");
                assert_eq!(again.root_page(), child.root_page());
                stack.push((p, child));
            }
            // Seek to an exact key finds it; to the key plus a zero byte
            // finds the successor.
            assert_eq!(
                b.cursor().seek(item.key()).expect("seek").as_ref(),
                Some(item)
            );
            let mut k = item.key().to_vec();
            k.push(0);
            let succ = b.cursor().seek(&k).expect("seek");
            assert_eq!(
                succ.map(|i| i.key().to_vec()),
                fwd.iter()
                    .find(|i| i.key() >= k.as_slice())
                    .map(|i| i.key().to_vec()),
                "successor of {k:?}"
            );
        }
        // Seek past the end: None, then Prev gives the last entry.
        let mut c = b.cursor();
        assert!(c.seek(&[0xff; 2000]).expect("seek").is_none());
        assert_eq!(c.prev().expect("prev").as_ref(), fwd.last());
        // Next at the end stays; Prev at the start returns None and
        // repositions on the first entry; direction changes keep working.
        let mut c = b.cursor();
        if c.last().expect("last").is_some() {
            assert!(c.next().expect("next").is_none());
            assert_eq!(c.prev().expect("prev").as_ref(), fwd.iter().rev().nth(1));
            let after = if fwd.len() >= 2 { fwd.last() } else { None };
            assert_eq!(c.next().expect("next").as_ref(), after);
        }
        let mut c = b.cursor();
        c.first().expect("first");
        assert!(c.prev().expect("prev").is_none());
        assert_eq!(c.next().expect("next").as_ref(), fwd.get(1));
        assert!(b.cursor().next().expect("next").is_none());
        assert!(b.cursor().prev().expect("prev").is_none());
        assert!(b.get(b"\x00no such key").expect("get").is_none());
        assert!(b.bucket(b"\x00no such bucket").expect("bucket").is_none());
    }
}

#[test]
fn open_rejects_non_databases() {
    assert!(matches!(
        common::open_bytes(Vec::new()),
        Err(Error::EmptyFile)
    ));
    assert!(matches!(
        common::open_bytes(vec![0; 100]),
        Err(Error::Invalid)
    ));
    assert!(matches!(
        common::open_bytes(vec![0; 2048]),
        Err(Error::Invalid)
    ));
    // 2049..8191 bytes: the meta-1 probe can read, bbolt falls back to the
    // OS page size and finds the file too small.
    assert!(matches!(
        common::open_bytes(vec![0; 5000]),
        Err(Error::FileTooSmall { .. })
    ));
    assert!(matches!(
        common::open_bytes(vec![0; 8192]),
        Err(Error::Invalid)
    ));
}

/// A source whose reads fail once the meta pages have been read.
struct FailingSource(Vec<u8>);

impl Source for FailingSource {
    fn size(&self) -> std::io::Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        if offset >= 2 * 4096 {
            return Err(std::io::Error::other("injected read failure"));
        }
        let at = offset as usize;
        buf.copy_from_slice(&self.0[at..at + buf.len()]);
        Ok(())
    }
}

/// Each class of error an importer distinguishes.
#[test]
fn error_classes() {
    let dir = common::scratch("error-classes");
    let class = |r: purse_boltread::Result<Db>| r.expect_err("must fail").class();
    // Cannot open: missing file.
    let missing = dir.join("missing.db");
    let e = Db::open(&missing).expect_err("missing");
    assert!(
        matches!(&e, Error::Open(io) if io.kind() == std::io::ErrorKind::NotFound),
        "{e}"
    );
    assert_eq!(e.class(), ErrorClass::CannotOpen);
    assert!(std::error::Error::source(&e).is_some());
    // Cannot open: permission denied (not testable as root).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("noperm.db");
        std::fs::write(&p, common::frozen()).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        if std::fs::File::open(&p).is_err() {
            let e = Db::open(&p).expect_err("denied");
            assert!(
                matches!(&e, Error::Open(io) if io.kind() == std::io::ErrorKind::PermissionDenied),
                "{e}"
            );
            assert_eq!(e.class(), ErrorClass::CannotOpen);
        }
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    // Not a bbolt file.
    let junk = dir.join("junk.db");
    std::fs::write(&junk, vec![0x5a; 16384]).expect("write");
    assert_eq!(class(Db::open(&junk)), ErrorClass::NotBolt);
    assert_eq!(class(common::open_bytes(Vec::new())), ErrorClass::NotBolt);
    // Unsupported: a valid meta from a different format version is
    // reported as such; so is a page size this reader refuses.
    let mut v = common::frozen();
    for page in 0..2 {
        v[page * 4096 + 20] = 3;
        common::fix_checksum(&mut v, page, 4096);
    }
    assert_eq!(class(common::open_bytes(v)), ErrorClass::Unsupported);
    // Corrupt: both metas fail their checksum.
    let mut c = common::frozen();
    c[16 + 56] ^= 1;
    c[4096 + 16 + 56] ^= 1;
    assert_eq!(class(common::open_bytes(c)), ErrorClass::Corrupt);
    // Corrupt: a structural defect found while reading.
    let mut r = common::frozen();
    for page in 0..2 {
        r[page * 4096 + 32..page * 4096 + 40].copy_from_slice(&(1u64 << 40).to_le_bytes());
        common::fix_checksum(&mut r, page, 4096);
    }
    let db = common::open_bytes(r).expect("opens");
    let e = dump(&db, &mut std::io::sink()).expect_err("root out of bounds");
    assert_eq!(e.class(), ErrorClass::Corrupt);
    // Io: a read failing after the open.
    let db = Db::from_source(FailingSource(common::frozen()), &OpenOptions::new()).expect("opens");
    let e = dump(&db, &mut std::io::sink()).expect_err("read fails");
    assert!(matches!(e, Error::Io(_)), "{e}");
    assert_eq!(e.class(), ErrorClass::Io);
    assert!(std::error::Error::source(&e).is_some());
    // Cannot lock: produced when flock is unsupported, which no local
    // filesystem here offers; the variant and its class.
    let e = Error::LockUnavailable(std::io::Error::from(std::io::ErrorKind::Unsupported));
    assert_eq!(e.class(), ErrorClass::CannotLock);
    assert!(std::error::Error::source(&e).is_some());
    assert!(e.to_string().contains("dcrwallet is stopped"));
    // Locked: see the lock tests.
    assert_eq!(Error::Locked.class(), ErrorClass::Locked);
}

/// Paths that are not regular files are refused before anything is read.
#[test]
fn not_regular_files() {
    let dir = common::scratch("not-regular");
    let sub = dir.join("a-directory");
    std::fs::create_dir(&sub).expect("mkdir");
    let e = Db::open(&sub).expect_err("directory");
    assert!(matches!(e, Error::NotRegularFile), "{e}");
    assert_eq!(e.class(), ErrorClass::CannotOpen);
    #[cfg(unix)]
    {
        let file = dir.join("frozen.db");
        std::fs::write(&file, common::frozen()).expect("write");
        let to_dir = dir.join("link-to-dir");
        std::os::unix::fs::symlink(&sub, &to_dir).expect("symlink");
        assert!(matches!(Db::open(&to_dir), Err(Error::NotRegularFile)));
        let to_file = dir.join("link-to-file");
        std::os::unix::fs::symlink(&file, &to_file).expect("symlink");
        Db::open(&to_file).expect("a symlink to a file is followed");
        // A Unix socket (std can make one; a FIFO it cannot).
        let sock = dir.join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
        assert!(matches!(Db::open(&sock), Err(Error::NotRegularFile)));
    }
}

/// `verify_unchanged` detects a meta page rewritten, the file grown or
/// shrunk after the open, and passes when nothing changed.
#[test]
fn verify_unchanged_detects_writes() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = common::scratch("verify-unchanged");
    let path = dir.join("frozen.db");
    type Change = (&'static str, fn(&std::fs::File));
    let cases: [Change; 4] = [
        ("untouched", |_| {}),
        ("meta 0 rewritten", |f| {
            let mut f = f;
            f.seek(SeekFrom::Start(16 + 48)).expect("seek");
            f.write_all(&[9]).expect("write");
        }),
        ("meta 1 page padding rewritten", |f| {
            let mut f = f;
            f.seek(SeekFrom::Start(4096 + 2000)).expect("seek");
            f.write_all(&[1]).expect("write");
        }),
        ("appended", |f| f.set_len(300_000).expect("grow")),
    ];
    for (name, change) in cases {
        std::fs::write(&path, common::frozen()).expect("write");
        // Without the lock, so that the file can be written meanwhile on
        // Windows too.
        let db = Db::open_with(&path, &OpenOptions::new().lock(false)).expect("open");
        let f = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open for writing");
        change(&f);
        drop(f);
        let r = db.verify_unchanged();
        if name == "untouched" {
            r.expect("unchanged");
        } else {
            let e = r.expect_err(name);
            assert!(matches!(e, Error::Changed { .. }), "{name}: {e}");
            assert_eq!(e.class(), ErrorClass::Changed);
        }
    }
    std::fs::write(&path, common::frozen()).expect("write");
    let db = Db::open_with(&path, &OpenOptions::new().lock(false)).expect("open");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|f| f.set_len(200_000))
        .expect("truncate");
    assert!(matches!(db.verify_unchanged(), Err(Error::Changed { .. })));
}

/// Several threads share one `Db` and read concurrently.
#[test]
fn concurrent_readers_share_one_db() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Db>();
    send_sync::<purse_boltread::Bucket<'static>>();
    send_sync::<purse_boltread::Cursor<'static>>();
    let bytes = common::fixture_bytes("upstream/v1.db.gz");
    let want = common::recorded_digest("upstream/v1.db.gz");
    // A small cache, so that threads race on eviction too.
    let db = Db::from_source(bytes, &OpenOptions::new().cache_bytes(64 << 10)).expect("open");
    let paths = common::bucket_paths(&db);
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for t in 0..8 {
            let (db, paths, want) = (&db, &paths, &want);
            handles.push(s.spawn(move || {
                for round in 0..5 {
                    if (t + round) % 2 == 0 {
                        let d = dump(db, &mut std::io::sink()).expect("dump");
                        assert_eq!(&d.sha256_hex(), want);
                    } else {
                        let mut n = 0;
                        for p in paths {
                            let b = db.bucket_path(p).expect("lookup").expect("exists");
                            n += collect(b.iter()).len();
                        }
                        assert!(n > 3000);
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
    });
}
