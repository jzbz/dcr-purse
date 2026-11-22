// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The first clause of the Phase 1 exit (brief section 6), through the
//! address manager that ships: for random seeds and for every seed a
//! targeted search finds with a leading-zero key on dcrwallet's derivation
//! routes, the account extended public keys of both coin types and the
//! first address of both branches of each account equal the Go oracle's
//! (`tools/oracle/cmd/hdkeychain`, command `wallet`, which calls
//! dcrwallet's own `udb.HDKeysFromSeed`; its lines are described in
//! `tools/oracle/cmd/hdkeychain/wallet.go`).
//!
//! # What the Rust side runs
//!
//! For each seed, two address managers, each on its own in-memory
//! `purse-store`, as two wallets made from the seed hold them:
//!
//! - the legacy coin type, as a restored wallet keeps it:
//!   [`Manager::create`] from the seed (`createAddressManager`, which calls
//!   `HDKeysFromSeed` and stores the coin type keys and account 0),
//!   [`Manager::open`], [`Manager::unlock`], then [`Manager::new_account`]
//!   for account 1 (`Wallet.NextAccount`, from the coin type key parsed
//!   from its stored string, padded back to 32 bytes);
//! - the SLIP0044 coin type, as a new wallet moves to it: the same, with
//!   [`Manager::upgrade_to_slip0044_coin_type`] between unlocking and the
//!   new account.
//!
//! Each manager's lines are read through its public operations only:
//! [`Manager::coin_type`] for the coin type, [`Manager::coin_type_priv_key`]
//! neutered for the `C|` line (the neutered key's string does not depend on
//! whether the private key was stored stripped), and
//! [`Manager::account_extended_pub_key`] for each account's xpub (account
//! 0's as stored at creation, from the coin type key as `Child` left it).
//! The first addresses take four operations, because the address a
//! listing returns is not the derivation that recorded it, and dcrwallet
//! hands addresses out from a fourth:
//!
//! - while the manager is unlocked, [`Manager::sync_account_to_addr_index`]
//!   records child 0 of both branches. It derives each publicly from the
//!   account xpub parsed from the store and stores only the path (account,
//!   branch, index) under the HASH160 of that derivation: the hash index by
//!   which a wallet recognises its addresses.
//! - [`Manager::for_each_account_address`] lists them, building each address
//!   by deriving its path again: privately while the manager is unlocked,
//!   publicly from the account xpub once it is locked
//!   (`chainAddressRowToManaged`). The manager is listed once each way, and
//!   the strings compared with Go are the locked, public listing's, which
//!   derive as Go's `deriveBranches` and `deriveChildAddress` do; the
//!   unlocked, private listing must equal it.
//! - [`Manager::address`] looks each compared address up by its HASH160 and
//!   must find the row of that same path (account, branch, child 0). This
//!   is what checks the derivation `SyncAccountToAddrIndex` keyed its rows
//!   by, which no listing reads back.
//! - [`Manager::account_branch_extended_pub_key`] gives each branch's key,
//!   from which dcrwallet's address buffers derive account 0's addresses
//!   after the coin type upgrade (`wallet/cointype.go`) and during
//!   discovery (`wallet/discovery.go`). It must serialize as the compared
//!   account xpub's child of that branch, and its child 0's P2PKH address
//!   must be the compared address.
//!
//! A seed `HDKeysFromSeed` refuses fails in [`Manager::create`]; its
//! error, rendered with the daemon's separator, is the seed's one `E|`
//! line.
//!
//! # Test-only parameters
//!
//! Three choices make a wallet per seed cheap enough for 10^6 seeds; none
//! reaches derivation:
//!
//! - **The scrypt cost.** dcrwallet seals master keys with scrypt at
//!   `N = 2^18, r = 8` everywhere but simnet (`scryptOptionsForNet`), four
//!   derivations of 256 MiB per wallet here (two at creation, one at open,
//!   one at unlock). The managers are given the network's chain parameters
//!   with `simnet` set ([`WalletNet::cheap`]), which `purse-udb` reads only
//!   to choose simnet's cost, `N = 2, r = 1, p = 1`
//!   ([`ChainParams::scrypt_cost`]); the name, extended key versions, coin
//!   types and address prefixes are the network's own, and the test checks
//!   that `simnet` is the only field changed. `dcrwallet_scrypt_costs_give_the_same_lines`
//!   runs seeds on mainnet, testnet3 and regnet with the unmodified
//!   parameters, at dcrwallet's real cost, and holds their lines equal to
//!   Go's and to the cheap path's.
//! - **The store.** Each manager's store is in memory (redb's
//!   `InMemoryBackend` under `purse-store`), not a file: nothing is written
//!   to disk and no commit waits for a sync.
//! - **Randomness.** Salts, crypto keys and nonces come from a fixed
//!   SplitMix64 stream instead of the operating system; they are sealed
//!   values only.
//!
//! Where the time per seed goes is printed by the ignored
//! `cost_per_seed`.
//!
//! # Runs
//!
//! The default sizes run in the normal test suite. The full run, 10^6
//! random seeds plus a search over 4,000,000 seeds, is ignored; run it with
//!
//! ```text
//! PURSE_TEST_SEED=0x50687365310001 PURSE_REQUIRE_ORACLE=1 cargo test --release \
//!     -p purse-udb --test wallet_diff -- --ignored --nocapture million_seeds
//! ```
//!
//! Environment:
//! - `PURSE_HD_WALLET_SEEDS`: random seeds (default 4000; 1,000,000 in the
//!   ignored test).
//! - `PURSE_HD_WALLET_LZ_PER_WORKER`: seeds each of the 16 search streams
//!   screens (default 25,000; 250,000 in the ignored test).
//! - `PURSE_TEST_SEED`: the run seed, mixed with each test's label (printed
//!   by each test); unset, every run uses the same inputs. The labels are
//!   those the tests had in `purse-hdkeychain`, so the same seeds are drawn.
//! - `PURSE_REQUIRE_ORACLE`: fail instead of skipping without Go.
//!
//! The frozen file `tests/data/wallet_lines.txt` holds Go's full lines for
//! a fixed set of seeds and is replayed without Go; regenerate it with
//!
//! ```text
//! PURSE_HD_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release \
//!     -p purse-udb --test wallet_diff -- --ignored regenerate_wallet_vectors
//! ```
//!
//! Until 2026-11-22 these tests lived in `purse-hdkeychain` and composed
//! `purse-hdkeychain` and `stdaddr` in dcrwallet's steps themselves,
//! written before the address manager existed (`docs/PHASE1-EXIT.md`,
//! section 1).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dcroxide_txscript::stdaddr::{self, Address};
use purse_errors::{AnyError, DAEMON_SEPARATOR};
use purse_hdkeychain::{ExtendedKey, HdVersions};
use purse_snacl::{Limits, RandomSourceError};
use purse_store::{OpenOptions, Store, WriteTx};
use purse_testutil::serde_json::{self, json};
use purse_testutil::{Oracle, SplitMix64, hex, oracle_or_skip, unhex};
use purse_udb::Manager;
use purse_udb::addrmgr::{BoxedRandom, ChainParams, EXTERNAL_BRANCH, INTERNAL_BRANCH};
use redb::backends::InMemoryBackend;
use sha2::{Digest, Sha256};

/// Seeds per oracle request in digest mode.
const BATCH: usize = 2048;

/// Search streams.
const WORKERS: u64 = 16;

/// The wallets' passphrases (any non-empty private passphrase serves).
const PUB_PASS: &[u8] = b"public";
const PRIV_PASS: &[u8] = b"private";

/// The name given to account 1.
const ACCOUNT_1_NAME: &[u8] = b"account-1";

/// The accounts compared for each coin type: 0 as wallet creation derives
/// it, 1 as `udb.Manager.NewAccount` does.
const WALLET_ACCOUNTS: [u32; 2] = [0, 1];

/// Hardened index `i'`.
const fn h(i: u32) -> u32 {
    i | 0x8000_0000
}

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().replace('_', "").parse().ok())
        .unwrap_or(default)
}

/// Runs `f` over `items` on all cores, keeping the order.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker panicked"))
            .collect()
    })
}

/// A random seed: usually 32 bytes, one time in eight any allowed length.
fn random_seed(rng: &mut SplitMix64) -> Vec<u8> {
    let len = if rng.below(8) == 0 {
        16 + rng.below(49) as usize
    } else {
        32
    };
    rng.bytes(len)
}

fn leading_zeros(b: &[u8]) -> usize {
    b.iter().take_while(|&&x| x == 0).count()
}

// ---------------------------------------------------------------------------
// Networks and wallets.
// ---------------------------------------------------------------------------

/// A network as the address manager sees it.
struct WalletNet {
    /// Its name for the oracle.
    name: &'static str,
    /// Its chain parameters, unmodified: dcrwallet's scrypt cost.
    real: ChainParams,
    /// The same with `simnet` set: simnet's scrypt cost, nothing else
    /// changed (the module documentation, "Test-only parameters").
    cheap: ChainParams,
}

/// The four Decred networks.
fn wallet_nets() -> Vec<WalletNet> {
    ["mainnet", "testnet3", "simnet", "regnet"]
        .into_iter()
        .map(|name| {
            let real = ChainParams::by_name(name).expect("network");
            let cheap = ChainParams {
                simnet: true,
                ..real
            };
            WalletNet { name, real, cheap }
        })
        .collect()
}

/// Every network's cheap parameters differ from its own in `simnet` alone,
/// and select simnet's scrypt cost.
#[test]
fn cheap_parameters_change_only_the_scrypt_cost() {
    for net in wallet_nets() {
        assert_eq!(
            ChainParams {
                simnet: net.real.simnet,
                ..net.cheap
            },
            net.real,
            "{}",
            net.name
        );
        assert_eq!(net.cheap.scrypt_cost(), (2, 1, 1), "{}", net.name);
        assert_eq!(net.real.name, net.name);
    }
}

/// A fixed SplitMix64 stream as a random source: salts, keys and nonces.
fn random_source(state: u64) -> impl FnMut(&mut [u8]) -> Result<(), RandomSourceError> {
    let mut r = SplitMix64(state);
    move |buf: &mut [u8]| {
        r.fill(buf);
        Ok(())
    }
}

/// The error that rolls a write transaction back.
struct Rollback(AnyError);

impl From<purse_store::Error> for Rollback {
    fn from(e: purse_store::Error) -> Self {
        Rollback(e.to_purse_error().into())
    }
}

/// Runs `f` in a write transaction, committed on success.
fn update<T>(
    store: &Store,
    f: impl FnOnce(&mut WriteTx<'_>) -> Result<T, AnyError>,
) -> Result<T, AnyError> {
    store
        .update(|tx| f(tx).map_err(Rollback))
        .map_err(|Rollback(e)| e)
}

/// An empty store in memory.
fn memory_store() -> Store {
    OpenOptions::new()
        .create_on_backend(InMemoryBackend::new(), |_| Ok(()))
        .expect("in-memory store")
}

/// A wallet's store, its open, unlocked address manager and the chain
/// parameters the manager was made with.
struct Wallet {
    store: Store,
    mgr: Manager,
    chain: ChainParams,
}

/// Creates the address manager of `seed` in a new store, opens it and
/// unlocks it. `Err` is [`Manager::create`]'s error (a seed
/// `HDKeysFromSeed` refuses); every later step must succeed.
fn create_wallet(seed: &[u8], chain: &ChainParams) -> Result<Wallet, AnyError> {
    let store = memory_store();
    let mut rng = random_source(0x7365_6564_2d64_6966);
    update(&store, |tx| {
        Manager::create(
            tx,
            chain,
            seed,
            PUB_PASS,
            PRIV_PASS,
            &mut rng,
            &Limits::DEFAULT,
        )
    })?;
    let tx = store.read().expect("read");
    let open_rng: BoxedRandom = Box::new(random_source(0x6f70_656e_2d64_6966));
    let mgr = Manager::open(&mut &tx, chain, PUB_PASS, open_rng, Limits::DEFAULT).expect("open");
    drop(tx);
    mgr.unlock(PRIV_PASS).expect("unlock");
    Ok(Wallet {
        store,
        mgr,
        chain: *chain,
    })
}

/// Child 0 of an account's external and internal branch, in that order.
type FirstAddresses = [Address; 2];

/// Records child 0 of both branches of `account` with
/// `SyncAccountToAddrIndex`, which derives each publicly from the account
/// xpub parsed from the store and keys its row (account, branch, index) by
/// the HASH160 of that derivation.
fn sync_first_addresses(w: &Wallet, account: u32) -> Result<(), AnyError> {
    update(&w.store, |tx| {
        w.mgr
            .sync_account_to_addr_index(tx, account, 0, EXTERNAL_BRANCH)?;
        w.mgr
            .sync_account_to_addr_index(tx, account, 0, INTERNAL_BRANCH)
    })
}

/// Lists `account`'s addresses with `ForEachAccountAddress`, which builds
/// each from its row's path by deriving it again: privately from the
/// account's private key while the manager is unlocked, publicly from the
/// account xpub while it is locked (`chainAddressRowToManaged`). Holds the
/// listing to exactly the two rows [`sync_first_addresses`] recorded.
fn list_first_addresses(w: &Wallet, account: u32) -> Result<FirstAddresses, AnyError> {
    let tx = w.store.read().expect("read");
    let mut first: [Option<Address>; 2] = [None, None];
    w.mgr.for_each_account_address(&mut &tx, account, |m| {
        assert_eq!(m.account(), account, "address of another account");
        assert!(!m.imported(), "imported address in a derived account");
        assert_eq!(
            m.index(),
            Some(0),
            "account {account}: a child other than 0"
        );
        let slot = &mut first[usize::from(m.internal())];
        assert!(slot.is_none(), "account {account}: two rows for one branch");
        *slot = Some(m.address().clone());
        Ok(())
    })?;
    let [Some(ext), Some(int)] = first else {
        panic!("account {account}: child 0 of a branch not listed");
    };
    Ok([ext, int])
}

/// Child 0 of both branches of `account`, recorded and then listed while
/// the manager is unlocked, so derived privately when listed.
fn unlocked_first_addresses(w: &Wallet, account: u32) -> Result<FirstAddresses, AnyError> {
    assert!(!w.mgr.is_locked(), "the manager must be unlocked");
    sync_first_addresses(w, account)?;
    list_first_addresses(w, account)
}

/// The `A|` line of an account (or its `E|` line), read from the manager
/// once it is locked: its extended public key ([`Manager::account_extended_pub_key`])
/// and child 0 of both branches, listed again and so now derived publicly
/// from the account xpub, as Go's `deriveBranches` and `deriveChildAddress`
/// derive them.
///
/// Three checks tie what is compared to what the manager recorded and
/// hands out:
/// - the public listing equals `unlocked`, the private listing of the same
///   rows, so the account's public and private derivations agree;
/// - each address, looked up by its HASH160 with [`Manager::address`] (the
///   hash index a wallet recognises its addresses by), finds the row of
///   that same path: the account, the branch and child 0. The rows
///   `SyncAccountToAddrIndex` wrote are keyed by its own derivation, which
///   no listing reads back, so this is what checks that derivation.
/// - each branch's key from [`Manager::account_branch_extended_pub_key`]
///   (from which dcrwallet hands out account 0's addresses after the coin
///   type upgrade, `wallet/cointype.go`, and during discovery,
///   `wallet/discovery.go`) serializes as the compared account xpub's child
///   of that branch, and its child 0's P2PKH address is the compared
///   address.
fn account_line(w: &Wallet, coin_type: u32, account: u32, unlocked: &FirstAddresses) -> String {
    let derive = || -> Result<String, AnyError> {
        assert!(w.mgr.is_locked(), "the manager must be locked");
        let public = list_first_addresses(w, account)?;
        for (branch, (p, u)) in public.iter().zip(unlocked).enumerate() {
            assert!(
                p == u,
                "account {account} branch {branch}: child 0 derived publicly ({}) and privately \
                 ({}) differ",
                p.encode(),
                u.encode()
            );
        }
        let tx = w.store.read().expect("read");
        for (internal, addr) in [false, true].into_iter().zip(&public) {
            let m = w.mgr.address(&mut &tx, addr)?;
            assert_eq!(
                (m.account(), m.internal(), m.index(), m.imported()),
                (account, internal, Some(0), false),
                "the row recorded under the HASH160 of {} (account {account}, internal {internal}, \
                 child 0) is another path's",
                addr.encode()
            );
            assert!(
                m.address() == addr,
                "the row under {} rebuilds {}",
                addr.encode(),
                m.address().encode()
            );
        }
        let xpub = w.mgr.account_extended_pub_key(&mut &tx, account)?;
        for (branch, addr) in [EXTERNAL_BRANCH, INTERNAL_BRANCH].into_iter().zip(&public) {
            let key = w
                .mgr
                .account_branch_extended_pub_key(&mut &tx, account, branch)?;
            let want = xpub
                .child(branch)
                .expect("branch key: child 0 listed above");
            assert!(
                *key.string() == *want.string(),
                "account {account} branch {branch}: AccountBranchExtendedPubKey gave {}, not \
                 the account xpub's child {branch}, {}",
                *key.string(),
                *want.string()
            );
            let child = key.child(0).expect("child 0: listed above");
            let from_branch = Address::PubKeyHashEcdsaSecp256k1V0 {
                net_id: w.chain.ids.pub_key_hash_ecdsa,
                hash: stdaddr::hash160(child.serialized_pub_key()),
            };
            assert!(
                from_branch == *addr,
                "account {account} branch {branch}: child 0 of AccountBranchExtendedPubKey is \
                 {}, not {}",
                from_branch.encode(),
                addr.encode()
            );
        }
        let [ext, int] = public.map(|a| a.encode());
        Ok(format!(
            "A|{coin_type}|{account}|{}|{ext}|{int}",
            *xpub.string()
        ))
    };
    derive()
        .unwrap_or_else(|err| format!("E|{coin_type}|{account}|{}", err.render(DAEMON_SEPARATOR)))
}

/// The `E|` line of an account.
fn account_error(coin_type: u32, account: u32, err: &AnyError) -> String {
    format!("E|{coin_type}|{account}|{}", err.render(DAEMON_SEPARATOR))
}

/// The account lines of a manager given account 1 (`new`, the result of
/// [`Manager::new_account`]): child 0 of both branches of each account
/// recorded and listed while unlocked, then the manager locked and each
/// account's line read ([`account_line`]).
fn account_lines(w: &Wallet, coin_type: u32, new: &Result<u32, AnyError>, lines: &mut Vec<String>) {
    let mut unlocked = Vec::with_capacity(WALLET_ACCOUNTS.len());
    for account in WALLET_ACCOUNTS {
        if account != 0 {
            match new {
                Ok(n) => assert_eq!(*n, account, "new account number"),
                Err(err) => {
                    unlocked.push(Err(account_error(coin_type, account, err)));
                    continue;
                }
            }
        }
        unlocked.push(
            unlocked_first_addresses(w, account)
                .map_err(|err| account_error(coin_type, account, &err)),
        );
    }
    w.mgr.lock().expect("lock");
    for (account, first) in WALLET_ACCOUNTS.into_iter().zip(unlocked) {
        lines.push(match first {
            Ok(first) => account_line(w, coin_type, account, &first),
            Err(line) => line,
        });
    }
}

/// The lines of one coin type: a manager created from the seed, moved to
/// the SLIP0044 coin type when `slip0044` is set, given account 1, then
/// locked ([`account_lines`]).
fn coin_type_lines(
    seed: &[u8],
    chain: &ChainParams,
    slip0044: bool,
) -> Result<Vec<String>, AnyError> {
    let w = create_wallet(seed, chain)?;
    if slip0044 {
        update(&w.store, |tx| w.mgr.upgrade_to_slip0044_coin_type(tx))
            .expect("upgrade to the SLIP0044 coin type");
    }
    let tx = w.store.read().expect("read");
    let coin_type = w.mgr.coin_type(&mut &tx).expect("coin type");
    let want = if slip0044 {
        chain.slip0044_coin_type
    } else {
        chain.legacy_coin_type
    };
    assert_eq!(coin_type, want, "active coin type");
    let ct = w
        .mgr
        .coin_type_priv_key(&mut &tx)
        .expect("coin type private key");
    drop(tx);
    let mut lines = Vec::with_capacity(3);
    lines.push(format!("C|{coin_type}|{}", *ct.neuter().string()));
    drop(ct);
    let new = update(&w.store, |tx| w.mgr.new_account(tx, ACCOUNT_1_NAME));
    account_lines(&w, coin_type, &new, &mut lines);
    Ok(lines)
}

/// The lines of one seed through managers made with `chain`: the legacy
/// then the SLIP0044 coin type, or the single `E|` line of a seed the
/// managers refuse.
fn wallet_lines_with(seed: &[u8], chain: &ChainParams) -> Vec<String> {
    let mut lines = Vec::with_capacity(6);
    for slip0044 in [false, true] {
        match coin_type_lines(seed, chain, slip0044) {
            Ok(l) => lines.extend(l),
            Err(err) => {
                assert!(!slip0044, "only the SLIP0044 manager refused the seed");
                return vec![format!("E|{}", err.render(DAEMON_SEPARATOR))];
            }
        }
    }
    lines
}

/// The lines of one seed, at the cheap scrypt cost.
fn wallet_lines(seed: &[u8], net: &WalletNet) -> Vec<String> {
    wallet_lines_with(seed, &net.cheap)
}

/// The digest of one seed's lines, as the oracle computes it.
fn wallet_digest(lines: &[String]) -> String {
    let mut hasher = Sha256::new();
    for l in lines {
        hasher.update(l.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

// ---------------------------------------------------------------------------
// Comparing with Go.
// ---------------------------------------------------------------------------

/// What a wallet comparison matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WalletCompared {
    /// Seeds whose answers were compared and equal.
    seeds: usize,
    /// Account lines (`A|`) among them: account xpubs, each with the first
    /// address of both branches.
    accounts: usize,
    /// Error lines among them.
    errors: usize,
}

impl WalletCompared {
    fn add(&mut self, lines: &[String]) {
        self.seeds += 1;
        self.accounts += lines.iter().filter(|l| l.starts_with("A|")).count();
        self.errors += lines.iter().filter(|l| l.starts_with("E|")).count();
    }

    fn merge(&mut self, other: WalletCompared) {
        self.seeds += other.seeds;
        self.accounts += other.accounts;
        self.errors += other.errors;
    }
}

fn go_wallet(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
    digest: bool,
) -> serde_json::Value {
    let seeds: Vec<String> = seeds.iter().map(|s| hex(s)).collect();
    oracle.call(
        "wallet",
        json!({ "net": net.name, "digest": digest, "seeds": seeds }),
    )
}

/// Go's lines for each seed.
fn go_wallet_lines(oracle: &mut Oracle, net: &WalletNet, seeds: &[Vec<u8>]) -> Vec<Vec<String>> {
    serde_json::from_value(go_wallet(oracle, net, seeds, false)).expect("wallet lines")
}

fn first_difference(go: &[String], rust: &[String], seed: &[u8]) -> String {
    for i in 0..go.len().max(rust.len()) {
        let g = go.get(i).map_or("<none>", String::as_str);
        let r = rust.get(i).map_or("<none>", String::as_str);
        if g != r {
            return format!("seed {} line {i}:\n  go:   {g}\n  rust: {r}", hex(seed));
        }
    }
    "no difference".to_owned()
}

/// Compares seeds with Go line by line; panics with the first difference.
fn compare_wallet_lines(oracle: &mut Oracle, net: &WalletNet, seeds: &[Vec<u8>]) -> WalletCompared {
    let go = go_wallet_lines(oracle, net, seeds);
    assert_eq!(
        go.len(),
        seeds.len(),
        "Go answered a different number of seeds"
    );
    let rust = par_map(seeds, |seed| wallet_lines(seed, net));
    let mut compared = WalletCompared::default();
    for ((g, r), seed) in go.iter().zip(&rust).zip(seeds) {
        assert!(
            g == r,
            "mismatch on {}: {}",
            net.name,
            first_difference(g, r, seed)
        );
        compared.add(r);
    }
    compared
}

/// Compares seeds with Go by digest, in batches, Go and Rust working on a
/// batch at once; re-asks Go for full lines to report a mismatch.
fn compare_wallet_digests(
    oracle: &mut Oracle,
    net: &WalletNet,
    seeds: &[Vec<u8>],
    batch: usize,
) -> WalletCompared {
    let mut compared = WalletCompared::default();
    for chunk in seeds.chunks(batch.max(1)) {
        let (go, rust) = std::thread::scope(|s| {
            let go = s.spawn(|| go_wallet(oracle, net, chunk, true));
            let rust = par_map(chunk, |seed| {
                let lines = wallet_lines(seed, net);
                (wallet_digest(&lines), lines)
            });
            (go.join().expect("oracle thread"), rust)
        });
        let go: Vec<String> = serde_json::from_value(go).expect("wallet digests");
        assert_eq!(
            go.len(),
            chunk.len(),
            "Go answered a different number of seeds"
        );
        for ((g, (r, lines)), seed) in go.iter().zip(&rust).zip(chunk) {
            if g != r {
                let gl = go_wallet_lines(oracle, net, std::slice::from_ref(seed));
                panic!(
                    "digest mismatch on {}: {}",
                    net.name,
                    first_difference(&gl[0], lines, seed)
                );
            }
            compared.add(lines);
        }
    }
    compared
}

// ---------------------------------------------------------------------------
// Leading-zero search along dcrwallet's own routes.
// ---------------------------------------------------------------------------

mod filter {
    use dcroxide_dcrec::secp256k1;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha512;

    fn hmac512(key: &[u8], data: &[u8]) -> [u8; 64] {
        let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key).expect("any key length");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    /// (32-byte key, chain code) of the master, or None if unusable.
    pub fn master(seed: &[u8]) -> Option<([u8; 32], [u8; 32])> {
        let lr = hmac512(b"Bitcoin seed", seed);
        let (k, c) = lr.split_at(32);
        let k: [u8; 32] = k.try_into().ok()?;
        secp256k1::check_scalar(&k).ok()?;
        Some((k, c.try_into().ok()?))
    }

    /// Hardened child of a parent stored as `stored` (Decred's preimage:
    /// the stored bytes left-aligned after the 0x00 byte): (32-byte child
    /// key, chain code).
    pub fn hardened(stored: &[u8], cc: &[u8; 32], i: u32) -> Option<([u8; 32], [u8; 32])> {
        let mut data = [0u8; 37];
        data[1..1 + stored.len()].copy_from_slice(stored);
        data[33..].copy_from_slice(&i.to_be_bytes());
        let ilr = hmac512(cc, &data);
        let (il, ir) = ilr.split_at(32);
        let il: [u8; 32] = il.try_into().ok()?;
        let sum = *secp256k1::add_private_tweak(&il, stored).ok()?.serialize();
        Some((sum, ir.try_into().ok()?))
    }
}

fn stripped(b: &[u8; 32]) -> &[u8] {
    &b[leading_zeros(b)..]
}

/// The most leading zero bytes of a private key at each hardened depth
/// (master, purpose, coin type, account) on the routes dcrwallet takes for
/// mainnet's coin types 20 and 42: purpose and coin type keys by `Child`
/// (stored stripped), account 0 from the stripped coin type key, account 1
/// from the coin type key padded by a re-parse. A fast filter (HMAC-SHA512
/// and a scalar addition per step, no point multiplication); it selects
/// candidates only, and every candidate is compared with Go.
fn screen_wallet(seed: &[u8]) -> [usize; 4] {
    let lz = leading_zeros;
    let mut best = [0usize; 4];
    let Some((mk, mc)) = filter::master(seed) else {
        return best;
    };
    best[0] = lz(&mk);
    let Some((pk, pc)) = filter::hardened(&mk, &mc, h(44)) else {
        return best;
    };
    best[1] = lz(&pk);
    for coin in [20, 42] {
        let Some((ck, cc)) = filter::hardened(stripped(&pk), &pc, h(coin)) else {
            continue;
        };
        best[2] = best[2].max(lz(&ck));
        for (acct, stored) in [(0, stripped(&ck)), (1, &ck[..])] {
            if let Some((ak, _)) = filter::hardened(stored, &cc, h(acct)) {
                best[3] = best[3].max(lz(&ak));
            }
        }
    }
    best
}

/// Runs [`screen_wallet`] over `workers` deterministic streams of
/// `per_worker` 32-byte seeds and returns every seed with a leading zero
/// byte on some key, with its counts.
fn search_wallet(base: u64, workers: u64, per_worker: u64) -> Vec<(Vec<u8>, [usize; 4])> {
    let ws: Vec<u64> = (0..workers).collect();
    let found = par_map(&ws, |&w| {
        let mut rng = SplitMix64(base ^ w.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut out = Vec::new();
        let mut seed = [0u8; 32];
        for _ in 0..per_worker {
            rng.fill(&mut seed);
            let z = screen_wallet(&seed);
            if z.iter().any(|&n| n > 0) {
                out.push((seed.to_vec(), z));
            }
        }
        out
    });
    found.into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// The differential.
// ---------------------------------------------------------------------------

/// How many of `total` random seeds each of `nets` networks gets: an even
/// split, the first `total % nets` networks one more.
fn per_network(total: usize, nets: usize) -> Vec<usize> {
    (0..nets)
        .map(|i| total / nets + usize::from(i < total % nets))
        .collect()
}

/// Compares `total` random seeds (usually 32 bytes, one in eight of another
/// allowed length), split evenly over the four networks
/// ([`per_network`]); on each network the first eight line by line, the
/// rest in batches of at most [`BATCH`] by digest.
fn random_seeds(oracle: &mut Oracle, total: usize) -> WalletCompared {
    let nets = wallet_nets();
    let quotas = per_network(total, nets.len());
    let mut rng = SplitMix64::from_env("hdkeychain-wallet-random");
    let start = Instant::now();
    let mut compared = WalletCompared::default();
    let mut on_net = Vec::with_capacity(nets.len());
    let mut batches = 0usize;
    for (net, &quota) in nets.iter().zip(&quotas) {
        let mut here = WalletCompared::default();
        let lines: Vec<Vec<u8>> = (0..8.min(quota)).map(|_| random_seed(&mut rng)).collect();
        here.merge(compare_wallet_lines(oracle, net, &lines));
        while here.seeds < quota {
            let n = BATCH.min(quota - here.seeds);
            let seeds: Vec<Vec<u8>> = (0..n).map(|_| random_seed(&mut rng)).collect();
            here.merge(compare_wallet_digests(oracle, net, &seeds, BATCH));
            batches += 1;
            if total >= 100_000 && batches.is_multiple_of(50) {
                eprintln!(
                    "  {} seeds compared ({:.1?})",
                    compared.seeds + here.seeds,
                    start.elapsed()
                );
            }
        }
        on_net.push(here.seeds);
        compared.merge(here);
    }
    // Every network got its share, counted from what was compared.
    assert_eq!(on_net, quotas, "seeds compared per network");
    assert_eq!(compared.seeds, total, "seeds compared vs generated");
    // Every seed has an allowed length and an invalid child is < 2^-127 per
    // step, so no line is an error; 2 coin types x 2 accounts per seed.
    assert_eq!(compared.errors, 0, "unexpected error lines");
    assert_eq!(compared.accounts, total * 4, "four account lines per seed");
    let split: Vec<String> = nets
        .iter()
        .zip(&on_net)
        .map(|(net, n)| format!("{} {n}", net.name))
        .collect();
    eprintln!(
        "wallet random seeds: {} seeds ({}), {} account xpubs with the first address of both \
         branches, read from purse-udb's address manager, equal to Go ({:.1?})",
        compared.seeds,
        split.join(", "),
        compared.accounts,
        start.elapsed()
    );
    compared
}

/// The default and full runs split their random seeds evenly over the
/// networks. (They once went in whole batches to one network after
/// another, so the default 4,000 compared only 8 seeds each on simnet and
/// regnet.)
#[test]
fn random_seeds_split_evenly_over_networks() {
    assert_eq!(per_network(4000, 4), [1000; 4]);
    assert_eq!(per_network(1_000_000, 4), [250_000; 4]);
    assert_eq!(per_network(4003, 4), [1001, 1001, 1001, 1000]);
    assert_eq!(per_network(3, 4), [1, 1, 1, 0]);
    for total in [0, 1, 7, 4000, 12_345] {
        let q = per_network(total, 4);
        assert_eq!(q.iter().sum::<usize>(), total);
        assert!(q.iter().max().unwrap() - q.iter().min().unwrap() <= 1);
    }
}

/// Searches `WORKERS * per_worker` seeds for leading-zero keys on
/// dcrwallet's routes and compares every seed found on mainnet (whose coin
/// types the search follows). Returns the comparison and the seeds found.
fn leading_zero_seeds(oracle: &mut Oracle, per_worker: u64) -> (WalletCompared, usize) {
    let base = SplitMix64::from_env("hdkeychain-wallet-leading-zero").0;
    let start = Instant::now();
    let found = search_wallet(base, WORKERS, per_worker);
    let screened = WORKERS * per_worker;
    let mut by_class: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (_, z) in &found {
        for (d, &n) in z.iter().enumerate() {
            if n > 0 {
                *by_class.entry((d, n)).or_default() += 1;
            }
        }
    }
    let nets = wallet_nets();
    let mainnet = &nets[0];
    let seeds: Vec<Vec<u8>> = found.iter().map(|(s, _)| s.clone()).collect();
    let mut compared = WalletCompared::default();
    if !seeds.is_empty() {
        let first = 8.min(seeds.len());
        compared.merge(compare_wallet_lines(oracle, mainnet, &seeds[..first]));
        compared.merge(compare_wallet_digests(
            oracle,
            mainnet,
            &seeds[first..],
            BATCH,
        ));
    }
    assert_eq!(compared.seeds, seeds.len(), "every seed found is compared");
    assert_eq!(compared.errors, 0, "unexpected error lines");
    eprintln!(
        "wallet leading-zero seeds: screened {screened}, found and compared {} seeds \
         ({} account lines) with Go, read from purse-udb's address manager ({:.1?})",
        compared.seeds,
        compared.accounts,
        start.elapsed()
    );
    for ((d, z), n) in &by_class {
        let name = ["master", "purpose", "coin type", "account"][*d];
        eprintln!("  {name:>9} key: {n:>7} seeds with {z} leading zero byte(s)");
    }
    for d in 0..4 {
        let n: usize = by_class
            .iter()
            .filter(|((dd, _), _)| *dd == d)
            .map(|(_, n)| n)
            .sum();
        assert!(
            n * 2000 >= screened as usize,
            "only {n} seeds with a zero at depth {d} of {screened} screened"
        );
    }

    // The provenance split was exercised: where a coin type key is stored
    // stripped, account 1 from the in-memory key and from the re-parsed key
    // (the one Go's answer was compared against) differ.
    let mut split = 0usize;
    for (seed, z) in &found {
        if z[2] == 0 {
            continue;
        }
        let m = ExtendedKey::new_master(seed, &HdVersions::MAINNET).expect("master");
        for coin in [20, 42] {
            let ct = m
                .child(h(44))
                .and_then(|p| p.child(h(coin)))
                .expect("coin type key");
            if ct.serialized_priv_key().expect("private").len() == 32 {
                continue;
            }
            let in_memory = ct.child(h(1)).expect("account 1").neuter().string();
            let reparsed = ExtendedKey::new_key_from_string(&ct.string(), &HdVersions::MAINNET)
                .and_then(|k| k.child(h(1)))
                .expect("account 1 from the re-parsed key")
                .neuter()
                .string();
            assert_ne!(*in_memory, *reparsed, "a stripped coin type key must split");
            split += 1;
        }
    }
    eprintln!("  coin type keys stored stripped (account 1 split exercised): {split}");
    assert!(split > 0, "no stripped coin type key among the seeds found");
    (compared, seeds.len())
}

#[test]
fn random_seeds_match_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    random_seeds(&mut oracle, env_count("PURSE_HD_WALLET_SEEDS", 4000));
}

#[test]
fn leading_zero_seeds_match_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let per_worker = env_count("PURSE_HD_WALLET_LZ_PER_WORKER", 25_000) as u64;
    leading_zero_seeds(&mut oracle, per_worker);
}

/// Seeds of disallowed lengths fail in `HDKeysFromSeed`, reached through
/// [`Manager::create`], with Go's text.
#[test]
fn seed_lengths_match_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let mut rng = SplitMix64::from_env("hdkeychain-wallet-lengths");
    let seeds: Vec<Vec<u8>> = [0usize, 1, 15, 16, 32, 64, 65, 100]
        .iter()
        .map(|&n| rng.bytes(n))
        .collect();
    let mut errors = 0;
    for net in &wallet_nets() {
        errors += compare_wallet_lines(&mut oracle, net, &seeds).errors;
    }
    assert_eq!(errors, 4 * 5, "five disallowed lengths on four networks");
}

/// The cheap scrypt cost changes no line: seeds on the three networks
/// where dcrwallet's cost is `N = 2^18, r = 8` (four 256 MiB derivations
/// per manager, two managers per seed), with the unmodified chain
/// parameters, equal Go's lines and the cheap path's. Among them, on
/// mainnet, a seed whose coin type keys are stored stripped.
#[test]
fn dcrwallet_scrypt_costs_give_the_same_lines() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-udb-wallet-real-cost");
    let start = Instant::now();
    let mut compared = WalletCompared::default();
    for net in wallet_nets().iter().filter(|n| !n.real.simnet) {
        assert_eq!(net.real.scrypt_cost(), (262_144, 8, 1), "{}", net.name);
        let mut seeds = vec![rng.bytes(32)];
        if net.name == "mainnet" {
            // A seed the search flags for a stripped coin type key.
            let found = search_wallet(rng.next_u64(), 1, 2000);
            let (seed, _) = found
                .into_iter()
                .find(|(_, z)| z[2] > 0)
                .expect("a stripped coin type key in 2000 seeds");
            seeds.push(seed);
        }
        let go = go_wallet_lines(&mut oracle, net, &seeds);
        let real = par_map(&seeds, |seed| wallet_lines_with(seed, &net.real));
        for ((g, r), seed) in go.iter().zip(&real).zip(&seeds) {
            assert!(
                g == r,
                "mismatch on {} at dcrwallet's cost: {}",
                net.name,
                first_difference(g, r, seed)
            );
            assert_eq!(
                *r,
                wallet_lines(seed, net),
                "{}: the cheap cost changed seed {}",
                net.name,
                hex(seed)
            );
            compared.add(r);
        }
    }
    assert_eq!(compared.seeds, 4);
    assert_eq!(compared.accounts, 16);
    eprintln!(
        "wallet at dcrwallet's scrypt cost: {} seeds, {} account lines, equal to Go and to \
         the cheap cost ({:.1?})",
        compared.seeds,
        compared.accounts,
        start.elapsed()
    );
}

/// The full run of the exit criterion; see the module documentation.
#[test]
#[ignore = "10^6 seeds and a 4,000,000-seed search, a wallet per seed and coin type: long in a release build; run by hand"]
fn million_seeds_match_go() {
    let mut oracle = oracle_or_skip("hdkeychain").expect("the Go oracle is required");
    let start = Instant::now();
    let random = random_seeds(&mut oracle, env_count("PURSE_HD_WALLET_SEEDS", 1_000_000));
    let per_worker = env_count("PURSE_HD_WALLET_LZ_PER_WORKER", 250_000) as u64;
    let (lz, found) = leading_zero_seeds(&mut oracle, per_worker);
    eprintln!(
        "million_seeds_match_go: {} random and {found} leading-zero seeds, {} account lines, \
         equal to Go ({:.1?})",
        random.seeds,
        random.accounts + lz.accounts,
        start.elapsed()
    );
}

/// Where a seed's time goes, phase by phase, on one thread: not a parity
/// test and not a benchmark of record, a guide to what dominates.
#[test]
#[ignore = "prints a time breakdown; run by hand with --nocapture"]
fn cost_per_seed() {
    let n = env_count("PURSE_HD_WALLET_COST_SEEDS", 200);
    let mut rng = SplitMix64::from_env("purse-udb-wallet-cost");
    let nets = wallet_nets();
    let net = &nets[0];
    let mut t: BTreeMap<&str, Duration> = BTreeMap::new();
    let mut lap = |name: &'static str, since: &mut Instant| {
        let now = Instant::now();
        *t.entry(name).or_default() += now - *since;
        *since = now;
    };
    let total = Instant::now();
    for _ in 0..n {
        let seed = rng.bytes(32);
        for slip0044 in [false, true] {
            let mut at = Instant::now();
            let store = memory_store();
            lap("1 store creation", &mut at);
            let mut src = random_source(1);
            update(&store, |tx| {
                Manager::create(
                    tx,
                    &net.cheap,
                    &seed,
                    PUB_PASS,
                    PRIV_PASS,
                    &mut src,
                    &Limits::DEFAULT,
                )
            })
            .expect("create");
            lap("2 Manager::create (commit included)", &mut at);
            let tx = store.read().expect("read");
            let mgr = Manager::open(
                &mut &tx,
                &net.cheap,
                PUB_PASS,
                Box::new(random_source(2)),
                Limits::DEFAULT,
            )
            .expect("open");
            drop(tx);
            lap("3 Manager::open", &mut at);
            mgr.unlock(PRIV_PASS).expect("unlock");
            lap("4 Manager::unlock", &mut at);
            if slip0044 {
                update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
                lap("5 upgrade_to_slip0044_coin_type", &mut at);
            }
            let tx = store.read().expect("read");
            let ct = mgr.coin_type_priv_key(&mut &tx).expect("coin type key");
            let _ = ct.neuter().string();
            drop(tx);
            lap("6 coin type key", &mut at);
            update(&store, |tx| mgr.new_account(tx, ACCOUNT_1_NAME)).expect("new account");
            lap("7 new_account", &mut at);
            let w = Wallet {
                store,
                mgr,
                chain: net.cheap,
            };
            let mut unlocked = Vec::new();
            for account in WALLET_ACCOUNTS {
                sync_first_addresses(&w, account).expect("sync");
            }
            lap("8 record child 0 of both branches", &mut at);
            for account in WALLET_ACCOUNTS {
                unlocked.push(list_first_addresses(&w, account).expect("list"));
            }
            lap("9 list them unlocked (private)", &mut at);
            w.mgr.lock().expect("lock");
            for (account, first) in WALLET_ACCOUNTS.into_iter().zip(&unlocked) {
                let line = account_line(&w, 0, account, first);
                assert!(line.starts_with("A|"), "{line}");
            }
            lap("a locked: list, look up, xpubs (public)", &mut at);
        }
    }
    let total = total.elapsed();
    let n32 = u32::try_from(n).expect("small");
    eprintln!("{n} mainnet seeds, two managers each, one thread: {total:.2?}");
    for (name, d) in &t {
        eprintln!(
            "  {name:<40} {:>9.1?} per seed  {:>5.1}%",
            *d / n32,
            100.0 * d.as_secs_f64() / total.as_secs_f64()
        );
    }
    // The derivation inside Manager::create, alone, for comparison.
    let mut rng = SplitMix64::from_env("purse-udb-wallet-cost");
    let at = Instant::now();
    for _ in 0..n {
        let seed = rng.bytes(32);
        for _ in 0..2 {
            purse_udb::addrmgr::hd_keys_from_seed(&seed, &net.cheap).expect("keys");
        }
    }
    eprintln!(
        "  of 2, hd_keys_from_seed twice:          {:>9.1?} per seed",
        at.elapsed() / n32
    );
}

// ---------------------------------------------------------------------------
// Frozen vectors.
//
// tests/data/wallet_lines.txt holds, after its header, one entry per seed:
//
//     W <network> <seed hex, or ~ for the empty seed>
//     <one line of Go's answer>...
// ---------------------------------------------------------------------------

const WALLET_LINES: &str = "wallet_lines.txt";
const WALLET_ENTRIES: usize = 58;
const FROZEN_RANDOM_SEED: u64 = 0x7761_6c6c_6574_0001;
const FROZEN_SEARCH_BASE: u64 = 0x7761_6c6c_6574_0002;
const FROZEN_RANDOM_PER_NET: usize = 8;
const FROZEN_LZ_PER_DEPTH: usize = 6;

/// The `tests/data` directory.
fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// The SHA-256 of a vector file's body: every line that is not a comment
/// (nor empty), each followed by a newline, in order.
fn body_sha256(text: &str) -> String {
    let mut h = Sha256::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        h.update(line.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// The frozen set: eight random seeds per network, two disallowed lengths
/// on mainnet, and up to six leading-zero seeds per hardened depth from a
/// fixed search (two-zero seeds first).
fn frozen_seeds() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = Vec::new();
    let mut rng = SplitMix64(FROZEN_RANDOM_SEED);
    for net in wallet_nets() {
        for _ in 0..FROZEN_RANDOM_PER_NET {
            out.push((net.name, random_seed(&mut rng)));
        }
    }
    out.push(("mainnet", Vec::new()));
    out.push(("mainnet", rng.bytes(65)));
    let mut found = search_wallet(FROZEN_SEARCH_BASE, WORKERS, 2000);
    found.sort_by_key(|(_, z)| std::cmp::Reverse(*z.iter().max().expect("4 depths")));
    let mut per_depth = [0usize; 4];
    for (seed, z) in found {
        let max = *z.iter().max().expect("4 depths");
        let depth = z.iter().position(|&n| n == max).expect("max present");
        if per_depth[depth] < FROZEN_LZ_PER_DEPTH {
            per_depth[depth] += 1;
            out.push(("mainnet", seed));
        }
    }
    out
}

fn net_by_name(name: &str) -> WalletNet {
    wallet_nets()
        .into_iter()
        .find(|n| n.name == name)
        .unwrap_or_else(|| panic!("unknown network {name}"))
}

fn read_wallet_file(text: &str) -> Vec<(String, Vec<u8>, Vec<String>)> {
    let mut entries: Vec<(String, Vec<u8>, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("W ") {
            let (net, seed) = rest.split_once(' ').expect("W <net> <seed>");
            let seed = if seed == "~" { Vec::new() } else { unhex(seed) };
            entries.push((net.to_owned(), seed, Vec::new()));
        } else {
            entries
                .last_mut()
                .expect("line before W")
                .2
                .push(line.to_owned());
        }
    }
    entries
}

fn header_field<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .map(str::trim)
        .unwrap_or_else(|| panic!("{WALLET_LINES}: no {key:?} header"))
}

#[test]
fn frozen_wallet_lines() {
    let text = std::fs::read_to_string(data_dir().join(WALLET_LINES))
        .unwrap_or_else(|e| panic!("read tests/data/{WALLET_LINES}: {e}"));
    let entries = read_wallet_file(&text);
    assert_eq!(
        header_field(&text, "# entries:")
            .parse::<usize>()
            .expect("count"),
        entries.len(),
        "entry count"
    );
    assert_eq!(entries.len(), WALLET_ENTRIES, "pinned entry count");
    assert_eq!(
        body_sha256(&text),
        header_field(&text, "# body-sha256:"),
        "body hash"
    );
    assert_eq!(
        header_field(&text, "# dcrwallet commit:"),
        purse_testutil::DCRWALLET_PARITY_COMMIT
    );

    // The file holds exactly the deterministic set.
    let want = frozen_seeds();
    assert_eq!(want.len(), entries.len(), "stale file: regenerate");
    for ((net, seed, _), (want_net, want_seed)) in entries.iter().zip(&want) {
        assert_eq!(
            (net.as_str(), seed),
            (*want_net, want_seed),
            "stale file: regenerate"
        );
    }
    let rust = par_map(&entries, |(net, seed, _)| {
        wallet_lines(seed, &net_by_name(net))
    });
    let mut accounts = 0usize;
    for ((net, seed, lines), rust) in entries.iter().zip(&rust) {
        assert_eq!(rust, lines, "seed {} on {net}", hex(seed));
        accounts += lines.iter().filter(|l| l.starts_with("A|")).count();
    }
    assert_eq!(accounts, (WALLET_ENTRIES - 2) * 4);
}

#[test]
#[ignore = "writes tests/data from the Go oracle; set PURSE_HD_REGEN=1"]
fn regenerate_wallet_vectors() {
    if std::env::var_os("PURSE_HD_REGEN").is_none() {
        eprintln!("skipping: set PURSE_HD_REGEN=1 to regenerate tests/data");
        return;
    }
    let mut oracle = oracle_or_skip("hdkeychain").expect("the Go oracle is required");
    let mut body = String::new();
    for (name, seed) in frozen_seeds() {
        let net = net_by_name(name);
        let go = go_wallet_lines(&mut oracle, &net, std::slice::from_ref(&seed));
        assert_eq!(
            go[0],
            wallet_lines(&seed, &net),
            "Rust disagrees with Go; not writing"
        );
        let seed_field = if seed.is_empty() {
            "~".to_owned()
        } else {
            hex(&seed)
        };
        let _ = writeln!(body, "W {name} {seed_field}");
        for l in &go[0] {
            let _ = writeln!(body, "{l}");
        }
    }
    let entries = body.lines().filter(|l| l.starts_with("W ")).count();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# Account extended public keys and first addresses dcrwallet derives from seeds.\n\
         #\n\
         # tests/wallet_diff.rs `frozen_seeds`: {FROZEN_RANDOM_PER_NET} random seeds per network \
         from\n\
         # SplitMix64({FROZEN_RANDOM_SEED:#x}), the empty seed and a 65-byte seed on mainnet, \
         then up to\n\
         # {FROZEN_LZ_PER_DEPTH} seeds per hardened depth with a leading-zero key, from \
         `search_wallet`\n\
         # with base {FROZEN_SEARCH_BASE:#x} over {WORKERS} x 2000 seeds.\n\
         #\n\
         # Every expected value is the output of the Go oracle tools/oracle/cmd/hdkeychain\n\
         # (command wallet: dcrwallet's udb.HDKeysFromSeed, hdkeychain v3.1.3, stdaddr),\n\
         # written by `PURSE_HD_REGEN=1 cargo test --release -p purse-udb --test\n\
         # wallet_diff -- --ignored regenerate_wallet_vectors`, which also checked the Rust\n\
         # side (purse-udb's address manager) equal. Format:\n\
         # tools/oracle/cmd/hdkeychain/wallet.go.\n\
         # dcrwallet commit: {}\n\
         # entries: {entries}\n\
         # body-sha256: {}",
        purse_testutil::DCRWALLET_PARITY_COMMIT,
        body_sha256(&body)
    );
    text.push_str(&body);
    let path = data_dir().join(WALLET_LINES);
    std::fs::write(&path, &text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote tests/data/{WALLET_LINES} ({entries} entries)");
}
