// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared test helpers: the independent arithmetic model and generators of
//! polynomials with known roots.

#![allow(
    dead_code,
    reason = "each test crate uses a different part of these helpers"
)]

pub mod model;

use std::collections::BTreeSet;

use model::{P, expand, scale};
use purse_testutil::SplitMix64;

/// A uniformly random canonical residue.
pub fn random_fe(rng: &mut SplitMix64) -> u128 {
    loop {
        let v = ((u128::from(rng.next_u64()) << 64) | u128::from(rng.next_u64())) & P;
        if v != P {
            return v;
        }
    }
}

/// A random nonzero residue.
pub fn random_nonzero(rng: &mut SplitMix64) -> u128 {
    loop {
        let v = random_fe(rng);
        if v != 0 {
            return v;
        }
    }
}

/// `n` distinct residues: `forced` first (deduplicated, kept in order),
/// then random ones.
pub fn distinct_roots(rng: &mut SplitMix64, n: usize, forced: &[u128]) -> Vec<u128> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(n);
    for &r in forced.iter().take(n) {
        assert!(r < P);
        if seen.insert(r) {
            out.push(r);
        }
    }
    while out.len() < n {
        let r = random_fe(rng);
        if seen.insert(r) {
            out.push(r);
        }
    }
    out
}

/// How the leading coefficient of a test polynomial is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lead {
    /// Monic.
    One,
    /// F − 1, as `mixing.Coefficients` makes every input the client passes.
    Client,
    /// A random nonzero scalar.
    Random,
}

/// The coefficients of `lead · ∏ (x − r)`.
pub fn polynomial(rng: &mut SplitMix64, roots: &[u128], lead: Lead) -> Vec<u128> {
    let monic = expand(roots);
    match lead {
        Lead::One => monic,
        Lead::Client => scale(&monic, P - 1),
        Lead::Random => scale(&monic, random_nonzero(rng)),
    }
}

/// `roots` sorted ascending.
pub fn sorted(roots: &[u128]) -> Vec<u128> {
    let mut s = roots.to_vec();
    s.sort_unstable();
    s
}

/// A count from the environment variable `name`, or `default`.
pub fn env_count(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(v) => v
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name}={v:?} is not a count")),
        Err(_) => default,
    }
}

/// Whether the tests that are slow without optimisation run: always in an
/// optimised build (no `debug_assertions`), and in a debug build only when
/// `PURSE_MIXSOLVER_HEAVY` is set. What they add over the debug subset is
/// degree, not new code paths: the largest accumulations an overflow check
/// could catch are covered at every degree by the unit test of 4100
/// maximal products.
pub fn heavy() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("PURSE_MIXSOLVER_HEAVY").is_some()
}

/// Lower-case hex of a residue, without leading zeros (Go's `Text(16)`).
pub fn hex128(v: u128) -> String {
    format!("{v:x}")
}

/// Parses `hex128`'s output.
pub fn parse_hex128(s: &str) -> u128 {
    u128::from_str_radix(s, 16).unwrap_or_else(|e| panic!("bad hex {s:?}: {e}"))
}
