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
//! loss between transactions. A batch runs the chain operations (blocks
//! connected above the tip, detached from a height, a reorganisation,
//! a detached block promoted, a detached header recorded, a missing
//! filter added) on hashes drawn from a small set, so that blocks come
//! back after a detach, and refusals (a main-chain block delivered again
//! with its own header, given the next height or one far above it, and
//! in a reorganisation that keeps it; its hash with a header that is not
//! its own; the genesis block detached; nothing to promote; and a block
//! row with wallet state connected or detached by a header batch) occur
//! and must leave the batch usable with nothing written. The same chain
//! operations also run durably in a write transaction of `Store::update`,
//! committed or abandoned, where block rows with wallet state connect and
//! detach freely; those rows then stand in the chain for later header
//! batches to refuse. Record operations (VSP hosts and tickets, multisig
//! outputs, treasury policies and agenda preferences, refusals of an empty
//! host and of a ticket naming no host among them) run in durable write
//! transactions, committed or abandoned, and after each the record tables
//! must hold their invariants by the store's `ReadTx::check_records` and by
//! the tests' `support::records` on the model. The model is a map per
//! table, the chain and record operations written out on it as their
//! documentation states them. After every operation the
//! store's whole contents must equal the model's, and the chain tables
//! must hold their invariants, by the store's own check
//! (`ReadTx::check_chain`) and by the tests' independent one
//! (`support::chain`, run on the model); after a power loss they must
//! equal the model as of some commit since the last durable one.
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

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use purse_errors::Kind;
use purse_store::{
    AgendaChoice, AgendaKey, AgendaPrefs, BlockByHash, BlockRow, Blocks, CFilterRow, CFilters,
    CFiltersDetached, ChainBlock, Codec, Error, FORMAT_MAGIC, Hash32, Hash160, HeaderBytes,
    Headers, HeadersDetached, Height, MetaKey, MultisigBlock, MultisigOutRow, MultisigSpender,
    OpenOptions, Outpoint, PiKey, ReadTx, SCHEMA_VERSION, Store, SyncBatch, TSpendPolicies, Table,
    TableId, TicketAgendaKey, TicketAgendaPrefs, Tip, TreasuryKeyPolicies, TreasuryVote,
    VspTSpendKey, VspTSpendPolicies, VspTicketRow, VspTreasuryKey, VspTreasuryKeyPolicies, WriteTx,
};
use purse_testutil::SplitMix64;
use support::chain::chain_view_of;
use support::chain_header;
use support::powerloss::{Cut, PowerLoss, Shared};
use support::records::records_view_of;

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
    /// `None`: connected without a filter (a detached one is kept).
    filter_len: Option<usize>,
    /// The block row carries wallet state (wallet transactions), which a
    /// header batch refuses to connect or detach.
    wallet: bool,
}

/// One chain operation of a header batch.
#[derive(Clone, Debug)]
enum ChainOp {
    /// Blocks connected above the tip.
    Connect(Vec<SyncBlock>),
    /// Every block at this height and above detached.
    Disconnect(u32),
    /// The blocks above `tip - depth` detached and `blocks` connected there.
    Reorg { depth: u32, blocks: Vec<SyncBlock> },
    /// The detached block of this hash promoted at the next height, its
    /// row carrying wallet state when the flag is set.
    Promote(u8, bool),
    /// A detached header (and filter) recorded for this hash.
    Detach { hash: u8, filter: bool },
    /// The filter of the main-chain block at this height set.
    Filter { height: u32, len: usize },
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
        batches: Vec<Vec<ChainOp>>,
        end: SyncEnd,
    },
    /// Chain operations in a durable write transaction through
    /// `Store::update`, committed or abandoned.
    Chain {
        ops: Vec<ChainOp>,
        commit: bool,
    },
    /// Record operations (`purse_store::records`) and policy and
    /// preference writes in a durable write transaction, committed or
    /// abandoned.
    Records {
        ops: Vec<RecOp>,
        commit: bool,
    },
    Reopen,
    Crash,
}

/// One record operation, on values drawn from small sets so that hosts,
/// tickets and outputs are written again, moved and deleted.
#[derive(Clone, Debug)]
enum RecOp {
    /// `add_vsp_host` of host number `host` (0 is the empty host, which is
    /// refused), with a public key of bytes `pubkey`.
    Host { host: u8, pubkey: u8 },
    /// `put_vsp_ticket`; `host_id` may name no host (refused).
    Ticket {
        ticket: u8,
        status: u32,
        host_id: u32,
        fee: u8,
    },
    /// `delete_vsp_ticket`.
    DelTicket(u8),
    /// `put_multisig_out` of output `(hash, index)`.
    Multisig {
        hash: u8,
        index: u32,
        script: u8,
        mined: bool,
        spent: bool,
        stake: bool,
    },
    /// `delete_multisig_out`.
    DelMultisig { hash: u8, index: u32 },
    /// A treasury policy in table number `table` (of the four) at key
    /// `key`: written when `vote` is non-zero, deleted when it is 0, as
    /// dcrwallet's setters do.
    Policy { table: u8, key: u8, vote: u8 },
    /// An agenda preference, per ticket when `ticket` is set.
    Agenda {
        ticket: Option<u8>,
        version: u32,
        agenda: u8,
        choice: u8,
    },
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

fn sync_block(rng: &mut SplitMix64) -> SyncBlock {
    SyncBlock {
        hash: rng.below(24) as u8,
        filter_len: (rng.below(4) != 0).then(|| rng.below(300) as usize),
        wallet: rng.below(6) == 0,
    }
}

fn chain_op(rng: &mut SplitMix64) -> ChainOp {
    match rng.below(12) {
        0..=4 => ChainOp::Connect((0..1 + rng.below(4)).map(|_| sync_block(rng)).collect()),
        5 => ChainOp::Disconnect(rng.below(24) as u32),
        6 | 7 => ChainOp::Reorg {
            depth: rng.below(5) as u32,
            blocks: (0..rng.below(5)).map(|_| sync_block(rng)).collect(),
        },
        8 => ChainOp::Promote(rng.below(24) as u8, rng.below(6) == 0),
        9 => ChainOp::Detach {
            hash: rng.below(24) as u8,
            filter: rng.below(2) == 0,
        },
        _ => ChainOp::Filter {
            height: rng.below(24) as u32,
            len: rng.below(100) as usize,
        },
    }
}

fn rec_op(rng: &mut SplitMix64) -> RecOp {
    match rng.below(14) {
        0 | 1 => RecOp::Host {
            host: rng.below(5) as u8,
            pubkey: rng.below(4) as u8,
        },
        2..=4 => RecOp::Ticket {
            ticket: rng.below(8) as u8,
            status: rng.below(4) as u32,
            host_id: rng.below(6) as u32,
            fee: rng.below(3) as u8,
        },
        5 => RecOp::DelTicket(rng.below(8) as u8),
        6..=8 => RecOp::Multisig {
            hash: rng.below(4) as u8,
            index: [0, 1, 256][rng.below(3) as usize],
            script: rng.below(3) as u8,
            mined: rng.below(2) == 0,
            spent: rng.below(3) == 0,
            stake: rng.below(4) == 0,
        },
        9 => RecOp::DelMultisig {
            hash: rng.below(4) as u8,
            index: [0, 1, 256][rng.below(3) as usize],
        },
        10 | 11 => RecOp::Policy {
            table: rng.below(4) as u8,
            key: rng.below(4) as u8,
            vote: rng.below(4) as u8,
        },
        _ => RecOp::Agenda {
            ticket: (rng.below(2) == 0).then(|| rng.below(4) as u8),
            version: 10 + rng.below(2) as u32,
            agenda: rng.below(3) as u8,
            choice: rng.below(3) as u8,
        },
    }
}

fn writes(rng: &mut SplitMix64) -> Vec<Wr> {
    let n = 1 + rng.below(12) as usize;
    (0..n).map(|_| write(rng)).collect()
}

fn op(rng: &mut SplitMix64) -> Op {
    match rng.below(24) {
        0..=8 => Op::Update {
            writes: writes(rng),
            commit: rng.below(6) != 0,
        },
        9..=11 => Op::Read((0..1 + rng.below(4)).map(|_| check(rng)).collect()),
        12 => Op::Snapshot(writes(rng)),
        13..=15 => {
            let n = 1 + rng.below(7) as usize;
            let batches = (0..n)
                .map(|_| (0..1 + rng.below(4)).map(|_| chain_op(rng)).collect())
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
        18..=19 => Op::Crash,
        20..=21 => Op::Records {
            ops: (0..1 + rng.below(8)).map(|_| rec_op(rng)).collect(),
            commit: rng.below(6) != 0,
        },
        _ => Op::Chain {
            ops: (0..1 + rng.below(4)).map(|_| chain_op(rng)).collect(),
            commit: rng.below(6) != 0,
        },
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
    consistent(model, after)
}

/// The tests' own consistency check of the chain tables (`support::chain`)
/// on a state the store was found to hold.
fn consistent(state: &State, after: &str) -> Res<()> {
    chain_view_of(|id| state.get(&id).cloned().unwrap_or_default())
        .map(drop)
        .map_err(|x| format!("after {after}: chain tables inconsistent: {x}"))
}

// The chain operations on the model.

fn hash_of(seed: u8) -> Hash32 {
    Hash32([seed; 32])
}

fn model_get<T: Table>(m: &State, key: &T::Key) -> Option<T::Value> {
    m.get(&T::ID)
        .and_then(|r| r.get(key.encode().as_ref()))
        .map(|v| T::Value::decode(v).expect("model row decodes"))
}

fn model_put<T: Table>(m: &mut State, key: &T::Key, value: &T::Value) {
    put(
        m,
        T::ID,
        key.encode().as_ref().to_vec(),
        value.encode().as_ref().to_vec(),
    );
}

fn model_del<T: Table>(m: &mut State, key: &T::Key) {
    del(m, T::ID, key.encode().as_ref());
}

/// The model's tip: the last main-chain block.
fn model_tip(m: &State) -> Option<(u32, Hash32)> {
    m.get(&TableId::Blocks)
        .and_then(|r| r.iter().next_back())
        .map(|(k, v)| {
            (
                Height::decode(k).expect("height").0,
                BlockRow::decode(v).expect("block row").hash,
            )
        })
}

fn set_model_tip(m: &mut State) {
    match model_tip(m) {
        Some((height, hash)) => put(
            m,
            TableId::Meta,
            MetaKey::Tip.name().to_vec(),
            Tip { height, hash }.encode().to_vec(),
        ),
        None => del(m, TableId::Meta, MetaKey::Tip.name()),
    }
}

/// `WriteTx::disconnect_from`, as documented.
fn model_disconnect_from(m: &mut State, height: u32) {
    let Some((tip, _)) = model_tip(m) else { return };
    if tip < height {
        return;
    }
    for h in (height..=tip).rev() {
        let key = Height(h);
        let row = model_get::<Blocks>(m, &key).expect("main-chain row");
        let header = model_get::<Headers>(m, &key);
        let filter = model_get::<CFilters>(m, &key);
        model_del::<Blocks>(m, &key);
        model_del::<Headers>(m, &key);
        model_del::<CFilters>(m, &key);
        model_del::<BlockByHash>(m, &row.hash);
        if let Some(header) = header {
            model_put::<HeadersDetached>(m, &row.hash, &header);
        }
        if let Some(filter) = filter {
            model_put::<CFiltersDetached>(m, &row.hash, &filter);
        }
    }
    set_model_tip(m);
}

/// `WriteTx::connect_block`, as documented.
fn model_connect(
    m: &mut State,
    height: u32,
    row: &BlockRow,
    header: &HeaderBytes,
    filter: Option<&CFilterRow>,
) {
    let key = Height(height);
    let detached = model_get::<CFiltersDetached>(m, &row.hash);
    model_del::<HeadersDetached>(m, &row.hash);
    model_del::<CFiltersDetached>(m, &row.hash);
    model_put::<Blocks>(m, &key, row);
    model_put::<Headers>(m, &key, header);
    model_put::<BlockByHash>(m, &row.hash, &key);
    if let Some(f) = filter.or(detached.as_ref()) {
        model_put::<CFilters>(m, &key, f);
    }
    set_model_tip(m);
}

type OwnedBlock = (BlockRow, HeaderBytes, Option<CFilterRow>);

/// The rows of `b` connected at `height` above `prev`.
fn owned_block(b: &SyncBlock, height: u32, prev: &Hash32) -> OwnedBlock {
    let row = BlockRow {
        hash: hash_of(b.hash),
        time: i64::from(height),
        vote_bits: u16::from(b.hash),
        invalid: false,
        wallet_txs: u32::from(b.wallet),
    };
    let header = chain_header(height, prev, b.hash ^ 0x5a);
    let filter = b.filter_len.map(|n| CFilterRow {
        key: [b.hash; 16],
        filter: vec![b.hash; n],
    });
    (row, header, filter)
}

fn as_chain(v: &[OwnedBlock]) -> Vec<ChainBlock<'_>> {
    v.iter()
        .map(|(row, header, filter)| ChainBlock {
            row,
            header,
            filter: filter.as_ref(),
        })
        .collect()
}

/// The blocks of `blocks` that may connect above the model's height
/// `keep` (nothing kept: `None`), each above the one before: those whose
/// hash is neither kept in the main chain nor earlier in the list.
fn resolve(m: &State, keep: Option<(u32, Hash32)>, blocks: &[SyncBlock]) -> Vec<OwnedBlock> {
    let mut out: Vec<OwnedBlock> = Vec::new();
    let mut height = keep.map_or(0, |(k, _)| k + 1);
    let mut prev = keep.map_or(Hash32::default(), |(_, h)| h);
    for b in blocks {
        let hash = hash_of(b.hash);
        let kept =
            model_get::<BlockByHash>(m, &hash).is_some_and(|h| keep.is_some_and(|(k, _)| h.0 <= k));
        if kept || out.iter().any(|o| o.0.hash == hash) {
            continue;
        }
        let o = owned_block(b, height, &prev);
        prev = hash;
        height += 1;
        out.push(o);
    }
    out
}

/// The counters of [`CHAIN`], by index.
const CONNECTED: usize = 0;
const DETACHED: usize = 1;
const REORGS: usize = 2;
const PROMOTED: usize = 3;
const REFUSED: usize = 4;
const DURABLE: usize = 5;
const WALLET_REFUSED: usize = 6;
const DURABLE_WALLET: usize = 7;
const DELIVERED_AGAIN: usize = 8;

thread_local! {
    /// What the chain operations of a run did on this thread (blocks
    /// connected, blocks detached, reorganisations, promotions, refusals,
    /// chain operations committed durably, header-batch refusals of wallet
    /// state, wallet-state blocks connected or detached durably, and
    /// main-chain blocks delivered again), so that a run whose operations
    /// were all skipped or refused does not pass for one that tested them.
    static CHAIN: [Cell<u64>; 9] = const { [const { Cell::new(0) }; 9] };
}

fn count(i: usize, n: u64) {
    CHAIN.with(|c| c[i].set(c[i].get() + n));
}

thread_local! {
    /// The record operations committed on this thread, by kind (in the
    /// order of [`RECORD_KINDS`]).
    static RECORDS: [Cell<u64>; 9] = const { [const { Cell::new(0) }; 9] };
}

const RECORD_KINDS: [&str; 9] = [
    "hosts recorded",
    "empty hosts refused",
    "tickets written",
    "tickets refused for an unknown host",
    "tickets deleted",
    "multisig outputs written",
    "multisig outputs deleted",
    "policies written or cleared",
    "agenda preferences written",
];

fn count_record_ops(ops: &[RecOp]) {
    for op in ops {
        let i = match op {
            RecOp::Host { host: 0, .. } => 1,
            RecOp::Host { .. } => 0,
            // At most four hosts exist (hosts 1 to 4), so host id 5 is
            // always refused; ids 1 to 4 not yet given are refused too and
            // counted with the written ones.
            RecOp::Ticket { host_id, .. } if *host_id > 4 => 3,
            RecOp::Ticket { .. } => 2,
            RecOp::DelTicket(_) => 4,
            RecOp::Multisig { .. } => 5,
            RecOp::DelMultisig { .. } => 6,
            RecOp::Policy { .. } => 7,
            RecOp::Agenda { .. } => 8,
        };
        RECORDS.with(|c| c[i].set(c[i].get() + 1));
    }
}

#[derive(Debug)]
enum SyncErr {
    Store(Error),
    Mismatch(String),
    /// A durable chain update abandoned on purpose.
    Abandon,
}

impl From<Error> for SyncErr {
    fn from(x: Error) -> Self {
        SyncErr::Store(x)
    }
}

/// A refusal the model predicts: the call must fail with `kind`, and the
/// batch goes on.
fn refused<T>(r: purse_store::Result<T>, kind: Kind, what: &str) -> Result<(), SyncErr> {
    match r {
        Err(x) if x.kind() == kind => {
            count(REFUSED, 1);
            Ok(())
        }
        Err(x) => Err(SyncErr::Mismatch(format!(
            "{what}: expected {kind:?}, got {x}"
        ))),
        Ok(_) => Err(SyncErr::Mismatch(format!("{what}: expected a refusal"))),
    }
}

/// The chain operations of a header batch ([`SyncBatch`], relaxed) and of
/// a durable write transaction ([`WriteTx`]), so one model drives both.
trait ChainTx {
    /// A header batch, which refuses block rows with wallet state.
    const RELAXED: bool;
    fn connect_blocks(&mut self, h: u32, b: &[ChainBlock<'_>]) -> purse_store::Result<()>;
    fn disconnect_from(&mut self, h: u32) -> purse_store::Result<Vec<(u32, BlockRow)>>;
    fn reorganize(
        &mut self,
        fork: u32,
        b: &[ChainBlock<'_>],
    ) -> purse_store::Result<Vec<(u32, BlockRow)>>;
    fn promote_block(&mut self, h: u32, row: &BlockRow) -> purse_store::Result<()>;
    fn insert_detached(
        &mut self,
        hash: &Hash32,
        header: &HeaderBytes,
        filter: Option<&CFilterRow>,
    ) -> purse_store::Result<()>;
    fn put_cfilter_at(&mut self, h: u32, f: &CFilterRow) -> purse_store::Result<()>;
}

macro_rules! chain_tx {
    ($t:ty, $relaxed:expr) => {
        impl ChainTx for $t {
            const RELAXED: bool = $relaxed;
            fn connect_blocks(&mut self, h: u32, b: &[ChainBlock<'_>]) -> purse_store::Result<()> {
                <$t>::connect_blocks(self, h, b)
            }
            fn disconnect_from(&mut self, h: u32) -> purse_store::Result<Vec<(u32, BlockRow)>> {
                <$t>::disconnect_from(self, h)
            }
            fn reorganize(
                &mut self,
                fork: u32,
                b: &[ChainBlock<'_>],
            ) -> purse_store::Result<Vec<(u32, BlockRow)>> {
                <$t>::reorganize(self, fork, b)
            }
            fn promote_block(&mut self, h: u32, row: &BlockRow) -> purse_store::Result<()> {
                <$t>::promote_block(self, h, row)
            }
            fn insert_detached(
                &mut self,
                hash: &Hash32,
                header: &HeaderBytes,
                filter: Option<&CFilterRow>,
            ) -> purse_store::Result<()> {
                <$t>::insert_detached(self, hash, header, filter)
            }
            fn put_cfilter_at(&mut self, h: u32, f: &CFilterRow) -> purse_store::Result<()> {
                <$t>::put_cfilter_at(self, h, f)
            }
        }
    };
}

chain_tx!(SyncBatch<'_, '_>, true);
chain_tx!(WriteTx<'_>, false);

/// The model's main-chain rows at `from` and above, highest first: what a
/// detach from `from` returns.
fn model_rows_from(m: &State, from: u32) -> Vec<(u32, BlockRow)> {
    let mut v: Vec<(u32, BlockRow)> = m
        .get(&TableId::Blocks)
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            (
                Height::decode(k).expect("height").0,
                BlockRow::decode(v).expect("block row"),
            )
        })
        .filter(|(h, _)| *h >= from)
        .collect();
    v.reverse();
    v
}

fn has_wallet_state(row: &BlockRow) -> bool {
    row.wallet_txs != 0 || row.invalid
}

/// A refusal of wallet state by a header batch: kind `Invalid`.
fn wallet_refused<T>(r: purse_store::Result<T>, what: &str) -> Result<(), SyncErr> {
    count(WALLET_REFUSED, 1);
    refused(r, Kind::Invalid, what)
}

/// The detach the store reported is the model's, row for row.
fn same_detach(
    got: &[(u32, BlockRow)],
    want: &[(u32, BlockRow)],
    what: &str,
) -> Result<(), SyncErr> {
    if got != want {
        return Err(SyncErr::Mismatch(format!(
            "{what} detached {} rows, model {}: {got:?} against {want:?}",
            got.len(),
            want.len()
        )));
    }
    Ok(())
}

/// Offers the main-chain block `hash`, with its stored row and header, to
/// `call`, which must refuse it as dcrwallet's `ExtendMainChain` refuses a
/// block delivered again: kind `Exist`, or `Invalid` for the genesis
/// block (a header stating height 0).
fn probe_main_chain_block<C: ChainTx>(
    b: &mut C,
    m: &State,
    hash: &Hash32,
    call: impl FnOnce(&mut C, &[ChainBlock<'_>]) -> purse_store::Result<()>,
) -> Result<(), SyncErr> {
    let at = model_get::<BlockByHash>(m, hash).expect("indexed");
    let row = model_get::<Blocks>(m, &at).expect("main-chain row");
    let header = model_get::<Headers>(m, &at).expect("main-chain header");
    let block = ChainBlock {
        row: &row,
        header: &header,
        filter: None,
    };
    let kind = if at.0 == 0 {
        Kind::Invalid
    } else {
        Kind::Exist
    };
    count(DELIVERED_AGAIN, 1);
    refused(
        call(b, &[block]),
        kind,
        "a main-chain block delivered again",
    )
}

/// Runs one chain operation on the transaction and on the model.
fn apply_chain_op<C: ChainTx>(b: &mut C, m: &mut State, op: &ChainOp) -> Result<(), SyncErr> {
    let tip = model_tip(m);
    match op {
        ChainOp::Connect(blocks) => {
            let next = tip.map_or(0, |(t, _)| t + 1);
            if let Some(dup) = blocks
                .iter()
                .find(|x| model_get::<BlockByHash>(m, &hash_of(x.hash)).is_some())
            {
                // A main-chain block delivered again with its own row and
                // header is refused first, whatever height is given (the
                // next, or its own) and whatever wallet state its row
                // carries: dcrwallet's `Exist`, or `Invalid` for the
                // genesis block.
                let given = if dup.hash % 2 == 0 { next } else { u32::MAX };
                probe_main_chain_block(b, m, &hash_of(dup.hash), |b, c| {
                    b.connect_blocks(given, c).map(drop)
                })?;
                // Its hash with a header of the next height, which is not
                // its header: kind `Invalid`. (The probe carries no wallet
                // state, so that a header batch refuses it for the hash.)
                let mut o = owned_block(dup, next, &tip.map_or(Hash32::default(), |t| t.1));
                o.0.wallet_txs = 0;
                refused(
                    b.connect_blocks(next, &as_chain(&[o])),
                    Kind::Invalid,
                    "connect a main-chain hash with another header",
                )?;
            }
            let v = resolve(m, tip, blocks);
            if v.is_empty() {
                return Ok(());
            }
            let wallet = v.iter().filter(|o| has_wallet_state(&o.0)).count() as u64;
            if C::RELAXED && wallet > 0 {
                return wallet_refused(
                    b.connect_blocks(next, &as_chain(&v)),
                    "a header batch connects a block with wallet state",
                );
            }
            b.connect_blocks(next, &as_chain(&v))?;
            count(CONNECTED, v.len() as u64);
            count(DURABLE_WALLET, wallet);
            for (i, (row, header, filter)) in v.iter().enumerate() {
                model_connect(m, next + i as u32, row, header, filter.as_ref());
            }
        }
        ChainOp::Disconnect(h) => {
            if *h == 0 {
                return refused(
                    b.disconnect_from(0),
                    Kind::Invalid,
                    "detach the genesis block",
                );
            }
            let want = model_rows_from(m, *h);
            let wallet = want.iter().filter(|(_, r)| has_wallet_state(r)).count() as u64;
            if C::RELAXED && wallet > 0 {
                return wallet_refused(
                    b.disconnect_from(*h),
                    "a header batch detaches a block with wallet state",
                );
            }
            let got = b.disconnect_from(*h)?;
            same_detach(&got, &want, &format!("disconnect_from({h})"))?;
            count(DETACHED, got.len() as u64);
            count(DURABLE_WALLET, wallet);
            model_disconnect_from(m, *h);
        }
        ChainOp::Reorg { depth, blocks } => {
            let Some((t, _)) = tip else {
                return refused(
                    b.reorganize(0, &[]),
                    Kind::Invalid,
                    "reorganise an empty chain",
                );
            };
            let fork = t.saturating_sub(*depth);
            let fork_hash = model_get::<Blocks>(m, &Height(fork))
                .expect("fork row")
                .hash;
            // A block that stays, delivered again: refused as in a connect.
            if let Some(dup) = blocks.iter().find(|x| {
                model_get::<BlockByHash>(m, &hash_of(x.hash)).is_some_and(|h| h.0 <= fork)
            }) {
                probe_main_chain_block(b, m, &hash_of(dup.hash), |b, c| {
                    b.reorganize(fork, c).map(drop)
                })?;
            }
            let v = resolve(m, Some((fork, fork_hash)), blocks);
            let want = model_rows_from(m, fork + 1);
            let wallet = (want.iter().filter(|(_, r)| has_wallet_state(r)).count()
                + v.iter().filter(|o| has_wallet_state(&o.0)).count())
                as u64;
            if C::RELAXED && wallet > 0 {
                return wallet_refused(
                    b.reorganize(fork, &as_chain(&v)),
                    "a header batch reorganises over wallet state",
                );
            }
            let got = b.reorganize(fork, &as_chain(&v))?;
            if !got.is_empty() && !v.is_empty() {
                count(REORGS, 1);
            }
            same_detach(&got, &want, &format!("reorganize({fork})"))?;
            count(DURABLE_WALLET, wallet);
            model_disconnect_from(m, fork + 1);
            for (i, (row, header, filter)) in v.iter().enumerate() {
                model_connect(m, fork + 1 + i as u32, row, header, filter.as_ref());
            }
        }
        ChainOp::Promote(seed, wallet) => {
            let next = tip.map_or(0, |(t, _)| t + 1);
            let fits_tip = |hd: &HeaderBytes| {
                hd.height() == next && (next == 0 || Some(hd.prev_block()) == tip.map(|t| t.1))
            };
            // A detached block that fits the tip, if there is one;
            // otherwise the seed's, which may not.
            let hash = m
                .get(&TableId::HeadersDetached)
                .into_iter()
                .flatten()
                .find(|(_, v)| fits_tip(&HeaderBytes::decode(v).expect("header")))
                .map_or(hash_of(*seed), |(k, _)| Hash32::decode(k).expect("hash"));
            let seed = &hash.0[0];
            let row = BlockRow {
                hash,
                time: i64::from(next),
                vote_bits: u16::from(*seed),
                invalid: false,
                wallet_txs: u32::from(*wallet),
            };
            if C::RELAXED && *wallet {
                // Refused for the row's wallet state, or for no detached
                // header that fits: kind `Invalid` either way.
                return wallet_refused(
                    b.promote_block(next, &row),
                    "a header batch promotes a block with wallet state",
                );
            }
            let fits = model_get::<HeadersDetached>(m, &hash).filter(fits_tip);
            match fits {
                Some(header) => {
                    b.promote_block(next, &row)?;
                    count(PROMOTED, 1);
                    count(DURABLE_WALLET, u64::from(*wallet));
                    model_connect(m, next, &row, &header, None);
                }
                None => refused(b.promote_block(next, &row), Kind::Invalid, "promote")?,
            }
        }
        ChainOp::Detach { hash, filter } => {
            let h = hash_of(*hash);
            let header = chain_header(u32::from(*hash), &hash_of(hash ^ 1), hash ^ 0x33);
            let f = filter.then(|| CFilterRow {
                key: [*hash ^ 0x33; 16],
                filter: vec![*hash; 7],
            });
            if model_get::<BlockByHash>(m, &h).is_some() {
                return refused(
                    b.insert_detached(&h, &header, f.as_ref()),
                    Kind::Exist,
                    "detach a main-chain block",
                );
            }
            b.insert_detached(&h, &header, f.as_ref())?;
            model_put::<HeadersDetached>(m, &h, &header);
            if let Some(f) = f {
                model_put::<CFiltersDetached>(m, &h, &f);
            }
        }
        ChainOp::Filter { height, len } => {
            let f = CFilterRow {
                key: [0xf1; 16],
                filter: vec![0xf1; *len],
            };
            if model_get::<Blocks>(m, &Height(*height)).is_none() {
                return refused(
                    b.put_cfilter_at(*height, &f),
                    Kind::Invalid,
                    "filter above the chain",
                );
            }
            b.put_cfilter_at(*height, &f)?;
            model_put::<CFilters>(m, &Height(*height), &f);
        }
    }
    Ok(())
}

// The record operations on the model, written from their documentation
// (`purse_store::records`) over raw rows.

fn host_bytes(n: u8) -> Vec<u8> {
    if n == 0 {
        Vec::new()
    } else {
        format!("https://vsp{n}.example").into_bytes()
    }
}

fn rows_of(m: &State, t: TableId) -> Rows {
    m.get(&t).cloned().unwrap_or_default()
}

fn status_key(status: &[u8], ticket: &[u8]) -> Vec<u8> {
    [status, ticket].concat()
}

/// Expects a refusal of kind `Invalid` ending in `text`.
fn rec_refused<T: std::fmt::Debug>(r: purse_store::Result<T>, text: &str) -> Result<(), TxErr> {
    match r {
        Err(x) if x.kind() == Kind::Invalid && x.to_string().ends_with(text) => Ok(()),
        other => Err(TxErr::Mismatch(format!(
            "expected the refusal {text:?}, got {other:?}"
        ))),
    }
}

fn apply_rec_op(tx: &mut WriteTx<'_>, m: &mut State, op: &RecOp) -> Result<(), TxErr> {
    match op {
        RecOp::Host { host, pubkey } => {
            let host = host_bytes(*host);
            if host.is_empty() {
                return rec_refused(tx.add_vsp_host(&host, &[*pubkey; 33]), "key required");
            }
            let want = match rows_of(m, TableId::VspPubKeys).get(&host) {
                Some(row) => u32::from_be_bytes(row[..4].try_into().expect("id")),
                None => {
                    let id = rows_of(m, TableId::VspHosts)
                        .keys()
                        .next_back()
                        .map_or(0, |k| u32::from_be_bytes(k[..].try_into().expect("id")))
                        + 1;
                    put(
                        m,
                        TableId::VspHosts,
                        id.to_be_bytes().to_vec(),
                        host.clone(),
                    );
                    let mut v = id.to_be_bytes().to_vec();
                    v.extend_from_slice(&[*pubkey; 33]);
                    put(m, TableId::VspPubKeys, host.clone(), v);
                    id
                }
            };
            let got = tx.add_vsp_host(&host, &[*pubkey; 33])?;
            if got != want {
                return Err(TxErr::Mismatch(format!("host id {got}, model {want}")));
            }
        }
        RecOp::Ticket {
            ticket,
            status,
            host_id,
            fee,
        } => {
            let t = hash_of(*ticket).0.to_vec();
            let row = VspTicketRow {
                fee_hash: if *fee == 0 {
                    Hash32::default()
                } else {
                    hash_of(*fee)
                },
                fee_status: *status,
                host_id: *host_id,
            };
            if *host_id != 0
                && !rows_of(m, TableId::VspHosts).contains_key(&host_id.to_be_bytes()[..])
            {
                return rec_refused(
                    tx.put_vsp_ticket(&hash_of(*ticket), &row),
                    "unknown host id",
                );
            }
            if let Some(old) = rows_of(m, TableId::VspTickets).get(&t) {
                del(m, TableId::VspFeeStatus, &status_key(&old[32..36], &t));
            }
            put(m, TableId::VspTickets, t.clone(), row.encode().to_vec());
            put(
                m,
                TableId::VspFeeStatus,
                status_key(&status.to_be_bytes(), &t),
                Vec::new(),
            );
            tx.put_vsp_ticket(&hash_of(*ticket), &row)?;
        }
        RecOp::DelTicket(ticket) => {
            let t = hash_of(*ticket).0.to_vec();
            let was = match rows_of(m, TableId::VspTickets).get(&t) {
                Some(old) => {
                    del(m, TableId::VspFeeStatus, &status_key(&old[32..36], &t));
                    del(m, TableId::VspTickets, &t);
                    true
                }
                None => false,
            };
            let got = tx.delete_vsp_ticket(&hash_of(*ticket))?;
            if got != was {
                return Err(TxErr::Mismatch(format!("ticket delete {got}, model {was}")));
            }
        }
        RecOp::Multisig {
            hash,
            index,
            script,
            mined,
            spent,
            stake,
        } => {
            let o = Outpoint {
                hash: hash_of(*hash),
                index: *index,
            };
            let ok = o.encode().to_vec();
            let row = MultisigOutRow {
                script_hash: Hash160([*script; 20]),
                m: 1,
                n: 3,
                stake_tree: *stake,
                block: mined
                    .then(|| MultisigBlock::new(hash_of(0xbb), 7))
                    .flatten(),
                amount: 1_000 + i64::from(*index),
                spender: spent.then_some(MultisigSpender {
                    hash: hash_of(0xcc),
                    input: 1,
                }),
            };
            if let Some(old) = rows_of(m, TableId::MultisigOuts).get(&ok)
                && old[22] & 1 == 0
            {
                del(m, TableId::MultisigUnspent, &[&old[..20], &ok[..]].concat());
            }
            put(m, TableId::MultisigOuts, ok.clone(), row.encode().to_vec());
            if !spent {
                put(
                    m,
                    TableId::MultisigUnspent,
                    [&[*script; 20][..], &ok[..]].concat(),
                    Vec::new(),
                );
            }
            tx.put_multisig_out(&o, &row)?;
        }
        RecOp::DelMultisig { hash, index } => {
            let o = Outpoint {
                hash: hash_of(*hash),
                index: *index,
            };
            let ok = o.encode().to_vec();
            let old = rows_of(m, TableId::MultisigOuts).get(&ok).cloned();
            if let Some(old) = &old {
                if old[22] & 1 == 0 {
                    del(m, TableId::MultisigUnspent, &[&old[..20], &ok[..]].concat());
                }
                del(m, TableId::MultisigOuts, &ok);
            }
            let got = tx.delete_multisig_out(&o)?;
            if got.map(|r| r.encode().to_vec()) != old {
                return Err(TxErr::Mismatch(
                    "multisig delete returned another row".into(),
                ));
            }
        }
        RecOp::Policy { table, key, vote } => {
            let (t, k) = match table {
                0 => (TableId::TSpendPolicies, hash_of(*key).0.to_vec()),
                1 => (TableId::TreasuryKeyPolicies, vec![0x02 + *key; 33]),
                2 => (
                    TableId::VspTSpendPolicies,
                    [hash_of(*key).0, hash_of(0x70).0].concat(),
                ),
                _ => (
                    TableId::VspTreasuryKeyPolicies,
                    [
                        &hash_of(*key).0[..],
                        &vec![0x03; usize::from(*key) * 11][..],
                    ]
                    .concat(),
                ),
            };
            if *vote == 0 {
                let was = rows_of(m, t).contains_key(&k);
                del(m, t, &k);
                if tx.delete_raw(t, &k)? != was {
                    return Err(TxErr::Mismatch("policy delete".into()));
                }
            } else {
                let v = TreasuryVote::new(*vote).expect("non-zero");
                put(m, t, k.clone(), v.encode().to_vec());
                match t {
                    TableId::TSpendPolicies => tx.put::<TSpendPolicies>(&hash_of(*key), &v)?,
                    TableId::TreasuryKeyPolicies => {
                        tx.put::<TreasuryKeyPolicies>(&PiKey(k.clone()), &v)?
                    }
                    TableId::VspTSpendPolicies => tx.put::<VspTSpendPolicies>(
                        &VspTSpendKey {
                            ticket: hash_of(*key),
                            tspend: hash_of(0x70),
                        },
                        &v,
                    )?,
                    _ => tx.put::<VspTreasuryKeyPolicies>(
                        &VspTreasuryKey {
                            ticket: hash_of(*key),
                            pikey: k[32..].to_vec(),
                        },
                        &v,
                    )?,
                }
            }
        }
        RecOp::Agenda {
            ticket,
            version,
            agenda,
            choice,
        } => {
            let agenda = vec![b'a' + agenda; usize::from(*agenda)];
            let choice = AgendaChoice(vec![b'c'; usize::from(*choice)]);
            match ticket {
                None => {
                    let k = AgendaKey {
                        version: *version,
                        agenda,
                    };
                    put(m, TableId::AgendaPrefs, k.encode(), choice.encode());
                    tx.put::<AgendaPrefs>(&k, &choice)?;
                }
                Some(t) => {
                    let k = TicketAgendaKey {
                        ticket: hash_of(*t),
                        version: *version,
                        agenda,
                    };
                    put(m, TableId::TicketAgendaPrefs, k.encode(), choice.encode());
                    tx.put::<TicketAgendaPrefs>(&k, &choice)?;
                }
            }
        }
    }
    Ok(())
}

/// The record tables hold their invariants, by the store's check and by
/// the tests' own on the model.
fn check_records(store: &Store, model: &State, after: &str) -> Res<()> {
    store
        .read()
        .and_then(|tx| tx.check_records())
        .map_err(|x| format!("after {after}: {x}"))?;
    records_view_of(|id| rows_of(model, id))
        .map(drop)
        .map_err(|x| format!("after {after}: record tables inconsistent: {x}"))
}

/// The chain tables hold their invariants.
fn check_chain(store: &Store, after: &str) -> Res<()> {
    store
        .read()
        .and_then(|tx| tx.check_chain())
        .map_err(|x| format!("after {after}: {x}"))
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
        Op::Chain { ops, commit } => {
            let mut pending = model.clone();
            let r = h.store().update(|tx| {
                for o in ops {
                    apply_chain_op(tx, &mut pending, o)?;
                }
                if *commit {
                    Ok(())
                } else {
                    Err(SyncErr::Abandon)
                }
            });
            match r {
                Ok(()) => {
                    count(DURABLE, ops.len() as u64);
                    *model = pending;
                }
                Err(SyncErr::Abandon) => {}
                Err(SyncErr::Store(x)) => return Err(e(x)),
                Err(SyncErr::Mismatch(m)) => return Err(m),
            }
            compare(h.store(), model, "chain update")?;
            check_chain(h.store(), "chain update")
        }
        Op::Records { ops, commit } => {
            let mut pending = model.clone();
            let r = h.store().update(|tx| {
                for o in ops {
                    apply_rec_op(tx, &mut pending, o)?;
                }
                if *commit { Ok(()) } else { Err(TxErr::Abandon) }
            });
            match r {
                Ok(()) => {
                    count_record_ops(ops);
                    *model = pending;
                }
                Err(TxErr::Abandon) => {}
                Err(TxErr::Store(x)) => return Err(e(x)),
                Err(TxErr::Mismatch(m)) => return Err(m),
            }
            compare(h.store(), model, "record update")?;
            check_records(h.store(), model, "record update")
        }
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
    batches: &[Vec<ChainOp>],
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
        for (i, ops) in batches.iter().enumerate() {
            if end == SyncEnd::Crash(i) {
                break;
            }
            let mut pending = model.clone();
            let ((), durable) = sync
                .batch(|b| {
                    for op in ops {
                        apply_chain_op(b, &mut pending, op)?;
                    }
                    Ok::<_, SyncErr>(())
                })
                .map_err(|x| match x {
                    SyncErr::Store(x) => e(x),
                    SyncErr::Mismatch(m) => m,
                    SyncErr::Abandon => "a header batch abandoned".to_owned(),
                })?;
            *model = pending;
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
        consistent(model, "a power loss in a sync")?;
        return check_chain(h.store(), "a power loss in a sync");
    }
    compare(h.store(), model, "sync")?;
    check_chain(h.store(), "sync")
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
        Op::Chain { commit: true, .. } => "chain update",
        Op::Chain { commit: false, .. } => "abandoned chain update",
        Op::Records { commit: true, .. } => "record update",
        Op::Records { commit: false, .. } => "abandoned record update",
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
    let c: Vec<(&str, u64)> = [
        "connected",
        "detached",
        "reorganisations",
        "promoted",
        "refused",
        "durable chain operations",
        "wallet state refused by a header batch",
        "wallet-state blocks moved durably",
        "main-chain blocks delivered again",
    ]
    .into_iter()
    .zip(CHAIN.with(|c| c.each_ref().map(Cell::get)))
    .collect();
    eprintln!("chain operations: {c:?}");
    for &(what, n) in &c {
        assert!(n > 0, "no chain operation {what}: {c:?}");
    }
    let r: Vec<(&str, u64)> = RECORD_KINDS
        .into_iter()
        .zip(RECORDS.with(|c| c.each_ref().map(Cell::get)))
        .collect();
    eprintln!("record operations committed: {r:?}");
    for &(what, n) in &r {
        assert!(n > 0, "no record operation {what}: {r:?}");
    }
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
