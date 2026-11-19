//! Quick checks of the prototype: bbolt cursor semantics against a model and
//! against bbolt itself, the consistency check after every transaction and
//! its power to find drift, the digest's coverage, the schema version check,
//! the cross-check against dcrwallet on the tiny workload (with stake
//! invalidation, revalidation and reorganisations), a short crash run and a
//! sampled power-loss sweep. The measured runs are the `spike` binary's (see
//! README.md).

use std::path::PathBuf;
use std::process::Command;

use purse_testutil::SplitMix64;
use purse_testutil::serde_json::{self, Value};
use redb::{
    Database, ReadableDatabase as _, ReadableTable as _, TableDefinition, TableHandle as _,
};
use store_redb_spike::bench;
use store_redb_spike::store::{self, BoltCursor, Counters, K, ParsedOp, Store};
use store_redb_spike::workload::{self, Params};

fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("store-redb-spike")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp dir");
    d
}

/// bbolt v1.3.12's cursor over a sorted key list, written from its source
/// (cursor.go): the position is an element index; `seek` past the end
/// leaves it one past the last element, `next` at the last element stays
/// there, `prev` at the first element stays there (bbolt#733).
struct Model<'a> {
    keys: &'a [Vec<u8>],
    pos: usize,
}

impl Model<'_> {
    fn seek(&mut self, k: &[u8]) -> Option<Vec<u8>> {
        self.pos = self.keys.partition_point(|x| x.as_slice() < k);
        self.keys.get(self.pos).cloned()
    }
    fn next(&mut self) -> Option<Vec<u8>> {
        if self.pos + 1 >= self.keys.len() {
            self.pos = self.pos.min(self.keys.len());
            return None;
        }
        self.pos += 1;
        self.keys.get(self.pos).cloned()
    }
    fn prev(&mut self) -> Option<Vec<u8>> {
        if self.pos == 0 || self.keys.is_empty() {
            self.pos = 0;
            return None;
        }
        self.pos -= 1;
        self.keys.get(self.pos).cloned()
    }
}

#[test]
fn bolt_cursor_matches_bbolt_model() {
    const T: TableDefinition<K, K> = TableDefinition::new("t");
    let dir = tmp("cursor");
    let db = Database::create(dir.join("c.redb")).expect("create");
    let mut rng = SplitMix64::from_env("bolt_cursor_matches_bbolt_model");
    for n in [0usize, 1, 2, 3, 50, 700] {
        let mut keys: Vec<Vec<u8>> = (0..n)
            .map(|_| {
                let len = 1 + rng.below(3) as usize;
                rng.bytes(len)
            })
            .collect();
        keys.sort();
        keys.dedup();
        let txn = db.begin_write().expect("write");
        {
            let mut t = txn.open_table(T).expect("table");
            t.retain(|_, _| false).expect("clear");
            for k in &keys {
                t.insert(k.as_slice(), [].as_slice()).expect("insert");
            }
        }
        txn.commit().expect("commit");
        let rtxn = redb::ReadableDatabase::begin_read(&db).expect("read");
        let t = rtxn.open_table(T).expect("table");
        for _ in 0..300 {
            let len = rng.below(3) as usize;
            let seek = rng.bytes(len);
            let mut c = BoltCursor::new(&t);
            let mut m = Model {
                keys: &keys,
                pos: 0,
            };
            let got = c.seek(&seek).expect("seek").map(|e| e.0);
            assert_eq!(got, m.seek(&seek), "seek {seek:?} over {n} keys");
            for step in 0..8 {
                let fwd = rng.below(2) == 0;
                let (got, want) = if fwd {
                    (c.next().expect("next").map(|e| e.0), m.next())
                } else {
                    (c.prev().expect("prev").map(|e| e.0), m.prev())
                };
                assert_eq!(
                    got,
                    want,
                    "step {step} ({}) after seek {seek:?}",
                    if fwd { "next" } else { "prev" }
                );
            }
        }
    }
}

/// The tiny workload ingested into a fresh store at `dir/w.redb`.
fn ingested(dir: &std::path::Path) -> (Store, workload::Stats) {
    let p = Params::preset("tiny").expect("preset");
    let (w, stats) = workload::generate(&p);
    let ops: Vec<ParsedOp> = w
        .ops
        .iter()
        .map(|o| ParsedOp::parse(o).expect("parse"))
        .collect();
    let st = Store::create(&dir.join("w.redb")).expect("create");
    st.setup(w.accounts, &w.ranges).expect("setup");
    let mut c = Counters::default();
    for g in bench::groups_of(&ops) {
        st.apply(&ops[g], &mut c).expect("apply");
    }
    (st, stats)
}

#[test]
fn consistency_holds_after_every_transaction() {
    let dir = tmp("invariant");
    let p = Params::preset("tiny").expect("preset");
    let (w, stats) = workload::generate(&p);
    assert!(stats.disapprovals >= 4 && stats.reorgs >= 5, "{stats:?}");
    assert!(
        stats.remined > 0 && stats.dropped > 0 && stats.delayed > 0,
        "{stats:?}"
    );
    assert!(stats.cross_account > 0 && stats.no_change > 0, "{stats:?}");
    let ops: Vec<ParsedOp> = w
        .ops
        .iter()
        .map(|o| ParsedOp::parse(o).expect("parse"))
        .collect();
    let st = Store::create(&dir.join("w.redb")).expect("create");
    st.setup(w.accounts, &w.ranges).expect("setup");
    let mut c = Counters::default();
    for g in bench::groups_of(&ops) {
        st.apply(&ops[g], &mut c).expect("apply");
        let r = st.read().expect("read");
        assert_eq!(r.check_balances().expect("check"), None);
        // The two unspent walks apply the same filters (txmined.go:2463,
        // :2557), so the outpoint walk is the projection of the other.
        let mut walk = Vec::new();
        r.for_each_unspent_outpoint(|k, tree| walk.push((k.to_vec(), tree)))
            .expect("walk");
        let outputs: Vec<(Vec<u8>, i8)> = r
            .unspent_outputs()
            .expect("outputs")
            .into_iter()
            .map(|u| (u.outpoint.to_vec(), u.tree))
            .collect();
        assert_eq!(walk, outputs);
    }
    assert_eq!(c.sync_misses, 0);
    assert!(c.credits > 0 && c.commitments > 0);
    let r = st.read().expect("read");
    assert!(!r.invalid_blocks().expect("invalid").is_empty());
    assert!(!r.invalidated_credits().expect("ic").is_empty());
    assert!(!r.invalidated_debits().expect("id").is_empty());
}

/// Overwrites one row of `table` in the store with `f` applied to its value.
fn tamper(db: &Database, table: TableDefinition<K, K>, pick: usize, f: impl Fn(&mut Vec<u8>)) {
    let rtxn = db.begin_read().expect("read");
    let (k, mut v) = {
        let t = rtxn.open_table(table).expect("table");
        let e = t
            .iter()
            .expect("iter")
            .nth(pick)
            .expect("row")
            .expect("row");
        (e.0.value().to_vec(), e.1.value().to_vec())
    };
    drop(rtxn);
    f(&mut v);
    let txn = db.begin_write().expect("write");
    {
        let mut t = txn.open_table(table).expect("table");
        t.insert(k.as_slice(), v.as_slice()).expect("insert");
    }
    txn.commit().expect("commit");
}

#[test]
fn consistency_check_finds_drift_in_denormalised_fields() {
    // Each case changes one copied field, in a way the old check (sums
    // against the stored classes) could not see, and must be reported.
    type Case = (
        &'static str,
        TableDefinition<'static, K, K>,
        fn(&mut Vec<u8>),
    );
    let cases: [Case; 6] = [
        ("unspent class", store::UNSPENT, |v| {
            v[17] = store::class::M_CB
        }),
        ("unspent account", store::UNSPENT, |v| v[7] ^= 1),
        ("credit amount", store::CREDITS, |v| v[7] ^= 1),
        ("credit script offset", store::CREDITS, |v| v[16] ^= 1),
        ("unmined credit class", store::UNMINED_CREDITS, |v| {
            v[13] = store::class::U_CB_PUB
        }),
        ("block transaction count", store::BLOCKS, |v| v[46] ^= 1),
    ];
    let dir = tmp("drift");
    let (st, _) = ingested(&dir);
    assert_eq!(
        st.read().expect("read").check_balances().expect("check"),
        None
    );
    drop(st);
    for (i, (what, table, f)) in cases.iter().enumerate() {
        let path = dir.join(format!("case{i}.redb"));
        std::fs::copy(dir.join("w.redb"), &path).expect("copy");
        let db = Database::open(&path).expect("open");
        // The last row of BLOCKS is the tip; others pick a middle row.
        let n = {
            let rtxn = db.begin_read().expect("read");
            let t = rtxn.open_table(*table).expect("table");
            redb::ReadableTableMetadata::len(&t).expect("len") as usize
        };
        tamper(&db, *table, n / 2, f);
        let st = Store::from_db(db).expect("store");
        let found = st.read().expect("read").check_balances().expect("check");
        assert!(found.is_some(), "drift in {what} not reported");
    }
    // A missing per-account history row.
    let path = dir.join("acct.redb");
    std::fs::copy(dir.join("w.redb"), &path).expect("copy");
    let db = Database::open(&path).expect("open");
    let txn = db.begin_write().expect("write");
    {
        let mut t = txn.open_table(store::ACCT_HISTORY).expect("table");
        let k = t.first().expect("first").expect("row").0.value().to_vec();
        t.remove(k.as_slice()).expect("remove");
    }
    txn.commit().expect("commit");
    let st = Store::from_db(db).expect("store");
    assert!(
        st.read()
            .expect("read")
            .check_balances()
            .expect("check")
            .is_some()
    );
}

#[test]
fn digest_covers_every_table() {
    let dir = tmp("digest");
    let (st, _) = ingested(&dir);
    let base = st.read().expect("read").digest().expect("digest");
    for def in store::ALL_TABLES {
        let txn = st.db.begin_write().expect("write");
        {
            let mut t = txn.open_table(*def).expect("table");
            t.insert(b"\xff\xffjunk".as_slice(), b"x".as_slice())
                .expect("insert");
        }
        txn.commit().expect("commit");
        let d = st.read().expect("read").digest().expect("digest");
        assert_ne!(
            d,
            base,
            "a row added to {} leaves the digest unchanged",
            def.name()
        );
        let txn = st.db.begin_write().expect("write");
        {
            let mut t = txn.open_table(*def).expect("table");
            t.remove(b"\xff\xffjunk".as_slice()).expect("remove");
        }
        txn.commit().expect("commit");
    }
    assert_eq!(st.read().expect("read").digest().expect("digest"), base);
}

#[test]
fn newer_or_missing_schema_version_is_refused() {
    let dir = tmp("version");
    let path = dir.join("v.redb");
    drop(Store::create(&path).expect("create"));
    assert!(Store::open(&path).is_ok());
    let set = |v: Option<u32>| {
        let db = Database::open(&path).expect("open");
        let txn = db.begin_write().expect("write");
        {
            let mut t = txn.open_table(store::META).expect("meta");
            match v {
                Some(v) => {
                    t.insert(b"version".as_slice(), v.to_be_bytes().as_slice())
                        .expect("insert");
                }
                None => {
                    t.remove(b"version".as_slice()).expect("remove");
                }
            }
        }
        txn.commit().expect("commit");
    };
    set(Some(store::SCHEMA_VERSION + 1));
    match Store::open(&path) {
        Err(store::Error::Version { found, supported }) => {
            assert_eq!(found, Some(store::SCHEMA_VERSION + 1));
            assert_eq!(supported, store::SCHEMA_VERSION);
        }
        other => panic!("newer version opened: {:?}", other.err()),
    }
    set(None);
    assert!(matches!(
        Store::open(&path),
        Err(store::Error::Version { found: None, .. })
    ));
}

fn spike(args: &[&str]) {
    let status = Command::new(env!("CARGO_BIN_EXE_spike"))
        .args(args)
        .status()
        .expect("run spike");
    assert!(status.success(), "spike {args:?} failed");
}

#[test]
fn crosscheck_against_dcrwallet() {
    if purse_testutil::oracle_bin_or_skip("storebench").is_none() {
        return;
    }
    let dir = tmp("crosscheck");
    let d = dir.to_str().expect("utf-8 path");
    spike(&["run", "--dir", d, "--preset", "tiny", "--no-bench"]);
    let results: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!(
            "tiny/redb-{}/results.json",
            store_redb_spike::REDB_VERSION
        )))
        .expect("results"),
    )
    .expect("json");
    assert_eq!(
        results["crosscheck"]["identical"],
        Value::Bool(true),
        "{}",
        results["crosscheck"]
    );
    assert!(results["crosscheck"]["rust_lines"].as_u64() > Some(1000));
    // The compared export must reach what it claims to compare: immature
    // coinbase and stake generation at the final tip, invalidated rows,
    // the outpoint walk, selection and the address order.
    let export = std::fs::read_to_string(dir.join(format!(
        "tiny/redb-{}/rust-export.txt",
        store_redb_spike::REDB_VERSION
    )))
    .expect("export");
    let col = |line: &str, i: usize| -> i64 {
        line.split(' ')
            .nth(i)
            .and_then(|x| x.parse().ok())
            .unwrap_or(0)
    };
    let bal1: Vec<&str> = export
        .lines()
        .filter(|l| l.starts_with("balance 1 "))
        .collect();
    assert!(
        bal1.iter().any(|l| col(l, 4) > 0),
        "no immature coinbase at the tip"
    );
    assert!(
        bal1.iter().any(|l| col(l, 5) > 0),
        "no immature stake at the tip"
    );
    for kind in [
        "invalid ",
        "inv_credit ",
        "inv_debit ",
        "fe ",
        "select ",
        "sel ",
        "addrs ",
    ] {
        assert!(
            export.lines().any(|l| l.starts_with(kind)),
            "no {kind}lines"
        );
    }
}

#[test]
fn bolt_cursor_matches_bbolt_itself() {
    let Some(bin) = purse_testutil::oracle_bin_or_skip("storebench") else {
        return;
    };
    const T: TableDefinition<K, K> = TableDefinition::new("t");
    let dir = tmp("cursor-oracle");
    let mut go = purse_testutil::Oracle::spawn_at(&bin);
    let mut rng = SplitMix64::from_env("bolt_cursor_matches_bbolt_itself");
    for (round, n) in [1usize, 2, 3, 5, 40, 300].into_iter().enumerate() {
        let mut keys: Vec<Vec<u8>> = (0..n)
            .map(|_| {
                let len = 1 + rng.below(3) as usize;
                rng.bytes(len)
            })
            .collect();
        keys.sort();
        keys.dedup();
        // Walks from every key and its neighbours, the ends and random
        // keys, with step sequences that run off both ends.
        let mut seeks: Vec<Vec<u8>> = vec![Vec::new(), vec![0], vec![0xff; 4]];
        for k in &keys {
            seeks.push(k.clone());
            let mut k2 = k.clone();
            k2.push(0);
            seeks.push(k2);
        }
        for _ in 0..50 {
            let len = rng.below(4) as usize;
            seeks.push(rng.bytes(len));
        }
        let walks: Vec<(Vec<u8>, String)> = seeks
            .into_iter()
            .map(|k| {
                let steps: String = (0..8)
                    .map(|_| if rng.below(2) == 0 { 'n' } else { 'p' })
                    .collect();
                (k, steps)
            })
            .chain(keys.iter().map(|k| (k.clone(), "nnnppp".to_owned())))
            .chain(keys.iter().map(|k| (k.clone(), "pppnnn".to_owned())))
            .collect();
        let gdb = dir.join(format!("c{round}.db"));
        let r = go.call(
            "cursor",
            serde_json::json!({
                "db": gdb,
                "keys": keys.iter().map(|k| purse_testutil::hex(k)).collect::<Vec<_>>(),
                "walks": walks.iter().map(|(k, s)| serde_json::json!({"seek": purse_testutil::hex(k), "steps": s})).collect::<Vec<_>>(),
            }),
        );
        let db = Database::create(dir.join(format!("c{round}.redb"))).expect("create");
        let txn = db.begin_write().expect("write");
        {
            let mut t = txn.open_table(T).expect("table");
            for k in &keys {
                t.insert(k.as_slice(), [].as_slice()).expect("insert");
            }
        }
        txn.commit().expect("commit");
        let rtxn = db.begin_read().expect("read");
        let t = rtxn.open_table(T).expect("table");
        let dash =
            |e: Option<(Vec<u8>, Vec<u8>)>| e.map_or("-".to_owned(), |e| purse_testutil::hex(&e.0));
        for (i, (k, steps)) in walks.iter().enumerate() {
            let mut c = BoltCursor::new(&t);
            let mut got = vec![dash(c.seek(k).expect("seek"))];
            for s in steps.chars() {
                let e = if s == 'n' { c.next() } else { c.prev() };
                got.push(dash(e.expect("step")));
            }
            let want: Vec<String> = r["walks"][i]
                .as_array()
                .expect("walk")
                .iter()
                .map(|x| x.as_str().expect("key").to_owned())
                .collect();
            assert_eq!(
                got,
                want,
                "walk {i} (seek {}, {steps}) over {n} keys",
                purse_testutil::hex(k)
            );
        }
    }
}

#[test]
fn power_loss_sweep_lands_on_a_durable_commit() {
    let dir = tmp("powerloss");
    let d = dir.to_str().expect("utf-8 path");
    for extra in [&[][..], &["--torn"][..], &["--relaxed", "10"][..]] {
        let mut args = vec!["powerloss", "--dir", d, "--step", "97"];
        args.extend_from_slice(extra);
        spike(&args);
    }
    for name in [
        "powerloss.json",
        "powerloss-torn.json",
        "powerloss-relaxed10.json",
    ] {
        let report: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("powerloss").join(name)).expect("report"),
        )
        .expect("json");
        assert!(report["trials"].as_u64() > Some(20), "{report}");
        for k in [
            "lost_durable_commits",
            "not_at_boundary",
            "digest_mismatch",
            "inconsistent",
            "reopen_failed",
        ] {
            assert_eq!(report[k], Value::from(0), "{name}: {report}");
        }
    }
    // A backend that acknowledges syncs without making anything durable
    // must be caught.
    spike(&["powerloss", "--dir", d, "--step", "397", "--dishonest"]);
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("powerloss/powerloss-dishonest.json")).expect("report"),
    )
    .expect("json");
    assert!(
        report["lost_durable_commits"].as_u64() > Some(0),
        "{report}"
    );
}

#[test]
fn killed_writer_leaves_a_committed_state() {
    let dir = tmp("crash");
    let d = dir.to_str().expect("utf-8 path");
    spike(&["crash", "--dir", d, "--kills", "12"]);
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("crash/crash.json")).expect("report"),
    )
    .expect("json");
    assert_eq!(report["kills"], Value::from(12));
    let split = ["kills_before_open", "kills_in_first_txn", "kills_after_ack"]
        .iter()
        .map(|k| report[*k].as_u64().unwrap_or(0))
        .sum::<u64>();
    assert_eq!(split, 12, "{report}");
}
