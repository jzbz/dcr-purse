// SPDX-License-Identifier: ISC
//! Mixing root solver fuzz target: `purse_mixsolver::roots` on arbitrary
//! and on constructed polynomials (ADR-0010, Decision 4).
//!
//! The solver must never panic and must finish quickly, whatever the
//! coefficients; every `Ok` it returns is certified here with arithmetic of
//! this file's own: as many roots as the degree, strictly ascending, each
//! canonical and each a root. The first byte picks the mode:
//!
//! - 0: arbitrary coefficients (16 bytes each, little-endian), at most 65
//!   of them; with the mode's bit 3 set, the top bit of each is cleared so
//!   that most are canonical and the solver gets past validation. Any `Ok`
//!   must certify, and the answer must never be `SelfCheckFailed` (a defect
//!   of the crate) or `AttemptsExhausted`. The latter is excluded only in
//!   probability: a polynomial that splits exhausts the bound with
//!   probability at most (n − 1)·2⁻⁸⁰, and arbitrary bytes cannot grind the
//!   hash-derived splitting values towards it.
//! - 1: chosen roots (at most 48) and a nonzero leading scalar; the
//!   polynomial lead·∏(x − r) must give exactly the distinct roots back in
//!   ascending order, or `NotSplit` when two chosen roots coincide.
//! - 2: as 1, times x² − a, with the answer decided exactly: when a is a
//!   nonzero square with square roots ±s (s = a^((F+1)/4), F ≡ 3 mod 4),
//!   the polynomial's roots are the chosen ones and ±s, and it must solve
//!   to them when all are distinct, `NotSplit` otherwise; when a is 0 (x²,
//!   a repeated root) or not a square, the answer must be `NotSplit`. This
//!   reaches root pairs r, F − r, which mode 1 essentially never builds.
//! - 3: more coefficients than `MAX_DEGREE + 1`: `DegreeTooLarge`, at once.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_mixsolver::{Error, FIELD_PRIME as P, MAX_DEGREE, roots};

/// The largest degree tried on arbitrary input: high enough to reach every
/// path, low enough to keep each run to milliseconds.
const MAX_FUZZ_DEGREE: usize = 64;
const MAX_FUZZ_ROOTS: usize = 48;

fn mul_mod(a: u128, b: u128) -> u128 {
    // a, b < 2^127: schoolbook on 64-bit limbs into (hi, lo), then
    // x = hi·2^128 + lo ≡ 2·hi + lo, folded twice at bit 127.
    let (a0, a1) = (a as u64 as u128, a >> 64);
    let (b0, b1) = (b as u64 as u128, b >> 64);
    let ll = a0 * b0;
    let mid = a0 * b1 + a1 * b0; // < 2^128
    let hh = a1 * b1; // < 2^126
    let (lo, c) = ll.overflowing_add(mid << 64);
    let hi = hh + (mid >> 64) + c as u128;
    // x = hi·2^128 + lo; x >> 127 and x & P.
    let top = (hi << 1) | (lo >> 127);
    let s = top + (lo & P);
    let s = (s & P) + (s >> 127);
    if s >= P { s - P } else { s }
}

fn add_mod(a: u128, b: u128) -> u128 {
    let s = a + b;
    if s >= P { s - P } else { s }
}

fn sub_mod(a: u128, b: u128) -> u128 {
    if a >= b { a - b } else { P - (b - a) }
}

fn pow_mod(mut a: u128, mut e: u128) -> u128 {
    let mut r = 1;
    while e != 0 {
        if e & 1 == 1 {
            r = mul_mod(r, a);
        }
        a = mul_mod(a, a);
        e >>= 1;
    }
    r
}

fn eval(c: &[u128], x: u128) -> u128 {
    c.iter()
        .rev()
        .fold(0, |acc, &k| add_mod(mul_mod(acc, x), k))
}

/// lead·∏(x − r).
fn expand(roots: &[u128], lead: u128) -> Vec<u128> {
    let mut c = vec![lead];
    for &r in roots {
        let mut next = vec![0u128; c.len() + 1];
        for (k, &ck) in c.iter().enumerate() {
            next[k + 1] = add_mod(next[k + 1], ck);
            next[k] = sub_mod(next[k], mul_mod(r, ck));
        }
        c = next;
    }
    c
}

fn certify(coeffs: &[u128], got: &[u128]) {
    assert_eq!(got.len(), coeffs.len() - 1, "root count");
    assert!(
        got.windows(2).all(|w| w[0] < w[1]),
        "not strictly ascending"
    );
    for &r in got {
        assert!(r < P, "non-canonical root");
        assert_eq!(eval(coeffs, r), 0, "not a root");
    }
}

fn words(data: &[u8]) -> impl Iterator<Item = u128> + '_ {
    data.chunks_exact(16)
        .map(|c| u128::from_le_bytes(c.try_into().unwrap()))
}

fn canonical(v: u128) -> u128 {
    let v = (v & P) + (v >> 127);
    if v >= P { v - P } else { v }
}

/// Roots and lead from the bytes; `None` when there are too few bytes.
fn chosen(data: &[u8]) -> Option<(Vec<u128>, u128, Vec<u128>)> {
    let mut it = words(data).map(canonical);
    let lead = it.next()?.max(1);
    let r: Vec<u128> = it.take(MAX_FUZZ_ROOTS).collect();
    let mut distinct = r.clone();
    distinct.sort_unstable();
    distinct.dedup();
    Some((r, lead, distinct))
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    match mode % 4 {
        0 => {
            let mask = if mode & 8 != 0 { P } else { u128::MAX };
            let c: Vec<u128> = words(rest)
                .take(MAX_FUZZ_DEGREE + 1)
                .map(|v| v & mask)
                .collect();
            match roots(&c) {
                Ok(got) => certify(&c, &got),
                Err(e) => assert!(
                    !matches!(e, Error::SelfCheckFailed | Error::AttemptsExhausted { .. }),
                    "{e:?}"
                ),
            }
        }
        1 => {
            let Some((r, lead, distinct)) = chosen(rest) else {
                return;
            };
            if r.is_empty() {
                return;
            }
            let c = expand(&r, lead);
            let got = roots(&c);
            if distinct.len() == r.len() {
                assert_eq!(got.as_ref(), Ok(&distinct));
                assert_eq!(roots(&c), got, "not deterministic");
            } else {
                assert_eq!(got, Err(Error::NotSplit), "repeated root accepted");
            }
        }
        2 => {
            let Some((mut r, lead, _)) = chosen(rest) else {
                return;
            };
            let a = r.pop().unwrap_or(P - 1);
            // lead·∏(x − r)·(x² − a).
            let base = expand(&r, lead);
            let mut c = vec![0u128; base.len() + 2];
            for (k, &bk) in base.iter().enumerate() {
                c[k + 2] = add_mod(c[k + 2], bk);
                c[k] = sub_mod(c[k], mul_mod(a, bk));
            }
            let got = roots(&c);
            assert_ne!(got, Err(Error::SelfCheckFailed));
            // F ≡ 3 (mod 4): s = a^((F+1)/4) squares to a exactly when a is
            // a square.
            let s = pow_mod(a, (P + 1) >> 2);
            if a != 0 && mul_mod(s, s) == a {
                let mut all = r.clone();
                all.extend([s, P - s]);
                let mut distinct = all.clone();
                distinct.sort_unstable();
                distinct.dedup();
                if distinct.len() == all.len() {
                    assert_eq!(got.as_ref(), Ok(&distinct), "x² − a with a square");
                    assert_eq!(roots(&c), got, "not deterministic");
                } else {
                    assert_eq!(got, Err(Error::NotSplit), "repeated root accepted");
                }
            } else {
                // a = 0 (x² is a repeated root) or a non-square.
                assert_eq!(got, Err(Error::NotSplit), "a = {a:#x}");
            }
        }
        _ => {
            let extra = 1 + usize::from(rest.first().copied().unwrap_or(0) % 3);
            let c = vec![1u128; MAX_DEGREE + 1 + extra];
            assert_eq!(
                roots(&c),
                Err(Error::DegreeTooLarge {
                    degree: MAX_DEGREE + extra
                })
            );
        }
    }
});
