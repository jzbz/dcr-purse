// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against Go: `tools/oracle/cmd/mixsolver`.
//!
//! - `client_coefficients_are_the_negated_product` needs only Go: it asks
//!   `mixing.Coefficients` (the client's construction of the solver input)
//!   for the coefficients of chosen roots' power sums and requires
//!   −∏(x − rᵢ), the form every property test feeds the solver.
//! - The rest need the oracle **built with FLINT** (`go build -tags flint`,
//!   see `tools/oracle/cmd/mixsolver/main.go`): dcrwallet's solver,
//!   `decred.org/cspp/v2@v2.4.0`, under the client's acceptance rule,
//!   compared with [`purse_mixsolver::roots`] by the rule in
//!   `tests/flint_cases/mod.rs`. The plain build of the oracle (what the
//!   harness builds) answers "no-flint" and these tests skip with a notice,
//!   unless `PURSE_REQUIRE_FLINT` is set, which makes the skip a failure.
//!   Point `PURSE_ORACLE_DIR` at a directory holding the tagged
//!   `purse-oracle-mixsolver` and the reference solver program `csppsolver`
//!   (both built in tools/oracle, see main.go there): `agrees_with_flint`
//!   then runs every case both through the solver package in process and
//!   through `solverrpc.Roots` and `csppsolver`, the client's own path, and
//!   requires the two to be identical.
//! - `regenerate_flint_vectors` (ignored) writes the frozen vectors
//!   `tests/data/flint_vectors.jsonl` that `tests/frozen.rs` replays without
//!   Go:
//!
//!   ```text
//!   PURSE_ORACLE_DIR=<dir> PURSE_REQUIRE_ORACLE=1 PURSE_REQUIRE_FLINT=1 \
//!   PURSE_MIXSOLVER_REGEN=1 cargo test -p purse-mixsolver --test oracle -- \
//!       --ignored --exact regenerate_flint_vectors
//!   ```
//!
//! `PURSE_MIXSOLVER_FLINT_CASES` multiplies the random instances of the
//! live differential (default 1, about 200 cases; degrees 200 to 1024 are
//! added in an optimised build or under `PURSE_MIXSOLVER_HEAVY`).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

mod common;
mod flint_cases;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use common::model::{self, P};
use common::{Lead, distinct_roots, env_count, heavy, hex128, parse_hex128, polynomial, sorted};
use dcroxide_crypto::blake256;
use flint_cases::{
    CATEGORIES, Case, FROZEN_PATH, FROZEN_PLAN, FROZEN_SEED, GoOutcome, Plan, REQUIRE_FLINT, Tally,
    Verdict, cases, check, coeffs_json, flint_required, frozen_line,
};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip};

/// The mixsolver oracle with FLINT, its `flint.version` answer and the
/// reference solver program for the RPC path (when there is one), or
/// `None` (skip) when there is no Go or the oracle was built without FLINT;
/// a failure instead under `PURSE_REQUIRE_FLINT`, which also requires the
/// solver program.
fn flint_oracle() -> Option<(Oracle, Value, Option<String>)> {
    let Some(mut oracle) = oracle_or_skip("mixsolver") else {
        assert!(
            !flint_required(),
            "{REQUIRE_FLINT} is set but there is no mixsolver oracle (no Go toolchain)"
        );
        return None;
    };
    let version = match oracle.try_call("flint.version", Value::Null) {
        Ok(version) => version,
        Err(e) if e.kind.as_deref() == Some("no-flint") => {
            assert!(
                !flint_required(),
                "{REQUIRE_FLINT} is set but the mixsolver oracle answered: {}. Build it with \
                 `go build -tags flint -o $DIR/purse-oracle-mixsolver ./cmd/mixsolver` in \
                 tools/oracle and set PURSE_ORACLE_DIR=$DIR",
                e.text
            );
            eprintln!(
                "skipping: {} (set {REQUIRE_FLINT} to make this an error)",
                e.text
            );
            return None;
        }
        Err(e) => panic!("mixsolver oracle: flint.version failed: {e:?}"),
    };
    eprintln!("mixsolver oracle: FLINT {version}");
    let solver = std::env::var_os("PURSE_ORACLE_DIR")
        .map(|dir| PathBuf::from(dir).join(format!("csppsolver{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.is_file());
    match &solver {
        Some(p) => eprintln!("reference solver program: {}", p.display()),
        None => {
            assert!(
                !flint_required(),
                "{REQUIRE_FLINT} is set but there is no csppsolver in PURSE_ORACLE_DIR: build it \
                 with `go build -o $DIR/csppsolver decred.org/cspp/v2/cmd/csppsolver` in \
                 tools/oracle"
            );
            eprintln!("NOTICE: no csppsolver in PURSE_ORACLE_DIR; the RPC path is not compared");
        }
    }
    Some((
        oracle,
        version,
        solver.map(|p| p.to_string_lossy().into_owned()),
    ))
}

/// Go's outcome for `coeffs`: in process, and, with a solver program, also
/// through solverrpc, which must give the identical outcome.
fn go_outcome(oracle: &mut Oracle, solver: Option<&str>, case: &Case) -> GoOutcome {
    let args = coeffs_json(&case.coeffs);
    let direct = GoOutcome::from_json(&oracle.call("roots", args.clone()));
    if let Some(solver) = solver {
        let mut args = args;
        args["solver"] = json!(solver);
        let via_rpc = GoOutcome::from_json(&oracle.call("solverrpc.roots", args));
        assert_eq!(
            via_rpc,
            direct,
            "{} (degree {}): solverrpc and csppsolver answer otherwise than the solver package",
            case.category,
            case.coeffs.len().saturating_sub(1)
        );
    }
    direct
}

#[test]
fn client_coefficients_are_the_negated_product() {
    // Runs with either build of the oracle: "coefficients" is pure Go.
    let Some(mut oracle) = oracle_or_skip("mixsolver") else {
        return;
    };
    let mut rng = SplitMix64::from_env("client_coefficients_are_the_negated_product");
    for n in [1usize, 2, 3, 5, 16, 64, 200] {
        let r = distinct_roots(&mut rng, n, &[0, P - 1]);
        // S_k = Σ r^k for k = 1..n, the column sums of the SR vectors
        // (spec 12 §5.9.2; SRMix uses exponents 1..mtot, dcnet.go:48-57).
        let sums: Vec<String> = (1..=n as u128)
            .map(|k| {
                let s = r
                    .iter()
                    .fold(0, |acc, &x| model::add_mod(acc, model::pow_mod(x, k)));
                hex128(s)
            })
            .collect();
        let got = oracle.call("coefficients", json!({ "power_sums": sums }));
        let got: Vec<u128> = got
            .as_array()
            .expect("array")
            .iter()
            .map(|s| parse_hex128(s.as_str().expect("hex")))
            .collect();
        assert_eq!(got, polynomial(&mut rng, &r, Lead::Client), "n = {n}");
        assert_eq!(got.last(), Some(&(P - 1)), "leading coefficient F − 1");
        assert_eq!(purse_mixsolver::roots(&got), Ok(sorted(&r)));
    }
}

#[test]
fn agrees_with_flint() {
    let Some((mut oracle, _, solver)) = flint_oracle() else {
        return;
    };
    let plan = Plan {
        scale: env_count("PURSE_MIXSOLVER_FLINT_CASES", 1),
        heavy: heavy(),
    };
    let mut rng = SplitMix64::from_env("agrees_with_flint");
    let all = cases(&mut rng, plan);
    let start = Instant::now();
    let mut tally = Tally::default();
    // FLINT's own order of the values it returns, on accepted inputs of
    // degree two or more.
    let (mut ascending, mut descending, mut other) = (0usize, 0usize, 0usize);
    for case in &all {
        let go = go_outcome(&mut oracle, solver.as_deref(), case);
        let checked = check(case, &go);
        tally.add(case, &go, &checked);
        if go.accepted() && go.flint_order.len() >= 2 {
            let w = go.flint_order.windows(2);
            if w.clone().all(|w| w[0] < w[1]) {
                ascending += 1;
            } else if w.clone().all(|w| w[0] > w[1]) {
                descending += 1;
            } else {
                other += 1;
            }
        }
    }
    for category in CATEGORIES.iter().map(|&(c, _)| c) {
        let present = all.iter().any(|c| c.category == category);
        assert!(present || category == "max-degree", "no {category} cases");
    }
    eprintln!(
        "{} cases ({plan:?}) in {:.1?}, {}: {} accepted alike, {} refused alike, {} documented \
         divergences",
        all.len(),
        start.elapsed(),
        if solver.is_some() {
            "each also through solverrpc and csppsolver, identically"
        } else {
            "in process only"
        },
        tally.count(Verdict::SameAccept),
        tally.count(Verdict::SameReject),
        tally.count(Verdict::Divergence),
    );
    for line in tally.lines() {
        eprintln!("  {line}");
    }
    eprintln!(
        "FLINT's order on accepted inputs of degree >= 2: {ascending} ascending, {descending} \
         descending, {other} neither (the client sorts; the crate returns ascending)"
    );
}

#[test]
fn flint_reports_one_value_per_factor() {
    // RootFactors reports −(constant coefficient) of every monic factor,
    // whatever its degree, with its exponent (solver.go:83-100): the count
    // rule rests on it.
    let Some((mut oracle, _, _)) = flint_oracle() else {
        return;
    };
    let mut rng = SplitMix64::from_env("flint_reports_one_value_per_factor");
    let r = distinct_roots(&mut rng, 2, &[]);
    // (x² − a)(x − r0): a is a non-residue; the quadratic's "root" is a.
    let a = loop {
        let a = common::random_nonzero(&mut rng);
        if !model::is_square(a) {
            break a;
        }
    };
    let factors = |oracle: &mut Oracle, c: &[u128]| {
        let got = oracle.call("root-factors", coeffs_json(c));
        let mut pairs: Vec<(u128, u64)> = got["roots"]
            .as_array()
            .expect("roots")
            .iter()
            .zip(got["exps"].as_array().expect("exps"))
            .map(|(v, e)| {
                (
                    parse_hex128(v.as_str().expect("hex")),
                    e.as_u64().expect("exp"),
                )
            })
            .collect();
        pairs.sort_unstable();
        pairs
    };
    let c = model::poly_mul(&[model::neg_mod(a), 0, 1], &model::expand(&r[..1]));
    assert_eq!(factors(&mut oracle, &c), sorted_pairs(&[(a, 1), (r[0], 1)]));
    // (x − r0)²(x − r1): r0 with exponent 2.
    let c = polynomial(&mut rng, &[r[0], r[0], r[1]], Lead::Client);
    assert_eq!(
        factors(&mut oracle, &c),
        sorted_pairs(&[(r[0], 2), (r[1], 1)])
    );
    // The zero polynomial and a nonzero constant: no factors at all.
    assert_eq!(factors(&mut oracle, &[0, 0]), []);
    assert_eq!(factors(&mut oracle, &[7, 0, 0]), []);
}

fn sorted_pairs(p: &[(u128, u64)]) -> Vec<(u128, u64)> {
    let mut v = p.to_vec();
    v.sort_unstable();
    v
}

#[test]
#[ignore = "writes tests/data/flint_vectors.jsonl; needs the oracle built with FLINT"]
fn regenerate_flint_vectors() {
    assert!(
        std::env::var_os("PURSE_MIXSOLVER_REGEN").is_some(),
        "set PURSE_MIXSOLVER_REGEN=1 to rewrite {FROZEN_PATH}"
    );
    assert!(
        flint_required(),
        "set {REQUIRE_FLINT}=1: the vectors must come from FLINT and through csppsolver"
    );
    let Some((mut oracle, version, solver)) = flint_oracle() else {
        panic!("regenerating the FLINT vectors needs the oracle built with FLINT");
    };
    let info = oracle.build_info();
    let cspp = info
        .deps
        .iter()
        .find(|m| m.path == "decred.org/cspp/v2")
        .expect("the oracle links decred.org/cspp/v2");
    let all: Vec<Case> = cases(&mut SplitMix64(FROZEN_SEED), FROZEN_PLAN);
    let mut lines = Vec::with_capacity(all.len());
    let mut tally = Tally::default();
    let mut order_dependent = 0usize;
    for case in &all {
        let outcome = go_outcome(&mut oracle, solver.as_deref(), case);
        let checked = check(case, &outcome);
        tally.add(case, &outcome, &checked);
        order_dependent += usize::from(outcome.flint_order.len() >= 2);
        lines.push(frozen_line(case, &outcome));
    }
    let body = lines.join("\n");
    let mut text = String::new();
    let mut put = |l: &str| {
        text.push_str(l);
        text.push('\n');
    };
    put("# FLINT vectors for purse-mixsolver: dcrwallet's mixing root solver on fixed inputs.");
    put("#");
    put("# Generated by regenerate_flint_vectors (tests/oracle.rs) from");
    put("# tools/oracle/cmd/mixsolver built with -tags flint:");
    put(&format!(
        "#   {} {}, FLINT {} (__FLINT_RELEASE {}), {},",
        cspp.path,
        cspp.version,
        version["version"].as_str().unwrap_or("?"),
        version["release"],
        info.go
    ));
    put(&format!("#   dcrwallet {DCRWALLET_PARITY_COMMIT}."));
    put("# Every \"go\" value is solver.Roots(coeffs, mixing.F) in process under the");
    put("# client's rule (mixclient/client.go:2162-2187: \"roots\" sorted as the client");
    put("# sorts them, \"fp_roots\" the bytes its factored-polynomial message carries);");
    put("# each was also obtained through solverrpc.Roots and the csppsolver program,");
    put("# the client's own path, and was identical.");
    put(&format!(
        "# Inputs: tests/flint_cases/mod.rs `cases` with FROZEN_PLAN from SplitMix64 seed {FROZEN_SEED:#x};"
    ));
    put("# the replay (tests/frozen.rs) regenerates them and requires these, in this order.");
    put("#");
    put("# Reproducibility on regeneration: every field but \"flint_order\" is fixed by");
    put("# the mathematics (the roots, the client's bytes, the count of factors, the");
    put("# class and error text), so any FLINT that factors correctly reproduces it.");
    put("# \"flint_order\" is FLINT's factor order: with one value or none it is fixed");
    put(&format!(
        "# too; on the {order_dependent} entries with two or more it is FLINT's choice, reproduced"
    ));
    put("# byte for byte by regenerating with the FLINT named above (checked), not");
    put("# necessarily by another release. The replay asserts of it only that it is a");
    put("# permutation of the roots, or as long as the count.");
    put("# Do not edit by hand. Regenerate:");
    put(
        "#   PURSE_ORACLE_DIR=<dir> PURSE_REQUIRE_ORACLE=1 PURSE_REQUIRE_FLINT=1 PURSE_MIXSOLVER_REGEN=1 \\",
    );
    put(
        "#   cargo test -p purse-mixsolver --test oracle -- --ignored --exact regenerate_flint_vectors",
    );
    put("# (<dir>: the tagged purse-oracle-mixsolver and csppsolver, tools/oracle/cmd/mixsolver).");
    for line in tally.lines() {
        put(&format!("# count {line}"));
    }
    let _ = writeln!(text, "# entries: {}", lines.len());
    let _ = writeln!(
        text,
        "# body-blake256: {}",
        hex(&blake256::sum256(body.as_bytes()))
    );
    text.push_str(&body);
    text.push('\n');
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FROZEN_PATH);
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!(
        "wrote {} entries to {FROZEN_PATH} ({order_dependent} with FLINT's order of two or more values)",
        lines.len()
    );
}
