// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The solver: validation, the splitting test, Cantor-Zassenhaus with
//! deterministic splitting values, and the final check (ADR-0010,
//! Decision 1, steps 1-4).

use alloc::vec::Vec;

use dcroxide_crypto::blake256::Blake256;

use crate::field::{Fe, P};
use crate::poly::{Modulus, div_rem, eval, gcd, trim};
use crate::{Error, MAX_DEGREE};

/// (F − 1)/2 = 2¹²⁶ − 1, the exponent of Euler's criterion.
pub(crate) const HALF: u128 = P >> 1;

/// What a solve did besides its answer, for tests and the benchmark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Splitting values drawn from the hash, which is also the next value
    /// of the attempt counter.
    pub(crate) attempts: u64,
    /// Whether the first split, with δ = 0 (h − 1 itself), was nontrivial.
    pub(crate) first_split: bool,
    /// The most attempts any one factor needed.
    pub(crate) worst_piece: u32,
}

/// The hash state after absorbing the coefficients: each as 16 bytes,
/// big-endian, in ascending degree order.
pub(crate) fn delta_prefix(coefficients: &[u128]) -> Blake256 {
    let mut h = Blake256::new();
    for c in coefficients {
        h.update(&c.to_be_bytes());
    }
    h
}

/// The splitting value for attempt `counter`: the first 16 bytes of
/// BLAKE-256(coefficients ‖ counter as 8 bytes big-endian), read as a
/// big-endian integer and reduced modulo F.
pub(crate) fn delta(prefix: &Blake256, counter: u64) -> Fe {
    let mut h = prefix.clone();
    h.update(&counter.to_be_bytes());
    let digest = h.finalize();
    let head = digest.first_chunk::<16>().copied().unwrap_or_default();
    Fe::reduce(u128::from_be_bytes(head))
}

/// `w − 1`, normalized, for a residue `w`.
fn minus_one(mut w: Vec<Fe>) -> Vec<Fe> {
    match w.first_mut() {
        Some(c0) => *c0 = c0.sub(Fe::ONE),
        None => w.push(Fe::ONE.neg()),
    }
    trim(&mut w);
    w
}

/// Two monic factors whose product is the factor split.
type Factors = (Vec<Fe>, Vec<Fe>);

/// Splits the monic `f` by `w − 1`: `Some((gcd, f / gcd))` when the gcd is
/// a proper factor, `None` when the split is trivial.
fn split(f: &[Fe], w: Vec<Fe>) -> Result<Option<Factors>, Error> {
    let f1 = gcd(f, &minus_one(w));
    // f1 is a proper factor when 0 < deg f1 < deg f.
    if f1.len() < 2 || f1.len() >= f.len() {
        return Ok(None);
    }
    let (f2, r) = div_rem(f, &f1).ok_or(Error::SelfCheckFailed)?;
    if !r.is_empty() {
        return Err(Error::SelfCheckFailed);
    }
    Ok(Some((f1, f2)))
}

/// Checks and canonicalizes the input: `(a, degree)`.
fn validate(coefficients: &[u128]) -> Result<(Vec<Fe>, usize), Error> {
    let len = coefficients.len();
    let Some(degree) = len.checked_sub(1).filter(|&d| d >= 1) else {
        return Err(Error::TooFewCoefficients { len });
    };
    if degree > MAX_DEGREE {
        return Err(Error::DegreeTooLarge { degree });
    }
    let mut a = Vec::with_capacity(len);
    for (index, &c) in coefficients.iter().enumerate() {
        a.push(Fe::new(c).ok_or(Error::NonCanonical { index })?);
    }
    if a.last().is_none_or(|c| c.is_zero()) {
        return Err(Error::ZeroLeadingCoefficient);
    }
    Ok((a, degree))
}

/// The roots of the polynomial with coefficients `coefficients`, ascending,
/// trying at most `max_attempts` splitting values on any one factor.
pub(crate) fn solve(coefficients: &[u128], max_attempts: u32) -> Result<(Vec<u128>, Stats), Error> {
    let (a, degree) = validate(coefficients)?;
    let mut stats = Stats::default();

    // Step 1: g = a / lead(a); h = x^((F−1)/2) mod g.
    let mut g = a.clone();
    crate::poly::make_monic(&mut g);
    let mut found: Vec<Fe> = Vec::with_capacity(degree);
    let mut pieces: Vec<Vec<Fe>> = Vec::new();
    if degree == 1 {
        pieces.push(g);
    } else {
        let m = Modulus::new(&g).ok_or(Error::SelfCheckFailed)?;
        let h = m.pow_base(Fe::ZERO, HALF);

        // Step 2: x·h² ≡ x^F; g divides x^F − x, the product of every
        // (x − r) over F, exactly when its roots are distinct and in F.
        if m.square_mul_base(&h, Fe::ZERO) != m.reduce(&[Fe::ZERO, Fe::ONE]) {
            return Err(Error::NotSplit);
        }

        // Step 3, first split: gcd(g, h − 1) collects the roots r ≠ 0 that
        // are squares (Euler's criterion).
        match split(&g, h)? {
            Some((f1, f2)) => {
                stats.first_split = true;
                pieces.push(f2);
                pieces.push(f1);
            }
            None => pieces.push(g),
        }
    }

    // Step 3, recursively: gcd(f, (x + δ)^((F−1)/2) − 1) for δ from the
    // hash. A factor of degree one gives its root.
    let prefix = delta_prefix(coefficients);
    while let Some(f) = pieces.pop() {
        match f.as_slice() {
            [f0, _one] => {
                found.push(f0.neg());
                continue;
            }
            [] | [_] => return Err(Error::SelfCheckFailed),
            _ => {}
        }
        let m = Modulus::new(&f).ok_or(Error::SelfCheckFailed)?;
        let mut tries: u32 = 0;
        loop {
            if tries >= max_attempts {
                return Err(Error::AttemptsExhausted { degree: m.degree() });
            }
            // Both counters stay far below their limits: tries ≤
            // max_attempts and attempts ≤ max_attempts · MAX_DEGREE.
            tries = tries.wrapping_add(1);
            let d = delta(&prefix, stats.attempts);
            stats.attempts = stats.attempts.wrapping_add(1);
            if let Some((f1, f2)) = split(&f, m.pow_base(d, HALF))? {
                pieces.push(f2);
                pieces.push(f1);
                break;
            }
        }
        stats.worst_piece = stats.worst_piece.max(tries);
    }

    // Step 4: every root is a root of the input, they are distinct, and
    // there are `degree` of them; sorted ascending.
    let mut roots: Vec<u128> = found.iter().map(|r| r.get()).collect();
    roots.sort_unstable();
    let distinct = roots.windows(2).all(|w| matches!(w, [x, y] if x < y));
    let all_roots = found.iter().all(|&r| eval(&a, r).is_zero());
    if roots.len() != degree || !distinct || !all_roots {
        return Err(Error::SelfCheckFailed);
    }
    Ok((roots, stats))
}
