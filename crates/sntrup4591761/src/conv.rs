// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Exact integer multiplication in Z[x]/(x^761 − x − 1): the kernel of both
//! ring multiplications, [`crate::rq::mult`] and [`crate::r3::mult`].
//!
//! Go reduces after every one of the 579,121 products (`rq.Mult`,
//! `r3.Mult`), a serial chain of reductions. The result is the same if the
//! integer products are summed first and reduced once, as long as no sum
//! overflows; the callers bring their operands into a range where none can
//! (see [`mul`]) and reduce the 761 sums at the end. Each coefficient of
//! the product is then a dot product of one operand with a window of the
//! other reversed, a loop of independent 16-bit multiplications summed in
//! 32 bits, which compilers turn into vector multiply-add instructions
//! (`pmaddwd` on x86-64, `smlal` on AArch64) without any `unsafe`.
//!
//! Nothing here depends on a coefficient's value: the loop bounds and the
//! windows depend only on the index.

use zeroize::Zeroizing;

use crate::arith::P;

/// `out = a·b` in Z[x]/(x^761 − x − 1), exactly, in 32-bit integers.
///
/// Exact when no coefficient sum overflows `i32`: every coefficient of the
/// unreduced product is a sum of at most 761 terms, and every coefficient
/// of the reduced one a sum of at most three of those, so
/// `3 · 761 · max|a| · max|b| < 2^31` suffices. The callers pass
/// max|a| ≤ 2295 and max|b| ≤ 128 (`rq`: 670,654,080), or max|a| =
/// max|b| = 1 (`r3`: 2,283).
pub(crate) fn mul(out: &mut [i32; P], a: &[i16; P], b: &[i16; P]) {
    // b reversed: the coefficient k of a·b, the sum over i of a[i]·b[k − i],
    // is then a dot product of a[lo..=hi] with a contiguous window of brev.
    let mut brev = Zeroizing::new([0i16; P]);
    for (d, &s) in brev.iter_mut().zip(b.iter().rev()) {
        *d = s;
    }
    let mut full = Zeroizing::new([0i32; 2 * P - 1]);
    for (k, fk) in full.iter_mut().enumerate() {
        // i runs over lo..=hi; b[k − i] = brev[P − 1 − k + i].
        let lo = k.saturating_sub(P - 1);
        let hi = if k < P { k } else { P - 1 };
        let len = hi.wrapping_sub(lo).wrapping_add(1);
        let at = (P - 1).wrapping_add(lo).wrapping_sub(k);
        *fk = dot(
            &a[lo..lo.wrapping_add(len)],
            &brev[at..at.wrapping_add(len)],
        );
    }
    // x^761 = x + 1: the top coefficients fold into the bottom ones. A
    // coefficient P + j lands on j and j + 1, both below P, so the order of
    // the folds does not matter.
    for i in (P..2 * P - 1).rev() {
        let top = full[i];
        let j = i.wrapping_sub(P);
        full[j] = full[j].wrapping_add(top);
        full[j.wrapping_add(1)] = full[j.wrapping_add(1)].wrapping_add(top);
    }
    out.copy_from_slice(&full[..P]);
}

/// The dot product of two equally long slices, summed in `i32`.
#[inline(always)]
fn dot(x: &[i16], y: &[i16]) -> i32 {
    x.iter().zip(y.iter()).fold(0i32, |s, (&a, &b)| {
        s.wrapping_add(i32::from(a).wrapping_mul(i32::from(b)))
    })
}
