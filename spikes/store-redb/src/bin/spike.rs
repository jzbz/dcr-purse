//! The spike's driver.
//!
//! ```text
//! spike gen    --dir D --preset P            write D/workload-P.bin
//! spike run    --dir D --preset P [--no-go] [--no-bench] [--reuse-go]
//!                                            ingest, cross-check and time
//!                                            both stores; results in
//!                                            D/P/redb-<version>/
//! spike crash  --dir D [--kills N] [--preset tiny]
//!                                            kill the writer at random
//!                                            points and check every reopen
//! spike repair-time --db PATH [--quick-repair]
//!                                            time a reopen after a kill on
//!                                            an existing store, and what
//!                                            quick repair costs a commit
//! spike powerloss --dir D [--step N] [--torn] [--dishonest] [--relaxed K]
//!                                            cut power at every Nth storage
//!                                            operation of the tiny workload
//!                                            and check every reopen
//! spike backup --db R [--go-db G] [--reps N]
//!                                            hot backups of existing stores,
//!                                            the two sides alternating
//! spike headers --dir D [--n N] [--batch B] [--filter-len L] [--relaxed K]
//!                                            header-plus-filter ingest on
//!                                            both sides
//! ```
//!
//! `crash-writer` and `churn` are the children `crash` and `repair-time`
//! spawn.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{Oracle, OracleUnavailable, SplitMix64, hex};
use store_redb_spike::bench::{self, BenchArgs};
use store_redb_spike::chain;
use store_redb_spike::export::{self, ByAccountSpec, ExportSpec, RangeSpec};
use store_redb_spike::powerloss::{PowerLoss, Shared};
use store_redb_spike::store::{self, Counters, ParsedOp, Store};
use store_redb_spike::workload::{self, Params};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else {
        eprintln!("usage: spike gen|run|crash|repair-time [options]");
        std::process::exit(2);
    };
    let opts = parse_opts(&args[1..]);
    let r = match cmd.as_str() {
        "gen" => cmd_gen(&opts).map(|_| ()),
        "run" => cmd_run(&opts),
        "crash" => cmd_crash(&opts),
        "crash-writer" => cmd_crash_writer(&opts),
        "repair-time" => cmd_repair_time(&opts),
        "powerloss" => cmd_powerloss(&opts),
        "headers" => cmd_headers(&opts),
        "backup" => cmd_backup(&opts),
        "churn" => cmd_churn(&opts),
        _ => Err(format!("unknown command {cmd}").into()),
    };
    if let Err(e) = r {
        eprintln!("spike {cmd}: {e}");
        std::process::exit(1);
    }
}

fn parse_opts(args: &[String]) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args[i].trim_start_matches("--").to_owned();
        match args.get(i + 1) {
            Some(v) if !v.starts_with("--") => {
                m.insert(k, v.clone());
                i += 2;
            }
            _ => {
                m.insert(k, String::new());
                i += 1;
            }
        }
    }
    m
}

fn opt<'a>(o: &'a BTreeMap<String, String>, k: &str) -> Res<&'a str> {
    o.get(k)
        .map(String::as_str)
        .ok_or_else(|| format!("--{k} is required").into())
}

fn preset(o: &BTreeMap<String, String>, default: &str) -> Res<(String, Params)> {
    let name = o.get("preset").map_or(default, String::as_str).to_owned();
    let p = Params::preset(&name).ok_or_else(|| format!("unknown preset {name}"))?;
    Ok((name, p))
}

/// Generates the preset's workload unless the file exists; returns its path.
fn cmd_gen(o: &BTreeMap<String, String>) -> Res<PathBuf> {
    let dir = PathBuf::from(opt(o, "dir")?);
    let (name, p) = preset(o, "10k")?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("workload-{name}.bin"));
    if path.exists() {
        return Ok(path);
    }
    let t = Instant::now();
    let (w, stats) = workload::generate(&p);
    let tmp = path.with_extension("tmp");
    workload::write(&w, &tmp)?;
    std::fs::rename(&tmp, &path)?;
    let blocks = w.ops.iter().filter(|o| o.header.is_some()).count();
    let txs: usize = w.ops.iter().map(|o| o.txs.len()).sum();
    let info = json!({
        "preset": name, "seed": p.seed, "target_txs": p.txs, "blocks": blocks,
        "accounts": p.accounts, "tail_blocks": p.tail_blocks, "batch_blocks": p.batch_blocks,
        "ops": w.ops.len(), "tx_entries": txs,
        "receives": stats.receives, "sends": stats.sends,
        "consolidations": stats.consolidations, "tickets": stats.tickets,
        "votes": stats.votes, "coinbases": stats.coinbases,
        "unmined_first": stats.unmined_first, "unmined_at_end": stats.unmined_at_end,
        "addresses": stats.addresses,
        "cross_account": stats.cross_account, "no_change": stats.no_change,
        "disapprovals": stats.disapprovals, "invalidated": stats.invalidated,
        "remined": stats.remined, "dropped": stats.dropped, "reorgs": stats.reorgs,
        "delayed": stats.delayed,
        "file_size": std::fs::metadata(&path)?.len(),
        "generate_seconds": t.elapsed().as_secs_f64(),
    });
    std::fs::write(
        dir.join(format!("workload-{name}.json")),
        serde_json::to_string_pretty(&info)?,
    )?;
    eprintln!("generated {}: {info}", path.display());
    Ok(path)
}

fn parse_ops(w: &workload::Workload) -> Res<Vec<ParsedOp>> {
    Ok(w.ops
        .iter()
        .map(ParsedOp::parse)
        .collect::<Result<Vec<_>, _>>()?)
}

/// The probe, scan and range sets for the cross-check, from the ingested
/// Rust store (the same sets go to Go).
fn export_spec(st: &Store, seed: u64, accounts: u32, small: bool) -> Res<ExportSpec> {
    let r = st.read()?;
    let tip = r.tip()?.0 as i32;
    let mut rng = SplitMix64(seed ^ 0x9e0be);
    let n = r.unspent_n()?;
    let step = (n / 50).max(1);
    let mut existing: Vec<Vec<u8>> = Vec::new();
    use redb::ReadableTable as _;
    for (i, e) in r.unspent()?.iter()?.enumerate() {
        if (i as u64).is_multiple_of(step) {
            existing.push(e?.0.value().to_vec());
        }
    }
    let mut probes = vec![String::new(), "00".repeat(36), "ff".repeat(36)];
    // The keys at both ends, so the walks run off each end at every step
    // count: from the last key `next` returns nothing at once (bbolt then
    // stays on the last element, and `prev` must return the second to
    // last), from the second and third to last at the second and third
    // step.
    let all_keys: Vec<Vec<u8>> = {
        let t = r.unspent()?;
        let mut v = Vec::new();
        for e in t.iter()?.take(3) {
            v.push(e?.0.value().to_vec());
        }
        let mut back = Vec::new();
        for e in t.iter()?.rev().take(3) {
            back.push(e?.0.value().to_vec());
        }
        v.extend(back);
        v
    };
    for k in &all_keys {
        probes.push(hex(k));
    }
    for _ in 0..100 {
        probes.push(hex(&rng.bytes(36)));
    }
    for k in &existing {
        probes.push(hex(k));
        let idx = u32::from_be_bytes(k[32..36].try_into()?);
        let mut k2 = k[..32].to_vec();
        k2.extend_from_slice(&idx.wrapping_add(1).to_be_bytes());
        probes.push(hex(&k2));
        probes.push(hex(&k[..32]));
        let mut k3 = k[..32].to_vec();
        k3.extend_from_slice(&[0xff; 4]);
        probes.push(hex(&k3));
    }
    let mut scans = vec!["00".repeat(32), "ff".repeat(32)];
    for _ in 0..100 {
        scans.push(hex(&rng.bytes(32)));
    }
    for k in existing.iter().take(20) {
        scans.push(hex(&k[..32]));
    }
    let mut ranges = vec![
        rs("recent200blocks", tip, tip - 200, 0),
        rs("mid500fwd", tip / 3, tip / 3 + 500, 0),
        rs("unmined_then_down", -1, tip - 50, 0),
        rs("up_then_unmined", tip - 30, -1, 0),
        rs("single", tip - 5, tip - 5, 0),
        rs("recent100tx", tip, 0, 100),
        rs("unmined_only", -1, -1, 0),
        rs("beyond_tip", tip + 100, tip - 10, 0),
    ];
    if small {
        ranges.push(rs("all_fwd", 0, tip, 0));
        ranges.push(rs("all_back", tip, 0, 0));
    }
    // The whole per-account history at the smaller sizes (the debit-only
    // transactions of an account sit anywhere in it), the 500 most recent
    // at 1m.
    let by_account = (0..accounts)
        .map(|a| ByAccountSpec {
            name: format!("acct{a}"),
            account: a,
            limit: if small { 1 << 30 } else { 500 },
        })
        .collect();
    Ok(ExportSpec {
        minconfs: vec![0, 1, 2, 6, 15, 16, 17, 100, 100_000],
        probes,
        scans,
        ranges,
        by_account,
        select_accounts: accounts,
        select_detail: small,
    })
}

fn rs(name: &str, begin: i32, end: i32, limit: usize) -> RangeSpec {
    RangeSpec {
        name: name.into(),
        begin,
        end,
        limit,
    }
}

fn spec_json(db: &Path, out: &Path, s: &ExportSpec) -> Value {
    json!({
        "db": db, "out": out, "minconfs": s.minconfs, "probes": s.probes, "scans": s.scans,
        "ranges": s.ranges.iter().map(|r| json!({"name": r.name, "begin": r.begin,
            "end": r.end, "limit": r.limit})).collect::<Vec<_>>(),
        "by_account": s.by_account.iter().map(|b| json!({"name": b.name,
            "account": b.account, "limit": b.limit})).collect::<Vec<_>>(),
        "select_accounts": s.select_accounts, "select_detail": s.select_detail,
        "select_minconfs": export::SELECT_MINCONFS, "select_targets": export::SELECT_TARGETS,
    })
}

fn bench_args(w: &workload::Workload, tip: i32, accounts: u32, seed: u64) -> BenchArgs {
    let last = accounts - 1;
    let mut rng = SplitMix64(seed ^ 0x5c41);
    let mut scripts = Vec::new();
    let all: Vec<&[u8; 20]> = w.ranges.iter().flat_map(|r| r.hashes.iter()).collect();
    for _ in 0..500 {
        let h = all[rng.below(all.len() as u64) as usize];
        scripts.push(hex(&chain::p2pkh(h)));
        let mut f = [0u8; 20];
        rng.fill(&mut f);
        scripts.push(hex(&chain::p2pkh(&f)));
    }
    BenchArgs {
        min_reps: 5,
        max_reps: 200,
        budget_ms: 1000,
        // The wallet's usual depths, and deep ones: the balance query reads
        // per-height rows over the shorter side of the depth window, so its
        // cost peaks near half the chain's height.
        minconfs: vec![0, 1, 6, 100, tip / 2, 100_000],
        accounts: vec![0, 1, last],
        targets: vec![1_000_000, 100_000_000, 10_000_000_000],
        ranges: vec![
            rs("recent100tx", tip, 0, 100),
            rs("last1000blocks", tip, tip - 1000, 0),
            rs("mid1000blocks_fwd", tip / 2, tip / 2 + 999, 0),
            rs("unmined", -1, -1, 0),
            rs("all", 0, tip, 0),
        ],
        by_account: vec![
            ByAccountSpec {
                name: "acct0_recent100".into(),
                account: 0,
                limit: 100,
            },
            ByAccountSpec {
                name: format!("acct{last}_recent100"),
                account: last,
                limit: 100,
            },
        ],
        scripts,
        next_addr: 30,
        select_reps: 50,
    }
}

fn spawn_go() -> Res<Option<Oracle>> {
    match purse_testutil::build_oracle("storebench") {
        Ok(bin) => Ok(Some(Oracle::spawn_at(&bin))),
        Err(OracleUnavailable::Missing(why)) => {
            eprintln!("no Go baseline: {why}");
            Ok(None)
        }
        Err(OracleUnavailable::Broken(why)) => Err(why.into()),
    }
}

fn opens(n: usize, path: &Path, mut f: impl FnMut() -> Res<Value>) -> Res<Value> {
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for _ in 0..n {
        bench::drop_cache(path);
        cold.push(f()?);
    }
    for _ in 0..n {
        warm.push(f()?);
    }
    Ok(json!({"cold": cold, "warm": warm}))
}

fn cmd_run(o: &BTreeMap<String, String>) -> Res<()> {
    let dir = PathBuf::from(opt(o, "dir")?);
    let (name, p) = preset(o, "10k")?;
    let wl_path = cmd_gen(o)?;
    // The Go wallet lives in <dir>/<preset>/go and, with --reuse-go, is
    // kept from an earlier run (its setup and ingest results are kept
    // beside it); everything of the prototype's goes to a directory named
    // for the redb release.
    let base = dir.join(&name);
    let out = base.join(format!("redb-{}", store_redb_spike::REDB_VERSION));
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    let go_dir = base.join("go");
    let reuse_go = o.contains_key("reuse-go") && go_dir.join("ingest.json").exists();
    if !reuse_go && go_dir.exists() {
        std::fs::remove_dir_all(&go_dir)?;
    }
    std::fs::create_dir_all(&go_dir)?;
    std::fs::create_dir_all(out.join("rust"))?;
    let w = workload::read(&wl_path)?;
    let wseed = workload::wallet_seed(p.seed);
    let t = Instant::now();
    let ops = parse_ops(&w)?;
    eprintln!("[{name}] parsed {} ops in {:?}", ops.len(), t.elapsed());
    let mut results = json!({"preset": name, "workload": wl_path});

    // Both sides: create and record the addresses once, keep a copy of
    // each store as setup left it, then ingest from those copies, Go and
    // Rust alternating, `--rounds` times (default 1), so the ingest rows
    // pair runs of one session.
    let rounds: usize = o.get("rounds").map_or(Ok(1), |s| s.parse())?;
    let rs_db = out.join("rust/wallet.redb");
    let rs_setup = out.join("rust/setup.redb");
    let t = Instant::now();
    let st = Store::create(&rs_setup)?;
    st.setup(w.accounts, &w.ranges)?;
    drop(st);
    results["rust_setup_ns"] = json!(t.elapsed().as_nanos() as u64);
    eprintln!("[{name}] rust setup {:?}", t.elapsed());
    let mut go = spawn_go()?;
    let with_go = go.is_some() && !o.contains_key("no-go");
    let go_db = go_dir.join("wallet.db");
    let go_setup = go_dir.join("setup.db");
    let go_export = out.join("go-export.txt");
    if let Some(go) = go.as_mut().filter(|_| with_go && !reuse_go) {
        let r = go.call(
            "setup",
            json!({"db": go_setup, "workload": wl_path, "seed": hex(&wseed)}),
        );
        eprintln!("[{name}] go setup {r}");
        if r["mismatches"].as_u64() != Some(0) {
            return Err(format!("Go derived different addresses: {r}").into());
        }
        if r["genesis"].as_str() != Some(hex(&chain::SIMNET_GENESIS).as_str()) {
            return Err(format!("genesis mismatch: {r}").into());
        }
        results["go_setup"] = r;
    }
    let mut rust_rounds = Vec::new();
    let mut go_rounds = Vec::new();
    for round in 0..rounds {
        if let Some(go) = go.as_mut().filter(|_| with_go && !reuse_go) {
            let _ = std::fs::remove_file(&go_db);
            std::fs::copy(&go_setup, &go_db)?;
            let r = go.call(
                "ingest",
                json!({"db": go_db, "workload": wl_path, "gap": chain::GAP_LIMIT}),
            );
            eprintln!("[{name}] go ingest {round} {r}");
            go_rounds.push(r);
            results["go_size_after_ingest"] = json!(std::fs::metadata(&go_db)?.len());
        }
        let _ = std::fs::remove_file(&rs_db);
        std::fs::copy(&rs_setup, &rs_db)?;
        let st = Store::open(&rs_db)?;
        let ing = bench::ingest(&st, &ops)?;
        drop(st);
        eprintln!("[{name}] rust ingest {round} {ing}");
        rust_rounds.push(ing);
        results["rust_size_after_ingest"] = json!(std::fs::metadata(&rs_db)?.len());
    }
    let median = |v: &[Value]| -> Value {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by_key(|&i| v[i]["elapsed"].as_u64().unwrap_or(0));
        idx.get(idx.len() / 2)
            .map_or(Value::Null, |&i| v[i].clone())
    };
    results["rust_ingest"] = median(&rust_rounds);
    results["rust_ingest_rounds"] = json!(rust_rounds);
    if with_go && reuse_go {
        let kept: Value =
            serde_json::from_str(&std::fs::read_to_string(go_dir.join("ingest.json"))?)?;
        for k in [
            "go_setup",
            "go_ingest",
            "go_ingest_rounds",
            "go_size_after_ingest",
        ] {
            results[k] = kept[k].clone();
        }
        results["go_reused"] = json!(true);
        eprintln!("[{name}] reusing the Go wallet of an earlier run");
    } else if with_go {
        results["go_ingest"] = median(&go_rounds);
        results["go_ingest_rounds"] = json!(go_rounds);
        let kept = json!({"go_setup": results["go_setup"], "go_ingest": results["go_ingest"],
            "go_ingest_rounds": results["go_ingest_rounds"],
            "go_size_after_ingest": results["go_size_after_ingest"]});
        std::fs::write(
            go_dir.join("ingest.json"),
            serde_json::to_string_pretty(&kept)?,
        )?;
    }
    let _ = std::fs::remove_file(&rs_setup);

    let st = Store::open(&rs_db)?;
    let tip = st.read()?.tip()?.0 as i32;
    let small = p.txs <= 100_000;
    let spec = export_spec(&st, p.seed, w.accounts, small)?;
    let t = Instant::now();
    if let Some(m) = st.read()?.check_balances()? {
        return Err(format!("rust consistency check: {m}").into());
    }
    results["rust_check_ns"] = json!(t.elapsed().as_nanos() as u64);
    let t = Instant::now();
    let lines = export::export(&st.read()?, &spec)?;
    results["rust_export_ns"] = json!(t.elapsed().as_nanos() as u64);
    let rust_export = out.join("rust-export.txt");
    std::fs::write(&rust_export, lines.join("\n") + "\n")?;
    drop(st);

    if let Some(go) = go.as_mut().filter(|_| with_go) {
        let t = Instant::now();
        let r = go.call("export", spec_json(&go_db, &go_export, &spec));
        results["go_export_ns"] = json!(t.elapsed().as_nanos() as u64);
        results["go_export"] = r;
        let a = std::fs::read_to_string(&go_export)?;
        let b = std::fs::read_to_string(&rust_export)?;
        let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
        let first = al
            .iter()
            .zip(bl.iter())
            .position(|(x, y)| x != y)
            .or((al.len() != bl.len()).then_some(al.len().min(bl.len())));
        let sections = |ls: &[&str]| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            for l in ls {
                *m.entry(l.split(' ').next().unwrap_or("").to_owned())
                    .or_default() += 1;
            }
            m
        };
        results["crosscheck"] = json!({
            "identical": first.is_none(),
            "go_lines": al.len(), "rust_lines": bl.len(),
            "line_kinds": sections(&bl),
            "first_difference": first.map(|i| json!({"line": i + 1,
                "go": al.get(i), "rust": bl.get(i)})),
        });
        eprintln!("[{name}] crosscheck {}", results["crosscheck"]);
    }

    if o.contains_key("no-bench") {
        std::fs::write(
            out.join("results.json"),
            serde_json::to_string_pretty(&results)?,
        )?;
        return Ok(());
    }

    // Timed queries: Rust, then Go, never at the same time.
    let ba = bench_args(&w, tip, w.accounts, p.seed);
    let t = Instant::now();
    results["rust_bench"] = bench::bench(&rs_db, &ba, &wseed)?;
    eprintln!("[{name}] rust bench {:?}", t.elapsed());
    results["rust_open"] = opens(3, &rs_db, || Ok(bench::open_once(&rs_db)?))?;
    if let Some(go) = go.as_mut()
        && !o.contains_key("no-go")
    {
        let t = Instant::now();
        results["go_bench"] = go.call("bench", ba.go_json(&go_db));
        eprintln!("[{name}] go bench {:?}", t.elapsed());
        results["go_open"] = opens(3, &go_db, || Ok(go.call("open", json!({"db": go_db}))))?;
        results["go_file_size"] = json!(std::fs::metadata(&go_db)?.len());
    }
    results["rust_file_size"] = json!(std::fs::metadata(&rs_db)?.len());
    results["rust_redb_stats"] = bench::redb_stats(&rs_db)?;
    results["rust_compacted"] = bench::compacted_size(&rs_db, &out)?;
    results["rust_hot_backup"] = bench::hot_backup(&rs_db, &out.join("backup.redb"), false)?;
    results["rust_hot_backup_writer"] = bench::hot_backup(&rs_db, &out.join("backup.redb"), true)?;
    eprintln!(
        "[{name}] rust stats {} compacted {} backup {}",
        results["rust_redb_stats"], results["rust_compacted"], results["rust_hot_backup"]
    );
    if let Some(go) = go.as_mut()
        && !o.contains_key("no-go")
    {
        let bk = go_dir.join("backup.db");
        results["go_hot_backup"] = go.call("backup", json!({"db": go_db, "out": bk}));
        let _ = std::fs::remove_file(&bk);
        results["go_hot_backup_writer"] =
            go.call("backup", json!({"db": go_db, "out": bk, "writer": true}));
        let _ = std::fs::remove_file(&bk);
        eprintln!("[{name}] go backup {}", results["go_hot_backup"]);
    }
    results["host"] = host_info();
    results["redb"] = json!(store_redb_spike::REDB_VERSION);
    std::fs::write(
        out.join("results.json"),
        serde_json::to_string_pretty(&results)?,
    )?;
    let md = summary_md(&results);
    std::fs::write(out.join("summary.md"), &md)?;
    println!("{md}");
    Ok(())
}

fn host_info() -> Value {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        })
        .unwrap_or_default();
    json!({"cpu": cpu, "os": std::env::consts::OS, "arch": std::env::consts::ARCH})
}

fn ns(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn fmt_ns(x: f64) -> String {
    if x.is_nan() {
        "-".into()
    } else if x >= 1e9 {
        format!("{:.2} s", x / 1e9)
    } else if x >= 1e6 {
        format!("{:.2} ms", x / 1e6)
    } else if x >= 1e3 {
        format!("{:.1} µs", x / 1e3)
    } else {
        format!("{x:.0} ns")
    }
}

fn ratio(go: f64, rust: f64) -> String {
    if go.is_nan() || rust.is_nan() || rust == 0.0 {
        "-".into()
    } else {
        format!("{:.1}x", go / rust)
    }
}

fn summary_md(r: &Value) -> String {
    let mut s = String::new();
    let p = r["preset"].as_str().unwrap_or("");
    s += &format!(
        "## {p}\n\n| measurement | dcrwallet (bbolt) | prototype (redb) | Go/Rust |\n|---|---:|---:|---:|\n"
    );
    let gi = &r["go_ingest"];
    let ri = &r["rust_ingest"];
    let per_block = |i: &Value| ns(&i["sync_ns"]) / ns(&i["sync_blocks"]);
    s += &format!(
        "| ingest, history sync, per block (batched) | {} | {} | {} |\n",
        fmt_ns(per_block(gi)),
        fmt_ns(per_block(ri)),
        ratio(per_block(gi), per_block(ri))
    );
    for (label, k) in [
        ("ingest, tip block, p50", "tip_block"),
        ("ingest, unmined insert, p50", "unmined"),
        (
            "ingest, reorganisation (rollback + new blocks), p50",
            "reorg",
        ),
    ] {
        let (g, rr) = (ns(&gi[k]["p50"]), ns(&ri[k]["p50"]));
        s += &format!(
            "| {label} | {} | {} | {} |\n",
            fmt_ns(g),
            fmt_ns(rr),
            ratio(g, rr)
        );
    }
    let (g, rr) = (ns(&gi["elapsed"]), ns(&ri["elapsed"]));
    s += &format!(
        "| ingest, whole workload | {} | {} | {} |\n",
        fmt_ns(g),
        fmt_ns(rr),
        ratio(g, rr)
    );
    s += &format!(
        "| (Go time inside address sync derivation) | {} | | |\n",
        fmt_ns(ns(&gi["addr_sync_ns"]))
    );
    let (g, rr) = (ns(&gi["io_write_bytes"]), ns(&ri["io_write_bytes"]));
    s += &format!(
        "| ingest, bytes written (write_bytes) | {:.1} MiB | {:.1} MiB | {} |\n",
        g / 1048576.0,
        rr / 1048576.0,
        ratio(g, rr)
    );
    let elapsed = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| a.iter().map(|x| fmt_ns(ns(&x["elapsed"]))).collect())
            .unwrap_or_default()
    };
    s += &format!(
        "| ingest rounds, whole workload (alternating, Go first) | {} | {} | |\n",
        elapsed(&r["go_ingest_rounds"]).join(", "),
        elapsed(&r["rust_ingest_rounds"]).join(", ")
    );
    let mut go_t: BTreeMap<String, f64> = BTreeMap::new();
    if let Some(a) = r["go_bench"]["timings"].as_array() {
        for t in a {
            go_t.insert(
                t["name"].as_str().unwrap_or("").into(),
                ns(&t["summary"]["p50"]),
            );
        }
    }
    if let Some(a) = r["rust_bench"]["timings"].as_array() {
        for t in a {
            let n = t["name"].as_str().unwrap_or("");
            let rr = ns(&t["summary"]["p50"]);
            let g = go_t.get(n).copied().unwrap_or(f64::NAN);
            s += &format!(
                "| {n} (p50) | {} | {} | {} |\n",
                fmt_ns(g),
                fmt_ns(rr),
                ratio(g, rr)
            );
        }
    }
    for (label, k) in [("open, cold", "cold"), ("open, warm", "warm")] {
        let med = |v: &Value, f: &str| {
            let mut x: Vec<f64> = v[k]
                .as_array()
                .map(|a| a.iter().map(|e| ns(&e[f])).collect())
                .unwrap_or_default();
            x.sort_by(f64::total_cmp);
            x.get(x.len() / 2).copied().unwrap_or(f64::NAN)
        };
        for f in ["open", "first_balance"] {
            let (g, rr) = (med(&r["go_open"], f), med(&r["rust_open"], f));
            s += &format!(
                "| {label}: {f} (median of 3) | {} | {} | {} |\n",
                fmt_ns(g),
                fmt_ns(rr),
                ratio(g, rr)
            );
        }
    }
    let (g, rr) = (ns(&r["go_file_size"]), ns(&r["rust_file_size"]));
    s += &format!(
        "| file size | {:.1} MiB | {:.1} MiB | {} |\n",
        g / 1048576.0,
        rr / 1048576.0,
        ratio(g, rr)
    );
    let (g, rr) = (
        ns(&r["go_hot_backup"]["elapsed"]),
        ns(&r["rust_hot_backup"]["elapsed"]),
    );
    s += &format!(
        "| hot backup (bbolt Tx.Copy / redb snapshot copy) | {} | {} | {} |\n",
        fmt_ns(g),
        fmt_ns(rr),
        ratio(g, rr)
    );
    let (g, rr) = (
        ns(&r["go_hot_backup_writer"]["elapsed"]),
        ns(&r["rust_hot_backup_writer"]["elapsed"]),
    );
    s += &format!(
        "| hot backup with a concurrent writer | {} ({} commits) | {} ({} commits) | |\n",
        fmt_ns(g),
        r["go_hot_backup_writer"]["writer_commits"],
        fmt_ns(rr),
        r["rust_hot_backup_writer"]["writer_commits"]
    );
    s += &format!(
        "| file size after redb compaction | | {:.1} MiB | |\n",
        ns(&r["rust_compacted"]["size"]) / 1048576.0
    );
    s += &format!("\ncross-check: {}\n", r["crosscheck"]);
    s
}

// ---------------------------------------------------------------------------
// Crash safety

fn cmd_crash(o: &BTreeMap<String, String>) -> Res<()> {
    let dir = PathBuf::from(opt(o, "dir")?).join("crash");
    let kills: usize = o.get("kills").map_or(Ok(300), |s| s.parse())?;
    let max_ms: u64 = o.get("max-ms").map_or(Ok(40), |s| s.parse())?;
    let (name, p) = preset(o, "tiny")?;
    std::fs::create_dir_all(&dir)?;
    let wl_path = dir.join(format!("workload-{name}.bin"));
    let (w, _) = workload::generate(&p);
    workload::write(&w, &wl_path)?;
    let ops = parse_ops(&w)?;
    let groups = bench::groups_of(&ops);

    // The reference: the digest at every transaction boundary.
    let ref_db = dir.join("reference.redb");
    let _ = std::fs::remove_file(&ref_db);
    let st = Store::create(&ref_db)?;
    st.setup(w.accounts, &w.ranges)?;
    let mut digests: BTreeMap<u64, [u8; 32]> = BTreeMap::new();
    digests.insert(0, st.read()?.digest()?);
    let mut c = Counters::default();
    for g in &groups {
        st.apply(&ops[g.clone()], &mut c)?;
        let r = st.read()?;
        digests.insert(r.last_op()?, r.digest()?);
    }
    drop(st);
    let total = ops.len() as u64;
    eprintln!(
        "crash: {} operations in {} transactions",
        total,
        groups.len()
    );

    let exe = std::env::current_exe()?;
    let victim = dir.join("victim.redb");
    let mut rng = SplitMix64(0xc4a54);
    let mut done = 0usize;
    let mut cycles = 0usize;
    let mut mid_commit = 0usize;
    let mut at_ack = 0usize;
    let mut reopen_ns = Vec::new();
    let (mut before_open, mut in_first_txn, mut after_ack) = (0usize, 0usize, 0usize);
    let mut fresh = true;
    while done < kills {
        if fresh {
            let _ = std::fs::remove_file(&victim);
            let st = Store::create(&victim)?;
            st.setup(w.accounts, &w.ranges)?;
            fresh = false;
            cycles += 1;
        }
        let mut child = Command::new(&exe)
            .args(["crash-writer", "--db"])
            .arg(&victim)
            .arg("--workload")
            .arg(&wl_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped");
        let (opened_tx, opened_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let (mut opened, mut acked) = (false, None);
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line == "opened" {
                    opened = true;
                    let _ = opened_tx.send(());
                } else if let Some(n) = line.strip_prefix("committed ") {
                    acked = n.parse::<u64>().ok();
                }
            }
            (opened, acked)
        });
        // One kill in four is timed from the spawn, so some land in
        // process start and in redb's open (its header write and fsync);
        // the rest from the moment the writer reports the store open, so
        // they land in ingest.
        let from_spawn = rng.below(4) == 0;
        if !from_spawn {
            let _ = opened_rx.recv_timeout(Duration::from_secs(10));
        }
        std::thread::sleep(Duration::from_micros(rng.below(max_ms * 1000)));
        let killed = child.try_wait()?.is_none();
        if killed {
            child.kill()?;
        }
        let status = child.wait()?;
        let (opened, acked) = reader.join().map_err(|_| "reader thread")?;
        let t = Instant::now();
        let st = Store::open(&victim)?;
        let r = st.read()?;
        reopen_ns.push(t.elapsed().as_nanos());
        let j = r.last_op()?;
        let Some(want) = digests.get(&j) else {
            return Err(format!("reopened at op {j}, not a transaction boundary").into());
        };
        if killed {
            match (opened, acked) {
                (false, _) => before_open += 1,
                (true, None) => in_first_txn += 1,
                (true, Some(_)) => after_ack += 1,
            }
        }
        if let Some(a) = acked {
            if j < a {
                return Err(format!("acknowledged op {a} lost: reopened at {j}").into());
            }
            if killed {
                if j == a {
                    at_ack += 1;
                } else {
                    mid_commit += 1;
                }
            }
        }
        if r.digest()? != *want {
            return Err(format!("state at op {j} differs from the reference").into());
        }
        if let Some(m) = r.check_balances()? {
            return Err(format!("consistency check failed at op {j}: {m}").into());
        }
        drop(r);
        drop(st);
        if killed {
            done += 1;
        } else if !status.success() {
            return Err(format!("writer failed: {status}").into());
        }
        if j == total {
            fresh = true;
        }
    }
    let s = bench::Summary::of(&reopen_ns);
    let report = json!({
        "kills": done, "cycles": cycles, "operations": total, "transactions": groups.len(),
        "kills_before_open": before_open, "kills_in_first_txn": in_first_txn,
        "kills_after_ack": after_ack, "kills_after_open": in_first_txn + after_ack,
        "reopened_at_last_ack": at_ack, "reopened_past_last_ack": mid_commit,
        "reopen": s.json(), "max_ms": max_ms,
    });
    std::fs::write(
        dir.join("crash.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}

/// The writer `crash` kills: applies the workload's remaining
/// transactions, printing each one's last operation once it is committed.
fn cmd_crash_writer(o: &BTreeMap<String, String>) -> Res<()> {
    let db = PathBuf::from(opt(o, "db")?);
    let w = workload::read(Path::new(opt(o, "workload")?))?;
    let ops = parse_ops(&w)?;
    let st = Store::open(&db)?;
    let start = st.read()?.last_op()?;
    let mut c = Counters::default();
    let mut out = std::io::stdout().lock();
    writeln!(out, "opened")?;
    out.flush()?;
    for g in bench::groups_of(&ops) {
        if (g.end as u64) <= start {
            continue;
        }
        st.apply(&ops[g.clone()], &mut c)?;
        writeln!(out, "committed {}", g.end)?;
        out.flush()?;
    }
    Ok(())
}

/// Time a reopen of an existing store after killing a writer on it.
fn cmd_repair_time(o: &BTreeMap<String, String>) -> Res<()> {
    let db = PathBuf::from(opt(o, "db")?);
    let quick = o.contains_key("quick-repair");
    let t = Instant::now();
    let st = Store::open(&db)?;
    let clean = t.elapsed();
    // What quick repair costs a commit: 100 one-address commits with it
    // off, then 100 with it on (redb saves the allocator state in each).
    let mut commit_ns = BTreeMap::new();
    let mut i = 0x5000_0000u32;
    for q in [false, true] {
        let mut d = Vec::new();
        for _ in 0..100 {
            let t = Instant::now();
            churn_commit(&st, i, q)?;
            d.push(t.elapsed().as_nanos());
            i += 1;
        }
        commit_ns.insert(
            if q { "quick_repair" } else { "default" },
            bench::Summary::of(&d).json(),
        );
    }
    drop(st);
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(&exe);
    cmd.args(["churn", "--db"]).arg(&db);
    if quick {
        cmd.arg("--quick-repair");
    }
    let mut child = cmd.stdout(Stdio::null()).spawn()?;
    std::thread::sleep(Duration::from_millis(2_000));
    child.kill()?;
    child.wait()?;
    let t = Instant::now();
    let st = Store::open(&db)?;
    let repair = t.elapsed();
    let t = Instant::now();
    let check = st.read()?.check_balances()?;
    let report = json!({
        "db": db, "size": std::fs::metadata(&db)?.len(), "quick_repair": quick,
        "clean_open_ns": clean.as_nanos() as u64,
        "commit_ns": commit_ns,
        "open_after_kill_ns": repair.as_nanos() as u64,
        "consistency_check_ns": t.elapsed().as_nanos() as u64,
        "consistency": check.unwrap_or_else(|| "holds".into()),
    });
    println!("{report}");
    Ok(())
}

/// One write transaction recording a throwaway address in account 0's
/// internal branch beyond anything the workload uses.
fn churn_commit(st: &Store, i: u32, quick_repair: bool) -> Res<()> {
    let mut txn = st.db.begin_write()?;
    txn.set_quick_repair(quick_repair);
    {
        let mut w = store::W::open(&txn)?;
        let mut h = [0u8; 20];
        h[..4].copy_from_slice(&i.to_be_bytes());
        w.record_address(0, 1, i, &h)?;
    }
    txn.commit()?;
    Ok(())
}

/// A writer that never stops, one [`churn_commit`] after another.
fn cmd_churn(o: &BTreeMap<String, String>) -> Res<()> {
    let db = PathBuf::from(opt(o, "db")?);
    let quick = o.contains_key("quick-repair");
    let st = Store::open(&db)?;
    let mut i = 0x4000_0000u32;
    loop {
        churn_commit(&st, i, quick)?;
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Power loss

/// What a reopen after a power cut found.
#[derive(Default)]
struct Outcomes {
    trials: u64,
    cut_reached: u64,
    at_last_durable_ack: u64,
    past_last_durable_ack: u64,
    lost_nondurable: u64,
    lost_durable: u64,
    not_boundary: u64,
    digest_mismatch: u64,
    inconsistent: u64,
    open_failed: u64,
}

fn cmd_powerloss(o: &BTreeMap<String, String>) -> Res<()> {
    use redb::{Database, Durability};
    let dir = PathBuf::from(opt(o, "dir")?).join("powerloss");
    let (_, p) = preset(o, "tiny")?;
    let step: u64 = o.get("step").map_or(Ok(1), |s| s.parse())?;
    let torn = o.contains_key("torn");
    let honest = !o.contains_key("dishonest");
    let relaxed: usize = o.get("relaxed").map_or(Ok(0), |s| s.parse())?;
    std::fs::create_dir_all(&dir)?;
    let (w, _) = workload::generate(&p);
    let ops = parse_ops(&w)?;
    let groups = bench::groups_of(&ops);
    let durable =
        |gi: usize| relaxed == 0 || (gi + 1).is_multiple_of(relaxed) || gi + 1 == groups.len();
    let durability = |gi: usize| {
        if durable(gi) {
            Durability::Immediate
        } else {
            Durability::None
        }
    };
    let open = |b: &std::sync::Arc<PowerLoss>| -> Result<Store, store::Error> {
        Store::from_db(Database::builder().create_with_backend(Shared(b.clone()))?)
    };

    // The store after setup, the image every trial starts from.
    let base_b = PowerLoss::new(Vec::new(), true);
    let st = Store::create_on(Database::builder().create_with_backend(Shared(base_b.clone()))?)?;
    st.setup(w.accounts, &w.ranges)?;
    drop(st);
    let base = base_b.image();

    // The reference run: the digest at every transaction boundary, and the
    // number of storage operations the run makes.
    let ref_b = PowerLoss::new(base.clone(), true);
    let st = open(&ref_b)?;
    let mut digests: BTreeMap<u64, [u8; 32]> = BTreeMap::new();
    digests.insert(0, st.read()?.digest()?);
    let mut c = Counters::default();
    for (gi, g) in groups.iter().enumerate() {
        st.apply_with(&ops[g.clone()], &mut c, durability(gi))?;
        let r = st.read()?;
        digests.insert(r.last_op()?, r.digest()?);
    }
    drop(st);
    let total_ops = ref_b.ops();
    eprintln!(
        "powerloss: {} transactions, {total_ops} storage operations, {} syncs",
        groups.len(),
        ref_b.syncs()
    );

    let mut out = Outcomes::default();
    let mut first_failure: Option<String> = None;
    let mut n = 0u64;
    while n <= total_ops {
        out.trials += 1;
        let b = PowerLoss::new(base.clone(), honest);
        b.power_fails_after(n);
        let (mut acked, mut acked_durable) = (0u64, 0u64);
        if let Ok(st) = open(&b) {
            let mut c = Counters::default();
            for (gi, g) in groups.iter().enumerate() {
                if st
                    .apply_with(&ops[g.clone()], &mut c, durability(gi))
                    .is_err()
                {
                    break;
                }
                acked = g.end as u64;
                if durable(gi) {
                    acked_durable = acked;
                }
            }
        }
        if b.power_failed() {
            out.cut_reached += 1;
        }
        let img = b.cut(torn.then_some(0x7042 ^ n));
        let b2 = PowerLoss::new(img, true);
        let mut note = |what: &str, out_field: &mut u64| {
            *out_field += 1;
            if first_failure.is_none() {
                first_failure = Some(format!("cut after {n} operations: {what}"));
            }
        };
        match open(&b2) {
            Err(e) => note(&format!("reopen failed: {e}"), &mut out.open_failed),
            Ok(st) => {
                let r = st.read()?;
                let j = r.last_op()?;
                match digests.get(&j) {
                    None => note(&format!("reopened at op {j}"), &mut out.not_boundary),
                    Some(want) => {
                        if j < acked_durable {
                            note(
                                &format!("durable op {acked_durable} lost, reopened at {j}"),
                                &mut out.lost_durable,
                            );
                        } else if r.digest()? != *want {
                            note(
                                &format!("state at op {j} differs"),
                                &mut out.digest_mismatch,
                            );
                        } else if let Some(m) = r.check_balances()? {
                            note(
                                &format!("inconsistent at op {j}: {m}"),
                                &mut out.inconsistent,
                            );
                        } else {
                            if j < acked {
                                out.lost_nondurable += 1;
                            }
                            if j == acked_durable {
                                out.at_last_durable_ack += 1;
                            } else {
                                out.past_last_durable_ack += 1;
                            }
                        }
                    }
                }
            }
        }
        n += step;
    }
    let report = json!({
        "transactions": groups.len(), "storage_operations": total_ops,
        "syncs": ref_b.syncs(), "step": step, "torn": torn, "honest_sync": honest,
        "relaxed_every": relaxed, "redb": store_redb_spike::REDB_VERSION,
        "trials": out.trials, "cut_reached": out.cut_reached,
        "reopened_at_last_durable_ack": out.at_last_durable_ack,
        "reopened_past_last_durable_ack": out.past_last_durable_ack,
        "lost_nondurable_commits": out.lost_nondurable,
        "lost_durable_commits": out.lost_durable, "not_at_boundary": out.not_boundary,
        "digest_mismatch": out.digest_mismatch, "inconsistent": out.inconsistent,
        "reopen_failed": out.open_failed, "first_failure": first_failure,
    });
    let name = format!(
        "powerloss{}{}{}.json",
        if torn { "-torn" } else { "" },
        if honest { "" } else { "-dishonest" },
        if relaxed > 0 {
            format!("-relaxed{relaxed}")
        } else {
            String::new()
        }
    );
    std::fs::write(dir.join(name), serde_json::to_string_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Header ingest

fn cmd_headers(o: &BTreeMap<String, String>) -> Res<()> {
    let dir = PathBuf::from(opt(o, "dir")?).join("headers");
    let n: u32 = o.get("n").map_or(Ok(1_050_000), |s| s.parse())?;
    let batch: u32 = o.get("batch").map_or(Ok(2_000), |s| s.parse())?;
    let filter_len: usize = o.get("filter-len").map_or(Ok(0), |s| s.parse())?;
    let relaxed: u32 = o.get("relaxed").map_or(Ok(50), |s| s.parse())?;
    std::fs::create_dir_all(&dir)?;
    let mut results = json!({"n": n, "batch": batch, "filter_len": filter_len});
    let mut go = spawn_go()?;
    for round in 0..2 {
        let db = dir.join(format!("rust-immediate-{round}.redb"));
        let _ = std::fs::remove_file(&db);
        let r = bench::ingest_headers(
            &db,
            n,
            batch,
            filter_len,
            bench::HeaderDurability::Immediate,
        )?;
        eprintln!("[headers] rust immediate {r}");
        results[format!("rust_immediate_{round}")] = r;
        let _ = std::fs::remove_file(&db);
        if let Some(go) = go.as_mut() {
            let gdb = dir.join(format!("go-{round}.db"));
            let _ = std::fs::remove_file(&gdb);
            let r = go.call(
                "headers",
                json!({"db": gdb, "n": n, "batch": batch, "filter_len": filter_len,
                       "seed": hex(&[7u8; 32])}),
            );
            eprintln!("[headers] go {r}");
            results[format!("go_{round}")] = r;
            let _ = std::fs::remove_file(&gdb);
        }
        let db = dir.join(format!("rust-relaxed-{round}.redb"));
        let _ = std::fs::remove_file(&db);
        let r = bench::ingest_headers(
            &db,
            n,
            batch,
            filter_len,
            bench::HeaderDurability::RelaxedEvery(relaxed),
        )?;
        eprintln!("[headers] rust relaxed {r}");
        results[format!("rust_relaxed_{round}")] = r;
        let _ = std::fs::remove_file(&db);
    }
    std::fs::write(
        dir.join(format!("headers-{n}-{filter_len}.json")),
        serde_json::to_string_pretty(&results)?,
    )?;
    println!("{results}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Hot backup, repeated

fn cmd_backup(o: &BTreeMap<String, String>) -> Res<()> {
    let db = PathBuf::from(opt(o, "db")?);
    let go_db = o.get("go-db").map(PathBuf::from);
    let reps: usize = o.get("reps").map_or(Ok(3), |s| s.parse())?;
    let mut go = if go_db.is_some() { spawn_go()? } else { None };
    let dest = db.with_extension("backup");
    let mut out = json!({});
    for writer in [false, true] {
        let (mut rs, mut gs) = (Vec::new(), Vec::new());
        for _ in 0..reps {
            if let (Some(go), Some(g)) = (go.as_mut(), go_db.as_ref()) {
                let bk = g.with_extension("backup");
                let r = go.call("backup", json!({"db": g, "out": bk, "writer": writer}));
                let _ = std::fs::remove_file(&bk);
                gs.push(r);
            }
            rs.push(bench::hot_backup(&db, &dest, writer)?);
        }
        let key = if writer { "writer" } else { "alone" };
        out[format!("rust_{key}")] = json!(rs);
        out[format!("go_{key}")] = json!(gs);
    }
    println!("{out}");
    Ok(())
}
