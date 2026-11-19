// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The model-based property test of brief §6 Phase 2's exit: random
//! sequences of store operations run against the store and against an
//! in-memory model, and every observation is compared.
//!
//! An operation is a write transaction (puts, deletes, deletes through a
//! write cursor, refused puts that leave the transaction usable, and reads
//! of the transaction's own writes) that commits or is abandoned; reads in
//! a read transaction (gets, bounded ranges in either direction, prefixes,
//! counts, cursor walks); a read snapshot held across a commit; an initial
//! sync of header-and-filter batches at some durable interval, finished,
//! dropped, or cut by a power loss; a clean close and reopen; or a power
//! loss between transactions. The model is a map per table. After every
//! operation the store's whole contents must equal the model's; after a
//! power loss they must equal the model as of some commit since the last
//! durable one.
//!
//! Most cases run on an in-memory image with the power-loss backend; one in
//! eight runs on a file through `Store::open`, where a power loss cannot be
//! modelled and is replaced by a clean reopen.
//!
//! A failing sequence is shrunk (whole operations removed, then single
//! writes) and reported with its seed. `PURSE_TEST_SEED` seeds the run;
//! `PURSE_STORE_MODEL_CASES` sets the number of sequences (default 48).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and indices"
)]

mod support;

use std::collections::BTreeMap;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use purse_errors::Kind;
use purse_store::{
    BlockRow, CFilterRow, Codec as _, Error, FORMAT_MAGIC, Hash32, HeaderBytes, Height, MetaKey,
    OpenOptions, ReadTx, SCHEMA_VERSION, Store, TableId, Tip, WriteTx,
};
use purse_testutil::SplitMix64;
use support::powerloss::{Cut, PowerLoss, Shared};

type Rows = BTreeMap<Vec<u8>, Vec<u8>>;
type State = BTreeMap<TableId, Rows>;

/// The tables random writes go to.
const TABLES: &[TableId] = &[
    TableId::Accounts,
    TableId::Credits,
    TableId::Unspent,
    TableId::TxMined,
    TableId::Tickets,
];

#[derive(Clone, Debug)]
enum Check {
    Get(TableId, Vec<u8>),
    Range {
        table: TableId,
        lo: Option<Vec<u8>>,
        hi: Option<Vec<u8>>,
        rev: bool,
    },
    Prefix(TableId, Vec<u8>),
    Len(TableId),
    /// A cursor positioned by `seek(k)` (or `first`/`last` when `None`)
    /// stepped `steps` times forward or backward until it runs out.
    Walk {
        table: TableId,
        start: Option<Vec<u8>>,
        forward: bool,
        steps: usize,
    },
}

#[derive(Clone, Debug)]
enum Wr {
    Put(TableId, Vec<u8>, Vec<u8>),
    Del(TableId, Vec<u8>),
    /// A put bbolt refuses (an empty key); the transaction stays usable.
    Refused(TableId, Vec<u8>),
    /// A write cursor seeks `from` and deletes up to `count` entries,
    /// stepping with `next` after each delete.
    CursorDelete {
        table: TableId,
        from: Vec<u8>,
        count: usize,
    },
    /// A read inside the transaction, which sees its own writes.
    Check(Check),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncEnd {
    Finish,
    /// Dropped without `finish` (a sync stopped by an error).
    Drop,
    /// Power lost after this many batches.
    Crash(usize),
}

#[derive(Clone, Debug)]
struct SyncBlock {
    hash: u8,
    height: u32,
    filter_len: usize,
}

#[derive(Clone, Debug)]
enum Op {
    Update {
        writes: Vec<Wr>,
        commit: bool,
    },
    Read(Vec<Check>),
    /// A read transaction taken before a committing update keeps seeing
    /// the state before it.
    Snapshot(Vec<Wr>),
    Sync {
        interval: u32,
        batches: Vec<(Vec<u32>, Vec<SyncBlock>)>,
        end: SyncEnd,
    },
    Reopen,
    Crash,
}

// Generation.

fn key(rng: &mut SplitMix64) -> Vec<u8> {
    // Short keys over a few byte values: overwrites, deletes of present
    // keys and prefix relations are frequent.
    const ALPHABET: [u8; 6] = [0x00, 0x01, 0x40, 0x7f, 0x80, 0xff];
    let len = 1 + rng.below(4) as usize;
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize])
        .collect()
}

fn value(rng: &mut SplitMix64) -> Vec<u8> {
    let len = match rng.below(20) {
        0 => 2_000 + rng.below(9_000) as usize,
        1 => 0,
        _ => rng.below(64) as usize,
    };
    rng.bytes(len)
}

fn table(rng: &mut SplitMix64) -> TableId {
    TABLES[rng.below(TABLES.len() as u64) as usize]
}

fn opt_key(rng: &mut SplitMix64) -> Option<Vec<u8>> {
    (rng.below(4) != 0).then(|| key(rng))
}

fn check(rng: &mut SplitMix64) -> Check {
    let t = table(rng);
    match rng.below(5) {
        0 => Check::Get(t, key(rng)),
        1 => Check::Range {
            table: t,
            lo: opt_key(rng),
            hi: opt_key(rng),
            rev: rng.below(2) == 0,
        },
        2 => {
            let mut p = key(rng);
            p.truncate(1 + rng.below(2) as usize);
            Check::Prefix(t, p)
        }
        3 => Check::Len(t),
        _ => Check::Walk {
            table: t,
            start: opt_key(rng),
            forward: rng.below(2) == 0,
            steps: rng.below(12) as usize,
        },
    }
}

fn write(rng: &mut SplitMix64) -> Wr {
    match rng.below(16) {
        0..=7 => Wr::Put(table(rng), key(rng), value(rng)),
        8..=10 => Wr::Del(table(rng), key(rng)),
        11 => Wr::Refused(table(rng), value(rng)),
        12 => Wr::CursorDelete {
            table: table(rng),
            from: key(rng),
            count: rng.below(5) as usize,
        },
        _ => Wr::Check(check(rng)),
    }
}

fn writes(rng: &mut SplitMix64) -> Vec<Wr> {
    let n = 1 + rng.below(12) as usize;
    (0..n).map(|_| write(rng)).collect()
}

fn op(rng: &mut SplitMix64) -> Op {
    match rng.below(20) {
        0..=8 => Op::Update {
            writes: writes(rng),
            commit: rng.below(6) != 0,
        },
        9..=11 => Op::Read((0..1 + rng.below(4)).map(|_| check(rng)).collect()),
        12 => Op::Snapshot(writes(rng)),
        13..=15 => {
            let n = 1 + rng.below(7) as usize;
            let batches = (0..n)
                .map(|_| {
                    let removes = (0..rng.below(3)).map(|_| rng.below(24) as u32).collect();
                    let blocks = (0..1 + rng.below(4))
                        .map(|_| SyncBlock {
                            hash: rng.below(24) as u8,
                            height: rng.below(24) as u32,
                            filter_len: rng.below(300) as usize,
                        })
                        .collect();
                    (removes, blocks)
                })
                .collect();
            let end = match rng.below(4) {
                0 => SyncEnd::Drop,
                1 => SyncEnd::Crash(rng.below(n as u64 + 1) as usize),
                _ => SyncEnd::Finish,
            };
            Op::Sync {
                interval: 1 + rng.below(4) as u32,
                batches,
                end,
            }
        }
        16..=17 => Op::Reopen,
        _ => Op::Crash,
    }
}

// The model.

fn initial_state() -> State {
    let mut meta = Rows::new();
    meta.insert(MetaKey::Format.name().to_vec(), FORMAT_MAGIC.to_vec());
    meta.insert(
        MetaKey::Version.name().to_vec(),
        SCHEMA_VERSION.to_be_bytes().to_vec(),
    );
    State::from([(TableId::Meta, meta)])
}

fn put(s: &mut State, t: TableId, k: Vec<u8>, v: Vec<u8>) {
    s.entry(t).or_default().insert(k, v);
}

fn del(s: &mut State, t: TableId, k: &[u8]) {
    if let Some(rows) = s.get_mut(&t) {
        rows.remove(k);
        if rows.is_empty() {
            s.remove(&t);
        }
    }
}

fn rows(s: &State, t: TableId) -> Vec<(Vec<u8>, Vec<u8>)> {
    s.get(&t)
        .map(|r| r.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// What a check returns, on the model.
fn expect(s: &State, c: &Check) -> Vec<(Vec<u8>, Vec<u8>)> {
    match c {
        Check::Get(t, k) => s
            .get(t)
            .and_then(|r| r.get(k))
            .map(|v| vec![(k.clone(), v.clone())])
            .unwrap_or_default(),
        Check::Range { table, lo, hi, rev } => {
            let mut out: Vec<_> = rows(s, *table)
                .into_iter()
                .filter(|(k, _)| lo.as_ref().is_none_or(|lo| k >= lo))
                .filter(|(k, _)| hi.as_ref().is_none_or(|hi| k < hi))
                .collect();
            if *rev {
                out.reverse();
            }
            out
        }
        Check::Prefix(t, p) => rows(s, *t)
            .into_iter()
            .filter(|(k, _)| k.starts_with(p))
            .collect(),
        Check::Len(t) => {
            let n = s.get(t).map_or(0, BTreeMap::len) as u64;
            vec![(n.to_be_bytes().to_vec(), Vec::new())]
        }
        Check::Walk {
            table,
            start,
            forward,
            steps,
        } => {
            let all = rows(s, *table);
            let at = match (start, forward) {
                (Some(k), _) => all.iter().position(|(x, _)| x >= k),
                (None, true) => (!all.is_empty()).then_some(0),
                (None, false) => all.len().checked_sub(1),
            };
            let Some(mut i) = at else { return Vec::new() };
            let mut out = vec![all[i].clone()];
            for _ in 0..*steps {
                let next = if *forward {
                    Some(i + 1).filter(|&j| j < all.len())
                } else {
                    i.checked_sub(1)
                };
                match next {
                    Some(j) => {
                        i = j;
                        out.push(all[i].clone());
                    }
                    None => break,
                }
            }
            out
        }
    }
}

fn bound(k: &Option<Vec<u8>>, lo: bool) -> Bound<&[u8]> {
    match k {
        None => Bound::Unbounded,
        Some(k) if lo => Bound::Included(k.as_slice()),
        Some(k) => Bound::Excluded(k.as_slice()),
    }
}

type Res<T> = Result<T, String>;

fn e(x: Error) -> String {
    format!("store error: {x}")
}

/// What a check returns, on a read transaction.
fn observe_read(tx: &ReadTx, c: &Check) -> Res<Vec<(Vec<u8>, Vec<u8>)>> {
    Ok(match c {
        Check::Get(t, k) => tx
            .get_raw(*t, k)
            .map_err(e)?
            .map(|v| vec![(k.clone(), v)])
            .unwrap_or_default(),
        Check::Range { table, lo, hi, rev } => {
            let it = tx
                .range_raw(*table, bound(lo, true), bound(hi, false))
                .map_err(e)?;
            if *rev {
                it.rev().collect::<Result<_, _>>().map_err(e)?
            } else {
                it.collect::<Result<_, _>>().map_err(e)?
            }
        }
        Check::Prefix(t, p) => tx
            .prefix_raw(*t, p)
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?,
        Check::Len(t) => vec![(tx.len(*t).map_err(e)?.to_be_bytes().to_vec(), Vec::new())],
        Check::Walk {
            table,
            start,
            forward,
            steps,
        } => {
            let mut c = tx.cursor(*table).map_err(e)?;
            let mut out = Vec::new();
            let first = match (start, forward) {
                (Some(k), _) => c.seek(k),
                (None, true) => c.first(),
                (None, false) => c.last(),
            }
            .map_err(e)?;
            if let Some(x) = first {
                out.push(x);
                for _ in 0..*steps {
                    let x = if *forward { c.next() } else { c.prev() }.map_err(e)?;
                    match x {
                        Some(x) => out.push(x),
                        None => break,
                    }
                }
            }
            out
        }
    })
}

/// What a check returns, inside a write transaction.
fn observe_write(tx: &mut WriteTx<'_>, c: &Check) -> Res<Vec<(Vec<u8>, Vec<u8>)>> {
    Ok(match c {
        Check::Get(t, k) => tx
            .get_raw(*t, k)
            .map_err(e)?
            .map(|v| vec![(k.clone(), v)])
            .unwrap_or_default(),
        Check::Range { table, lo, hi, rev } => {
            let it = tx
                .range_raw(*table, bound(lo, true), bound(hi, false))
                .map_err(e)?;
            if *rev {
                it.rev().collect::<Result<_, _>>().map_err(e)?
            } else {
                it.collect::<Result<_, _>>().map_err(e)?
            }
        }
        Check::Prefix(t, p) => tx
            .prefix_raw(*t, p)
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?,
        Check::Len(t) => vec![(tx.len(*t).map_err(e)?.to_be_bytes().to_vec(), Vec::new())],
        Check::Walk {
            table,
            start,
            forward,
            steps,
        } => {
            let mut c = tx.cursor(*table).map_err(e)?;
            let mut out = Vec::new();
            let first = match (start, forward) {
                (Some(k), _) => c.seek(k),
                (None, true) => c.first(),
                (None, false) => c.last(),
            }
            .map_err(e)?;
            if let Some(x) = first {
                out.push(x);
                for _ in 0..*steps {
                    let x = if *forward { c.next() } else { c.prev() }.map_err(e)?;
                    match x {
                        Some(x) => out.push(x),
                        None => break,
                    }
                }
            }
            out
        }
    })
}

/// The whole contents a read transaction sees, as the model holds them.
fn dump(tx: &ReadTx) -> Res<State> {
    let mut s = State::new();
    for &t in TableId::ALL {
        let r: Rows = tx
            .range_raw(t, Bound::Unbounded, Bound::Unbounded)
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        if !r.is_empty() {
            s.insert(t, r);
        }
    }
    Ok(s)
}

fn diff(want: &State, got: &State) -> String {
    for &t in TableId::ALL {
        let (w, g) = (rows(want, t), rows(got, t));
        if w != g {
            let wk: Vec<String> = w.iter().map(|(k, _)| purse_testutil::hex(k)).collect();
            let gk: Vec<String> = g.iter().map(|(k, _)| purse_testutil::hex(k)).collect();
            if wk != gk {
                return format!("table {}: model keys {wk:?}, store keys {gk:?}", t.name());
            }
            let (k, mv, sv) = w
                .iter()
                .zip(&g)
                .find(|(a, b)| a.1 != b.1)
                .map(|(a, b)| (&a.0, &a.1, &b.1))
                .expect("rows differ in a value");
            return format!(
                "table {}, key {}: model value {} ({} bytes), store value {} ({} bytes)",
                t.name(),
                purse_testutil::hex(k),
                purse_testutil::hex(&mv[..mv.len().min(40)]),
                mv.len(),
                purse_testutil::hex(&sv[..sv.len().min(40)]),
                sv.len()
            );
        }
    }
    "no difference".into()
}

// The run.

/// Where the store lives.
enum Medium {
    Image(Arc<PowerLoss>),
    File(PathBuf),
}

struct Harness {
    medium: Medium,
    store: Option<Store>,
}

impl Harness {
    fn new(file: Option<PathBuf>) -> Res<Harness> {
        let (medium, store) = match file {
            Some(p) => {
                let _ = std::fs::remove_file(&p);
                let s = Store::create(&p).map_err(e)?;
                (Medium::File(p), s)
            }
            None => {
                let b = PowerLoss::new(Vec::new(), true);
                let s = OpenOptions::new()
                    .create_on_backend(Shared(b.clone()), |_| Ok(()))
                    .map_err(e)?;
                (Medium::Image(b), s)
            }
        };
        Ok(Harness {
            medium,
            store: Some(store),
        })
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store open")
    }

    fn reopen(&mut self) -> Res<()> {
        if let Some(s) = self.store.take() {
            s.close().map_err(e)?;
        }
        self.store = Some(match &self.medium {
            Medium::File(p) => Store::open(p).map_err(e)?,
            Medium::Image(b) => OpenOptions::new()
                .open_on_backend(Shared(b.clone()))
                .map_err(e)?,
        });
        Ok(())
    }

    fn can_crash(&self) -> bool {
        matches!(self.medium, Medium::Image(_))
    }

    /// Cuts the power (the store must already have been dropped, or be
    /// dropped here with the power out) and reopens the image it left.
    fn crash(&mut self) -> Res<()> {
        let Medium::Image(b) = &self.medium else {
            return self.reopen();
        };
        b.power_fails_after(0);
        drop(self.store.take());
        let image = b.cut(Cut::Clean);
        let b2 = PowerLoss::new(image, true);
        self.store = Some(
            OpenOptions::new()
                .open_on_backend(Shared(b2.clone()))
                .map_err(|x| format!("reopen after power loss: {x}"))?,
        );
        self.medium = Medium::Image(b2);
        Ok(())
    }
}

#[derive(Debug)]
enum TxErr {
    Store(Error),
    Abandon,
    Mismatch(String),
}

impl From<Error> for TxErr {
    fn from(x: Error) -> Self {
        TxErr::Store(x)
    }
}

/// Applies `ws` to the transaction and to `model` (the pending state).
fn apply(tx: &mut WriteTx<'_>, ws: &[Wr], model: &mut State) -> Result<(), TxErr> {
    for w in ws {
        match w {
            Wr::Put(t, k, v) => {
                tx.put_raw(*t, k, v)?;
                put(model, *t, k.clone(), v.clone());
            }
            Wr::Del(t, k) => {
                let was = model.get(t).is_some_and(|r| r.contains_key(k));
                let got = tx.delete_raw(*t, k)?;
                if got != was {
                    return Err(TxErr::Mismatch(format!(
                        "delete returned {got}, model {was}"
                    )));
                }
                del(model, *t, k);
            }
            Wr::Refused(t, v) => match tx.put_raw(*t, b"", v) {
                Err(x) if x.kind() == Kind::Invalid => {}
                other => {
                    return Err(TxErr::Mismatch(format!(
                        "empty key: expected an Invalid error, got {other:?}"
                    )));
                }
            },
            Wr::CursorDelete { table, from, count } => {
                let mut c = tx.cursor(*table)?;
                let mut at = c.seek(from)?;
                let mut n = 0;
                while let Some((k, _)) = at {
                    if n == *count {
                        break;
                    }
                    if !c.delete()? {
                        return Err(TxErr::Mismatch("cursor delete removed nothing".into()));
                    }
                    del(model, *table, &k);
                    n += 1;
                    at = c.next()?;
                }
            }
            Wr::Check(chk) => {
                let got = observe_write(tx, chk).map_err(TxErr::Mismatch)?;
                let want = expect(model, chk);
                if got != want {
                    return Err(TxErr::Mismatch(format!(
                        "in-transaction {chk:?}: store {} rows, model {} rows",
                        got.len(),
                        want.len()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn compare(store: &Store, model: &State, after: &str) -> Res<()> {
    let got = dump(&store.read().map_err(e)?)?;
    if &got != model {
        return Err(format!("after {after}: {}", diff(model, &got)));
    }
    Ok(())
}

fn sync_block_rows(b: &SyncBlock) -> (Hash32, HeaderBytes, CFilterRow, BlockRow) {
    let hash = Hash32([b.hash; 32]);
    let header = HeaderBytes([b.hash ^ 0x5a; 180]);
    let filter = CFilterRow {
        key: [b.hash; 16],
        filter: vec![b.hash; b.filter_len],
    };
    let row = BlockRow {
        hash,
        time: i64::from(b.height),
        vote_bits: u16::from(b.hash),
        invalid: false,
        wallet_txs: 0,
    };
    (hash, header, filter, row)
}

/// The height a model's `block_by_hash` gives `hash`.
fn model_height(m: &State, hash: &Hash32) -> Option<Vec<u8>> {
    m.get(&TableId::BlockByHash)
        .and_then(|r| r.get(hash.0.as_slice()))
        .cloned()
}

/// One batch's effect on the model, as `SyncBatch`'s documentation states
/// it: a removed or replaced block's hash index goes with it, when it
/// points at that height.
fn model_batch(m: &mut State, removes: &[u32], blocks: &[SyncBlock]) {
    for &h in removes {
        let k = Height(h).encode().to_vec();
        if let Some(row) = m.get(&TableId::Blocks).and_then(|r| r.get(&k)).cloned() {
            let row = BlockRow::decode(&row).expect("model block row");
            del(m, TableId::Blocks, &k);
            if model_height(m, &row.hash).as_deref() == Some(k.as_slice()) {
                del(m, TableId::BlockByHash, &row.hash.0);
            }
        }
    }
    for b in blocks {
        let k = Height(b.height).encode().to_vec();
        let (hash, ..) = sync_block_rows(b);
        if let Some(old) = m.get(&TableId::Blocks).and_then(|r| r.get(&k)).cloned() {
            let old = BlockRow::decode(&old).expect("model block row");
            if old.hash != hash && model_height(m, &old.hash).as_deref() == Some(k.as_slice()) {
                del(m, TableId::BlockByHash, &old.hash.0);
            }
        }
        // Each put of the batch sees the ones before it: apply it now.
        model_put_block(m, b);
    }
    if let Some(b) = blocks.last() {
        let (hash, ..) = sync_block_rows(b);
        let t = Tip {
            height: b.height,
            hash,
        };
        put(
            m,
            TableId::Meta,
            MetaKey::Tip.name().to_vec(),
            t.encode().to_vec(),
        );
    }
}

/// The rows one block of a batch puts.
fn model_put_block(m: &mut State, b: &SyncBlock) {
    let (hash, header, filter, row) = sync_block_rows(b);
    put(m, TableId::Headers, hash.0.to_vec(), header.0.to_vec());
    put(
        m,
        TableId::CFilters,
        hash.0.to_vec(),
        filter.encode().to_vec(),
    );
    put(
        m,
        TableId::Blocks,
        Height(b.height).encode().to_vec(),
        row.encode().to_vec(),
    );
    put(
        m,
        TableId::BlockByHash,
        hash.0.to_vec(),
        Height(b.height).encode().to_vec(),
    );
}

fn run_op(h: &mut Harness, model: &mut State, op: &Op) -> Res<()> {
    match op {
        Op::Update { writes, commit } => {
            let mut pending = model.clone();
            let r = h.store().update(|tx| {
                apply(tx, writes, &mut pending)?;
                if *commit { Ok(()) } else { Err(TxErr::Abandon) }
            });
            match r {
                Ok(()) => *model = pending,
                Err(TxErr::Abandon) => {}
                Err(TxErr::Store(x)) => return Err(e(x)),
                Err(TxErr::Mismatch(m)) => return Err(m),
            }
            compare(h.store(), model, "update")
        }
        Op::Read(checks) => {
            let tx = h.store().read().map_err(e)?;
            for c in checks {
                let got = observe_read(&tx, c)?;
                let want = expect(model, c);
                if got != want {
                    return Err(format!(
                        "{c:?}: store {} rows, model {} rows",
                        got.len(),
                        want.len()
                    ));
                }
            }
            Ok(())
        }
        Op::Snapshot(writes) => {
            let before = model.clone();
            let snap = h.store().read().map_err(e)?;
            let mut pending = model.clone();
            h.store()
                .update(|tx| apply(tx, writes, &mut pending))
                .map_err(|x| format!("{x:?}"))?;
            *model = pending;
            let seen = dump(&snap)?;
            if seen != before {
                return Err(format!(
                    "snapshot changed under a commit: {}",
                    diff(&before, &seen)
                ));
            }
            drop(snap);
            compare(h.store(), model, "snapshot update")
        }
        Op::Sync {
            interval,
            batches,
            end,
        } => run_sync(h, model, *interval, batches, *end),
        Op::Reopen => {
            h.reopen()?;
            compare(h.store(), model, "reopen")
        }
        Op::Crash => {
            // Every commit outside a sync is durable: nothing may be lost.
            h.crash()?;
            compare(h.store(), model, "power loss")
        }
    }
}

fn run_sync(
    h: &mut Harness,
    model: &mut State,
    interval: u32,
    batches: &[(Vec<u32>, Vec<SyncBlock>)],
    end: SyncEnd,
) -> Res<()> {
    let end = match end {
        SyncEnd::Crash(_) if !h.can_crash() => SyncEnd::Drop,
        other => other,
    };
    // The states a power loss may leave: the last durable one and each
    // commit after it.
    let mut since_durable = vec![model.clone()];
    {
        let store = h.store.as_ref().expect("store open");
        let mut sync = store.initial_sync().with_interval(interval).map_err(e)?;
        for (i, (removes, blocks)) in batches.iter().enumerate() {
            if end == SyncEnd::Crash(i) {
                break;
            }
            let ((), durable) = sync
                .batch(|b| {
                    for &x in removes {
                        b.remove_block(x)?;
                    }
                    let mut last = None;
                    for blk in blocks {
                        let (hash, header, filter, row) = sync_block_rows(blk);
                        b.put_header(&hash, &header)?;
                        b.put_cfilter(&hash, &filter)?;
                        b.put_block(blk.height, &row)?;
                        last = Some(Tip {
                            height: blk.height,
                            hash,
                        });
                    }
                    if let Some(t) = last {
                        b.set_tip(&t)?;
                    }
                    Ok::<_, Error>(())
                })
                .map_err(e)?;
            model_batch(model, removes, blocks);
            if durable {
                since_durable.clear();
            }
            since_durable.push(model.clone());
            if sync.pending() >= interval {
                return Err(format!(
                    "{} batches pending at interval {interval}",
                    sync.pending()
                ));
            }
        }
        match end {
            SyncEnd::Finish => sync.finish().map_err(e)?,
            SyncEnd::Drop => drop(sync),
            SyncEnd::Crash(_) => {
                if let Medium::Image(b) = &h.medium {
                    // The power goes before the sync's closing commit.
                    b.power_fails_after(0);
                }
                drop(sync);
            }
        }
    }
    if let SyncEnd::Crash(_) = end {
        h.crash()?;
        let got = dump(&h.store().read().map_err(e)?)?;
        let Some(i) = since_durable.iter().rposition(|s| *s == got) else {
            return Err(format!(
                "after a power loss in a sync: no state since the last durable commit \
                 matches ({} candidates; against the last: {})",
                since_durable.len(),
                diff(since_durable.last().expect("one state"), &got)
            ));
        };
        *model = since_durable.swap_remove(i);
        return Ok(());
    }
    compare(h.store(), model, "sync")
}

/// Runs a sequence; `Err` describes the first divergence.
fn run_case(ops: &[Op], file: Option<PathBuf>) -> Res<()> {
    let mut h = Harness::new(file)?;
    let mut model = initial_state();
    compare(h.store(), &model, "create")?;
    for (i, op) in ops.iter().enumerate() {
        run_op(&mut h, &mut model, op).map_err(|m| format!("op {i} ({}): {m}", op_name(op)))?;
    }
    h.reopen()?;
    compare(h.store(), &model, "final reopen")
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Update { commit: true, .. } => "update",
        Op::Update { commit: false, .. } => "abandoned update",
        Op::Read(_) => "read",
        Op::Snapshot(_) => "snapshot",
        Op::Sync { .. } => "sync",
        Op::Reopen => "reopen",
        Op::Crash => "power loss",
    }
}

/// Runs a sequence, turning a panic into a failure, so that a panicking
/// sequence shrinks too.
fn try_case(ops: &[Op], file: Option<&PathBuf>) -> Res<()> {
    match catch_unwind(AssertUnwindSafe(|| run_case(ops, file.cloned()))) {
        Ok(r) => r,
        Err(p) => Err(format!(
            "panic: {}",
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default()
        )),
    }
}

/// Shrinks a failing sequence: removes runs of operations (halving the run
/// length), then single writes inside updates, keeping every change after
/// which the sequence still fails.
fn shrink(mut ops: Vec<Op>, fails: impl Fn(&[Op]) -> Res<()>) -> (Vec<Op>, String) {
    let mut why = fails(&ops).expect_err("the sequence fails");
    let mut budget = 3_000u32;
    let mut chunk = ops.len().div_ceil(2).max(1);
    while chunk >= 1 && budget > 0 {
        let mut i = 0;
        let mut progressed = false;
        while i < ops.len() && budget > 0 {
            budget -= 1;
            let mut cand = ops.clone();
            cand.drain(i..(i + chunk).min(cand.len()));
            match fails(&cand) {
                Err(w) => {
                    ops = cand;
                    why = w;
                    progressed = true;
                }
                Ok(()) => i += chunk,
            }
        }
        if !progressed {
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    // Single writes.
    for i in 0..ops.len() {
        let mut j = 0;
        loop {
            let len = match &ops[i] {
                Op::Update { writes, .. } | Op::Snapshot(writes) => writes.len(),
                _ => 0,
            };
            if j >= len || budget == 0 {
                break;
            }
            budget -= 1;
            let mut cand = ops.clone();
            if let Op::Update { writes, .. } | Op::Snapshot(writes) = &mut cand[i] {
                writes.remove(j);
            }
            match fails(&cand) {
                Err(w) => {
                    ops = cand;
                    why = w;
                }
                Ok(()) => j += 1,
            }
        }
    }
    (ops, why)
}

#[test]
fn the_store_behaves_as_its_model() {
    let mut rng = SplitMix64::from_env("purse-store model");
    let cases: usize = std::env::var("PURSE_STORE_MODEL_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("purse-store-model");
    std::fs::create_dir_all(&dir).unwrap();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for case in 0..cases {
        let case_seed = rng.next_u64();
        let mut r = SplitMix64(case_seed);
        let n = 20 + r.below(60) as usize;
        let ops: Vec<Op> = (0..n).map(|_| op(&mut r)).collect();
        for o in &ops {
            *counts.entry(op_name(o)).or_default() += 1;
        }
        let file = (case % 8 == 7).then(|| dir.join(format!("case-{case}.redb")));
        if let Err(first) = try_case(&ops, file.as_ref()) {
            // Silence the panic hook while shrinking a panicking case.
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let (small, why) = shrink(ops, |o| try_case(o, file.as_ref()));
            std::panic::set_hook(hook);
            panic!(
                "case {case} (case seed {case_seed:#x}) failed: {first}\n\
                 shrunk to {} operations, failing with: {why}\n{small:#?}",
                small.len()
            );
        }
        if let Some(p) = file {
            let _ = std::fs::remove_file(p);
        }
    }
    eprintln!("model: {cases} sequences; operations run: {counts:?}");
}

/// The shrinker itself: a sequence with a planted divergence shrinks to the
/// operations that cause it.
#[test]
fn a_failing_sequence_shrinks() {
    // A model that disagrees with the store: run the sequence, then demand
    // a row nobody wrote.
    fn bad(ops: &[Op]) -> Res<()> {
        let mut h = Harness::new(None)?;
        let mut model = initial_state();
        for op in ops {
            run_op(&mut h, &mut model, op)?;
        }
        let tx = h.store().read().map_err(e)?;
        if tx.get_raw(TableId::Tickets, &[7]).map_err(e)?.is_some() {
            return Err("planted".into());
        }
        Ok(())
    }
    let mut r = SplitMix64(1);
    let mut ops: Vec<Op> = (0..30).map(|_| op(&mut r)).collect();
    ops.insert(
        13,
        Op::Update {
            writes: vec![
                Wr::Put(TableId::Accounts, vec![1], vec![1]),
                Wr::Put(TableId::Tickets, vec![7], Vec::new()),
                Wr::Put(TableId::Credits, vec![2], vec![2]),
            ],
            commit: true,
        },
    );
    // Remove later deletes of the planted row, which would hide it.
    for o in ops.iter_mut().skip(14) {
        if let Op::Update { writes, .. } | Op::Snapshot(writes) = o {
            writes.retain(|w| match w {
                Wr::Del(TableId::Tickets, _) | Wr::CursorDelete { .. } => false,
                Wr::Put(TableId::Tickets, k, _) => k != &[7],
                _ => true,
            });
        }
    }
    assert!(bad(&ops).is_err());
    let (cur, why) = shrink(ops, bad);
    assert_eq!(why, "planted");
    assert_eq!(cur.len(), 1, "{cur:#?}");
    let Op::Update { writes, .. } = &cur[0] else {
        panic!("{cur:#?}")
    };
    assert_eq!(writes.len(), 1, "{cur:#?}");
}
