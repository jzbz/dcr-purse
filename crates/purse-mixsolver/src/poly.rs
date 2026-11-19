// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Polynomials over F, as coefficient vectors in ascending degree order
//! (`a[k]` multiplies `x^k`, the order of `mixing.Coefficients` and of the
//! solver RPC's `A`).
//!
//! A *normalized* polynomial has no trailing zero coefficient, so the zero
//! polynomial is empty and the degree of a nonzero one is its length minus
//! one. A *residue* modulo a [`Modulus`] of degree d is a dense vector of
//! exactly d coefficients, possibly with zeros on top.
//!
//! Nothing here indexes a slice: windows are taken with `get`/`get_mut`,
//! which a correct caller never sees fail, and a failure there skips work
//! rather than panicking. The solver checks every root it returns, so a
//! skipped step could only ever surface as [`crate::Error::SelfCheckFailed`].

use alloc::vec;
use alloc::vec::Vec;

use crate::field::{Acc, Fe};

/// Drops trailing zero coefficients.
pub(crate) fn trim(a: &mut Vec<Fe>) {
    while a.last().is_some_and(|c| c.is_zero()) {
        a.pop();
    }
}

/// Scales `a` so that its leading coefficient is one; the zero polynomial
/// stays zero. `a` must be normalized.
pub(crate) fn make_monic(a: &mut [Fe]) {
    let Some(&lead) = a.last() else {
        return;
    };
    let inv = lead.inv();
    for c in a.iter_mut() {
        *c = c.mul(inv);
    }
}

/// `a(x)` by Horner's rule.
pub(crate) fn eval(a: &[Fe], x: Fe) -> Fe {
    a.iter().rev().fold(Fe::ZERO, |acc, &c| acc.mul(x).add(c))
}

/// The product `a · b`, by the schoolbook method with one reduction per
/// output coefficient.
#[cfg(test)]
pub(crate) fn mul(a: &[Fe], b: &[Fe]) -> Vec<Fe> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    // Both lengths are small (a few thousand at most in any caller).
    let len = a.len().saturating_add(b.len()).saturating_sub(1);
    let mut acc = vec![Acc::default(); len];
    for (i, &ai) in a.iter().enumerate() {
        if let Some(dst) = acc.get_mut(i..) {
            for (slot, &bj) in dst.iter_mut().zip(b) {
                slot.mul_add(ai, bj);
            }
        }
    }
    acc.into_iter().map(Acc::finish).collect()
}

/// Quotient and remainder of `a` by `b`, both normalized on return; `None`
/// when `b` is the zero polynomial or not normalized.
pub(crate) fn div_rem(a: &[Fe], b: &[Fe]) -> Option<(Vec<Fe>, Vec<Fe>)> {
    let (&lead, _) = b.split_last()?;
    if lead.is_zero() {
        return None;
    }
    let inv = lead.inv();
    // deg b = b.len() - 1; b is nonempty, so this does not wrap.
    let db = b.len().wrapping_sub(1);
    let mut r = a.to_vec();
    trim(&mut r);
    let mut q = vec![Fe::ZERO; r.len().saturating_sub(db)];
    while r.len() > db {
        let Some(&top) = r.last() else {
            break;
        };
        let c = top.mul(inv);
        // r.len() > db ≥ 0, so r.len() - 1 - db does not wrap.
        let shift = r.len().wrapping_sub(1).wrapping_sub(db);
        if let Some(slot) = q.get_mut(shift) {
            *slot = c;
        }
        if let Some(window) = r.get_mut(shift..) {
            for (rj, &bj) in window.iter_mut().zip(b) {
                *rj = rj.sub(c.mul(bj));
            }
        }
        // The top coefficient is now top - c·lead = 0 exactly.
        r.pop();
        trim(&mut r);
    }
    trim(&mut q);
    Some((q, r))
}

/// The monic greatest common divisor of `a` and `b` (zero for two zero
/// polynomials), by Euclid's algorithm.
pub(crate) fn gcd(a: &[Fe], b: &[Fe]) -> Vec<Fe> {
    let mut x = a.to_vec();
    trim(&mut x);
    let mut y = b.to_vec();
    trim(&mut y);
    while !y.is_empty() {
        let Some((_, r)) = div_rem(&x, &y) else {
            break;
        };
        x = core::mem::replace(&mut y, r);
    }
    make_monic(&mut x);
    x
}

/// Arithmetic modulo a fixed monic polynomial g of degree d ≥ 1: the hot
/// path of the solver.
///
/// A product of two residues has 2d − 1 coefficients, each a sum of
/// products kept in an [`Acc`] and reduced once; the reduction modulo g
/// then folds the top coefficients down one at a time, x^k ≡
/// x^(k−d)·(−g_low), again into accumulators. A modular squaring therefore
/// costs about d²/2 + d² multiply-accumulates and 3d reductions.
#[derive(Clone, Debug)]
pub(crate) struct Modulus {
    /// −g₀ … −g_(d−1): x^d ≡ Σ `neg_low[j]`·x^j (mod g).
    neg_low: Vec<Fe>,
}

impl Modulus {
    /// The modulus `g`, which must be normalized, monic and of degree at
    /// least one; `None` otherwise.
    pub(crate) fn new(g: &[Fe]) -> Option<Modulus> {
        let (&lead, low) = g.split_last()?;
        if lead != Fe::ONE || low.is_empty() {
            return None;
        }
        Some(Modulus {
            neg_low: low.iter().map(|c| c.neg()).collect(),
        })
    }

    /// The degree d.
    pub(crate) fn degree(&self) -> usize {
        self.neg_low.len()
    }

    /// Reduces a polynomial held in accumulators to a residue.
    fn reduce_acc(&self, mut acc: Vec<Acc>) -> Vec<Fe> {
        let d = self.degree();
        if acc.len() < d {
            acc.resize(d, Acc::default());
        }
        while acc.len() > d {
            let Some(top) = acc.pop() else {
                break;
            };
            let c = top.finish();
            if c.is_zero() {
                continue;
            }
            // The popped coefficient had index n = acc.len() ≥ d:
            // c·x^n ≡ c·x^(n−d)·Σ neg_low[j]·x^j, which adds c·neg_low[j]
            // to index n − d + j. Walking both from the top pairs them so.
            for (slot, &ng) in acc.iter_mut().rev().zip(self.neg_low.iter().rev()) {
                slot.mul_add(c, ng);
            }
        }
        acc.into_iter().map(Acc::finish).collect()
    }

    /// The residue of `t`, a polynomial of any length.
    pub(crate) fn reduce(&self, t: &[Fe]) -> Vec<Fe> {
        self.reduce_acc(t.iter().copied().map(Acc::from_fe).collect())
    }

    /// `a²`, unreduced, in 2·len(a) − 1 accumulators (none for empty `a`).
    fn square_acc(a: &[Fe]) -> Vec<Acc> {
        let len = a.len().saturating_mul(2).saturating_sub(1);
        let mut acc = vec![Acc::default(); len];
        // The products a_i·a_j with i < j, each once, at index i + j ...
        for (i, &ai) in a.iter().enumerate() {
            let (Some(rest), Some(dst)) = (
                a.get(i.saturating_add(1)..),
                acc.get_mut(i.saturating_mul(2).saturating_add(1)..),
            ) else {
                continue;
            };
            for (slot, &aj) in dst.iter_mut().zip(rest) {
                slot.mul_add(ai, aj);
            }
        }
        // ... counted twice, plus the squares a_i² at index 2i.
        for slot in &mut acc {
            slot.double();
        }
        for (slot, &ai) in acc.iter_mut().step_by(2).zip(a) {
            slot.mul_add(ai, ai);
        }
        acc
    }

    /// `a² mod g` for a residue `a`.
    pub(crate) fn square(&self, a: &[Fe]) -> Vec<Fe> {
        self.reduce_acc(Self::square_acc(a))
    }

    /// `a²·(x + δ) mod g` for a residue `a`.
    pub(crate) fn square_mul_base(&self, a: &[Fe], delta: Fe) -> Vec<Fe> {
        let s: Vec<Fe> = Self::square_acc(a).into_iter().map(Acc::finish).collect();
        // (x + δ)·s: coefficient k is s_(k−1) + δ·s_k.
        let mut t = Vec::with_capacity(s.len().saturating_add(1));
        let mut prev = Fe::ZERO;
        for &sk in &s {
            let mut c = Acc::from_fe(prev);
            c.mul_add(delta, sk);
            t.push(c);
            prev = sk;
        }
        t.push(Acc::from_fe(prev));
        self.reduce_acc(t)
    }

    /// `(x + δ)^e mod g`, by left-to-right square-and-multiply.
    pub(crate) fn pow_base(&self, delta: Fe, e: u128) -> Vec<Fe> {
        if e == 0 {
            return self.reduce(&[Fe::ONE]);
        }
        let mut r = self.reduce(&[delta, Fe::ONE]);
        // e ≠ 0, so its highest set bit is at 127 - leading_zeros; the
        // loop walks the bits below it.
        let top = (u128::BITS - 1).wrapping_sub(e.leading_zeros());
        for bit in (0..top).rev() {
            r = if e.wrapping_shr(bit) & 1 == 1 {
                self.square_mul_base(&r, delta)
            } else {
                self.square(&r)
            };
        }
        r
    }
}
