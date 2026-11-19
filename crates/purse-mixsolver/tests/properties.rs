// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests of [`purse_mixsolver::roots`] through the public API,
//! needing no reference implementation.
//!
//! **The success path is checked completely.** A polynomial of degree n
//! over a field has at most n roots, so n distinct values that each make
//! it zero are its whole root set. Every test here builds lead·∏(x − rᵢ)
//! from n chosen distinct roots with the independent model in
//! `tests/common/model.rs`, and requires exactly those roots back, in
//! ascending order; every `Ok` answer is also certified by evaluation and
//! counting on its own.
//!
//! **Failures** are checked by cause: each malformed or non-splitting
//! input returns its specific [`Error`]. That Go's FLINT solver rejects the
//! same inputs, and what the client does with each rejection, is checked
//! against FLINT by `agrees_with_flint` in `tests/oracle.rs` and, without
//! Go, by the frozen FLINT vectors (`tests/frozen.rs`).
//!
//! `PURSE_MIXSOLVER_CASES` multiplies the number of random instances
//! (default 1); `PURSE_TEST_SEED` picks the seed. Degrees above 128 run in
//! an optimised build, or in a debug build under `PURSE_MIXSOLVER_HEAVY`
//! (`common::heavy`).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

mod common;

use std::time::{Duration, Instant};

use common::model::{self, P};
use common::{
    Lead, distinct_roots, env_count, heavy, polynomial, random_fe, random_nonzero, sorted,
};
use purse_mixsolver::{Error, FIELD_PRIME, MAX_DEGREE, roots};
use purse_testutil::SplitMix64;

/// Certifies an answer on its own: `n` values, strictly ascending, each
/// canonical and a root of `coeffs`. By the degree bound this is the
/// complete root set.
fn certify(coeffs: &[u128], got: &[u128]) {
    assert_eq!(got.len(), coeffs.len() - 1, "root count");
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "not strictly ascending"
    );
    for &r in got {
        assert!(r < P, "non-canonical root {r:#x}");
        assert_eq!(model::eval(coeffs, r), 0, "{r:#x} is not a root");
    }
}

/// Solves lead·∏(x − r) and requires exactly `r` back.
fn check_roots(rng: &mut SplitMix64, r: &[u128], lead: Lead) {
    let coeffs = polynomial(rng, r, lead);
    let got = roots(&coeffs).unwrap_or_else(|e| {
        panic!("degree {} ({lead:?}) refused: {e}", r.len());
    });
    certify(&coeffs, &got);
    assert_eq!(got, sorted(r), "degree {} ({lead:?})", r.len());
}

const LEADS: [Lead; 3] = [Lead::Client, Lead::One, Lead::Random];

#[test]
fn recovers_random_root_sets() {
    let scale = env_count("PURSE_MIXSOLVER_CASES", 1);
    let mut rng = SplitMix64::from_env("recovers_random_root_sets");
    let degrees = [
        1usize, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 31, 32, 33, 63, 64, 100, 127, 128,
        200, 255, 256,
    ];
    for &n in degrees.iter().filter(|&&n| n <= 128 || heavy()) {
        // More instances at small degrees, where they are cheap and where
        // the splitting has the most distinct paths.
        let cases = scale
            * if n <= 16 {
                24
            } else if n <= 64 {
                6
            } else {
                2
            };
        for i in 0..cases {
            let r = distinct_roots(&mut rng, n, &[]);
            check_roots(&mut rng, &r, LEADS[i % LEADS.len()]);
        }
    }
}

#[test]
fn recovers_edge_roots() {
    let mut rng = SplitMix64::from_env("recovers_edge_roots");
    let k = random_fe(&mut rng) & !1;
    let edge_sets: [&[u128]; 9] = [
        &[0],
        &[P - 1],
        &[0, P - 1],
        &[0, 1],
        &[P - 2, P - 1],
        &[k, k + 1],
        &[0, 1, 2, 3, 4, 5, 6, 7],
        &[P - 1, P - 2, P - 3, P - 4, 0, 1, 2],
        &[1 << 126, (1 << 126) - 1, 1 << 64, (1 << 64) - 1],
    ];
    for set in edge_sets {
        for n in [set.len(), set.len() + 1, set.len() + 9] {
            let r = distinct_roots(&mut rng, n, set);
            for lead in LEADS {
                check_roots(&mut rng, &r, lead);
            }
        }
    }
    // A long run of adjacent roots.
    let run: Vec<u128> = (0..40).map(|i| k + i).collect();
    check_roots(&mut rng, &run, Lead::Client);
}

#[test]
fn recovers_at_512_and_at_the_largest_degree() {
    if !heavy() {
        eprintln!("skipping degrees 512 and 1024 in a debug build (set PURSE_MIXSOLVER_HEAVY)");
        return;
    }
    let mut rng = SplitMix64::from_env("recovers_at_512_and_at_the_largest_degree");
    let r = distinct_roots(&mut rng, 512, &[0, P - 1]);
    check_roots(&mut rng, &r, Lead::Client);
    let r = distinct_roots(&mut rng, MAX_DEGREE, &[0, 1, P - 2, P - 1]);
    check_roots(&mut rng, &r, Lead::Client);
}

#[test]
fn probe_input_of_dcrwallet() {
    // testsolver.go:25-29 calls Roots([1, 2], F): 1 + 2x has the one root
    // −1/2 = −2¹²⁶ = 2¹²⁶ − 1.
    let got = roots(&[1, 2]).expect("degree one always solves");
    assert_eq!(got, vec![(1u128 << 126) - 1]);
    certify(&[1, 2], &got);
}

#[test]
fn every_degree_one_polynomial_solves() {
    let mut rng = SplitMix64::from_env("every_degree_one_polynomial_solves");
    for _ in 0..200 {
        let c = [random_fe(&mut rng), random_nonzero(&mut rng)];
        certify(&c, &roots(&c).expect("degree one"));
    }
}

#[test]
fn repeated_roots_are_refused() {
    let mut rng = SplitMix64::from_env("repeated_roots_are_refused");
    for n in [2usize, 3, 4, 5, 8, 17, 64, 200] {
        for lead in LEADS {
            let mut r = distinct_roots(&mut rng, n - 1, &[]);
            let dup = r[rng.below(r.len() as u64) as usize];
            r.push(dup);
            let c = polynomial(&mut rng, &r, lead);
            assert_eq!(roots(&c), Err(Error::NotSplit), "n={n} {lead:?}");
        }
    }
    // A root of multiplicity three, and 0 repeated.
    let c = polynomial(&mut rng, &[7, 7, 7, 1], Lead::Client);
    assert_eq!(roots(&c), Err(Error::NotSplit));
    let c = polynomial(&mut rng, &[0, 0, 5], Lead::One);
    assert_eq!(roots(&c), Err(Error::NotSplit));
}

/// A random quadratic non-residue.
fn non_residue(rng: &mut SplitMix64) -> u128 {
    loop {
        let a = random_nonzero(rng);
        if !model::is_square(a) {
            return a;
        }
    }
}

/// A random non-cube (P ≡ 1 mod 3, so a third of the nonzero residues are
/// cubes).
fn non_cube(rng: &mut SplitMix64) -> u128 {
    loop {
        let a = random_nonzero(rng);
        if model::pow_mod(a, (P - 1) / 3) != 1 {
            return a;
        }
    }
}

#[test]
fn irreducible_factors_are_refused() {
    let mut rng = SplitMix64::from_env("irreducible_factors_are_refused");
    assert_eq!(P % 3, 1);
    // x² + 1 (−1 is a non-residue since P ≡ 3 mod 4).
    assert_eq!(roots(&[1, 0, 1]), Err(Error::NotSplit));
    assert_eq!(roots(&[P - 1, 0, P - 1]), Err(Error::NotSplit));
    for extra in [0usize, 1, 2, 5, 30, 100] {
        let a = non_residue(&mut rng);
        // x² − a is irreducible; times `extra` distinct linear factors.
        let quad = [model::neg_mod(a), 0, 1];
        let lin = model::expand(&distinct_roots(&mut rng, extra, &[0]));
        let c = model::scale(&model::poly_mul(&quad, &lin), random_nonzero(&mut rng));
        assert_eq!(roots(&c), Err(Error::NotSplit), "quadratic, {extra} linear");
        // x³ − b with b a non-cube is irreducible.
        let b = non_cube(&mut rng);
        let cubic = [model::neg_mod(b), 0, 0, 1];
        let c = model::scale(&model::poly_mul(&cubic, &lin), P - 1);
        assert_eq!(roots(&c), Err(Error::NotSplit), "cubic, {extra} linear");
    }
    // Two irreducible quadratics and nothing else.
    let (a, b) = (non_residue(&mut rng), non_residue(&mut rng));
    let c = model::poly_mul(&[model::neg_mod(a), 0, 1], &[model::neg_mod(b), 0, 1]);
    assert_eq!(roots(&c), Err(Error::NotSplit));
}

#[test]
fn random_polynomials_almost_never_split() {
    // A random polynomial of degree n with a fixed leading coefficient has
    // n distinct roots with probability C(F, n)/F^n, about 1/n!: at degree
    // 8 that is 2.5·10⁻⁵, so one seed in about 2,000 draws a degree-8
    // polynomial that splits, and the solver must then solve it. So every
    // Ok is certified, at most one degree-8 split is tolerated (two or more
    // in 20 draws: about 10⁻⁷), and from degree 16 on a split is a failure
    // (20 draws: about 10⁻¹²). The draws are those of the original test,
    // so the seeds on which it failed (360, 800, 1375, 4108 and 5040 among
    // the first 6000) replay the same polynomials.
    let mut rng = SplitMix64::from_env("random_polynomials_almost_never_split");
    let mut ok = 0;
    let mut ok_at_8 = 0;
    for n in [2usize, 3, 4, 8, 16, 64, 128] {
        for _ in 0..20 {
            let mut c: Vec<u128> = (0..n).map(|_| random_fe(&mut rng)).collect();
            c.push(P - 1);
            match roots(&c) {
                Ok(got) => {
                    certify(&c, &got);
                    ok += 1;
                    ok_at_8 += usize::from(n == 8);
                    assert!(n <= 8, "a random degree-{n} polynomial split");
                }
                Err(e) => assert_eq!(e, Error::NotSplit, "degree {n}"),
            }
        }
    }
    assert!(
        ok_at_8 <= 1,
        "{ok_at_8} of 20 random degree-8 polynomials split"
    );
    eprintln!("{ok} random polynomials split (certified), {ok_at_8} of them of degree 8");
}

#[test]
fn split_rate_matches_the_mathematics() {
    // The acceptance rate on random input is a property of the field, not
    // of the solver: C(F, n)/F^n, which is 1/n! to within n²/F. Within five
    // standard deviations (a false failure about once in 10⁶ seeds per
    // degree), the solver accepts that often and refuses the rest with
    // NotSplit, and every acceptance certifies.
    let mut rng = SplitMix64::from_env("split_rate_matches_the_mathematics");
    let scale = env_count("PURSE_MIXSOLVER_CASES", 1);
    for (n, draws, n_factorial) in [(2usize, 400usize, 2.0f64), (3, 600, 6.0), (4, 1200, 24.0)] {
        let draws = draws * scale;
        let mut split = 0usize;
        for i in 0..draws {
            let mut c: Vec<u128> = (0..n).map(|_| random_fe(&mut rng)).collect();
            c.push(if i % 2 == 0 {
                P - 1
            } else {
                random_nonzero(&mut rng)
            });
            match roots(&c) {
                Ok(got) => {
                    certify(&c, &got);
                    split += 1;
                }
                Err(e) => assert_eq!(e, Error::NotSplit, "degree {n}"),
            }
        }
        let p = 1.0 / n_factorial;
        let mean = draws as f64 * p;
        let sd = (mean * (1.0 - p)).sqrt();
        eprintln!("degree {n}: {split} of {draws} split (expected {mean:.1} ± {sd:.1})");
        assert!(
            (split as f64 - mean).abs() <= 5.0 * sd + 1.0,
            "degree {n}: {split} of {draws} split, expected {mean:.1} ± {sd:.1}"
        );
    }
}

#[test]
fn malformed_inputs_get_their_specific_errors() {
    let f = FIELD_PRIME;
    assert_eq!(roots(&[]), Err(Error::TooFewCoefficients { len: 0 }));
    assert_eq!(roots(&[5]), Err(Error::TooFewCoefficients { len: 1 }));
    assert_eq!(roots(&[0]), Err(Error::TooFewCoefficients { len: 1 }));
    // The zero polynomial and zero leading coefficients.
    assert_eq!(roots(&[0, 0]), Err(Error::ZeroLeadingCoefficient));
    assert_eq!(roots(&[0; 9]), Err(Error::ZeroLeadingCoefficient));
    assert_eq!(
        roots(&[f - 6, 5, f - 1, 0]),
        Err(Error::ZeroLeadingCoefficient)
    );
    assert_eq!(roots(&[3, 0]), Err(Error::ZeroLeadingCoefficient));
    // Non-canonical coefficients, anywhere; the first is reported.
    for bad in [f, f + 1, 1 << 127, u128::MAX] {
        assert_eq!(roots(&[bad, 1]), Err(Error::NonCanonical { index: 0 }));
        assert_eq!(roots(&[1, bad]), Err(Error::NonCanonical { index: 1 }));
        assert_eq!(
            roots(&[1, 2, bad, bad, 1]),
            Err(Error::NonCanonical { index: 2 })
        );
        // Before the leading-coefficient check.
        assert_eq!(roots(&[bad, 0]), Err(Error::NonCanonical { index: 0 }));
    }
    // F + 6 ≡ 6: the same polynomial as one that splits, still refused.
    assert_eq!(roots(&[f - 6, 5, f - 1]), Ok(vec![2, 3]));
    assert_eq!(
        roots(&[f - 6, f + 5, f - 1]),
        Err(Error::NonCanonical { index: 1 })
    );
}

#[test]
fn degree_limit_is_enforced_before_any_work() {
    let one_more = vec![1u128; MAX_DEGREE + 2];
    assert_eq!(
        roots(&one_more),
        Err(Error::DegreeTooLarge {
            degree: MAX_DEGREE + 1
        })
    );
    // Length is checked first, before canonical form.
    let mut bad = one_more.clone();
    bad[0] = u128::MAX;
    assert_eq!(
        roots(&bad),
        Err(Error::DegreeTooLarge {
            degree: MAX_DEGREE + 1
        })
    );
    let huge = vec![u128::MAX; 1 << 20];
    let start = Instant::now();
    assert_eq!(
        roots(&huge),
        Err(Error::DegreeTooLarge {
            degree: (1 << 20) - 1
        })
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    // Exactly MAX_DEGREE passes the length check: the later checks answer.
    let mut at_limit = vec![1u128; MAX_DEGREE + 1];
    at_limit[MAX_DEGREE] = u128::MAX;
    assert_eq!(
        roots(&at_limit),
        Err(Error::NonCanonical { index: MAX_DEGREE })
    );
    at_limit[MAX_DEGREE] = 0;
    assert_eq!(roots(&at_limit), Err(Error::ZeroLeadingCoefficient));
    if heavy() {
        // A full degree-1024 splitting test (this one does not split).
        assert_eq!(roots(&vec![1u128; MAX_DEGREE + 1]), Err(Error::NotSplit));
    }
}

#[test]
fn errors_display_and_compare() {
    let all = [
        Error::TooFewCoefficients { len: 1 },
        Error::DegreeTooLarge { degree: 1025 },
        Error::NonCanonical { index: 3 },
        Error::ZeroLeadingCoefficient,
        Error::NotSplit,
        Error::AttemptsExhausted { degree: 2 },
        Error::SelfCheckFailed,
    ];
    let texts: Vec<String> = all.iter().map(ToString::to_string).collect();
    for (i, t) in texts.iter().enumerate() {
        assert!(!t.is_empty());
        assert!(texts[..i].iter().all(|u| u != t), "duplicate text {t:?}");
    }
    let boxed: Box<dyn std::error::Error> = Box::new(Error::NotSplit);
    assert!(boxed.to_string().contains("distinct linear factors"));
}
