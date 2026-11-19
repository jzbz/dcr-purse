// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against Go: `tools/oracle/cmd/sntrup`, which links
//! `github.com/companyzero/sntrup4591761` at the version dcrwallet links.
//!
//! - `random_corpus_agrees_with_go` builds a fresh corpus (see
//!   `common::corpus`) from `PURSE_TEST_SEED` and requires the Rust crate's
//!   outcome for every request to equal Go's: key generation from random
//!   streams (with the bytes consumed) and from streams cut short at every
//!   kind of boundary, encapsulation to generated and to malformed public
//!   keys, and decapsulation of valid, tampered, aliased, random and
//!   wrong-key ciphertexts and of malformed private keys, and of
//!   ciphertexts that fail exactly one of Go's three checks (an r of the
//!   wrong weight, one body coefficient moved by 3), where Go must also
//!   return the key of the r they were built from.
//!   `PURSE_SNTRUP_CASES` multiplies its size; an optimised build (or
//!   `PURSE_SNTRUP_HEAVY`) runs a larger plan.
//! - `many_keygen_seeds_agree_with_go` compares key generation alone over
//!   more seeds.
//! - `non_invertible_g_agrees_with_go` compares key generation from
//!   streams whose first g candidates have no inverse in R3 (multiples of
//!   the modulus's factors over GF(3)).
//! - `frozen_vectors_agree_with_go` replays every request of the frozen
//!   file `tests/data/sntrup_vectors.txt` through Go and requires the
//!   recorded outcome, so the file and the oracle cannot drift apart.
//! - `regenerate_frozen_vectors` (ignored) writes that file:
//!
//!   ```text
//!   PURSE_SNTRUP_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release \
//!       -p sntrup4591761 --test oracle -- --ignored --exact regenerate_frozen_vectors
//!   ```

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

mod common;

use std::collections::BTreeMap;

use common::{
    Entry, FROZEN_PLAN, FROZEN_SEED, POLY_BYTES, Plan, Request, corpus, env_count, go_outcome,
    heavy, read_vectors, rejection_streams, resolve, rust_outcome, write_vectors,
};
use purse_testutil::serde_json::Value;
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip};

/// The module version the oracle must link: dcrwallet's go.mod at the pin.
const SNTRUP_MODULE: &str = "github.com/companyzero/sntrup4591761";
const SNTRUP_VERSION: &str = "v0.0.0-20220309191932-9e0f3af2f07a";

fn oracle() -> Option<Oracle> {
    let mut oracle = oracle_or_skip("sntrup")?;
    let info = oracle.call("build.info", Value::Null);
    let linked = info["deps"]
        .as_array()
        .expect("deps")
        .iter()
        .find(|d| d["path"] == SNTRUP_MODULE)
        .unwrap_or_else(|| panic!("the oracle does not link {SNTRUP_MODULE}"));
    assert_eq!(linked["version"], SNTRUP_VERSION, "{SNTRUP_MODULE} version");
    assert!(
        linked.get("replace").is_none(),
        "{SNTRUP_MODULE} is replaced"
    );
    Some(oracle)
}

/// Runs every request through both sides; panics on the first difference.
/// Returns the per-label counts and the per-label count of valid
/// decapsulations (Go's `ok` = 1).
fn compare(oracle: &mut Oracle, requests: &[Request]) -> BTreeMap<&'static str, (usize, usize)> {
    let mut tally: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut outcomes = Vec::with_capacity(requests.len());
    for (i, r) in requests.iter().enumerate() {
        let args = resolve(&r.args, &outcomes);
        let go = go_outcome(oracle, &r.cmd, &args);
        let rust = rust_outcome(&r.cmd, &args);
        assert_eq!(
            rust, go,
            "request {i} ({}, {}): Rust and Go differ\nargs: {}",
            r.cmd, r.label, r.args
        );
        if let Some(key) = &r.expect_key {
            assert_eq!(
                go["ok"]["key"], *key,
                "request {i} ({}): Go did not decrypt the r the ciphertext was built from",
                r.label
            );
        }
        let t = tally.entry(r.label).or_default();
        t.0 += 1;
        if go["ok"]["ok"] == 1 {
            t.1 += 1;
        }
        outcomes.push(go);
    }
    tally
}

/// Key generation from streams whose first g candidates are multiples of
/// a factor of x^761 − x − 1 over GF(3): Go rejects them as the crate does
/// (the corpus's rejected candidates are only g = 0), whatever the factor
/// and its position.
#[test]
fn non_invertible_g_agrees_with_go() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let mut rng = SplitMix64::from_env("non_invertible_g_agrees_with_go");
    let rest = rng.bytes(2 * POLY_BYTES);
    for (label, stream, rejected) in rejection_streams(&mut rng, &rest) {
        let args = purse_testutil::serde_json::json!({ "rand": hex(&stream) });
        let go = go_outcome(&mut oracle, "keygen", &args);
        assert_eq!(rust_outcome("keygen", &args), go, "{label}");
        assert_eq!(go["ok"]["consumed"], (2 + rejected) * POLY_BYTES, "{label}");
    }
}

#[test]
fn random_corpus_agrees_with_go() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let scale = env_count("PURSE_SNTRUP_CASES", 1);
    let plan = if heavy() {
        Plan {
            keys: 32 * scale,
            encaps_per_key: 4,
            random_inputs: 40 * scale,
        }
    } else {
        Plan {
            keys: 6 * scale,
            encaps_per_key: 2,
            random_inputs: 12 * scale,
        }
    };
    let mut rng = SplitMix64::from_env("random_corpus_agrees_with_go");
    let requests = corpus(&mut rng, plan);
    let tally = compare(&mut oracle, &requests);
    for (label, (n, valid)) in &tally {
        eprintln!("{label:>29}: {n:>5} requests, {valid:>5} valid decapsulations");
    }
    // Every category was reached, and the outcomes are the expected ones,
    // as Go itself reported them.
    let get = |l: &str| tally.get(l).copied().unwrap_or_default();
    assert_eq!(
        get("decapsulate").0,
        get("decapsulate").1,
        "every honest ciphertext decapsulates"
    );
    assert_eq!(get("decapsulate-wrong-key").1, 0);
    assert_eq!(get("decapsulate-random").1, 0);
    assert_eq!(get("decapsulate-random-ct").1, 0);
    assert_eq!(get("decapsulate-tampered").1, 0, "no tampering survives");
    // Each of these fails one check alone (see `compare`'s expected keys).
    for label in ["decapsulate-wrong-weight", "decapsulate-moved-coefficient"] {
        assert!(get(label).0 > 0, "no {label} request");
        assert_eq!(get(label).1, 0, "Go accepted a {label} ciphertext");
    }
    assert!(
        get("decapsulate-alias").0 > 0,
        "no ciphertext had a tail alias"
    );
    assert_eq!(
        get("decapsulate-alias").0,
        get("decapsulate-alias").1,
        "Go accepts every tail alias (QK-0740)"
    );
    assert!(get("keygen-short").0 > 0 && get("encapsulate-short").0 > 0);
    assert_eq!(get("keygen-retry").0, 2);
    assert!(get("decapsulate-bad-sk").0 > 0 && get("encapsulate-bad-pk").0 > 0);
}

#[test]
fn many_keygen_seeds_agree_with_go() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let n = env_count("PURSE_SNTRUP_CASES", 1) * if heavy() { 200 } else { 12 };
    let mut rng = SplitMix64::from_env("many_keygen_seeds_agree_with_go");
    let mut attempts: BTreeMap<usize, usize> = BTreeMap::new();
    for _ in 0..n {
        let args =
            purse_testutil::serde_json::json!({ "rand": hex(&rng.bytes(12 * common::POLY_BYTES)) });
        let go = go_outcome(&mut oracle, "keygen", &args);
        assert_eq!(rust_outcome("keygen", &args), go, "args: {args}");
        let consumed = go["ok"]["consumed"].as_u64().expect("consumed") as usize;
        // consumed = 3044·(g candidates + 1 for f).
        assert_eq!(consumed % common::POLY_BYTES, 0);
        *attempts
            .entry(consumed / common::POLY_BYTES - 1)
            .or_default() += 1;
    }
    eprintln!("g candidates drawn per key: {attempts:?}");
}

#[test]
fn frozen_vectors_agree_with_go() {
    let Some(mut oracle) = oracle() else {
        return;
    };
    let (_, entries) = read_vectors();
    let recorded: Vec<Value> = entries.iter().map(|e| e.outcome.clone()).collect();
    for (i, e) in entries.iter().enumerate() {
        let args = resolve(&e.args, &recorded[..i]);
        assert_eq!(
            go_outcome(&mut oracle, &e.cmd, &args),
            e.outcome,
            "frozen entry {i} ({}): Go no longer gives the recorded outcome",
            e.cmd
        );
    }
}

#[test]
#[ignore = "writes tests/data/sntrup_vectors.txt: PURSE_SNTRUP_REGEN=1 ... -- --ignored --exact regenerate_frozen_vectors"]
fn regenerate_frozen_vectors() {
    assert!(
        std::env::var_os("PURSE_SNTRUP_REGEN").is_some(),
        "set PURSE_SNTRUP_REGEN=1 to overwrite the frozen vectors"
    );
    let mut oracle = oracle().expect("the oracle is required to regenerate");
    let requests = corpus(&mut SplitMix64(FROZEN_SEED), FROZEN_PLAN);
    let mut outcomes: Vec<Value> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    for r in &requests {
        let args = resolve(&r.args, &outcomes);
        let outcome = go_outcome(&mut oracle, &r.cmd, &args);
        assert_eq!(
            rust_outcome(&r.cmd, &args),
            outcome,
            "{} ({})",
            r.cmd,
            r.label
        );
        outcomes.push(outcome.clone());
        entries.push(Entry {
            cmd: r.cmd.clone(),
            args: r.args.clone(),
            outcome,
        });
    }
    // The release only: a distribution's build may append its own tag.
    let go = oracle.go_version();
    let go = go.split(['-', ' ']).next().unwrap_or_default().to_owned();
    let header = vec![
        "Frozen vectors for crates/sntrup4591761".to_owned(),
        String::new(),
        format!(
            "Every outcome is the output of the Go oracle tools/oracle/cmd/sntrup, built with {go},"
        ),
        format!("linking {SNTRUP_MODULE} {SNTRUP_VERSION},"),
        format!("the version dcrwallet {DCRWALLET_PARITY_COMMIT} links."),
        "Regenerate with:".to_owned(),
        "  PURSE_SNTRUP_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test --release -p sntrup4591761 \\".to_owned(),
        "      --test oracle -- --ignored --exact regenerate_frozen_vectors".to_owned(),
        "which also checks the Rust crate agrees. Do not edit by hand. Regeneration reproduces".to_owned(),
        "this file byte for byte: every request is deterministic.".to_owned(),
        format!(
            "Inputs: tests/common/mod.rs corpus with SplitMix64({FROZEN_SEED:#x}) and FROZEN_PLAN; requests"
        ),
        "that use an earlier answer take it from the Rust crate. Each line is {cmd, args, outcome}:".to_owned(),
        "the request in compact form and Go's outcome, {\"ok\": result} or {\"err\": {text, kind}},".to_owned(),
        "in the shapes tools/oracle/cmd/sntrup/main.go documents. tests/common/mod.rs resolve expands".to_owned(),
        "the compact form into what was sent: rand_seed/rand_len are that many bytes of".to_owned(),
        "SplitMix64(rand_seed) appended to rand, and sk_ref/pk_ref the key pair recorded by that".to_owned(),
        "earlier keygen line (0-based, counting entry lines only).".to_owned(),
        String::new(),
    ];
    write_vectors(&header, &entries);
    eprintln!("wrote {} entries", entries.len());
}
