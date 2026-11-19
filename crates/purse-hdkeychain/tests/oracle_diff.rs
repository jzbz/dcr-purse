// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrd's hdkeychain v3.1.3, through the oracle
//! binary `tools/oracle/cmd/hdkeychain`.
//!
//! Environment:
//! - `PURSE_HD_SEEDS`: random seeds for `random_seeds_along_wallet_paths`
//!   (default 3000).
//! - `PURSE_HD_LZ_PER_WORKER`: seeds each of the 16 leading-zero search
//!   streams screens (default 25000).
//! - `PURSE_TEST_SEED`: the run seed, mixed with each test's label (printed
//!   by each test); unset, every run uses the same inputs.
//! - `PURSE_REQUIRE_ORACLE`: fail instead of skipping without Go.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

mod common;

use std::collections::BTreeMap;

use common::*;
use purse_hdkeychain::{
    ExtendedKey, HARDENED_KEY_START, HdVersions, MAX_SEED_BYTES, MIN_SEED_BYTES,
    RECOMMENDED_SEED_LEN, SeedError, generate_seed,
};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, oracle_or_skip};

fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().replace('_', "").parse().ok())
        .unwrap_or(default)
}

/// (a) Random seeds down m/44'/42'/acct'/branch/index and the legacy
/// m/44'/20'/..., with `child`, `child_bip32_std`, a re-parsed coin type
/// key, public derivation below the account and mixed variants, comparing
/// every intermediate key's line, on all four networks.
#[test]
fn random_seeds_along_wallet_paths() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let total = env_count("PURSE_HD_SEEDS", 3000);
    let mut rng = SplitMix64::from_env("hdkeychain-random-seeds");
    let start = std::time::Instant::now();

    // The first jobs line by line, so a digest bug cannot hide everything.
    let first = random_jobs(&mut rng, 32.min(total));
    let mut compared = compare_lines(&mut oracle, &HdVersions::MAINNET, &first);
    let mut generated = first.len();

    let batch = 2048usize;
    let mut round = 0usize;
    while generated < total {
        let n = batch.min(total - generated);
        let jobs = random_jobs(&mut rng, n);
        generated += jobs.len();
        let net = NETS[round % NETS.len()].1;
        compared.merge(compare_digests(&mut oracle, &net, &jobs, n));
        round += 1;
    }
    compared.assert_all(total);
    // Every seed is a valid length and every index derives (an invalid
    // child is < 2^-127 per step), so no line is an error.
    assert_eq!(compared.error_lines, 0, "unexpected error lines");
    assert_eq!(compared.lines, total * 10 * 6, "one line per root and step");
    eprintln!(
        "random_seeds_along_wallet_paths: {} seeds x 10 paths ({} key lines) agree with Go \
         ({:.1?})",
        compared.jobs,
        compared.lines,
        start.elapsed()
    );
}

/// Seeds outside the allowed lengths, and the boundaries.
#[test]
fn seed_lengths() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let mut rng = SplitMix64::from_env("hdkeychain-seed-lengths");
    let jobs: Vec<Job> = [0usize, 1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 100, 255]
        .iter()
        .map(|&n| Job::seed(&rng.bytes(n), vec![String::new(), format!("{}/0", h(0))]))
        .collect();
    let compared = compare_lines(&mut oracle, &HdVersions::MAINNET, &jobs);
    compared.assert_all(jobs.len());
    // 6 lengths are out of range: one error line per path.
    assert_eq!(compared.error_lines, 6 * 2);
}

/// (b) Seeds whose private keys on the hardened part of the wallet path
/// start with zero bytes, at every hardened depth, derived from the seed
/// and re-parsed from strings mid-path (the provenance split).
#[test]
fn leading_zero_seeds() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let per_worker = env_count("PURSE_HD_LZ_PER_WORKER", 25_000) as u64;
    let base = SplitMix64::from_env("hdkeychain-leading-zero").0;
    let start = std::time::Instant::now();
    let found = search(base, 16, per_worker);
    let screened = 16 * per_worker;

    // Keep every seed with two or more zero bytes anywhere, and up to 60
    // single-zero seeds per depth.
    let mut per_depth = [0usize; 4];
    let mut selected: Vec<Vec<u8>> = Vec::new();
    for (seed, z) in &found {
        let max = *z.iter().max().expect("4 depths");
        let depth = z.iter().position(|&n| n == max).expect("max present");
        if max >= 2 || per_depth[depth] < 60 {
            if max < 2 {
                per_depth[depth] += 1;
            }
            selected.push(seed.clone());
        }
    }

    let net = HdVersions::MAINNET;
    let paths = leading_zero_paths();
    let mut jobs: Vec<Job> = selected
        .iter()
        .map(|s| Job::seed(s, paths.clone()))
        .collect();

    // The same keys reached from their strings: purpose, coin type and
    // account keys derived with `child` (so possibly stored stripped),
    // serialized, and derived on from the parsed (padded) key; and the
    // account's public key.
    let mut split_differs = 0usize;
    for s in &selected {
        let m = ExtendedKey::new_master(s, &net).expect("master");
        let p = m.child(h(44)).expect("purpose");
        jobs.push(Job::key(
            &p.string(),
            vec![
                format!("{}/{}/0/0", h(42), h(0)),
                format!("{}s/{}s/0/0", h(20), h(1)),
            ],
        ));
        for coin in [42, 20] {
            let c = p.child(h(coin)).expect("coin");
            jobs.push(Job::key(
                &c.string(),
                vec![
                    format!("{}/0/0", h(0)),
                    format!("{}/1/5", h(1)),
                    format!("{}s/0/0", h(0)),
                    format!("{}n/0/0", h(0)),
                ],
            ));
            let a = c.child(h(0)).expect("account");
            jobs.push(Job::key(&a.string(), vec!["0/0".into(), "1/2".into()]));
            jobs.push(Job::key(
                &a.neuter().string(),
                vec!["0/0".into(), "1/2".into()],
            ));

            // The quirk itself: the account from the in-memory coin type
            // key and from its re-parsed string differ exactly when the
            // coin type key was stored stripped.
            let reparsed = ExtendedKey::new_key_from_string(&c.string(), &net)
                .and_then(|k| k.child(h(0)))
                .expect("reparsed account");
            let stripped = c.serialized_priv_key().expect("private").len() < 32;
            assert_eq!(
                *a.string() != *reparsed.string(),
                stripped,
                "provenance split must follow the stored length"
            );
            split_differs += usize::from(stripped);
        }
    }

    // Line by line for the first few, by digest for the rest.
    compare_lines(&mut oracle, &net, &jobs[..8.min(jobs.len())]).assert_all(8.min(jobs.len()));
    let compared = compare_digests(&mut oracle, &net, &jobs, 256);
    compared.assert_all(jobs.len());
    assert_eq!(compared.error_lines, 0, "unexpected error lines");

    // Report what was covered, classified by the crate's own (Go-checked)
    // derivation.
    let mut by_class: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut stripped_at = [0usize; 4];
    for s in &selected {
        let (zeros, stripped) = classify(s);
        for d in 0..4 {
            if zeros[d] > 0 {
                *by_class.entry((d, zeros[d])).or_default() += 1;
            }
            stripped_at[d] += usize::from(stripped[d]);
        }
    }
    eprintln!(
        "leading_zero_seeds: screened {screened}, compared {} seeds ({} jobs, {} key lines) \
         with Go in {:.1?}",
        selected.len(),
        compared.jobs,
        compared.lines,
        start.elapsed()
    );
    for ((d, z), n) in &by_class {
        let name = ["master", "purpose", "coin type", "account"][*d];
        eprintln!("  {name:>9} depth: {n:>4} seeds with {z} leading zero byte(s)");
    }
    eprintln!(
        "  stored stripped by child at purpose/coin/account: {:?}; coin type keys whose \
         re-parsed account differs: {split_differs}",
        &stripped_at[1..]
    );

    assert_eq!(stripped_at[0], 0, "a master key is never stripped");
    assert!(
        selected.len() >= 200,
        "only {} leading-zero seeds",
        selected.len()
    );
    for d in 0..4 {
        let n: usize = by_class
            .iter()
            .filter(|((dd, _), _)| *dd == d)
            .map(|(_, n)| n)
            .sum();
        assert!(n >= 25, "only {n} seeds with zeros at depth {d}");
    }
    let two: usize = by_class
        .iter()
        .filter(|((_, z), _)| *z >= 2)
        .map(|(_, n)| n)
        .sum();
    if per_worker >= 25_000 {
        assert!(two >= 3, "only {two} two-zero cases");
    }
    assert!(split_differs > 0, "no stripped coin type key was exercised");
}

/// (c) Parsing: a corpus of valid keys, wrong networks, mutations, length
/// and character edge cases and edge-valued payloads, compared with Go's
/// exact results and error texts, then derived from.
#[test]
fn parse_corpus_matches_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let mut rng = SplitMix64::from_env("hdkeychain-parse-corpus");
    let corpus = parse_corpus(&mut rng);

    let mut ok = 0usize;
    let mut parsed_compared = 0usize;
    let mut derived = Compared::default();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for (_, net) in NETS {
        let keys: Vec<&String> = corpus
            .iter()
            .filter(|(n, _)| *n == net)
            .map(|(_, s)| s)
            .collect();
        if keys.is_empty() {
            continue;
        }

        // The parse command.
        let go: Vec<String> = purse_testutil::serde_json::from_value(
            oracle.call("parse", json!({ "net": net_json(&net), "keys": keys })),
        )
        .expect("lines");
        assert_eq!(
            go.len(),
            keys.len(),
            "Go answered a different number of keys"
        );
        for (s, g) in keys.iter().zip(&go) {
            parsed_compared += 1;
            let r = match ExtendedKey::new_key_from_string(s, &net) {
                Ok(k) => key_line(&k),
                Err(e) => err_line(&e),
            };
            assert_eq!(&r, g, "parse {s:?}");
            if let Some(text) = g.strip_prefix("E|") {
                let key = if text.starts_with("invalid public key: x coordinate") {
                    "invalid public key: x coordinate .. is not on the secp256k1 curve".to_owned()
                } else {
                    text.to_owned()
                };
                *errors.entry(key).or_default() += 1;
            } else {
                ok += 1;
            }
        }

        // Derivation from every parsed key.
        let jobs: Vec<Job> = keys.iter().map(|s| Job::key(s, corpus_paths())).collect();
        derived.merge(compare_lines(&mut oracle, &net, &jobs));
    }
    assert_eq!(
        parsed_compared,
        corpus.len(),
        "strings compared vs generated"
    );
    derived.assert_all(corpus.len());
    eprintln!(
        "parse_corpus_matches_go: {} strings, {ok} parsed",
        corpus.len()
    );
    for (e, n) in &errors {
        eprintln!("  {n:>4} x {e}");
    }
    assert!(ok >= 80, "only {ok} corpus strings parse");
    assert!(
        errors.len() >= 9,
        "corpus reaches only {} distinct errors",
        errors.len()
    );
}

/// (d) Public derivation from extended public keys equals neutering the
/// private derivation, in Rust, and both equal Go.
#[test]
fn public_derivation_equals_neutered_private() {
    let mut rng = SplitMix64::from_env("hdkeychain-public-derivation");
    let mut jobs = Vec::new();
    for round in 0..64 {
        let net = NETS[round % 4].1;
        let seed = random_seed(&mut rng);
        let Ok(m) = ExtendedKey::new_master(&seed, &net) else {
            continue;
        };
        let acct = m
            .child(h(44))
            .and_then(|k| k.child(h(42)))
            .and_then(|k| k.child(h(rng.below(3) as u32)))
            .expect("account");
        let xpub = acct.neuter();
        for _ in 0..4 {
            let branch = rng.below(2) as u32;
            let index = rng.below(1 << 31) as u32;
            let via_priv = acct
                .child(branch)
                .and_then(|k| k.child(index))
                .expect("private");
            let via_pub = xpub
                .child(branch)
                .and_then(|k| k.child(index))
                .expect("public");
            assert!(!via_pub.is_private());
            assert_eq!(*via_priv.neuter().string(), *via_pub.string());
            assert_eq!(via_priv.serialized_pub_key(), via_pub.serialized_pub_key());
            assert_eq!(via_priv.parent_fingerprint(), via_pub.parent_fingerprint());
        }
        assert_eq!(
            xpub.child(HARDENED_KEY_START).map(|_| ()),
            Err(purse_hdkeychain::Error::DeriveHardFromPublic)
        );
        if net == HdVersions::MAINNET {
            jobs.push(Job::key(
                &xpub.string(),
                vec![
                    "0/0".into(),
                    format!("1/{}", rng.below(1 << 31)),
                    h(0).to_string(),
                    "0/1/2/3/4/5/6/7".into(),
                ],
            ));
        }
    }

    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let compared = compare_lines(&mut oracle, &HdVersions::MAINNET, &jobs);
    compared.assert_all(jobs.len());
    // Only the hardened step from each xpub fails.
    assert_eq!(compared.error_lines, jobs.len());
}

/// The constants and error texts equal Go's.
#[test]
fn constants_and_error_texts_match_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    use purse_hdkeychain::Error;
    let go = oracle.call("consts", Value::Null);
    let texts = [
        ("ErrDeriveHardFromPublic", Error::DeriveHardFromPublic),
        ("ErrNotPrivExtKey", Error::NotPrivExtKey),
        ("ErrInvalidChild", Error::InvalidChild),
        ("ErrUnusableSeed", Error::UnusableSeed),
        ("ErrInvalidSeedLen", Error::InvalidSeedLen),
        ("ErrBadChecksum", Error::BadChecksum),
        ("ErrInvalidKeyLen", Error::InvalidKeyLen),
        ("ErrWrongNetwork", Error::WrongNetwork),
    ];
    for (name, e) in texts {
        assert_eq!(go[name].as_str(), Some(e.to_string().as_str()), "{name}");
    }
    assert_eq!(go["RecommendedSeedLen"], json!(RECOMMENDED_SEED_LEN));
    assert_eq!(go["HardenedKeyStart"], json!(HARDENED_KEY_START));
    assert_eq!(go["MinSeedBytes"], json!(MIN_SEED_BYTES));
    assert_eq!(go["MaxSeedBytes"], json!(MAX_SEED_BYTES));
    for (name, net) in NETS {
        assert_eq!(
            go["nets"][name],
            net_json(&net),
            "{name} versions (chaincfg)"
        );
    }
}

/// `generate_seed` accepts and rejects the same lengths as Go's
/// `GenerateSeed`, with the same error text.
#[test]
fn generate_seed_lengths_match_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let lengths: Vec<u8> = (0..=255).collect();
    let go: Vec<String> = purse_testutil::serde_json::from_value(
        oracle.call("generate_seed", json!({ "lengths": lengths })),
    )
    .expect("lines");
    let mut rng = SplitMix64::from_env("hdkeychain-generate-seed");
    for (&n, g) in lengths.iter().zip(&go) {
        let r = match generate_seed(n, |b: &mut [u8]| {
            rng.fill(b);
            Ok::<(), std::convert::Infallible>(())
        }) {
            Ok(s) => format!("len={}", s.len()),
            Err(SeedError::InvalidSeedLen) => format!("E|{}", SeedError::<String>::InvalidSeedLen),
            Err(SeedError::Rand(never)) => match never {},
        };
        assert_eq!(&r, g, "length {n}");
    }
}

/// Depth is 16 bits in memory: from a key string at depth 255, 65,281
/// derivations wrap `depth()` to 0, while the string carries depth % 256
/// all along. Compared with Go by digest.
#[test]
#[ignore = "65,281 derivations: slow in debug builds; run with --release -- --ignored"]
fn depth_wraps_like_go() {
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let net = HdVersions::MAINNET;
    let m = ExtendedKey::new_master(&[7u8; 32], &net).expect("master");
    let mut jobs = Vec::new();
    for k in [m.clone(), m.neuter()] {
        let mut payload = decode_payload(&k.string());
        payload[4] = 255;
        let s = encode_payload(&payload);
        let steps: Vec<&str> = vec!["1"; 65_282];
        jobs.push(Job::key(&s, vec![steps.join("/")]));
    }
    let lines = run_job(&jobs[0], &net);
    assert!(
        lines[0][65_281].starts_with("K|1|0|1|"),
        "{}",
        lines[0][65_281]
    );
    assert!(
        lines[0][65_282].starts_with("K|1|1|1|"),
        "{}",
        lines[0][65_282]
    );
    let compared = compare_digests(&mut oracle, &net, &jobs, 2);
    compared.assert_all(jobs.len());
    assert_eq!(compared.error_lines, 0);
}
