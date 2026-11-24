// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! `purse-store` against dcrwallet's own database code on the same
//! workload: the measurement the brief's Phase 2 exit asks the schema ADR
//! to record (brief §6 Phase 2, "the schema ADR records the measured gains
//! against dcrwallet on the same workload"; ADR-0003, addendum "purse-store
//! against dcrwallet on the same workload"; `docs/bench-ledger.md`).
//!
//! **The two sides.** Go is `tools/oracle/cmd/storebench`: dcrwallet's
//! `wallet/udb` at the parity pin over bbolt, opened as dcrwallet opens it
//! (every commit synchronous), fed through the call sequence of the wallet
//! (`setup`, `ingest`) and timed through udb's own API (`hot`). Rust is
//! this crate's address manager ([`Manager`], the port of udb's) over
//! `purse-store`, every commit `Durability::Immediate`. The transaction
//! store of `purse-udb` is not written yet (brief §6 Phase 3), so the
//! transaction-store rows of each block (`blocks`, `history`, `tx_mined`,
//! `credits`, `debits`, `unspent`, `balances`, `balance_heights`,
//! `acct_history`) are written here, as ADR-0003 Decision items 2 and 4 say
//! the write path writes them, and the queries read them as the schema
//! intends (the balance from the running sums and the per-height rows of
//! the depth window, the range from `history`); both are the spike's
//! (`spikes/store-redb/src/store.rs`, `account_balance`,
//! `range_transactions`) moved onto `purse-store`'s typed tables. What
//! these figures measure is therefore the schema and the store, plus the
//! address manager as built; the transaction store's own code, when Phase
//! 3 writes it, must be measured again.
//!
//! **The same data on both sides.** A generator builds a simnet wallet
//! history from a fixed seed: receives to fresh or reused external
//! addresses and sends of one to three of the wallet's outputs with change
//! to a fresh internal address, over Zipf-weighted BIP0044 accounts, every
//! block approving its parent, no stake transactions, no unmined
//! transactions and no reorganisations (what the store's write path for
//! those would write is Phase 3's). Its addresses are derived from the
//! wallet seed with `purse-hdkeychain`, and both sides record them through
//! their address managers, each checking that every one is recorded at the
//! generator's account, branch and index. The history is written in
//! dcrwallet's `PURSEWL1` format (the spike's) as two files: the history
//! sync (blocks in batches of 2,000 per write transaction) and the blocks
//! at the tip (one write transaction each). Each side ingests both with
//! the wallet's call sequence: per output paying a recorded address, the
//! address lookup, the credit, `MarkUsed` and the gap-limit sync of the
//! branch. Before anything is timed the two databases are compared
//! through dcrwallet's canonical export (`storebench export`, the spike's
//! format): tip, unspent count, account counters, balances at minconf 0, 1
//! and 6, every unspent output with its script, the ranges below with
//! every credit and debit, the outpoint walk and each account's addresses
//! in `ForEachAccountAddress` order must be equal line for line; and every
//! timed query's answer must equal Go's in every round.
//!
//! **What is timed** (the brief's hot paths): `AccountBalance` of the
//! largest and the smallest account at minconf 1 and 6, and
//! `AccountBalances` at minconf 1; `RangeTransactions` over the last 1,000
//! blocks backward, 1,000 blocks mid-chain forward, the 100 most recent
//! transactions and the whole history; 1,000 address lookups by script
//! through `Manager.Address` and through `AddrAccount`, 1,000 account
//! lookups by name and 1,000 `AccountProperties`, each batch in one read
//! transaction; and commits: each block at the tip (its own durable write
//! transaction), the history sync per block, and 20 commits recording one
//! more external address. Queries repeat at least 5 times and until a
//! second has passed (at most 200 times), as storebench's `repeat` does;
//! a round reports each query's median. The timed work is the same on
//! both sides: a range decodes and keeps every transaction it returns, and
//! the text its check compares is built after the timing on both sides
//! (`storebench hot` copies udb's batches out of its callback and formats
//! them afterwards); a lookup batch builds its check text inside the
//! timing on both sides.
//!
//! **A session** (`hot_paths_against_dcrwallet`, ignored): generate the
//! workload, set up both wallets and ingest the history sync into each
//! (timed once per side), keep both files, then `PURSE_UDB_MEASURE_ROUNDS`
//! rounds (default 3), alternating which side goes first (Rust then Go,
//! Go then Rust, ...): each round copies its side's file, ingests the tip
//! blocks and runs the queries, Go in a fresh oracle process, Rust on a
//! freshly opened store. Release build, one test thread, the whole test
//! pinned to the same cores (both sides inherit them):
//!
//! ```sh
//! PURSE_STORE_BENCH_DIR=/some/dir/outside/the/repository PURSE_REQUIRE_ORACLE=1 \
//!   taskset -c 12-15 cargo test --release -p purse-udb --test measure -- \
//!   --ignored --nocapture --test-threads 1
//! ```
//!
//! Each session writes `session-<unix time>/` under `PURSE_STORE_BENCH_DIR`
//! with `results.tsv` (one line per side, round and figure), `notes.txt`
//! (start and end time, load averages, CPU affinity, versions, sizes) and
//! `table.md`. `PURSE_UDB_MEASURE_BLOCKS` changes the chain length
//! (default 40,000 blocks, about 100,000 wallet transactions).
//!
//! `same_data_and_answers_on_both_sides` runs the same pipeline at a small
//! size, one repetition of everything, in every test run: it holds the
//! export comparison and the query answers equal, and its times mean
//! nothing.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "measurement code: counters, sums of generated amounts, sizes and statistics"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dcroxide_txscript::stdaddr;
use dcroxide_txscript::stdscript::{self, ScriptType};
use dcroxide_wire::{BlockHeader, MsgTx, OutPoint, TxIn, TxOut};
use purse_errors::Kind;
use purse_hdkeychain::ExtendedKey;
use purse_snacl::{Limits, OsRandom};
use purse_store::{
    AcctHistory, AcctHistoryKey, BalanceClass, BalanceHeightKey, BalanceHeights, BalanceKey,
    BalanceRow, Balances, BlockRow, Blocks, CFilterRow, ChainBlock, CreditFlags, CreditKey,
    CreditRow, Credits, DebitRow, Debits, Error as StoreError, Hash32, HeaderBytes, Height,
    History, HistoryKey, Outpoint, OutputTag, ReadTx, ScriptLoc, Spender, Store, TxMined,
    TxMinedRow, TxRecKey, Unspent, UnspentRow, WriteTx,
};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{Oracle, SplitMix64, hex, oracle_bin_or_skip, oracle_or_skip};
use purse_udb::Manager;
use purse_udb::addrmgr::ChainParams;
use sha2::{Digest as _, Sha256};

const HARDENED: u32 = 0x8000_0000;
/// The gap limit both sides' ingest syncs past the last used child
/// (dcrwallet's default, `wallet.DefaultGapLimit`).
const GAP: u32 = 20;
const PUB_PASS: &[u8] = b"public";
const PRIV_PASS: &[u8] = b"private";

// ---------------------------------------------------------------------------
// The workload

/// What to generate.
#[derive(Clone, Debug)]
struct Preset {
    /// BIP0044 accounts (0 to `accounts - 1`).
    accounts: u32,
    /// Blocks after genesis.
    blocks: u32,
    /// The last `tip_blocks` blocks are one write transaction each.
    tip_blocks: u32,
    /// Blocks per write transaction before them.
    batch: u32,
    /// Wallet transactions per block: uniform in `0..txs_bound`.
    txs_bound: u64,
    /// Share of transactions that are sends (when the account has
    /// spendable outputs).
    send_share: f64,
}

impl Preset {
    fn small() -> Preset {
        Preset {
            accounts: 3,
            blocks: 240,
            tip_blocks: 20,
            batch: 50,
            txs_bound: 4,
            send_share: 0.3,
        }
    }

    fn measure() -> Preset {
        let blocks = std::env::var("PURSE_UDB_MEASURE_BLOCKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40_000);
        Preset {
            accounts: 4,
            blocks,
            tip_blocks: 300,
            batch: 2_000,
            txs_bound: 6,
            send_share: 0.3,
        }
    }
}

#[derive(Clone, Debug)]
struct Utxo {
    hash: [u8; 32],
    index: u32,
    value: i64,
    height: u32,
}

#[derive(Clone, Debug)]
struct Block {
    height: u32,
    hash: [u8; 32],
    header: [u8; 180],
    txs: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct AddrRange {
    account: u32,
    branch: u32,
    hashes: Vec<[u8; 20]>,
}

#[derive(Debug)]
struct Workload {
    seed: [u8; 32],
    accounts: u32,
    batch: u32,
    ranges: Vec<AddrRange>,
    sync: Vec<Block>,
    tip: Vec<Block>,
    receives: u64,
    sends: u64,
}

impl Workload {
    fn tip_height(&self) -> u32 {
        self.tip.last().or(self.sync.last()).map_or(0, |b| b.height)
    }

    fn transactions(&self) -> usize {
        self.sync.iter().chain(&self.tip).map(|b| b.txs.len()).sum()
    }

    fn addresses(&self) -> usize {
        self.ranges.iter().map(|r| r.hashes.len()).sum()
    }
}

/// A generated transaction and the wallet outputs it creates, as
/// (account, output index, amount).
type Built = (MsgTx, Vec<(u32, u32, i64)>);

struct Account {
    branches: [ExtendedKey; 2],
    derived: [Vec<[u8; 20]>; 2],
    next: [u32; 2],
    used_ext: Vec<u32>,
}

struct Gen {
    rng: SplitMix64,
    accts: Vec<Account>,
    utxos: Vec<Vec<Utxo>>,
    receives: u64,
    sends: u64,
}

impl Gen {
    fn unit(&mut self) -> f64 {
        (self.rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.rng.below(n as u64)).expect("fits")
    }

    fn hash32(&mut self) -> [u8; 32] {
        let mut h = [0; 32];
        self.rng.fill(&mut h);
        h
    }

    fn hash20(&mut self) -> [u8; 20] {
        let mut h = [0; 20];
        self.rng.fill(&mut h);
        h
    }

    /// Account `i` with weight `1/(i+1)`.
    fn pick_account(&mut self) -> u32 {
        let n = u32::try_from(self.accts.len()).expect("accounts");
        let total: f64 = (1..=n).map(|i| 1.0 / f64::from(i)).sum();
        let mut x = self.unit() * total;
        for i in 0..n {
            x -= 1.0 / f64::from(i + 1);
            if x <= 0.0 {
                return i;
            }
        }
        n - 1
    }

    fn address(&mut self, account: u32, branch: usize, index: u32) -> [u8; 20] {
        let a = &mut self.accts[account as usize];
        while a.derived[branch].len() <= index as usize {
            let i = u32::try_from(a.derived[branch].len()).expect("index");
            let child = a.branches[branch].child(i).expect("child derivation");
            a.derived[branch].push(stdaddr::hash160(child.serialized_pub_key()));
        }
        a.derived[branch][index as usize]
    }

    fn fresh(&mut self, account: u32, branch: usize) -> [u8; 20] {
        let a = &mut self.accts[account as usize];
        let idx = a.next[branch];
        a.next[branch] = idx + 1;
        if branch == 0 {
            a.used_ext.push(idx);
        }
        self.address(account, branch, idx)
    }

    fn receive_address(&mut self, account: u32) -> [u8; 20] {
        let used = self.accts[account as usize].used_ext.len();
        if used > 0 && self.unit() < 0.4 {
            let j = self.below(used);
            let idx = self.accts[account as usize].used_ext[j];
            self.address(account, 0, idx)
        } else {
            self.fresh(account, 0)
        }
    }

    fn sig_script(&mut self) -> Vec<u8> {
        // A P2PKH signature script's shape: a 72-byte push, a 33-byte push.
        let mut s = Vec::with_capacity(107);
        s.push(0x48);
        s.extend_from_slice(&self.rng.bytes(72));
        s.push(0x21);
        s.extend_from_slice(&self.rng.bytes(33));
        s
    }

    fn input(&mut self, hash: [u8; 32], index: u32, value: i64, height: u32) -> TxIn {
        let mut previous_out_point = OutPoint::default();
        previous_out_point.hash.0 = hash;
        previous_out_point.index = index;
        TxIn {
            previous_out_point,
            sequence: u32::MAX,
            value_in: value,
            block_height: height,
            block_index: self.rng.below(20) as u32,
            signature_script: self.sig_script(),
        }
    }

    fn amount(&mut self) -> i64 {
        // Log-uniform between 0.001 and 1,000 coins.
        let e = 5.0 + self.unit() * 6.0;
        10f64.powf(e) as i64
    }

    /// A receive: one foreign input, one output to the wallet and one
    /// foreign change output, in random order.
    fn receive(&mut self, height: u32) -> Built {
        let account = self.pick_account();
        let mut tx = MsgTx::default();
        let value = self.amount();
        let h = self.hash32();
        let idx = self.rng.below(4) as u32;
        let inp = self.input(h, idx, value + 100_000, height.saturating_sub(10));
        tx.tx_in.push(inp);
        let to = self.receive_address(account);
        let foreign = self.hash20();
        let wallet_out = TxOut {
            value,
            version: 0,
            pk_script: p2pkh(&to),
        };
        let change = TxOut {
            value: 97_500,
            version: 0,
            pk_script: p2pkh(&foreign),
        };
        let wallet_index = if self.unit() < 0.5 {
            tx.tx_out.push(wallet_out);
            tx.tx_out.push(change);
            0
        } else {
            tx.tx_out.push(change);
            tx.tx_out.push(wallet_out);
            1
        };
        self.receives += 1;
        (tx, vec![(account, wallet_index, value)])
    }

    /// A send from one account: one to three of its outputs mined before
    /// this block, a payment to a foreign address and, unless the change
    /// is dust, change to a fresh internal address of the same account.
    fn send(&mut self, height: u32) -> Option<Built> {
        let account = self.pick_account();
        let n = match self.below(10) {
            0..=6 => 1,
            7..=8 => 2,
            _ => 3,
        };
        let mut ins = Vec::new();
        let mut tries = 0;
        while ins.len() < n && tries < n * 8 + 8 {
            tries += 1;
            let len = self.utxos[account as usize].len();
            if len == 0 {
                break;
            }
            let i = self.below(len);
            let u = &self.utxos[account as usize][i];
            if u.height < height && u.value >= 20_000 {
                ins.push(self.utxos[account as usize].swap_remove(i));
            }
        }
        if ins.is_empty() {
            return None;
        }
        let mut tx = MsgTx::default();
        let mut total = 0i64;
        for u in &ins {
            let inp = self.input(u.hash, u.index, u.value, u.height);
            tx.tx_in.push(inp);
            total += u.value;
        }
        let fee = 2_000 + 1_500 * ins.len() as i64;
        let pay = ((total - fee) as f64 * (0.2 + 0.75 * self.unit())) as i64;
        let change = total - fee - pay;
        let payee = self.hash20();
        let pay_out = TxOut {
            value: pay,
            version: 0,
            pk_script: p2pkh(&payee),
        };
        let mut outputs = Vec::new();
        if change >= 6_000 {
            let to = self.fresh(account, 1);
            let change_out = TxOut {
                value: change,
                version: 0,
                pk_script: p2pkh(&to),
            };
            let ci = if self.unit() < 0.5 {
                tx.tx_out.push(change_out);
                tx.tx_out.push(pay_out);
                0
            } else {
                tx.tx_out.push(pay_out);
                tx.tx_out.push(change_out);
                1
            };
            outputs.push((account, ci, change));
        } else {
            tx.tx_out.push(pay_out);
        }
        self.sends += 1;
        Some((tx, outputs))
    }
}

fn p2pkh(hash: &[u8; 20]) -> Vec<u8> {
    let mut s = Vec::with_capacity(25);
    s.extend_from_slice(&[0x76, 0xa9, 0x14]);
    s.extend_from_slice(hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

fn simnet_genesis() -> BlockHeader {
    dcroxide_chaincfg::simnet_params().genesis_block.header
}

fn generate(p: &Preset, chain: &ChainParams) -> Workload {
    let mut rng = SplitMix64(0x6d65_6173_7572_6532);
    let mut seed = [0u8; 32];
    rng.fill(&mut seed);
    let master = ExtendedKey::new_master(&seed, &chain.hd).expect("master key");
    let coin = master
        .child(44 + HARDENED)
        .and_then(|k| k.child(chain.legacy_coin_type + HARDENED))
        .expect("coin type key");
    let accts = (0..p.accounts)
        .map(|a| {
            let acct = coin.child(a + HARDENED).expect("account key").neuter();
            Account {
                branches: [
                    acct.child(0).expect("branch"),
                    acct.child(1).expect("branch"),
                ],
                derived: [Vec::new(), Vec::new()],
                next: [0, 0],
                used_ext: Vec::new(),
            }
        })
        .collect();
    let mut g = Gen {
        rng,
        accts,
        utxos: vec![Vec::new(); p.accounts as usize],
        receives: 0,
        sends: 0,
    };
    let genesis = simnet_genesis();
    let mut header = genesis;
    let mut prev = genesis.block_hash().0;
    let mut blocks = Vec::with_capacity(p.blocks as usize);
    for height in 1..=p.blocks {
        let ntx = g.below(p.txs_bound as usize);
        let mut txs = Vec::with_capacity(ntx);
        let mut new_utxos = Vec::new();
        for _ in 0..ntx {
            let built = if g.unit() < p.send_share {
                g.send(height)
            } else {
                None
            };
            let (tx, outs) = built.unwrap_or_else(|| g.receive(height));
            let hash = tx.tx_hash().0;
            for (account, index, value) in outs {
                new_utxos.push((
                    account,
                    Utxo {
                        hash,
                        index,
                        value,
                        height,
                    },
                ));
            }
            txs.push(tx.serialize());
        }
        for (account, u) in new_utxos {
            g.utxos[account as usize].push(u);
        }
        let mut roots = [0u8; 64];
        g.rng.fill(&mut roots);
        header.prev_block.0 = prev;
        header.merkle_root.0.copy_from_slice(&roots[..32]);
        header.stake_root.0.copy_from_slice(&roots[32..]);
        header.vote_bits = 1;
        header.voters = 5;
        header.height = height;
        header.timestamp = genesis.timestamp + height * 300;
        header.nonce = g.rng.next_u64() as u32;
        let hash = header.block_hash().0;
        blocks.push(Block {
            height,
            hash,
            header: header.serialize(),
            txs,
        });
        prev = hash;
    }
    let mut ranges = Vec::new();
    for account in 0..p.accounts {
        for branch in 0..2usize {
            // Every child up to the last used one plus the gap, so the
            // ingest's gap-limit syncs record nothing new on either side.
            let n = g.accts[account as usize].next[branch] + GAP;
            let hashes = (0..n).map(|i| g.address(account, branch, i)).collect();
            ranges.push(AddrRange {
                account,
                branch: u32::try_from(branch).expect("branch"),
                hashes,
            });
        }
    }
    let split = blocks.len().saturating_sub(p.tip_blocks as usize);
    let tip = blocks.split_off(split);
    Workload {
        seed,
        accounts: p.accounts,
        batch: p.batch,
        ranges,
        sync: blocks,
        tip,
        receives: g.receives,
        sends: g.sends,
    }
}

/// Writes `blocks` as a `PURSEWL1` file (the spike's format, which
/// storebench reads): the account count and address ranges, then one block
/// operation per block, kind 1 (history sync, a commit every `batch`
/// blocks and at the last) or 3 (at the tip, a commit each).
fn write_workload(path: &Path, w: &Workload, blocks: &[Block], tip: bool) {
    let mut f = Vec::new();
    f.extend_from_slice(b"PURSEWL1");
    let u32_len = |n: usize| u32::try_from(n).expect("fits").to_le_bytes();
    f.extend_from_slice(&w.accounts.to_le_bytes());
    f.extend_from_slice(&u32_len(w.ranges.len()));
    for r in &w.ranges {
        f.extend_from_slice(&r.account.to_le_bytes());
        f.extend_from_slice(&r.branch.to_le_bytes());
        f.extend_from_slice(&u32_len(r.hashes.len()));
        for h in &r.hashes {
            f.extend_from_slice(h);
        }
    }
    f.extend_from_slice(&u32_len(blocks.len()));
    let batch = w.batch as usize;
    for (i, b) in blocks.iter().enumerate() {
        let commit = tip || (i + 1) % batch == 0 || i + 1 == blocks.len();
        f.push(if tip { 3 } else { 1 });
        f.push(u8::from(commit));
        f.extend_from_slice(&b.header);
        f.extend_from_slice(&u32_len(b.txs.len()));
        for tx in &b.txs {
            f.extend_from_slice(&u32_len(tx.len()));
            f.extend_from_slice(tx);
        }
    }
    fs::write(path, f).expect("write workload");
}

// ---------------------------------------------------------------------------
// Timing

#[derive(Clone, Copy, Debug, Default)]
struct Summary {
    n: usize,
    min: f64,
    p50: f64,
    p90: f64,
    max: f64,
}

/// Nearest-rank percentiles, as storebench's `summarize`.
fn summarize(d: &[f64]) -> Summary {
    if d.is_empty() {
        return Summary::default();
    }
    let mut s = d.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = |p: usize| s[((p * s.len()).div_ceil(100)).max(1) - 1];
    Summary {
        n: s.len(),
        min: s[0],
        p50: rank(50),
        p90: rank(90),
        max: s[s.len() - 1],
    }
}

fn go_summary(v: &Value) -> Summary {
    let f = |k: &str| {
        v[k].as_f64()
            .unwrap_or_else(|| panic!("summary field {k} in {v}"))
    };
    Summary {
        n: v["n"].as_u64().expect("n") as usize,
        min: f("min"),
        p50: f("p50"),
        p90: f("p90"),
        max: f("max"),
    }
}

#[derive(Clone, Copy, Debug)]
struct Reps {
    min: usize,
    max: usize,
    budget: Duration,
}

impl Reps {
    fn json(self) -> Value {
        json!({"min_reps": self.min, "max_reps": self.max, "budget_ms": self.budget.as_millis() as u64})
    }
}

/// storebench's `repeat`: at least `min` runs and until the budget has
/// passed, at most `max`, each timed.
fn repeat(r: Reps, mut f: impl FnMut()) -> Summary {
    let mut d = Vec::new();
    let start = Instant::now();
    while d.len() < r.max && (d.len() < r.min || start.elapsed() < r.budget) {
        let t0 = Instant::now();
        f();
        d.push(t0.elapsed().as_nanos() as f64);
    }
    summarize(&d)
}

fn ns(d: Duration) -> f64 {
    d.as_nanos() as f64
}

// ---------------------------------------------------------------------------
// The Rust side: the address manager over purse-store, and the
// transaction-store rows the schema's write path writes

struct RustWallet {
    store: Store,
    mgr: Manager,
    chain: ChainParams,
}

fn open_manager(store: &Store, chain: &ChainParams) -> Manager {
    let mut tx = store.read().expect("read");
    Manager::open(
        &mut tx,
        chain,
        PUB_PASS,
        Box::new(OsRandom),
        Limits::DEFAULT,
    )
    .expect("open manager")
}

fn open_rust(path: &Path, chain: &ChainParams) -> RustWallet {
    let store = Store::open(path).expect("open store");
    let mgr = open_manager(&store, chain);
    RustWallet {
        store,
        mgr,
        chain: *chain,
    }
}

/// `setup` on the Rust side: the wallet from the seed, the accounts, every
/// address of the ranges, the genesis block; then every address checked.
fn rust_setup(path: &Path, w: &Workload, chain: &ChainParams) -> (RustWallet, usize) {
    let store = Store::create(path).expect("create store");
    store
        .update(|tx| {
            Manager::create(
                tx,
                chain,
                &w.seed,
                PUB_PASS,
                PRIV_PASS,
                &mut OsRandom,
                &Limits::DEFAULT,
            )
            .expect("create manager");
            Ok::<_, StoreError>(())
        })
        .expect("commit creation");
    let mgr = open_manager(&store, chain);
    let genesis = simnet_genesis();
    store
        .update(|tx| {
            mgr.unlock(PRIV_PASS).expect("unlock");
            for a in 1..w.accounts {
                let n = mgr
                    .new_account(tx, format!("acct-{a}").as_bytes())
                    .expect("new account");
                assert_eq!(n, a);
            }
            for r in &w.ranges {
                if let Some(last) = r.hashes.len().checked_sub(1) {
                    mgr.sync_account_to_addr_index(
                        tx,
                        r.account,
                        u32::try_from(last).expect("index"),
                        r.branch,
                    )
                    .expect("sync");
                }
            }
            mgr.lock().expect("lock");
            let header = HeaderBytes(genesis.serialize());
            let row = BlockRow {
                hash: Hash32(genesis.block_hash().0),
                time: i64::from(genesis.timestamp),
                vote_bits: genesis.vote_bits,
                invalid: false,
                wallet_txs: 0,
            };
            tx.connect_block(
                0,
                ChainBlock {
                    row: &row,
                    header: &header,
                    filter: None,
                },
            )?;
            Ok::<_, StoreError>(())
        })
        .expect("commit setup");
    let mut mismatches = 0;
    {
        let mut tx = store.read().expect("read");
        for r in &w.ranges {
            for (i, h) in r.hashes.iter().enumerate() {
                let addr = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(h, chain)
                    .expect("address");
                match mgr.address(&mut tx, &addr) {
                    Ok(ma)
                        if ma.account() == r.account
                            && u32::from(ma.internal()) == r.branch
                            && ma.index() == Some(u32::try_from(i).expect("index")) => {}
                    _ => mismatches += 1,
                }
            }
        }
    }
    (
        RustWallet {
            store,
            mgr,
            chain: *chain,
        },
        mismatches,
    )
}

fn bump(old: Option<BalanceRow>, amount: i64, sign: i64) -> Option<BalanceRow> {
    let o = old.unwrap_or_default();
    let row = BalanceRow {
        sum: o.sum + sign * amount,
        count: o.count + sign,
    };
    assert!(row.count >= 0, "balance count below zero");
    (row.count != 0).then_some(row)
}

/// The running sums of ADR-0003 Decision item 4: the class total and, for
/// a depth-dependent class, the per-height row; a row whose count reaches
/// zero is removed.
fn bal_delta(
    tx: &mut WriteTx<'_>,
    account: u32,
    class: BalanceClass,
    height: u32,
    amount: i64,
    sign: i64,
) -> Result<(), StoreError> {
    let k = BalanceKey { account, class };
    match bump(tx.get::<Balances>(&k)?, amount, sign) {
        Some(r) => tx.put::<Balances>(&k, &r)?,
        None => {
            tx.delete::<Balances>(&k)?;
        }
    }
    if class.is_windowed() {
        let k = BalanceHeightKey {
            account,
            class,
            height,
        };
        match bump(tx.get::<BalanceHeights>(&k)?, amount, sign) {
            Some(r) => tx.put::<BalanceHeights>(&k, &r)?,
            None => {
                tx.delete::<BalanceHeights>(&k)?;
            }
        }
    }
    Ok(())
}

/// One mined transaction, as `processTransactionRecord` then
/// `InsertMinedTx` and `AddCredit` handle it (storebench's `processTx`):
/// the inputs that spend wallet outputs become debits, the record goes
/// under the block, and each output paying a recorded address becomes a
/// credit, followed by `MarkUsed` and the gap-limit sync of its branch.
fn process_tx(
    tx: &mut WriteTx<'_>,
    rw: &RustWallet,
    bytes: &[u8],
    height: u32,
) -> Result<(), StoreError> {
    let (msg, _) = MsgTx::from_bytes(bytes).expect("own transaction");
    let hash = Hash32(msg.tx_hash().0);
    let mut block = tx.block(height)?.expect("block of the transaction");
    let seq = block.wallet_txs;
    for (i, input) in msg.tx_in.iter().enumerate() {
        let prev = &input.previous_out_point;
        let op = Outpoint {
            hash: Hash32(prev.hash.0),
            index: prev.index,
        };
        let Some(u) = tx.get::<Unspent>(&op)? else {
            continue;
        };
        let i = u32::try_from(i).expect("input index");
        let ck = CreditKey {
            hash: op.hash,
            height: u.height,
            index: op.index,
        };
        let mut c = tx
            .get::<Credits>(&ck)?
            .expect("the credit of an unspent output");
        c.spender = Some(Spender {
            hash,
            height,
            input: i,
        });
        tx.put::<Credits>(&ck, &c)?;
        tx.put::<Debits>(
            &CreditKey {
                hash,
                height,
                index: i,
            },
            &DebitRow {
                amount: u.amount,
                credit: ck,
            },
        )?;
        tx.put::<AcctHistory>(
            &AcctHistoryKey {
                account: u.account,
                height,
                position: seq,
            },
            &hash,
        )?;
        tx.delete::<Unspent>(&op)?;
        bal_delta(tx, u.account, u.class, u.height, u.amount, -1)?;
    }
    block.wallet_txs += 1;
    tx.put::<Blocks>(&Height(height), &block)?;
    tx.put::<History>(
        &HistoryKey {
            height,
            position: seq,
        },
        &hash,
    )?;
    tx.put::<TxMined>(
        &TxRecKey { hash, height },
        &TxMinedRow {
            position: seq,
            received: 1_700_000_000,
            tx: bytes.to_vec(),
        },
    )?;
    // dcroxide's `MsgTx.PkScriptLocs` (the private copy this file kept
    // before the dcroxide pin had it is gone).
    let locs = msg.pk_script_locs();
    for (i, out) in msg.tx_out.iter().enumerate() {
        let (class, addrs) = stdscript::extract_addrs(out.version, &out.pk_script, &rw.chain);
        if class == ScriptType::NonStandard || out.value == 0 {
            continue;
        }
        for addr in addrs {
            let ma = match rw.mgr.address(&mut *tx, &addr) {
                Ok(ma) => ma,
                Err(e) if e.is_kind(Kind::NotExist) => continue,
                Err(e) => panic!("address lookup: {e}"),
            };
            let index = u32::try_from(i).expect("output index");
            let account = ma.account();
            let flags = CreditFlags {
                change: ma.internal(),
                tag: OutputTag::NonStake,
                coinbase: false,
                expiry: msg.expiry != 0,
            };
            let script = ScriptLoc {
                offset: u32::try_from(locs[i]).expect("offset"),
                len: u16::try_from(out.pk_script.len()).expect("script length"),
                class: 1,
            };
            tx.put::<Credits>(
                &CreditKey {
                    hash,
                    height,
                    index,
                },
                &CreditRow {
                    amount: out.value,
                    flags,
                    account,
                    script,
                    spender: None,
                },
            )?;
            tx.put::<Unspent>(
                &Outpoint { hash, index },
                &UnspentRow {
                    height,
                    account,
                    amount: out.value,
                    flags,
                    class: BalanceClass::MinedRegular,
                    script,
                },
            )?;
            bal_delta(
                tx,
                account,
                BalanceClass::MinedRegular,
                height,
                out.value,
                1,
            )?;
            tx.put::<AcctHistory>(
                &AcctHistoryKey {
                    account,
                    height,
                    position: seq,
                },
                &hash,
            )?;
            // markUsedAddress (wallet/addresses.go:718).
            rw.mgr.mark_used(tx, &addr).expect("mark used");
            let props = rw
                .mgr
                .account_properties(&mut *tx, account)
                .expect("account properties");
            let (last_used, branch) = if ma.internal() {
                (props.last_used_internal_index, 1)
            } else {
                (props.last_used_external_index, 0)
            };
            rw.mgr
                .sync_account_to_addr_index(
                    tx,
                    account,
                    (HARDENED - 1).min(last_used.wrapping_add(GAP)),
                    branch,
                )
                .expect("gap sync");
        }
    }
    Ok(())
}

/// `extendMainChain` with the block's transactions.
fn ingest_block(tx: &mut WriteTx<'_>, rw: &RustWallet, b: &Block) -> Result<(), StoreError> {
    let header = HeaderBytes(b.header);
    let time = u32::from_le_bytes(b.header[136..140].try_into().expect("4 bytes"));
    let row = BlockRow {
        hash: Hash32(b.hash),
        time: i64::from(time),
        vote_bits: u16::from_le_bytes([b.header[100], b.header[101]]),
        invalid: false,
        wallet_txs: 0,
    };
    let mut key = [0u8; 16];
    key.copy_from_slice(&b.header[36..52]);
    tx.connect_block(
        b.height,
        ChainBlock {
            row: &row,
            header: &header,
            filter: Some(&CFilterRow {
                key,
                filter: Vec::new(),
            }),
        },
    )?;
    for bytes in &b.txs {
        process_tx(tx, rw, bytes, b.height)?;
    }
    Ok(())
}

/// Ingests `blocks`, `batch` per durable write transaction, as storebench's
/// `ingest` groups them; the time of each commit.
fn rust_ingest(rw: &RustWallet, blocks: &[Block], batch: usize) -> Vec<f64> {
    let mut times = Vec::new();
    for chunk in blocks.chunks(batch) {
        let t0 = Instant::now();
        rw.store
            .update(|tx| {
                for b in chunk {
                    ingest_block(tx, rw, b)?;
                }
                if let Some(last) = chunk.last() {
                    tx.set_processed_marker(&Hash32(last.hash))?;
                }
                Ok::<_, StoreError>(())
            })
            .expect("ingest commit");
        times.push(ns(t0.elapsed()));
    }
    times
}

// ---------------------------------------------------------------------------
// The Rust side's queries

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Bal {
    spendable: i64,
    immature_coinbase: i64,
    immature_stake: i64,
    locked: i64,
    voting_authority: i64,
    unconfirmed: i64,
    total: i64,
}

impl Bal {
    fn text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {}",
            self.spendable,
            self.immature_coinbase,
            self.immature_stake,
            self.locked,
            self.voting_authority,
            self.unconfirmed,
            self.total
        )
    }
}

/// The sum of a depth-dependent class over heights `from..=tip`: the
/// per-height rows inside the window, or the class total less the rows
/// below it, whichever side is shorter (ADR-0003 Decision item 4).
fn window(rtx: &ReadTx, account: u32, class: BalanceClass, from: i64, tip: i64, total: i64) -> i64 {
    if from <= 0 {
        return total;
    }
    let from = u32::try_from(from).unwrap_or(u32::MAX);
    let sum = |lo: u32, hi: u32| -> i64 {
        let lo = BalanceHeightKey {
            account,
            class,
            height: lo,
        };
        let hi = BalanceHeightKey {
            account,
            class,
            height: hi,
        };
        rtx.range::<BalanceHeights>(Bound::Included(&lo), Bound::Included(&hi))
            .expect("range")
            .map(|r| r.expect("row").1.sum)
            .sum()
    };
    if tip - i64::from(from) < i64::from(from) {
        sum(from, u32::MAX)
    } else {
        total - sum(0, from - 1)
    }
}

/// `AccountBalance` from the running sums (the spike's `account_balance`,
/// for the classes this workload writes); `None` when the account has
/// none.
fn account_balance(rtx: &ReadTx, account: u32, min_conf: i32, tip: u32) -> Option<Bal> {
    let mut present = false;
    let mut b = Bal::default();
    let tip = i64::from(tip);
    for row in rtx
        .prefix::<Balances>(&account.to_be_bytes())
        .expect("balances")
    {
        let (k, v) = row.expect("balance row");
        present |= v.count != 0;
        match k.class {
            BalanceClass::MinedRegular => {
                let unconf = if min_conf <= 0 {
                    0
                } else {
                    window(
                        rtx,
                        account,
                        k.class,
                        tip + 2 - i64::from(min_conf),
                        tip,
                        v.sum,
                    )
                };
                b.spendable += v.sum - unconf;
                b.total += v.sum;
            }
            other => panic!("balance class {other:?} is not written by this workload"),
        }
    }
    present.then_some(b)
}

fn account_balances(rtx: &ReadTx, min_conf: i32, tip: u32) -> BTreeMap<u32, Bal> {
    let mut accounts: Vec<u32> = Vec::new();
    for row in rtx.iter::<Balances>().expect("balances") {
        let (k, _) = row.expect("balance row");
        if accounts.last() != Some(&k.account) {
            accounts.push(k.account);
        }
    }
    accounts
        .into_iter()
        .filter_map(|a| account_balance(rtx, a, min_conf, tip).map(|b| (a, b)))
        .collect()
}

/// `TxDetails` of a mined record: the transaction decoded, its credits
/// (index, amount, spent, change) and debits (index, amount).
struct Details {
    hash: Hash32,
    height: u32,
    #[allow(
        dead_code,
        reason = "decoded as dcrwallet's TxDetails decodes it; not printed"
    )]
    tx: MsgTx,
    credits: Vec<(u32, i64, bool, bool)>,
    debits: Vec<(u32, i64)>,
}

fn mined_details(rtx: &ReadTx, hash: Hash32, height: u32) -> Details {
    let rec = rtx
        .get::<TxMined>(&TxRecKey { hash, height })
        .expect("get")
        .expect("record of a history entry");
    let (tx, _) = MsgTx::from_bytes(&rec.tx).expect("stored transaction");
    let lo = CreditKey {
        hash,
        height,
        index: 0,
    };
    let hi = CreditKey {
        hash,
        height,
        index: u32::MAX,
    };
    let credits = rtx
        .range::<Credits>(Bound::Included(&lo), Bound::Included(&hi))
        .expect("credits")
        .map(|r| {
            let (k, c) = r.expect("credit");
            (k.index, c.amount, c.spender.is_some(), c.flags.change)
        })
        .collect();
    let debits = rtx
        .range::<Debits>(Bound::Included(&lo), Bound::Included(&hi))
        .expect("debits")
        .map(|r| {
            let (k, d) = r.expect("debit");
            (k.index, d.amount)
        })
        .collect();
    Details {
        hash,
        height,
        tx,
        credits,
        debits,
    }
}

/// `RangeTransactions` over mined blocks (spec 04 section 5.16), walking
/// `history`: blocks ascending when `begin < end`, otherwise descending, a
/// block's transactions in their order either way, one batch per block,
/// stopping after `limit` transactions when it is above 0. No transaction
/// of this workload is unmined, and `begin` and `end` are never negative.
fn range_transactions(rtx: &ReadTx, begin: u32, end: u32, limit: usize) -> Vec<Vec<Details>> {
    let mut batches: Vec<Vec<Details>> = Vec::new();
    let mut n = 0usize;
    let entry = |r: purse_store::Result<(HistoryKey, Hash32)>| r.expect("history entry");
    let (lo, hi) = if begin < end {
        (begin, end)
    } else {
        (end, begin)
    };
    let lo = HistoryKey {
        height: lo,
        position: 0,
    };
    let hi = HistoryKey {
        height: hi,
        position: u32::MAX,
    };
    let iter = rtx
        .range::<History>(Bound::Included(&lo), Bound::Included(&hi))
        .expect("history");
    let mut group: Vec<(HistoryKey, Hash32)> = Vec::new();
    let mut flush = |group: &mut Vec<(HistoryKey, Hash32)>, batches: &mut Vec<Vec<Details>>| {
        if group.is_empty() {
            return false;
        }
        group.sort_by_key(|(k, _)| k.position);
        let batch: Vec<Details> = group
            .drain(..)
            .map(|(k, h)| mined_details(rtx, h, k.height))
            .collect();
        n += batch.len();
        batches.push(batch);
        limit > 0 && n >= limit
    };
    if begin < end {
        for r in iter {
            let (k, h) = entry(r);
            if group.first().is_some_and(|(g, _)| g.height != k.height)
                && flush(&mut group, &mut batches)
            {
                return batches;
            }
            group.push((k, h));
        }
    } else {
        for r in iter.rev() {
            let (k, h) = entry(r);
            if group.first().is_some_and(|(g, _)| g.height != k.height)
                && flush(&mut group, &mut batches)
            {
                return batches;
            }
            group.push((k, h));
        }
    }
    flush(&mut group, &mut batches);
    batches
}

/// The batches as storebench's export and `hot` write them.
fn range_lines(batches: &[Vec<Details>]) -> Vec<String> {
    let mut lines = Vec::new();
    for batch in batches {
        lines.push(format!("batch {}", batch.len()));
        for d in batch {
            let mut s = format!("tx {} {}", hex(&d.hash.0), d.height);
            for (i, amount, spent, change) in &d.credits {
                let _ = write!(s, " c{i}:{amount}:{spent}:{change}");
            }
            for (i, amount) in &d.debits {
                let _ = write!(s, " d{i}:{amount}");
            }
            lines.push(s);
        }
    }
    lines
}

fn digest_text(n: usize, lines: &[String]) -> String {
    let mut h = Sha256::new();
    for l in lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    format!("{n} {}", hex(&h.finalize()))
}

// ---------------------------------------------------------------------------
// The queries both sides run

#[derive(Clone, Debug)]
struct RangeSpec {
    name: &'static str,
    begin: u32,
    end: u32,
    limit: usize,
}

#[derive(Clone, Debug)]
struct HotArgs {
    reps: Reps,
    balances: Vec<(u32, i32)>,
    all_minconfs: Vec<i32>,
    ranges: Vec<RangeSpec>,
    scripts: Vec<Vec<u8>>,
    names: Vec<String>,
    props: Vec<u32>,
    next_addr: usize,
}

impl HotArgs {
    fn new(w: &Workload, reps: Reps) -> HotArgs {
        let tip = w.tip_height();
        let mut rng = SplitMix64(0x6c6f_6f6b_7570_7331);
        let mut scripts = Vec::with_capacity(1_000);
        let all: Vec<&[u8; 20]> = w.ranges.iter().flat_map(|r| &r.hashes).collect();
        for i in 0..1_000u32 {
            if i % 10 == 9 {
                let mut h = [0u8; 20];
                rng.fill(&mut h);
                scripts.push(p2pkh(&h));
            } else {
                let j = usize::try_from(rng.below(all.len() as u64)).expect("fits");
                scripts.push(p2pkh(all[j]));
            }
        }
        let mut names: Vec<String> = (0..w.accounts)
            .map(|a| {
                if a == 0 {
                    "default".to_owned()
                } else {
                    format!("acct-{a}")
                }
            })
            .collect();
        names.push("no-such-account".to_owned());
        let names = (0..1_000).map(|i| names[i % names.len()].clone()).collect();
        let props = (0..1_000u32).map(|i| i % w.accounts).collect();
        let last = w.accounts - 1;
        HotArgs {
            reps,
            balances: vec![(0, 1), (0, 6), (last, 1), (last, 6)],
            all_minconfs: vec![1],
            ranges: vec![
                RangeSpec {
                    name: "last_1000_blocks_backward",
                    begin: tip,
                    end: tip.saturating_sub(999),
                    limit: 0,
                },
                RangeSpec {
                    name: "mid_1000_blocks_forward",
                    begin: tip / 2,
                    end: tip / 2 + 999,
                    limit: 0,
                },
                RangeSpec {
                    name: "recent_100_transactions",
                    begin: tip,
                    end: 0,
                    limit: 100,
                },
                RangeSpec {
                    name: "whole_history",
                    begin: 0,
                    end: tip,
                    limit: 0,
                },
            ],
            scripts,
            names,
            props,
            next_addr: 20,
        }
    }

    fn ranges_json(&self) -> Value {
        Value::Array(
            self.ranges
                .iter()
                .map(|r| json!({"name": r.name, "begin": r.begin, "end": r.end, "limit": r.limit}))
                .collect(),
        )
    }

    fn json(&self, db: &Path) -> Value {
        let mut v = self.reps.json();
        v["db"] = json!(db.to_str().expect("utf-8 path"));
        v["balances"] = Value::Array(
            self.balances
                .iter()
                .map(|(a, mc)| json!({"account": a, "minconf": mc}))
                .collect(),
        );
        v["all_minconfs"] = json!(self.all_minconfs);
        v["ranges"] = self.ranges_json();
        v["scripts"] = json!(self.scripts.iter().map(|s| hex(s)).collect::<Vec<_>>());
        v["names"] = json!(self.names);
        v["props"] = json!(self.props);
        v["next_addr"] = json!(self.next_addr);
        v
    }
}

type Figures = BTreeMap<String, Summary>;
type Checks = BTreeMap<String, String>;

/// The Rust half of storebench's `hot`.
fn rust_hot(rw: &RustWallet, a: &HotArgs) -> (Figures, Checks) {
    let mut figs = Figures::new();
    let mut checks = Checks::new();
    let tip = rw
        .store
        .read()
        .expect("read")
        .tip()
        .expect("tip")
        .expect("a tip")
        .height;
    for &(account, mc) in &a.balances {
        let name = format!("balance/account={account}/minconf={mc}");
        let mut got = Bal::default();
        let s = repeat(a.reps, || {
            let rtx = rw.store.read().expect("read");
            got = account_balance(&rtx, account, mc, tip).unwrap_or_default();
        });
        figs.insert(name.clone(), s);
        checks.insert(name, got.text());
    }
    for &mc in &a.all_minconfs {
        let name = format!("balance_all/minconf={mc}");
        let mut got = BTreeMap::new();
        let s = repeat(a.reps, || {
            let rtx = rw.store.read().expect("read");
            got = account_balances(&rtx, mc, tip);
        });
        figs.insert(name.clone(), s);
        let parts: Vec<String> = got
            .iter()
            .map(|(a, b)| format!("{a}:{}", b.text()))
            .collect();
        checks.insert(name, parts.join(";"));
    }
    for r in &a.ranges {
        let name = format!("range/{}", r.name);
        let mut got = Vec::new();
        let s = repeat(a.reps, || {
            let rtx = rw.store.read().expect("read");
            got = range_transactions(&rtx, r.begin, r.end, r.limit);
        });
        figs.insert(name.clone(), s);
        let n = got.iter().map(Vec::len).sum();
        checks.insert(name, digest_text(n, &range_lines(&got)));
    }
    if !a.scripts.is_empty() {
        let mut lines = Vec::new();
        let mut hits = 0;
        let s = repeat(a.reps, || {
            lines.clear();
            hits = 0;
            let mut rtx = rw.store.read().expect("read");
            for script in &a.scripts {
                let (_, addrs) = stdscript::extract_addrs(0, script, &rw.chain);
                let mut line = "-".to_owned();
                if let [addr] = addrs.as_slice()
                    && let Ok(ma) = rw.mgr.address(&mut rtx, addr)
                {
                    line = format!(
                        "{} {} {}",
                        ma.account(),
                        u8::from(ma.internal()),
                        ma.index().unwrap_or(0)
                    );
                    hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/address".to_owned(), s);
        checks.insert("lookup/address".to_owned(), digest_text(hits, &lines));
        let s = repeat(a.reps, || {
            lines.clear();
            hits = 0;
            let mut rtx = rw.store.read().expect("read");
            for script in &a.scripts {
                let (_, addrs) = stdscript::extract_addrs(0, script, &rw.chain);
                let mut line = "-".to_owned();
                if let [addr] = addrs.as_slice()
                    && let Ok(acct) = rw.mgr.addr_account(&mut rtx, addr)
                {
                    line = acct.to_string();
                    hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/addr_account".to_owned(), s);
        checks.insert("lookup/addr_account".to_owned(), digest_text(hits, &lines));
    }
    if !a.names.is_empty() {
        let mut lines = Vec::new();
        let mut hits = 0;
        let s = repeat(a.reps, || {
            lines.clear();
            hits = 0;
            let mut rtx = rw.store.read().expect("read");
            for name in &a.names {
                let mut line = "-".to_owned();
                if let Ok(acct) = rw.mgr.lookup_account(&mut rtx, name.as_bytes()) {
                    line = acct.to_string();
                    hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/account_by_name".to_owned(), s);
        checks.insert(
            "lookup/account_by_name".to_owned(),
            digest_text(hits, &lines),
        );
    }
    if !a.props.is_empty() {
        let mut lines = Vec::new();
        let s = repeat(a.reps, || {
            lines.clear();
            let mut rtx = rw.store.read().expect("read");
            for &acct in &a.props {
                let p = rw
                    .mgr
                    .account_properties(&mut rtx, acct)
                    .expect("account properties");
                lines.push(format!(
                    "{} {} {} {} {} {}",
                    p.account_number,
                    String::from_utf8_lossy(&p.account_name),
                    p.last_used_external_index,
                    p.last_used_internal_index,
                    p.last_returned_external_index,
                    p.last_returned_internal_index
                ));
            }
        });
        figs.insert("lookup/account_properties".to_owned(), s);
        checks.insert(
            "lookup/account_properties".to_owned(),
            digest_text(lines.len(), &lines),
        );
    }
    if a.next_addr > 0 {
        let mut next = 0u32;
        {
            let mut rtx = rw.store.read().expect("read");
            rw.mgr
                .for_each_account_address(&mut rtx, 0, |ma| {
                    if let Some(i) = ma.index()
                        && !ma.internal()
                        && i >= next
                    {
                        next = i + 1;
                    }
                    Ok(())
                })
                .expect("account addresses");
        }
        let first = next;
        let mut d = Vec::new();
        for _ in 0..a.next_addr {
            let child = next;
            next += 1;
            let t0 = Instant::now();
            rw.store
                .update(|tx| {
                    rw.mgr
                        .sync_account_to_addr_index(tx, 0, child, 0)
                        .expect("record one address");
                    Ok::<_, StoreError>(())
                })
                .expect("commit");
            d.push(ns(t0.elapsed()));
        }
        figs.insert("commit/next_address".to_owned(), summarize(&d));
        checks.insert("commit/next_address".to_owned(), format!("{first} {next}"));
    }
    (figs, checks)
}

/// The lines of storebench's canonical export for a store holding only what
/// this workload writes (no probes, scans, per-account histories or
/// selections are asked for; nothing is unmined, stake-invalidated or
/// imported).
fn rust_export(rw: &RustWallet, minconfs: &[i32], ranges: &[RangeSpec]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rtx = rw.store.read().expect("read");
    let tip = rtx.tip().expect("tip").expect("a tip");
    lines.push(format!("tip {} {}", tip.height, hex(&tip.hash.0)));
    lines.push(format!("keyn {}", rtx.unspent_count().expect("count")));
    let accounts: Vec<u32> = rw
        .mgr
        .for_each_account(&mut rtx)
        .expect("accounts")
        .into_iter()
        .filter(|&a| a != 0x7FFF_FFFF)
        .collect();
    for &a in &accounts {
        let p = rw.mgr.account_properties(&mut rtx, a).expect("properties");
        lines.push(format!(
            "account {a} {} {} {} {}",
            p.last_used_external_index,
            p.last_used_internal_index,
            p.last_returned_external_index,
            p.last_returned_internal_index
        ));
    }
    for &mc in minconfs {
        for (a, b) in account_balances(&rtx, mc, tip.height) {
            lines.push(format!("balance {mc} {a} {}", b.text()));
        }
    }
    let unspent: Vec<(Outpoint, UnspentRow)> = rtx
        .iter::<Unspent>()
        .expect("unspent")
        .map(|r| r.expect("unspent row"))
        .collect();
    lines.push(format!("unspent {}", unspent.len()));
    for (op, u) in &unspent {
        let rec = rtx
            .get::<TxMined>(&TxRecKey {
                hash: op.hash,
                height: u.height,
            })
            .expect("get")
            .expect("record of an unspent output");
        let off = u.script.offset as usize;
        let script = &rec.tx[off..off + usize::from(u.script.len)];
        lines.push(format!(
            "utxo {} {} 0 {} {} false {}",
            hex(&op.hash.0),
            op.index,
            u.height,
            u.amount,
            hex(script)
        ));
    }
    for r in ranges {
        lines.push(format!(
            "range {} {} {} {}",
            r.name, r.begin, r.end, r.limit
        ));
        lines.extend(range_lines(&range_transactions(
            &rtx, r.begin, r.end, r.limit,
        )));
    }
    lines.push(format!("foreach {}", unspent.len()));
    for (op, _) in &unspent {
        lines.push(format!("fe {} {} 0", hex(&op.hash.0), op.index));
    }
    for &a in &accounts {
        let mut h = Sha256::new();
        let mut n = 0;
        rw.mgr
            .for_each_account_address(&mut rtx, a, |ma| {
                h.update(ma.addr_hash());
                n += 1;
                Ok(())
            })
            .expect("account addresses");
        lines.push(format!("addrs {a} {n} {}", hex(&h.finalize())));
    }
    lines
}

// ---------------------------------------------------------------------------
// A session

const EXPORT_MINCONFS: [i32; 3] = [0, 1, 6];

fn spawn_go(bin: &Path) -> Oracle {
    Oracle::spawn_at(bin)
}

fn go_export_args(db: &Path, out: &Path, a: &HotArgs) -> Value {
    json!({
        "db": db.to_str().expect("utf-8 path"),
        "out": out.to_str().expect("utf-8 path"),
        "minconfs": EXPORT_MINCONFS,
        "probes": [],
        "scans": [],
        "ranges": a.ranges_json(),
        "by_account": [],
        "select_accounts": 0,
        "select_detail": false,
        "select_minconfs": [],
        "select_targets": [],
    })
}

fn file_size(p: &Path) -> u64 {
    fs::metadata(p).map_or(0, |m| m.len())
}

fn load_average() -> String {
    fs::read_to_string("/proc/loadavg")
        .map_or_else(|_| "unknown".to_owned(), |s| s.trim().to_owned())
}

fn affinity() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Cpus_allowed_list"))
                .map(|l| l.split_whitespace().last().unwrap_or("").to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One line per side, round and figure.
struct Record {
    lines: Vec<String>,
}

impl Record {
    fn add(&mut self, side: &str, round: &str, name: &str, s: &Summary) {
        self.lines.push(format!(
            "{side}\t{round}\t{name}\t{}\t{:.0}\t{:.0}\t{:.0}\t{:.0}",
            s.n, s.p50, s.p90, s.min, s.max
        ));
    }
}

/// Compares two sets of check values, panicking at the first difference.
fn assert_checks_equal(go: &Checks, rust: &Checks, round: usize) {
    assert_eq!(
        go.keys().collect::<Vec<_>>(),
        rust.keys().collect::<Vec<_>>(),
        "round {round}: the two sides ran different queries"
    );
    for (k, g) in go {
        assert_eq!(Some(g), rust.get(k), "round {round}: {k} differs");
    }
}

fn compare_exports(go_path: &Path, rust: &[String]) {
    let go = fs::read_to_string(go_path).expect("read Go export");
    let go: Vec<&str> = go.lines().collect();
    for (i, (g, r)) in go.iter().zip(rust).enumerate() {
        assert_eq!(*g, r.as_str(), "export line {} differs", i + 1);
    }
    assert_eq!(go.len(), rust.len(), "export lengths differ");
}

struct SessionResult {
    record: Record,
    notes: String,
    table: String,
}

fn run_session(dir: &Path, p: &Preset, reps: Reps, rounds: usize) -> Option<SessionResult> {
    let mut first_oracle = oracle_or_skip("storebench")?;
    let bin = oracle_bin_or_skip("storebench")?;
    let info = first_oracle.build_info();
    drop(first_oracle);
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let mut notes = String::new();
    let _ = writeln!(notes, "started (unix): {}", unix_now());
    let _ = writeln!(notes, "load average at start: {}", load_average());
    let _ = writeln!(notes, "cpus allowed: {}", affinity());
    let _ = writeln!(notes, "go: {}", info.go);
    for m in &info.deps {
        if m.path.contains("dcrwallet") || m.path.contains("bbolt") {
            let _ = writeln!(notes, "go module: {} {}", m.path, m.version);
        }
    }
    let _ = writeln!(notes, "redb: {}", purse_store::REDB_VERSION);
    let _ = writeln!(notes, "preset: {p:?}; reps: {reps:?}; rounds: {rounds}");

    let t0 = Instant::now();
    let w = generate(p, &chain);
    let _ = writeln!(
        notes,
        "workload: {} blocks ({} at the tip), {} transactions ({} receives, {} sends), {} addresses; generated in {:.1} s",
        w.tip_height(),
        w.tip.len(),
        w.transactions(),
        w.receives,
        w.sends,
        w.addresses(),
        t0.elapsed().as_secs_f64()
    );
    let wl_sync = dir.join("sync.wl");
    let wl_tip = dir.join("tip.wl");
    write_workload(&wl_sync, &w, &w.sync, false);
    write_workload(&wl_tip, &w, &w.tip, true);
    let path_str = |p: &Path| p.to_str().expect("utf-8 path").to_owned();
    let mut record = Record { lines: Vec::new() };

    // Setup and history sync, once per side.
    let go_dir = dir.join("go");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&go_dir).expect("mkdir");
    fs::create_dir_all(&rust_dir).expect("mkdir");
    let go_base = go_dir.join("base.db");
    let rust_base = rust_dir.join("base.redb");
    {
        let mut go = spawn_go(&bin);
        let r = go.call(
            "setup",
            json!({"db": path_str(&go_base), "workload": path_str(&wl_sync), "seed": hex(&w.seed)}),
        );
        assert_eq!(
            r["mismatches"],
            json!(0),
            "Go recorded other addresses: {r}"
        );
        let r = go.call(
            "ingest",
            json!({"db": path_str(&go_base), "workload": path_str(&wl_sync), "gap": GAP}),
        );
        let blocks = r["sync_blocks"].as_f64().expect("sync_blocks").max(1.0);
        let per_block = r["sync_ns"].as_f64().expect("sync_ns") / blocks;
        let s = Summary {
            n: 1,
            min: per_block,
            p50: per_block,
            p90: per_block,
            max: per_block,
        };
        record.add("go", "setup", "ingest/history_sync_per_block", &s);
        let _ = writeln!(notes, "go history sync: {r}");
    }
    {
        let (rw, mismatches) = rust_setup(&rust_base, &w, &chain);
        assert_eq!(
            mismatches, 0,
            "the Rust address manager recorded other addresses"
        );
        let times = rust_ingest(&rw, &w.sync, w.batch as usize);
        let total: f64 = times.iter().sum();
        let per_block = total / w.sync.len().max(1) as f64;
        let s = Summary {
            n: 1,
            min: per_block,
            p50: per_block,
            p90: per_block,
            max: per_block,
        };
        record.add("rust", "setup", "ingest/history_sync_per_block", &s);
        let _ = writeln!(
            notes,
            "rust history sync: {} commits, {:.3} s",
            times.len(),
            total / 1e9
        );
        rw.store.close().expect("close");
    }
    let _ = writeln!(
        notes,
        "file sizes after the history sync: go {} bytes, rust {} bytes",
        file_size(&go_base),
        file_size(&rust_base)
    );

    let args = HotArgs::new(&w, reps);
    let mut go_checks: Option<Checks> = None;
    let mut rust_checks: Option<Checks> = None;
    let mut go_export: Option<PathBuf> = None;
    let mut rust_export_lines: Option<Vec<String>> = None;
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            ["rust", "go"]
        } else {
            ["go", "rust"]
        };
        for side in order {
            let label = round.to_string();
            if side == "go" {
                let work = go_dir.join("work.db");
                fs::copy(&go_base, &work).expect("copy");
                let mut go = spawn_go(&bin);
                let r = go.call(
                    "ingest",
                    json!({"db": path_str(&work), "workload": path_str(&wl_tip), "gap": GAP}),
                );
                record.add(
                    "go",
                    &label,
                    "commit/tip_block",
                    &go_summary(&r["tip_block"]),
                );
                if go_export.is_none() {
                    let out = dir.join("export-go.txt");
                    go.call("export", go_export_args(&work, &out, &args));
                    go_export = Some(out);
                }
                let r = go.call("hot", args.json(&work));
                for t in r["timings"].as_array().expect("timings") {
                    let name = t["name"].as_str().expect("name");
                    record.add("go", &label, name, &go_summary(&t["summary"]));
                }
                let checks: Checks = r["checks"]
                    .as_object()
                    .expect("checks")
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().expect("check text").to_owned()))
                    .collect();
                if let Some(rc) = &rust_checks {
                    assert_checks_equal(&checks, rc, round);
                }
                go_checks.get_or_insert(checks);
                drop(go);
                let _ = fs::remove_file(&work);
            } else {
                let work = rust_dir.join("work.redb");
                fs::copy(&rust_base, &work).expect("copy");
                let rw = open_rust(&work, &chain);
                let times = rust_ingest(&rw, &w.tip, 1);
                record.add("rust", &label, "commit/tip_block", &summarize(&times));
                if rust_export_lines.is_none() {
                    rust_export_lines = Some(rust_export(&rw, &EXPORT_MINCONFS, &args.ranges));
                }
                let (figs, checks) = rust_hot(&rw, &args);
                for (name, s) in &figs {
                    record.add("rust", &label, name, s);
                }
                if let Some(gc) = &go_checks {
                    assert_checks_equal(gc, &checks, round);
                }
                rust_checks.get_or_insert(checks);
                rw.store.close().expect("close");
                let _ = fs::remove_file(&work);
            }
            if let (Some(g), Some(r)) = (&go_export, &rust_export_lines) {
                compare_exports(g, r);
            }
        }
    }
    let _ = writeln!(notes, "load average at end: {}", load_average());
    let _ = writeln!(notes, "ended (unix): {}", unix_now());
    let table = table(&record);
    Some(SessionResult {
        record,
        notes,
        table,
    })
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn human(nanos: f64) -> String {
    if nanos >= 1e9 {
        format!("{:.2} s", nanos / 1e9)
    } else if nanos >= 1e6 {
        format!("{:.2} ms", nanos / 1e6)
    } else if nanos >= 1e3 {
        format!("{:.1} µs", nanos / 1e3)
    } else {
        format!("{nanos:.0} ns")
    }
}

/// Per figure: each side's median of its rounds' medians, their range, and
/// Rust's median over Go's.
fn table(record: &Record) -> String {
    let mut by: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
    for l in &record.lines {
        let f: Vec<&str> = l.split('\t').collect();
        let p50: f64 = f[4].parse().expect("p50");
        by.entry(f[2].to_owned())
            .or_default()
            .entry(f[0].to_owned())
            .or_default()
            .push(p50);
    }
    let mut t = String::from(
        "| figure | Go median (range) | Rust median (range) | Rust/Go |\n|---|---|---|---|\n",
    );
    for (name, sides) in &mut by {
        let mut cell = |side: &str| -> (String, f64) {
            match sides.get_mut(side) {
                Some(v) if !v.is_empty() => {
                    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = v.iter().copied().fold(0.0, f64::max);
                    let m = median(v);
                    (format!("{} ({}–{})", human(m), human(lo), human(hi)), m)
                }
                _ => ("—".to_owned(), 0.0),
            }
        };
        let (g, gm) = cell("go");
        let (r, rm) = cell("rust");
        let ratio = if gm > 0.0 {
            format!("{:.4}", rm / gm)
        } else {
            "—".to_owned()
        };
        let _ = writeln!(t, "| {name} | {g} | {r} | {ratio} |");
    }
    t
}

fn scratch(label: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-udb")
        .join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The whole pipeline at a small size, one repetition of everything: the
/// two databases' exports equal line for line, and every query's answer
/// equal in both rounds of each side.
#[test]
fn same_data_and_answers_on_both_sides() {
    let dir = scratch("measure-small");
    let reps = Reps {
        min: 1,
        max: 1,
        budget: Duration::ZERO,
    };
    let Some(result) = run_session(&dir, &Preset::small(), reps, 2) else {
        return;
    };
    // Every figure of both sides was produced.
    let names = |side: &str| -> Vec<String> {
        let mut v: Vec<String> = result
            .record
            .lines
            .iter()
            .filter(|l| l.starts_with(&format!("{side}\t")))
            .map(|l| l.split('\t').nth(2).expect("name").to_owned())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(names("go"), names("rust"));
    assert_eq!(names("go").len(), 16, "{:?}", names("go"));
    // The export the two sides matched covers credits, debits, spent and
    // unspent outputs, change, and every account's addresses.
    let export = fs::read_to_string(dir.join("export-go.txt")).expect("Go export");
    let count = |prefix: &str| export.lines().filter(|l| l.starts_with(prefix)).count();
    assert!(count("utxo ") > 50, "{} unspent outputs", count("utxo "));
    assert!(count("tx ") > 200, "{} transaction lines", count("tx "));
    assert_eq!(count("addrs "), 3);
    assert_eq!(count("balance "), 9);
    for needle in [" d0:", ":true:false", ":false:true", ":true:true"] {
        assert!(export.contains(needle), "no {needle:?} in the export");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// One measured session (see the module documentation).
#[test]
#[ignore = "a measurement: release build, PURSE_STORE_BENCH_DIR, pinned, minutes"]
fn hot_paths_against_dcrwallet() {
    let base = PathBuf::from(
        std::env::var_os("PURSE_STORE_BENCH_DIR")
            .expect("set PURSE_STORE_BENCH_DIR to a directory outside the repository"),
    );
    let dir = base.join(format!("session-{}", unix_now()));
    fs::create_dir_all(&dir).expect("create session directory");
    let rounds = std::env::var("PURSE_UDB_MEASURE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let reps = Reps {
        min: 5,
        max: 200,
        budget: Duration::from_secs(1),
    };
    let Some(result) = run_session(&dir, &Preset::measure(), reps, rounds) else {
        return;
    };
    let mut tsv = String::from("side\tround\tfigure\tn\tp50_ns\tp90_ns\tmin_ns\tmax_ns\n");
    for l in &result.record.lines {
        tsv.push_str(l);
        tsv.push('\n');
    }
    fs::write(dir.join("results.tsv"), tsv).expect("write results");
    fs::write(dir.join("notes.txt"), &result.notes).expect("write notes");
    fs::write(dir.join("table.md"), &result.table).expect("write table");
    for f in ["go/base.db", "rust/base.redb", "sync.wl", "tip.wl"] {
        let _ = fs::remove_file(dir.join(f));
    }
    eprintln!("{}\n{}", result.notes, result.table);
}
