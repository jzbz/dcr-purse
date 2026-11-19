// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Fault injection (brief §6 Phase 2 exit): a power cut at every storage
//! operation of a scripted workload leaves the store at a committed state.
//!
//! The workload creates a store, runs wallet-shaped write transactions
//! (many small rows, a transaction record of several pages, overwrites, a
//! bulk delete that lets the file shrink), an initial sync of header and
//! filter batches with a header-only reorganisation, and more writes after
//! it. A reference run records the digest of every table after every
//! commit and counts the run's storage operations (writes, length changes,
//! syncs). Then, for every count `n`, a fresh run on the same starting image
//! loses power after its `n`th operation; the image the cut leaves is
//! reopened through the store's own open (which repairs it), and:
//!
//! - it holds exactly the rows of some commit (its digest is one the
//!   reference recorded), never a mixture;
//! - no commit acknowledged as durable is lost: the commit it holds is at
//!   or after the last durable commit that returned `Ok`, and at most the
//!   one commit in flight when the power went;
//! - under relaxed durability, the commits lost are only initial-sync
//!   batches committed without durability since the last durable one,
//!   fewer than the interval (`sync.rs`, ADR-0003 Decision item 9);
//! - the reopened store takes a further durable commit and opens cleanly
//!   after it.
//!
//! Cuts are clean (every unsynced write undone), torn (each 4 KiB piece of
//! each unsynced write kept or undone at random, the length kept or undone
//! at random) or a killed process's (every write kept, synced or not). A
//! backend whose sync acknowledges without making anything durable must make
//! the sweep fail: otherwise the sweep would prove nothing about durability.
//!
//! `PURSE_STORE_SWEEP_STEP` (default 1: every operation) cuts at every
//! that-many-th operation instead, for quick local runs.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: step and operation counters"
)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use purse_store::{
    BlockRow, CFilterRow, Error, Hash32, HeaderBytes, OpenOptions, Store, TableId, Tip,
};
use purse_testutil::SplitMix64;
use support::powerloss::{Cut, PowerLoss, Shared};
use support::{StoreDigest, digest, short};

/// One write of an update step.
#[derive(Clone, Debug)]
enum W {
    Put(TableId, Vec<u8>, Vec<u8>),
    Del(TableId, Vec<u8>),
}

/// One header-and-filter batch of initial sync.
#[derive(Clone, Debug)]
struct Batch {
    first_height: u32,
    blocks: Vec<(Hash32, HeaderBytes, CFilterRow)>,
    /// Heights removed (a header-only reorganisation) before the batch's
    /// blocks are recorded.
    remove: Vec<u32>,
}

#[derive(Clone, Debug)]
enum Step {
    /// A durable write transaction through `Store::update`.
    Update(Vec<W>),
    /// A batch through `InitialSync::batch`.
    Batch(Batch),
    /// `InitialSync::finish`.
    Finish,
}

/// Tables the update steps write: wallet rows, not the sync tables.
const WALLET_TABLES: &[TableId] = &[
    TableId::Accounts,
    TableId::AddrByHash,
    TableId::AddrByPath,
    TableId::TxMined,
    TableId::Credits,
    TableId::Unspent,
    TableId::Debits,
    TableId::History,
    TableId::Balances,
];

/// `base` to `base + extra - 1` random bytes.
fn sized(rng: &mut SplitMix64, base: usize, extra: u64) -> Vec<u8> {
    let n = base + rng.below(extra) as usize;
    rng.bytes(n)
}

fn hash(rng: &mut SplitMix64) -> Hash32 {
    let mut h = [0u8; 32];
    rng.fill(&mut h);
    Hash32(h)
}

/// The scripted workload. It is fixed (not seeded from the environment) so
/// that the number of cuts, and so the run time, does not vary.
fn workload() -> Vec<Step> {
    let mut rng = SplitMix64(0x706f_7765_726c_6f73);
    let mut steps = Vec::new();
    let mut live: Vec<(TableId, Vec<u8>)> = Vec::new();
    let update = |rng: &mut SplitMix64, live: &mut Vec<(TableId, Vec<u8>)>, rows: usize| {
        let mut ws = Vec::new();
        for _ in 0..rows {
            let t = WALLET_TABLES[rng.below(WALLET_TABLES.len() as u64) as usize];
            let k = sized(rng, 8, 40);
            let v = sized(rng, 0, 120);
            live.push((t, k.clone()));
            ws.push(W::Put(t, k, v));
        }
        // Overwrite and delete some earlier rows.
        for _ in 0..rows / 8 {
            if live.is_empty() {
                break;
            }
            let i = rng.below(live.len() as u64) as usize;
            let (t, k) = live[i].clone();
            if rng.below(2) == 0 {
                ws.push(W::Put(t, k, sized(rng, 0, 200)));
            } else {
                live.swap_remove(i);
                ws.push(W::Del(t, k));
            }
        }
        Step::Update(ws)
    };
    for rows in [40, 5, 120, 1, 60] {
        steps.push(update(&mut rng, &mut live, rows));
    }
    // A transaction record spanning several pages.
    steps.push(Step::Update(vec![W::Put(
        TableId::TxMined,
        rng.bytes(36),
        rng.bytes(60_000),
    )]));
    // Initial sync: 10 batches of 40 blocks, a reorganisation in the 7th.
    let mut height = 0u32;
    for b in 0..10 {
        let remove = if b == 6 {
            (height - 5..height).rev().collect()
        } else {
            Vec::new()
        };
        if b == 6 {
            height -= 5;
        }
        let blocks = (0..40)
            .map(|_| {
                let mut hdr = [0u8; 180];
                rng.fill(&mut hdr);
                let mut key = [0u8; 16];
                rng.fill(&mut key);
                let filter = sized(&mut rng, 150, 250);
                (hash(&mut rng), HeaderBytes(hdr), CFilterRow { key, filter })
            })
            .collect();
        steps.push(Step::Batch(Batch {
            first_height: height,
            blocks,
            remove,
        }));
        height += 40;
    }
    steps.push(Step::Finish);
    for rows in [30, 2, 90] {
        steps.push(update(&mut rng, &mut live, rows));
    }
    // A bulk delete: the file may give space back at its end.
    let mut ws: Vec<W> = live.drain(..).map(|(t, k)| W::Del(t, k)).collect();
    ws.truncate(ws.len() * 3 / 4);
    steps.push(Step::Update(ws));
    steps.push(update(&mut rng, &mut live, 3));
    steps
}

/// What a run acknowledged before it stopped.
#[derive(Clone, Copy, Debug, Default)]
struct Acked {
    /// Commits that returned `Ok`, the creation included.
    commits: usize,
    /// Of those, the number up to and including the last durable one.
    durable: usize,
}

fn apply_update(store: &Store, step_no: usize, ws: &[W]) -> purse_store::Result<()> {
    store.update(|tx| {
        for w in ws {
            match w {
                W::Put(t, k, v) => tx.put_raw(*t, k, v)?,
                W::Del(t, k) => {
                    tx.delete_raw(*t, k)?;
                }
            }
        }
        // Every commit changes the rows, so every commit has its own digest.
        tx.put_raw(TableId::Meta, b"test_step", &(step_no as u64).to_be_bytes())
    })
}

/// Runs the workload from creation on `backend`, stopping at the first
/// error. `on_commit` is called after every acknowledged commit with the
/// store and whether that commit was durable. Returns what was
/// acknowledged.
fn run(
    backend: &Arc<PowerLoss>,
    steps: &[Step],
    interval: u32,
    mut on_commit: impl FnMut(&Store, bool),
) -> Acked {
    let mut acked = Acked::default();
    let store = match OpenOptions::new().create_on_backend(Shared(backend.clone()), |tx| {
        tx.put_raw(TableId::Meta, b"test_step", &u64::MAX.to_be_bytes())
    }) {
        Ok(s) => s,
        Err(_) => return acked,
    };
    acked.commits = 1;
    acked.durable = 1;
    on_commit(&store, true);
    let mut sync = None;
    for (i, step) in steps.iter().enumerate() {
        let durable = match step {
            Step::Update(ws) => match apply_update(&store, i, ws) {
                Ok(()) => true,
                Err(_) => break,
            },
            Step::Batch(b) => {
                let s = sync.get_or_insert_with(|| {
                    store
                        .initial_sync()
                        .with_interval(interval)
                        .expect("interval in range")
                });
                let r = s.batch(|sb| {
                    for &h in &b.remove {
                        sb.remove_block(h)?;
                    }
                    let mut last = None;
                    for (j, (hash, hdr, filter)) in b.blocks.iter().enumerate() {
                        let height = b.first_height + j as u32;
                        sb.put_header(hash, hdr)?;
                        sb.put_cfilter(hash, filter)?;
                        sb.put_block(
                            height,
                            &BlockRow {
                                hash: *hash,
                                time: i64::from(height) * 300,
                                vote_bits: 1,
                                invalid: false,
                                wallet_txs: 0,
                            },
                        )?;
                        last = Some(Tip {
                            height,
                            hash: *hash,
                        });
                    }
                    if let Some(t) = last {
                        sb.set_tip(&t)?;
                        sb.set_processed_marker(&t.hash)?;
                    }
                    Ok::<_, Error>(())
                });
                match r {
                    Ok(((), durable)) => durable,
                    Err(_) => break,
                }
            }
            Step::Finish => {
                let Some(s) = sync.take() else { continue };
                let had_pending = s.pending() > 0;
                if s.finish().is_err() {
                    break;
                }
                if !had_pending {
                    continue;
                }
                // The finishing commit changes no row; it makes the
                // batches before it durable.
                true
            }
        };
        acked.commits += 1;
        if durable {
            acked.durable = acked.commits;
        }
        on_commit(&store, durable);
    }
    // A sync abandoned by an error is dropped here; its flush meets the
    // dead backend and fails, as in a crash.
    drop(sync);
    drop(store);
    acked
}

#[derive(Debug, Default)]
struct Report {
    trials: u64,
    cut_reached: u64,
    /// Reopened at the last durable acknowledged commit.
    at_last_durable: u64,
    /// Reopened past it: a later non-durable commit, or the one in flight.
    past_last_durable: u64,
    /// Non-durable acknowledged commits lost (relaxed mode only).
    lost_nondurable: u64,
    /// Largest number of acknowledged commits lost in one trial.
    max_lost: usize,
    /// No commit acknowledged, and the image does not open as a store.
    not_created: u64,
    failures: Vec<String>,
}

struct Mode {
    name: &'static str,
    /// The kind of cut; a torn cut's seed is mixed with the cut's place.
    cut: Cut,
    honest: bool,
    interval: u32,
}

fn sweep_step() -> u64 {
    std::env::var("PURSE_STORE_SWEEP_STEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&s| s > 0)
        .unwrap_or(1)
}

fn sweep(mode: &Mode) -> Report {
    let steps = workload();

    // The reference run: the digest after every commit (index 0: before
    // creation) and the number of storage operations.
    let reference = PowerLoss::new(Vec::new(), true);
    let mut digests: Vec<StoreDigest> = vec![[0; 32]];
    let mut durable_flags = vec![true];
    let acked = run(&reference, &steps, mode.interval, |store, durable| {
        digests.push(digest(&store.read().expect("read")));
        durable_flags.push(durable);
    });
    assert_eq!(acked.commits + 1, digests.len());
    let total = reference.ops();
    eprintln!(
        "{}: {} commits, {total} storage operations, {} syncs",
        mode.name,
        acked.commits,
        reference.syncs()
    );
    let mut seen: BTreeMap<StoreDigest, usize> = BTreeMap::new();
    for (i, d) in digests.iter().enumerate().skip(1) {
        // Only a finishing commit, which changes no row, repeats a digest,
        // that of the batch just before it.
        if let Some(prev) = seen.insert(*d, i) {
            assert_eq!(prev + 1, i, "commits {prev} and {i} have the same digest");
        }
    }

    let mut report = Report::default();
    let step = sweep_step();
    let mut n = 0u64;
    while n <= total {
        report.trials += 1;
        let b = PowerLoss::new(Vec::new(), mode.honest);
        b.power_fails_after(n);
        let acked = run(&b, &steps, mode.interval, |_, _| {});
        if b.power_failed() {
            report.cut_reached += 1;
        }
        let image = b.cut(match mode.cut {
            Cut::Torn(seed) => Cut::Torn(seed ^ n),
            other => other,
        });
        if let Err(why) = check_cut(&image, acked, &digests, &durable_flags, &mut report) {
            report
                .failures
                .push(format!("cut after {n} operations: {why}"));
        }
        n += step;
    }
    report
}

/// Reopens the image a cut left and checks it against the reference.
fn check_cut(
    image: &[u8],
    acked: Acked,
    digests: &[StoreDigest],
    durable_flags: &[bool],
    report: &mut Report,
) -> Result<(), String> {
    let b2 = PowerLoss::new(image.to_vec(), true);
    let store = match OpenOptions::new().open_on_backend(Shared(b2.clone())) {
        Ok(s) => s,
        Err(e) => {
            if acked.durable == 0 {
                // Creation never committed: the image holds no store, which
                // the open reports (an empty image opens as an empty redb
                // file, refused as not initialised).
                report.not_created += 1;
                return Ok(());
            }
            return Err(format!(
                "reopen failed with {} durable commits acknowledged: {e}",
                acked.durable
            ));
        }
    };
    let got = digest(&store.read().map_err(|e| e.to_string())?);
    // The commits the reopened store may hold: from the last durable
    // acknowledged one to the one in flight when the power went.
    let lo = acked.durable;
    let hi = (acked.commits + 1).min(digests.len() - 1);
    let Some(j) = (lo.max(1)..=hi).rev().find(|&j| digests[j] == got) else {
        let elsewhere = digests.iter().position(|d| *d == got);
        return Err(match elsewhere {
            Some(j) => format!(
                "reopened at commit {j}, outside {lo}..={hi} (acknowledged {}, durable {})",
                acked.commits, acked.durable
            ),
            None => format!(
                "reopened at no commit (digest {}; acknowledged {}, durable {})",
                short(&got),
                acked.commits,
                acked.durable
            ),
        });
    };
    if j == lo {
        report.at_last_durable += 1;
    } else {
        report.past_last_durable += 1;
    }
    if j < acked.commits {
        // Only non-durable commits may be lost, and only those after the
        // last durable one.
        let lost = acked.commits - j;
        if durable_flags[j + 1..=acked.commits].iter().any(|&d| d) {
            return Err(format!(
                "lost a durable commit: reopened at {j}, acknowledged {}",
                acked.commits
            ));
        }
        report.lost_nondurable += lost as u64;
        report.max_lost = report.max_lost.max(lost);
    }
    // The repaired store takes a durable commit and opens cleanly after it.
    store
        .update(|tx| tx.put_raw(TableId::Meta, b"after_cut", b"1"))
        .map_err(|e| format!("commit after reopen: {e}"))?;
    let after = digest(&store.read().map_err(|e| e.to_string())?);
    store
        .close()
        .map_err(|e| format!("close after reopen: {e}"))?;
    let store = OpenOptions::new()
        .check_integrity(true)
        .open_on_backend(Shared(b2))
        .map_err(|e| format!("second reopen: {e}"))?;
    let again = digest(&store.read().map_err(|e| e.to_string())?);
    if again != after {
        return Err("second reopen lost the commit after the cut".into());
    }
    Ok(())
}

fn assert_clean(mode: &Mode, r: &Report) {
    eprintln!("{}: {r:?}", mode.name);
    assert!(
        r.failures.is_empty(),
        "{}: {} of {} trials failed; first: {}",
        mode.name,
        r.failures.len(),
        r.trials,
        r.failures.first().map_or("", String::as_str)
    );
    assert!(r.trials > 100, "{}: only {} trials", mode.name, r.trials);
    assert!(r.cut_reached > 0, "{}: no cut was reached", mode.name);
}

fn durable(name: &'static str, cut: Cut) -> Mode {
    Mode {
        name,
        cut,
        honest: true,
        interval: 1,
    }
}

fn relaxed(name: &'static str, cut: Cut) -> Mode {
    Mode {
        name,
        cut,
        honest: true,
        interval: 3,
    }
}

#[test]
fn every_clean_cut_lands_on_the_last_durable_commit() {
    let mode = durable("durable, clean", Cut::Clean);
    let r = sweep(&mode);
    assert_clean(&mode, &r);
    assert_eq!(r.lost_nondurable, 0, "every commit was durable");
}

#[test]
fn every_torn_cut_lands_on_a_committed_state() {
    let mode = durable("durable, torn", Cut::Torn(0x746f_726e));
    let r = sweep(&mode);
    assert_clean(&mode, &r);
    assert_eq!(r.lost_nondurable, 0, "every commit was durable");
}

/// A killed process: every write survives. A commit whose writes all
/// happened before the kill is kept even if its sync never ran.
#[test]
fn every_kill_lands_on_a_committed_state() {
    let mode = durable("durable, killed", Cut::Kept);
    let r = sweep(&mode);
    assert_clean(&mode, &r);
    assert_eq!(r.lost_nondurable, 0, "every commit was durable");
    assert!(r.past_last_durable > 0, "no commit in flight landed: {r:?}");
}

/// The bound of relaxed durability: at most the batches committed without
/// durability since the last durable commit are lost, fewer than the
/// interval.
fn assert_relaxed(mode: &Mode) -> Report {
    let r = sweep(mode);
    assert_clean(mode, &r);
    assert!(
        r.max_lost < mode.interval as usize,
        "{}: lost {} commits",
        mode.name,
        r.max_lost
    );
    r
}

#[test]
fn relaxed_sync_loses_only_batches_since_the_last_durable_commit() {
    let r = assert_relaxed(&relaxed("relaxed, clean", Cut::Clean));
    // A cut inside the relaxed stretch loses its non-durable batches; the
    // sweep must have seen that happen.
    assert!(r.lost_nondurable > 0, "{r:?}");
}

#[test]
fn relaxed_sync_torn_or_killed_stays_within_the_bound() {
    assert_relaxed(&relaxed("relaxed, torn", Cut::Torn(0x7265_6c61)));
    assert_relaxed(&relaxed("relaxed, killed", Cut::Kept));
}

/// A backend whose sync acknowledges without making anything durable must
/// make the sweep fail: a reopened image then lacks commits acknowledged as
/// durable.
#[test]
fn a_dishonest_sync_is_caught() {
    let mode = Mode {
        name: "dishonest",
        cut: Cut::Clean,
        honest: false,
        interval: 1,
    };
    let r = sweep(&mode);
    eprintln!(
        "{}: {} failures of {} trials",
        mode.name,
        r.failures.len(),
        r.trials
    );
    assert!(
        r.failures.len() as u64 > r.trials / 2,
        "a backend that never makes anything durable went unnoticed: {r:?}"
    );
}

/// A store created on a backend opens on another backend over the same
/// image, and creating on a backend that already holds a store is refused
/// without touching it.
#[test]
fn backends_open_and_refuse_as_files_do() {
    let b = PowerLoss::new(Vec::new(), true);
    let store = OpenOptions::new()
        .create_on_backend(Shared(b.clone()), |tx| {
            tx.put_raw(TableId::Accounts, &[1, 0, 0, 0], b"row")
        })
        .unwrap();
    let want = digest(&store.read().unwrap());
    store.close().unwrap();

    let err = OpenOptions::new()
        .create_on_backend(Shared(b.clone()), |_| Ok(()))
        .err()
        .expect("a backend holding a store is refused");
    assert_eq!(err.kind(), purse_errors::Kind::Invalid, "{err}");
    let store = OpenOptions::new().open_on_backend(Shared(b)).unwrap();
    assert_eq!(digest(&store.read().unwrap()), want);
}

/// A storage error inside a commit leaves the commit's outcome unknown, and
/// the error says so (`Error::commit_outcome_unknown`): the store refuses
/// later writes, and after a reopen the commit is either wholly there or
/// wholly absent. One storage operation of one update fails, at each
/// operation in turn, the power staying on. Some failures (a failed sync of
/// the commit) leave the commit in the file, which is why `Err` alone
/// cannot be read as "not kept".
#[test]
fn a_failed_commit_reports_an_unknown_outcome() {
    let base = {
        let b = PowerLoss::new(Vec::new(), true);
        let store = OpenOptions::new()
            .create_on_backend(Shared(b.clone()), |_| Ok(()))
            .unwrap();
        store.close().unwrap();
        b.image()
    };
    let key_a = [1u8, 0, 0, 0];
    let key_b = [2u8, 0, 0, 0];
    let ops_of_update = {
        let b = PowerLoss::new(base.clone(), true);
        let store = OpenOptions::new()
            .open_on_backend(Shared(b.clone()))
            .unwrap();
        let before = b.ops();
        store
            .update(|tx| tx.put_raw(TableId::Accounts, &key_a, b"a"))
            .unwrap();
        b.ops() - before
    };
    assert!(ops_of_update > 2, "{ops_of_update}");
    let (mut kept, mut lost) = (0, 0);
    for n in 0..ops_of_update {
        let b = PowerLoss::new(base.clone(), true);
        let store = OpenOptions::new()
            .open_on_backend(Shared(b.clone()))
            .unwrap();
        b.fail_once_after(n);
        let err = store
            .update(|tx| tx.put_raw(TableId::Accounts, &key_a, b"a"))
            .expect_err("a storage operation of the commit failed");
        assert!(err.commit_outcome_unknown(), "op {n}: {err}");
        assert_eq!(err.kind(), purse_errors::Kind::Io, "op {n}: {err}");
        // The handle refuses further writes; that refusal is not a failed
        // commit, and nothing of it is kept.
        let refused = store
            .update(|tx| tx.put_raw(TableId::Accounts, &key_b, b"b"))
            .expect_err("writes are refused until a reopen");
        assert!(!refused.commit_outcome_unknown(), "op {n}: {refused}");
        drop(store);
        let store = OpenOptions::new()
            .open_on_backend(Shared(b.clone()))
            .unwrap_or_else(|e| panic!("op {n}: reopen: {e}"));
        let tx = store.read().unwrap();
        assert_eq!(tx.get_raw(TableId::Accounts, &key_b).unwrap(), None);
        match tx.get_raw(TableId::Accounts, &key_a).unwrap() {
            Some(v) => {
                assert_eq!(v, b"a");
                kept += 1;
            }
            None => lost += 1,
        }
    }
    eprintln!("{ops_of_update} operations: commit kept after {kept} failures, lost after {lost}");
    assert!(kept > 0, "no failure left the commit in place");
    assert!(lost > 0, "no failure lost the commit");
}

/// Two initial syncs on one store share the bound: their batches count
/// together, so a power cut loses fewer than `MAX_DURABLE_INTERVAL` of
/// them however the syncs interleave on one thread, and a durable
/// `Store::update` resets the count for both. Syncs on several threads are
/// `concurrent_syncs_keep_the_durable_bound`'s.
#[test]
fn two_initial_syncs_share_the_durable_bound() {
    use purse_store::MAX_DURABLE_INTERVAL;
    let b = PowerLoss::new(Vec::new(), true);
    let store = OpenOptions::new()
        .create_on_backend(Shared(b.clone()), |_| Ok(()))
        .unwrap();
    let block = |i: u32| BlockRow {
        hash: Hash32({
            let mut h = [0u8; 32];
            h[..4].copy_from_slice(&i.to_be_bytes());
            h
        }),
        ..BlockRow::default()
    };
    let mut a = store.initial_sync();
    let mut c = store.initial_sync();
    let mut durable = 0;
    let mut height = 0u32;
    for _ in 0..(MAX_DURABLE_INTERVAL - 1) {
        for s in [&mut a, &mut c] {
            height += 1;
            let ((), d) = s.batch(|x| x.put_block(height, &block(height))).unwrap();
            durable += u32::from(d);
            assert!(s.pending() < MAX_DURABLE_INTERVAL);
        }
    }
    assert_eq!(a.pending(), c.pending());
    assert!(durable >= 1, "98 batches with no durable commit");
    // Every batch up to the last durable commit survives the cut; the
    // syncs' closing commits fail with the power gone.
    let pending = a.pending();
    b.power_fails_after(0);
    drop(a);
    drop(c);
    drop(store);
    let image = b.cut(Cut::Clean);
    let store = OpenOptions::new()
        .open_on_backend(Shared(PowerLoss::new(image, true)))
        .unwrap();
    let kept = store.read().unwrap().len(TableId::Blocks).unwrap();
    assert_eq!(kept, u64::from(height - pending), "pending {pending}");
    assert!(height - (kept as u32) < MAX_DURABLE_INTERVAL);

    // A durable update resets the count every sync sees.
    let mut s = store.initial_sync();
    let mut t = store.initial_sync();
    s.batch(|x| x.put_block(1_000, &block(1_000))).unwrap();
    assert_eq!(t.pending(), 1);
    store
        .update(|tx| tx.put_raw(TableId::Tickets, &[9; 32], b""))
        .unwrap();
    assert_eq!(s.pending(), 0);
    assert_eq!(t.pending(), 0);
    let ((), d) = t.batch(|x| x.put_block(1_001, &block(1_001))).unwrap();
    assert!(!d);
    s.finish().unwrap();
    assert_eq!(t.pending(), 0);
    t.finish().unwrap();
}

/// Initial syncs on several threads at once, with durable `Store::update`s
/// among them, keep the shared bound: in commit order (each commit's
/// closure takes a ticket, and closures run one at a time under the
/// writer lock), no run of consecutive non-durable commits reaches
/// `MAX_DURABLE_INTERVAL`, and once every sync's `finish` has returned a
/// clean power cut loses no batch.
///
/// This is a stress test of the whole path, not the test of the race it
/// was written for: redb releases its writer lock inside `commit`, and
/// when the count was updated after that, outside any lock, runs of 95
/// and more showed up, but only in some runs of 160,000 commits on two
/// threads and in most runs on sixteen; 15 runs of this test (at three
/// sizes up to 64,000 commits on sixteen threads) against that code all
/// passed. The window is pinned deterministically by the unit test
/// `a_durable_reset_never_lands_after_a_later_relaxed_commit` in
/// `src/store.rs`, which fails on that code every time.
#[test]
fn concurrent_syncs_keep_the_durable_bound() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Barrier, Mutex};

    use purse_store::MAX_DURABLE_INTERVAL;

    const ROUNDS: u32 = 4;
    const THREADS: u32 = 8;
    const PER_THREAD: u32 = 1_000;
    let block = |i: u32| BlockRow {
        hash: Hash32({
            let mut h = [0u8; 32];
            h[..4].copy_from_slice(&i.to_be_bytes());
            h
        }),
        ..BlockRow::default()
    };
    let mut worst = 0u32;
    let mut durable_seen = 0u32;
    for round in 0..ROUNDS {
        let b = PowerLoss::new(Vec::new(), true);
        let store = OpenOptions::new()
            .create_on_backend(Shared(b.clone()), |_| Ok(()))
            .unwrap();
        let ticket = AtomicU32::new(0);
        let log: Mutex<Vec<(u32, bool)>> = Mutex::new(Vec::new());
        let before_finish = Barrier::new(THREADS as usize);
        std::thread::scope(|scope| {
            for t in 0..THREADS {
                let (store, ticket, log, before_finish) = (&store, &ticket, &log, &before_finish);
                let block = &block;
                scope.spawn(move || {
                    let mut sync = store.initial_sync();
                    for i in 0..PER_THREAD {
                        let height = t * 1_000_000 + i + 1;
                        // In odd rounds thread 0 also makes a durable
                        // client write now and then, which resets every
                        // sync's count; even rounds are syncs alone.
                        let entry = if round % 2 == 1 && t == 0 && i % 97 == 0 {
                            let n = store
                                .update(|tx| {
                                    let n = ticket.fetch_add(1, Ordering::SeqCst);
                                    tx.put_raw(TableId::Tickets, &height.to_be_bytes(), b"")?;
                                    Ok::<_, Error>(n)
                                })
                                .unwrap();
                            (n, true)
                        } else {
                            sync.batch(|x| {
                                let n = ticket.fetch_add(1, Ordering::SeqCst);
                                x.put_block(height, &block(height))?;
                                Ok::<_, Error>(n)
                            })
                            .unwrap()
                        };
                        log.lock().unwrap().push(entry);
                    }
                    // No sync's closing commit falls among the logged ones.
                    before_finish.wait();
                    sync.finish().unwrap();
                });
            }
        });
        let mut log = log.into_inner().unwrap();
        log.sort_unstable();
        let mut run = 0u32;
        for &(_, durable) in &log {
            if durable {
                durable_seen += 1;
                run = 0;
            } else {
                run += 1;
                worst = worst.max(run);
            }
        }
        assert!(
            worst < MAX_DURABLE_INTERVAL,
            "round {round}: {worst} consecutive non-durable commits"
        );
        let written = store.read().unwrap().len(TableId::Blocks).unwrap();
        b.power_fails_after(0);
        drop(store);
        let image = b.cut(Cut::Clean);
        let store = OpenOptions::new()
            .open_on_backend(Shared(PowerLoss::new(image, true)))
            .unwrap();
        let kept = store.read().unwrap().len(TableId::Blocks).unwrap();
        assert_eq!(
            kept, written,
            "round {round}: every finish returned Ok, and a clean cut lost batches"
        );
    }
    eprintln!("longest non-durable run {worst}; durable commits {durable_seen}");
    // The workload must reach the bound for the check to mean anything.
    assert_eq!(
        worst,
        MAX_DURABLE_INTERVAL - 1,
        "the bound was never reached"
    );
}
