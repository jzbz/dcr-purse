// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Go's float-to-integer conversion as amd64 builds of dcrwallet perform it.

/// Go's `int64(f)` (and `int(f)`) on amd64: `CVTTSD2SQ`, which truncates
/// toward zero and gives `0x8000000000000000` for a NaN and for any value
/// outside `[-2^63, 2^63)`.
///
/// arm64 saturates instead and gives 0 for a NaN (Rust's `as` is that
/// conversion). ADR-0012 Rule 9(a) fixes amd64's answer on every target.
pub(crate) fn amd64_f64_to_i64(f: f64) -> i64 {
    // 2^63, exactly representable.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if (-LIMIT..LIMIT).contains(&f) {
        // In range, so `as` truncates exactly as CVTTSD2SQ does.
        f as i64
    } else {
        i64::MIN
    }
}

#[cfg(test)]
mod tests {
    use super::amd64_f64_to_i64;

    #[test]
    fn out_of_range_and_nan_give_the_indefinite_integer() {
        assert_eq!(amd64_f64_to_i64(f64::NAN), i64::MIN);
        assert_eq!(amd64_f64_to_i64(f64::INFINITY), i64::MIN);
        assert_eq!(amd64_f64_to_i64(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(amd64_f64_to_i64(9_223_372_036_854_775_808.0), i64::MIN);
        assert_eq!(amd64_f64_to_i64(-9_223_372_036_854_775_808.0), i64::MIN);
        assert_eq!(amd64_f64_to_i64(-1.9), -1);
        assert_eq!(amd64_f64_to_i64(1.9), 1);
        assert_eq!(
            amd64_f64_to_i64(9_223_372_036_854_774_784.0),
            9_223_372_036_854_774_784
        );
    }
}
