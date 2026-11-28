// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests of the creation entry point, [`purse_udb::initialize`]
//! and [`purse_udb::initialize_watch_only`] (Go's `udb.Initialize` and
//! `InitializeWatchOnly`), over random networks, seeds, keys, passphrases
//! and failures (`PURSE_TEST_SEED` moves them, `PURSE_UDB_CASES` sets
//! their number).
//!
//! - A creation that fails commits nothing: every row of every table is
//!   as it was, whether the failure is an existing wallet (refused with
//!   Go's bucket error before anything is drawn), an empty private
//!   passphrase, a seed of the wrong length, an extended key that does not
//!   parse, is another network's or is private (QK-0762), a random source
//!   that fails at a randomly chosen byte, or a scrypt cost over the
//!   limits; and a store a creation failed on takes a creation afterwards,
//!   as Go's database does.
//! - A random source failing at each of the creation's requests to it in
//!   turn (on simnet, wallets of both kinds), at a random byte within the
//!   request, commits nothing, and the store then takes a creation: every
//!   point at which the source can fail, since a request fails whole.
//! - A commit that fails, at every storage operation of the creation's
//!   transaction in turn, leaves on reopen either the store as it was or
//!   the whole wallet, row for row the wallet the same creation writes
//!   without a failure, and never anything between; a creation that
//!   returned success left the whole wallet. Every such error renders as
//!   the store's own error converted (`to_purse_error`) and holds it as a
//!   [`CreationStoreError`]; one that left the whole wallet must say the
//!   commit's outcome is unknown ([`purse_udb::creation_outcome_unknown`]).
//!   Every storage operation of the creation falls in its commit, so every
//!   such error says so, including those that left nothing; an error that
//!   does not say so after a whole wallet was kept fails the test. No error
//!   of the other failures says so.
//!
//! Every wallet a creation leaves, whole, is held to the store's
//! consistency checks as `txstore::verify_backup` runs them (the
//! operation-script runner's check, ADR-0018 BK4).
//!
//! On mainnet, testnet3 and regnet, whose scrypt cost is 2^18 x 8, only
//! the failures that come before a key derivation are drawn (the random
//! source failing at its first byte, a cost over the limits, the refusals
//! of arguments and of an existing wallet); simnet's cost is 2 and takes
//! every failure. Not covered: a store error inside the transaction other
//! than at the commit, which the store gives no way to inject.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

use std::ops::Bound;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use purse_errors::{AnyError, Kind};
use purse_snacl::{Limits, RandomSourceError};
use purse_store::{BackendError, OpenOptions, RawEntry, StorageBackend, Store, TableId};
use purse_testutil::SplitMix64;
use purse_udb::addrmgr::{BoxedRandom, ChainParams, hd_keys_from_seed};
use purse_udb::txstore::TxStore;
use purse_udb::{CreationStoreError, Manager};
use redb::backends::InMemoryBackend;

const NETWORKS: [&str; 4] = ["mainnet", "testnet3", "simnet", "regnet"];

fn cases() -> usize {
    std::env::var("PURSE_UDB_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
}

fn memory_store() -> Store {
    OpenOptions::new()
        .create_on_backend(InMemoryBackend::new(), |_| Ok(()))
        .expect("in-memory store")
}

/// Every row of every table, raw.
fn dump(store: &Store) -> Vec<Vec<RawEntry>> {
    let tx = store.read().expect("read");
    TableId::ALL
        .iter()
        .map(|t| {
            tx.range_raw(*t, Bound::Unbounded, Bound::Unbounded)
                .expect("range")
                .collect::<purse_store::Result<Vec<_>>>()
                .expect("rows")
        })
        .collect()
}

/// A random source keyed by `key` that fails every request reaching past
/// `budget` bytes, counting what it hands out in `drawn`.
fn source(key: u64, budget: u64, drawn: &Arc<AtomicU64>) -> BoxedRandom {
    let mut r = SplitMix64(key);
    let drawn = Arc::clone(drawn);
    Box::new(move |buf: &mut [u8]| -> Result<(), RandomSourceError> {
        let n = u64::try_from(buf.len()).expect("small");
        let had = drawn.load(Ordering::SeqCst);
        if had + n > budget {
            return Err(RandomSourceError::new("injected failure"));
        }
        r.fill(buf);
        drawn.store(had + n, Ordering::SeqCst);
        Ok(())
    })
}

/// What a creation is asked to make.
#[derive(Clone, Debug)]
enum Make {
    Seed {
        seed: Vec<u8>,
        pub_pass: Vec<u8>,
        priv_pass: Vec<u8>,
    },
    Xpub {
        key: Vec<u8>,
        pub_pass: Vec<u8>,
    },
}

fn run(
    store: &Store,
    chain: &ChainParams,
    make: &Make,
    rng: &mut BoxedRandom,
    limits: &Limits,
) -> Result<(), AnyError> {
    match make {
        Make::Seed {
            seed,
            pub_pass,
            priv_pass,
        } => purse_udb::initialize(store, chain, seed, pub_pass, priv_pass, &mut **rng, limits),
        Make::Xpub { key, pub_pass } => {
            purse_udb::initialize_watch_only(store, chain, key, pub_pass, &mut **rng, limits)
        }
    }
}

fn pass(rng: &mut SplitMix64) -> Vec<u8> {
    let n = usize::try_from(rng.below(12)).expect("small");
    let mut p = rng.bytes(n);
    p.push(b'!');
    p
}

/// A usable seed of 16 to 64 bytes and its keys on `chain`.
fn usable_seed(rng: &mut SplitMix64, chain: &ChainParams) -> Vec<u8> {
    loop {
        let len = 16 + usize::try_from(rng.below(49)).expect("small");
        let seed = rng.bytes(len);
        if hd_keys_from_seed(&seed, chain).is_ok() {
            return seed;
        }
    }
}

/// A valid creation on `chain`: from a seed, or from account 0's extended
/// public key under either coin type.
fn valid(rng: &mut SplitMix64, chain: &ChainParams) -> Make {
    let seed = usable_seed(rng, chain);
    if rng.below(2) == 0 {
        Make::Seed {
            seed,
            pub_pass: pass(rng),
            priv_pass: pass(rng),
        }
    } else {
        let keys = hd_keys_from_seed(&seed, chain).expect("usable");
        let acct = if rng.below(2) == 0 {
            &keys.account0_legacy
        } else {
            &keys.account0_slip0044
        };
        Make::Xpub {
            key: acct.neuter().string().as_bytes().to_vec(),
            pub_pass: pass(rng),
        }
    }
}

/// The bytes a successful creation of `make` draws on simnet.
fn draws(make: &Make) -> u64 {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let drawn = Arc::new(AtomicU64::new(0));
    let mut src = source(1, u64::MAX, &drawn);
    run(&memory_store(), &chain, make, &mut src, &Limits::DEFAULT).expect("create");
    drawn.load(Ordering::SeqCst)
}

/// The store's consistency checks as `txstore::verify_backup` runs them
/// (every row decoded, the chain's, the record tables' and the transaction
/// tables' with their decoded part, ADR-0018 BK4): the operation-script
/// runner's check (`tests/support/runner.rs`, `consistent`).
fn consistent(tx: &purse_store::ReadTx) -> purse_store::Result<()> {
    tx.check_rows()?;
    tx.check_chain()?;
    tx.check_records()?;
    tx.check_tx_tables()?;
    let mut r = tx;
    purse_udb::txstore::check_decoded(&mut r)
}

/// The wallet `make` describes is in `store`, whole: the store passes the
/// consistency checks ([`consistent`]), the address manager opens with the
/// public passphrase, watching-only as made, and the transaction store's
/// tip is the genesis block.
fn assert_whole(store: &Store, chain: &ChainParams, make: &Make, ctx: &str) {
    let (pub_pass, watching) = match make {
        Make::Seed { pub_pass, .. } => (pub_pass, false),
        Make::Xpub { pub_pass, .. } => (pub_pass, true),
    };
    let tx = store.read().expect("read");
    consistent(&tx)
        .unwrap_or_else(|e| panic!("{ctx}: the store fails the consistency checks: {e}"));
    let m = Manager::open(
        &mut &tx,
        chain,
        pub_pass,
        source(9, u64::MAX, &Arc::default()),
        Limits::DEFAULT,
    )
    .unwrap_or_else(|e| panic!("{ctx}: open: {e}"));
    assert_eq!(m.watching_only(), watching, "{ctx}");
    let ts = TxStore::new(chain).expect("txstore");
    let (hash, height) = ts.main_chain_tip(&mut &tx).expect("tip");
    assert_eq!(height, 0, "{ctx}");
    assert_eq!(&hash, ts.genesis_hash(), "{ctx}");
}

#[test]
fn a_failing_initialize_commits_nothing() {
    let mut rng = SplitMix64::from_env("purse-udb-initialize-fails");
    let mut seen = [0usize; 8];
    for case in 0..cases() * 4 {
        let net = NETWORKS[usize::try_from(rng.below(4)).expect("small")];
        let chain = ChainParams::by_name(net).expect("network");
        let costly = net != "simnet";
        let store = memory_store();
        let mut make = valid(&mut rng, &chain);
        let mut limits = Limits::DEFAULT;
        let mut budget = u64::MAX;
        // The failure, and the error it must give (None: any error).
        let mode = rng.below(8);
        let want: Option<(&str, Option<Kind>)> = match mode {
            // An existing wallet of either kind, or a transaction store
            // alone, made on simnet whatever the network of the creation.
            0..=2 => {
                let sim = ChainParams::by_name("simnet").expect("simnet");
                match mode {
                    0 | 1 => {
                        let prior = loop {
                            let m = valid(&mut rng, &sim);
                            if matches!(m, Make::Seed { .. }) == (mode == 0) {
                                break m;
                            }
                        };
                        let mut src = source(rng.next_u64(), u64::MAX, &Arc::default());
                        run(&store, &sim, &prior, &mut src, &Limits::DEFAULT).expect("prior");
                    }
                    _ => {
                        let ts = TxStore::new(&sim).expect("txstore");
                        store
                            .update(|tx| {
                                ts.create(tx).expect("transaction store");
                                Ok::<_, purse_store::Error>(())
                            })
                            .expect("commit");
                    }
                }
                Some((
                    "I/O error:: item already exists: bucket already exists",
                    Some(Kind::Io),
                ))
            }
            3 => {
                make = Make::Seed {
                    seed: usable_seed(&mut rng, &chain),
                    pub_pass: pass(&mut rng),
                    priv_pass: Vec::new(),
                };
                Some((
                    "invalid operation: private passphrase may not be empty",
                    Some(Kind::Invalid),
                ))
            }
            4 => {
                let len = if rng.below(2) == 0 {
                    rng.below(16)
                } else {
                    65 + rng.below(16)
                };
                make = Make::Seed {
                    seed: rng.bytes(usize::try_from(len).expect("small")),
                    pub_pass: pass(&mut rng),
                    priv_pass: pass(&mut rng),
                };
                Some(("seed length must be between 128 and 512 bits", None))
            }
            5 => {
                let seed = usable_seed(&mut rng, &chain);
                let keys = hd_keys_from_seed(&seed, &chain).expect("usable");
                let other = NETWORKS
                    .into_iter()
                    .find(|n| *n != net)
                    .and_then(ChainParams::by_name)
                    .expect("another network");
                let theirs = hd_keys_from_seed(&seed, &other).expect("usable");
                let public = keys.account0_legacy.neuter().string().to_string();
                let mut bad_sum = public.clone().into_bytes();
                let last = bad_sum.last_mut().expect("non-empty");
                *last = if *last == b'2' { b'3' } else { b'2' };
                let (key, w) = match rng.below(5) {
                    0 => (
                        keys.account0_slip0044.string().as_bytes().to_vec(),
                        Some((
                            "invalid operation: extended key must be an xpub",
                            Some(Kind::Invalid),
                        )),
                    ),
                    1 => (
                        theirs.account0_legacy.neuter().string().as_bytes().to_vec(),
                        Some((
                            "the provided serialized extended key is for the wrong network",
                            None,
                        )),
                    ),
                    2 => (
                        Vec::new(),
                        Some((
                            "the provided serialized extended key length is invalid",
                            None,
                        )),
                    ),
                    3 => (bad_sum, None),
                    _ => {
                        let n = usize::try_from(rng.below(120)).expect("small");
                        (rng.bytes(n), None)
                    }
                };
                make = Make::Xpub {
                    key,
                    pub_pass: pass(&mut rng),
                };
                w
            }
            6 => {
                // The source fails at a byte of the creation's draws: one
                // chosen at random on simnet, the first elsewhere (each
                // request in turn is
                // `a_random_source_failing_at_each_request_commits_nothing`).
                budget = if costly { 0 } else { rng.below(draws(&make)) };
                None
            }
            _ => {
                let (n, r, p) = chain.scrypt_cost();
                let work = u64::try_from(n * r * p).expect("positive");
                limits.max_scrypt_work = work - 1 - rng.below(work.min(4) - 1);
                None
            }
        };
        let mode = usize::try_from(mode).expect("small");
        seen[mode] += 1;
        let before = dump(&store);
        let drawn = Arc::new(AtomicU64::new(0));
        let mut src = source(rng.next_u64(), budget, &drawn);
        let ctx = format!("case {case} {net} mode {mode} {make:?}");
        let err = run(&store, &chain, &make, &mut src, &limits).expect_err(&ctx);
        // Not a store error, so nothing can have been committed.
        assert!(
            err.find_foreign::<CreationStoreError>().is_none(),
            "{ctx}: {err}"
        );
        assert!(!purse_udb::creation_outcome_unknown(&err), "{ctx}: {err}");
        if let Some((text, kind)) = want {
            assert!(
                err.render(purse_errors::DAEMON_SEPARATOR).contains(text),
                "{ctx}: {err}"
            );
            if let Some(kind) = kind {
                assert!(err.is_kind(kind), "{ctx}: {err}");
            }
            // Refused before anything is drawn.
            assert_eq!(drawn.load(Ordering::SeqCst), 0, "{ctx}");
        }
        assert_eq!(dump(&store), before, "{ctx}: {err}");
        // The store takes a creation afterwards, unless it held a wallet
        // already (simnet only: elsewhere a creation derives two keys at
        // dcrwallet's cost).
        if mode > 2 && !costly {
            let ok = valid(&mut rng, &chain);
            let mut src = source(rng.next_u64(), u64::MAX, &Arc::default());
            run(&store, &chain, &ok, &mut src, &Limits::DEFAULT)
                .unwrap_or_else(|e| panic!("{ctx}: then {ok:?}: {e}"));
            assert_whole(&store, &chain, &ok, &ctx);
        }
    }
    eprintln!("mode -> cases: {seen:?}");
    if cases() >= 24 {
        assert!(seen.iter().all(|n| *n > 0), "{seen:?}");
    }
}

/// The sizes of the requests a successful creation of `make` makes of
/// its random source on simnet, in order.
fn requests(make: &Make) -> Vec<u64> {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let sizes = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&sizes);
    let mut r = SplitMix64(1);
    let mut src: BoxedRandom = Box::new(move |buf: &mut [u8]| -> Result<(), RandomSourceError> {
        log.lock()
            .expect("lock")
            .push(u64::try_from(buf.len()).expect("small"));
        r.fill(buf);
        Ok(())
    });
    run(&memory_store(), &chain, make, &mut src, &Limits::DEFAULT).expect("create");
    sizes.lock().expect("lock").clone()
}

#[test]
fn a_random_source_failing_at_each_request_commits_nothing() {
    let mut rng = SplitMix64::from_env("purse-udb-initialize-each-request");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let (mut seeds, mut xpubs, mut points) = (0usize, 0usize, 0usize);
    let makes = cases().max(2);
    for round in 0..makes {
        // Both kinds in turn.
        let make = loop {
            let m = valid(&mut rng, &chain);
            if matches!(m, Make::Seed { .. }) == (round % 2 == 0) {
                break m;
            }
        };
        let sizes = requests(&make);
        assert!(!sizes.is_empty(), "{make:?}: nothing drawn");
        let mut before_request = 0u64;
        for (i, size) in sizes.iter().enumerate() {
            // Fail request `i`: the budget ends at a random byte inside it.
            let budget = before_request + rng.below(*size);
            let store = memory_store();
            let empty = dump(&store);
            let drawn = Arc::new(AtomicU64::new(0));
            let mut src = source(rng.next_u64(), budget, &drawn);
            let ctx = format!(
                "round {round} request {i} of {} at {budget}: {make:?}",
                sizes.len()
            );
            let err = run(&store, &chain, &make, &mut src, &Limits::DEFAULT).expect_err(&ctx);
            // Nothing past the failing request was asked for.
            assert_eq!(drawn.load(Ordering::SeqCst), before_request, "{ctx}: {err}");
            assert_eq!(dump(&store), empty, "{ctx}: {err}");
            let mut src = source(rng.next_u64(), u64::MAX, &Arc::default());
            run(&store, &chain, &make, &mut src, &Limits::DEFAULT)
                .unwrap_or_else(|e| panic!("{ctx}: then: {e}"));
            assert_whole(&store, &chain, &make, &ctx);
            before_request += size;
            points += 1;
        }
        match make {
            Make::Seed { .. } => seeds += 1,
            Make::Xpub { .. } => xpubs += 1,
        }
    }
    eprintln!(
        "{seeds} seed and {xpubs} extended key creations, failing at each of {points} requests"
    );
    assert!(seeds > 0 && xpubs > 0);
}

/// An in-memory storage backend shared with the test, whose writes,
/// truncations and syncs fail from the `fail_at`-th on (counted from 0).
#[derive(Debug, Default)]
struct Faulty {
    data: Mutex<Vec<u8>>,
    ops: AtomicU64,
    fail_at: AtomicU64,
}

impl Faulty {
    fn admit(&self) -> Result<(), BackendError> {
        let n = self.ops.fetch_add(1, Ordering::SeqCst);
        if n >= self.fail_at.load(Ordering::SeqCst) {
            return Err(BackendError::Io(std::io::Error::other("injected failure")));
        }
        Ok(())
    }

    /// No failure from now on; the count starts again.
    fn heal(&self) {
        self.fail_at.store(u64::MAX, Ordering::SeqCst);
        self.ops.store(0, Ordering::SeqCst);
    }

    /// Fail from the `n`-th operation from now on.
    fn fail_after(&self, n: u64) {
        self.ops.store(0, Ordering::SeqCst);
        self.fail_at.store(n, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
struct Shared(Arc<Faulty>);

impl StorageBackend for Shared {
    fn len(&self) -> Result<u64, BackendError> {
        Ok(u64::try_from(self.0.data.lock().expect("lock").len()).expect("small"))
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), BackendError> {
        let data = self.0.data.lock().expect("lock");
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        match a.checked_add(out.len()).and_then(|end| data.get(a..end)) {
            Some(src) => {
                out.copy_from_slice(src);
                Ok(())
            }
            None => Err(BackendError::Io(std::io::Error::from(
                std::io::ErrorKind::UnexpectedEof,
            ))),
        }
    }

    fn set_len(&self, len: u64) -> Result<(), BackendError> {
        self.0.admit()?;
        let len = usize::try_from(len).map_err(|_| BackendError::Unsupported)?;
        self.0.data.lock().expect("lock").resize(len, 0);
        Ok(())
    }

    fn sync_data(&self) -> Result<(), BackendError> {
        self.0.admit()
    }

    fn write(&self, offset: u64, bytes: &[u8]) -> Result<(), BackendError> {
        self.0.admit()?;
        let a = usize::try_from(offset).map_err(|_| BackendError::Unsupported)?;
        let mut data = self.0.data.lock().expect("lock");
        if data.len() < a + bytes.len() {
            data.resize(a + bytes.len(), 0);
        }
        data[a..a + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn a_failing_commit_leaves_nothing_or_the_whole_wallet() {
    let mut rng = SplitMix64::from_env("purse-udb-initialize-commit-fails");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let (mut nothing, mut whole, mut ok) = (0usize, 0usize, 0usize);
    let (mut unknown_nothing, mut unknown_whole) = (0usize, 0usize);
    for round in 0..cases() {
        let make = valid(&mut rng, &chain);
        let key = rng.next_u64();
        let fresh = || {
            let faulty = Arc::new(Faulty::default());
            faulty.heal();
            let store = OpenOptions::new()
                .create_on_backend(Shared(Arc::clone(&faulty)), |_| Ok(()))
                .expect("store");
            (faulty, store)
        };
        // The reference: the same creation without a failure, and the
        // storage operations it takes.
        let (faulty, store) = fresh();
        let empty = dump(&store);
        faulty.heal();
        let mut src = source(key, u64::MAX, &Arc::default());
        run(&store, &chain, &make, &mut src, &Limits::DEFAULT).expect("reference");
        let ops = faulty.ops.load(Ordering::SeqCst);
        let reference = dump(&store);
        assert_whole(&store, &chain, &make, &format!("round {round} reference"));
        assert_ne!(reference, empty);
        assert!(ops > 0);
        drop(store);
        for at in 0..=ops {
            let ctx = format!("round {round} fail at op {at} of {ops}: {make:?}");
            let (faulty, store) = fresh();
            faulty.fail_after(at);
            let mut src = source(key, u64::MAX, &Arc::default());
            let r = run(&store, &chain, &make, &mut src, &Limits::DEFAULT);
            // Dropped while failing, as at a crash.
            drop(store);
            faulty.heal();
            let store = OpenOptions::new()
                .open_on_backend(Shared(Arc::clone(&faulty)))
                .unwrap_or_else(|e| panic!("{ctx}: reopen: {e}"));
            let rows = dump(&store);
            // Every failure here is the store's: rendered as the store's
            // own error converted, and held whole.
            let unknown = match &r {
                Ok(()) => false,
                Err(err) => {
                    let leaf = err
                        .find_foreign::<CreationStoreError>()
                        .unwrap_or_else(|| panic!("{ctx}: not a store error: {err}"));
                    assert_eq!(
                        err.render(purse_errors::DAEMON_SEPARATOR),
                        leaf.store_error()
                            .to_purse_error()
                            .render(purse_errors::DAEMON_SEPARATOR),
                        "{ctx}"
                    );
                    assert!(err.is_kind(leaf.store_error().kind()), "{ctx}: {err}");
                    assert_eq!(
                        purse_udb::creation_outcome_unknown(err),
                        leaf.commit_outcome_unknown(),
                        "{ctx}"
                    );
                    leaf.commit_outcome_unknown()
                }
            };
            if rows == empty {
                assert!(r.is_err(), "{ctx}: success, and nothing kept");
                nothing += 1;
                unknown_nothing += usize::from(unknown);
            } else {
                assert_eq!(rows, reference, "{ctx}: neither nothing nor the wallet");
                assert_whole(&store, &chain, &make, &ctx);
                whole += 1;
                ok += usize::from(r.is_ok());
                // A creation that failed and yet left the wallet says so.
                if let Err(err) = &r {
                    assert!(unknown, "{ctx}: whole wallet behind a known failure: {err}");
                    unknown_whole += 1;
                }
            }
        }
    }
    eprintln!(
        "nothing {nothing}, whole {whole} ({ok} reported as made); outcome unknown: \
         {unknown_nothing} left nothing, {unknown_whole} left the wallet"
    );
    assert!(nothing > 0 && whole > 0);
}
