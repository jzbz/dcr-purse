// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Small elements of R = Z[x]/(x^761 − x − 1): random generation and the
//! 191-byte encoding (Go's `zx` package, `zx/random.go`, `zx/encoding.go`).

use core::hint::black_box;

use zeroize::Zeroizing;

use crate::RandomSource;
use crate::arith::{P, W};

/// The length of an encoded small element.
pub(crate) const SMALL_BYTES: usize = 191;
/// The bytes one random element draws: four per coefficient.
pub(crate) const RANDOM_BYTES: usize = 4 * P;

/// Draws [`RANDOM_BYTES`] from `rng` and reads them as 761 little-endian
/// 32-bit words, in coefficient order (Go's `random32`, called 761 times).
fn random_words<R: RandomSource + ?Sized>(
    rng: &mut R,
    words: &mut [u32; P],
) -> Result<(), R::Error> {
    let mut buf = Zeroizing::new([0u8; RANDOM_BYTES]);
    rng.fill(&mut buf[..])?;
    for (w, b) in words.iter_mut().zip(buf.as_chunks::<4>().0.iter()) {
        *w = u32::from_le_bytes(*b);
    }
    Ok(())
}

/// A random small element: each coefficient −1, 0 or 1 from one 32-bit
/// word, `((w & 0x3fffffff)·3 >> 30) − 1` (`zx.RandomSmall`,
/// `zx/random.go:64-76`).
pub(crate) fn random_small<R: RandomSource + ?Sized>(
    g: &mut [i8; P],
    rng: &mut R,
) -> Result<(), R::Error> {
    let mut words = Zeroizing::new([0u32; P]);
    random_words(rng, &mut words)?;
    for (gi, &w) in g.iter_mut().zip(words.iter()) {
        // (w & 0x3fffffff)·3 < 2^32 and the shift leaves 0, 1 or 2.
        *gi = (((w & 0x3fff_ffff).wrapping_mul(3) >> 30) as i8).wrapping_sub(1);
    }
    Ok(())
}

/// A random t-small element: exactly 286 coefficients ±1, the rest 0
/// (`zx.RandomTSmall`, `zx/random.go:78-98`).
///
/// The first 286 words get bit 0 cleared (they become ±1), the others bit
/// 0 set and bit 1 cleared (they become 0); sorting all 761 words as
/// unsigned integers shuffles those marks by the random high bits, and the
/// low two bits of each sorted word, minus one, are the coefficients.
pub(crate) fn random_tsmall<R: RandomSource + ?Sized>(
    f: &mut [i8; P],
    rng: &mut R,
) -> Result<(), R::Error> {
    let mut r = Zeroizing::new([0u32; P]);
    random_words(rng, &mut r)?;
    for (i, ri) in r.iter_mut().enumerate() {
        if i < W {
            // Go: r[i] &= -2.
            *ri &= !1;
        } else {
            // Go: r[i] = (r[i] & -3) | 1.
            *ri = (*ri & !2) | 1;
        }
    }
    sort(&mut r);
    for (fi, &ri) in f.iter_mut().zip(r.iter()) {
        *fi = ((ri & 3) as i8).wrapping_sub(1);
    }
    Ok(())
}

/// Leaves the smaller of `x[a]`, `x[b]` (as unsigned 32-bit integers) in
/// `x[a]` and the larger in `x[b]`, in constant time.
///
/// Go's `minmax` exchanges when `subtle.ConstantTimeLessOrEq(int(uint32(y)),
/// int(uint32(x)))`; with Go 1.26 or later on a 64-bit target that is
/// `y <= x` on the zero-extended values, an unsigned comparison (spec 12
/// §5.10.2, Quirk Q40). Exchanging equal values changes nothing, so this is
/// exactly an unsigned compare-exchange. The mask is computed on 64 bits,
/// where `x − y` borrows exactly when y > x, and passed through
/// `black_box` so the optimiser cannot turn the select into a branch.
#[inline(always)]
fn minmax(x: &mut [u32; P], a: usize, b: usize) {
    let (xa, xb) = (x[a], x[b]);
    // Bit 63 of x − y is 1 exactly when y > x (both below 2^32).
    let borrow = u64::from(xa).wrapping_sub(u64::from(xb)) >> 63;
    // All ones when xb <= xa (exchange), zero otherwise.
    let mask = black_box((borrow as u32).wrapping_sub(1));
    let t = (xa ^ xb) & mask;
    x[a] = xa ^ t;
    x[b] = xb ^ t;
}

/// Go's sorting network for n = 761 (`zx/random.go:38-59`): the same
/// compare-exchange sequence, so the same output for any comparison.
fn sort(x: &mut [u32; P]) {
    const N: usize = P;
    let mut top = 1usize;
    while top < N.wrapping_sub(top) {
        top = top.wrapping_add(top);
    }
    let mut p = top;
    while p > 0 {
        for i in 0..N.wrapping_sub(p) {
            if i & p == 0 {
                minmax(x, i, i.wrapping_add(p));
            }
        }
        let mut q = top;
        while q > p {
            for i in 0..N.wrapping_sub(q) {
                if i & p == 0 {
                    minmax(x, i.wrapping_add(p), i.wrapping_add(q));
                }
            }
            q >>= 1;
        }
        p >>= 1;
    }
}

/// Packs a small element into 191 bytes, four coefficients per byte, two
/// bits each as c + 1, the last byte holding coefficient 760 alone
/// (`zx.Encode`, `zx/encoding.go:10-25`).
///
/// Go adds in `int8`, so a coefficient outside {−1, 0, 1} wraps as there;
/// every caller passes a canonical one.
pub(crate) fn encode(f: &[i8; P], c: &mut [u8; SMALL_BYTES]) {
    for (ci, q) in c.iter_mut().zip(f.as_chunks::<4>().0.iter()) {
        let mut c0 = q[0].wrapping_add(1);
        c0 = c0.wrapping_add(q[1].wrapping_add(1).wrapping_shl(2));
        c0 = c0.wrapping_add(q[2].wrapping_add(1).wrapping_shl(4));
        c0 = c0.wrapping_add(q[3].wrapping_add(1).wrapping_shl(6));
        *ci = c0 as u8;
    }
    c[190] = f[760].wrapping_add(1) as u8;
}

/// Unpacks 191 bytes into a small element (`zx.Decode`,
/// `zx/encoding.go:28-46`). It does not validate: the two-bit value 3
/// decodes to the coefficient 2, and the top six bits of the last byte are
/// ignored, as in Go.
pub(crate) fn decode(c: &[u8; SMALL_BYTES], f: &mut [i8; P]) {
    for (q, &byte) in f.as_chunks_mut::<4>().0.iter_mut().zip(c.iter()) {
        let mut c0 = byte;
        for fj in q.iter_mut() {
            *fj = ((c0 & 3) as i8).wrapping_sub(1);
            c0 >>= 2;
        }
    }
    f[760] = ((c[190] & 3) as i8).wrapping_sub(1);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: arithmetic on small values"
    )]

    use super::*;

    #[test]
    fn sort_is_an_unsigned_ascending_sort() {
        let mut rng = purse_testutil::SplitMix64(0x736f_7274);
        for round in 0..200 {
            let mut x = [0u32; P];
            for xi in &mut x {
                let v = rng.next_u64() as u32;
                // Some rounds use few distinct values, to exercise ties.
                *xi = if round % 4 == 0 { v % 7 } else { v };
            }
            let mut want = x;
            want.sort_unstable();
            sort(&mut x);
            assert_eq!(x, want, "round {round}");
        }
    }

    #[test]
    fn tsmall_has_weight_286_and_draws_3044_bytes() {
        let mut stream = purse_testutil::SplitMix64(0x7473_6d61_6c6c);
        let mut drawn = 0usize;
        let mut src = |buf: &mut [u8]| -> Result<(), ()> {
            drawn += buf.len();
            stream.fill(buf);
            Ok(())
        };
        for _ in 0..20 {
            let mut f = [0i8; P];
            random_tsmall(&mut f, &mut src).unwrap();
            assert_eq!(f.iter().filter(|&&c| c != 0).count(), W);
            assert!(f.iter().all(|&c| (-1..=1).contains(&c)));
        }
        assert_eq!(drawn, 20 * RANDOM_BYTES);
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut rng = purse_testutil::SplitMix64(0x7a78);
        for _ in 0..50 {
            let mut f = [0i8; P];
            for fi in &mut f {
                *fi = (rng.below(3) as i8) - 1;
            }
            let mut c = [0u8; SMALL_BYTES];
            encode(&f, &mut c);
            let mut back = [0i8; P];
            decode(&c, &mut back);
            assert_eq!(back, f);
            // Unused bits of the last byte are zero.
            assert_eq!(c[190] & !3, 0);
        }
    }
}
