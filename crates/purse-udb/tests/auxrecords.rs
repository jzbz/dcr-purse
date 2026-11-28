// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests of the auxiliary records (`purse_udb::vsp`,
//! `purse_udb::treasury`, `purse_udb::agendas`): random sequences of their
//! operations over small sets of tickets, hosts, pi keys and agendas, run
//! against a model of dcrwallet's rules (spec 05 sections 5.7 and 5.8;
//! `wallet/udb/agendas.go`), with
//!
//! - every reader's answer equal to the model's after every operation, and
//!   the store's own record check (`ReadTx::check_records`) passing;
//! - every refused operation writing nothing: the record tables' raw rows
//!   are the same after the refusal as before it, in the same transaction,
//!   which then commits (the refusal leaves it usable);
//! - an abandoned transaction leaving the store as it was;
//! - the store closed and opened again answering every reader, and writing
//!   every record table's rows and the logical export, as it did before.
//!
//! `PURSE_TEST_SEED` moves the streams; `PURSE_UDB_AUX_CASES` sets the
//! number of sequences (default 10, 80 operations each; every operation
//! commits durably, which is most of the time taken).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts and ids"
)]

mod support;

use std::collections::BTreeMap;
use std::ops::Bound;

use purse_errors::{AnyError, Kind};
use purse_snacl::{Limits, OsRandom};
use purse_store::{Hash32, RawEntry, ReadTx, Store, WriteTx};
use purse_testutil::SplitMix64;
use purse_udb::Manager;
use purse_udb::addrmgr::ChainParams;
use purse_udb::export::{ExportOptions, export};
use purse_udb::vsp::VspTicket;
use purse_udb::{agendas, treasury, vsp};
use support::scratch;

type H = [u8; 32];

/// dcrwallet's rules over plain maps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Model {
    /// ticket → (fee hash, status, host id)
    tickets: BTreeMap<H, (H, u32, u32)>,
    /// host id − 1 → (host, first public key)
    hosts: Vec<(Vec<u8>, Vec<u8>)>,
    tspend: BTreeMap<H, u8>,
    vsp_tspend: BTreeMap<(H, H), u8>,
    keys: BTreeMap<Vec<u8>, u8>,
    vsp_keys: BTreeMap<(H, Vec<u8>), u8>,
    agendas: BTreeMap<(u32, Vec<u8>), Vec<u8>>,
    ticket_agendas: BTreeMap<(H, u32, Vec<u8>), Vec<u8>>,
}

/// One operation, as drawn.
#[derive(Clone, Debug)]
enum Op {
    SetTicket(H, H, u32, Vec<u8>, Vec<u8>),
    DeleteTicket(H),
    TSpend(H, u8),
    VspTSpend(H, H, u8),
    Key(Vec<u8>, u8),
    VspKey(H, Vec<u8>, u8),
    Agenda(u32, Vec<u8>, Vec<u8>),
    TicketAgenda(H, u32, Vec<u8>, Vec<u8>),
}

const MAX_KEY: usize = purse_store::MAX_KEY_SIZE;

/// The pools the operations draw from.
struct Pools {
    hashes: Vec<H>,
    hosts: Vec<Vec<u8>>,
    pikeys: Vec<Vec<u8>>,
    agendas: Vec<Vec<u8>>,
    choices: Vec<Vec<u8>>,
}

impl Pools {
    fn new(r: &mut SplitMix64) -> Pools {
        // Four random hashes and the two ends of the hash range, which
        // bound every per-status range of the fee status index.
        let mut hashes: Vec<H> = (0..4)
            .map(|_| {
                let mut b = [0u8; 32];
                r.fill(&mut b);
                b
            })
            .collect();
        hashes.extend([[0; 32], [0xff; 32]]);
        Pools {
            hashes,
            hosts: vec![
                b"https://a.example".to_vec(),
                b"https://a.example/".to_vec(),
                b"https://b.example".to_vec(),
                b"\xff\x00host".to_vec(),
                Vec::new(),
                // The longest host taken, then one byte more: the store's
                // length guards on both sides of the limit.
                vec![b'h'; MAX_KEY],
                vec![b'h'; MAX_KEY + 1],
            ],
            pikeys: vec![
                [&[2u8][..], &[0xc1; 32]].concat(),
                [&[3u8][..], &[0xd2; 32]].concat(),
                vec![2, 1],
                Vec::new(),
                vec![1; MAX_KEY + 1],
                vec![1; MAX_KEY - 31],
                vec![1; MAX_KEY - 32],
            ],
            agendas: vec![
                b"treasury".to_vec(),
                b"maxblocksize".to_vec(),
                Vec::new(),
                vec![b'a'; MAX_KEY - 4],
                vec![b'a'; MAX_KEY - 3],
            ],
            choices: vec![
                b"yes".to_vec(),
                b"no".to_vec(),
                Vec::new(),
                b"\x00".to_vec(),
            ],
        }
    }

    fn hash(&self, r: &mut SplitMix64) -> H {
        self.hashes[r.below(self.hashes.len() as u64) as usize]
    }

    fn pick(v: &[Vec<u8>], r: &mut SplitMix64) -> Vec<u8> {
        // The long entries are drawn rarely, but for one of exactly the
        // key limit (only the host pool has one): a host stored at the
        // limit is set again within a sequence, so the store's guard on
        // finding a known host is reached at its boundary.
        let i = r.below(v.len() as u64) as usize;
        if v[i].len() > 64 && v[i].len() != MAX_KEY && r.below(4) != 0 {
            v[0].clone()
        } else {
            v[i].clone()
        }
    }

    fn policy(r: &mut SplitMix64) -> u8 {
        match r.below(6) {
            0 => 0,
            1 => r.below(256) as u8,
            n => 1 + (n % 2) as u8,
        }
    }

    fn op(&self, r: &mut SplitMix64) -> Op {
        match r.below(10) {
            0..=2 => Op::SetTicket(
                self.hash(r),
                if r.below(3) == 0 {
                    [0; 32]
                } else {
                    self.hash(r)
                },
                r.below(5) as u32,
                Self::pick(&self.hosts, r),
                vec![r.below(3) as u8 + 1; 33],
            ),
            3 => Op::DeleteTicket(self.hash(r)),
            4 => Op::TSpend(self.hash(r), Self::policy(r)),
            5 => Op::VspTSpend(self.hash(r), self.hash(r), Self::policy(r)),
            6 => Op::Key(Self::pick(&self.pikeys, r), Self::policy(r)),
            7 => Op::VspKey(self.hash(r), Self::pick(&self.pikeys, r), Self::policy(r)),
            8 => Op::Agenda(
                10 + r.below(2) as u32,
                Self::pick(&self.agendas, r),
                Self::pick(&self.choices, r),
            ),
            _ => Op::TicketAgenda(
                self.hash(r),
                10 + r.below(2) as u32,
                Self::pick(&self.agendas, r),
                Self::pick(&self.choices, r),
            ),
        }
    }
}

impl Model {
    /// Applies `op` under dcrwallet's rules: `Err(kind)` when Go refuses it
    /// (nothing changes then).
    fn apply(&mut self, op: &Op) -> Result<(), Kind> {
        match op {
            Op::SetTicket(t, fee, st, host, pk) => {
                if host.is_empty() || host.len() > MAX_KEY {
                    return Err(Kind::Invalid);
                }
                let id = match self.hosts.iter().position(|(h, _)| h == host) {
                    Some(i) => i + 1,
                    None => {
                        self.hosts.push((host.clone(), pk.clone()));
                        self.hosts.len()
                    }
                };
                self.tickets
                    .insert(*t, (*fee, *st, u32::try_from(id).expect("small")));
            }
            Op::DeleteTicket(t) => {
                self.tickets.remove(t);
            }
            Op::TSpend(h, p) => set(&mut self.tspend, *h, *p),
            Op::VspTSpend(t, h, p) => set(&mut self.vsp_tspend, (*t, *h), *p),
            Op::Key(k, p) => {
                if *p != 0 && (k.is_empty() || k.len() > MAX_KEY) {
                    return Err(Kind::Invalid);
                }
                set(&mut self.keys, k.clone(), *p);
            }
            Op::VspKey(t, k, p) => {
                if *p != 0 && k.len() + 32 > MAX_KEY {
                    return Err(Kind::Invalid);
                }
                set(&mut self.vsp_keys, (*t, k.clone()), *p);
            }
            Op::Agenda(v, a, c) => {
                if a.len() + 4 > MAX_KEY {
                    return Err(Kind::Io);
                }
                self.agendas.insert((*v, a.clone()), c.clone());
            }
            Op::TicketAgenda(t, v, a, c) => {
                if a.len() + 4 > MAX_KEY {
                    return Err(Kind::Io);
                }
                self.ticket_agendas.insert((*t, *v, a.clone()), c.clone());
            }
        }
        Ok(())
    }
}

fn set<K: Ord>(m: &mut BTreeMap<K, u8>, k: K, p: u8) {
    if p == 0 {
        m.remove(&k);
    } else {
        m.insert(k, p);
    }
}

/// Runs `op` through purse-udb.
fn run(tx: &mut WriteTx<'_>, op: &Op) -> Result<(), AnyError> {
    match op {
        Op::SetTicket(t, fee, st, host, pk) => {
            let rec = VspTicket {
                fee_hash: Hash32(*fee),
                fee_tx_status: *st,
                vsp_host_id: 99,
                host: host.clone(),
                pub_key: pk.clone(),
            };
            vsp::set_vsp_ticket(tx, &Hash32(*t), &rec).map(drop)
        }
        Op::DeleteTicket(t) => vsp::delete_vsp_ticket(tx, &Hash32(*t)),
        Op::TSpend(h, p) => treasury::set_tspend_policy(tx, &Hash32(*h), *p),
        Op::VspTSpend(t, h, p) => treasury::set_vsp_tspend_policy(tx, &Hash32(*t), &Hash32(*h), *p),
        Op::Key(k, p) => treasury::set_treasury_key_policy(tx, k, *p),
        Op::VspKey(t, k, p) => treasury::set_vsp_treasury_key_policy(tx, &Hash32(*t), k, *p),
        Op::Agenda(v, a, c) => agendas::set_default_agenda_preference(tx, *v, a, c),
        Op::TicketAgenda(t, v, a, c) => {
            agendas::set_ticket_agenda_preference(tx, &Hash32(*t), *v, a, c)
        }
    }
}

/// Every record table's raw rows.
fn dump(tx: &mut WriteTx<'_>) -> Vec<Vec<RawEntry>> {
    purse_store::RECORD_TABLES
        .iter()
        .map(|t| {
            tx.range_raw(*t, Bound::Unbounded, Bound::Unbounded)
                .expect("range")
                .collect::<purse_store::Result<Vec<_>>>()
                .expect("rows")
        })
        .collect()
}

fn dump_read(tx: &ReadTx) -> Vec<Vec<RawEntry>> {
    purse_store::RECORD_TABLES
        .iter()
        .map(|t| {
            tx.range_raw(*t, Bound::Unbounded, Bound::Unbounded)
                .expect("range")
                .collect::<purse_store::Result<Vec<_>>>()
                .expect("rows")
        })
        .collect()
}

/// Every reader against the model.
fn check(m: &Model, p: &Pools, tx: &mut ReadTx, ctx: &str) {
    tx.check_records().unwrap_or_else(|e| panic!("{ctx}: {e}"));
    for t in &p.hashes {
        let got = vsp::get_vsp_ticket(tx, &Hash32(*t));
        match m.tickets.get(t) {
            Some((fee, st, id)) => {
                let (host, pk) = m.hosts[*id as usize - 1].clone();
                let want = VspTicket {
                    fee_hash: Hash32(*fee),
                    fee_tx_status: *st,
                    vsp_host_id: *id,
                    host,
                    pub_key: pk,
                };
                assert_eq!(got.expect("ticket"), want, "{ctx}");
            }
            None => assert!(got.expect_err("none").is_kind(Kind::NotExist), "{ctx}"),
        }
    }
    for st in -1..6i64 {
        let got = vsp::get_vsp_tickets_by_fee_status(tx, st).expect("listing");
        let want: Vec<(H, u32)> = m
            .tickets
            .iter()
            .filter(|(_, (_, s, _))| i64::from(*s) == st)
            .map(|(t, (_, _, id))| (*t, *id))
            .collect();
        assert_eq!(
            got.iter()
                .map(|(t, v)| (t.0, v.vsp_host_id))
                .collect::<Vec<_>>(),
            want,
            "{ctx}: status {st}"
        );
        assert!(
            got.iter()
                .all(|(_, v)| v.host.is_empty() && v.pub_key.is_empty())
        );
    }
    assert_eq!(vsp::get_vsp_host(tx, 0).expect("host 0"), b"", "{ctx}");
    for id in 1..=m.hosts.len() + 1 {
        let got = vsp::get_vsp_host(tx, u32::try_from(id).expect("small"));
        match m.hosts.get(id - 1) {
            Some((h, _)) => assert_eq!(&got.expect("host"), h, "{ctx}"),
            None => assert!(got.expect_err("none").is_kind(Kind::NotExist), "{ctx}"),
        }
    }
    for h in &p.hosts {
        let got = vsp::get_vsp_pub_key(tx, h);
        match m.hosts.iter().position(|(x, _)| x == h) {
            Some(i) => assert_eq!(
                got.expect("pubkey"),
                (u32::try_from(i + 1).expect("small"), m.hosts[i].1.clone()),
                "{ctx}"
            ),
            None => assert!(got.expect_err("none").is_kind(Kind::NotExist), "{ctx}"),
        }
    }
    for a in &p.hashes {
        let want = m.tspend.get(a).copied().unwrap_or(0);
        assert_eq!(
            treasury::tspend_policy(tx, &Hash32(*a)).expect("p"),
            want,
            "{ctx}"
        );
        for b in &p.hashes {
            let want = m.vsp_tspend.get(&(*a, *b)).copied().unwrap_or(0);
            let got = treasury::vsp_tspend_policy(tx, &Hash32(*a), &Hash32(*b)).expect("p");
            assert_eq!(got, want, "{ctx}");
        }
        for k in &p.pikeys {
            let want = m.vsp_keys.get(&(*a, k.clone())).copied().unwrap_or(0);
            let got = treasury::vsp_treasury_key_policy(tx, &Hash32(*a), k).expect("p");
            assert_eq!(got, want, "{ctx}");
        }
    }
    for k in &p.pikeys {
        let want = m.keys.get(k).copied().unwrap_or(0);
        assert_eq!(
            treasury::treasury_key_policy(tx, k).expect("p"),
            want,
            "{ctx}"
        );
    }
    let tspend: Vec<(H, u8)> = treasury::tspend_policies(tx)
        .expect("all")
        .into_iter()
        .map(|(h, v)| (h.0, v))
        .collect();
    assert_eq!(
        tspend,
        m.tspend.clone().into_iter().collect::<Vec<_>>(),
        "{ctx}"
    );
    let vsp_tspend: Vec<((H, H), u8)> = treasury::vsp_tspend_policies(tx)
        .expect("all")
        .into_iter()
        .map(|(k, v)| ((k.ticket.0, k.tspend.0), v))
        .collect();
    assert_eq!(
        vsp_tspend,
        m.vsp_tspend.clone().into_iter().collect::<Vec<_>>(),
        "{ctx}"
    );
    assert_eq!(
        treasury::treasury_key_policies(tx).expect("all"),
        m.keys.clone().into_iter().collect::<Vec<_>>(),
        "{ctx}"
    );
    let vsp_keys: Vec<((H, Vec<u8>), u8)> = treasury::vsp_treasury_key_policies(tx)
        .expect("all")
        .into_iter()
        .map(|(k, v)| ((k.ticket.0, k.pikey), v))
        .collect();
    assert_eq!(
        vsp_keys,
        m.vsp_keys.clone().into_iter().collect::<Vec<_>>(),
        "{ctx}"
    );
    for v in 10..12 {
        for a in &p.agendas {
            let want = m.agendas.get(&(v, a.clone())).cloned().unwrap_or_default();
            assert_eq!(
                agendas::default_agenda_preference(tx, v, a).expect("c"),
                want,
                "{ctx}"
            );
            for t in &p.hashes {
                let want = m
                    .ticket_agendas
                    .get(&(*t, v, a.clone()))
                    .cloned()
                    .unwrap_or_default();
                let got = agendas::ticket_agenda_preference(tx, &Hash32(*t), v, a).expect("c");
                assert_eq!(got, want, "{ctx}");
            }
        }
    }
}

/// Why a write transaction did not commit.
#[derive(Debug)]
enum Abort {
    /// The test abandoned it.
    Abandoned,
    /// The store failed.
    Store(purse_store::Error),
}

impl From<purse_store::Error> for Abort {
    fn from(e: purse_store::Error) -> Abort {
        Abort::Store(e)
    }
}

fn export_text(store: &Store, chain: &ChainParams) -> String {
    let tx = store.read().expect("read");
    let opts = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/auxrecords.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    export(&tx, chain, &opts).expect("export")
}

#[test]
fn auxiliary_records_follow_dcrwallets_rules() {
    let cases: usize = std::env::var("PURSE_UDB_AUX_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let mut r = SplitMix64::from_env("purse-udb auxiliary records");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let (mut refused, mut abandoned, mut reopened, mut applied) = (0, 0, 0, 0);
    // Sets of a host at the key limit that the store already held.
    let mut known_at_limit = 0;
    for case in 0..cases {
        let pools = Pools::new(&mut r);
        let dir = scratch("auxrecords");
        let path = dir.join(purse_store::STORE_FILE_NAME);
        let mut store = Store::create(&path).expect("create");
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &[7; 32],
                    b"public",
                    b"private",
                    &mut OsRandom,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        let mut model = Model::default();
        for step in 0..80 {
            let op = pools.op(&mut r);
            let ctx = format!("case {case} step {step} {op:?}");
            let ctx = &ctx[..ctx.len().min(200)];
            let abandon = r.below(8) == 0;
            let mut next = model.clone();
            let want = next.apply(&op);
            let outcome: Result<Result<(), AnyError>, Abort> = store.update(|tx| {
                let before = dump(tx);
                let got = run(tx, &op);
                if got.is_err() {
                    assert_eq!(dump(tx), before, "{ctx}: a refusal wrote");
                }
                if abandon {
                    return Err(Abort::Abandoned);
                }
                Ok(got)
            });
            match (abandon, outcome) {
                (true, Err(Abort::Abandoned)) => abandoned += 1,
                (false, Ok(got)) => match (&want, &got) {
                    (Ok(()), Ok(())) => {
                        if let Op::SetTicket(_, _, _, host, _) = &op
                            && host.len() == MAX_KEY
                            && model.hosts.iter().any(|(h, _)| h == host)
                        {
                            known_at_limit += 1;
                        }
                        model = next;
                        applied += 1;
                    }
                    (Err(k), Err(e)) => {
                        assert!(e.is_kind(*k), "{ctx}: {e}");
                        refused += 1;
                    }
                    _ => panic!("{ctx}: model {want:?}, store {got:?}"),
                },
                (_, Err(Abort::Store(e))) => panic!("{ctx}: store: {e}"),
                (_, o) => panic!("{ctx}: {o:?}"),
            }
            let mut tx = store.read().expect("read");
            check(&model, &pools, &mut tx, ctx);
            drop(tx);
            if step % 20 == 19 {
                let before_rows = dump_read(&store.read().expect("read"));
                let before_export = export_text(&store, &chain);
                store.close().expect("close");
                store = Store::open(&path).expect("open");
                reopened += 1;
                let tx = store.read().expect("read");
                assert_eq!(dump_read(&tx), before_rows, "{ctx}: rows after a reopen");
                drop(tx);
                assert_eq!(
                    export_text(&store, &chain),
                    before_export,
                    "{ctx}: export after a reopen"
                );
                let mut tx = store.read().expect("read");
                check(&model, &pools, &mut tx, ctx);
            }
        }
        let text = export_text(&store, &chain);
        purse_export_check::parse(&text).expect("the export parses");
        store.close().expect("close");
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!(
        "{cases} sequences: {applied} applied, {refused} refused, {abandoned} abandoned, \
         {reopened} reopens, {known_at_limit} known hosts at the key limit set again"
    );
    assert!(
        refused > cases && abandoned > cases && applied > cases * 40,
        "too few of each outcome"
    );
    assert!(
        cases < 10 || known_at_limit > 0,
        "no known host at the key limit was set again"
    );
}
