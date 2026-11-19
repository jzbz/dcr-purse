// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The first clause of the Phase 1 exit (brief section 6): for random seeds
//! and for every seed a targeted search finds with a leading-zero key on
//! dcrwallet's derivation routes, the account extended public keys of both
//! coin types and the first address of both branches of each account equal
//! the Go oracle's (`tools/oracle/cmd/hdkeychain`, command `wallet`, which
//! calls dcrwallet's own `udb.HDKeysFromSeed`). What is derived, and how,
//! is described in `tests/common/wallet.rs`.
//!
//! The default sizes run in the normal test suite. The full run, 10^6
//! random seeds plus a search over 4,000,000 seeds, is ignored; run it with
//!
//! ```text
//! PURSE_REQUIRE_ORACLE=1 cargo test --release -p purse-hdkeychain \
//!     --test wallet_diff -- --ignored --nocapture million_seeds
//! ```
//!
//! Environment:
//! - `PURSE_HD_WALLET_SEEDS`: random seeds (default 4000; 1,000,000 in the
//!   ignored test).
//! - `PURSE_HD_WALLET_LZ_PER_WORKER`: seeds each of the 16 search streams
//!   screens (default 25,000; 250,000 in the ignored test).
//! - `PURSE_TEST_SEED`: the run seed, mixed with each test's label (printed
//!   by each test); unset, every run uses the same inputs.
//! - `PURSE_REQUIRE_ORACLE`: fail instead of skipping without Go.
//!
//! The frozen file `tests/data/wallet_lines.txt` holds Go's full lines for
//! a fixed set of seeds and is replayed without Go; regenerate it with
//!
//! ```text
//! PURSE_HD_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release \
//!     -p purse-hdkeychain --test wallet_diff -- --ignored regenerate_wallet_vectors
//! ```

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Instant;

use common::wallet::{
    WalletCompared, WalletNet, compare_wallet_digests, compare_wallet_lines, go_wallet_lines,
    search_wallet, wallet_lines, wallet_nets,
};
use common::{body_sha256, data_dir, h, random_seed};
use purse_hdkeychain::{ExtendedKey, HdVersions};
use purse_testutil::{Oracle, SplitMix64, hex, oracle_or_skip, unhex};

/// Seeds per oracle request in digest mode.
const BATCH: usize = 2048;

/// Search streams.
const WORKERS: u64 = 16;

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().replace('_', "").parse().ok())
        .unwrap_or(default)
}

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
         branches, equal to Go ({:.1?})",
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
         ({} account lines) with Go ({:.1?})",
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

/// Seeds of disallowed lengths fail in `HDKeysFromSeed` with Go's text.
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

/// The full run of the exit criterion; see the module documentation.
#[test]
#[ignore = "10^6 seeds and a 4,000,000-seed search: minutes in a release build; run by hand"]
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
    let mut accounts = 0usize;
    for ((net, seed, lines), (want_net, want_seed)) in entries.iter().zip(&want) {
        assert_eq!(
            (net.as_str(), seed),
            (*want_net, want_seed),
            "stale file: regenerate"
        );
        let rust = wallet_lines(seed, &net_by_name(net));
        assert_eq!(&rust, lines, "seed {} on {net}", hex(seed));
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
         # written by `PURSE_HD_REGEN=1 cargo test --release -p purse-hdkeychain --test\n\
         # wallet_diff -- --ignored regenerate_wallet_vectors`, which also checked the Rust\n\
         # side equal. Format: crates/purse-hdkeychain/tests/common/wallet.rs.\n\
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
