// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Rq = GF(q)[x]/(x^761 − x − 1), q = 4591: multiplication by a small
//! element, the reciprocal of 3·f, rounding, and the two encodings (Go's
//! `rq` package, `rq/rq.go`, `rq/encoding.go`).
//!
//! Coefficients are `i16`, canonical in −2295..=2295 wherever this crate
//! computes them. The decoders reproduce Go exactly on every input,
//! including malformed bytes, where they can return coefficients outside
//! that range; [`mult`] reduces every product, so its output is canonical
//! whatever its `i16` input (see its documentation).

use zeroize::Zeroizing;

use crate::arith::{P, QSHIFT, freezeq, freezeq_wide, nonzero_mask, productq, recipq};
use crate::conv;

/// The length of an encoded element of Rq (a public key).
pub(crate) const ENCODED_BYTES: usize = 1218;
/// The length of an encoded rounded element (a ciphertext's body).
pub(crate) const ROUNDED_BYTES: usize = 1015;

/// h = f·g in Rq (`rq.Mult`, `rq/rq.go:79-102`).
///
/// Go reduces after every term, and because every partial sum is reduced,
/// each of its reductions is exact for any `i16` `f` and any `i8` `g`
/// (|r| ≤ 2295 plus |f·g| ≤ 2^22, below the 2^23 where [`freezeq`] is
/// exact): Go's result is the true product of `f` mod q and `g`, in
/// canonical form, whatever `f` holds, including the out-of-range
/// coefficients the decoders give for malformed bytes. This computes that
/// product directly: `f` reduced to −2295..=2295 (exact, |f| < 2^15),
/// the integer product by [`conv::mul`] (|g| ≤ 128 keeps every sum below
/// 2^31), and one reduction of each coefficient by [`freezeq_wide`].
/// `reference::rq_mult`, Go's order statement for statement, is the test
/// model it is checked against.
pub(crate) fn mult(h: &mut [i16; P], f: &[i16; P], g: &[i8; P]) {
    let mut a = Zeroizing::new([0i16; P]);
    for (ai, &fi) in a.iter_mut().zip(f.iter()) {
        *ai = freezeq(i32::from(fi));
    }
    let mut b = Zeroizing::new([0i16; P]);
    for (bi, &gi) in b.iter_mut().zip(g.iter()) {
        *bi = i16::from(gi);
    }
    let mut acc = Zeroizing::new([0i32; P]);
    conv::mul(&mut acc, &a, &b);
    for (hi, &x) in h.iter_mut().zip(acc.iter()) {
        *hi = freezeq_wide(x);
    }
}

/// 2^16 mod q: the Montgomery radix.
const MONT_R: i16 = 1262;
/// q^−1 mod 2^16.
const QINV: i16 = 15631;

/// a·b·2^−16 mod q, in −q+1..=q−1, for |a·b| < q·2^15 (Montgomery
/// reduction in 16-bit lanes), given `a_qinv` = a·q^−1 mod 2^16: with
/// m = a·b·q^−1 mod 2^16, a·b − m·q is divisible by 2^16, so the high
/// halves of a·b and m·q differ by exactly (a·b − m·q)/2^16, whose
/// magnitude is below |a·b|/2^16 + q/2. Three 16-bit multiplications with
/// no division and no branch; LLVM vectorises the loops that use it (on
/// baseline x86-64, in 32-bit lanes: `pmaddwd`, `pmuludq`).
#[inline(always)]
pub(crate) fn mont(a: i16, a_qinv: i16, b: i16) -> i16 {
    let ab_hi = (i32::from(a).wrapping_mul(i32::from(b)) >> 16) as i16;
    let m = b.wrapping_mul(a_qinv);
    let mq_hi = (i32::from(m).wrapping_mul(4591) >> 16) as i16;
    ab_hi.wrapping_sub(mq_hi)
}

/// The bound every coefficient of the working arrays of [`reciprocal3`]
/// keeps: if |x|, |y| ≤ 5234 and |a|, |b| ≤ 2295, then
/// |mont(a, x) − mont(b, y)| ≤ 2·(2295·5234/2^16 + 2295.5) < 4958.
#[cfg(test)]
pub(crate) const LOOSE: i16 = 5234;

/// r = 1/(3·s) in Rq (`rq.Reciprocal3`, `rq/rq.go:31-70`). Returns 0 when
/// 3·s is invertible and −1 otherwise; then `r` holds no inverse, and not
/// what Go leaves there. Every nonzero element of Rq is invertible
/// (x^761 − x − 1 is irreducible mod q), and key generation, like Go's,
/// passes a t-small f and does not look at the result.
///
/// Go runs 1523 steps of an extended Euclidean loop over arrays of 762 and
/// 1524 coefficients, with a modular inversion in every step
/// (`reference::rq_reciprocal3`). This is the constant-time divstep
/// inversion of the NTRU Prime round-2 reference code (`Rq_recip3`, public
/// domain): 2·761 − 1 steps over four arrays of 762 coefficients, each step
/// g ← (f0·g − g0·f)/x and w ← f0·w − g0·v, with no inversion until the
/// end. The products are 16-bit Montgomery products ([`mont`]) against f0
/// and g0 scaled by 2^16, and the arrays are kept loosely reduced (within
/// ±5234, `LOOSE`); only f0 and g0 are reduced fully, once per step. The
/// swaps are fused into the updates, and v and w are processed only up to
/// the degree they can have after k steps, k + 1, which depends on the step
/// number alone. The input polynomial is Go's, `int16(3·s[i])` with the
/// product wrapping in `int8`; the inverse of a polynomial is unique, so
/// the results are equal whenever there is one (checked against
/// `reference::rq_reciprocal3` and against Go). The number of steps is
/// fixed, and no branch or index depends on `s`: the swap is a mask
/// (through `black_box`, so the compiler cannot turn it into a branch).
pub(crate) fn reciprocal3(r: &mut [i16; P], s: &[i8; P]) -> i32 {
    // Reversed polynomials, as in `r3::reciprocal`: f = 1 − x^760 − x^761,
    // g = 3·s reversed.
    let mut f = Zeroizing::new([0i16; P + 1]);
    f[0] = 1;
    f[P - 1] = -1;
    f[P] = -1;
    let mut g = Zeroizing::new([0i16; P + 1]);
    for (gi, &si) in g[..P].iter_mut().rev().zip(s.iter()) {
        *gi = freezeq(i32::from(3i8.wrapping_mul(si)));
    }
    let mut v = Zeroizing::new([0i16; P + 1]);
    let mut w = Zeroizing::new([0i16; P + 1]);
    w[0] = 1;
    let mut delta: i32 = 1;

    for k in 0..2 * P - 1 {
        // v and w have degree at most k before this step and k + 1 after
        // it (each step multiplies v by x), so only their first `live`
        // coefficients can be nonzero; `live` depends on the step number
        // alone.
        let live = k.wrapping_add(2).min(P + 1);
        // v ← x·v, dropping the coefficient of x^762.
        v.copy_within(..live.wrapping_sub(1), 1);
        v[0] = 0;
        // Swap (f, g) and (v, w) when delta > 0 and g0 ≠ 0.
        let f0 = freezeq(i32::from(f[0]));
        let g0 = freezeq(i32::from(g[0]));
        let swap =
            core::hint::black_box((delta.wrapping_neg() >> 31) & nonzero_mask(i32::from(g0)));
        delta ^= swap & (delta ^ delta.wrapping_neg());
        delta = delta.wrapping_add(1);
        let m = swap as i16;
        // f0 and g0 after the swap.
        let t = m & (f0 ^ g0);
        let (f0, g0) = (f0 ^ t, g0 ^ t);
        let (f0r, g0r) = (productq(f0, MONT_R), productq(g0, MONT_R));
        let (f0q, g0q) = (f0r.wrapping_mul(QINV), g0r.wrapping_mul(QINV));
        // The swap of f and g, fused with g ← (f0·g − g0·f)/x: coefficient
        // 0 of f0·g − g0·f cancels, and g[i] is written from coefficient
        // i + 1, after it was read.
        let t = m & (f[0] ^ g[0]);
        f[0] ^= t;
        for i in 0..P {
            let j = i.wrapping_add(1);
            let t = m & (f[j] ^ g[j]);
            let (fj, gj) = (f[j] ^ t, g[j] ^ t);
            f[j] = fj;
            g[i] = mont(f0r, f0q, gj).wrapping_sub(mont(g0r, g0q, fj));
        }
        g[P] = 0;
        // The swap of v and w, fused with w ← f0·w − g0·v.
        for (vi, wi) in v[..live].iter_mut().zip(w[..live].iter_mut()) {
            let t = m & (*vi ^ *wi);
            let (a, b) = (*vi ^ t, *wi ^ t);
            *vi = a;
            *wi = mont(f0r, f0q, b).wrapping_sub(mont(g0r, g0q, a));
        }
    }

    // r = (v reversed)/f0.
    let scale = recipq(freezeq(i32::from(f[0])));
    for (ri, &vi) in r.iter_mut().zip(v[..P].iter().rev()) {
        *ri = productq(scale, freezeq(i32::from(vi)));
    }
    nonzero_mask(delta)
}

/// Rounds each coefficient to the nearest multiple of 3, in place
/// (`rq.Round3`, `rq/rq.go:72-76`). Go adds 2295 in `int16`.
pub(crate) fn round3(h: &mut [i16; P]) {
    for hi in h.iter_mut() {
        let x = i32::from(hi.wrapping_add(QSHIFT as i16));
        let rounded = (21846i32.wrapping_mul(x).wrapping_add(32768) >> 16)
            .wrapping_mul(3)
            .wrapping_sub(QSHIFT);
        *hi = rounded as i16;
    }
}

/// Packs an element of Rq into 1218 bytes: five coefficients in eight
/// bytes, the last one in two (`rq.Encode`, `rq/encoding.go:14-60`).
pub(crate) fn encode(f: &[i16; P], c: &mut [u8; ENCODED_BYTES]) {
    for (q, out) in f
        .as_chunks::<5>()
        .0
        .iter()
        .zip(c.as_chunks_mut::<8>().0.iter_mut())
    {
        let f1 = i32::from(q[1]).wrapping_add(QSHIFT).wrapping_mul(3);
        let f2 = i32::from(q[2]).wrapping_add(QSHIFT).wrapping_mul(9);
        let f3 = i32::from(q[3]).wrapping_add(QSHIFT).wrapping_mul(27);
        let f4 = i32::from(q[4]).wrapping_add(QSHIFT).wrapping_mul(81);
        let mut f0 = i32::from(q[0]).wrapping_add(QSHIFT);
        f0 = f0.wrapping_add(f1.wrapping_shl(11));
        out[0] = f0 as u8;
        f0 >>= 8;
        out[1] = f0 as u8;
        f0 >>= 8;
        f0 = f0.wrapping_add(f2.wrapping_shl(6));
        out[2] = f0 as u8;
        f0 >>= 8;
        out[3] = f0 as u8;
        f0 >>= 8;
        f0 = f0.wrapping_add(f3.wrapping_shl(1));
        out[4] = f0 as u8;
        f0 >>= 8;
        f0 = f0.wrapping_add(f4.wrapping_shl(4));
        out[5] = f0 as u8;
        f0 >>= 8;
        out[6] = f0 as u8;
        f0 >>= 8;
        out[7] = f0 as u8;
    }
    let f0 = i32::from(f[760]).wrapping_add(QSHIFT);
    c[1216] = f0 as u8;
    c[1217] = (f0 >> 8) as u8;
}

/// `int32(x + q − qshift)` then [`freezeq`]: the last step of each decoded
/// coefficient, on Go's `uint32`.
#[inline(always)]
fn freeze_shifted(x: u32) -> i16 {
    const Q_MINUS_SHIFT: u32 = 4591 - 2295;
    freezeq(x.wrapping_add(Q_MINUS_SHIFT) as i32)
}

/// Unpacks 1218 bytes into an element of Rq (`rq.Decode`,
/// `rq/encoding.go:63-127`), on Go's wrapping `uint32` arithmetic. It does
/// not validate: an out-of-range encoding gives whatever Go's arithmetic
/// gives, which may be a non-canonical `i16`.
pub(crate) fn decode(c: &[u8; ENCODED_BYTES], f: &mut [i16; P]) {
    for (b, out) in c
        .as_chunks::<8>()
        .0
        .iter()
        .zip(f.as_chunks_mut::<5>().0.iter_mut())
    {
        let mut c0 = u32::from(b[0]);
        let mut c1 = u32::from(b[1]);
        let mut c2 = u32::from(b[2]);
        let mut c3 = u32::from(b[3]);
        let mut c4 = u32::from(b[4]);
        let mut c5 = u32::from(b[5]);
        let mut c6 = u32::from(b[6]);
        let c7 = u32::from(b[7]);

        c6 = c6.wrapping_add(c7.wrapping_shl(8));
        let f4 = 103_564u32
            .wrapping_mul(c6)
            .wrapping_add(405u32.wrapping_mul(c5.wrapping_add(1)))
            >> 19;
        c5 = c5.wrapping_add(c6.wrapping_shl(8));
        c5 = c5.wrapping_sub(f4.wrapping_mul(81).wrapping_shl(4));
        c4 = c4.wrapping_add(c5.wrapping_shl(8));
        let f3 = 9709u32.wrapping_mul(c4.wrapping_add(2)) >> 19;
        c4 = c4.wrapping_sub(f3.wrapping_mul(27).wrapping_shl(1));
        c3 = c3.wrapping_add(c4.wrapping_shl(8));
        let f2 = 233_017u32
            .wrapping_mul(c3)
            .wrapping_add(910u32.wrapping_mul(c2.wrapping_add(2)))
            >> 19;
        c2 = c2.wrapping_add(c3.wrapping_shl(8));
        c2 = c2.wrapping_sub(f2.wrapping_mul(9).wrapping_shl(6));
        c1 = c1.wrapping_add(c2.wrapping_shl(8));
        let f1 = 21845u32
            .wrapping_mul(c1.wrapping_add(2))
            .wrapping_add(85u32.wrapping_mul(c0))
            >> 19;
        c1 = c1.wrapping_sub(f1.wrapping_mul(3).wrapping_shl(3));
        c0 = c0.wrapping_add(c1.wrapping_shl(8));
        let f0 = c0;

        out[0] = freeze_shifted(f0);
        out[1] = freeze_shifted(f1);
        out[2] = freeze_shifted(f2);
        out[3] = freeze_shifted(f3);
        out[4] = freeze_shifted(f4);
    }
    let c0 = u32::from(c[1216]).wrapping_add(u32::from(c[1217]).wrapping_shl(8));
    f[760] = freeze_shifted(c0);
}

/// Packs an element rounded by [`round3`] into 1015 bytes: three
/// coefficients in four bytes, each as a digit (x + 2295)/3 below 1531,
/// combined in radix 1536; the last two in three bytes
/// (`rq.EncodeRounded`, `rq/encoding.go:130-177`).
pub(crate) fn encode_rounded(f: &[i16; P], c: &mut [u8; ROUNDED_BYTES]) {
    let third = |x: i16| 21846i32.wrapping_mul(i32::from(x).wrapping_add(QSHIFT)) >> 16;
    for (q, out) in f
        .as_chunks::<3>()
        .0
        .iter()
        .zip(c.as_chunks_mut::<4>().0.iter_mut())
    {
        let mut f0 = third(q[0]);
        let mut f1 = third(q[1]);
        let mut f2 = third(q[2]);
        f2 = f2.wrapping_mul(3);
        f1 = f1.wrapping_add(f2.wrapping_shl(9));
        f1 = f1.wrapping_mul(3);
        f0 = f0.wrapping_add(f1.wrapping_shl(9));
        out[0] = f0 as u8;
        f0 >>= 8;
        out[1] = f0 as u8;
        f0 >>= 8;
        out[2] = f0 as u8;
        f0 >>= 8;
        out[3] = f0 as u8;
    }
    let mut f0 = third(f[759]);
    let mut f1 = third(f[760]);
    f1 = f1.wrapping_mul(3);
    f0 = f0.wrapping_add(f1.wrapping_shl(9));
    c[1012] = f0 as u8;
    f0 >>= 8;
    c[1013] = f0 as u8;
    f0 >>= 8;
    c[1014] = f0 as u8;
}

/// `int32(x·3 + q − qshift)` then [`freezeq`], on Go's `uint32`.
#[inline(always)]
fn freeze_rounded(x: u32) -> i16 {
    freeze_shifted(x.wrapping_mul(3))
}

/// Unpacks 1015 bytes into a rounded element (`rq.DecodeRounded`,
/// `rq/encoding.go:180-231`), on Go's wrapping `uint32` arithmetic. It does
/// not validate: values beyond the encoder's range decode to whatever Go's
/// arithmetic gives, reduced by [`freezeq`].
pub(crate) fn decode_rounded(c: &[u8; ROUNDED_BYTES], f: &mut [i16; P]) {
    for (b, out) in c
        .as_chunks::<4>()
        .0
        .iter()
        .zip(f.as_chunks_mut::<3>().0.iter_mut())
    {
        *out = decode_rounded_group(*b);
    }
    let [f759, f760] = decode_rounded_tail([c[1012], c[1013], c[1014]]);
    f[759] = f759;
    f[760] = f760;
}

/// One four-byte group of [`decode_rounded`]: three coefficients.
#[inline(always)]
pub(crate) fn decode_rounded_group(b: [u8; 4]) -> [i16; 3] {
    let mut c0 = u32::from(b[0]);
    let mut c1 = u32::from(b[1]);
    let mut c2 = u32::from(b[2]);
    let c3 = u32::from(b[3]);

    let f2 = 14_913_081u32
        .wrapping_mul(c3)
        .wrapping_add(58_254u32.wrapping_mul(c2))
        .wrapping_add(228u32.wrapping_mul(c1.wrapping_add(2)))
        >> 21;
    c2 = c2.wrapping_add(c3.wrapping_shl(8));
    c2 = c2.wrapping_sub(f2.wrapping_mul(9).wrapping_shl(2));
    let f1 = 89_478_485u32
        .wrapping_mul(c2)
        .wrapping_add(349_525u32.wrapping_mul(c1))
        .wrapping_add(1365u32.wrapping_mul(c0.wrapping_add(1)))
        >> 21;
    c1 = c1.wrapping_add(c2.wrapping_shl(8));
    c1 = c1.wrapping_sub(f1.wrapping_mul(3).wrapping_shl(1));
    c0 = c0.wrapping_add(c1.wrapping_shl(8));
    let f0 = c0;
    [freeze_rounded(f0), freeze_rounded(f1), freeze_rounded(f2)]
}

/// The last three bytes of [`decode_rounded`]: coefficients 759 and 760.
/// Not injective: 964,530 of the 2^24 values decode to the same valid pair
/// as another (QK-0740; `rounded_tail_aliases_exhaustive`).
#[inline(always)]
pub(crate) fn decode_rounded_tail(b: [u8; 3]) -> [i16; 2] {
    let mut c0 = u32::from(b[0]);
    let mut c1 = u32::from(b[1]);
    let c2 = u32::from(b[2]);
    let f1 = 89_478_485u32
        .wrapping_mul(c2)
        .wrapping_add(349_525u32.wrapping_mul(c1))
        .wrapping_add(1365u32.wrapping_mul(c0.wrapping_add(1)))
        >> 21;
    c1 = c1.wrapping_add(c2.wrapping_shl(8));
    c1 = c1.wrapping_sub(f1.wrapping_mul(3).wrapping_shl(1));
    c0 = c0.wrapping_add(c1.wrapping_shl(8));
    let f0 = c0;
    [freeze_rounded(f0), freeze_rounded(f1)]
}
