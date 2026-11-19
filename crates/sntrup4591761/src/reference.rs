// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The test model: Go's multiplications and reciprocals, statement for
//! statement (`rq/rq.go`, `r3/r3.go` and their `vector` packages), which
//! the crate shipped before the arithmetic was optimised. The optimised
//! [`crate::rq`] and [`crate::r3`] are checked against these on arbitrary
//! inputs (`src/tests.rs`), and these against Go by the oracle; the
//! component benchmark times one against the other.

use crate::arith::{P, freeze3, freezeq, productq, recipq, smaller_mask, swap_int};

/// `rq.Mult` (`rq/rq.go:79-102`): a schoolbook product with a reduction
/// after every term, then the reduction by x^761 = x + 1 from the top.
pub(crate) fn rq_mult(h: &mut [i16; P], f: &[i16; P], g: &[i8; P]) {
    let mut fg = [0i16; 2 * P - 1];
    for i in 0..P {
        let mut r = 0i16;
        for j in 0..=i {
            r = freezeq(
                i32::from(r)
                    .wrapping_add(i32::from(f[j]).wrapping_mul(i32::from(g[i.wrapping_sub(j)]))),
            );
        }
        fg[i] = r;
    }
    for i in P..2 * P - 1 {
        let mut r = 0i16;
        for j in (i.wrapping_sub(P).wrapping_add(1))..P {
            r = freezeq(
                i32::from(r)
                    .wrapping_add(i32::from(f[j]).wrapping_mul(i32::from(g[i.wrapping_sub(j)]))),
            );
        }
        fg[i] = r;
    }
    for i in (P..2 * P - 1).rev() {
        fg[i.wrapping_sub(P)] =
            freezeq(i32::from(fg[i.wrapping_sub(P)]).wrapping_add(i32::from(fg[i])));
        fg[i.wrapping_sub(P).wrapping_add(1)] = freezeq(
            i32::from(fg[i.wrapping_sub(P).wrapping_add(1)]).wrapping_add(i32::from(fg[i])),
        );
    }
    h.copy_from_slice(&fg[..P]);
}

/// `r3.Mult` (`r3/r3.go:73-96`), in the same order as [`rq_mult`].
pub(crate) fn r3_mult(h: &mut [i8; P], f: &[i8; P], g: &[i8; P]) {
    let mut fg = [0i8; 2 * P - 1];
    for i in 0..P {
        let mut r = 0i8;
        for j in 0..=i {
            r = freeze3(
                i32::from(r)
                    .wrapping_add(i32::from(f[j]).wrapping_mul(i32::from(g[i.wrapping_sub(j)]))),
            );
        }
        fg[i] = r;
    }
    for i in P..2 * P - 1 {
        let mut r = 0i8;
        for j in (i.wrapping_sub(P).wrapping_add(1))..P {
            r = freeze3(
                i32::from(r)
                    .wrapping_add(i32::from(f[j]).wrapping_mul(i32::from(g[i.wrapping_sub(j)]))),
            );
        }
        fg[i] = r;
    }
    for i in (P..2 * P - 1).rev() {
        fg[i.wrapping_sub(P)] =
            freeze3(i32::from(fg[i.wrapping_sub(P)]).wrapping_add(i32::from(fg[i])));
        fg[i.wrapping_sub(P).wrapping_add(1)] = freeze3(
            i32::from(fg[i.wrapping_sub(P).wrapping_add(1)]).wrapping_add(i32::from(fg[i])),
        );
    }
    h.copy_from_slice(&fg[..P]);
}

/// `rq.Reciprocal3` (`rq/rq.go:31-70`): 1/(3·s) by Go's extended Euclidean
/// loop, 1523 iterations over arrays of 762 and 1524 coefficients.
/// Returns 0 when 3·s is invertible and −1 otherwise.
pub(crate) fn rq_reciprocal3(r: &mut [i16; P], s: &[i8; P]) -> i32 {
    const LOOPS: usize = 2 * P + 1;
    let mut f = [0i16; P + 1];
    f[0] = -1;
    f[1] = -1;
    f[P] = 1;
    let mut g = [0i16; P + 1];
    for (gi, &si) in g.iter_mut().zip(s.iter()) {
        *gi = i16::from(3i8.wrapping_mul(si));
    }
    let mut d: i32 = P as i32;
    let mut e: i32 = P as i32;
    let mut u = [0i16; LOOPS + 1];
    let mut v = [0i16; LOOPS + 1];
    v[0] = 1;
    for _ in 0..LOOPS {
        let c = productq(g[P], recipq(f[P]));
        minus_product_shift_q(&mut g[..], &f[..], c);
        minus_product_shift_q(&mut v[..], &u[..], c);
        e = e.wrapping_sub(1);
        let m = smaller_mask(e, d) & mask_set_q(g[P]);
        swap_int(&mut e, &mut d, m);
        swap_q(&mut f[..], &mut g[..], m);
        swap_q(&mut u[..], &mut v[..], m);
    }
    let finv = recipq(f[P]);
    for (ri, &ui) in r.iter_mut().zip(u[P..].iter()) {
        *ri = productq(ui, finv);
    }
    smaller_mask(0, d)
}

fn minus_product_shift_q(z: &mut [i16], y: &[i16], c: i16) {
    for (zi, &yi) in z.iter_mut().zip(y.iter()) {
        *zi = freezeq(i32::from(*zi).wrapping_sub(i32::from(yi).wrapping_mul(i32::from(c))));
    }
    z.copy_within(..z.len().wrapping_sub(1), 1);
    z[0] = 0;
}

fn swap_q(x: &mut [i16], y: &mut [i16], mask: i32) {
    let c = mask as i16;
    for (xi, yi) in x.iter_mut().zip(y.iter_mut()) {
        let t = c & (*xi ^ *yi);
        *xi ^= t;
        *yi ^= t;
    }
}

fn mask_set_q(x: i16) -> i32 {
    i32::from(x as u16).wrapping_neg() >> 30
}

/// `r3.Reciprocal` (`r3/r3.go:31-70`): 1/s in R3 by the same loop as
/// [`rq_reciprocal3`]. Returns 0 when `s` is invertible and −1 otherwise,
/// and then leaves in `r` whatever Go's loop leaves.
pub(crate) fn r3_reciprocal(r: &mut [i8; P], s: &[i8; P]) -> i32 {
    const LOOPS: usize = 2 * P + 1;
    let mut f = [0i8; P + 1];
    f[0] = -1;
    f[1] = -1;
    f[P] = 1;
    let mut g = [0i8; P + 1];
    g[..P].copy_from_slice(s);
    let mut d: i32 = P as i32;
    let mut e: i32 = P as i32;
    let mut u = [0i8; LOOPS + 1];
    let mut v = [0i8; LOOPS + 1];
    v[0] = 1;
    for _ in 0..LOOPS {
        let c = g[P].wrapping_mul(f[P]);
        minus_product_shift_3(&mut g[..], &f[..], c);
        minus_product_shift_3(&mut v[..], &u[..], c);
        e = e.wrapping_sub(1);
        let m = smaller_mask(e, d) & i32::from(g[P].wrapping_neg().wrapping_mul(g[P]));
        swap_int(&mut e, &mut d, m);
        swap_3(&mut f[..], &mut g[..], m);
        swap_3(&mut u[..], &mut v[..], m);
    }
    let lc = f[P];
    for (ri, &ui) in r.iter_mut().zip(u[P..].iter()) {
        *ri = ui.wrapping_mul(lc);
    }
    smaller_mask(0, d)
}

fn minus_product_shift_3(z: &mut [i8], y: &[i8], c: i8) {
    for (zi, &yi) in z.iter_mut().zip(y.iter()) {
        *zi = freeze3(i32::from(*zi).wrapping_sub(i32::from(yi).wrapping_mul(i32::from(c))));
    }
    z.copy_within(..z.len().wrapping_sub(1), 1);
    z[0] = 0;
}

fn swap_3(x: &mut [i8], y: &mut [i8], mask: i32) {
    let c = mask as i8;
    for (xi, yi) in x.iter_mut().zip(y.iter_mut()) {
        let t = c & (*xi ^ *yi);
        *xi ^= t;
        *yi ^= t;
    }
}
