// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Arithmetic in GF(3) and GF(q), q = 4591: Go's `r3/mod3` and `rq/modq`.
//!
//! Every operation is the Go code's, on the same integer widths: Go's
//! `int32` arithmetic wraps, so every `+`, `-` and `*` here is a
//! `wrapping_*` call, and `>>` on a signed value is Go's arithmetic shift.
//! No branch and no memory index depends on a value.

/// The degree p of the ring's modulus x^p − x − 1.
pub(crate) const P: usize = 761;
/// The weight w of a t-small element: the number of nonzero coefficients.
pub(crate) const W: usize = 286;
/// (q − 1) / 2: the representatives of GF(q) are −QSHIFT..=QSHIFT.
pub(crate) const QSHIFT: i32 = 2295;

/// Reduces `a` modulo 3 to {−1, 0, 1} (`mod3.Freeze`, `r3/mod3/mod3.go:10-14`).
///
/// Canonical for every input this crate passes (|a| ≤ 2^15, checked by
/// `freeze3_is_canonical_on_its_domain`); outside that Go's wrapping
/// arithmetic is reproduced exactly, whatever it gives.
#[inline(always)]
pub(crate) fn freeze3(a: i32) -> i8 {
    let a = a.wrapping_sub(3i32.wrapping_mul(10923i32.wrapping_mul(a) >> 15));
    let a = a.wrapping_sub(
        3i32.wrapping_mul(89_478_485i32.wrapping_mul(a).wrapping_add(134_217_728) >> 28),
    );
    // Go's int8(a): the low byte, as `as` truncates.
    a as i8
}

/// Reduces `a` modulo q to −2295..=2295 (`modq.Freeze`,
/// `rq/modq/modq.go:10-14`).
///
/// Canonical for |a| < 2^23 (checked exhaustively by
/// `freezeq_is_canonical_on_its_domain`), which covers every sum and
/// product the ring arithmetic forms. Larger inputs reach it only from the
/// decoders on malformed bytes; there Go's wrapping is reproduced exactly,
/// and the result may be any `i16`.
#[inline(always)]
pub(crate) fn freezeq(a: i32) -> i16 {
    let a = a.wrapping_sub(4591i32.wrapping_mul(228i32.wrapping_mul(a) >> 20));
    let a = a.wrapping_sub(
        4591i32.wrapping_mul(58_470i32.wrapping_mul(a).wrapping_add(134_217_728) >> 28),
    );
    // Go's int16(a): the low 16 bits.
    a as i16
}

/// Reduces any `i32` modulo q to −2295..=2295, exactly.
///
/// Go has no counterpart: it never forms a sum this large. A Barrett step
/// on 64 bits, ⌊a·935520 / 2^32⌋ (935520 ≈ 2^32/q), is within one of
/// ⌊a/q⌋ for every `i32` (the relative error of the constant is below
/// 2^−25), so the remainder is within ±2q, where [`freezeq`] is exact.
/// Checked on every `i32` by `freezeq_wide_is_canonical_everywhere`
/// (ignored; release) and on a sample in every run.
#[inline(always)]
pub(crate) fn freezeq_wide(a: i32) -> i16 {
    let qe = (i64::from(a).wrapping_mul(935_520) >> 32) as i32;
    freezeq(a.wrapping_sub(qe.wrapping_mul(4591)))
}

/// `modq.Product`: `freezeq(a·b)`.
#[inline(always)]
pub(crate) fn productq(a: i16, b: i16) -> i16 {
    freezeq(i32::from(a).wrapping_mul(i32::from(b)))
}

/// The inverse of `a` modulo q, a^4589 = a^(q−2), by Go's fixed addition
/// chain with a reduction after every step (`modq.Reciprocal`,
/// `rq/modq/modq.go:27-46`). Zero maps to zero.
pub(crate) fn recipq(a1: i16) -> i16 {
    let a2 = productq(a1, a1);
    let a3 = productq(a2, a1);
    let a4 = productq(a2, a2);
    let a8 = productq(a4, a4);
    let a16 = productq(a8, a8);
    let a32 = productq(a16, a16);
    let a35 = productq(a32, a3);
    let a70 = productq(a35, a35);
    let a140 = productq(a70, a70);
    let a143 = productq(a140, a3);
    let a286 = productq(a143, a143);
    let a572 = productq(a286, a286);
    let a1144 = productq(a572, a572);
    let a1147 = productq(a1144, a3);
    let a2294 = productq(a1147, a1147);
    let a4588 = productq(a2294, a2294);
    productq(a4588, a1)
}

/// All ones when `x` is nonzero, else zero (for |x| < 2^31).
#[inline(always)]
pub(crate) fn nonzero_mask(x: i32) -> i32 {
    (x | x.wrapping_neg()) >> 31
}

/// Go's `smallerMask(x, y)`: −1 when x < y, else 0, as `(x − y) >> 31`.
///
/// Go computes it on 64-bit `int`; the operands here (the degree counters
/// of the reciprocal loops) stay within ±2·761, where the 32-bit form
/// gives the same value.
#[cfg(test)]
#[inline(always)]
pub(crate) fn smaller_mask(x: i32, y: i32) -> i32 {
    x.wrapping_sub(y) >> 31
}

/// Go's `swapInt(x, y, mask)`: exchanges `x` and `y` when `mask` is −1.
#[cfg(test)]
#[inline(always)]
pub(crate) fn swap_int(x: &mut i32, y: &mut i32, mask: i32) {
    let t = mask & (*x ^ *y);
    *x ^= t;
    *y ^= t;
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: reference arithmetic on small values"
    )]

    use super::*;

    /// The canonical residue of `a` in −(m−1)/2..=(m−1)/2.
    fn centered(a: i64, m: i64) -> i64 {
        let r = a.rem_euclid(m);
        if r > m / 2 { r - m } else { r }
    }

    #[test]
    fn freezeq_is_canonical_on_its_domain() {
        for a in -(1i32 << 23)..(1i32 << 23) {
            assert_eq!(
                i64::from(freezeq(a)),
                centered(i64::from(a), 4591),
                "freezeq({a})"
            );
        }
    }

    /// Every i32 whose low 12 bits are 0x000, 0x7ff or 0xfff, every one
    /// within 2^16 of zero or of either end, and every multiple of q ± 2295
    /// near the ends.
    #[test]
    fn freezeq_wide_is_canonical_on_a_sample() {
        let check = |a: i32| {
            assert_eq!(
                i64::from(freezeq_wide(a)),
                centered(i64::from(a), 4591),
                "freezeq_wide({a})"
            );
        };
        for hi in 0..(1u32 << 20) {
            for lo in [0x000u32, 0x7ff, 0xfff] {
                check(((hi << 12) | lo) as i32);
            }
        }
        for d in 0..(1i32 << 16) {
            for a in [d, -d, i32::MAX - d, i32::MIN + d] {
                check(a);
            }
        }
        let top = i32::MAX / 4591;
        for k in (top - 64)..=top {
            for off in [-2296, -2295, -1, 0, 1, 2295, 2296] {
                check((k * 4591).saturating_add(off));
                check((-k * 4591).saturating_add(off));
            }
        }
    }

    #[test]
    #[ignore = "2^32 reductions: run in a release build"]
    fn freezeq_wide_is_canonical_everywhere() {
        for a in i32::MIN..=i32::MAX {
            assert_eq!(
                i64::from(freezeq_wide(a)),
                centered(i64::from(a), 4591),
                "freezeq_wide({a})"
            );
        }
    }

    #[test]
    fn freeze3_is_canonical_on_its_domain() {
        for a in -(1i32 << 15)..=(1i32 << 15) {
            assert_eq!(
                i64::from(freeze3(a)),
                centered(i64::from(a), 3),
                "freeze3({a})"
            );
        }
    }

    #[test]
    fn recipq_inverts_every_nonzero_residue() {
        assert_eq!(recipq(0), 0);
        for a in -2295i16..=2295 {
            if a != 0 {
                assert_eq!(productq(a, recipq(a)), 1, "recipq({a})");
            }
        }
    }

    #[test]
    fn smaller_mask_and_swap() {
        assert_eq!(smaller_mask(1, 2), -1);
        assert_eq!(smaller_mask(2, 2), 0);
        assert_eq!(smaller_mask(3, 2), 0);
        assert_eq!(smaller_mask(-1523, 761), -1);
        let (mut x, mut y) = (5, 9);
        swap_int(&mut x, &mut y, 0);
        assert_eq!((x, y), (5, 9));
        swap_int(&mut x, &mut y, -1);
        assert_eq!((x, y), (9, 5));
    }
}
