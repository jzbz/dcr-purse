// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! R3 = GF(3)[x]/(x^761 − x − 1): multiplication and the reciprocal (Go's
//! `r3` package, `r3/r3.go`).

use zeroize::{Zeroize, Zeroizing};

use crate::arith::{P, freeze3, nonzero_mask};
use crate::conv;

/// h = f·g in R3 (`r3.Mult`, `r3/r3.go:73-96`).
///
/// Go reduces after every term; each of its [`freeze3`] inputs is within
/// ±(1 + |f·g|), at most 2^14 + 1 for any `i8` operands, where `freeze3`
/// is exact, so its result is the true product mod 3 in {−1, 0, 1}. This
/// computes that product directly: both operands reduced to {−1, 0, 1},
/// the integer product by [`conv::mul`] (every sum within ±2283), and one
/// [`freeze3`] of each coefficient. `reference::r3_mult` is the test model.
pub(crate) fn mult(h: &mut [i8; P], f: &[i8; P], g: &[i8; P]) {
    let mut a = Zeroizing::new([0i16; P]);
    for (ai, &fi) in a.iter_mut().zip(f.iter()) {
        *ai = i16::from(freeze3(i32::from(fi)));
    }
    let mut b = Zeroizing::new([0i16; P]);
    for (bi, &gi) in b.iter_mut().zip(g.iter()) {
        *bi = i16::from(freeze3(i32::from(gi)));
    }
    let mut acc = Zeroizing::new([0i32; P]);
    conv::mul(&mut acc, &a, &b);
    for (hi, &x) in h.iter_mut().zip(acc.iter()) {
        *hi = freeze3(x);
    }
}

/// The 64-bit words of a bitsliced array of P + 1 = 762 coefficients.
const WORDS: usize = (P + 1).div_ceil(64);
/// The bits of the last word that hold coefficients (762 − 704 = 58).
const TOP_MASK: u64 = (1u64 << (P + 1 - 64 * (WORDS - 1))) - 1;

/// An element of GF(3)[x] of degree ≤ 761, bitsliced: bit i of `pos` is
/// set when coefficient i is 1, bit i of `neg` when it is −1; never both.
#[derive(Clone, Copy, Default)]
struct Bits {
    pos: [u64; WORDS],
    neg: [u64; WORDS],
}

impl Zeroize for Bits {
    fn zeroize(&mut self) {
        self.pos.zeroize();
        self.neg.zeroize();
    }
}

impl Bits {
    /// Coefficient 0, as −1, 0 or 1.
    #[inline(always)]
    fn c0(&self) -> i32 {
        ((self.pos[0] & 1) as i32).wrapping_sub((self.neg[0] & 1) as i32)
    }

    /// Multiplies by x, dropping the coefficient of x^762.
    #[inline(always)]
    fn shift_up(&mut self) {
        for plane in [&mut self.pos, &mut self.neg] {
            for w in (1..WORDS).rev() {
                plane[w] = (plane[w] << 1) | (plane[w.wrapping_sub(1)] >> 63);
            }
            plane[0] <<= 1;
            plane[WORDS - 1] &= TOP_MASK;
        }
    }

    /// Divides by x, dropping coefficient 0.
    #[inline(always)]
    fn shift_down(&mut self) {
        for plane in [&mut self.pos, &mut self.neg] {
            for w in 0..WORDS - 1 {
                plane[w] = (plane[w] >> 1) | (plane[w.wrapping_add(1)] << 63);
            }
            plane[WORDS - 1] >>= 1;
        }
    }

    /// Exchanges `self` and `other` when `mask` is all ones.
    #[inline(always)]
    fn swap(&mut self, other: &mut Self, mask: u64) {
        for w in 0..WORDS {
            let t = mask & (self.pos[w] ^ other.pos[w]);
            self.pos[w] ^= t;
            other.pos[w] ^= t;
            let t = mask & (self.neg[w] ^ other.neg[w]);
            self.neg[w] ^= t;
            other.neg[w] ^= t;
        }
    }

    /// `self += c·y` coefficient by coefficient in GF(3), `c` given as the
    /// masks `cp` (all ones when c = 1) and `cn` (all ones when c = −1).
    #[inline(always)]
    fn add_scaled(&mut self, y: &Self, cp: u64, cn: u64) {
        for w in 0..WORDS {
            // b = c·y: c = −1 exchanges the planes, c = 0 clears both.
            let bp = (y.pos[w] & cp) | (y.neg[w] & cn);
            let bn = (y.neg[w] & cp) | (y.pos[w] & cn);
            let (ap, an) = (self.pos[w], self.neg[w]);
            let az = !(ap | an);
            let bz = !(bp | bn);
            // 1 = 1 + 0 = 0 + 1 = −1 + −1; −1 = −1 + 0 = 0 + −1 = 1 + 1.
            self.pos[w] = (ap & bz) | (bp & az) | (an & bn);
            self.neg[w] = (an & bz) | (bn & az) | (ap & bp);
        }
    }
}

/// r = 1/s in R3 (`r3.Reciprocal`, `r3/r3.go:31-70`). Returns 0 when `s`
/// is invertible and −1 otherwise; then `r` holds no inverse, and not what
/// Go leaves there (key generation discards it, as Go does).
///
/// Go runs 1523 steps of an extended Euclidean loop over arrays of 762 and
/// 1524 coefficients, one byte each (`reference::r3_reciprocal`). This is
/// the constant-time divstep inversion of the NTRU Prime round-2 reference
/// code (`r3_recip`, public domain): 2·761 − 1 steps over four arrays of
/// 762 coefficients, here bitsliced, so one step is a few hundred word
/// operations. Both compute the inverse when there is one, and the inverse
/// is unique, so the results are equal whenever Go's is used; both report
/// invertibility exactly. The number of steps is fixed, and `s` reaches no
/// branch and no index: the swap is a mask, and the scalar `c` of each
/// step selects by masks.
pub(crate) fn reciprocal(r: &mut [i8; P], s: &[i8; P]) -> i32 {
    // The polynomials are reversed: f is x^761·m(1/x) = 1 − x^760 − x^761
    // for the modulus m = x^761 − x − 1, and g is s reversed.
    let mut st = Zeroizing::new([Bits::default(); 4]);
    let [f, g, v, w] = &mut *st;
    f.pos[0] = 1;
    f.neg[(P - 1) / 64] |= 1 << ((P - 1) % 64);
    f.neg[P / 64] |= 1 << (P % 64);
    for (i, &si) in s.iter().enumerate() {
        // Go's arithmetic depends on s only mod 3.
        let c = freeze3(i32::from(si));
        let k = (P - 1).wrapping_sub(i);
        // c = 1 is the byte 0x01, c = −1 the byte 0xff.
        g.pos[k / 64] |= (((c & 1) & !(c >> 1)) as u8 as u64 & 1) << (k % 64);
        g.neg[k / 64] |= (((c >> 1) & 1) as u8 as u64) << (k % 64);
    }
    w.pos[0] = 1;
    let mut delta: i32 = 1;

    for _ in 0..2 * P - 1 {
        v.shift_up();
        // c = −g0·f0 (the inverse of f0 = ±1 is f0) cancels coefficient 0
        // of g + c·f; it is the same after the swap.
        let (f0, g0) = (f.c0(), g.c0());
        let c = g0.wrapping_mul(f0).wrapping_neg();
        let swap = core::hint::black_box((delta.wrapping_neg() >> 31) & nonzero_mask(g0));
        delta ^= swap & (delta ^ delta.wrapping_neg());
        delta = delta.wrapping_add(1);
        let swap = swap as u64;
        f.swap(g, swap);
        v.swap(w, swap);
        // All ones when c is 1, or when c is −1.
        let cp = core::hint::black_box(((c & 1) & !(c >> 1)) as u64).wrapping_neg();
        let cn = core::hint::black_box((c >> 1) as u64);
        g.add_scaled(f, cp, cn);
        w.add_scaled(v, cp, cn);
        g.shift_down();
    }

    // r = f0·(v reversed), f0 = ±1 being its own inverse.
    let f0 = f.c0() as i8;
    for (i, ri) in r.iter_mut().enumerate() {
        let k = (P - 1).wrapping_sub(i);
        let c = (((v.pos[k / 64] >> (k % 64)) & 1) as i8)
            .wrapping_sub(((v.neg[k / 64] >> (k % 64)) & 1) as i8);
        *ri = c.wrapping_mul(f0);
    }
    nonzero_mask(delta)
}
