// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The measurements behind ADR-0003's Phase 2 addendum (brief §6 Phase 2
//! exit, item (e)): ADR-0014 Open point 6, main-chain headers and filters
//! keyed by height or by hash on a grown file, and ADR-0003 Open point 9,
//! fewer tables per commit.
//!
//! Both are ignored tests, meant for a release build on a quiet machine:
//!
//! ```sh
//! PURSE_STORE_BENCH_DIR=/some/dir/outside/the/repository \
//!   cargo test --release -p purse-store --test measure -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Every store is a real file in `PURSE_STORE_BENCH_DIR`, committed with
//! `Durability::Immediate` through [`Store::update`], under a backend that
//! wraps redb's own file backend and counts the bytes written, the write
//! calls and the syncs. Byte and sync counts are deterministic; times
//! depend on the machine and its load. Results go to
//! `PURSE_STORE_BENCH_DIR/*.txt` and to standard error.
//!
//! **Headers and filters (`headers_and_filters_by_height_or_hash`).** Five
//! ways of writing the header-and-filter tables, each grown to `N` blocks
//! (`PURSE_STORE_BENCH_BLOCKS`, default 1,050,000, about mainnet's height
//! when the parity pin's chaincfg was cut) in batches of 2,000 blocks, one
//! commit per batch, with a 180-byte header and a committed filter of 170 to
//! 369 bytes (270 on average, ADR-0003's assumption) per block:
//!
//! - `hash`: the schema as first built, before height keys. `headers` and
//!   `cfilters` keyed by block hash; `blocks` by height; `block_by_hash`.
//! - `height`: `headers` and `cfilters` keyed by height (u32 BE), `blocks`
//!   by height, `block_by_hash`: the layout schema version 1 now has
//!   (`purse_store::chain`), written here as raw rows rather than through
//!   the chain operations, so without their reads and checks. The detached
//!   tables, keyed by hash, stay empty here.
//! - `merged` (Open point 9): one table keyed by height whose value is the
//!   block row, the header and the filter, and `block_by_hash`: two tables
//!   a commit instead of four.
//! - `chain`: the height layout as built, written through the chain
//!   operations (`WriteTx::connect_blocks`, each batch a durable
//!   `Store::update`), with their reads and checks, and read through the
//!   typed readers (`ReadTx::header_at`, `ReadTx::header`,
//!   `ReadTx::cfilters_in`). The headers are the same bytes as the other
//!   layouts' with the two fields the operations check (height, previous
//!   block) set, so the sizes are the same.
//! - `chain-relaxed`: `chain` with the ingest as initial sync makes it,
//!   each batch an `InitialSync::batch` (`SyncBatch::connect_blocks`) at
//!   the default interval, one durable commit in 50, and
//!   `InitialSync::finish` at the end; the tip commits after it are durable
//!   `Store::update`s, as after a real sync.
//!
//! `PURSE_STORE_BENCH_LAYOUTS` (comma-separated names) runs only those.
//!
//! On each grown file: the cost of the last 50 batches; 200 single-block
//! commits at the tip (steady state after sync); and warm reads: a header
//! by height, a header by hash, and runs of 2,000 consecutive filters by
//! height (`GetMainChainCFilters`' shape during a rescan). On the two
//! `chain` files, also durable reorganisations at the tip: 100 commits per
//! depth (1 and 6 blocks), alternating between two branches, so that half
//! detach the main chain's blocks to the detached tables and half bring
//! them back from there.
//!
//! **Tables per commit (`commit_cost_by_tables_touched`).** Fifteen tables
//! grown to 100,000 rows each (36-byte random keys, 60-byte values, a
//! wallet's credit and unspent rows' shape); then commits of 30 new rows,
//! spread evenly over 1, 2, 4, 8 or 15 of the tables, 100 commits per
//! spread, the spreads interleaved. The extra cost of each table a commit
//! touches is what merging tables would save.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "measurement code: counters and sizes"
)]

use std::fmt::Write as _;
use std::fs::OpenOptions as FsOpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use purse_store::{
    BlockRow, CFilterRow, ChainBlock, Codec as _, Error, Hash32, HeaderBytes, Height, InitialSync,
    OpenOptions, Store, TableId, Tip,
};
use purse_testutil::SplitMix64;
use redb::backends::FileBackend;
use redb::{BackendError, StorageBackend};

/// redb's file backend, counting what reaches it.
#[derive(Debug)]
struct Counting {
    inner: FileBackend,
    bytes: AtomicU64,
    writes: AtomicU64,
    syncs: AtomicU64,
}

#[derive(Debug, Clone)]
struct Counted(Arc<Counting>);

#[derive(Clone, Copy, Debug, Default)]
struct Io {
    bytes: u64,
    writes: u64,
    syncs: u64,
}

impl Counting {
    fn io(&self) -> Io {
        Io {
            bytes: self.bytes.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            syncs: self.syncs.load(Ordering::Relaxed),
        }
    }
}

impl Io {
    fn since(self, before: Io) -> Io {
        Io {
            bytes: self.bytes - before.bytes,
            writes: self.writes - before.writes,
            syncs: self.syncs - before.syncs,
        }
    }
}

impl StorageBackend for Counted {
    fn len(&self) -> Result<u64, BackendError> {
        self.0.inner.len()
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), BackendError> {
        self.0.inner.read(offset, out)
    }

    fn set_len(&self, len: u64) -> Result<(), BackendError> {
        self.0.inner.set_len(len)
    }

    fn sync_data(&self) -> Result<(), BackendError> {
        self.0.syncs.fetch_add(1, Ordering::Relaxed);
        self.0.inner.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), BackendError> {
        self.0.writes.fetch_add(1, Ordering::Relaxed);
        self.0.bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
        self.0.inner.write(offset, data)
    }

    fn close(&self) -> Result<(), BackendError> {
        self.0.inner.close()
    }
}

fn bench_dir() -> PathBuf {
    let d = PathBuf::from(
        std::env::var_os("PURSE_STORE_BENCH_DIR")
            .expect("set PURSE_STORE_BENCH_DIR to a directory outside the repository"),
    );
    std::fs::create_dir_all(&d).expect("bench dir");
    d
}

/// A new store in a file at `path`, behind a counting backend.
fn create(path: &Path) -> (Store, Arc<Counting>) {
    let _ = std::fs::remove_file(path);
    let file = FsOpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create file");
    let c = Arc::new(Counting {
        inner: FileBackend::new(file),
        bytes: AtomicU64::new(0),
        writes: AtomicU64::new(0),
        syncs: AtomicU64::new(0),
    });
    let store = OpenOptions::new()
        .create_on_backend(Counted(c.clone()), |_| Ok(()))
        .expect("create store");
    (store, c)
}

fn median(v: &mut [Duration]) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn pct(v: &mut [Duration], p: usize) -> Duration {
    v.sort();
    v[(v.len() * p / 100).min(v.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn write_out(name: &str, text: &str) {
    let p = bench_dir().join(name);
    std::fs::write(&p, text).expect("write results");
    eprintln!("{text}");
    eprintln!("written to {}", p.display());
}

// Headers and filters.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Hash,
    Height,
    Merged,
    Chain,
    ChainRelaxed,
}

impl Layout {
    fn name(self) -> &'static str {
        match self {
            Layout::Hash => "hash",
            Layout::Height => "height",
            Layout::Merged => "merged",
            Layout::Chain => "chain",
            Layout::ChainRelaxed => "chain-relaxed",
        }
    }

    /// Written and read through the chain operations and typed readers.
    fn typed(self) -> bool {
        matches!(self, Layout::Chain | Layout::ChainRelaxed)
    }
}

struct Block {
    height: u32,
    hash: [u8; 32],
    header: [u8; 180],
    filter: Vec<u8>,
    row: Vec<u8>,
}

/// The block at `height`: the same bytes for every layout.
fn block(height: u32) -> Block {
    let mut r = SplitMix64(0x6865_6164_6572_7300 ^ u64::from(height));
    let mut hash = [0u8; 32];
    r.fill(&mut hash);
    let mut header = [0u8; 180];
    r.fill(&mut header);
    let mut key = [0u8; 16];
    r.fill(&mut key);
    let len = 170 + r.below(200) as usize;
    let filter = CFilterRow {
        key,
        filter: r.bytes(len),
    }
    .encode()
    .to_vec();
    let row = BlockRow {
        hash: purse_store::Hash32(hash),
        time: 1_454_954_400 + i64::from(height) * 300,
        vote_bits: 1,
        invalid: false,
        wallet_txs: 0,
    }
    .encode()
    .to_vec();
    Block {
        height,
        hash,
        header,
        filter,
        row,
    }
}

/// A block's typed rows for the chain operations.
type Typed = (BlockRow, HeaderBytes, CFilterRow);

/// The typed rows of `blocks`, consecutive heights, each header stating
/// its height and naming the block before it as its previous block.
fn typed(blocks: &[Block]) -> Vec<Typed> {
    let mut prev = match blocks.first() {
        Some(b) if b.height > 0 => block(b.height - 1).hash,
        _ => [0; 32],
    };
    blocks
        .iter()
        .map(|b| {
            let mut header = b.header;
            header[HeaderBytes::PREV_BLOCK_OFFSET..HeaderBytes::PREV_BLOCK_OFFSET + 32]
                .copy_from_slice(&prev);
            header[HeaderBytes::HEIGHT_OFFSET..HeaderBytes::HEIGHT_OFFSET + 4]
                .copy_from_slice(&b.height.to_le_bytes());
            prev = b.hash;
            (
                BlockRow::decode(&b.row).expect("row"),
                HeaderBytes(header),
                CFilterRow::decode(&b.filter).expect("filter"),
            )
        })
        .collect()
}

fn as_chain(v: &[Typed]) -> Vec<ChainBlock<'_>> {
    v.iter()
        .map(|(row, header, filter)| ChainBlock {
            row,
            header,
            filter: Some(filter),
        })
        .collect()
}

fn put_block(
    tx: &mut purse_store::WriteTx<'_>,
    layout: Layout,
    b: &Block,
) -> purse_store::Result<()> {
    let hk = Height(b.height).encode();
    match layout {
        Layout::Hash => {
            tx.put_raw(TableId::Headers, &b.hash, &b.header)?;
            tx.put_raw(TableId::CFilters, &b.hash, &b.filter)?;
            tx.put_raw(TableId::Blocks, &hk, &b.row)?;
        }
        Layout::Height => {
            tx.put_raw(TableId::Headers, &hk, &b.header)?;
            tx.put_raw(TableId::CFilters, &hk, &b.filter)?;
            tx.put_raw(TableId::Blocks, &hk, &b.row)?;
        }
        Layout::Merged => {
            let mut v = Vec::with_capacity(b.row.len() + 180 + b.filter.len());
            v.extend_from_slice(&b.row);
            v.extend_from_slice(&b.header);
            v.extend_from_slice(&b.filter);
            tx.put_raw(TableId::Blocks, &hk, &v)?;
        }
        Layout::Chain | Layout::ChainRelaxed => unreachable!("written by connect_blocks"),
    }
    tx.put_raw(TableId::BlockByHash, &b.hash, &hk)?;
    tx.set_tip(&Tip {
        height: b.height,
        hash: purse_store::Hash32(b.hash),
    })
}

/// Commits `blocks` in one transaction, durable unless `sync` is given
/// (a relaxed initial-sync batch); its time and I/O.
fn commit_blocks(
    store: &Store,
    c: &Counting,
    layout: Layout,
    blocks: &[Block],
    sync: Option<&mut InitialSync<'_>>,
) -> (Duration, Io) {
    let rows = if layout.typed() {
        typed(blocks)
    } else {
        Vec::new()
    };
    let first = blocks.first().map_or(0, |b| b.height);
    let before = c.io();
    let t = Instant::now();
    match sync {
        Some(s) => {
            s.batch(|b| b.connect_blocks(first, &as_chain(&rows)))
                .expect("batch");
        }
        None => store
            .update(|tx| {
                if layout.typed() {
                    return tx.connect_blocks(first, &as_chain(&rows));
                }
                for b in blocks {
                    put_block(tx, layout, b)?;
                }
                Ok::<_, Error>(())
            })
            .expect("commit"),
    }
    (t.elapsed(), c.io().since(before))
}

fn header_by_height(tx: &purse_store::ReadTx, layout: Layout, h: u32) -> Vec<u8> {
    let hk = Height(h).encode();
    match layout {
        Layout::Chain | Layout::ChainRelaxed => {
            tx.header_at(h).unwrap().expect("header").0.to_vec()
        }
        Layout::Hash => {
            let row = tx.get_raw(TableId::Blocks, &hk).unwrap().expect("block");
            tx.get_raw(TableId::Headers, &row[..32])
                .unwrap()
                .expect("header")
        }
        Layout::Height => tx.get_raw(TableId::Headers, &hk).unwrap().expect("header"),
        Layout::Merged => {
            let v = tx.get_raw(TableId::Blocks, &hk).unwrap().expect("block");
            v[47..47 + 180].to_vec()
        }
    }
}

fn header_by_hash(tx: &purse_store::ReadTx, layout: Layout, hash: &[u8; 32]) -> Vec<u8> {
    match layout {
        Layout::Chain | Layout::ChainRelaxed => tx
            .header(&Hash32(*hash))
            .unwrap()
            .expect("header")
            .0
            .to_vec(),
        Layout::Hash => tx.get_raw(TableId::Headers, hash).unwrap().expect("header"),
        Layout::Height => {
            let hk = tx
                .get_raw(TableId::BlockByHash, hash)
                .unwrap()
                .expect("height");
            tx.get_raw(TableId::Headers, &hk).unwrap().expect("header")
        }
        Layout::Merged => {
            let hk = tx
                .get_raw(TableId::BlockByHash, hash)
                .unwrap()
                .expect("height");
            let v = tx.get_raw(TableId::Blocks, &hk).unwrap().expect("block");
            v[47..47 + 180].to_vec()
        }
    }
}

/// The filters of `n` consecutive heights from `from`; their total size.
fn filter_run(tx: &purse_store::ReadTx, layout: Layout, from: u32, n: u32) -> usize {
    let lo = Height(from).encode();
    let hi = Height(from + n).encode();
    let range = |t| {
        tx.range_raw(
            t,
            std::ops::Bound::Included(lo.as_slice()),
            std::ops::Bound::Excluded(hi.as_slice()),
        )
        .unwrap()
    };
    let mut total = 0;
    match layout {
        Layout::Chain | Layout::ChainRelaxed => {
            for e in tx.cfilters_in(from..from + n).unwrap() {
                total += e.unwrap().1.encode().len();
            }
        }
        Layout::Hash => {
            for e in range(TableId::Blocks) {
                let (_, row) = e.unwrap();
                total += tx
                    .get_raw(TableId::CFilters, &row[..32])
                    .unwrap()
                    .expect("filter")
                    .len();
            }
        }
        Layout::Height => {
            for e in range(TableId::CFilters) {
                total += e.unwrap().1.len();
            }
        }
        Layout::Merged => {
            for e in range(TableId::Blocks) {
                total += e.unwrap().1.len() - 47 - 180;
            }
        }
    }
    total
}

fn measure_layout(layout: Layout, n: u32, out: &mut String) {
    const BATCH: u32 = 2_000;
    let path = bench_dir().join(format!("headers-{}.redb", layout.name()));
    let (store, c) = create(&path);
    let started = Instant::now();
    let mut batch_times = Vec::new();
    let mut batch_io = Vec::new();
    let mut sync = (layout == Layout::ChainRelaxed).then(|| store.initial_sync());
    let mut h = 0;
    while h < n {
        let end = (h + BATCH).min(n);
        let blocks: Vec<Block> = (h..end).map(block).collect();
        let (t, io) = commit_blocks(&store, &c, layout, &blocks, sync.as_mut());
        batch_times.push(t);
        batch_io.push(io);
        h = end;
    }
    sync.map(InitialSync::finish).transpose().expect("finish");
    let grow = started.elapsed();
    let grow_io = c.io();
    let grown_len = std::fs::metadata(&path).unwrap().len();
    let last = batch_times.len().saturating_sub(50);
    let mut tail_times = batch_times[last..].to_vec();
    let tail_bytes: u64 =
        batch_io[last..].iter().map(|i| i.bytes).sum::<u64>() / (batch_io.len() - last) as u64;

    // Steady state: single-block commits at the tip.
    let mut tip_times = Vec::new();
    let mut tip_io = Io::default();
    for i in 0..200 {
        let (t, io) = commit_blocks(&store, &c, layout, &[block(n + i)], None);
        tip_times.push(t);
        tip_io.bytes += io.bytes;
        tip_io.writes += io.writes;
        tip_io.syncs += io.syncs;
    }
    let top = n + 200;
    let reorgs = if layout.typed() {
        reorganisations(&store, &c, top)
    } else {
        String::new()
    };

    // Warm reads.
    let tx = store.read().unwrap();
    let mut rng = SplitMix64(0x7265_6164_7300 ^ u64::from(n));
    let heights: Vec<u32> = (0..100_000)
        .map(|_| rng.below(u64::from(top)) as u32)
        .collect();
    // Warm the cache over what is read.
    for &x in &heights {
        std::hint::black_box(header_by_height(&tx, layout, x));
    }
    let t = Instant::now();
    for &x in &heights {
        std::hint::black_box(header_by_height(&tx, layout, x));
    }
    let by_height = t.elapsed() / heights.len() as u32;
    let hashes: Vec<[u8; 32]> = heights.iter().map(|&x| block(x).hash).collect();
    for x in &hashes {
        std::hint::black_box(header_by_hash(&tx, layout, x));
    }
    let t = Instant::now();
    for x in &hashes {
        std::hint::black_box(header_by_hash(&tx, layout, x));
    }
    let by_hash = t.elapsed() / hashes.len() as u32;
    let runs: Vec<u32> = (0..200)
        .map(|_| rng.below(u64::from(top - BATCH)) as u32)
        .collect();
    for &x in &runs {
        std::hint::black_box(filter_run(&tx, layout, x, BATCH));
    }
    let t = Instant::now();
    let mut bytes = 0;
    for &x in &runs {
        bytes += filter_run(&tx, layout, x, BATCH);
    }
    let per_run = t.elapsed() / runs.len() as u32;
    drop(tx);
    store.close().unwrap();
    let closed_len = std::fs::metadata(&path).unwrap().len();

    let _ = writeln!(
        out,
        "layout {}: {n} blocks in {} batches of {BATCH}\n\
         \x20 grow: {:.1} s, {:.2} GiB written in {} writes, {} syncs; file {:.0} MiB\n\
         \x20 last 50 batches: p50 {:.1} ms, p90 {:.1} ms, {:.2} MiB written per batch\n\
         \x20 200 tip commits of one block: p50 {:.2} ms, p90 {:.2} ms, {:.1} KiB and {:.1} writes per commit\n\
         \x20 warm reads: header by height {:.2} us, header by hash {:.2} us, 2,000 filters by height {:.2} ms ({} KiB)\n\
         \x20 file after close: {:.0} MiB{reorgs}",
        layout.name(),
        batch_times.len(),
        grow.as_secs_f64(),
        grow_io.bytes as f64 / (1u64 << 30) as f64,
        grow_io.writes,
        grow_io.syncs,
        grown_len as f64 / (1u64 << 20) as f64,
        ms(median(&mut tail_times)),
        ms(pct(&mut tail_times, 90)),
        tail_bytes as f64 / (1u64 << 20) as f64,
        ms(median(&mut tip_times.clone())),
        ms(pct(&mut tip_times, 90)),
        tip_io.bytes as f64 / 200.0 / 1024.0,
        tip_io.writes as f64 / 200.0,
        us(by_height),
        us(by_hash),
        ms(per_run),
        bytes / runs.len() / 1024,
        closed_len as f64 / (1u64 << 20) as f64,
    );
    eprintln!("{out}");
    let _ = std::fs::remove_file(&path);
}

/// Durable reorganisations at the tip of a grown `chain` file whose tip is
/// at `top - 1`: for each depth, 100 commits alternating between the main
/// chain's last blocks and a branch of as many, each commit detaching the
/// one and connecting the other (the main chain's blocks come back from the
/// detached tables with their filters). The main chain ends as it began,
/// each depth's side branch detached.
fn reorganisations(store: &Store, c: &Counting, top: u32) -> String {
    let tip = top - 1;
    let mut out = String::new();
    // Each depth leaves its side branch detached.
    let mut detached = 0;
    for depth in [1u32, 6] {
        detached += u64::from(depth);
        let fork = tip - depth;
        let main: Vec<Typed> = typed(&(fork + 1..=tip).map(block).collect::<Vec<_>>());
        let mut rng = SplitMix64(0x6272_616e_6368 ^ u64::from(depth));
        let mut prev = block(fork).hash;
        let side: Vec<Typed> = main
            .iter()
            .map(|b| {
                let mut hash = [0u8; 32];
                rng.fill(&mut hash);
                let mut header = b.1;
                header.0[HeaderBytes::PREV_BLOCK_OFFSET..HeaderBytes::PREV_BLOCK_OFFSET + 32]
                    .copy_from_slice(&prev);
                prev = hash;
                let row = BlockRow {
                    hash: Hash32(hash),
                    ..b.0
                };
                (row, header, b.2.clone())
            })
            .collect();
        // Back to the main chain: no filter passed, the detached one returns.
        let back: Vec<ChainBlock<'_>> = main
            .iter()
            .map(|(row, header, _)| ChainBlock {
                row,
                header,
                filter: None,
            })
            .collect();
        let to_side = as_chain(&side);
        let mut times = Vec::new();
        let mut io = Io::default();
        for i in 0..100 {
            let blocks = if i % 2 == 0 { &to_side } else { &back };
            let before = c.io();
            let t = Instant::now();
            store
                .update(|tx| tx.reorganize(fork, blocks).map(drop))
                .expect("reorganize");
            times.push(t.elapsed());
            let d = c.io().since(before);
            io.bytes += d.bytes;
            io.writes += d.writes;
        }
        let tx = store.read().unwrap();
        assert_eq!(
            tx.tip().unwrap().map(|t| t.hash.0),
            Some(block(tip).hash),
            "the main chain is back"
        );
        assert_eq!(tx.len(TableId::HeadersDetached).unwrap(), detached);
        assert_eq!(tx.len(TableId::CFiltersDetached).unwrap(), detached);
        tx.check_chain().expect("chain");
        let _ = write!(
            out,
            "\n\x20 100 reorganisations of depth {depth}: p50 {:.2} ms, p90 {:.2} ms, {:.1} KiB and {:.1} writes per commit",
            ms(median(&mut times.clone())),
            ms(pct(&mut times, 90)),
            io.bytes as f64 / 100.0 / 1024.0,
            io.writes as f64 / 100.0,
        );
    }
    out
}

#[test]
#[ignore = "a measurement: release build, minutes, about 3 GiB of disk"]
fn headers_and_filters_by_height_or_hash() {
    let n: u32 = std::env::var("PURSE_STORE_BENCH_BLOCKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_050_000);
    let mut out = format!(
        "purse-store headers-and-filters measurement, redb {}\n",
        purse_store::REDB_VERSION
    );
    let only = std::env::var("PURSE_STORE_BENCH_LAYOUTS").ok();
    for layout in [
        Layout::Hash,
        Layout::Height,
        Layout::Merged,
        Layout::Chain,
        Layout::ChainRelaxed,
    ] {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|x| x == layout.name()))
        {
            continue;
        }
        measure_layout(layout, n, &mut out);
    }
    write_out("headers-and-filters.txt", &out);
}

// Tables per commit.

const WALLET_TABLES: [TableId; 15] = [
    TableId::Accounts,
    TableId::AddrByHash,
    TableId::AddrByPath,
    TableId::AddrByAcctSha,
    TableId::TxMined,
    TableId::TxUnmined,
    TableId::History,
    TableId::AcctHistory,
    TableId::Credits,
    TableId::Unspent,
    TableId::UnminedCredits,
    TableId::Debits,
    TableId::Commitments,
    TableId::Balances,
    TableId::BalanceHeights,
];

#[test]
#[ignore = "a measurement: release build, minutes"]
fn commit_cost_by_tables_touched() {
    const ROWS: u64 = 100_000;
    const PER_COMMIT: usize = 30;
    const COMMITS: usize = 100;
    let path = bench_dir().join("tables-per-commit.redb");
    let (store, c) = create(&path);
    let mut rng = SplitMix64(0x7461_626c_6573);
    // Grow every table, 20,000 rows a commit.
    let t = Instant::now();
    for &table in &WALLET_TABLES {
        let mut left = ROWS;
        while left > 0 {
            let n = left.min(20_000);
            store
                .update(|tx| {
                    for _ in 0..n {
                        let k = rng.bytes(36);
                        let v = rng.bytes(60);
                        tx.put_raw(table, &k, &v)?;
                    }
                    Ok::<_, Error>(())
                })
                .unwrap();
            left -= n;
        }
    }
    let grow = t.elapsed();
    let grown_len = std::fs::metadata(&path).unwrap().len();

    let spreads = [1usize, 2, 4, 8, 15];
    let mut times: Vec<Vec<Duration>> = vec![Vec::new(); spreads.len()];
    let mut io: Vec<Io> = vec![Io::default(); spreads.len()];
    for _ in 0..COMMITS {
        for (si, &k) in spreads.iter().enumerate() {
            // Which tables: a random choice of `k` each commit.
            let mut tables = WALLET_TABLES.to_vec();
            for i in 0..k {
                let j = i + rng.below((tables.len() - i) as u64) as usize;
                tables.swap(i, j);
            }
            let rows: Vec<(TableId, Vec<u8>, Vec<u8>)> = (0..PER_COMMIT)
                .map(|i| (tables[i % k], rng.bytes(36), rng.bytes(60)))
                .collect();
            let before = c.io();
            let t = Instant::now();
            store
                .update(|tx| {
                    for (table, key, value) in &rows {
                        tx.put_raw(*table, key, value)?;
                    }
                    Ok::<_, Error>(())
                })
                .unwrap();
            times[si].push(t.elapsed());
            let d = c.io().since(before);
            io[si].bytes += d.bytes;
            io[si].writes += d.writes;
            io[si].syncs += d.syncs;
        }
    }
    let mut out = format!(
        "purse-store tables-per-commit measurement, redb {}\n\
         15 tables of {ROWS} rows (36-byte random keys, 60-byte values), grown in {:.1} s, file {:.0} MiB\n\
         commits of {PER_COMMIT} new rows spread evenly over k tables, {COMMITS} commits per k, interleaved\n",
        purse_store::REDB_VERSION,
        grow.as_secs_f64(),
        grown_len as f64 / (1u64 << 20) as f64,
    );
    for (si, &k) in spreads.iter().enumerate() {
        let _ = writeln!(
            out,
            "  k = {k:>2}: p50 {:.2} ms, p90 {:.2} ms, {:.1} KiB in {:.1} writes and {:.1} syncs per commit",
            ms(median(&mut times[si].clone())),
            ms(pct(&mut times[si], 90)),
            io[si].bytes as f64 / COMMITS as f64 / 1024.0,
            io[si].writes as f64 / COMMITS as f64,
            io[si].syncs as f64 / COMMITS as f64,
        );
    }
    store.close().unwrap();
    let _ = std::fs::remove_file(&path);
    write_out("tables-per-commit.txt", &out);
}
