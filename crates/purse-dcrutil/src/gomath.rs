// Copyright (c) 2026 The dcr-purse developers
// Copyright 2009 The Go Authors. All rights reserved.
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file. The port of Go's math.Pow10 is also under Go's BSD-style
// license; see the NOTICE file.

//! The pieces of Go's runtime arithmetic that `Amount` depends on: Go's
//! `math.Pow10`, whose table products are not always the correctly rounded
//! power of ten, and the `float64` to `int64` conversion as Go performs it
//! on amd64.

/// `pow10tab` (Go `math/pow10.go`): 10^i for i < 32.
const POW10TAB: [f64; 32] = [
    1e00, 1e01, 1e02, 1e03, 1e04, 1e05, 1e06, 1e07, 1e08, 1e09, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
    1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27, 1e28, 1e29, 1e30, 1e31,
];

/// `pow10postab32`: 10^(32 i).
const POW10POSTAB32: [f64; 10] = [
    1e00, 1e32, 1e64, 1e96, 1e128, 1e160, 1e192, 1e224, 1e256, 1e288,
];

/// `pow10negtab32`: 10^(-32 i).
const POW10NEGTAB32: [f64; 11] = [
    1e-00, 1e-32, 1e-64, 1e-96, 1e-128, 1e-160, 1e-192, 1e-224, 1e-256, 1e-288, 1e-320,
];

/// Go's `math.Pow10(n)` with Go's `int` as 64 bits: a product (or, for
/// negative `n`, a quotient) of two table entries for -323 <= n <= 308,
/// `+Inf` above that range and 0 below it.
pub(crate) fn pow10(n: i64) -> f64 {
    if (0..=308).contains(&n) {
        // n is in 0..=308, so the quotient is below 10 and the remainder
        // below 32: both indices are in range.
        let n = n.unsigned_abs();
        table(&POW10POSTAB32, n / 32) * table(&POW10TAB, n % 32)
    } else if (-323..0).contains(&n) {
        // -n is in 1..=323: indices below 11 and 32.
        let m = n.unsigned_abs();
        table(&POW10NEGTAB32, m / 32) / table(&POW10TAB, m % 32)
    } else if n > 0 {
        f64::INFINITY
    } else {
        0.0
    }
}

/// A table entry at an index the caller has bounded; 1 for an index out of
/// range, which the callers' ranges rule out.
fn table(t: &[f64], i: u64) -> f64 {
    usize::try_from(i)
        .ok()
        .and_then(|i| t.get(i))
        .copied()
        .unwrap_or(1.0)
}

/// 2^63 as a float64, the first value above `int64`'s range.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;

/// Go's `int64(f)` as amd64 executes it: `CVTTSD2SQ`, which truncates
/// toward zero and returns `0x8000000000000000` (`i64::MIN`) for NaN and
/// for every value outside `[-2^63, 2^63)`.
///
/// Go leaves an out-of-range conversion implementation-defined; arm64's
/// `FCVTZS` saturates instead (and gives 0 for NaN), which is what Rust's
/// `as` does on every target, and riscv64, 386 and arm give still other
/// answers (see the `amount` module). This function is amd64's rule on
/// every target.
pub(crate) fn go_amd64_f64_to_i64(f: f64) -> i64 {
    if (-TWO_POW_63..TWO_POW_63).contains(&f) {
        // In range, so `as` is plain truncation toward zero.
        f as i64
    } else {
        i64::MIN
    }
}

/// dcrutil's `round` (`amount.go:61-66`) as amd64 executes it: `f - 0.5`
/// for negative `f`, else `f + 0.5`, each one IEEE-754 operation rounded
/// to nearest, then [`go_amd64_f64_to_i64`].
///
/// The caller's multiplication and this addition are separate roundings:
/// Rust never fuses floating-point operations, and Go on amd64 at GOAMD64
/// v1 (the default) or v2 does not either. At GOAMD64 v3 and v4 Go does
/// fuse the multiplication with the `+ 0.5` (`VFMADD231SD`), so those
/// builds are not what this function computes.
pub(crate) fn go_amd64_round(f: f64) -> i64 {
    let shifted = if f < 0.0 { f - 0.5 } else { f + 0.5 };
    go_amd64_f64_to_i64(shifted)
}

/// The NaN amd64 produces for 0/0 (`DIVSD`'s default NaN, the sign bit set:
/// `0xfff8000000000000`). arm64 produces `0x7ff8000000000000` (see the
/// `amount` module for the other architectures).
pub(crate) const AMD64_DEFAULT_NAN: f64 = f64::from_bits(0xfff8_0000_0000_0000);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_follows_amd64_at_the_edges() {
        assert_eq!(go_amd64_f64_to_i64(f64::NAN), i64::MIN);
        assert_eq!(go_amd64_f64_to_i64(-TWO_POW_63), i64::MIN);
        assert_eq!(go_amd64_f64_to_i64(TWO_POW_63), i64::MIN);
        assert_eq!(go_amd64_f64_to_i64(f64::INFINITY), i64::MIN);
        assert_eq!(go_amd64_f64_to_i64(f64::NEG_INFINITY), i64::MIN);
        let below = f64::from_bits(TWO_POW_63.to_bits() - 1);
        assert_eq!(go_amd64_f64_to_i64(below), 0x7fff_ffff_ffff_fc00);
        assert_eq!(go_amd64_f64_to_i64(-1.9), -1);
        assert_eq!(go_amd64_f64_to_i64(-0.0), 0);
    }

    #[test]
    fn pow10_is_the_table_product() {
        for n in -330..=320_i64 {
            let p = pow10(n);
            if n > 308 {
                assert_eq!(p, f64::INFINITY);
            } else if n < -323 {
                assert_eq!(p.to_bits(), 0);
            } else {
                assert!(p.is_finite() && p > 0.0, "{n}");
            }
        }
        assert_eq!(pow10(8), 1e8);
        assert_eq!(pow10(i64::MIN), 0.0);
        assert_eq!(pow10(i64::MAX), f64::INFINITY);
    }
}
