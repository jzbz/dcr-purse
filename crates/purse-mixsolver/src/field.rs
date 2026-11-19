// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Arithmetic in F = 2¹²⁷ − 1 on `u128`.
//!
//! An [`Fe`] always holds its canonical residue, a value below [`P`]; every
//! constructor reduces or refuses, so no operation sees anything else.
//! Because the modulus is a Mersenne prime, 2¹²⁷ ≡ 1, so a value is reduced
//! by adding its bits above position 127 to its low 127 bits: shifts and
//! adds, no division.
//!
//! The arithmetic is explicit about overflow: every `+`, `-`, `*` and shift
//! on an integer is a `wrapping_*` or `overflowing_*` call whose operands
//! are bounded so that it cannot wrap (each says why), or whose wrap is
//! the point (`overflowing_add` feeding a carry). Nothing here panics.

/// The field prime 2¹²⁷ − 1 (`mixing.F`, `dcrd/mixing@v0.7.5/field.go:11-16`).
pub(crate) const P: u128 = u128::MAX >> 1;

/// A field element: a canonical residue modulo [`P`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Fe(u128);

/// The full 256-bit product of two `u128` values as `(high, low)` halves,
/// computed on 64-bit limbs.
pub(crate) fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    // Truncating casts take the low limbs; the shifts take the high ones.
    let (a0, a1) = (a as u64, a.wrapping_shr(64) as u64);
    let (b0, b1) = (b as u64, b.wrapping_shr(64) as u64);
    // A product of two u64 values is below 2^128 and never wraps.
    let ll = u128::from(a0).wrapping_mul(u128::from(b0));
    let lh = u128::from(a0).wrapping_mul(u128::from(b1));
    let hl = u128::from(a1).wrapping_mul(u128::from(b0));
    let hh = u128::from(a1).wrapping_mul(u128::from(b1));
    // The middle sum can exceed 2^128: its carry is worth 2^192, which is
    // 2^64 in the high half.
    let (mid, mid_carry) = lh.overflowing_add(hl);
    let (lo, lo_carry) = ll.overflowing_add(mid.wrapping_shl(64));
    // The full product is below 2^256, so the high half cannot wrap.
    let hi = hh
        .wrapping_add(mid.wrapping_shr(64))
        .wrapping_add(u128::from(mid_carry).wrapping_shl(64))
        .wrapping_add(u128::from(lo_carry));
    (hi, lo)
}

/// Folds a product of two canonical residues, `hi·2¹²⁸ + lo` with
/// `hi < 2¹²⁶`, to a value below 2¹²⁸ congruent to it modulo [`P`].
///
/// With x = hi·2¹²⁸ + lo, x ≡ (x >> 127) + (x & P) because 2¹²⁷ ≡ 1. Both
/// terms are below 2¹²⁷ when x < 2²⁵⁴, so their sum does not wrap.
fn fold(hi: u128, lo: u128) -> u128 {
    let top = hi.wrapping_shl(1) | lo.wrapping_shr(127);
    top.wrapping_add(lo & P)
}

impl Fe {
    /// Zero.
    pub(crate) const ZERO: Fe = Fe(0);
    /// One.
    pub(crate) const ONE: Fe = Fe(1);

    /// The element `v`, or `None` when `v` is not canonical (`v >= P`).
    pub(crate) fn new(v: u128) -> Option<Fe> {
        (v < P).then_some(Fe(v))
    }

    /// The residue of any `u128`.
    pub(crate) fn reduce(v: u128) -> Fe {
        // (v & P) ≤ P and v >> 127 ≤ 1, so the sum is at most P + 1 and
        // one conditional subtraction makes it canonical.
        let t = (v & P).wrapping_add(v.wrapping_shr(127));
        Fe(if t >= P { t.wrapping_sub(P) } else { t })
    }

    /// The canonical residue.
    pub(crate) fn get(self) -> u128 {
        self.0
    }

    /// Whether this is zero.
    pub(crate) fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// `self + rhs`.
    pub(crate) fn add(self, rhs: Fe) -> Fe {
        // Both are below 2^127, so the sum is below 2^128 and below 2P.
        let s = self.0.wrapping_add(rhs.0);
        Fe(if s >= P { s.wrapping_sub(P) } else { s })
    }

    /// `self - rhs`.
    pub(crate) fn sub(self, rhs: Fe) -> Fe {
        if self.0 >= rhs.0 {
            Fe(self.0.wrapping_sub(rhs.0))
        } else {
            // self < rhs < P: P - rhs + self is in (0, P).
            Fe(P.wrapping_sub(rhs.0).wrapping_add(self.0))
        }
    }

    /// `-self`.
    pub(crate) fn neg(self) -> Fe {
        if self.0 == 0 {
            self
        } else {
            Fe(P.wrapping_sub(self.0))
        }
    }

    /// `self · rhs`.
    pub(crate) fn mul(self, rhs: Fe) -> Fe {
        let (hi, lo) = mul_wide(self.0, rhs.0);
        Fe::reduce(fold(hi, lo))
    }

    /// `self^e`, by square-and-multiply from the most significant bit.
    pub(crate) fn pow(self, e: u128) -> Fe {
        let mut r = Fe::ONE;
        for bit in (0..u128::BITS).rev() {
            r = r.mul(r);
            if e.wrapping_shr(bit) & 1 == 1 {
                r = r.mul(self);
            }
        }
        r
    }

    /// `self⁻¹` by Fermat's little theorem (`self^(P−2)`); zero maps to
    /// zero, which no caller passes (each inverts a leading coefficient of
    /// a nonzero polynomial).
    pub(crate) fn inv(self) -> Fe {
        // P ≥ 2, so P - 2 does not wrap.
        self.pow(P.wrapping_sub(2))
    }
}

/// A sum of products of canonical residues, reduced once at the end.
///
/// Each product is split into three values below 2¹²⁸ congruent to it in
/// sum ([`Acc::mul_add`]), each added to `lo`; a wrap of `lo` is a multiple
/// of 2¹²⁸ ≡ 2 (mod P), counted in `carries`. No accumulator in this crate
/// takes more than about 2·[`crate::MAX_DEGREE`] products, so `carries`
/// stays below 2¹⁴ and neither it nor its doubling in [`Acc::double`] can
/// wrap.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Acc {
    lo: u128,
    carries: u64,
}

impl Acc {
    /// An accumulator holding `a`.
    pub(crate) fn from_fe(a: Fe) -> Acc {
        Acc {
            lo: a.0,
            carries: 0,
        }
    }

    fn add_folded(&mut self, v: u128) {
        let (lo, carry) = self.lo.overflowing_add(v);
        self.lo = lo;
        self.carries = self.carries.wrapping_add(u64::from(carry));
    }

    /// Adds `a · b`.
    ///
    /// On 64-bit limbs, a·b = hh·2¹²⁸ + mid·2⁶⁴ + ll with a₁, b₁ < 2⁶³
    /// (both operands are below 2¹²⁷), so hh < 2¹²⁶ and mid = a₀b₁ + a₁b₀ <
    /// 2¹²⁸ does not wrap. With 2¹²⁸ ≡ 2, a·b ≡ ll + (mid mod 2⁶⁴)·2⁶⁴ +
    /// 2·(hh + ⌊mid / 2⁶⁴⌋), three terms below 2¹²⁸ each (the last because
    /// hh + ⌊mid / 2⁶⁴⌋ < 2¹²⁷). This is [`mul_wide`] and [`fold`] without
    /// the carries a general product needs.
    pub(crate) fn mul_add(&mut self, a: Fe, b: Fe) {
        let (a0, a1) = (a.0 as u64, a.0.wrapping_shr(64) as u64);
        let (b0, b1) = (b.0 as u64, b.0.wrapping_shr(64) as u64);
        let ll = u128::from(a0).wrapping_mul(u128::from(b0));
        let hh = u128::from(a1).wrapping_mul(u128::from(b1));
        let mid = u128::from(a0)
            .wrapping_mul(u128::from(b1))
            .wrapping_add(u128::from(a1).wrapping_mul(u128::from(b0)));
        let high = hh.wrapping_add(mid.wrapping_shr(64)).wrapping_shl(1);
        self.add_folded(ll);
        self.add_folded(mid.wrapping_shl(64));
        self.add_folded(high);
    }

    /// Doubles the accumulated value.
    pub(crate) fn double(&mut self) {
        // value = carries·2^128 + lo; twice that moves lo's top bit into
        // the carries.
        let top = self.lo.wrapping_shr(127);
        self.lo = self.lo.wrapping_shl(1);
        self.carries = self
            .carries
            .wrapping_shl(1)
            .wrapping_add(u64::from(top == 1));
    }

    /// The accumulated value's residue.
    pub(crate) fn finish(self) -> Fe {
        // carries·2^128 ≡ 2·carries; carries is far below 2^63.
        let wraps = Fe::reduce(u128::from(self.carries).wrapping_shl(1));
        Fe::reduce(self.lo).add(wraps)
    }
}
