// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The backup of ADR-0018 as `purse-store` builds it (`src/backup.rs`):
//! the snapshot copy, the backup record, publication through `.partial/`,
//! and the verify path for a file handed in from outside.
//!
//! - A backup taken while another thread commits durably equals one of the
//!   states the writer committed, never a mixture, and not the last one
//!   when commits landed during the copy (the writer records the store's
//!   digest after every commit; `PURSE_STORE_BACKUP_CASES`, default 4).
//! - A power cut at every storage operation of a backup's copy, commit and
//!   compaction (clean, torn and with every write kept, as a killed
//!   process leaves the page cache): the image left either fails to
//!   verify or verifies as the complete backup, and once the commit has
//!   returned it always verifies; the source store's storage sees no
//!   operation at all. A transient storage error at every operation does
//!   the same, and the store takes further backups. `PURSE_STORE_SWEEP_STEP`
//!   (default 1) cuts at every that-many-th operation instead.
//! - A stop at every step of a file backup (how process shutdown stops it)
//!   never publishes a file, leaves the source as it was, and the next
//!   backup removes what `.partial/` holds and publishes; files a user put
//!   in the backups directory are never touched.
//! - Names are refused as ADR-0018 Decision item 4 lists, and an existing
//!   name, with nothing written; one backup runs at a time. A backup into
//!   the directory `""` (the current one) publishes and returns its file.
//! - `verify_backup` refuses missing, empty, random, truncated, bit-flipped,
//!   foreign, newer-version, wrong-version, wrong-network and edited files
//!   and inconsistent stores with an error, never a panic, never writes the
//!   file it is given and leaves nothing in its scratch directory. It
//!   refuses a directory, a device, a socket and a FIFO without blocking,
//!   and never removes the file it is given, even one lying in its scratch
//!   directory under the prefix reserved to its copies. Where a
//!   flipped bit lands in bytes nothing reads, it verifies, and then only
//!   as the original backup.
//! - The backup record round-trips, decodes only canonical encodings, and
//!   its layout is pinned.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters, sizes and offsets into small images"
)]

mod support;

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use purse_errors::Kind;
use purse_store::backup::{
    BACKUP_NAME_DOT_TEXT, BACKUP_NAME_DOTDOT_TEXT, BACKUP_NAME_EMPTY_TEXT,
    BACKUP_NAME_NOT_PORTABLE_TEXT, BACKUP_NAME_SEPARATOR_TEXT, BACKUP_RUNNING_TEXT,
    PARTIAL_NOT_DIR_TEXT, VERIFY_COPY_PREFIX, VERIFY_NOT_A_FILE_TEXT,
};
use purse_store::{
    AgendaChoice, AgendaKey, AgendaPrefs, BACKUP_RECORD_KEY, Backup, BackupInfo, BackupRecord,
    BackupStep, Codec, Error, Hash32, OpenOptions, PARTIAL_DIR, SCHEMA_VERSION, STORE_FILE_NAME,
    Schema, Store, TableId, Tickets, TxUnmined, TxUnminedRow, check_backup_name, verify_backup,
    verify_backup_on_backend,
};
use purse_testutil::SplitMix64;
use redb::{ReadableTable as _, TableDefinition};
use support::powerloss::{Cut, PowerLoss, Shared};
use support::{StoreDigest, digest, short};

/// The checked-in version-1 store: a row in every table, a chain and
/// record tables that pass both checks.
static FIXTURE: &[u8] = include_bytes!("data/schema-v1.redb");

const NET: &str = "simnet";

fn info() -> BackupInfo<'static> {
    BackupInfo {
        network: NET,
        version: "2.2.0-pre+dcr-purse.0.1.0",
        time: 1_794_990_000_123_456_789,
    }
}

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-store-backup")
        .join(format!("{label}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("scratch dir");
    p
}

fn h(rng: &mut SplitMix64) -> Hash32 {
    let mut b = [0u8; 32];
    rng.fill(&mut b);
    Hash32(b)
}

/// Writes `n` rows of each of three tables, values of a few hundred bytes
/// to a few kilobytes, in one durable commit.
fn bulk(store: &Store, rng: &mut SplitMix64, n: usize) {
    store
        .update(|tx| {
            for _ in 0..n {
                let size = 100 + rng.below(3000) as usize;
                let row = TxUnminedRow {
                    received: rng.next_u64() as i64,
                    tx: rng.bytes(size),
                };
                tx.put::<TxUnmined>(&h(rng), &row)?;
                tx.put::<Tickets>(&h(rng), &purse_store::Empty)?;
                let (a, c) = (1 + rng.below(30) as usize, 1 + rng.below(20) as usize);
                let key = AgendaKey {
                    version: rng.below(12) as u32,
                    agenda: rng.bytes(a),
                };
                tx.put::<AgendaPrefs>(&key, &AgendaChoice(rng.bytes(c)))?;
            }
            Ok::<_, Error>(())
        })
        .expect("bulk rows");
}

/// The fixture on a power-loss backend of its own, so that the test can
/// count the source's storage operations, plus `n` bulk rows.
fn source(rng: &mut SplitMix64, n: usize) -> (Store, Arc<PowerLoss>) {
    let pl = PowerLoss::new(FIXTURE.to_vec(), true);
    let store = OpenOptions::new()
        .open_on_backend(Shared(pl.clone()))
        .expect("open fixture");
    if n > 0 {
        bulk(&store, rng, n);
    }
    (store, pl)
}

fn store_digest(store: &Store) -> StoreDigest {
    let tx = store.read().expect("read");
    let d = digest(&tx);
    tx.close().expect("close");
    d
}

/// The rows of a backup file without its record: the file is copied, the
/// copy opened as a store, the record row removed, and the store digested
/// as the live one is.
fn content_digest(file: &Path) -> StoreDigest {
    let dir = scratch("content");
    let copy = dir.join(STORE_FILE_NAME);
    std::fs::copy(file, &copy).expect("copy backup");
    let store = OpenOptions::new()
        .check_integrity(true)
        .open(&copy)
        .expect("open backup copy");
    store
        .update(|tx| {
            assert!(tx.delete_raw(TableId::Meta, BACKUP_RECORD_KEY)?);
            Ok::<_, Error>(())
        })
        .expect("remove record");
    let d = store_digest(&store);
    store.close().expect("close");
    let _ = std::fs::remove_dir_all(&dir);
    d
}

fn sweep_step() -> u64 {
    std::env::var("PURSE_STORE_SWEEP_STEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

#[test]
fn a_backup_is_the_snapshot_under_a_concurrent_writer() {
    let cases: u64 = std::env::var("PURSE_STORE_BACKUP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let mut rng = SplitMix64::from_env("backup-concurrent-writer");
    for case in 0..cases {
        let seed = rng.next_u64();
        let mut r = SplitMix64(seed);
        let n = 150 + r.below(250) as usize;
        let (store, _) = source(&mut r, n);
        let dir = scratch("concurrent");
        let states: Mutex<Vec<StoreDigest>> = Mutex::new(vec![store_digest(&store)]);
        let commits = AtomicU64::new(0);
        let stop = AtomicBool::new(false);
        let backup: Backup = std::thread::scope(|s| {
            let writer = s.spawn(|| {
                let mut w = SplitMix64(seed ^ 0x5eed);
                let mut written: Vec<Hash32> = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    store
                        .update(|tx| {
                            for _ in 0..1 + w.below(4) {
                                let k = h(&mut w);
                                let size = 50 + w.below(2000) as usize;
                                tx.put::<TxUnmined>(
                                    &k,
                                    &TxUnminedRow {
                                        received: 7,
                                        tx: w.bytes(size),
                                    },
                                )?;
                                written.push(k);
                            }
                            if written.len() > 6 && w.below(2) == 0 {
                                let k = written.swap_remove(w.below(written.len() as u64) as usize);
                                assert!(tx.delete::<TxUnmined>(&k)?);
                            }
                            tx.put::<Tickets>(&h(&mut w), &purse_store::Empty)
                        })
                        .expect("writer commit");
                    // One writer: what a read sees now is exactly this commit.
                    let d = store_digest(&store);
                    states.lock().expect("states").push(d);
                    commits.fetch_add(1, Ordering::SeqCst);
                }
            });
            while commits.load(Ordering::SeqCst) < 2 {
                std::thread::yield_now();
            }
            let mut waited = false;
            let b = store
                .backup_observed(&dir, "concurrent.redb", &info(), &mut |step| {
                    if let BackupStep::TableCopied(_) = step
                        && !waited
                    {
                        // The snapshot is open: let the writer commit
                        // twice more under it.
                        waited = true;
                        let at = commits.load(Ordering::SeqCst);
                        let deadline = Instant::now() + Duration::from_secs(20);
                        while commits.load(Ordering::SeqCst) < at + 2 {
                            assert!(Instant::now() < deadline, "the writer stalled");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                    ControlFlow::Continue(())
                })
                .expect("backup under a writer");
            stop.store(true, Ordering::SeqCst);
            writer.join().expect("writer");
            b
        });
        assert_eq!(backup.path, dir.join("concurrent.redb"));
        assert!(!dir.join(PARTIAL_DIR).join("concurrent.redb").exists());
        let v = verify_backup(&backup.path, &dir.join("verify"), Some(NET)).expect("verifies");
        assert_eq!(v.record.as_ref(), Some(&backup.record));
        assert!(v.consistency_checked);
        let got = content_digest(&backup.path);
        let states = states.into_inner().expect("states");
        let at = states.iter().position(|d| *d == got);
        assert!(
            at.is_some(),
            "case {case} (seed {seed:#x}): the backup {} is no committed state",
            short(&got)
        );
        assert_ne!(
            at,
            Some(states.len() - 1),
            "case {case}: commits made under the snapshot reached the backup"
        );
        store.close().expect("close");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Runs a backup into a fresh power-loss backend that fails after `n`
/// operations (or once, at the `n`th, with `transient`); returns the
/// backend, the result, and whether the commit returned.
fn cut_backup(
    store: &Store,
    n: u64,
    transient: bool,
) -> (Arc<PowerLoss>, Result<BackupRecord, Error>, bool) {
    let dest = PowerLoss::new(Vec::new(), true);
    if transient {
        dest.fail_once_after(n);
    } else {
        dest.power_fails_after(n);
    }
    let mut committed = false;
    let r = store.backup_to_backend_observed(Shared(dest.clone()), &info(), &mut |s| {
        if s == BackupStep::Committed {
            committed = true;
        }
        ControlFlow::Continue(())
    });
    (dest, r, committed)
}

fn verify_image(image: Vec<u8>) -> Result<purse_store::Verified, Error> {
    verify_backup_on_backend(Shared(PowerLoss::new(image, true)), Some(NET))
}

#[test]
fn a_power_cut_at_every_storage_operation_of_a_backup_publishes_nothing_partial() {
    let mut rng = SplitMix64::from_env("backup-power-cut");
    let (store, src) = source(&mut rng, 40);
    let before = store_digest(&store);
    let reference = PowerLoss::new(Vec::new(), true);
    let record = store
        .backup_to_backend(Shared(reference.clone()), &info())
        .expect("reference backup");
    let total = reference.ops();
    assert!(
        total > 20,
        "a backup of the fixture makes {total} operations"
    );
    let v = verify_image(reference.image()).expect("the reference verifies");
    assert_eq!(v.record.as_ref(), Some(&record));
    let step = sweep_step();
    let src_ops = src.ops();
    for cut in [Cut::Clean, Cut::Torn(rng.next_u64()), Cut::Kept] {
        let (mut refused, mut verified, mut after_commit) = (0, 0, 0);
        let mut n = 0;
        while n <= total {
            let (dest, r, committed) = cut_backup(&store, n, false);
            if n < total {
                assert!(dest.power_failed() || r.is_ok(), "{cut:?} at {n}");
            } else {
                r.as_ref().expect("no cut: the backup completes");
            }
            match verify_image(dest.cut(cut)) {
                Ok(v) => {
                    assert_eq!(
                        v.record.as_ref(),
                        Some(&record),
                        "{cut:?} at {n}: an image verified as another backup"
                    );
                    verified += 1;
                }
                Err(e) => {
                    assert!(
                        !committed,
                        "{cut:?} at {n}: the commit had returned, yet the image is refused: {e}"
                    );
                    assert!(
                        r.is_err(),
                        "{cut:?} at {n}: a backup that returned Ok is refused"
                    );
                    refused += 1;
                }
            }
            if committed {
                after_commit += 1;
            }
            assert_eq!(src.ops(), src_ops, "the backup wrote to the source");
            n += step;
        }
        eprintln!(
            "{cut:?}: {total} operations, {refused} images refused, {verified} verified \
             ({after_commit} cut after the commit returned)"
        );
        assert!(
            refused > 0 && verified > 0,
            "{cut:?}: the sweep missed a side"
        );
        if step == 1 {
            assert!(after_commit > 0, "{cut:?}: no cut fell in the compaction");
        }
    }
    assert_eq!(store_digest(&store), before, "the source changed");
    // The source still takes a durable commit, and its image reopens.
    store
        .update(|tx| tx.put::<Tickets>(&Hash32([0xee; 32]), &purse_store::Empty))
        .expect("write after the sweep");
    let after = store_digest(&store);
    store.close().expect("close");
    let reopened = OpenOptions::new()
        .open_on_backend(Shared(PowerLoss::new(src.cut(Cut::Clean), true)))
        .expect("reopen source");
    assert_eq!(store_digest(&reopened), after);
}

#[test]
fn a_transient_storage_error_at_every_operation_leaves_the_source_usable() {
    let mut rng = SplitMix64::from_env("backup-transient");
    let (store, src) = source(&mut rng, 20);
    let before = store_digest(&store);
    let reference = PowerLoss::new(Vec::new(), true);
    let record = store
        .backup_to_backend(Shared(reference.clone()), &info())
        .expect("reference backup");
    let total = reference.ops();
    let src_ops = src.ops();
    let step = sweep_step();
    let mut failed = 0;
    let mut n = 0;
    while n < total {
        let (dest, r, _) = cut_backup(&store, n, true);
        match r {
            Ok(rec) => {
                assert_eq!(rec, record);
                let v = verify_image(dest.image()).expect("a backup that returned Ok verifies");
                assert_eq!(v.record, Some(record.clone()));
            }
            Err(e) => {
                assert!(!e.backup_stopped());
                failed += 1;
            }
        }
        assert_eq!(src.ops(), src_ops);
        n += step;
    }
    assert!(failed > 0);
    assert_eq!(store_digest(&store), before);
    // The lock was released by every failed backup.
    let again = PowerLoss::new(Vec::new(), true);
    assert_eq!(
        store
            .backup_to_backend(Shared(again), &info())
            .expect("backup"),
        record
    );
}

fn step_labels(store: &Store) -> Vec<String> {
    let dir = scratch("steps");
    let mut labels = Vec::new();
    store
        .backup_observed(&dir, "w", &info(), &mut |s| {
            labels.push(format!("{s:?}"));
            ControlFlow::Continue(())
        })
        .expect("reference backup");
    let _ = std::fs::remove_dir_all(&dir);
    labels
}

#[test]
fn a_stop_at_every_step_never_publishes_and_the_next_backup_cleans_up() {
    let mut rng = SplitMix64::from_env("backup-stop");
    let (store, _) = source(&mut rng, 30);
    let before = store_digest(&store);
    let labels = step_labels(&store);
    assert_eq!(labels.first().map(String::as_str), Some("Prepared"));
    assert_eq!(labels.last().map(String::as_str), Some("Closed"));
    let commit_at = labels
        .iter()
        .position(|l| l == "Committed")
        .expect("a commit step");
    assert_eq!(
        labels
            .iter()
            .filter(|l| l.starts_with("TableCopied"))
            .count(),
        TableId::ALL.len()
    );
    let mut reference_record = None;
    for stop_at in 0..labels.len() {
        let dir = scratch("stop");
        let partial = dir.join(PARTIAL_DIR);
        std::fs::create_dir_all(&partial).unwrap();
        // A user's file in the backups directory, and leftovers of an
        // interrupted backup in `.partial/`, a directory among them.
        std::fs::write(dir.join("w.tmp"), b"mine").unwrap();
        std::fs::write(partial.join("left-over"), b"x").unwrap();
        std::fs::create_dir_all(partial.join("sub").join("deeper")).unwrap();
        std::fs::write(partial.join("sub").join("deeper").join("f"), b"y").unwrap();
        let mut seen = 0;
        let e = store
            .backup_observed(&dir, "w", &info(), &mut |_| {
                seen += 1;
                if seen > stop_at {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .expect_err("stopped");
        assert!(e.backup_stopped(), "{e}");
        assert_eq!(e.kind(), Kind::Other);
        assert!(!dir.join("w").exists(), "stopped at {stop_at}: published");
        let left = partial.join("w");
        assert!(left.exists(), "stopped at {stop_at}: its file is gone");
        assert!(!partial.join("left-over").exists());
        assert!(!partial.join("sub").exists());
        match verify_backup(&left, &dir.join("verify"), Some(NET)) {
            Ok(v) => {
                assert!(
                    stop_at >= commit_at,
                    "stopped at {stop_at}: verifies before the commit"
                );
                let r = v.record.expect("record");
                assert_eq!(*reference_record.get_or_insert_with(|| r.clone()), r);
            }
            Err(e) => {
                assert!(
                    stop_at < commit_at,
                    "stopped at {stop_at}: refused after the commit: {e}"
                );
                assert_eq!(e.kind(), Kind::NotExist, "{e}");
            }
        }
        assert_eq!(store_digest(&store), before);
        // The next backup removes what is in `.partial/` and publishes.
        let b = store.backup(&dir, "w", &info()).expect("next backup");
        assert_eq!(b.path, dir.join("w"));
        assert_eq!(std::fs::read_dir(&partial).unwrap().count(), 0);
        assert_eq!(std::fs::read(dir.join("w.tmp")).unwrap(), b"mine");
        verify_backup(&b.path, &dir.join("verify"), Some(NET)).expect("verifies");
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(reference_record.is_some());
}

#[test]
fn refused_names_write_nothing() {
    let mut rng = SplitMix64::from_env("backup-names");
    let (store, _) = source(&mut rng, 0);
    let long = "x".repeat(256);
    let cases: &[(&str, &str)] = &[
        ("", BACKUP_NAME_EMPTY_TEXT),
        ("a/b", BACKUP_NAME_SEPARATOR_TEXT),
        ("/abs", BACKUP_NAME_SEPARATOR_TEXT),
        ("a\\b", BACKUP_NAME_SEPARATOR_TEXT),
        ("..", BACKUP_NAME_DOTDOT_TEXT),
        ("a..b", BACKUP_NAME_DOTDOT_TEXT),
        (".hidden", BACKUP_NAME_DOT_TEXT),
        (".partial", BACKUP_NAME_DOT_TEXT),
        (".", BACKUP_NAME_DOT_TEXT),
        ("C:w", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("w:stream", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("w?", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("a\0b", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("a\nb", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("w.", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("w ", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("CON", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("nul.txt", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("Lpt9", BACKUP_NAME_NOT_PORTABLE_TEXT),
        // Superscript digits are digits to Windows too, and the console
        // names open console handles (Go's isReservedBaseName); COM0 and
        // LPT0 are on Microsoft's list.
        ("COM¹", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("com²", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("COM³.redb", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("lpt¹", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("LPT²", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("Lpt³.bak", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("COM0", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("lpt0.x", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("CONIN$", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("conout$", BACKUP_NAME_NOT_PORTABLE_TEXT),
        ("CONOUT$.txt", BACKUP_NAME_NOT_PORTABLE_TEXT),
        (&long, BACKUP_NAME_NOT_PORTABLE_TEXT),
    ];
    let root = scratch("names");
    let dir = root.join("backups");
    for &(name, text) in cases {
        let e = store.backup(&dir, name, &info()).expect_err(name);
        assert_eq!(e.kind(), Kind::Invalid, "{name:?}");
        assert_eq!(
            e.to_string(),
            format!("invalid operation: {text}"),
            "{name:?}"
        );
        assert!(!dir.exists(), "{name:?}: the backups directory was created");
        assert_eq!(
            check_backup_name(name).unwrap_err().to_string(),
            e.to_string()
        );
    }
    for name in [
        "w",
        "w.tmp",
        "wallet-2026-11-20.redb",
        "ü.bak",
        "a b",
        "console",
        "com10",
        "com⁴",
        "conin",
        "CONOUT$x",
        "lpt$",
    ] {
        check_backup_name(name).unwrap_or_else(|e| panic!("{name:?}: {e}"));
    }
    check_backup_name(&"x".repeat(255)).expect("255 bytes");
    // Texts the backup record cannot hold are refused before anything too.
    for bad in [
        BackupInfo {
            network: "",
            ..info()
        },
        BackupInfo {
            version: &long,
            ..info()
        },
    ] {
        let e = store.backup(&dir, "w", &bad).unwrap_err();
        assert_eq!(e.kind(), Kind::Invalid);
        assert!(!dir.exists());
    }
}

#[test]
fn an_existing_name_is_refused_and_nothing_in_backups_is_touched() {
    let mut rng = SplitMix64::from_env("backup-existing");
    let (store, _) = source(&mut rng, 5);
    let dir = scratch("existing");
    let partial = dir.join(PARTIAL_DIR);
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(dir.join("w"), b"a user's file").unwrap();
    std::fs::write(partial.join("left-over"), b"x").unwrap();
    let e = store.backup(&dir, "w", &info()).expect_err("exists");
    assert_eq!(e.kind(), Kind::Exist);
    assert_eq!(std::fs::read(dir.join("w")).unwrap(), b"a user's file");
    // Refused before the clean-up.
    assert!(partial.join("left-over").exists());
    // `w.tmp`, then `w2`: both stay, byte for byte.
    let first = store.backup(&dir, "w.tmp", &info()).expect("w.tmp");
    let first_bytes = std::fs::read(&first.path).unwrap();
    assert!(!partial.join("left-over").exists());
    store.backup(&dir, "w2", &info()).expect("w2");
    assert_eq!(std::fs::read(&first.path).unwrap(), first_bytes);
    assert_eq!(std::fs::read(dir.join("w")).unwrap(), b"a user's file");
    let e = store.backup(&dir, "w.tmp", &info()).expect_err("exists");
    assert_eq!(e.kind(), Kind::Exist);
    assert_eq!(std::fs::read(&first.path).unwrap(), first_bytes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_that_is_not_a_directory_is_refused() {
    let mut rng = SplitMix64::from_env("backup-partial-file");
    let (store, _) = source(&mut rng, 0);
    let dir = scratch("partial-file");
    std::fs::write(dir.join(PARTIAL_DIR), b"not a directory").unwrap();
    let e = store.backup(&dir, "w", &info()).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!("invalid operation: {PARTIAL_NOT_DIR_TEXT}")
    );
    assert_eq!(
        std::fs::read(dir.join(PARTIAL_DIR)).unwrap(),
        b"not a directory"
    );
    assert!(!dir.join("w").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.partial` that links elsewhere is refused, so the clean-up never
/// removes a file outside the backups directory (symbolic links need no
/// privilege on Unix only).
#[cfg(unix)]
#[test]
fn a_partial_that_links_elsewhere_is_refused() {
    let mut rng = SplitMix64::from_env("backup-partial-link");
    let (store, _) = source(&mut rng, 0);
    let root = scratch("partial-link");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("precious"), b"keep").unwrap();
    let dir = root.join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join(PARTIAL_DIR)).unwrap();
    let e = store.backup(&dir, "w", &info()).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!("invalid operation: {PARTIAL_NOT_DIR_TEXT}")
    );
    assert_eq!(std::fs::read(elsewhere.join("precious")).unwrap(), b"keep");
    // A link inside `.partial` is removed, not followed.
    std::fs::remove_file(dir.join(PARTIAL_DIR)).unwrap();
    std::fs::create_dir(dir.join(PARTIAL_DIR)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join(PARTIAL_DIR).join("link")).unwrap();
    store.backup(&dir, "w", &info()).expect("backup");
    assert_eq!(std::fs::read(elsewhere.join("precious")).unwrap(), b"keep");
    assert!(!dir.join(PARTIAL_DIR).join("link").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn one_backup_runs_at_a_time() {
    let mut rng = SplitMix64::from_env("backup-one-at-a-time");
    let (store, _) = source(&mut rng, 5);
    let dir = scratch("one-at-a-time");
    let mut tried = false;
    store
        .backup_observed(&dir, "a", &info(), &mut |_| {
            if !tried {
                tried = true;
                let e = store.backup(&dir, "b", &info()).unwrap_err();
                assert_eq!(
                    e.to_string(),
                    format!("invalid operation: {BACKUP_RUNNING_TEXT}")
                );
                let e = store
                    .backup_to_backend(Shared(PowerLoss::new(Vec::new(), true)), &info())
                    .unwrap_err();
                assert_eq!(
                    e.to_string(),
                    format!("invalid operation: {BACKUP_RUNNING_TEXT}")
                );
                // Writers are not blocked by a running backup.
                store
                    .update(|tx| tx.put::<Tickets>(&Hash32([9; 32]), &purse_store::Empty))
                    .expect("a write during the backup");
            }
            ControlFlow::Continue(())
        })
        .expect("a");
    assert!(tried);
    store.backup(&dir, "b", &info()).expect("b after a");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_published_backup_is_compact_and_opens_as_a_store() {
    let mut rng = SplitMix64::from_env("backup-compact");
    let (store, _) = source(&mut rng, 200);
    let dir = scratch("compact");
    let b = store.backup(&dir, "w", &info()).expect("backup");
    // Nothing is left for a further compaction, in either mode.
    let copy = dir.join("copy");
    std::fs::copy(&b.path, &copy).unwrap();
    {
        let mut db = redb::Database::open(&copy).expect("open copy");
        assert!(!db.compact(redb::CompactionMode::Balanced).expect("compact"));
    }
    let v = verify_backup(&b.path, &dir.join("verify"), Some(NET)).expect("verifies");
    assert_eq!(v.record, Some(b.record.clone()));
    assert_eq!(v.rows, b.record.rows());
    assert_eq!(b.record.schema_version(), SCHEMA_VERSION);
    assert_eq!(b.record.network(), NET);
    assert_eq!(b.record.version(), info().version);
    assert_eq!(b.record.time(), info().time);
    // Its rows are the store's.
    assert_eq!(content_digest(&b.path), store_digest(&store));
    // A backup of a backup's store copies its rows, not its record.
    let restored = OpenOptions::new()
        .open(&copy)
        .expect("a backup opens as a store");
    let again = restored
        .backup(&dir, "again", &info())
        .expect("backup of it");
    assert_eq!(again.record, b.record);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bits of `path`'s mode outside the owner's (Unix).
#[cfg(unix)]
fn others_bits(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o077
}

/// A backup, the directories it creates and a new store are private to
/// their owner on Unix, as dcrwallet makes `wallet.db` (mode 0600) and its
/// directories (0700): a backup holds everything the store holds. The
/// process umask can only narrow the modes, so the test asks only that no
/// bit outside the owner's is set, whatever the umask, and that the owner
/// can read and write.
#[cfg(unix)]
#[test]
fn backups_and_stores_are_created_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = scratch("modes");
    let live = root.join(STORE_FILE_NAME);
    let store = Store::create(&live).expect("create");
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(others_bits(&live), 0, "store mode {:o}", mode(&live));
    assert_eq!(mode(&live) & 0o600, 0o600);
    // A backups directory two levels below an existing one: both new
    // directories and `.partial` are 0700, the file 0600.
    let dir = root.join("net").join("backups");
    let b = store.backup(&dir, "w", &info()).expect("backup");
    for d in [root.join("net"), dir.clone(), dir.join(PARTIAL_DIR)] {
        assert_eq!(others_bits(&d), 0, "{} mode {:o}", d.display(), mode(&d));
        assert_eq!(mode(&d) & 0o700, 0o700);
    }
    assert_eq!(others_bits(&b.path), 0, "backup mode {:o}", mode(&b.path));
    // An existing backups directory keeps the mode its owner gave it.
    let open = root.join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
    let b2 = store.backup(&open, "w", &info()).expect("backup");
    assert_eq!(mode(&open), 0o755);
    assert_eq!(others_bits(&b2.path), 0);
    assert_eq!(others_bits(&open.join(PARTIAL_DIR)), 0);
    // The scratch directory verify creates.
    let scratch_dir = root.join("verify-scratch");
    verify_backup(&b.path, &scratch_dir, Some(NET)).expect("verifies");
    assert_eq!(others_bits(&scratch_dir), 0);
    store.close().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// `verify_backup` copies only a regular file: a directory (on every
/// platform), and on Unix a device, which could be read without end, a
/// socket and a FIFO, are refused before anything is copied, and the
/// scratch directory stays empty.
#[test]
fn verify_refuses_what_is_not_a_regular_file() {
    let root = scratch("verify-not-file");
    let scratch_dir = root.join("scratch");
    let not_a_file = format!("invalid operation: {VERIFY_NOT_A_FILE_TEXT}");
    let e = verify_backup(&root, &scratch_dir, None).unwrap_err();
    assert_eq!(e.kind(), Kind::Invalid, "{e}");
    assert_eq!(e.to_string(), not_a_file, "a directory");
    #[cfg(unix)]
    {
        for dev in ["/dev/zero", "/dev/null"] {
            let e = verify_backup(Path::new(dev), &scratch_dir, None).unwrap_err();
            assert_eq!(e.to_string(), not_a_file, "{dev}");
        }
        // In the system's temporary directory: a socket's path has a
        // length limit (about 100 bytes) the target directory can exceed.
        let sock = std::env::temp_dir().join(format!("purse-verify-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
        let e = verify_backup(&sock, &scratch_dir, None).unwrap_err();
        drop(listener);
        let _ = std::fs::remove_file(&sock);
        assert_eq!(e.to_string(), not_a_file, "a socket");
        // A FIFO with no writer: a plain read-only open of it waits for a
        // writer forever. The call runs on a thread with a time limit, and
        // a writer is opened only to release it if it hangs.
        let fifo = root.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(made.success(), "mkfifo: {made}");
        let (tx, rx) = std::sync::mpsc::channel();
        let (f, s) = (fifo.clone(), scratch_dir.clone());
        let t = std::thread::spawn(move || {
            let _ = tx.send(verify_backup(&f, &s, None).map(drop));
        });
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(r) => assert_eq!(r.unwrap_err().to_string(), not_a_file, "a FIFO"),
            Err(_) => {
                let _writer = std::fs::OpenOptions::new().write(true).open(&fifo);
                let _ = t.join();
                panic!("verify_backup blocked on a FIFO with no writer");
            }
        }
        t.join().unwrap();
    }
    assert_eq!(
        std::fs::read_dir(&scratch_dir).map_or(0, Iterator::count),
        0
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The file handed to `verify_backup` is only ever read, even when it lies
/// in the scratch directory under the prefix reserved to the copies (where
/// the sweep of left-over copies once deleted it, whether it then verified
/// or not), or is reached there through a link; the other files under the
/// prefix are still swept.
#[test]
fn verify_never_removes_its_input_from_scratch() {
    let mut rng = SplitMix64::from_env("verify-input-in-scratch");
    let (store, _) = source(&mut rng, 5);
    let root = scratch("verify-input-in-scratch");
    let b = store
        .backup(&root.join("backups"), "w", &info())
        .expect("backup");
    let scratch_dir = root.join("scratch");
    std::fs::create_dir_all(&scratch_dir).unwrap();
    let input = scratch_dir.join(format!("{VERIFY_COPY_PREFIX}mine.redb"));
    std::fs::copy(&b.path, &input).unwrap();
    let bytes = std::fs::read(&input).unwrap();
    let stale = scratch_dir.join(format!("{VERIFY_COPY_PREFIX}stale"));
    std::fs::write(&stale, b"left").unwrap();
    let v = verify_backup(&input, &scratch_dir, Some(NET)).expect("verifies");
    assert_eq!(v.record, Some(b.record.clone()));
    assert_eq!(
        std::fs::read(&input).unwrap(),
        bytes,
        "a backup that verifies"
    );
    assert!(!stale.exists(), "a left-over copy is still swept");
    // Through a relative spelling of the same path.
    let relative = scratch_dir
        .join("..")
        .join("scratch")
        .join(format!("{VERIFY_COPY_PREFIX}mine.redb"));
    verify_backup(&relative, &scratch_dir, Some(NET)).expect("verifies");
    assert_eq!(std::fs::read(&input).unwrap(), bytes, "a relative spelling");
    // A file that does not verify stays too.
    let junk = scratch_dir.join(format!("{VERIFY_COPY_PREFIX}junk"));
    std::fs::write(&junk, b"user data").unwrap();
    verify_backup(&junk, &scratch_dir, None).expect_err("junk");
    assert_eq!(
        std::fs::read(&junk).unwrap(),
        b"user data",
        "a file that fails"
    );
    // Through a link in scratch to a file outside it (Unix: links need no
    // privilege there): neither the link nor the file is removed.
    #[cfg(unix)]
    {
        let link = scratch_dir.join(format!("{VERIFY_COPY_PREFIX}link"));
        std::os::unix::fs::symlink(&b.path, &link).unwrap();
        verify_backup(&link, &scratch_dir, Some(NET)).expect("verifies");
        assert!(link.exists(), "the link handed in");
        assert!(b.path.exists());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The child half of `a_backup_into_the_empty_directory_path_succeeds`:
/// with `PURSE_STORE_BACKUP_CWD` set, it backs a store up into the
/// directory `""` of its current directory, which the parent set. Without
/// the variable it does nothing. A child process, because the current
/// directory is the whole process's and the other tests run beside it.
#[test]
fn backup_into_cwd_child() {
    if std::env::var_os("PURSE_STORE_BACKUP_CWD").is_none() {
        return;
    }
    let mut rng = SplitMix64::from_env("backup-cwd");
    let (store, _) = source(&mut rng, 5);
    let b = store
        .backup(Path::new(""), "w", &info())
        .expect("a backup into the directory \"\"");
    assert_eq!(b.path, Path::new("w"));
    assert!(Path::new("w").is_file());
    assert_eq!(
        std::fs::read_dir(PARTIAL_DIR).map_or(1, Iterator::count),
        0,
        "nothing left in .partial"
    );
}

/// A backup into the directory `""` (the current one, as a relative path
/// spells it) publishes its file and returns it: the sync of the directory
/// after the rename once opened `""`, which fails, and reported an error
/// for a file it had published, so a retry under the same name was refused.
#[test]
fn a_backup_into_the_empty_directory_path_succeeds() {
    let cwd = scratch("backup-cwd");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "backup_into_cwd_child", "--test-threads=1"])
        .env("PURSE_STORE_BACKUP_CWD", "1")
        .current_dir(&cwd)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "child: {}\n{stdout}", out.status);
    // The filter matched the child, which ran: not an empty run.
    assert!(stdout.contains("1 passed"), "child: {stdout}");
    let v = verify_backup(&cwd.join("w"), &cwd.join("verify"), Some(NET)).expect("verifies");
    assert!(v.record.is_some());
    let _ = std::fs::remove_dir_all(&cwd);
}

/// Every verification first removes the copies a killed one left in its
/// scratch directory (files named with the reserved prefix), and nothing
/// else there.
#[test]
fn verify_removes_copies_a_killed_verify_left() {
    let mut rng = SplitMix64::from_env("verify-sweep");
    let (store, _) = source(&mut rng, 5);
    let root = scratch("verify-sweep");
    let b = store
        .backup(&root.join("backups"), "w", &info())
        .expect("backup");
    let scratch_dir = root.join("scratch");
    std::fs::create_dir_all(scratch_dir.join(format!("{VERIFY_COPY_PREFIX}dir"))).unwrap();
    for n in ["1-2-3.redb", "stale"] {
        std::fs::write(
            scratch_dir.join(format!("{VERIFY_COPY_PREFIX}{n}")),
            b"left",
        )
        .unwrap();
    }
    std::fs::write(scratch_dir.join("users-file"), b"keep").unwrap();
    std::fs::write(scratch_dir.join("verify-1.redb"), b"keep").unwrap();
    verify_backup(&b.path, &scratch_dir, Some(NET)).expect("verifies");
    let mut left: Vec<String> = std::fs::read_dir(&scratch_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            format!("{VERIFY_COPY_PREFIX}dir"),
            "users-file".to_owned(),
            "verify-1.redb".to_owned()
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The child half of `a_killed_verify_leaves_no_copy_behind`: with
/// `PURSE_STORE_VERIFY_LOOP` set to `<file>\n<scratch>`, it verifies the
/// file again and again until it is killed. Without the variable it does
/// nothing.
#[test]
fn verify_loop_child() {
    let Some(v) = std::env::var_os("PURSE_STORE_VERIFY_LOOP") else {
        return;
    };
    let v = v.into_string().unwrap();
    let (file, scratch_dir) = v.split_once('\n').unwrap();
    loop {
        verify_backup(Path::new(file), Path::new(scratch_dir), Some(NET)).expect("verifies");
    }
}

/// A process killed while it verifies leaves no copy of the file it was
/// given: on Unix the copy has no name once it is created (an empty file
/// may be caught in the instant before its name is removed, never one
/// holding bytes), and on every platform the next verification into the
/// same scratch directory removes whatever a killed one left. The kill
/// lands wherever the child happens to be, several times.
#[test]
fn a_killed_verify_leaves_no_copy_behind() {
    let mut rng = SplitMix64::from_env("verify-kill");
    let (store, _) = source(&mut rng, 400);
    let root = scratch("verify-kill");
    let b = store
        .backup(&root.join("backups"), "w", &info())
        .expect("backup");
    let scratch_dir = root.join("scratch");
    let copies = || -> Vec<(String, u64)> {
        std::fs::read_dir(&scratch_dir)
            .map(|d| {
                d.map(|e| {
                    let e = e.unwrap();
                    (
                        e.file_name().into_string().unwrap(),
                        e.metadata().map_or(0, |m| m.len()),
                    )
                })
                .filter(|(n, _)| n.starts_with(VERIFY_COPY_PREFIX))
                .collect()
            })
            .unwrap_or_default()
    };
    for round in 0..4u64 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "verify_loop_child", "--test-threads=1"])
            .env(
                "PURSE_STORE_VERIFY_LOOP",
                format!("{}\n{}", b.path.display(), scratch_dir.display()),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(150 + 70 * round));
        child.kill().unwrap();
        child.wait().unwrap();
        if cfg!(unix) {
            for (name, len) in copies() {
                assert_eq!(
                    len, 0,
                    "round {round}: a killed verify left {name} with {len} bytes"
                );
            }
        }
        verify_backup(&b.path, &scratch_dir, Some(NET)).expect("verifies");
        assert_eq!(copies(), [], "round {round}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// What `verify_backup` makes of `bytes`, written to a file: the file must
/// be unchanged afterwards and the scratch directory empty.
fn verify_bytes(
    dir: &Path,
    bytes: &[u8],
    network: Option<&str>,
) -> Result<purse_store::Verified, Error> {
    let file = dir.join("candidate");
    std::fs::write(&file, bytes).unwrap();
    let scratch = dir.join("verify");
    let r = verify_backup(&file, &scratch, network);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "verify wrote its input"
    );
    assert_eq!(
        std::fs::read_dir(&scratch).map_or(0, Iterator::count),
        0,
        "verify left a file in its scratch directory"
    );
    r
}

/// Edits a copy of a backup file with redb directly and returns its bytes.
fn edited(dir: &Path, base: &[u8], edit: impl FnOnce(&redb::WriteTransaction)) -> Vec<u8> {
    let p = dir.join("edit");
    std::fs::write(&p, base).unwrap();
    {
        let db = redb::Database::create(&p).expect("open for edit");
        let w = db.begin_write().unwrap();
        edit(&w);
        w.commit().unwrap();
    }
    let b = std::fs::read(&p).unwrap();
    std::fs::remove_file(&p).unwrap();
    b
}

const RAW: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("meta");

fn put_meta(w: &redb::WriteTransaction, key: &[u8], value: &[u8]) {
    w.open_table(RAW).unwrap().insert(key, value).unwrap();
}

fn record_with(r: &BackupRecord, network: &str, schema: u32, rows: u64) -> Vec<u8> {
    BackupRecord::new(network, schema, r.version(), r.time(), rows, r.digest())
        .unwrap()
        .encode()
}

#[test]
fn verify_refuses_hostile_files_without_panicking() {
    let mut rng = SplitMix64::from_env("backup-hostile");
    let (store, _) = source(&mut rng, 30);
    let dir = scratch("hostile");
    let b = store.backup(&dir, "good", &info()).expect("backup");
    let good = std::fs::read(&b.path).unwrap();
    let rec = b.record.clone();
    let v = verify_bytes(&dir, &good, Some(NET)).expect("the original verifies");
    assert_eq!(v.record.as_ref(), Some(&rec));

    // Missing.
    let e = verify_backup(&dir.join("absent"), &dir.join("verify"), None).unwrap_err();
    assert_eq!(e.kind(), Kind::NotExist);

    // Not a store at all.
    for bytes in [
        Vec::new(),
        vec![0u8; 1],
        vec![0u8; 8192],
        b"this is not a wallet store\n".repeat(100),
        rng.bytes(4096),
        rng.bytes(70_000),
    ] {
        verify_bytes(&dir, &bytes, None).expect_err("not a store");
    }

    // Truncated.
    let len = good.len();
    for cut in [1, 512, 4096, len / 3, len / 2, len - 4096, len - 1] {
        verify_bytes(&dir, &good[..cut], None).expect_err("truncated");
    }

    // One bit flipped: refused, or, where nothing reads the byte, the
    // original backup exactly.
    let mut positions: Vec<usize> = (0..len).step_by(len / 300 + 1).collect();
    positions.extend((0..100).map(|_| rng.below(len as u64) as usize));
    positions.extend(0..64); // the header
    let (mut refused, mut intact) = (0, 0);
    for p in positions {
        let mut bad = good.clone();
        bad[p] ^= 1 << rng.below(8);
        match verify_bytes(&dir, &bad, Some(NET)) {
            Ok(v) => {
                assert_eq!(v.record.as_ref(), Some(&rec), "flip at {p}");
                intact += 1;
            }
            Err(_) => refused += 1,
        }
    }
    eprintln!("bit flips: {refused} refused, {intact} verified intact");
    assert!(refused > 0);

    // A redb file of someone else's: no meta table, or a meta table of
    // other types.
    let other = edited(&dir, &[], |w| {
        w.open_table(TableDefinition::<u64, u64>::new("numbers"))
            .unwrap()
            .insert(1, 2)
            .unwrap();
    });
    let e = verify_bytes(&dir, &other, None).unwrap_err();
    assert!(
        e.to_string().contains("not a dcr-purse wallet store"),
        "{e}"
    );
    let typed = edited(&dir, &[], |w| {
        w.open_table(TableDefinition::<u64, u64>::new("meta"))
            .unwrap()
            .insert(1, 2)
            .unwrap();
    });
    let e = verify_bytes(&dir, &typed, None).unwrap_err();
    assert_eq!(e.kind(), Kind::Invalid, "{e}");
    assert!(
        e.to_string().contains("not a dcr-purse wallet store"),
        "{e}"
    );

    // The store's own refusals.
    let newer = edited(&dir, &good, |w| {
        put_meta(w, b"version", &(SCHEMA_VERSION + 1).to_be_bytes());
    });
    let e = verify_bytes(&dir, &newer, None).unwrap_err();
    assert_eq!(
        e.newer_version_info(),
        Some((SCHEMA_VERSION + 1, SCHEMA_VERSION))
    );
    let zero = edited(&dir, &good, |w| {
        put_meta(w, b"version", &0u32.to_be_bytes())
    });
    assert_eq!(
        verify_bytes(&dir, &zero, None).unwrap_err().kind(),
        Kind::Invalid
    );
    let no_version = edited(&dir, &good, |w| {
        w.open_table(RAW)
            .unwrap()
            .remove(b"version".as_slice())
            .unwrap();
    });
    assert_eq!(
        verify_bytes(&dir, &no_version, None).unwrap_err().kind(),
        Kind::NotExist
    );
    let format = edited(&dir, &good, |w| put_meta(w, b"format", b"someone else"));
    let e = verify_bytes(&dir, &format, None).unwrap_err();
    assert!(
        e.to_string().contains("not a dcr-purse wallet store"),
        "{e}"
    );

    // The record against the rows.
    let row_changed = edited(&dir, &good, |w| {
        w.open_table(TableDefinition::<&[u8], &[u8]>::new("tickets"))
            .unwrap()
            .insert([0x42u8; 32].as_slice(), b"".as_slice())
            .unwrap();
    });
    let e = verify_bytes(&dir, &row_changed, None).unwrap_err();
    assert!(e.unverified().is_some(), "{e}");
    assert_eq!(e.kind(), Kind::Io);
    let more_rows = edited(&dir, &good, |w| {
        put_meta(
            w,
            BACKUP_RECORD_KEY,
            &record_with(&rec, NET, rec.schema_version(), rec.rows() + 1),
        );
    });
    let e = verify_bytes(&dir, &more_rows, None).unwrap_err();
    assert!(e.unverified().unwrap().contains("rows"), "{e}");
    let wrong_schema = edited(&dir, &good, |w| {
        put_meta(
            w,
            BACKUP_RECORD_KEY,
            &record_with(&rec, NET, rec.schema_version() + 1, rec.rows()),
        );
    });
    let e = verify_bytes(&dir, &wrong_schema, None).unwrap_err();
    assert!(e.unverified().unwrap().contains("schema version"), "{e}");
    let mainnet = edited(&dir, &good, |w| {
        put_meta(
            w,
            BACKUP_RECORD_KEY,
            &record_with(&rec, "mainnet", rec.schema_version(), rec.rows()),
        );
    });
    let e = verify_bytes(&dir, &mainnet, Some(NET)).unwrap_err();
    assert_eq!(e.wrong_network(), Some("mainnet"));
    assert_eq!(e.kind(), Kind::Invalid);
    assert_eq!(
        e.to_string(),
        "invalid operation: backup is of network mainnet, not simnet"
    );
    verify_bytes(&dir, &mainnet, None).expect("no network asked for");
    for garbage in [&b""[..], &[2u8; 60][..], &[1u8][..]] {
        let g = edited(&dir, &good, |w| put_meta(w, BACKUP_RECORD_KEY, garbage));
        let e = verify_bytes(&dir, &g, None).unwrap_err();
        assert_eq!(e.corrupt_row().map(|(t, _)| t), Some(TableId::Meta), "{e}");
    }

    // A newer store, backed up honestly by a build of its version.
    let newer_store = OpenOptions::new()
        .schema(Schema {
            version: SCHEMA_VERSION + 1,
            migrations: &[],
        })
        .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
        .unwrap();
    let nb = newer_store.backup(&dir, "newer", &info()).expect("backup");
    assert_eq!(nb.record.schema_version(), SCHEMA_VERSION + 1);
    let e = verify_backup(&nb.path, &dir.join("verify"), None).unwrap_err();
    assert_eq!(
        e.newer_version_info(),
        Some((SCHEMA_VERSION + 1, SCHEMA_VERSION))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_checks_a_plain_store_copy_by_its_consistency() {
    let dir = scratch("plain");
    // A plain copy of a stopped store: no record, every check but the
    // record's.
    let v = verify_bytes(&dir, FIXTURE, Some(NET)).expect("the fixture verifies");
    assert_eq!(v.record, None);
    assert!(v.consistency_checked);
    assert_eq!(v.schema_version, SCHEMA_VERSION);
    // A row that does not decode by its table's type.
    let undecodable = edited(&dir, FIXTURE, |w| {
        w.open_table(TableDefinition::<&[u8], &[u8]>::new("accounts"))
            .unwrap()
            .insert([1u8, 0, 0, 0].as_slice(), [0xffu8; 3].as_slice())
            .unwrap();
    });
    let e = verify_bytes(&dir, &undecodable, None).unwrap_err();
    assert_eq!(
        e.corrupt_row().map(|(t, _)| t),
        Some(TableId::Accounts),
        "{e}"
    );
    // An unknown meta row.
    let unknown = edited(&dir, FIXTURE, |w| put_meta(w, b"zzz", b"1"));
    let e = verify_bytes(&dir, &unknown, None).unwrap_err();
    assert_eq!(e.corrupt_row().map(|(t, _)| t), Some(TableId::Meta), "{e}");
    // Rows that contradict the chain's invariants: a header row removed.
    let broken = edited(&dir, FIXTURE, |w| {
        let mut t = w
            .open_table(TableDefinition::<&[u8], &[u8]>::new("headers"))
            .unwrap();
        let first = t.first().unwrap().unwrap().0.value().to_vec();
        t.remove(first.as_slice()).unwrap();
    });
    let e = verify_bytes(&dir, &broken, None).unwrap_err();
    assert!(e.inconsistency().is_some(), "{e}");
    // A table of the schema missing.
    let missing = edited(&dir, FIXTURE, |w| {
        assert!(
            w.delete_table(TableDefinition::<&[u8], &[u8]>::new("tickets"))
                .unwrap()
        );
    });
    let e = verify_bytes(&dir, &missing, None).unwrap_err();
    assert!(e.to_string().contains("tickets: table missing"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_backup_record_round_trips_and_its_layout_is_pinned() {
    let r = BackupRecord::new("simnet", 1, "2.2.0", -2, 3, [0xab; 32]).unwrap();
    let bytes = r.encode();
    let mut want = vec![1u8, 6];
    want.extend_from_slice(b"simnet");
    want.extend_from_slice(&[0, 0, 0, 1, 5]);
    want.extend_from_slice(b"2.2.0");
    want.extend_from_slice(&(-2i64).to_be_bytes());
    want.extend_from_slice(&3u64.to_be_bytes());
    want.extend_from_slice(&[0xab; 32]);
    assert_eq!(bytes, want);
    assert_eq!(BackupRecord::decode(&bytes).unwrap(), r);

    let mut rng = SplitMix64::from_env("backup-record");
    let text = |rng: &mut SplitMix64| -> String {
        let n = 1 + rng.below(255) as usize;
        (0..n)
            .map(|_| char::from(b'!' + rng.below(90) as u8))
            .collect()
    };
    for _ in 0..2000 {
        let r = BackupRecord::new(
            &text(&mut rng),
            1 + rng.below(u64::from(u32::MAX)) as u32,
            &text(&mut rng),
            rng.next_u64() as i64,
            rng.next_u64(),
            {
                let mut d = [0; 32];
                rng.fill(&mut d);
                d
            },
        )
        .unwrap();
        assert_eq!(BackupRecord::decode(&r.encode()).unwrap(), r);
    }
    // Only canonical encodings decode: valid encodings with a byte
    // changed, a length moved, bytes dropped or added.
    let base = BackupRecord::new("testnet3", 1, "0.1.0", 9, 9, [7; 32])
        .unwrap()
        .encode();
    let mut decoded = 0;
    for _ in 0..20_000 {
        let mut b = base.clone();
        match rng.below(3) {
            0 => {
                let i = rng.below(b.len() as u64) as usize;
                b[i] = rng.next_u64() as u8;
            }
            1 => b.truncate(rng.below(b.len() as u64) as usize),
            _ => {
                let n = 1 + rng.below(8) as usize;
                b.extend(rng.bytes(n));
            }
        }
        if let Ok(r) = BackupRecord::decode(&b) {
            assert_eq!(r.encode(), b);
            decoded += 1;
        }
    }
    eprintln!("{decoded} mutated encodings decoded");
    assert!(decoded > 0);
    // Every refusal of the decoder.
    let refusals: &[(&[u8], &str)] = &[
        (&[], "backup record len 0"),
        (&[2], "backup record: unknown format"),
        (&[1, 0], "backup record: empty network"),
        (&[1, 1, 0xff], "backup record: network is not UTF-8"),
        (&[1, 1, b'n', 0, 0, 0, 0], "backup record: schema version 0"),
        (&[1, 1, b'n', 0, 0, 0, 1, 0], "backup record: empty version"),
        (
            &[1, 1, b'n', 0, 0, 0, 1, 1, 0xc3],
            "backup record: version is not UTF-8",
        ),
        (&[1, 1, b'n', 0, 0, 0, 1, 1, b'v'], "backup record len 9"),
    ];
    for (b, text) in refusals {
        assert_eq!(BackupRecord::decode(b).unwrap_err().to_string(), *text);
    }
    let mut long = want.clone();
    long.push(0);
    assert_eq!(
        BackupRecord::decode(&long).unwrap_err().to_string(),
        format!("backup record len {}", long.len())
    );
    // The constructor's refusals.
    let n256 = "n".repeat(256);
    for (net, schema, ver) in [
        ("", 1, "v"),
        (n256.as_str(), 1, "v"),
        ("n", 0, "v"),
        ("n", 1, ""),
    ] {
        assert_eq!(
            BackupRecord::new(net, schema, ver, 0, 0, [0; 32])
                .unwrap_err()
                .kind(),
            Kind::Invalid
        );
    }
    BackupRecord::new(&"n".repeat(255), 1, &"v".repeat(255), 0, 0, [0; 32]).unwrap();
    // The store's file name is not dcrwallet's.
    assert_ne!(STORE_FILE_NAME, "wallet.db");
    check_backup_name(STORE_FILE_NAME).expect("a portable name");
}
