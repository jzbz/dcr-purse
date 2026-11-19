// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! An independent model of arithmetic modulo 2¹²⁷ − 1 for the tests, sharing
//! no code with the crate: products are formed on 32-bit limbs into a
//! 256-bit value (eight `u32` limbs, little-endian) and reduced either by
//! bitwise long division (`rem_slow`, which works for any modulus and knows
//! nothing of its form) or by splitting the 256-bit value at bit 127
//! (`mul_mod`, fast enough to expand degree-1024 products in a debug build;
//! checked against `rem_slow` in the crate's unit tests). Polynomial
//! remainders (`poly_rem`) are schoolbook long division on top of these.
//!
//! Also included by the crate's unit tests (`src/tests.rs`, `#[path]`).

#![allow(
    dead_code,
    reason = "each test crate uses a different part of the model"
)]

/// The modulus 2¹²⁷ − 1, written out rather than computed by shifting.
pub const P: u128 = 0x7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;

/// A 256-bit value as eight 32-bit limbs, least significant first.
pub type U256 = [u32; 8];

fn limbs(a: u128) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (i, limb) in out.iter_mut().enumerate() {
        *limb = (a >> (32 * i)) as u32;
    }
    out
}

/// The full product of two `u128` values, schoolbook on 32-bit limbs.
pub fn mul_full(a: u128, b: u128) -> U256 {
    let (a, b) = (limbs(a), limbs(b));
    let mut out = [0u32; 8];
    for i in 0..4 {
        let mut carry = 0u64;
        for j in 0..4 {
            let t = u64::from(a[i]) * u64::from(b[j]) + u64::from(out[i + j]) + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        // Row i's final carry lands in limb i + 4, untouched so far.
        out[i + 4] = carry as u32;
    }
    out
}

/// The 256-bit value of a `u128`.
pub fn widen(a: u128) -> U256 {
    let l = limbs(a);
    [l[0], l[1], l[2], l[3], 0, 0, 0, 0]
}

fn bit(x: &U256, i: usize) -> u128 {
    u128::from((x[i / 32] >> (i % 32)) & 1)
}

/// `x mod m` by binary long division, for any modulus 0 < m < 2¹²⁷.
pub fn rem_slow(x: &U256, m: u128) -> u128 {
    assert!(m != 0 && m < (1u128 << 127));
    let mut r = 0u128;
    for i in (0..256).rev() {
        // r < m < 2^127, so 2r + 1 < 2^128.
        r = (r << 1) | bit(x, i);
        if r >= m {
            r -= m;
        }
    }
    r
}

/// `x mod P` for x < 2²⁵⁴, by x = H·2¹²⁷ + L ≡ H + L.
fn rem_p(x: &U256) -> u128 {
    assert!(x[7] >> 30 == 0, "product too large");
    let half = |l: &[u32]| {
        l.iter()
            .rev()
            .fold(0u128, |acc, &limb| (acc << 32) | u128::from(limb))
    };
    let (lo, hi) = (half(&x[..4]), half(&x[4..]));
    // L = the low 127 bits; H = x >> 127 = hi·2 + the top bit of lo
    // (hi < 2¹²⁶, so the shift loses nothing).
    let low = lo & P;
    let high = (hi << 1) | (lo >> 127);
    let s = low + high;
    if s >= P { s - P } else { s }
}

/// `a · b mod P` for canonical a, b.
pub fn mul_mod(a: u128, b: u128) -> u128 {
    assert!(a < P && b < P, "non-canonical operand");
    rem_p(&mul_full(a, b))
}

/// `a · b mod P` by long division (slow).
pub fn mul_mod_slow(a: u128, b: u128) -> u128 {
    rem_slow(&mul_full(a, b), P)
}

/// `a + b mod P` for canonical a, b.
pub fn add_mod(a: u128, b: u128) -> u128 {
    assert!(a < P && b < P);
    let s = a + b;
    if s >= P { s - P } else { s }
}

/// `a − b mod P` for canonical a, b.
pub fn sub_mod(a: u128, b: u128) -> u128 {
    assert!(a < P && b < P);
    if a >= b { a - b } else { P - (b - a) }
}

/// `−a mod P` for canonical a.
pub fn neg_mod(a: u128) -> u128 {
    sub_mod(0, a)
}

/// `a^e mod P`, right to left.
pub fn pow_mod(mut a: u128, mut e: u128) -> u128 {
    let mut r = 1u128;
    while e != 0 {
        if e & 1 == 1 {
            r = mul_mod(r, a);
        }
        a = mul_mod(a, a);
        e >>= 1;
    }
    r
}

/// `t mod g` by schoolbook long division with model arithmetic only, as
/// `deg g` coefficients (zero-padded); `g` must have a nonzero top
/// coefficient.
pub fn poly_rem(t: &[u128], g: &[u128]) -> Vec<u128> {
    let dg = g.len() - 1;
    assert!(g[dg] != 0, "divisor with a zero top coefficient");
    let inv = pow_mod(g[dg], P - 2);
    let mut r = t.to_vec();
    while r.len() > dg {
        let top = r.pop().expect("nonempty");
        // r had degree shift + dg: subtract top/lead · x^shift · g.
        let shift = r.len() - dg;
        let c = mul_mod(top, inv);
        for (j, &gj) in g[..dg].iter().enumerate() {
            r[shift + j] = sub_mod(r[shift + j], mul_mod(c, gj));
        }
    }
    r.resize(dg, 0);
    r
}

/// The monic polynomial ∏ (x − r), coefficients ascending.
pub fn expand(roots: &[u128]) -> Vec<u128> {
    let mut c = vec![1u128];
    for &r in roots {
        // c·(x − r): new[k] = c[k−1] − r·c[k].
        let mut next = vec![0u128; c.len() + 1];
        for (k, &ck) in c.iter().enumerate() {
            next[k + 1] = add_mod(next[k + 1], ck);
            next[k] = sub_mod(next[k], mul_mod(r, ck));
        }
        c = next;
    }
    c
}

/// Every coefficient times `s`.
pub fn scale(c: &[u128], s: u128) -> Vec<u128> {
    c.iter().map(|&x| mul_mod(x, s)).collect()
}

/// The product of two polynomials.
pub fn poly_mul(a: &[u128], b: &[u128]) -> Vec<u128> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u128; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            out[i + j] = add_mod(out[i + j], mul_mod(x, y));
        }
    }
    out
}

/// `a(x)`, term by term with explicit powers (not Horner).
pub fn eval(a: &[u128], x: u128) -> u128 {
    let mut sum = 0u128;
    let mut power = 1u128;
    for &c in a {
        sum = add_mod(sum, mul_mod(c, power));
        power = mul_mod(power, x);
    }
    sum
}

/// Whether `a` is a nonzero square modulo P (Euler's criterion).
pub fn is_square(a: u128) -> bool {
    a != 0 && pow_mod(a, (P - 1) / 2) == 1
}
