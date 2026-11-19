// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Unit tests of the internals: the field and polynomial arithmetic against
//! the independent model in `tests/common/model.rs`, the δ derivation
//! against its documented preimage, the attempt counter and bound against
//! a prediction made from that documentation alone, and the determinism
//! self-vectors in `tests/data/self_vectors.txt`.

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use common::model::{self, P as MODEL_P};
use common::{Lead, distinct_roots, polynomial, random_fe, sorted};
use dcroxide_crypto::blake256;
use purse_testutil::{SplitMix64, hex};

use crate::field::{Acc, Fe, P, mul_wide};
use crate::poly::{Modulus, div_rem, eval, gcd, make_monic, mul, trim};
use crate::solve::{HALF, Stats, delta, delta_prefix, solve};
use crate::{Error, FIELD_PRIME, MAX_ATTEMPTS_PER_SPLIT, MAX_DEGREE, roots};

fn fe(v: u128) -> Fe {
    Fe::new(v).expect("canonical test value")
}

fn fes(v: &[u128]) -> Vec<Fe> {
    v.iter().map(|&x| fe(x)).collect()
}

fn vals(v: &[Fe]) -> Vec<u128> {
    v.iter().map(|x| x.get()).collect()
}

fn rand_u128(rng: &mut SplitMix64) -> u128 {
    (u128::from(rng.next_u64()) << 64) | u128::from(rng.next_u64())
}

/// Values at and around every boundary the arithmetic has: 0, 1, P − 1,
/// P, P + 1, 2¹²⁷, the 64-bit limb edges and 2¹²⁸ − 1.
const BOUNDARY: [u128; 16] = [
    0,
    1,
    2,
    (1 << 64) - 1,
    1 << 64,
    (1 << 64) + 1,
    (1 << 126) - 1,
    1 << 126,
    MODEL_P - 1,
    MODEL_P,
    MODEL_P + 1,
    1 << 127,
    (1 << 127) + 1,
    u128::MAX - 1,
    u128::MAX,
    0x8000_0000_0000_0000_7fff_ffff_ffff_ffff,
];

/// Canonical boundary values.
fn canonical_boundary() -> Vec<u128> {
    BOUNDARY.iter().copied().filter(|&v| v < MODEL_P).collect()
}

fn u256_of(hi: u128, lo: u128) -> model::U256 {
    let mut out = [0u32; 8];
    for i in 0..4 {
        out[i] = (lo >> (32 * i)) as u32;
        out[i + 4] = (hi >> (32 * i)) as u32;
    }
    out
}

// ---------------------------------------------------------------------------
// The field

#[test]
fn model_reductions_agree() {
    let mut rng = SplitMix64::from_env("model_reductions_agree");
    let mut values = canonical_boundary();
    values.extend((0..200).map(|_| random_fe(&mut rng)));
    for &a in &values {
        for &b in &values {
            assert_eq!(
                model::mul_mod(a, b),
                model::mul_mod_slow(a, b),
                "{a:#x} * {b:#x}"
            );
        }
    }
}

#[test]
fn constant_is_the_mersenne_prime() {
    assert_eq!(P, MODEL_P);
    assert_eq!(FIELD_PRIME, MODEL_P);
    assert_eq!(P, (1u128 << 127) - 1);
    assert_eq!(HALF, (MODEL_P - 1) / 2);
}

#[test]
fn mul_wide_matches_the_model() {
    let mut rng = SplitMix64::from_env("mul_wide_matches_the_model");
    let mut values = BOUNDARY.to_vec();
    values.extend((0..300).map(|_| rand_u128(&mut rng)));
    for &a in &values {
        for &b in &values {
            let (hi, lo) = mul_wide(a, b);
            assert_eq!(u256_of(hi, lo), model::mul_full(a, b), "{a:#x} * {b:#x}");
        }
    }
}

#[test]
fn reduce_matches_long_division_at_every_boundary() {
    let mut rng = SplitMix64::from_env("reduce_boundaries");
    let mut values = BOUNDARY.to_vec();
    values.extend((0..10_000).map(|_| rand_u128(&mut rng)));
    for &v in &values {
        let want = model::rem_slow(&model::widen(v), MODEL_P);
        assert_eq!(Fe::reduce(v).get(), want, "reduce({v:#x})");
        assert_eq!(
            Fe::new(v).map(Fe::get),
            (v < MODEL_P).then_some(v),
            "new({v:#x})"
        );
    }
    // The named cases of the task, explicitly.
    assert_eq!(Fe::reduce(0).get(), 0);
    assert_eq!(Fe::reduce(1).get(), 1);
    assert_eq!(Fe::reduce(MODEL_P - 1).get(), MODEL_P - 1);
    assert_eq!(Fe::reduce(MODEL_P).get(), 0);
    assert_eq!(Fe::reduce(MODEL_P + 1).get(), 1);
    assert_eq!(Fe::reduce(1 << 127).get(), 1);
    assert_eq!(Fe::reduce(u128::MAX).get(), 1);
    assert_eq!(Fe::reduce(u128::MAX - 1).get(), 0);
}

#[test]
fn field_operations_match_the_model() {
    let mut rng = SplitMix64::from_env("field_operations_match_the_model");
    let mut values = canonical_boundary();
    values.extend((0..150).map(|_| random_fe(&mut rng)));
    for &a in &values {
        assert_eq!(fe(a).neg().get(), model::neg_mod(a));
        for &b in &values {
            let (x, y) = (fe(a), fe(b));
            assert_eq!(x.add(y).get(), model::add_mod(a, b), "{a:#x} + {b:#x}");
            assert_eq!(x.sub(y).get(), model::sub_mod(a, b), "{a:#x} - {b:#x}");
            assert_eq!(x.mul(y).get(), model::mul_mod_slow(a, b), "{a:#x} * {b:#x}");
        }
    }
    for _ in 0..50 {
        let (a, e) = (random_fe(&mut rng), rand_u128(&mut rng));
        assert_eq!(fe(a).pow(e).get(), model::pow_mod(a, e));
    }
}

#[test]
fn field_identities() {
    let mut rng = SplitMix64::from_env("field_identities");
    let mut values = canonical_boundary();
    values.extend((0..200).map(|_| random_fe(&mut rng)));
    assert_eq!(Fe::ZERO.inv(), Fe::ZERO, "inv(0) is defined as 0");
    for w in values.windows(3) {
        let (a, b, c) = (fe(w[0]), fe(w[1]), fe(w[2]));
        if !b.is_zero() {
            assert_eq!(a.mul(b).mul(b.inv()), a, "(a·b)·b⁻¹");
            assert_eq!(b.mul(b.inv()), Fe::ONE);
            // Fermat: b^(P−1) = 1.
            assert_eq!(b.pow(MODEL_P - 1), Fe::ONE);
        }
        assert_eq!(a.pow(MODEL_P), a, "a^P = a");
        assert_eq!(a.mul(b.add(c)), a.mul(b).add(a.mul(c)), "distributivity");
        assert_eq!(a.add(b).sub(b), a);
        assert_eq!(a.add(a.neg()), Fe::ZERO);
        assert_eq!(a.mul(b), b.mul(a));
    }
    // P ≡ 3 (mod 4): −1 is not a square, so x² + 1 has no roots.
    assert_eq!(Fe::ONE.neg().pow(HALF), Fe::ONE.neg());
    // 2 is a square (P ≡ 7 mod 8), and 2⁻¹ = 2¹²⁶.
    assert_eq!(fe(2).pow(HALF), Fe::ONE);
    assert_eq!(fe(2).inv().get(), 1 << 126);
}

#[test]
fn accumulator_matches_the_model() {
    let mut rng = SplitMix64::from_env("accumulator_matches_the_model");
    for terms in [0usize, 1, 2, 3, 7, 64, 1000, 4100] {
        for pattern in 0..3 {
            let mut acc = Acc::default();
            let mut want = 0u128;
            for _ in 0..terms {
                let (a, b) = match pattern {
                    // The largest product, every time: the most carries.
                    0 => (MODEL_P - 1, MODEL_P - 1),
                    1 => (random_fe(&mut rng), random_fe(&mut rng)),
                    _ => (MODEL_P - 1, random_fe(&mut rng)),
                };
                acc.mul_add(fe(a), fe(b));
                want = model::add_mod(want, model::mul_mod(a, b));
            }
            assert_eq!(acc.finish().get(), want, "{terms} terms, pattern {pattern}");
            let mut doubled = acc;
            doubled.double();
            assert_eq!(doubled.finish().get(), model::add_mod(want, want));
            let seeded = {
                let mut s = Acc::from_fe(fe(MODEL_P - 1));
                s.mul_add(fe(MODEL_P - 1), fe(MODEL_P - 1));
                s.finish().get()
            };
            assert_eq!(
                seeded,
                model::add_mod(MODEL_P - 1, model::mul_mod(MODEL_P - 1, MODEL_P - 1))
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Polynomials

fn random_poly(rng: &mut SplitMix64, len: usize) -> Vec<u128> {
    (0..len).map(|_| random_fe(rng)).collect()
}

fn model_trim(mut v: Vec<u128>) -> Vec<u128> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

#[test]
fn polynomial_product_matches_the_model() {
    let mut rng = SplitMix64::from_env("polynomial_product_matches_the_model");
    for (la, lb) in [(0, 3), (1, 1), (1, 5), (4, 4), (7, 13), (40, 33)] {
        let (a, b) = (random_poly(&mut rng, la), random_poly(&mut rng, lb));
        assert_eq!(vals(&mul(&fes(&a), &fes(&b))), model::poly_mul(&a, &b));
    }
    let top = vec![MODEL_P - 1; 64];
    assert_eq!(
        vals(&mul(&fes(&top), &fes(&top))),
        model::poly_mul(&top, &top)
    );
}

#[test]
fn division_with_remainder() {
    let mut rng = SplitMix64::from_env("division_with_remainder");
    for (la, lb) in [
        (0, 1),
        (1, 1),
        (5, 1),
        (3, 5),
        (10, 4),
        (64, 31),
        (200, 100),
    ] {
        let a = random_poly(&mut rng, la);
        let mut b = random_poly(&mut rng, lb);
        if let Some(top) = b.last_mut() {
            *top = model::add_mod(*top, 1).max(1);
        }
        let (q, r) = div_rem(&fes(&a), &fes(&b)).expect("nonzero divisor");
        assert!(r.len() < b.len(), "deg r < deg b");
        assert!(r.last().is_none_or(|c| !c.is_zero()) && q.last().is_none_or(|c| !c.is_zero()));
        // a = q·b + r in the model.
        let qb = model::poly_mul(&vals(&q), &b);
        let mut sum = vec![0u128; qb.len().max(r.len())];
        for (i, &c) in qb.iter().enumerate() {
            sum[i] = model::add_mod(sum[i], c);
        }
        for (i, c) in r.iter().enumerate() {
            sum[i] = model::add_mod(sum[i], c.get());
        }
        assert_eq!(model_trim(sum), model_trim(a.clone()), "{la}/{lb}");
        // The remainder is unique, so it is also the model's.
        let mut dense = vals(&r);
        dense.resize(b.len() - 1, 0);
        assert_eq!(dense, model::poly_rem(&a, &b), "{la}/{lb} remainder");
    }
    assert!(div_rem(&fes(&[1, 2]), &[]).is_none());
    assert!(
        div_rem(&fes(&[1, 2]), &fes(&[1, 0])).is_none(),
        "unnormalized divisor"
    );
}

#[test]
fn gcd_of_products_with_known_roots() {
    let mut rng = SplitMix64::from_env("gcd_of_products_with_known_roots");
    for (nf, ng, nh) in [(0, 1, 1), (1, 0, 0), (3, 2, 5), (10, 10, 10), (40, 7, 1)] {
        let all = distinct_roots(&mut rng, nf + ng + nh, &[]);
        let (f, rest) = all.split_at(nf);
        let (g, h) = rest.split_at(ng);
        let (f, g, h) = (model::expand(f), model::expand(g), model::expand(h));
        let a = model::scale(&model::poly_mul(&f, &g), 5);
        let b = model::scale(&model::poly_mul(&f, &h), MODEL_P - 3);
        assert_eq!(vals(&gcd(&fes(&a), &fes(&b))), f, "gcd = f");
        assert_eq!(vals(&gcd(&fes(&a), &[])), {
            let mut m = fes(&a);
            make_monic(&mut m);
            vals(&m)
        });
    }
    assert!(gcd(&[], &[]).is_empty());
}

#[test]
fn horner_matches_term_by_term_evaluation() {
    let mut rng = SplitMix64::from_env("horner_matches_term_by_term_evaluation");
    for len in [0, 1, 2, 9, 100] {
        let a = random_poly(&mut rng, len);
        for x in canonical_boundary()
            .into_iter()
            .chain([random_fe(&mut rng)])
        {
            assert_eq!(eval(&fes(&a), fe(x)).get(), model::eval(&a, x));
        }
    }
}

/// `t mod g` in the model (`model::poly_rem`, which shares no code with the
/// crate), as a dense residue of `deg g` coefficients.
fn model_rem(t: &[u128], g: &[u128]) -> Vec<u128> {
    model::poly_rem(t, g)
}

#[test]
fn modular_arithmetic() {
    let mut rng = SplitMix64::from_env("modular_arithmetic");
    for d in [1usize, 2, 3, 8, 33, 64] {
        let mut g = random_poly(&mut rng, d);
        g.push(1);
        let m = Modulus::new(&fes(&g)).expect("monic");
        assert_eq!(m.degree(), d);
        for len in [0, 1, d, 2 * d + 3] {
            let t = random_poly(&mut rng, len);
            let want = if len == 0 {
                vec![0; d]
            } else {
                model_rem(&t, &g)
            };
            assert_eq!(vals(&m.reduce(&fes(&t))), want, "reduce d={d} len={len}");
        }
        let a = model_rem(&random_poly(&mut rng, d), &g);
        let delta = random_fe(&mut rng);
        let sq = model::poly_mul(&a, &a);
        assert_eq!(
            vals(&m.square(&fes(&a))),
            model_rem(&sq, &g),
            "square d={d}"
        );
        let sqb = model::poly_mul(&sq, &[delta, 1]);
        assert_eq!(
            vals(&m.square_mul_base(&fes(&a), fe(delta))),
            model_rem(&sqb, &g),
            "square·(x+δ) d={d}"
        );
        // (x + δ)^e by repeated multiplication in the model.
        let mut naive = vec![1u128];
        for e in 0u128..20 {
            assert_eq!(
                vals(&m.pow_base(fe(delta), e)),
                model_rem(&naive, &g),
                "e={e}"
            );
            naive = model::poly_mul(&naive, &[delta, 1]);
        }
    }
    assert!(Modulus::new(&fes(&[3, 2])).is_none(), "not monic");
    assert!(Modulus::new(&fes(&[1])).is_none(), "degree 0");
    assert!(Modulus::new(&[]).is_none());
}

#[test]
fn frobenius_tells_split_from_not() {
    let mut rng = SplitMix64::from_env("frobenius_tells_split_from_not");
    // x^P ≡ x modulo a product of distinct linear factors ...
    let g = model::expand(&distinct_roots(&mut rng, 12, &[0, MODEL_P - 1]));
    let m = Modulus::new(&fes(&g)).expect("monic");
    assert_eq!(vals(&m.pow_base(Fe::ZERO, MODEL_P)), model_rem(&[0, 1], &g));
    // ... and not modulo x² + 1 (irreducible) or (x − 5)².
    for g in [vec![1, 0, 1], model::expand(&[5, 5])] {
        let m = Modulus::new(&fes(&g)).expect("monic");
        assert_ne!(vals(&m.pow_base(Fe::ZERO, MODEL_P)), model_rem(&[0, 1], &g));
    }
}

#[test]
fn trim_and_monic() {
    let mut v = fes(&[1, 2, 0, 0]);
    trim(&mut v);
    assert_eq!(vals(&v), [1, 2]);
    make_monic(&mut v);
    assert_eq!(vals(&v), [model::pow_mod(2, MODEL_P - 2), 1]);
    let mut z = fes(&[0, 0]);
    trim(&mut z);
    assert!(z.is_empty());
}

// ---------------------------------------------------------------------------
// δ and the attempt counter

/// The documented δ, computed without the crate's hasher reuse.
fn documented_delta(coefficients: &[u128], counter: u64) -> u128 {
    let mut preimage = Vec::new();
    for c in coefficients {
        preimage.extend_from_slice(&c.to_be_bytes());
    }
    preimage.extend_from_slice(&counter.to_be_bytes());
    let digest = blake256::sum256(&preimage);
    let head = u128::from_be_bytes(digest[..16].try_into().expect("16 bytes"));
    model::rem_slow(&model::widen(head), MODEL_P)
}

#[test]
fn delta_is_blake256_of_the_documented_preimage() {
    let mut rng = SplitMix64::from_env("delta_is_blake256");
    for len in [2usize, 3, 4, 5, 64, 1025] {
        let c = random_poly(&mut rng, len);
        let prefix = delta_prefix(&c);
        for counter in [0u64, 1, 2, 79, 80, 1 << 40, u64::MAX] {
            assert_eq!(delta(&prefix, counter).get(), documented_delta(&c, counter));
        }
    }
}

/// Which side of a split with δ the root r falls on: the gcd with
/// (x + δ)^((P−1)/2) − 1 takes r exactly when r + δ is a nonzero square.
fn square_side(r: u128, delta: u128) -> bool {
    model::is_square(model::add_mod(r, delta))
}

/// Searches, from a fixed seed, for a client-form quadratic whose roots
/// the first split (δ = 0) and the first `k` hashed values all leave
/// together, and which attempt `k` separates.
fn ground_quadratic(k: u64) -> (Vec<u128>, [u128; 2], u64) {
    let mut rng = SplitMix64(0x6772_696e_6400_0000 | k);
    for trial in 1u64.. {
        let (r, s) = (random_fe(&mut rng), random_fe(&mut rng));
        if r == s || square_side(r, 0) != square_side(s, 0) {
            continue;
        }
        let c = polynomial(&mut rng, &[r, s], Lead::Client);
        let together = |counter| {
            let d = documented_delta(&c, counter);
            square_side(r, d) == square_side(s, d)
        };
        if (0..k).all(together) && !together(k) {
            return (c, [r, s], trial);
        }
    }
    unreachable!()
}

#[test]
fn a_ground_delta_sequence_costs_the_attacker_and_hits_the_bound() {
    // Grinding for k consecutive failures costs about 2^(k+2) trials; the
    // solver's attempts follow the documented sequence exactly.
    for k in [0u64, 1, 3, 8] {
        let (c, rs, trials) = ground_quadratic(k);
        eprintln!("k = {k}: found after {trials} trials");
        let (got, stats) = solve(&c, MAX_ATTEMPTS_PER_SPLIT).expect("splits");
        assert_eq!(got, sorted(&rs));
        assert_eq!(
            stats,
            Stats {
                attempts: k + 1,
                first_split: false,
                worst_piece: u32::try_from(k + 1).unwrap(),
            }
        );
        // With a bound of exactly k attempts, the same input exhausts it.
        let bound = u32::try_from(k).unwrap();
        assert_eq!(
            solve(&c, bound),
            Err(Error::AttemptsExhausted { degree: 2 })
        );
        assert_eq!(solve(&c, bound + 1).map(|(r, _)| r), Ok(sorted(&rs)));
    }
}

#[test]
fn first_split_trivial_when_all_roots_are_on_one_side() {
    let mut rng = SplitMix64::from_env("first_split_trivial");
    for want_square in [true, false] {
        let mut set = Vec::new();
        while set.len() < 24 {
            let r = random_fe(&mut rng);
            if model::is_square(r) == want_square && !set.contains(&r) {
                set.push(r);
            }
        }
        if !want_square {
            // 0 is on the non-square side of the first split.
            set[0] = 0;
        }
        let c = polynomial(&mut rng, &set, Lead::Client);
        let (got, stats) = solve(&c, MAX_ATTEMPTS_PER_SPLIT).expect("splits");
        assert_eq!(got, sorted(&set));
        assert!(!stats.first_split, "squares={want_square}");
        assert!(stats.attempts >= 1);
    }
}

#[test]
fn solving_is_deterministic() {
    let mut rng = SplitMix64::from_env("solving_is_deterministic");
    for degree in [2usize, 7, 64] {
        let r = distinct_roots(&mut rng, degree, &[]);
        let c = polynomial(&mut rng, &r, Lead::Random);
        let first = solve(&c, MAX_ATTEMPTS_PER_SPLIT).expect("splits");
        for _ in 0..3 {
            assert_eq!(solve(&c, MAX_ATTEMPTS_PER_SPLIT).expect("splits"), first);
        }
        assert_eq!(roots(&c), Ok(first.0));
    }
}

// ---------------------------------------------------------------------------
// Determinism self-vectors

const SELF_VECTORS: &str = "tests/data/self_vectors.txt";
const SELF_VECTOR_SEED: u64 = 0x7365_6c66_7665_6301;

fn self_vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SELF_VECTORS)
}

/// The inputs: (label, coefficients), regenerated from fixed seeds.
fn self_vector_cases() -> Vec<(String, Vec<u128>)> {
    let mut rng = SplitMix64(SELF_VECTOR_SEED);
    let mut cases = Vec::new();
    let plan = [
        (1usize, Lead::Client),
        (1, Lead::Random),
        (2, Lead::Client),
        (3, Lead::One),
        (4, Lead::Random),
        (5, Lead::Client),
        (8, Lead::Client),
        (16, Lead::One),
        (33, Lead::Client),
        (64, Lead::Client),
        (100, Lead::Random),
        (128, Lead::Client),
        (256, Lead::Client),
    ];
    for (degree, lead) in plan {
        let r = distinct_roots(&mut rng, degree, &[]);
        cases.push((
            format!("random-{degree}-{lead:?}").to_lowercase(),
            polynomial(&mut rng, &r, lead),
        ));
    }
    let edges = distinct_roots(&mut rng, 16, &[0, 1, 2, MODEL_P - 2, MODEL_P - 1, 1 << 126]);
    cases.push((
        "edges-16".into(),
        polynomial(&mut rng, &edges, Lead::Client),
    ));
    for (label, want_square) in [("squares-32", true), ("non-squares-32", false)] {
        let mut set = BTreeSet::new();
        while set.len() < 32 {
            let r = random_fe(&mut rng);
            if model::is_square(r) == want_square {
                set.insert(r);
            }
        }
        let set: Vec<u128> = set.into_iter().collect();
        cases.push((label.into(), polynomial(&mut rng, &set, Lead::Client)));
    }
    for k in [3u64, 8] {
        cases.push((format!("ground-{k}"), ground_quadratic(k).0));
    }
    cases
}

fn digest16(values: &[u128]) -> String {
    let mut bytes = Vec::new();
    for v in values {
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    hex(&blake256::sum256(&bytes)[..16])
}

fn self_vector_line(label: &str, c: &[u128]) -> String {
    let (r, stats) = solve(c, MAX_ATTEMPTS_PER_SPLIT).expect("every self-vector splits");
    let prefix = delta_prefix(c);
    let delta0 = delta(&prefix, 0).get();
    let last = match stats.attempts.checked_sub(1) {
        Some(n) => format!("{:x}", delta(&prefix, n).get()),
        None => "-".to_owned(),
    };
    format!(
        "{label} degree={} coeffs={} attempts={} first_split={} worst={} delta0={delta0:x} \
         delta_last={last} roots={}",
        c.len() - 1,
        digest16(c),
        stats.attempts,
        u8::from(stats.first_split),
        stats.worst_piece,
        digest16(&r),
    )
}

fn body_of(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn determinism_self_vectors() {
    let text = std::fs::read_to_string(self_vectors_path())
        .unwrap_or_else(|e| panic!("read {SELF_VECTORS}: {e}"));
    let header = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{SELF_VECTORS}: no {key:?} header"))
            .to_owned()
    };
    let body = body_of(&text);
    assert_eq!(
        header("# body-blake256:"),
        hex(&blake256::sum256(body.as_bytes())),
        "{SELF_VECTORS}: body hash"
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        header("# entries:").parse::<usize>().ok(),
        Some(lines.len())
    );
    let cases = self_vector_cases();
    assert_eq!(lines.len(), cases.len(), "{SELF_VECTORS}: entry count");
    for ((label, c), line) in cases.iter().zip(&lines) {
        assert_eq!(&self_vector_line(label, c), line, "{SELF_VECTORS}: {label}");
    }
}

#[test]
#[ignore = "rewrites tests/data/self_vectors.txt: PURSE_MIXSOLVER_REGEN=1 ... -- --ignored regenerate_self_vectors"]
fn regenerate_self_vectors() {
    assert!(
        std::env::var_os("PURSE_MIXSOLVER_REGEN").is_some(),
        "set PURSE_MIXSOLVER_REGEN=1 to rewrite {SELF_VECTORS}"
    );
    let lines: Vec<String> = self_vector_cases()
        .iter()
        .map(|(label, c)| self_vector_line(label, c))
        .collect();
    let body = lines.join("\n");
    let mut text = String::new();
    for l in [
        "# Determinism self-vectors for purse-mixsolver.",
        "#",
        "# SELF-GENERATED: every value below was computed by this crate, not by Go or",
        "# FLINT. They pin that the crate is deterministic across platforms and",
        "# releases: the BLAKE-256 splitting values (delta0 = attempt 0, delta_last =",
        "# the last attempt drawn), the number of attempts, whether the first split",
        "# (delta = 0) was nontrivial, the most attempts one factor needed, and a",
        "# digest of the roots. They say nothing about parity with Go; that is the",
        "# FLINT differential's (tests/oracle.rs) and the FLINT vectors'. A change",
        "# here means the splitting path changed, which is observable only as time",
        "# but must be deliberate: the roots themselves are checked independently by",
        "# the property tests.",
        "#",
        "# delta(c) is this crate's rule, which ADR-0010 leaves open in its encodings",
        "# (crate documentation, \"Departures from ADR-0010\"): the first 16 bytes,",
        "# read big-endian and reduced modulo 2^127 - 1, of BLAKE-256(a_0 || ... ||",
        "# a_n || c), each coefficient as given (before scaling to monic) as 16 bytes",
        "# big-endian and the counter c as 8 bytes big-endian.",
        "#",
        "# Inputs: src/tests.rs self_vector_cases, from SplitMix64 seed 0x73656c6676656301",
        "# (coeffs = the first 16 bytes of BLAKE-256 over the coefficients, 16 bytes",
        "# big-endian each; roots likewise over the sorted roots).",
        "# Regenerate: PURSE_MIXSOLVER_REGEN=1 cargo test -p purse-mixsolver --lib --",
        "#   --ignored regenerate_self_vectors",
    ] {
        text.push_str(l);
        text.push('\n');
    }
    let _ = writeln!(text, "# entries: {}", lines.len());
    let _ = writeln!(
        text,
        "# body-blake256: {}",
        hex(&blake256::sum256(body.as_bytes()))
    );
    text.push_str(&body);
    text.push('\n');
    std::fs::write(self_vectors_path(), text).expect("write self-vectors");
}

#[test]
#[ignore = "statistics: cargo test --release -p purse-mixsolver --lib -- --ignored attempt_statistics --nocapture"]
fn attempt_statistics() {
    let mut rng = SplitMix64::from_env("attempt_statistics");
    for (degree, reps) in [(2usize, 4000), (8, 2000), (64, 300), (512, 30)] {
        let mut attempts = Vec::new();
        let mut worst = 0;
        let mut first = 0;
        for _ in 0..reps {
            let r = distinct_roots(&mut rng, degree, &[]);
            let c = polynomial(&mut rng, &r, Lead::Client);
            let (_, s) = solve(&c, MAX_ATTEMPTS_PER_SPLIT).expect("splits");
            attempts.push(s.attempts);
            worst = worst.max(s.worst_piece);
            first += usize::from(s.first_split);
        }
        attempts.sort_unstable();
        eprintln!(
            "degree {degree:>4}: attempts median {} max {}, worst factor {worst} tries, \
             first split nontrivial {first}/{reps}",
            attempts[attempts.len() / 2],
            attempts[attempts.len() - 1],
        );
    }
}

#[test]
fn limits_are_the_documented_ones() {
    assert_eq!(MAX_DEGREE, 1024, "mixing.MaxMtot");
    assert_eq!(MAX_ATTEMPTS_PER_SPLIT, 80);
    // (n − 1)·2^−80 ≤ 2^−70 at the largest degree.
    assert!((MAX_DEGREE - 1) as u128 <= 1 << (MAX_ATTEMPTS_PER_SPLIT - 70));
}
