// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Cursor conformance (brief §6 Phase 2): the store's cursors and ranges
//! against bbolt v1.3.12 itself, reached through dcrwallet's `walletdb`
//! interface and bdb driver by the oracle `tools/oracle/cmd/boltcursor`,
//! and against the frozen vectors that oracle wrote
//! (`tests/data/cursor_vectors.jsonl`).
//!
//! bbolt's page layout, and with it which cursor defects a script
//! reaches, depends on its page size, and a database dcrwallet creates
//! takes the host's (4096 bytes on x86-64 Linux and Windows, 16384 on
//! Apple silicon). The oracle sets the page size instead, so every host
//! runs the same layout: 4096 bytes ([`PAGE_SIZE`]) for every script, and
//! 16 and 64 KiB as well for the named cases, a random corpus and the
//! scripts that need several leaves. The
//! oracle reports the layout it built, and the tests assert that those
//! scripts had several leaves.
//!
//! The store's side walks `tickets` as `c`; one test runs the named cases
//! and a corpus with the cursor on each record table of Phase 2 in turn
//! (`purse_store::RECORD_TABLES`), and another holds the one record order
//! a client sees, the unspent multisig outputs of a script, against
//! bbolt's walk of dcrwallet's `mu` keys.
//!
//! A script is a key set for a table `c`, one for a table `o`, and a list
//! of operations run in one transaction against one cursor on `c`:
//! cursor moves, cursor deletes, puts, deletes and gets on `c`, writes to
//! `o`, a fresh cursor, a key count and a bounded range walked as
//! dcrwallet walks one. Every script runs on the store in a read
//! transaction ([`purse_store::ReadCursor`]) when it writes nothing, and
//! always in a write transaction ([`purse_store::WriteCursor`]).
//!
//! What is compared. Scripts marked exact (the named cases, the
//! single-direction delete walks, the dcrwallet patterns and every
//! read-only script) are compared op by op and by final contents. In the
//! random write scripts an op is compared where bbolt's answer does not
//! depend on its page layout (the module documentation of
//! `purse_store::cursor`, spec 01 §10 Q9):
//!
//! - `first`, `last`, `seek`, `fresh`, `get`, a forward range, and every
//!   write's result, always, while the two sides' contents agree;
//! - `next`, and the cursor's `delete`, when the cursor's table has not
//!   been written since the cursor's last `first`, `last` or `seek`, and
//!   the cursor has not since run off the end with a `next` in a
//!   transaction that deleted from the table: bbolt's `next` leaves its
//!   cursor on the last leaf even when that leaf was emptied in the
//!   transaction, so its later `Delete` removes nothing and its `Prev`
//!   returns the entry it ran off from again (QK-0749);
//! - `prev` and a reverse range as `next`, and only when nothing was
//!   deleted from the table in the transaction: bbolt's `Prev` that steps
//!   into a leaf emptied earlier in the transaction returns nil
//!   (QK-0747);
//! - a `prev` that runs off the start (returns nil from a positioned
//!   cursor) re-runs bbolt's `first`, a fresh descent, and leaves the
//!   cursor where `First` would: so it counts as a positioning, and a
//!   further `prev` while the cursor is still off the start is compared
//!   whatever was written since, provided nothing had been deleted from
//!   the table when the cursor ran off (else `first` may have skipped an
//!   emptied leaf, and the next `Prev` steps back into it, QK-0747);
//! - `count` (bbolt's `KeyN`) when the table has not been written in the
//!   transaction (spec 04 §5.1 item 6);
//! - final contents of `c` unless a cursor `delete` went uncompared (the
//!   two sides could then have deleted different keys); of `o` always.
//!
//! A script in which bbolt panics (a cursor `Delete` that indexes -1,
//! QK-0748) is accepted only where the panicking op is not compared. One in
//! which bbolt never answers cannot be compared at all: its `Last` on a
//! bucket of several leaves emptied in the transaction loops forever
//! (QK-0750), and so does a reverse range without an upper bound there,
//! which starts with `Last`. Every oracle call has a time limit, so such a
//! script fails naming QK-0750 instead of hanging the test; no generator
//! here reaches it (a multi-leaf table holds about 140 keys or more at
//! 4 KiB pages, and
//! no generated script deletes them all before a `Last`). The divergences
//! outside the rule are pinned by named scripts that hold the exact ops
//! where Go and the store differ (bolt#620, `KeyN` in a write transaction,
//! a `Next` after `Next` ran off the end of a leaf not yet written in the
//! transaction, QK-0747, QK-0749) or where bbolt panics (a cursor `Delete` never
//! positioned, QK-0748), in the live run and in the frozen vectors, and
//! the hang by a test of its own. The multi-leaf ones are run again at 16
//! and 64 KiB pages, as they are and with their keys scaled to the page
//! size: as they are their bucket is one leaf and bbolt agrees with the
//! store; scaled, they differ as at 4 KiB. The named cases, a random
//! corpus of its own seed, edge scripts and the other divergences run at
//! those sizes too, under the same rule.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and indices over small generated scripts"
)]

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use purse_store::{Codec as _, Error, OpenOptions, RawEntry, ReadTx, Store, TableId, WriteTx};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip, unhex};
use sha2::{Digest, Sha256};

thread_local! {
    /// The table every script's cursor walks on the store's side (`c` in
    /// the oracle): `tickets`, or the record table a test names
    /// ([`with_cursor_table`]). bbolt's answers do not depend on it.
    static CURSOR_TABLE: Cell<TableId> = const { Cell::new(TableId::Tickets) };
}

/// The table every script's cursor walks on this thread.
fn c_table() -> TableId {
    CURSOR_TABLE.with(Cell::get)
}

/// Runs `f` with the store's side of every script on table `t`.
fn with_cursor_table<R>(t: TableId, f: impl FnOnce() -> R) -> R {
    let was = CURSOR_TABLE.with(|c| c.replace(t));
    let r = f();
    CURSOR_TABLE.with(|c| c.set(was));
    r
}

/// The other table (`o` in the oracle).
const O: TableId = TableId::Unspent;

// ---------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    First,
    Last,
    Next,
    Prev,
    Seek(Vec<u8>),
    Delete,
    Put(Vec<u8>, Vec<u8>),
    Del(Vec<u8>),
    Get(Vec<u8>),
    OPut(Vec<u8>, Vec<u8>),
    ODel(Vec<u8>),
    Fresh,
    Count,
    Range {
        lo: Vec<u8>,
        hi: Option<Vec<u8>>,
        rev: bool,
    },
}

impl Op {
    fn to_json(&self) -> Value {
        match self {
            Op::First => json!({"op": "first"}),
            Op::Last => json!({"op": "last"}),
            Op::Next => json!({"op": "next"}),
            Op::Prev => json!({"op": "prev"}),
            Op::Seek(k) => json!({"op": "seek", "k": hex(k)}),
            Op::Delete => json!({"op": "delete"}),
            Op::Put(k, v) => json!({"op": "put", "k": hex(k), "v": hex(v)}),
            Op::Del(k) => json!({"op": "del", "k": hex(k)}),
            Op::Get(k) => json!({"op": "get", "k": hex(k)}),
            Op::OPut(k, v) => json!({"op": "oput", "k": hex(k), "v": hex(v)}),
            Op::ODel(k) => json!({"op": "odel", "k": hex(k)}),
            Op::Fresh => json!({"op": "fresh"}),
            Op::Count => json!({"op": "count"}),
            Op::Range { lo, hi, rev } => {
                let mut o = json!({"op": "range", "k": hex(lo)});
                if let Some(hi) = hi {
                    o["hi"] = json!(hex(hi));
                }
                if *rev {
                    o["rev"] = json!(true);
                }
                o
            }
        }
    }

    fn from_json(v: &Value) -> Op {
        let bytes = |key: &str| unhex(v.get(key).and_then(Value::as_str).unwrap_or(""));
        match v["op"].as_str().expect("op name") {
            "first" => Op::First,
            "last" => Op::Last,
            "next" => Op::Next,
            "prev" => Op::Prev,
            "seek" => Op::Seek(bytes("k")),
            "delete" => Op::Delete,
            "put" => Op::Put(bytes("k"), bytes("v")),
            "del" => Op::Del(bytes("k")),
            "get" => Op::Get(bytes("k")),
            "oput" => Op::OPut(bytes("k"), bytes("v")),
            "odel" => Op::ODel(bytes("k")),
            "fresh" => Op::Fresh,
            "count" => Op::Count,
            "range" => Op::Range {
                lo: bytes("k"),
                hi: v.get("hi").and_then(Value::as_str).map(unhex),
                rev: v.get("rev").and_then(Value::as_bool).unwrap_or(false),
            },
            other => panic!("unknown op {other:?}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Script {
    name: String,
    entries: Vec<RawEntry>,
    other: Vec<RawEntry>,
    writable: bool,
    /// Every op and the final contents are compared, not only what the
    /// conformance rule covers.
    exact: bool,
    ops: Vec<Op>,
}

fn pairs_json(p: &[RawEntry]) -> Value {
    Value::Array(p.iter().map(|(k, v)| json!([hex(k), hex(v)])).collect())
}

fn pairs_from(v: &Value) -> Vec<RawEntry> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    (
                        unhex(p[0].as_str().expect("key")),
                        unhex(p[1].as_str().expect("value")),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Script {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "exact": self.exact,
            "writable": self.writable,
            "entries": pairs_json(&self.entries),
            "other": pairs_json(&self.other),
            "ops": self.ops.iter().map(Op::to_json).collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Value) -> Script {
        Script {
            name: v["name"].as_str().expect("name").to_owned(),
            exact: v["exact"].as_bool().expect("exact"),
            writable: v["writable"].as_bool().expect("writable"),
            entries: pairs_from(&v["entries"]),
            other: pairs_from(&v["other"]),
            ops: v["ops"]
                .as_array()
                .expect("ops")
                .iter()
                .map(Op::from_json)
                .collect(),
        }
    }

    fn request(&self, path: &Path, page_size: u32) -> Value {
        json!({
            "path": path,
            "page_size": page_size,
            "entries": pairs_json(&self.entries),
            "other": pairs_json(&self.other),
            "writable": self.writable,
            "ops": self.ops.iter().map(Op::to_json).collect::<Vec<_>>(),
        })
    }
}

// ---------------------------------------------------------------------
// The store's side
// ---------------------------------------------------------------------

/// What one run of a script returns, in the oracle's shape.
#[derive(Clone, Debug, PartialEq)]
struct Outcome {
    results: Vec<Value>,
    final_: Option<Value>,
}

fn moved(e: purse_store::Result<Option<RawEntry>>) -> purse_store::Result<Value> {
    Ok(match e? {
        Some((k, v)) => json!({"k": hex(&k), "v": hex(&v)}),
        None => json!({"k": null, "v": null}),
    })
}

fn done(r: purse_store::Result<()>) -> Value {
    match r {
        Ok(()) => json!({}),
        Err(e) => {
            let mut o = json!({"err": e.to_string()});
            if e.kind() != purse_errors::Kind::Other {
                o["kind"] = json!(e.kind().as_str());
            }
            o
        }
    }
}

fn hi_bound(hi: &Option<Vec<u8>>) -> Bound<&[u8]> {
    hi.as_deref().map_or(Bound::Unbounded, Bound::Excluded)
}

/// Lower above upper: redb's range is not asked (what bbolt's walk
/// returns then is empty, and so is this).
fn inverted(lo: &[u8], hi: &Option<Vec<u8>>) -> bool {
    hi.as_deref().is_some_and(|h| h < lo)
}

fn kv_json(
    entries: impl Iterator<Item = purse_store::Result<RawEntry>>,
) -> purse_store::Result<Value> {
    let kv: Vec<Value> = entries
        .map(|e| e.map(|(k, v)| json!([hex(&k), hex(&v)])))
        .collect::<purse_store::Result<_>>()?;
    Ok(json!({"kv": kv}))
}

fn read_range(
    tx: &ReadTx,
    lo: &[u8],
    hi: &Option<Vec<u8>>,
    rev: bool,
) -> purse_store::Result<Value> {
    if inverted(lo, hi) {
        return Ok(json!({"kv": []}));
    }
    let it = tx.range_raw(c_table(), Bound::Included(lo), hi_bound(hi))?;
    if rev { kv_json(it.rev()) } else { kv_json(it) }
}

fn write_range(
    tx: &mut WriteTx<'_>,
    lo: &[u8],
    hi: &Option<Vec<u8>>,
    rev: bool,
) -> purse_store::Result<Value> {
    if inverted(lo, hi) {
        return Ok(json!({"kv": []}));
    }
    let it = tx.range_raw(c_table(), Bound::Included(lo), hi_bound(hi))?;
    if rev { kv_json(it.rev()) } else { kv_json(it) }
}

fn new_store(s: &Script) -> Store {
    OpenOptions::new()
        .create_on_backend(redb::backends::InMemoryBackend::new(), |tx| {
            for (k, v) in &s.entries {
                tx.put_raw(c_table(), k, v)?;
            }
            for (k, v) in &s.other {
                tx.put_raw(O, k, v)?;
            }
            Ok(())
        })
        .expect("create the store")
}

/// Runs a read-only script with a [`purse_store::ReadCursor`].
fn run_read(store: &Store, s: &Script) -> Vec<Value> {
    let tx = store.read().expect("read transaction");
    let mut c = tx.cursor(c_table()).expect("cursor");
    let mut out = Vec::new();
    for op in &s.ops {
        let r = match op {
            Op::First => moved(c.first()),
            Op::Last => moved(c.last()),
            Op::Next => moved(c.next()),
            Op::Prev => moved(c.prev()),
            Op::Seek(k) => moved(c.seek(k)),
            Op::Get(k) => tx
                .get_raw(c_table(), k)
                .map(|v| json!({"v": v.map(|v| hex(&v))})),
            Op::Count => tx.len(c_table()).map(|n| json!({"n": n})),
            Op::Fresh => {
                c = tx.cursor(c_table()).expect("cursor");
                Ok(json!({}))
            }
            Op::Range { lo, hi, rev } => read_range(&tx, lo, hi, *rev),
            w => panic!("{}: write {w:?} in a read-only script", s.name),
        };
        out.push(r.expect("read op"));
    }
    out
}

/// Ends a write transaction: `Done` aborts it with the results.
enum Abort {
    Store(Error),
    Done(Vec<Value>),
}

impl From<Error> for Abort {
    fn from(e: Error) -> Self {
        Abort::Store(e)
    }
}

fn write_ops(tx: &mut WriteTx<'_>, s: &Script) -> purse_store::Result<Vec<Value>> {
    let mut out = Vec::new();
    let mut c = tx.cursor(c_table())?;
    for op in &s.ops {
        let r = match op {
            Op::First => moved(c.first())?,
            Op::Last => moved(c.last())?,
            Op::Next => moved(c.next())?,
            Op::Prev => moved(c.prev())?,
            Op::Seek(k) => moved(c.seek(k))?,
            Op::Delete => done(c.delete().map(drop)),
            Op::Put(k, v) => done(c.tx().put_raw(c_table(), k, v)),
            Op::Del(k) => done(c.tx().delete_raw(c_table(), k).map(drop)),
            Op::Get(k) => json!({"v": c.tx().get_raw(c_table(), k)?.map(|v| hex(&v))}),
            Op::OPut(k, v) => done(c.tx().put_raw(O, k, v)),
            Op::ODel(k) => done(c.tx().delete_raw(O, k).map(drop)),
            Op::Fresh => {
                c = tx.cursor(c_table())?;
                json!({})
            }
            Op::Count => json!({"n": c.tx().len(c_table())?}),
            Op::Range { lo, hi, rev } => write_range(c.tx(), lo, hi, *rev)?,
        };
        out.push(r);
    }
    Ok(out)
}

fn contents(store: &Store) -> Value {
    let tx = store.read().expect("read transaction");
    let dump = |t: TableId| {
        let kv: Vec<Value> = tx
            .range_raw(t, Bound::Unbounded, Bound::Unbounded)
            .expect("range")
            .map(|e| {
                let (k, v) = e.expect("entry");
                json!([hex(&k), hex(&v)])
            })
            .collect();
        Value::Array(kv)
    };
    json!({"c": dump(c_table()), "o": dump(O)})
}

/// Runs a script with a [`purse_store::WriteCursor`]: committed when it
/// writes, aborted otherwise.
fn run_write(s: &Script) -> Outcome {
    let store = new_store(s);
    if s.writable {
        let results = store
            .update(|tx| write_ops(tx, s))
            .unwrap_or_else(|e| panic!("{}: {e}", s.name));
        Outcome {
            results,
            final_: Some(contents(&store)),
        }
    } else {
        match store.update(|tx| -> Result<(), Abort> { Err(Abort::Done(write_ops(tx, s)?)) }) {
            Err(Abort::Done(results)) => Outcome {
                results,
                final_: None,
            },
            Err(Abort::Store(e)) => panic!("{}: {e}", s.name),
            Ok(()) => unreachable!(),
        }
    }
}

/// Every run of `s` on the store: the read cursor (read-only scripts) and
/// the write cursor.
fn runs(s: &Script) -> Vec<(&'static str, Outcome)> {
    let mut out = Vec::new();
    if !s.writable {
        let store = new_store(s);
        out.push((
            "read cursor",
            Outcome {
                results: run_read(&store, s),
                final_: None,
            },
        ));
    }
    out.push(("write cursor", run_write(s)));
    out
}

fn go_outcome(go: &Value) -> Outcome {
    Outcome {
        results: go["results"].as_array().expect("results").clone(),
        final_: go.get("final").filter(|f| !f.is_null()).cloned(),
    }
}

/// How long one script may take in the oracle. The largest generated
/// script answers in well under a second; only bbolt's `Last` loop
/// (QK-0750) runs into this.
const ORACLE_LIMIT: Duration = Duration::from_secs(120);

/// Numbers the oracle's database files across every test of this binary,
/// which run on parallel threads: each call gets a path of its own.
static ORACLE_RUNS: AtomicUsize = AtomicUsize::new(0);

/// A database path no other oracle call of this process uses.
fn oracle_db_path() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("purse-store-conformance");
    std::fs::create_dir_all(&dir).expect("create dir");
    let n = ORACLE_RUNS.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("s{}-{n}.db", std::process::id()));
    // Left by an earlier process that had this process id and crashed;
    // no live call of this process has the name.
    let _ = std::fs::remove_file(&path);
    path
}

/// The bbolt page size the scripts run at unless a test names another.
///
/// dcrwallet's driver opens its database with `bolt.Open(path, 0600, nil)`,
/// so the database takes bbolt's default, the host's page size: 4096 bytes
/// on x86-64 Linux and Windows, 16384 on Apple silicon. How many keys a
/// leaf holds follows from it, and so whether a script reaches the
/// multi-leaf defects (QK-0747, QK-0749, QK-0750). The oracle sets the page
/// size instead (`tools/oracle/cmd/boltcursor`), so every host runs the
/// same bbolt layout; 4096 is the size of the common hosts and of the frozen
/// vectors. `multileaf_defects_at_16_kib_pages` and
/// `multileaf_defects_at_64_kib_pages` run the multi-leaf scripts, the
/// named cases and a random corpus at the larger sizes.
const PAGE_SIZE: u32 = 4096;

/// Runs `s` in the oracle at [`PAGE_SIZE`].
fn run_oracle(oracle: &mut Oracle, s: &Script) -> Value {
    run_oracle_at(oracle, s, PAGE_SIZE)
}

/// Runs `s` in the oracle with bbolt pages of `page_size` bytes, and
/// checks that the database had that page size.
fn run_oracle_at(oracle: &mut Oracle, s: &Script, page_size: u32) -> Value {
    let go = oracle
        .call_within("run", s.request(&oracle_db_path(), page_size), ORACLE_LIMIT)
        .unwrap_or_else(|| {
            panic!(
                "{}: bbolt did not answer within {ORACLE_LIMIT:?}; a Last (or an unbounded \
                 reverse range) on a bucket of several leaves emptied in the transaction \
                 never returns (QK-0750)\nscript: {}",
                s.name,
                s.to_json()
            )
        });
    assert_eq!(
        go["layout"]["page_size"].as_u64(),
        Some(u64::from(page_size)),
        "{}: the oracle's page size: {}",
        s.name,
        go["layout"]
    );
    go
}

/// The number of leaf pages of `c` after the setup transaction, from the
/// oracle's answer (0 for a bucket stored inline in its parent).
fn leaf_pages(go: &Value) -> u64 {
    go["layout"]["leaf_pages"]
        .as_u64()
        .expect("layout.leaf_pages")
}

// ---------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------

/// The least number of operations a random corpus of 400 scripts must
/// compare for each one the rule skips: the guard that the rule has not
/// grown to skip what it should compare. The ratio follows the seed. Over
/// `PURSE_TEST_SEED` 1 to 150 its lowest was 8.32 at 4 KiB pages (seed 40),
/// 8.85 at 16 KiB (seed 134) and 8.75 at 64 KiB (seed 67), against 11.4 at
/// the default seed. The guard sits about a quarter below those, so that
/// a fresh seed passes, while a rule that skipped twice as much would fail
/// at the default seed. Until 2026-11-20 it was 10, which 9, 9 and 7 of
/// those 150 seeds missed at 4, 16 and 64 KiB, with no comparison
/// failing.
const MIN_COMPARED_PER_SKIPPED: usize = 6;

/// [`MIN_COMPARED_PER_SKIPPED`] for the corpus of 60 smaller scripts run on
/// each record table: its lowest over the same seeds was 5.31 (seed 66).
const MIN_COMPARED_PER_SKIPPED_RECORD_TABLES: usize = 4;

/// The least number of the 400 random scripts that must span several
/// leaves at 4 KiB pages: over the same seeds the fewest was 46 (seed
/// 108), against 69 at the default seed. It was 50 until 2026-11-20.
const MIN_MULTILEAF_SCRIPTS: usize = 35;

#[derive(Default, Debug)]
struct Tally {
    scripts: usize,
    compared: usize,
    skipped: usize,
    deletes_compared: usize,
    steps_after_delete: usize,
    /// `prev`s compared from off the start after a write.
    prevs_again: usize,
    finals: usize,
    go_panics: usize,
}

/// What the conformance rule compares in one script.
#[derive(Debug, PartialEq, Eq)]
struct Rule {
    /// Per op, whether it is compared.
    ops: Vec<bool>,
    /// Whether the final contents of `c` are compared.
    final_c: bool,
    /// The `prev`s compared only because the cursor was off the start
    /// (the table written or deleted from since the last positioning).
    prevs_again: usize,
}

/// The ops of `s` the conformance rule compares, and whether the final
/// contents of `c` are compared. `ours` is what the store returned: where
/// both sides agree so far, it tells where a `next` ran off the end and
/// where a `prev` ran off the start.
fn comparable(s: &Script, ours: &[Value]) -> Rule {
    if s.exact || !s.writable {
        return Rule {
            ops: vec![true; s.ops.len()],
            final_c: true,
            prevs_again: 0,
        };
    }
    let mut dirty = false; // c written since the last positioning
    let mut synced = true; // both cursors stand on the same entry
    let mut written = false; // c written in the transaction
    let mut deleted = false; // c deleted from in the transaction
    let mut same = true; // both sides' contents of c agree
    let mut positioned = false; // the cursor was positioned since it was made
    let mut off_start = false; // the last move was a compared `prev` off the start
    let mut off_clean = false; // and nothing was deleted from c when it ran off
    let mut prevs_again = 0;
    let mut out = Vec::with_capacity(s.ops.len());
    for (i, op) in s.ops.iter().enumerate() {
        let cmp = match op {
            Op::First | Op::Last | Op::Seek(_) | Op::Fresh => {
                dirty = false;
                synced = true;
                positioned = *op != Op::Fresh;
                off_start = false;
                same
            }
            Op::Next | Op::Prev => {
                let plain = !dirty && !(deleted && *op == Op::Prev);
                // bbolt's cursor off the start stands at index 0 of every
                // level, so its `Prev` re-runs `first` whatever was written.
                let again = *op == Op::Prev && off_start && off_clean;
                let ok = same && synced && (plain || again);
                if ok && !plain {
                    prevs_again += 1;
                }
                let null = ours.get(i).is_some_and(|r| r["k"].is_null());
                // A `next` off the end in a transaction that deleted from
                // `c`: bbolt's cursor may stand on an emptied last leaf
                // rather than on the entry it ran off from (QK-0749).
                let off_end = *op == Op::Next && null;
                synced = ok && !(deleted && off_end);
                off_start = ok && positioned && *op == Op::Prev && null;
                if off_start {
                    // bbolt's `Prev` called `first`: a fresh descent, as
                    // `First`.
                    dirty = false;
                    off_clean = !deleted;
                }
                ok
            }
            Op::Delete => {
                let ok = same && synced && !dirty;
                same = ok;
                dirty = true;
                written = true;
                deleted = true;
                ok
            }
            Op::Put(..) => {
                dirty = true;
                written = true;
                true
            }
            Op::Del(_) => {
                dirty = true;
                written = true;
                deleted = true;
                true
            }
            Op::Get(_) => same,
            Op::Range { rev, .. } => same && !(deleted && *rev),
            Op::Count => same && !written,
            Op::OPut(..) | Op::ODel(_) => true,
        };
        out.push(cmp);
    }
    Rule {
        ops: out,
        final_c: same,
        prevs_again,
    }
}

fn compare(label: &str, s: &Script, go: &Outcome, ours: &Outcome, tally: &mut Tally) {
    let Rule {
        ops: cmp,
        final_c,
        prevs_again,
    } = comparable(s, &ours.results);
    tally.prevs_again += prevs_again;
    // bbolt can panic on a cursor Delete outside the compared ops (after
    // Last on a bucket emptied in the transaction and a later Put it
    // indexes -1, QK-0748); the oracle stops the script there and rolls it
    // back, returning no final contents. The panic may be the script's last
    // op, so that every op has a result: the script panicked all the same.
    let ran = go.results.len();
    let last = go.results.last();
    let panicked = last.is_some_and(|r| r.get("panic").is_some());
    if panicked || ran < s.ops.len() {
        assert!(
            panicked && !cmp[ran - 1] && s.ops[ran - 1] == Op::Delete,
            "{label} {}: the oracle ran {ran} of {} ops, the last {last:?}\nscript: {}",
            s.name,
            s.ops.len(),
            s.to_json()
        );
        tally.go_panics += 1;
    }
    let mut after_delete = false;
    for (i, op) in s.ops.iter().enumerate().take(ran) {
        if *op == Op::Delete {
            after_delete = true;
        }
        if !cmp[i] || go.results[i].get("panic").is_some() {
            tally.skipped += 1;
            continue;
        }
        tally.compared += 1;
        if *op == Op::Delete {
            tally.deletes_compared += 1;
        }
        if after_delete && matches!(op, Op::Next | Op::Prev) {
            tally.steps_after_delete += 1;
        }
        assert_eq!(
            ours.results[i],
            go.results[i],
            "{label} {}: op {i} ({op:?}) differs from bbolt\nscript: {}",
            s.name,
            s.to_json()
        );
    }
    if s.writable && !panicked {
        let g = go.final_.as_ref().unwrap_or_else(|| {
            panic!(
                "{label} {}: no final contents from the oracle; its last result {:?}\nscript: {}",
                s.name,
                go.results.last(),
                s.to_json()
            )
        });
        let mut o = ours.final_.clone().expect("final contents");
        // The frozen vectors hold digests of the final contents.
        if g["c"].is_string() {
            o = digests(&o);
        }
        assert_eq!(o["o"], g["o"], "{label} {}: final contents of o", s.name);
        if final_c {
            tally.finals += 1;
            assert_eq!(o["c"], g["c"], "{label} {}: final contents of c", s.name);
        }
    }
    tally.scripts += 1;
}

/// The SHA-256 of a table's contents as the oracle lists them, one
/// `<key hex> <value hex>` line each.
fn contents_sha256(kv: &Value) -> String {
    let mut text = String::new();
    for e in kv.as_array().expect("contents") {
        writeln!(
            text,
            "{} {}",
            e[0].as_str().expect("key"),
            e[1].as_str().expect("value")
        )
        .expect("write to String");
    }
    body_sha256(&text)
}

/// Final contents with each table replaced by its digest.
fn digests(f: &Value) -> Value {
    json!({"c": contents_sha256(&f["c"]), "o": contents_sha256(&f["o"])})
}

fn check_script(s: &Script, go: &Value, tally: &mut Tally) {
    let go = go_outcome(go);
    for (label, ours) in runs(s) {
        compare(label, s, &go, &ours, tally);
    }
}

// ---------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------

/// Key shapes: each a family of keys dcrwallet stores or that stress the
/// order (prefixes, 0x00 and 0xff bytes).
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// 1 to 4 bytes over {00, 01, 7f, 80, fe, ff}.
    Alphabet,
    /// Account numbers, 4 bytes little-endian (the `accounts` order).
    Accounts,
    /// Outpoints: a 32-byte hash from a small pool (some sharing long
    /// prefixes) and a 4-byte big-endian index (the `unspent` order).
    Outpoints,
    /// Block heights, 4 bytes big-endian (dcrwallet's block records).
    Heights,
    /// 1 to 12 random bytes.
    Wide,
}

struct KeyGen {
    shape: Shape,
    hashes: Vec<Vec<u8>>,
}

const ALPHABET: [u8; 6] = [0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff];
const ACCOUNTS: [u32; 10] = [
    0,
    1,
    2,
    255,
    256,
    65_536,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_fffe,
    0xffff_ffff,
];

impl KeyGen {
    fn new(r: &mut SplitMix64, shape: Shape) -> KeyGen {
        let mut hashes: Vec<Vec<u8>> = Vec::new();
        for _ in 0..6 {
            let h = match hashes.last() {
                // A neighbour of the previous hash: same first 31 bytes.
                Some(prev) if r.below(3) == 0 => {
                    let mut h = prev.clone();
                    h[31] ^= 1 << r.below(8);
                    h
                }
                _ => r.bytes(32),
            };
            hashes.push(h);
        }
        KeyGen { shape, hashes }
    }

    fn key(&self, r: &mut SplitMix64) -> Vec<u8> {
        match self.shape {
            Shape::Alphabet => (0..1 + r.below(4))
                .map(|_| ALPHABET[r.below(6) as usize])
                .collect(),
            Shape::Accounts => {
                let n = if r.below(3) == 0 {
                    r.next_u64() as u32
                } else {
                    ACCOUNTS[r.below(10) as usize]
                };
                n.to_le_bytes().to_vec()
            }
            Shape::Outpoints => {
                let mut k = self.hashes[r.below(6) as usize].clone();
                let i: u32 = match r.below(4) {
                    0 => r.next_u64() as u32,
                    _ => r.below(300) as u32,
                };
                k.extend_from_slice(&i.to_be_bytes());
                k
            }
            Shape::Heights => (r.below(2000) as u32).to_be_bytes().to_vec(),
            Shape::Wide => {
                let n = 1 + r.below(12) as usize;
                r.bytes(n)
            }
        }
    }

    /// A key to seek or look up: one of `keys`, a neighbour of one, a
    /// prefix, the empty key, all-0xff, or a random key of the shape.
    fn probe(&self, r: &mut SplitMix64, keys: &[Vec<u8>]) -> Vec<u8> {
        let pick = |r: &mut SplitMix64| keys[r.below(keys.len() as u64) as usize].clone();
        match r.below(10) {
            0..=2 if !keys.is_empty() => pick(r),
            3 if !keys.is_empty() => {
                let mut k = pick(r);
                k.push(0);
                k
            }
            4 if !keys.is_empty() => {
                let mut k = pick(r);
                let last = k.len() - 1;
                k[last] = k[last].wrapping_sub(1);
                k
            }
            5 if !keys.is_empty() => {
                let k = pick(r);
                let n = r.below(k.len() as u64) as usize;
                k[..n].to_vec()
            }
            6 => match self.shape {
                // dcrwallet's random UTXO seek: 32 random bytes.
                Shape::Outpoints if r.below(2) == 0 => r.bytes(32),
                Shape::Outpoints => self.hashes[r.below(6) as usize].clone(),
                _ => Vec::new(),
            },
            7 => vec![0xff; 1 + r.below(40) as usize],
            _ => self.key(r),
        }
    }
}

fn value(r: &mut SplitMix64, max: u64) -> Vec<u8> {
    let n = r.below(max + 1) as usize;
    r.bytes(n)
}

fn key_set(r: &mut SplitMix64, g: &KeyGen, n: usize, vmax: u64) -> Vec<RawEntry> {
    let mut m: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for _ in 0..n {
        let k = g.key(r);
        let v = value(r, vmax);
        m.insert(k, v);
    }
    m.into_iter().collect()
}

/// The longest value for a set of `n` keys. bbolt's leaf element has a
/// 16-byte header, so a few hundred short entries already span several
/// 4 KiB leaves; the largest sets (live runs only) get longer values for
/// three-level trees. Short values keep the frozen vectors small.
fn vmax_for(n: usize) -> u64 {
    match n {
        0..=100 => 6,
        101..=1000 => 4,
        _ => 24,
    }
}

fn shape_of(r: &mut SplitMix64) -> Shape {
    match r.below(5) {
        0 => Shape::Alphabet,
        1 => Shape::Accounts,
        2 => Shape::Outpoints,
        3 => Shape::Heights,
        _ => Shape::Wide,
    }
}

/// A random script over `n` keys: read-only or writing, its ops drawn
/// with the weights below.
fn random_script(r: &mut SplitMix64, name: String, n: usize, writable: bool) -> Script {
    let shape = shape_of(r);
    let g = KeyGen::new(r, shape);
    let vmax = vmax_for(n);
    let entries = key_set(r, &g, n, vmax);
    let n_other = r.below(4) as usize;
    let other = key_set(r, &g, n_other, 4);
    let mut keys: Vec<Vec<u8>> = entries.iter().map(|e| e.0.clone()).collect();
    let mut positioned = false;
    let mut ops = Vec::new();
    for _ in 0..20 + r.below(60) {
        let w = if writable { r.below(30) } else { r.below(22) };
        let op = match w {
            0 | 1 => Op::First,
            2 | 3 => Op::Last,
            4..=8 => Op::Next,
            9..=13 => Op::Prev,
            14..=16 => Op::Seek(g.probe(r, &keys)),
            17 => Op::Get(g.probe(r, &keys)),
            18 if n <= 100 || r.below(3) == 0 => Op::Range {
                lo: g.probe(r, &keys),
                hi: (r.below(3) != 0).then(|| g.probe(r, &keys)),
                rev: r.below(2) == 0,
            },
            18 => Op::Get(g.probe(r, &keys)),
            19 => Op::Count,
            20 | 21 => {
                if r.below(3) == 0 {
                    Op::Fresh
                } else {
                    Op::Next
                }
            }
            22..=24 if positioned => Op::Delete,
            22 | 25 | 26 => {
                let k = match r.below(20) {
                    0 => Vec::new(), // refused: key required
                    1..=8 if !keys.is_empty() => keys[r.below(keys.len() as u64) as usize].clone(),
                    _ => g.key(r),
                };
                if !k.is_empty() {
                    keys.push(k.clone());
                }
                Op::Put(k, value(r, vmax))
            }
            23 | 24 | 27 => Op::Del(g.probe(r, &keys)),
            28 => Op::OPut(g.key(r), value(r, 4)),
            _ => Op::ODel(g.probe(r, &keys)),
        };
        match op {
            Op::First | Op::Last | Op::Seek(_) => positioned = true,
            Op::Fresh => positioned = false,
            _ => {}
        }
        ops.push(op);
    }
    Script {
        name,
        entries,
        other,
        writable,
        exact: false,
        ops,
    }
}

/// A walk in one direction that deletes some of the entries it stands on
/// (and writes `o` between steps), in a transaction that has not written
/// `c` before: bbolt's cursor then reads each leaf's committed copy, so
/// what it returns is fixed by the keys alone and equals the store's
/// (module documentation of `purse_store::cursor`). The walk is planned
/// on the key set and stops where it runs off the end.
fn delete_walk(r: &mut SplitMix64, name: String, n: usize) -> Script {
    let shape = shape_of(r);
    let g = KeyGen::new(r, shape);
    let vmax = vmax_for(n);
    let entries = key_set(r, &g, n, vmax);
    let mut live: BTreeSet<Vec<u8>> = entries.iter().map(|e| e.0.clone()).collect();
    let keys: Vec<Vec<u8>> = live.iter().cloned().collect();
    let mut ops = Vec::new();
    let forward = r.below(3) != 0;
    let start = if !forward {
        ops.push(Op::Last);
        live.last().cloned()
    } else if r.below(2) == 0 {
        ops.push(Op::First);
        live.first().cloned()
    } else {
        let k = g.probe(r, &keys);
        let at = live.range(k.clone()..).next().cloned();
        ops.push(Op::Seek(k));
        at
    };
    let p_delete = r.below(4) + 1; // of 5
    let mut at = start;
    while let Some(k) = at {
        if r.below(5) < p_delete {
            ops.push(Op::Delete);
            live.remove(&k);
        }
        if r.below(6) == 0 {
            ops.push(Op::OPut(g.key(r), value(r, 4)));
        }
        if r.below(10) == 0 {
            ops.push(Op::Get(g.probe(r, &keys)));
        }
        if forward {
            ops.push(Op::Next);
            at = live
                .range((Bound::Excluded(k), Bound::Unbounded))
                .next()
                .cloned();
        } else {
            ops.push(Op::Prev);
            at = live.range(..k).next_back().cloned();
        }
    }
    // Off the end, the cursor still stands on the entry it ran off from.
    if r.below(2) == 0 {
        ops.push(Op::Delete);
    }
    Script {
        name,
        entries,
        other: Vec::new(),
        writable: true,
        exact: true,
        ops,
    }
}

/// A script that keeps running off the ends of the table and writes
/// around them: `prev` and `next` past the first and last entry, puts of
/// keys that sort below the first or above the last, deletions of the
/// entries at the ends, and cursor deletes. The conformance rule decides
/// what is compared, `prev` off the start being a positioning (the
/// documentation of `comparable`).
fn edge_script(r: &mut SplitMix64, name: String) -> Script {
    let n = [0usize, 1, 2, 3, 5, 12, 40, 150, 400][r.below(9) as usize];
    let shape = shape_of(r);
    let g = KeyGen::new(r, shape);
    let vmax = vmax_for(n);
    let entries = key_set(r, &g, n, vmax);
    let all: Vec<Vec<u8>> = entries.iter().map(|e| e.0.clone()).collect();
    // The table's keys as the writes leave them (cursor deletes aside).
    let mut live: BTreeSet<Vec<u8>> = all.iter().cloned().collect();
    let mut ops = vec![match r.below(3) {
        0 => Op::First,
        1 => Op::Last,
        _ => Op::Seek(g.probe(r, &all)),
    }];
    // A key just below the first entry (a prefix of it, or it with its
    // last byte lowered), or above the last (it extended).
    let below = |r: &mut SplitMix64, k: &[u8]| -> Vec<u8> {
        if k.len() > 1 && r.below(2) == 0 {
            k[..k.len() - 1].to_vec()
        } else {
            let mut k = k.to_vec();
            let last = k.len() - 1;
            k[last] = k[last].saturating_sub(1);
            k
        }
    };
    for _ in 0..10 + r.below(40) {
        let first = live.first().cloned();
        let last = live.last().cloned();
        // Deletions are rare: after one, a `prev` is compared only in the
        // cases the rule allows (QK-0747).
        let op = match r.below(20) {
            0..=5 => Op::Prev,
            6..=8 => Op::Next,
            9 | 10 => match &first {
                Some(k) => Op::Put(below(r, k), value(r, vmax)),
                None => Op::Put(g.key(r), value(r, vmax)),
            },
            11 => match &last {
                Some(k) => {
                    let mut k = k.clone();
                    k.push(r.below(256) as u8);
                    Op::Put(k, value(r, vmax))
                }
                None => Op::Put(g.key(r), value(r, vmax)),
            },
            12 => match &first {
                Some(k) if r.below(3) == 0 => Op::Del(k.clone()),
                _ => Op::Prev,
            },
            13 => match &last {
                Some(k) if r.below(3) == 0 => Op::Del(k.clone()),
                _ => Op::Next,
            },
            14 if r.below(3) == 0 => Op::Delete,
            14 | 15 => Op::First,
            16 => Op::Last,
            17 => Op::Seek(g.probe(r, &all)),
            _ => Op::Put(g.key(r), value(r, vmax)),
        };
        match &op {
            Op::Put(k, _) if !k.is_empty() && k.len() <= purse_store::MAX_KEY_SIZE => {
                live.insert(k.clone());
            }
            Op::Del(k) => {
                live.remove(k);
            }
            _ => {}
        }
        ops.push(op);
    }
    Script {
        name,
        entries,
        other: Vec::new(),
        writable: true,
        exact: false,
        ops,
    }
}

/// dcrwallet's `randomUTXO` (wallet/udb/txmined.go:2356-2420, spec 04
/// §5.19), its cursor ops recorded while the algorithm runs on the
/// store's read cursor: seek a random 32-byte key, walk one way while the
/// transaction hash stays the same, re-seek the chosen key, and walk the
/// other way when the first found nothing. The comparison against bbolt
/// then holds every recorded op.
fn random_utxo(r: &mut SplitMix64, name: String, n: usize) -> Script {
    let g = KeyGen::new(r, Shape::Outpoints);
    let entries = key_set(r, &g, n, 6);
    let probe = Script {
        name: name.clone(),
        entries: entries.clone(),
        other: Vec::new(),
        writable: false,
        exact: true,
        ops: Vec::new(),
    };
    let store = new_store(&probe);
    let tx = store.read().expect("read");
    let mut c = tx.cursor(c_table()).expect("cursor");
    let mut ops = Vec::new();
    for _ in 0..4 {
        let rand_key = if r.below(3) == 0 {
            g.hashes[r.below(6) as usize].clone()
        } else {
            r.bytes(32)
        };
        let prev_first = r.below(2) == 0;
        let mut keys = utxo_pass(&rand_key, prev_first, &mut ops, &mut c);
        if keys.is_empty() {
            keys = utxo_pass(&rand_key, !prev_first, &mut ops, &mut c);
        }
        if !keys.is_empty() {
            let k = keys[r.below(keys.len() as u64) as usize].clone();
            ops.push(Op::Seek(k.clone()));
            c.seek(&k).expect("seek");
        }
    }
    Script { ops, ..probe }
}

/// One pass of `randomUTXO`: seek `rand_key`, step back once when walking
/// backward, then collect the keys sharing the first one's transaction
/// hash (a skip function drops some), recording every op.
fn utxo_pass(
    rand_key: &[u8],
    back: bool,
    ops: &mut Vec<Op>,
    c: &mut purse_store::ReadCursor<'_>,
) -> Vec<Vec<u8>> {
    let skip = |k: &[u8]| k.last().is_some_and(|b| b % 3 == 0);
    let mut keys: Vec<Vec<u8>> = Vec::new();
    ops.push(Op::Seek(rand_key.to_vec()));
    let mut e = c.seek(rand_key).expect("seek");
    if back {
        ops.push(Op::Prev);
        e = c.prev().expect("prev");
    }
    while let Some((k, _)) = e {
        if keys.first().is_some_and(|f| f[..32] != k[..32]) {
            break;
        }
        if !skip(&k) {
            keys.push(k);
        }
        ops.push(if back { Op::Prev } else { Op::Next });
        e = if back { c.prev() } else { c.next() }.expect("step");
    }
    keys
}

/// dcrwallet's `Rollback` walk (wallet/udb/txmined.go:1903-2205): a
/// reverse block iterator over height keys (`makeReverseBlockIterator`:
/// seek ff ff ff ff, then `Prev` since nothing was found), writes to other
/// buckets for each block, a reposition (`Seek` of the current height)
/// before each `Prev`, and the block records deleted after the walk.
fn rollback_walk(r: &mut SplitMix64, name: String, n: usize) -> Script {
    let mut heights: Vec<u32> = (0..n).map(|_| r.below(500) as u32).collect();
    heights.sort_unstable();
    heights.dedup();
    let entries: Vec<RawEntry> = heights
        .iter()
        .map(|h| (h.to_be_bytes().to_vec(), value(r, 8)))
        .collect();
    let cut = heights.get(heights.len() / 2).copied().unwrap_or(0);
    let mut ops = vec![Op::Seek(vec![0xff; 4]), Op::Prev];
    let mut removed = Vec::new();
    for h in heights.iter().rev() {
        if *h < cut {
            break;
        }
        removed.push(*h);
        for _ in 0..r.below(3) {
            ops.push(Op::OPut(r.bytes(36), value(r, 8)));
        }
        if r.below(2) == 0 {
            ops.push(Op::ODel(r.bytes(36)));
        }
        ops.push(Op::Seek(h.to_be_bytes().to_vec()));
        ops.push(Op::Prev);
    }
    for h in removed {
        ops.push(Op::Del(h.to_be_bytes().to_vec()));
    }
    ops.push(Op::Last);
    Script {
        name,
        entries,
        other: Vec::new(),
        writable: true,
        exact: true,
        ops,
    }
}

/// dcrwallet's upgrade 17 (`accountVariablesUpgrade`,
/// wallet/udb/upgrades.go:1388-1410, QK-0119): a forward walk over the
/// account keys that puts a new row at each key it stands on.
fn put_current_walk(r: &mut SplitMix64, name: String) -> Script {
    let g = KeyGen::new(r, Shape::Accounts);
    let entries = key_set(r, &g, 12, 6);
    let mut ops = vec![Op::First];
    for (k, _) in &entries {
        ops.push(Op::Put(k.clone(), value(r, 6)));
        ops.push(Op::Next);
    }
    Script {
        name,
        entries,
        other: Vec::new(),
        writable: true,
        exact: true,
        ops,
    }
}

/// dcrwallet's upgrade pattern of collecting what a cursor walk finds and
/// writing after the walk (`hasExpiryUpgrade`, wallet/udb/upgrades.go:
/// 643-676; `upgradeToVersion2`'s block records, txdb.go:2379-2401).
fn collect_then_put(r: &mut SplitMix64, name: String) -> Script {
    let g = KeyGen::new(r, Shape::Outpoints);
    let entries = key_set(r, &g, 30, 6);
    let mut ops = vec![Op::First];
    ops.extend(entries.iter().map(|_| Op::Next));
    for (k, _) in &entries {
        if r.below(2) == 0 {
            ops.push(Op::Put(k.clone(), value(r, 8)));
        }
    }
    ops.push(Op::Fresh);
    ops.push(Op::First);
    ops.extend(entries.iter().map(|_| Op::Next));
    Script {
        name,
        entries,
        other: Vec::new(),
        writable: true,
        exact: true,
        ops,
    }
}

fn le(n: u32) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

fn kv(keys: &[Vec<u8>]) -> Vec<RawEntry> {
    let mut v: Vec<RawEntry> = keys
        .iter()
        .map(|k| {
            let mut val = b"v".to_vec();
            val.extend_from_slice(k);
            (k.clone(), val)
        })
        .collect();
    v.sort();
    v
}

/// 400 two-byte big-endian keys with one-byte values: about 7.6 KiB of
/// leaf elements, which bbolt spreads over three 4 KiB leaves (keys 0 to
/// 105, 106 to 211, 212 to 399), and which fit one leaf of 16 KiB.
fn wide() -> Vec<RawEntry> {
    wide_n(400)
}

/// `n` two-byte big-endian keys with one-byte values, the keys 0 to n - 1.
fn wide_n(n: u16) -> Vec<RawEntry> {
    (0..n)
        .map(|i| (i.to_be_bytes().to_vec(), vec![i as u8]))
        .collect()
}

/// How many times larger than 4096 bytes `page_size` is: the factor by
/// which the multi-leaf scripts scale their keys so that each leaf
/// boundary falls near where the 4 KiB layout puts it. bbolt cuts a leaf
/// larger than a page into pieces of half a page (`DefaultFillPercent`),
/// 19 bytes an element here: 106 keys a piece at 4 KiB, 430 at 16 KiB
/// (4 x 106 = 424), 1723 at 64 KiB (16 x 106 = 1696), the last piece
/// taking the rest once it fits a page. The tests check the leaf count the
/// oracle reports rather than rely on this arithmetic.
fn scale_of(page_size: u32) -> u16 {
    assert!(
        page_size >= 4096 && page_size.is_power_of_two(),
        "{page_size}"
    );
    u16::try_from(page_size / 4096).expect("page size up to 256 MiB")
}

/// The scripts of [`divergence_cases`] that need `c` to span several
/// leaves (QK-0747, QK-0749), with their keys and deletions multiplied by
/// `f`: at a page size of `f` x 4 KiB their buckets of 400 x `f` keys span
/// three leaves, and they differ from the store where the 4 KiB scripts do.
/// `f` = 1 gives the scripts of [`divergence_cases`] themselves; a larger
/// `f` adds `_x<f>` to each name.
fn multileaf_divergences(f: u16) -> Vec<(Script, Vec<usize>)> {
    use Op::*;
    let k = |i: u16| i.to_be_bytes().to_vec();
    let name = |base: &str| {
        if f == 1 {
            base.to_owned()
        } else {
            format!("{base}_x{f}")
        }
    };
    let wide = wide_n(400 * f);
    // Keys 100 to 299 deleted (x f): the middle leaf is empty, and the
    // last leaf's keys below 300 are gone.
    let mut empty_a_leaf: Vec<Op> = (100 * f..300 * f).map(|i| Del(k(i))).collect();
    let at = empty_a_leaf.len();
    empty_a_leaf.extend([Seek(k(300 * f)), Prev, Prev]);
    // Every key from 200 (x f) on deleted: the leaves after the one holding
    // the last key left, 200 x f - 1, are empty in-memory nodes.
    let empty_the_tail = || -> Vec<Op> { (200 * f..400 * f).map(|i| Del(k(i))).collect() };
    let tail = empty_the_tail().len();
    let last_left = k(200 * f - 1);
    let mut prev_after_next = empty_the_tail();
    prev_after_next.extend([Seek(last_left.clone()), Next, Prev, Prev]);
    let mut delete_after_next = empty_the_tail();
    delete_after_next.extend([Seek(last_left.clone()), Next, Delete, Get(last_left)]);
    vec![
        // QK-0747: bbolt's Prev that steps into a leaf emptied earlier in
        // the transaction returns nil; the store returns the key below.
        (
            named(
                &name("div_qk0747_prev_into_an_emptied_leaf_is_nil"),
                wide.clone(),
                true,
                empty_a_leaf,
            ),
            vec![at + 1, at + 2],
        ),
        // QK-0749: after a Next that ran off the end over leaves emptied in
        // the transaction, bbolt's cursor stands on the last, empty leaf:
        // Prev returns the entry it ran off from again (then the one below
        // it), where the store returns the key below (then the next).
        (
            named(
                &name("div_qk0749_prev_after_next_over_emptied_leaves_repeats"),
                wide.clone(),
                true,
                prev_after_next,
            ),
            vec![tail + 2, tail + 3],
        ),
        // QK-0749: the same position, and bbolt's cursor Delete removes
        // nothing (it deletes the empty leaf's nil key), where the store's
        // removes the entry the cursor ran off from; the Get after it shows
        // the difference.
        (
            named(
                &name("div_qk0749_delete_after_next_over_emptied_leaves_removes_nothing"),
                wide,
                true,
                delete_after_next,
            ),
            vec![tail + 3],
        ),
    ]
}

fn named(name: &str, entries: Vec<RawEntry>, writable: bool, ops: Vec<Op>) -> Script {
    Script {
        name: name.to_owned(),
        entries,
        other: Vec::new(),
        writable,
        exact: true,
        ops,
    }
}

/// The cases of spec 04 §5.1 and spec 01 §5.7's cursor edge cases, by
/// name. Each is compared op by op with bbolt. Items 7 and 8 of §5.1 have
/// no counterpart to compare: the store's cursors have no `Close`, and
/// every entry they return is an owned copy, never memory of the engine.
fn named_cases() -> Vec<Script> {
    use Op::*;
    let accounts: Vec<Vec<u8>> = ACCOUNTS.iter().map(|n| le(*n)).collect();
    let hash = |b: u8| {
        let mut h = vec![0x11; 32];
        h[0] = b;
        h
    };
    let outpoint = |h: &[u8], i: u32| {
        let mut k = h.to_vec();
        k.extend_from_slice(&i.to_be_bytes());
        k
    };
    let mut outpoints = Vec::new();
    for b in [0x00, 0x7f, 0x80, 0xff] {
        for i in [0, 1, 255, 256, 65_536, 0x0100_0000] {
            outpoints.push(outpoint(&hash(b), i));
        }
    }
    let bdf: Vec<Vec<u8>> = vec![b"b".to_vec(), b"d".to_vec(), b"f".to_vec()];
    let walk_all = |n: usize, first: Op, step: Op| {
        let mut v = vec![first];
        v.extend(std::iter::repeat_n(step, n));
        v
    };
    // A bucket bbolt spreads over several leaves: 400 two-byte keys with
    // one-byte values, 19 bytes an element with its header.
    let wide = wide();
    vec![
        // Item 1: ascending byte order; accounts in little-endian byte order.
        named(
            "s5_1_1_accounts_iterate_in_little_endian_byte_order",
            kv(&accounts),
            false,
            [
                walk_all(accounts.len(), First, Next),
                walk_all(accounts.len(), Last, Prev),
            ]
            .concat(),
        ),
        // Item 1: outpoints by raw hash, then big-endian index.
        named(
            "s5_1_1_outpoints_iterate_by_raw_hash_then_big_endian_index",
            kv(&outpoints),
            false,
            walk_all(outpoints.len(), First, Next),
        ),
        // Item 2: Seek is the first key >= k, nil past the end.
        named(
            "s5_1_2_seek_is_the_first_key_at_or_above",
            kv(&bdf),
            false,
            vec![
                Seek(b"c".to_vec()),
                Seek(b"d".to_vec()),
                Seek(b"".to_vec()),
                Seek(b"a".to_vec()),
                Seek(b"f".to_vec()),
                Seek(b"f\x00".to_vec()),
                Seek(b"g".to_vec()),
                Seek(hash(0x7f)),
            ],
        ),
        // Item 2 on outpoints: a 32-byte hash seeks to its first output.
        named(
            "s5_1_2_seek_a_hash_finds_its_first_output",
            kv(&outpoints),
            false,
            vec![
                Seek(hash(0x7f)),
                Seek(hash(0x80)[..31].to_vec()),
                Seek(outpoint(&hash(0x7f), 2)),
                Seek(vec![0xff; 32]),
                Seek(vec![0xff; 36]),
            ],
        ),
        // Item 3: Prev right after Seek is the greatest key below the one
        // Seek returned.
        named(
            "s5_1_3_prev_after_seek_is_the_key_below",
            kv(&bdf),
            false,
            vec![
                Seek(b"c".to_vec()),
                Prev,
                Seek(b"d".to_vec()),
                Prev,
                Seek(b"a".to_vec()),
                Prev,
                Next,
            ],
        ),
        // Item 3: previous of end is last.
        named(
            "s5_1_3_previous_of_end_is_last",
            kv(&bdf),
            false,
            vec![
                Seek(b"g".to_vec()),
                Prev,
                Prev,
                Seek(b"zz".to_vec()),
                Next,
                Prev,
            ],
        ),
        // Item 3 on a bucket spanning leaves: previous of end is last.
        named(
            "s5_1_3_previous_of_end_is_last_across_leaves",
            wide.clone(),
            false,
            vec![
                Seek(vec![0xff, 0xff]),
                Prev,
                Prev,
                Seek(vec![0x00, 0xc8]),
                Prev,
                Seek(vec![0x00, 0xc7, 0x00]),
                Prev,
            ],
        ),
        // Item 3: Prev from the first key is nil, and the cursor stays on
        // the first (bbolt#733).
        named(
            "s5_1_3_prev_from_the_first_is_nil",
            kv(&bdf),
            false,
            vec![First, Prev, Prev, Next, Next, Next, Next],
        ),
        // Item 4: Next from the last key is nil, and the cursor stays on
        // the last.
        named(
            "s5_1_4_next_from_the_last_is_nil",
            kv(&bdf),
            false,
            vec![Last, Next, Next, Prev, Prev, Prev, Prev],
        ),
        // Item 5: Get of the empty key is nil; a Put of it is refused.
        named(
            "s5_1_5_get_of_the_empty_key_is_nil",
            kv(&bdf),
            true,
            vec![
                Get(Vec::new()),
                Put(Vec::new(), b"x".to_vec()),
                Get(Vec::new()),
                Get(b"d".to_vec()),
                Get(b"e".to_vec()),
                Del(Vec::new()),
            ],
        ),
        // Item 6: KeyN in a read transaction is the number of keys.
        named(
            "s5_1_6_keyn_in_a_read_transaction",
            wide.clone(),
            false,
            vec![Count],
        ),
        named(
            "s5_1_6_keyn_of_an_empty_bucket",
            Vec::new(),
            false,
            vec![Count],
        ),
        // Spec 01 §5.7: moves on a cursor never positioned.
        named(
            "s01_next_and_prev_unpositioned_are_nil",
            kv(&bdf),
            false,
            vec![Next, Prev, Next, Fresh, Prev, Next],
        ),
        // Spec 01 §5.7: an empty bucket.
        named(
            "s01_moves_on_an_empty_bucket",
            Vec::new(),
            false,
            vec![
                Last,
                Next,
                Prev,
                First,
                Prev,
                Next,
                Seek(b"a".to_vec()),
                Prev,
                Seek(Vec::new()),
            ],
        ),
        named(
            "s01_seek_of_the_empty_key_is_the_first",
            kv(&bdf),
            false,
            vec![Seek(Vec::new()), Next, Seek(Vec::new()), Prev],
        ),
        // Spec 01 §5.7: Delete after a Seek past the end removes nothing.
        named(
            "s01_delete_after_seek_past_the_end_removes_nothing",
            kv(&bdf),
            true,
            vec![Seek(b"z".to_vec()), Delete, Prev],
        ),
        // bbolt stays on the last element after Next ran off the end:
        // Delete removes it.
        named(
            "s01_delete_after_next_ran_off_the_end_removes_the_last",
            kv(&bdf),
            true,
            vec![Last, Next, Delete],
        ),
        // bbolt repositions on the first element after Prev ran off the
        // start: Delete removes it.
        named(
            "s01_delete_after_prev_ran_off_the_start_removes_the_first",
            kv(&bdf),
            true,
            vec![First, Prev, Delete],
        ),
        // Delete on an empty bucket after First and Last.
        named(
            "s01_delete_on_an_empty_bucket",
            Vec::new(),
            true,
            vec![First, Delete, Last, Delete],
        ),
        // Every Prev that runs off the start repositions bbolt's cursor on
        // the first entry as the bucket then is (cursor.go:248-277 calls
        // first(), a fresh descent): a second Prev off the start after the
        // first entry was deleted stands on the new first, so Next returns
        // the entry after it.
        named(
            "w_second_prev_off_the_start_after_the_first_is_deleted",
            kv(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]),
            true,
            vec![First, Prev, Del(b"a".to_vec()), Prev, Next],
        ),
        // The same after a Put below the first entry: the second Prev
        // stands on the new first, and Next returns the old first.
        named(
            "w_second_prev_off_the_start_after_a_smaller_put",
            kv(&[b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]),
            true,
            vec![First, Prev, Put(b"a".to_vec(), b"v".to_vec()), Prev, Next],
        ),
        // A cursor Delete after the second Prev removes the new first.
        named(
            "w_delete_after_a_second_prev_off_the_start",
            kv(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]),
            true,
            vec![
                First,
                Prev,
                Del(b"a".to_vec()),
                Prev,
                Delete,
                Get(b"b".to_vec()),
            ],
        ),
        // Prev off the start of an emptied bucket stands on nothing; after
        // Puts, a second Prev stands on the first of them.
        named(
            "w_prev_off_an_emptied_bucket_then_puts",
            kv(&[b"a".to_vec()]),
            true,
            vec![
                First,
                Delete,
                Prev,
                Put(b"x".to_vec(), b"1".to_vec()),
                Put(b"y".to_vec(), b"2".to_vec()),
                Prev,
                Next,
            ],
        ),
        // Prev after a Seek on an empty bucket runs off the start too: after
        // Puts, a second Prev returns nil and stands on the first.
        named(
            "w_prev_after_seek_on_an_empty_bucket_then_puts",
            Vec::new(),
            true,
            vec![
                Seek(b"m".to_vec()),
                Prev,
                Put(b"x".to_vec(), b"1".to_vec()),
                Put(b"y".to_vec(), b"2".to_vec()),
                Prev,
                Next,
            ],
        ),
        // After Next ran off the end the cursor stays on the last entry; on
        // a leaf already written in the transaction (a live node), a Put
        // above it is the next Next's answer.
        named(
            "w_next_after_next_off_the_end_finds_a_put_on_a_written_leaf",
            kv(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]),
            true,
            vec![
                Put(b"b".to_vec(), b"w".to_vec()),
                Last,
                Next,
                Put(b"z".to_vec(), b"1".to_vec()),
                Next,
            ],
        ),
        // Writes to another bucket never move the cursor.
        named(
            "writes_elsewhere_leave_the_cursor_alone",
            wide.clone(),
            true,
            vec![
                Seek(vec![0x00, 0x64]),
                OPut(vec![1], vec![2]),
                Next,
                ODel(vec![1]),
                Prev,
                Prev,
                OPut(vec![3], Vec::new()),
                Next,
            ],
        ),
        // Bounded ranges walked as dcrwallet walks them, both ways.
        named(
            "bounded_ranges",
            wide.clone(),
            false,
            vec![
                Range {
                    lo: vec![0x00, 0x10],
                    hi: Some(vec![0x00, 0x20]),
                    rev: false,
                },
                Range {
                    lo: vec![0x00, 0x10],
                    hi: Some(vec![0x00, 0x20]),
                    rev: true,
                },
                Range {
                    lo: vec![0x01],
                    hi: None,
                    rev: true,
                },
                Range {
                    lo: Vec::new(),
                    hi: Some(vec![0x00, 0x03, 0x00]),
                    rev: true,
                },
                Range {
                    lo: vec![0x00, 0x20],
                    hi: Some(vec![0x00, 0x10]),
                    rev: false,
                },
                Range {
                    lo: vec![0x00, 0x20],
                    hi: Some(vec![0x00, 0x10]),
                    rev: true,
                },
                Range {
                    lo: vec![0x00, 0x20],
                    hi: Some(vec![0x00, 0x20]),
                    rev: true,
                },
            ],
        ),
    ]
}

/// The scripts where bbolt and the store differ by design, each with the
/// op indices where they differ. Pinned by name in `divergences`.
fn divergence_cases() -> Vec<(Script, Vec<usize>)> {
    use Op::*;
    let abcd = kv(&[
        b"a".to_vec(),
        b"b".to_vec(),
        b"c".to_vec(),
        b"d".to_vec(),
        b"e".to_vec(),
    ]);
    let [qk0747, qk0749_prev, qk0749_delete]: [(Script, Vec<usize>); 3] =
        multileaf_divergences(1).try_into().expect("three scripts");
    vec![
        // bolt#620 (QK-0004): after a write to the bucket earlier in the
        // transaction the leaf is a live node, and a cursor Delete then
        // Next skips the entry after the deleted one. The store returns it.
        (
            named(
                "div_bolt620_delete_then_next_skips_on_a_written_leaf",
                abcd.clone(),
                true,
                vec![
                    Put(b"z".to_vec(), Vec::new()),
                    First,
                    Delete,
                    Next,
                    Delete,
                    Next,
                ],
            ),
            vec![3, 5],
        ),
        // bbolt's KeyN in a write transaction counts committed pages only
        // (spec 04 §5.1 item 6, QK-0003); the store counts the
        // transaction's own writes.
        (
            named(
                "div_keyn_in_a_write_transaction_ignores_its_writes",
                abcd.clone(),
                true,
                vec![
                    Count,
                    Put(b"z".to_vec(), Vec::new()),
                    Count,
                    Del(b"a".to_vec()),
                    Count,
                ],
            ),
            vec![2],
        ),
        // QK-0747 and QK-0749 (multileaf_divergences).
        qk0747,
        qk0749_prev,
        // After Next ran off the end of a leaf not yet written in the
        // transaction, bbolt's cursor reads the leaf's committed copy, so a
        // Put above the last entry is not the next Next's answer (spec 01
        // §10 Q9); the store returns it, as on a written leaf (the named
        // case `w_next_after_next_off_the_end_finds_a_put_on_a_written_leaf`).
        (
            named(
                "div_next_after_next_off_the_end_misses_a_put_on_a_committed_leaf",
                kv(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]),
                true,
                vec![Last, Next, Put(b"z".to_vec(), b"1".to_vec()), Next],
            ),
            vec![3],
        ),
        // QK-0749 (multileaf_divergences).
        qk0749_delete,
    ]
}

/// Scripts where bbolt panics on a cursor `Delete` (the oracle catches
/// the panic and stops), each with the op that panics and the contents of
/// `c` the store ends with, its `delete` removing nothing.
fn panic_cases() -> Vec<(Script, usize, Value)> {
    vec![
        // A cursor Delete never positioned indexes -1 (spec 01 §5.7).
        (
            named(
                "div_delete_unpositioned_panics_in_bbolt",
                kv(&[b"a".to_vec()]),
                true,
                vec![Op::Get(b"a".to_vec()), Op::Delete],
            ),
            1,
            json!([["61", "7661"]]),
        ),
        // QK-0748: Last on a bucket emptied in the transaction leaves the
        // index at -1 on the live node; after a Put, Delete indexes -1.
        (
            named(
                "div_qk0748_delete_after_last_on_an_emptied_bucket_panics",
                kv(&[b"a".to_vec()]),
                true,
                vec![
                    Op::Del(b"a".to_vec()),
                    Op::Last,
                    Op::Put(b"b".to_vec(), Vec::new()),
                    Op::Delete,
                ],
            ),
            3,
            json!([["62", ""]]),
        ),
    ]
}

/// The random corpus: read-only and writing scripts over key sets from
/// empty to `max_n`, delete walks, `randomUTXO` and `Rollback` walks.
fn corpus(seed_label: &str, seed: Option<u64>, scripts: usize, big: bool) -> Vec<Script> {
    let mut r = match seed {
        Some(s) => SplitMix64(s),
        None => SplitMix64::from_env(seed_label),
    };
    // The frozen corpus stays below 400 keys a set to keep the file small;
    // the named cases hold 400-key buckets that span leaves.
    let sizes: &[usize] = if big {
        &[0, 1, 2, 3, 5, 12, 40, 150, 400, 1500, 6000]
    } else {
        &[0, 1, 2, 3, 5, 12, 40, 150]
    };
    let mut out = Vec::new();
    for i in 0..scripts {
        let mut n = sizes[r.below(sizes.len() as u64) as usize];
        // The largest sets are rare: they make three-level trees.
        if n > 1000 && r.below(4) != 0 {
            n = 150;
        }
        let s = match i % 10 {
            0..=3 => random_script(&mut r, format!("read-{i}"), n, false),
            4..=6 => random_script(&mut r, format!("write-{i}"), n, true),
            7 | 8 => delete_walk(&mut r, format!("walk-{i}"), n),
            _ => {
                if r.below(2) == 0 {
                    random_utxo(&mut r, format!("utxo-{i}"), n.max(3))
                } else {
                    rollback_walk(&mut r, format!("rollback-{i}"), n.max(2))
                }
            }
        };
        out.push(s);
    }
    out.push(put_current_walk(
        &mut r,
        "upgrade17_put_current_inside_a_walk".to_owned(),
    ));
    out.push(collect_then_put(&mut r, "collect_then_put".to_owned()));
    out
}

// ---------------------------------------------------------------------
// Tests against the oracle
// ---------------------------------------------------------------------

fn check_divergence(s: &Script, want: &[usize], go: &Value) {
    let go = go_outcome(go);
    for (label, ours) in runs(s) {
        let differ: Vec<usize> = (0..s.ops.len())
            .filter(|&i| ours.results[i] != go.results[i])
            .collect();
        assert_eq!(
            differ, want,
            "{label} {}: ops where the store and bbolt differ\nGo: {:?}\nours: {:?}",
            s.name, go.results, ours.results
        );
    }
}

fn check_panic(s: &Script, at: usize, want_c: &Value, go: &Value) {
    let results = go["results"].as_array().expect("results");
    assert_eq!(
        results.len(),
        at + 1,
        "{}: bbolt stops at the panic",
        s.name
    );
    assert!(
        results[at]["panic"]
            .as_str()
            .is_some_and(|p| p == "runtime error: index out of range"),
        "{}: bbolt's Delete: {results:?}",
        s.name
    );
    assert!(go.get("final").is_none_or(Value::is_null));
    let ours = run_write(s);
    assert_eq!(
        &ours.results[..at],
        &results[..at],
        "{}: before the panic",
        s.name
    );
    assert_eq!(ours.results[at], json!({}), "{}", s.name);
    assert_eq!(&ours.final_.expect("final")["c"], want_c, "{}", s.name);
}

/// The generated corpus, every named case and the dcrwallet patterns
/// against bbolt itself.
#[test]
fn cursors_match_bbolt() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let mut tally = Tally::default();
    let named = named_cases();
    let random = corpus("cursor-conformance", None, 400, true);
    let mut os_page_size = None;
    for s in &named {
        let go = run_oracle(&mut oracle, s);
        check_script(s, &go, &mut tally);
        os_page_size = go["layout"]["os_page_size"].as_u64();
    }
    // Counted over the random corpus alone.
    let mut multileaf = 0;
    for s in &random {
        let go = run_oracle(&mut oracle, s);
        check_script(s, &go, &mut tally);
        multileaf += usize::from(leaf_pages(&go) > 1);
    }
    // The scripts that need several leaves must have had them: a page
    // size the oracle did not apply would otherwise pass or fail them for
    // the wrong reason.
    let spans: BTreeSet<String> = multileaf_divergences(1)
        .into_iter()
        .map(|(s, _)| s.name)
        .collect();
    for (s, want) in divergence_cases() {
        let go = run_oracle(&mut oracle, &s);
        if spans.contains(&s.name) {
            assert!(leaf_pages(&go) > 1, "{}: {}", s.name, go["layout"]);
        }
        check_divergence(&s, &want, &go);
    }
    for (s, at, want_c) in panic_cases() {
        let go = run_oracle(&mut oracle, &s);
        check_panic(&s, at, &want_c, &go);
    }
    eprintln!(
        "{tally:?}\n{multileaf} random scripts over several leaves at {PAGE_SIZE}-byte \
         pages (this host's page size, which dcrwallet would use: {os_page_size:?})"
    );
    assert!(
        tally.compared > MIN_COMPARED_PER_SKIPPED * tally.skipped,
        "{tally:?}"
    );
    assert!(tally.deletes_compared > 100, "{tally:?}");
    assert!(tally.steps_after_delete > 100, "{tally:?}");
    assert!(
        multileaf > MIN_MULTILEAF_SCRIPTS,
        "{multileaf} random scripts over several leaves"
    );
}

/// The named cases and a random corpus with the store's cursor on each
/// record table of Phase 2 in turn, against bbolt's answers for the same
/// scripts: every table of the schema is the same raw-bytes B-tree, and
/// this holds it for the new ones.
#[test]
fn cursors_match_bbolt_on_every_record_table() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let mut scripts = named_cases();
    scripts.extend(corpus("cursor-conformance-records", None, 60, false));
    let answers: Vec<Value> = scripts.iter().map(|s| run_oracle(&mut oracle, s)).collect();
    for t in purse_store::RECORD_TABLES {
        let mut tally = Tally::default();
        with_cursor_table(t, || {
            for (s, go) in scripts.iter().zip(&answers) {
                check_script(s, go, &mut tally);
            }
        });
        eprintln!("{}: {tally:?}", t.name());
        assert!(
            tally.compared > MIN_COMPARED_PER_SKIPPED_RECORD_TABLES * tally.skipped,
            "{}: {tally:?}",
            t.name()
        );
    }
}

/// The unspent multisig outputs of a script come back in dcrwallet's order:
/// bbolt's walk of the `mu` bucket (bare outpoints, `First` then `Next`,
/// as `UnspentMultisigCreditsForAddress` walks it,
/// `wallet/udb/txmined.go:2886-2927`) filtered by script, equal to the
/// store's prefix walk of `multisig_unspent` for each script.
#[test]
fn unspent_multisig_listing_is_bbolts_mu_walk() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let mut r = SplitMix64::from_env("purse-store multisig mu order");
    for case in 0..40 {
        let n = 1 + r.below(if case % 4 == 0 { 400 } else { 40 }) as usize;
        // Outpoints over few hashes, so that indexes of one hash (1, 256,
        // 2^24, ...) sort big-endian; each paying one of three scripts.
        let mut outs: BTreeMap<Vec<u8>, u8> = BTreeMap::new();
        while outs.len() < n {
            let mut op = vec![0u8; 36];
            if r.below(2) == 0 {
                op[0] = r.below(4) as u8;
            } else {
                r.fill(&mut op[..32]);
            }
            let index: u32 = match r.below(4) {
                0 => r.below(3) as u32,
                1 => 1 << (8 * r.below(4)),
                _ => r.next_u64() as u32,
            };
            op[32..].copy_from_slice(&index.to_be_bytes());
            outs.insert(op, r.below(3) as u8);
        }
        let mut entries: Vec<RawEntry> = outs.keys().map(|k| (k.clone(), vec![0])).collect();
        // Insert in a shuffled order: the order comes from the keys alone.
        for i in (1..entries.len()).rev() {
            entries.swap(i, r.below(i as u64 + 1) as usize);
        }
        let mut ops = vec![Op::First];
        ops.extend(std::iter::repeat_n(Op::Next, n));
        let script = Script {
            name: format!("mu-walk-{case}"),
            entries,
            other: Vec::new(),
            writable: false,
            exact: true,
            ops,
        };
        let go = run_oracle(&mut oracle, &script);
        let walked: Vec<Vec<u8>> = go["results"]
            .as_array()
            .expect("results")
            .iter()
            .filter_map(|v| v["k"].as_str().map(unhex))
            .collect();
        assert_eq!(walked.len(), n, "{}: bbolt's walk", script.name);
        let store = OpenOptions::new()
            .create_on_backend(redb::backends::InMemoryBackend::new(), |tx| {
                for (k, s) in &outs {
                    let op = purse_store::Outpoint {
                        hash: purse_store::Hash32(k[..32].try_into().expect("hash")),
                        index: u32::from_be_bytes(k[32..].try_into().expect("index")),
                    };
                    tx.put_multisig_out(
                        &op,
                        &purse_store::MultisigOutRow {
                            script_hash: purse_store::Hash160([*s; 20]),
                            m: 1,
                            n: 1,
                            stake_tree: false,
                            block: None,
                            amount: 1,
                            spender: None,
                        },
                    )?;
                }
                Ok(())
            })
            .expect("store");
        let tx = store.read().expect("read");
        for s in 0..3u8 {
            let want: Vec<Vec<u8>> = walked.iter().filter(|k| outs[*k] == s).cloned().collect();
            let got: Vec<Vec<u8>> = tx
                .unspent_multisig_for(&purse_store::Hash160([s; 20]))
                .expect("listing")
                .iter()
                .map(|o| o.encode().to_vec())
                .collect();
            assert_eq!(got, want, "{}: script {s}", script.name);
        }
    }
}

/// Scripts that run off the ends of the table and write around them
/// (`edge_script`) against bbolt itself.
#[test]
fn moves_off_the_ends_match_bbolt() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let mut r = SplitMix64::from_env("cursor-edges");
    let mut tally = Tally::default();
    for i in 0..400 {
        let s = edge_script(&mut r, format!("edge-{i}"));
        let go = run_oracle(&mut oracle, &s);
        check_script(&s, &go, &mut tally);
    }
    eprintln!("{tally:?}");
    assert!(tally.compared > tally.skipped, "{tally:?}");
    assert!(tally.prevs_again > 30, "{tally:?}");
}

/// Two put limits bbolt has and the store keeps, one key over and at each
/// (too large to sit in the frozen vectors).
#[test]
fn key_limits_match_bbolt() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let max = purse_store::MAX_KEY_SIZE;
    let s = named(
        "key_limits",
        kv(&[b"a".to_vec()]),
        true,
        vec![
            Op::Put(vec![7; max + 1], b"x".to_vec()),
            Op::Put(vec![7; max], b"x".to_vec()),
            Op::Get(vec![7; max]),
            Op::Seek(vec![7; max]),
        ],
    );
    let go = run_oracle(&mut oracle, &s);
    assert_eq!(
        go["results"][0]["err"].as_str(),
        Some("invalid operation: key too large")
    );
    check_script(&s, &go, &mut Tally::default());
}

/// QK-0750: bbolt's `Last` on a bucket of several leaves whose every key
/// was deleted earlier in the transaction never returns, while `First`,
/// `Seek` and `Prev` there return nil at once; the store's `last` returns
/// `None`. The hang is shown by a time limit; the frozen vectors cannot
/// hold it, since Go has no answer to record.
#[test]
fn bbolt_last_on_an_emptied_multileaf_bucket_never_returns() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    check_last_on_an_emptied_bucket(&mut oracle, PAGE_SIZE, 400, true);
}

/// Runs QK-0750's scripts at `page_size` on a bucket of `n` keys
/// ([`wide_n`]), every one deleted before the moves: a control (`First`,
/// `Seek`, `Prev`, `Count`), which Go answers as the store does, then
/// `Last`. When `hangs`, the bucket must span several leaves and bbolt's
/// `Last` must not answer; the oracle cannot be used afterwards. Otherwise
/// the bucket must be one leaf (or inline in its parent, which bbolt does
/// with a bucket under a quarter of a page) and bbolt's `Last` must return
/// nil, as the store's does.
fn check_last_on_an_emptied_bucket(oracle: &mut Oracle, page_size: u32, n: u16, hangs: bool) {
    let with = |name: &str, tail: Vec<Op>| {
        let mut ops: Vec<Op> = (0..n).map(|i| Op::Del(i.to_be_bytes().to_vec())).collect();
        ops.extend(tail);
        named(&format!("{name}_{n}_at_{page_size}"), wide_n(n), true, ops)
    };
    let control = with(
        "qk0750_control",
        vec![
            Op::First,
            Op::Seek(Vec::new()),
            Op::Seek(vec![0xff, 0xff]),
            Op::Prev,
            Op::Count,
        ],
    );
    let go = run_oracle_at(oracle, &control, page_size);
    let leaves = leaf_pages(&go);
    if hangs {
        assert!(leaves > 1, "{}: {}", control.name, go["layout"]);
    } else {
        assert!(leaves <= 1, "{}: {}", control.name, go["layout"]);
    }
    let ours = run_write(&control);
    for i in 0..control.ops.len() - 1 {
        assert_eq!(
            ours.results[i], go["results"][i],
            "{}: op {i}",
            control.name
        );
    }
    // Last: the store stands on nothing; bbolt never answers when the
    // emptied bucket spans several leaves.
    let last = with("qk0750_last", vec![Op::Last]);
    let ours = run_write(&last);
    let at = usize::from(n);
    assert_eq!(ours.results[at], json!({"k": null, "v": null}));
    let answer = oracle.call_within(
        "run",
        last.request(&oracle_db_path(), page_size),
        Duration::from_secs(10),
    );
    if hangs {
        assert!(
            answer.is_none(),
            "bbolt answered Last on an emptied multi-leaf bucket (QK-0750 fixed?): {answer:?}"
        );
    } else {
        let answer = answer.expect("bbolt answers Last on an emptied one-leaf bucket");
        assert_eq!(answer["results"][at], ours.results[at], "{}", last.name);
    }
}

/// The multi-leaf defects at 16 KiB pages, the page size dcrwallet's
/// database gets on Apple silicon ([`multileaf_defects_at`]).
#[test]
fn multileaf_defects_at_16_kib_pages() {
    multileaf_defects_at(16384);
}

/// The multi-leaf defects at 64 KiB pages, the page size of Linux kernels
/// built with 64 KiB pages ([`multileaf_defects_at`]).
#[test]
fn multileaf_defects_at_64_kib_pages() {
    multileaf_defects_at(65536);
}

/// QK-0747, QK-0749 and QK-0750 follow bbolt's page size, which a
/// dcrwallet database takes from the host that created it. At
/// `page_size`, the 4 KiB scripts' bucket of 400 keys is one leaf (at
/// 64 KiB, inline in its parent: under a quarter of a page), and Go
/// answers every op of them as the store does, `Last` included; the same
/// scripts with their keys scaled by `page_size` / 4096
/// ([`multileaf_divergences`]) span several leaves and differ where the
/// 4 KiB scripts do, and `Last` on the emptied bucket never returns. The
/// Go panics ([`panic_cases`], QK-0748 among them) need no second leaf and
/// show at every page size. The rest of the differential holds at
/// `page_size` as at 4 KiB ([`corpus_matches_at`]).
fn multileaf_defects_at(page_size: u32) {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    for (s, at, want_c) in panic_cases() {
        let go = run_oracle_at(&mut oracle, &s, page_size);
        check_panic(&s, at, &want_c, &go);
    }
    for (s, _) in multileaf_divergences(1) {
        let go = run_oracle_at(&mut oracle, &s, page_size);
        assert!(leaf_pages(&go) <= 1, "{}: {}", s.name, go["layout"]);
        check_divergence(&s, &[], &go);
    }
    check_last_on_an_emptied_bucket(&mut oracle, page_size, 400, false);
    corpus_matches_at(&mut oracle, page_size);
    let f = scale_of(page_size);
    for (s, want) in multileaf_divergences(f) {
        let go = run_oracle_at(&mut oracle, &s, page_size);
        assert!(leaf_pages(&go) > 1, "{}: {}", s.name, go["layout"]);
        check_divergence(&s, &want, &go);
    }
    // Last, since the hang leaves the oracle unusable.
    check_last_on_an_emptied_bucket(&mut oracle, page_size, 400 * f, true);
}

/// The general differential at `page_size`, the layout a dcrwallet
/// database created on a host with that page size has: every named case,
/// a random corpus of its own seed, scripts off the ends of the table
/// ([`edge_script`]) and the divergences that need no second leaf, checked
/// as `cursors_match_bbolt` and `moves_off_the_ends_match_bbolt` check
/// them at [`PAGE_SIZE`].
fn corpus_matches_at(oracle: &mut Oracle, page_size: u32) {
    let mut tally = Tally::default();
    let random = corpus(&format!("cursor-conformance-{page_size}"), None, 400, true);
    for s in named_cases().iter().chain(&random) {
        let go = run_oracle_at(oracle, s, page_size);
        check_script(s, &go, &mut tally);
    }
    let mut r = SplitMix64::from_env(&format!("cursor-edges-{page_size}"));
    let mut edges = Tally::default();
    for i in 0..100 {
        let s = edge_script(&mut r, format!("edge-{i}"));
        let go = run_oracle_at(oracle, &s, page_size);
        check_script(&s, &go, &mut edges);
    }
    let spans: BTreeSet<String> = multileaf_divergences(1)
        .into_iter()
        .map(|(s, _)| s.name)
        .collect();
    for (s, want) in divergence_cases() {
        if !spans.contains(&s.name) {
            let go = run_oracle_at(oracle, &s, page_size);
            check_divergence(&s, &want, &go);
        }
    }
    eprintln!("{page_size}-byte pages: {tally:?}\nedges: {edges:?}");
    assert!(
        tally.compared > MIN_COMPARED_PER_SKIPPED * tally.skipped,
        "{tally:?}"
    );
    assert!(tally.deletes_compared > 100, "{tally:?}");
    assert!(tally.steps_after_delete > 100, "{tally:?}");
    assert!(edges.compared > edges.skipped, "{edges:?}");
}

/// A random write script in which bbolt panics (QK-0748) on an op the rule
/// does not compare is accepted, whether the panic stops the script early
/// or falls on its last op. In the second case every op has a result but
/// the oracle, having rolled the transaction back, returns no final
/// contents; the comparison once took a full set of results for a script
/// that ran to its end and demanded them. Over `PURSE_TEST_SEED` 1 to 150
/// that failed at 4 seeds of the 16 KiB corpus, 2 of the 64 KiB corpus and
/// 1 of the record tables' corpus (the panic `Go's final contents`).
#[test]
fn a_go_panic_on_an_uncompared_op_is_accepted_wherever_it_falls() {
    let Some(mut oracle) = oracle_or_skip("boltcursor") else {
        return;
    };
    let ops = vec![
        Op::Del(b"a".to_vec()),
        Op::Last,
        Op::Put(b"b".to_vec(), Vec::new()),
        Op::Delete,
    ];
    let mut tail = ops.clone();
    tail.push(Op::Get(b"b".to_vec()));
    for (name, ops) in [("panic-last", ops), ("panic-then-more", tail)] {
        let mut s = named(name, kv(&[b"a".to_vec()]), true, ops);
        s.exact = false;
        let go = run_oracle(&mut oracle, &s);
        let mut tally = Tally::default();
        check_script(&s, &go, &mut tally);
        assert!(tally.go_panics > 0, "{name}: {tally:?}");
        assert_eq!(tally.finals, 0, "{name}: {tally:?}");
    }
}

/// The rule leaves out what follows a `next` that ran off the end in a
/// transaction that deleted from `c` (QK-0749): the cursor `delete`, the
/// `prev` and the final contents of `c`, and, once that `delete` went
/// uncompared, every later read of `c`; the `next` itself is compared.
#[test]
fn the_rule_skips_moves_after_next_ran_off_over_deletions() {
    let mut ops: Vec<Op> = (200u16..400)
        .map(|i| Op::Del(i.to_be_bytes().to_vec()))
        .collect();
    let at = ops.len();
    ops.extend([
        Op::Seek(vec![0x00, 0xc7]),
        Op::Next,
        Op::Delete,
        Op::Prev,
        Op::Seek(vec![0x00, 0x10]),
        Op::Next,
    ]);
    let s = Script {
        exact: false,
        ..named("rule_after_next_off_the_end", wide(), true, ops)
    };
    let ours = run_write(&s);
    let rule = comparable(&s, &ours.results);
    assert_eq!(
        &rule.ops[at..],
        &[true, true, false, false, false, false],
        "seek, next, delete, prev, seek, next"
    );
    assert!(!rule.final_c);
    // Without the deletions the same moves are bbolt's and compared.
    let s = Script {
        exact: false,
        ..named(
            "rule_after_next_off_the_end_no_deletes",
            wide(),
            true,
            vec![Op::Last, Op::Next, Op::Prev, Op::Next, Op::Delete],
        )
    };
    let ours = run_write(&s);
    assert_eq!(
        comparable(&s, &ours.results),
        Rule {
            ops: vec![true; 5],
            final_c: true,
            prevs_again: 0
        },
        "no deletion: every op compared"
    );
}

/// A `prev` that ran off the start is a positioning (bbolt's `Prev` calls
/// its `first`): a further `prev` and the moves after it are compared after
/// writes to the table, unless something had been deleted from it when the
/// cursor ran off.
#[test]
fn the_rule_compares_moves_after_prev_ran_off_the_start() {
    let abcd = kv(&[b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
    let script = |name: &str, entries: Vec<RawEntry>, ops: Vec<Op>| Script {
        exact: false,
        ..named(name, entries, true, ops)
    };
    // first, prev (off the start), del a, prev (again), next, delete.
    let s = script(
        "rule_second_prev_off_the_start",
        abcd.clone(),
        vec![
            Op::First,
            Op::Prev,
            Op::Del(b"a".to_vec()),
            Op::Prev,
            Op::Next,
            Op::Delete,
        ],
    );
    let ours = run_write(&s);
    assert_eq!(
        comparable(&s, &ours.results),
        Rule {
            ops: vec![true; 6],
            final_c: true,
            prevs_again: 1
        }
    );
    // The same after a deletion before the cursor ran off: the first prev
    // off the start is not compared, so neither is anything after it.
    let s = script(
        "rule_second_prev_after_an_earlier_delete",
        abcd.clone(),
        vec![
            Op::Del(b"c".to_vec()),
            Op::First,
            Op::Prev,
            Op::Put(b"0".to_vec(), Vec::new()),
            Op::Prev,
            Op::Next,
        ],
    );
    let ours = run_write(&s);
    let rule = comparable(&s, &ours.results);
    assert_eq!(rule.ops, [true, true, false, true, false, false]);
    assert_eq!(rule.prevs_again, 0);
    // A second prev off the start after a deletion is compared, and the
    // next after it; a third is not (first may have skipped a leaf the
    // deletion emptied).
    let s = script(
        "rule_third_prev_after_a_delete",
        abcd,
        vec![
            Op::First,
            Op::Prev,
            Op::Del(b"a".to_vec()),
            Op::Prev,
            Op::Del(b"b".to_vec()),
            Op::Prev,
            Op::Next,
        ],
    );
    let ours = run_write(&s);
    let rule = comparable(&s, &ours.results);
    assert_eq!(rule.ops, [true, true, true, true, true, false, false]);
    assert_eq!(rule.prevs_again, 1);
}

// ---------------------------------------------------------------------
// Frozen vectors
// ---------------------------------------------------------------------

const VECTORS: &str = "tests/data/cursor_vectors.jsonl";
/// The fixed seed of the frozen corpus (independent of `PURSE_TEST_SEED`).
const FROZEN_SEED: u64 = 0x6375_7273_6f72_7631;
const FROZEN_RANDOM: usize = 90;
/// The fixed seed of the frozen scripts that run off the ends
/// (`edge_script`).
const FROZEN_EDGE_SEED: u64 = 0x6375_7273_6f72_6532;
const FROZEN_EDGES: usize = 16;
/// The number of entries the file holds: 27 named cases, 6 divergences,
/// 2 Go panics, the random corpus and its 2 dcrwallet patterns, and the
/// scripts that run off the ends.
const FROZEN_ENTRIES: usize = 27 + 6 + 2 + FROZEN_RANDOM + 2 + FROZEN_EDGES;

fn vectors_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS)
}

fn body_sha256(body: &str) -> String {
    hex(&Sha256::digest(body.as_bytes()))
}

fn body_of(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect()
}

fn header_field<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key:?} header"))
        .trim()
}

/// Replays the vectors the Go oracle wrote; runs without Go. The file's
/// entry count and body hash are checked, and the count is pinned here.
#[test]
fn frozen_vectors() {
    let text = std::fs::read_to_string(vectors_path()).expect("read the frozen vectors");
    assert!(
        text.lines()
            .take_while(|l| l.starts_with('#'))
            .any(|l| l == format!("# dcrwallet commit: {DCRWALLET_PARITY_COMMIT}")),
        "the vectors do not name the parity commit"
    );
    let body = body_of(&text);
    assert_eq!(
        body_sha256(&body),
        header_field(&text, "# body-sha256:"),
        "body hash"
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines.len().to_string(),
        header_field(&text, "# entries:"),
        "entry count"
    );
    assert_eq!(
        lines.len(),
        FROZEN_ENTRIES,
        "the vectors are truncated or stale"
    );

    let mut tally = Tally::default();
    let mut names = BTreeSet::new();
    let divergences: BTreeMap<String, Vec<usize>> = divergence_cases()
        .into_iter()
        .map(|(s, want)| (s.name, want))
        .collect();
    let panics = panic_cases();
    for line in lines {
        let rec: Value = serde_json::from_str(line).expect("vector line");
        let s = Script::from_json(&rec["script"]);
        names.insert(s.name.clone());
        if let Some((_, at, want_c)) = panics.iter().find(|p| p.0.name == s.name) {
            check_panic(&s, *at, want_c, &rec["go"]);
        } else if let Some(want) = divergences.get(&s.name) {
            check_divergence(&s, want, &rec["go"]);
        } else {
            check_script(&s, &rec["go"], &mut tally);
        }
    }
    for s in named_cases() {
        assert!(names.contains(&s.name), "no frozen vector for {}", s.name);
    }
    for name in divergences.keys().chain(panics.iter().map(|p| &p.0.name)) {
        assert!(names.contains(name), "no frozen vector for {name}");
    }
    eprintln!("{tally:?}");
    assert!(tally.deletes_compared > 20, "{tally:?}");
}

/// Rewrites `tests/data/cursor_vectors.jsonl` from the Go oracle and
/// checks the store against it:
/// `PURSE_REQUIRE_ORACLE=1 cargo test -p purse-store --test conformance -- --ignored regenerate`.
#[test]
#[ignore = "writes tests/data from the Go oracle; run by hand to regenerate"]
fn regenerate_frozen_vectors() {
    let mut oracle = oracle_or_skip("boltcursor").expect("the Go oracle is required to regenerate");
    // The release only: a distribution's build appends its own tags.
    let go_version = oracle
        .go_version()
        .split(['-', ' ', '+'])
        .next()
        .unwrap_or_default()
        .to_owned();
    let mut scripts: Vec<Script> = named_cases();
    scripts.extend(divergence_cases().into_iter().map(|(s, _)| s));
    scripts.extend(panic_cases().into_iter().map(|(s, ..)| s));
    scripts.extend(corpus("frozen", Some(FROZEN_SEED), FROZEN_RANDOM, false));
    let mut r = SplitMix64(FROZEN_EDGE_SEED);
    scripts.extend((0..FROZEN_EDGES).map(|i| edge_script(&mut r, format!("edge-{i}"))));
    assert_eq!(scripts.len(), FROZEN_ENTRIES);
    let mut body = String::new();
    for s in &scripts {
        let mut go = run_oracle(&mut oracle, s);
        if let Some(f) = go.get("final").filter(|f| !f.is_null()) {
            go["final"] = digests(f);
        }
        // The layout is checked where it matters (`cursors_match_bbolt`);
        // the vectors hold the answers only.
        go.as_object_mut().expect("an object").remove("layout");
        writeln!(body, "{}", json!({"script": s.to_json(), "go": go})).expect("write to String");
    }
    let mut text = String::new();
    text.push_str(
        "# Frozen cursor-conformance vectors for purse-store, generated by the Go oracle\n\
         # tools/oracle/cmd/boltcursor: bbolt v1.3.12 reached through dcrwallet's walletdb\n\
         # interface and bdb driver at the parity pin.\n",
    );
    writeln!(
        text,
        "# Every script ran with {PAGE_SIZE}-byte bbolt pages, whatever the host's page size."
    )
    .expect("write to String");
    writeln!(text, "# Built with {go_version}.").expect("write to String");
    writeln!(text, "# dcrwallet commit: {DCRWALLET_PARITY_COMMIT}").expect("write to String");
    text.push_str(
        "# Regenerate with:\n\
         #   PURSE_REQUIRE_ORACLE=1 cargo test -p purse-store --test conformance \\\n\
         #       -- --ignored regenerate_frozen_vectors\n\
         # Do not edit by hand. Each line after this header is {\"script\", \"go\"}: a script\n\
         # (its key sets, whether it writes, whether every op is compared, its ops) and\n\
         # bbolt's results, the final contents of a writing script as the SHA-256 of each\n\
         # table's \"<key hex> <value hex>\" lines. The named cases come first, then the\n\
         # divergences and the Go panics, then\n",
    );
    writeln!(
        text,
        "# {FROZEN_RANDOM} generated scripts from SplitMix64 seed {FROZEN_SEED:#x} and two \
         dcrwallet patterns,\n# then {FROZEN_EDGES} scripts that run off the ends of the table, \
         from seed {FROZEN_EDGE_SEED:#x}."
    )
    .expect("write to String");
    writeln!(text, "# entries: {}", scripts.len()).expect("write to String");
    writeln!(text, "# body-sha256: {}", body_sha256(&body)).expect("write to String");
    text.push_str(&body);
    std::fs::write(vectors_path(), &text).expect("write the frozen vectors");
    // The file just written must pass.
    frozen_vectors();
}

// ---------------------------------------------------------------------
// Store-only properties of the API around the cursors
// ---------------------------------------------------------------------

/// The typed cursor answers as the raw cursor under it, decoded.
#[test]
fn typed_cursor_follows_the_raw_cursor() {
    use purse_store::{AccountKey, AccountRow, Accounts};
    let store = OpenOptions::new()
        .create_on_backend(redb::backends::InMemoryBackend::new(), |tx| {
            for n in ACCOUNTS {
                let row = AccountRow {
                    name: format!("acct-{n}").into_bytes(),
                    ..AccountRow::default()
                };
                tx.put::<Accounts>(&AccountKey(n), &row)?;
            }
            Ok(())
        })
        .expect("create");
    let tx = store.read().expect("read");
    let mut c = tx.typed_cursor::<Accounts>().expect("cursor");
    let mut forward = Vec::new();
    let mut e = c.first().expect("first");
    while let Some((k, row)) = e {
        assert_eq!(row.name, format!("acct-{}", k.0).into_bytes());
        forward.push(k.0);
        e = c.next().expect("next");
    }
    let mut want = ACCOUNTS.to_vec();
    want.sort_by_key(|n| n.to_le_bytes());
    assert_eq!(forward, want);
    // Previous of end is last, through the typed seek.
    assert!(c.seek_bytes(&[0xff; 5]).expect("seek").is_none());
    assert_eq!(c.prev().expect("prev").map(|e| e.0.0), want.last().copied());
    assert_eq!(
        c.seek(&AccountKey(256)).expect("seek").map(|e| e.0.0),
        Some(256)
    );
    assert_eq!(c.raw().next().expect("raw next").map(|e| e.0), Some(le(1)));
    drop(c);
    drop(tx);

    // A typed write cursor deletes through its raw cursor and writes
    // through the transaction between moves.
    store
        .update(|tx| {
            let mut c = tx.typed_cursor::<Accounts>()?;
            let mut seen = Vec::new();
            let mut e = c.first()?;
            while let Some((k, _)) = e {
                seen.push(k.0);
                if k.0 % 2 == 0 {
                    c.raw().delete()?;
                }
                if k.0 == 1 {
                    // Inserted ahead of the cursor (0x02.. sorts after 0x01..):
                    // reached by a later step.
                    c.raw()
                        .tx()
                        .put::<Accounts>(&AccountKey(3), &AccountRow::default())?;
                }
                e = c.next()?;
            }
            assert!(seen.contains(&3), "{seen:?}");
            Ok::<_, Error>(())
        })
        .expect("update");
    let left: Vec<u32> = store
        .read()
        .expect("read")
        .iter::<Accounts>()
        .expect("iter")
        .map(|e| e.expect("entry").0.0)
        .collect();
    let mut want: Vec<u32> = ACCOUNTS.iter().copied().filter(|n| n % 2 == 1).collect();
    want.push(3);
    want.sort_by_key(|n| n.to_le_bytes());
    assert_eq!(left, want);
}

/// Writes between a write cursor's moves, in its own table, follow the
/// store's rule: nothing deleted is returned, nothing is skipped or
/// repeated, an insertion ahead is reached.
#[test]
fn writes_between_moves_follow_the_rule() {
    let mut r = SplitMix64::from_env("cursor-writes-between-moves");
    for round in 0..300 {
        let g = KeyGen::new(&mut r, Shape::Alphabet);
        let s = Script {
            name: format!("rule-{round}"),
            entries: {
                let n = r.below(30) as usize;
                key_set(&mut r, &g, n, 2)
            },
            other: Vec::new(),
            writable: true,
            exact: true,
            ops: Vec::new(),
        };
        let store = new_store(&s);
        store
            .update(|tx| {
                let mut model: BTreeSet<Vec<u8>> = s.entries.iter().map(|e| e.0.clone()).collect();
                let mut c = tx.cursor(c_table())?;
                let forward = r.below(2) == 0;
                let mut at = if forward { c.first()? } else { c.last()? }.map(|e| e.0);
                let mut seen = Vec::new();
                while let Some(k) = at {
                    assert!(model.contains(&k), "round {round}: returned a deleted key");
                    seen.push(k.clone());
                    for _ in 0..r.below(3) {
                        let w = g.key(&mut r);
                        if r.below(2) == 0 {
                            c.tx().put_raw(c_table(), &w, b"")?;
                            model.insert(w);
                        } else {
                            c.tx().delete_raw(c_table(), &w)?;
                            model.remove(&w);
                        }
                    }
                    if r.below(3) == 0 {
                        c.delete()?;
                        model.remove(&k);
                    }
                    let want = if forward {
                        model
                            .range((Bound::Excluded(k.clone()), Bound::Unbounded))
                            .next()
                    } else {
                        model.range(..k.clone()).next_back()
                    }
                    .cloned();
                    at = if forward { c.next()? } else { c.prev()? }.map(|e| e.0);
                    assert_eq!(at, want, "round {round}: step from {k:?}");
                }
                let mut sorted = seen.clone();
                sorted.sort();
                sorted.dedup();
                if !forward {
                    sorted.reverse();
                }
                assert_eq!(seen, sorted, "round {round}: visited out of order or twice");
                Ok::<_, Error>(())
            })
            .expect("update");
    }
}

/// Where the write cursor's documented rule (`purse_store::WriteCursor`,
/// "Writes between moves") says it stands.
#[derive(Clone, Debug)]
enum At {
    Unpositioned,
    On(Vec<u8>),
    EndSeek,
    EndNext(Vec<u8>),
    Start(Vec<u8>),
    Nowhere,
}

/// The documented rule, written out over a sorted set of keys.
struct RuleModel {
    keys: BTreeSet<Vec<u8>>,
    at: At,
}

impl RuleModel {
    fn land(&mut self, k: Option<Vec<u8>>, none: At) -> Option<Vec<u8>> {
        self.at = k.clone().map_or(none, At::On);
        k
    }

    fn first(&mut self) -> Option<Vec<u8>> {
        let k = self.keys.first().cloned();
        self.land(k, At::EndSeek)
    }

    fn last(&mut self) -> Option<Vec<u8>> {
        let k = self.keys.last().cloned();
        self.land(k, At::EndSeek)
    }

    fn seek(&mut self, p: &[u8]) -> Option<Vec<u8>> {
        let k = self.keys.range(p.to_vec()..).next().cloned();
        self.land(k, At::EndSeek)
    }

    fn next(&mut self) -> Option<Vec<u8>> {
        let (At::On(k) | At::Start(k) | At::EndNext(k)) = self.at.clone() else {
            return None;
        };
        let n = self
            .keys
            .range((Bound::Excluded(k.clone()), Bound::Unbounded))
            .next()
            .cloned();
        self.land(n, At::EndNext(k))
    }

    fn prev(&mut self) -> Option<Vec<u8>> {
        let p = match &self.at {
            At::Unpositioned => return None,
            At::On(k) | At::EndNext(k) => self.keys.range(..k.clone()).next_back().cloned(),
            At::EndSeek => self.keys.last().cloned(),
            At::Start(_) | At::Nowhere => None,
        };
        if p.is_none() {
            self.at = self.keys.first().cloned().map_or(At::Nowhere, At::Start);
            return None;
        }
        self.land(p, At::EndSeek)
    }

    fn delete(&mut self) -> bool {
        match &self.at {
            At::On(k) | At::EndNext(k) | At::Start(k) => self.keys.remove(&k.clone()),
            _ => false,
        }
    }
}

/// Random moves, cursor deletes and writes to the cursor's own table, the
/// ends of the table included, answer as the write cursor's documented
/// rule says, where bbolt's answer may depend on its page layout and the
/// oracle cannot judge (the rule is the store's own; docs/PARITY.md).
#[test]
fn the_write_cursor_follows_its_documented_rule() {
    let mut r = SplitMix64::from_env("cursor-documented-rule");
    for round in 0..400 {
        let g = KeyGen::new(&mut r, Shape::Alphabet);
        let n = r.below(10) as usize;
        let s = named(
            &format!("doc-rule-{round}"),
            key_set(&mut r, &g, n, 2),
            true,
            Vec::new(),
        );
        let store = new_store(&s);
        store
            .update(|tx| {
                let mut m = RuleModel {
                    keys: s.entries.iter().map(|e| e.0.clone()).collect(),
                    at: At::Unpositioned,
                };
                let mut c = tx.cursor(c_table())?;
                let key = |e: Option<RawEntry>| e.map(|e| e.0);
                for step in 0..60 {
                    let what = r.below(14);
                    let (got, want) = match what {
                        0 => (key(c.first()?), m.first()),
                        1 => (key(c.last()?), m.last()),
                        2..=4 => (key(c.next()?), m.next()),
                        5..=7 => (key(c.prev()?), m.prev()),
                        8 => {
                            let p = g.key(&mut r);
                            (key(c.seek(&p)?), m.seek(&p))
                        }
                        9 => {
                            let deleted = c.delete()?;
                            assert_eq!(deleted, m.delete(), "round {round} step {step}: delete");
                            continue;
                        }
                        10..=11 => {
                            let k = g.key(&mut r);
                            c.tx().put_raw(c_table(), &k, b"")?;
                            m.keys.insert(k);
                            continue;
                        }
                        _ => {
                            let k = match m
                                .keys
                                .iter()
                                .nth(r.below(m.keys.len() as u64 + 1) as usize)
                            {
                                Some(k) => k.clone(),
                                None => g.key(&mut r),
                            };
                            c.tx().delete_raw(c_table(), &k)?;
                            m.keys.remove(&k);
                            continue;
                        }
                    };
                    assert_eq!(
                        got, want,
                        "round {round} step {step}: op {what}, at {:?}",
                        m.at
                    );
                }
                Ok::<_, Error>(())
            })
            .expect("update");
    }
}

/// An empty key, and one over [`purse_store::MAX_KEY_SIZE`], are refused
/// with bbolt's kind and text through `convertErr`, and the transaction
/// stays usable.
#[test]
fn put_limits() {
    let store = new_store(&named("limits", Vec::new(), true, Vec::new()));
    store
        .update(|tx| {
            let e = tx.put_raw(c_table(), b"", b"v").expect_err("empty key");
            assert_eq!(e.to_string(), "invalid operation: key required");
            assert_eq!(e.kind(), purse_errors::Kind::Invalid);
            let e = tx
                .put_raw(c_table(), &vec![1; purse_store::MAX_KEY_SIZE + 1], b"v")
                .expect_err("long key");
            assert_eq!(e.to_string(), "invalid operation: key too large");
            tx.put_raw(c_table(), &vec![1; purse_store::MAX_KEY_SIZE], b"v")?;
            tx.put_raw(c_table(), b"k", b"")?;
            Ok::<_, Error>(())
        })
        .expect("update");
    assert_eq!(store.read().expect("read").len(c_table()).expect("len"), 2);
}

/// Ranges with the lower bound above the upper: empty, as bbolt's walk.
#[test]
fn inverted_ranges_are_empty() {
    let s = named(
        "inverted",
        kv(&[b"a".to_vec(), b"b".to_vec()]),
        false,
        Vec::new(),
    );
    let store = new_store(&s);
    let tx = store.read().expect("read");
    for (lo, hi) in [
        (Bound::Included(&b"b"[..]), Bound::Excluded(&b"a"[..])),
        (Bound::Excluded(&b"b"[..]), Bound::Included(&b"b"[..])),
        (Bound::Included(&b"b"[..]), Bound::Excluded(&b"b"[..])),
    ] {
        let n = tx.range_raw(c_table(), lo, hi).map(|it| it.count());
        assert!(matches!(n, Ok(0)), "range {lo:?}..{hi:?}: {n:?}");
    }
}
