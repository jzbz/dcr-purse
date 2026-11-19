//! Timing: the Rust half of the measurements, mirroring the Go baseline's
//! `ingest`, `bench` and `open` commands call for call.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use purse_hdkeychain::{ExtendedKey, HdVersions};
use purse_testutil::SplitMix64;
use purse_testutil::serde_json::{Value, json};

use crate::chain::{self, LEGACY_COIN_TYPE, ScriptKind};
use crate::export::{ByAccountSpec, RangeSpec};
use crate::store::{Counters, ParsedOp, Result, Store, W};
use crate::workload::{OpKind, txn_groups};

/// Nearest-rank summary of durations in nanoseconds (the Go side computes
/// the same).
#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    /// Samples.
    pub n: usize,
    /// Sum.
    pub total: u128,
    /// Minimum.
    pub min: u128,
    /// Median.
    pub p50: u128,
    /// 90th percentile.
    pub p90: u128,
    /// 99th percentile.
    pub p99: u128,
    /// Maximum.
    pub max: u128,
}

impl Summary {
    /// Summarises `d`.
    pub fn of(d: &[u128]) -> Summary {
        if d.is_empty() {
            return Summary::default();
        }
        let mut s = d.to_vec();
        s.sort_unstable();
        let rank = |p: usize| {
            let i = (p * s.len()).div_ceil(100).max(1);
            s[i - 1]
        };
        Summary {
            n: s.len(),
            total: s.iter().sum(),
            min: s[0],
            p50: rank(50),
            p90: rank(90),
            p99: rank(99),
            max: s[s.len() - 1],
        }
    }

    /// As JSON, in the Go side's shape.
    pub fn json(&self) -> Value {
        json!({"n": self.n, "total": self.total as u64, "min": self.min as u64,
               "p50": self.p50 as u64, "p90": self.p90 as u64, "p99": self.p99 as u64,
               "max": self.max as u64})
    }
}

/// Runs `f` at least `min` times and until `budget` has elapsed, at most
/// `max` times.
pub fn repeat(
    min: usize,
    max: usize,
    budget: Duration,
    mut f: impl FnMut() -> Result<()>,
) -> Result<Summary> {
    let mut d = Vec::new();
    let start = Instant::now();
    while d.len() < max && (d.len() < min || start.elapsed() < budget) {
        let t = Instant::now();
        f()?;
        d.push(t.elapsed().as_nanos());
    }
    Ok(Summary::of(&d))
}

/// This process's `write_bytes` from `/proc/self/io`: the bytes it caused
/// to be sent to the storage layer (dirtied in the page cache), or -1 where
/// the file does not exist. The Go side reads the same counter.
pub fn io_write_bytes() -> i64 {
    std::fs::read_to_string("/proc/self/io")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("write_bytes: "))
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(-1)
}

/// Applies a parsed workload, one redb write transaction per group, timing
/// each, and returns the Go side's `ingest` shape.
pub fn ingest(store: &Store, ops: &[ParsedOp]) -> Result<Value> {
    let io0 = io_write_bytes();
    let start = Instant::now();
    let mut c = Counters::default();
    let (mut sync_txns, mut sync_blocks, mut sync_ns) = (0u64, 0u64, 0u128);
    let mut tip = Vec::new();
    let mut unmined = Vec::new();
    let mut reorg = Vec::new();
    let mut txs = 0usize;
    for g in groups_of(ops) {
        let group = &ops[g];
        txs += group.iter().map(|o| o.txs.len()).sum::<usize>();
        let t = Instant::now();
        store.apply(group, &mut c)?;
        let d = t.elapsed().as_nanos();
        match group[0].kind {
            OpKind::Reorg => reorg.push(d),
            OpKind::SyncBlock => {
                sync_txns += 1;
                sync_blocks += group.len() as u64;
                sync_ns += d;
            }
            OpKind::TipBlock => tip.push(d),
            OpKind::Unmined => unmined.push(d),
        }
    }
    Ok(json!({
        "elapsed": start.elapsed().as_nanos() as u64,
        "io_write_bytes": io_write_bytes() - io0,
        "ops": ops.len(),
        "txs": txs,
        "sync_txns": sync_txns,
        "sync_blocks": sync_blocks,
        "sync_ns": sync_ns as u64,
        "tip_block": Summary::of(&tip).json(),
        "unmined": Summary::of(&unmined).json(),
        "reorg": Summary::of(&reorg).json(),
        "credits": c.credits,
        "commitments": c.commitments,
        "lookups": c.lookups,
        "lookup_misses": c.lookup_misses,
        "sync_misses": c.sync_misses,
    }))
}

/// The write-transaction groups of parsed operations, with the commit
/// markers the workload file carries.
pub fn groups_of(ops: &[ParsedOp]) -> Vec<std::ops::Range<usize>> {
    let marks: Vec<crate::workload::Op> = ops
        .iter()
        .map(|o| crate::workload::Op {
            kind: o.kind,
            commit: o.commit,
            unpublished: false,
            header: None,
            rollback_to: 0,
            txs: Vec::new(),
        })
        .collect();
    txn_groups(&marks)
}

/// What both sides time; the Go side receives the same values as JSON.
#[derive(Clone, Debug)]
pub struct BenchArgs {
    /// Minimum repetitions of each timed query.
    pub min_reps: usize,
    /// Maximum repetitions.
    pub max_reps: usize,
    /// Time budget per query, after the minimum.
    pub budget_ms: u64,
    /// Confirmation depths for balances.
    pub minconfs: Vec<i32>,
    /// Accounts for balances and selection.
    pub accounts: Vec<u32>,
    /// Selection targets, in atoms.
    pub targets: Vec<i64>,
    /// Range queries.
    pub ranges: Vec<RangeSpec>,
    /// Per-account history queries.
    pub by_account: Vec<ByAccountSpec>,
    /// Scripts to look up (hex).
    pub scripts: Vec<String>,
    /// Next-address calls.
    pub next_addr: usize,
    /// Selection calls per target.
    pub select_reps: usize,
}

impl BenchArgs {
    /// The Go command's arguments.
    pub fn go_json(&self, db: &Path) -> Value {
        json!({
            "db": db,
            "min_reps": self.min_reps,
            "max_reps": self.max_reps,
            "budget_ms": self.budget_ms,
            "minconfs": self.minconfs,
            "accounts": self.accounts,
            "targets": self.targets,
            "ranges": self.ranges.iter().map(|r| json!({"name": r.name, "begin": r.begin,
                "end": r.end, "limit": r.limit})).collect::<Vec<_>>(),
            "by_account": self.by_account.iter().map(|b| json!({"name": b.name,
                "account": b.account, "limit": b.limit})).collect::<Vec<_>>(),
            "scripts": self.scripts,
            "next_addr": self.next_addr,
            "gap": chain::GAP_LIMIT,
            "select_reps": self.select_reps,
        })
    }
}

/// The external branch key of account 0, for next-address derivation.
pub fn account0_external(wallet_seed: &[u8; 32]) -> ExtendedKey {
    let master = ExtendedKey::new_master(wallet_seed, &HdVersions::SIMNET).expect("master");
    master
        .child(44 + 0x8000_0000)
        .and_then(|k| k.child(LEGACY_COIN_TYPE + 0x8000_0000))
        .and_then(|k| k.child(0x8000_0000))
        .map(|k| k.neuter())
        .and_then(|k| k.child(0))
        .expect("branch key")
}

/// Runs the timed queries against the store at `path`; returns the Go
/// side's `bench` shape.
pub fn bench(path: &Path, a: &BenchArgs, wallet_seed: &[u8; 32]) -> Result<Value> {
    let store = Store::open(path)?;
    let budget = Duration::from_millis(a.budget_ms);
    let mut out = Vec::new();
    let mut add = |name: String, s: Summary, note: String| {
        out.push(json!({"name": name, "summary": s.json(), "note": note}));
    };
    let tip = store.read()?.tip()?.0;

    for &mc in &a.minconfs {
        for &acct in &a.accounts {
            let s = repeat(a.min_reps, a.max_reps, budget, || {
                let r = store.read()?;
                std::hint::black_box(r.account_balance(acct, mc, tip)?);
                Ok(())
            })?;
            add(
                format!("balance/minconf={mc}/account={acct}"),
                s,
                String::new(),
            );
        }
        let s = repeat(a.min_reps, a.max_reps, budget, || {
            let r = store.read()?;
            std::hint::black_box(r.account_balances(mc, tip)?);
            Ok(())
        })?;
        add(format!("balance_all/minconf={mc}"), s, String::new());
    }

    let mut n = 0;
    let s = repeat(a.min_reps, a.max_reps, budget, || {
        let r = store.read()?;
        n = r.unspent_outputs()?.len();
        Ok(())
    })?;
    add("unspent/outputs".into(), s, format!("{n} outputs"));
    let s = repeat(a.min_reps, a.max_reps, budget, || {
        let r = store.read()?;
        let mut count = 0usize;
        r.for_each_unspent_outpoint(|_, _| count += 1)?;
        std::hint::black_box(count);
        Ok(())
    })?;
    add("unspent/outpoints".into(), s, String::new());

    let mut rng = SplitMix64(0x5e1ec7);
    for &target in &a.targets {
        let (mut selected, mut short, mut i) = (0usize, 0usize, 0usize);
        let s = repeat(a.select_reps, a.select_reps, Duration::ZERO, || {
            let acct = a.accounts[i % a.accounts.len()];
            i += 1;
            let r = store.read()?;
            let seed = rng.next_u64();
            let mut src = r.input_source(acct, 1, tip as i32, SplitMix64(seed))?;
            let d = src.select(target)?;
            selected += d.inputs.len();
            if d.amount < target {
                short += 1;
            }
            Ok(())
        })?;
        add(
            format!("select/target={target}"),
            s,
            format!("{selected} inputs selected, {short} short"),
        );
    }

    for rs in &a.ranges {
        let mut n = 0usize;
        let s = repeat(a.min_reps, a.max_reps, budget, || {
            n = 0;
            let r = store.read()?;
            r.range_transactions(rs.begin, rs.end, |d| {
                n += d.len();
                rs.limit > 0 && n >= rs.limit
            })?;
            Ok(())
        })?;
        add(format!("range/{}", rs.name), s, format!("{n} transactions"));
    }
    for b in &a.by_account {
        let mut n = 0usize;
        let s = repeat(a.min_reps, a.max_reps, budget, || {
            let r = store.read()?;
            let hashes = r.by_account(b.account, b.limit)?;
            n = hashes.len();
            for h in &hashes {
                std::hint::black_box(r.tx_details(h)?);
            }
            Ok(())
        })?;
        add(
            format!("by_account/{}", b.name),
            s,
            format!("{n} transactions"),
        );
    }

    let scripts: Vec<Vec<u8>> = a.scripts.iter().map(|h| purse_testutil::unhex(h)).collect();
    let mut hits = 0;
    let s = repeat(a.min_reps, a.max_reps, budget, || {
        hits = 0;
        let r = store.read()?;
        for script in &scripts {
            if let ScriptKind::PubKeyHash { hash, .. } = chain::classify(script)
                && r.address(&hash)?.is_some()
            {
                hits += 1;
            }
        }
        Ok(())
    })?;
    add(
        "lookup/batch".into(),
        s,
        format!("{} scripts, {hits} hits", scripts.len()),
    );

    // Next-address bookkeeping: derive the child (as dcrwallet's address
    // sync does inside its transaction) and record it in one write
    // transaction.
    let branch = account0_external(wallet_seed);
    let mut next = store.read()?.last_path(0, 0)?.map_or(0, |i| i + 1);
    let s = repeat(a.next_addr, a.next_addr, Duration::ZERO, || {
        let child = next;
        next += 1;
        let key = branch.child(child).expect("child");
        let h = chain::hash160(key.serialized_pub_key());
        let txn = store.db.begin_write()?;
        {
            let mut w = W::open(&txn)?;
            w.record_address(0, 0, child, &h)?;
        }
        txn.commit()?;
        Ok(())
    })?;
    add("next_address".into(), s, String::new());

    drop(store);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(json!({"timings": out, "file_size": size}))
}

/// Opens the store and runs the first balance query, timing both.
pub fn open_once(path: &Path) -> Result<Value> {
    let t = Instant::now();
    let store = Store::open(path)?;
    let open = t.elapsed().as_nanos() as u64;
    let t = Instant::now();
    let r = store.read()?;
    let tip = r.tip()?.0;
    std::hint::black_box(r.account_balances(1, tip)?);
    let first = t.elapsed().as_nanos() as u64;
    Ok(json!({"open": open, "first_balance": first}))
}

/// Drops a file's pages from the page cache (`dd iflag=nocache`, which
/// needs no privileges).
pub fn drop_cache(path: &Path) {
    let _ = Command::new("dd")
        .arg(format!("if={}", path.display()))
        .args(["iflag=nocache", "count=0", "status=none"])
        .status();
}

/// redb's own accounting of the file: tree height, pages, stored and
/// fragmented bytes.
pub fn redb_stats(path: &Path) -> Result<Value> {
    let store = Store::open(path)?;
    let txn = store.db.begin_write()?;
    let st = txn.stats()?;
    let v = json!({
        "tree_height": st.tree_height(), "allocated_pages": st.allocated_pages(),
        "leaf_pages": st.leaf_pages(), "branch_pages": st.branch_pages(),
        "stored_bytes": st.stored_bytes(), "metadata_bytes": st.metadata_bytes(),
        "fragmented_bytes": st.fragmented_bytes(), "page_size": st.page_size(),
    });
    txn.abort()?;
    Ok(v)
}

/// The size of a compacted copy of the store (the copy is deleted).
pub fn compacted_size(path: &Path, scratch: &Path) -> Result<Value> {
    let copy = scratch.join("compacted.redb");
    std::fs::copy(path, &copy).map_err(|e| crate::store::Error::Invalid(e.to_string()))?;
    let t = Instant::now();
    let mut db = redb::Database::open(&copy)?;
    let mut rounds = 0;
    while db
        .compact(redb::CompactionMode::Balanced)
        .map_err(|e| crate::store::Error::Invalid(e.to_string()))?
    {
        rounds += 1;
    }
    drop(db);
    let elapsed = t.elapsed().as_nanos() as u64;
    let size = std::fs::metadata(&copy).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&copy);
    Ok(json!({"size": size, "rounds": rounds, "elapsed": elapsed}))
}

/// A consistent copy: a read transaction's snapshot copied into a new file
/// (one durable commit). With `writer`, a thread keeps committing
/// throwaway addresses meanwhile, one per write transaction. Checks that
/// the copy's digest, over every table, equals the snapshot's.
pub fn hot_backup(path: &Path, dest: &Path, with_writer: bool) -> Result<Value> {
    let _ = std::fs::remove_file(dest);
    let store = Store::open(path)?;
    let stop = std::sync::atomic::AtomicBool::new(false);
    let commits = std::sync::atomic::AtomicU64::new(0);
    let (elapsed, snap_digest) = std::thread::scope(|s| -> Result<(u128, [u8; 32])> {
        let writer = s.spawn(|| -> Result<()> {
            let mut i = 0x2000_0000u32;
            while with_writer && !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let txn = store.db.begin_write()?;
                {
                    let mut w = W::open(&txn)?;
                    let mut h = [0xeeu8; 20];
                    h[..4].copy_from_slice(&i.to_be_bytes());
                    w.record_address(0, 1, i, &h)?;
                }
                txn.commit()?;
                commits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                i += 1;
            }
            Ok(())
        });
        // Let the writer get going before the snapshot is taken.
        if with_writer {
            std::thread::sleep(Duration::from_millis(20));
        }
        let t = Instant::now();
        let r = store.read()?;
        r.copy_to(dest)?;
        let elapsed = t.elapsed().as_nanos();
        let d = r.digest()?;
        drop(r);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        writer.join().expect("writer thread")?;
        Ok((elapsed, d))
    })?;
    let copy = Store::open(dest)?;
    let same = copy.read()?.digest()? == snap_digest;
    let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    drop(copy);
    let _ = std::fs::remove_file(dest);
    Ok(
        json!({"elapsed": elapsed as u64, "size": size, "writer_commits": commits.into_inner(),
              "copy_equals_snapshot": same}),
    )
}

/// How the header-ingest measurement commits.
#[derive(Clone, Copy, Debug)]
pub enum HeaderDurability {
    /// Every batch durable (`Durability::Immediate`), as dcrwallet's bbolt
    /// commits are.
    Immediate,
    /// Batches with `Durability::None`, every `n`th and the last durable.
    RelaxedEvery(u32),
}

/// Header-plus-filter ingest, as SPV initial sync or a rescan from scratch
/// writes it before any transaction: `n` generated headers with filters of
/// `filter_len` bytes, `batch` per write transaction, into a new store at
/// `path`. Generation is outside the timed region.
pub fn ingest_headers(
    path: &Path,
    n: u32,
    batch: u32,
    filter_len: usize,
    mode: HeaderDurability,
) -> Result<Value> {
    use redb::Durability;
    let store = Store::create(path)?;
    let mut rng = SplitMix64(0x4eade55);
    let mut prev = chain::SIMNET_GENESIS;
    let mut times = Vec::new();
    let mut durable = 0u32;
    let io0 = io_write_bytes();
    let mut start = 1u32;
    let mut k = 0u32;
    while start <= n {
        let cnt = batch.min(n - start + 1);
        let mut hs = Vec::with_capacity(cnt as usize);
        for i in 0..cnt {
            let height = start + i;
            let mut h = [0u8; 64];
            rng.fill(&mut h);
            let hd = dcroxide_wire::BlockHeader {
                version: 1,
                prev_block: dcroxide_chainhash::Hash(prev),
                merkle_root: dcroxide_chainhash::Hash(h[..32].try_into().expect("32")),
                stake_root: dcroxide_chainhash::Hash(h[32..].try_into().expect("32")),
                vote_bits: 1,
                final_state: [0; 6],
                voters: 5,
                fresh_stake: 0,
                revocations: 0,
                pool_size: 64,
                bits: 0x207f_ffff,
                sbits: 100_000_000,
                height,
                size: 1_000,
                timestamp: 1_700_000_000 + height * 300,
                nonce: rng.next_u64() as u32,
                extra_data: [0; 32],
                stake_version: 0,
            };
            prev = hd.block_hash().0;
            let mut filter = vec![0u8; 16 + filter_len];
            rng.fill(&mut filter);
            hs.push((hd.serialize(), filter));
        }
        k += 1;
        let last = start + cnt > n;
        let d = match mode {
            HeaderDurability::Immediate => Durability::Immediate,
            HeaderDurability::RelaxedEvery(e) if last || k.is_multiple_of(e) => {
                Durability::Immediate
            }
            HeaderDurability::RelaxedEvery(_) => Durability::None,
        };
        if matches!(d, Durability::Immediate) {
            durable += 1;
        }
        let t = Instant::now();
        let mut txn = store.db.begin_write()?;
        txn.set_durability(d)?;
        {
            let mut w = W::open(&txn)?;
            for (h, f) in &hs {
                w.attach_header(h, f)?;
            }
            w.flush_meta()?;
        }
        txn.commit()?;
        times.push(t.elapsed().as_nanos());
        start += cnt;
    }
    let io = io_write_bytes() - io0;
    let total: u128 = times.iter().sum();
    drop(store);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "n": n, "batch": batch, "filter_len": filter_len, "mode": format!("{mode:?}"),
        "durable_commits": durable, "elapsed": total as u64,
        "batches": Summary::of(&times).json(), "io_write_bytes": io, "file_size": size,
    }))
}
