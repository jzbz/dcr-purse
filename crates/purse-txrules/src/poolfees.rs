// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The VSP ("stake pool") fee of a ticket (dcrwallet
//! `wallet/txrules/poolfees.go`, spec 10 §4.4).
//!
//! The fee for a percentage `p`, the vote subsidy `s` adjusted for the
//! subsidy reductions expected before the ticket votes, the ticket price
//! `v` and a fee term `z` is
//!
//! ```text
//!     f = p·s·(v + z) / (s + v)
//! ```
//!
//! computed by dcrwallet as `((p·100 · s · (v + z)) << 64) / ((s + v) ·
//! 10000) >> 64` over big integers, the percentage being
//! `int64(floor(poolFee * 100))` in float64. The parameter Go names
//! `relayFee` is `z`: one caller passes a ticket's absolute fee, the other
//! the relay fee rate (QK-0269).

use dcroxide_chaincfg::Params;
use dcroxide_standalone::{SubsidyCache, SubsidySplitVariant};
use num_bigint::{BigInt, BigUint, Sign};
use purse_dcrutil::Amount;
use purse_errors::{Error, Kind, e};

use crate::gofloat::amd64_f64_to_i64;

/// Whether `fee_rate` is a percentage from 0.01 to 100.00, that is whether
/// `floor(fee_rate * 100)` in float64 lies in `[1, 10000]` (Go
/// `ValidPoolFeeRate`; it has no caller in dcrwallet).
pub fn valid_pool_fee_rate(fee_rate: f64) -> bool {
    let test = (fee_rate * 100.0).floor();
    (1.0..=10000.0).contains(&test)
}

/// The VSP fee for a ticket, computing the vote subsidy from `params`
/// (Go `StakePoolTicketFee`).
///
/// A convenience over [`StakePoolFees`], which keeps the subsidy cache
/// between calls; see [`StakePoolFees::ticket_fee`] for the arguments and
/// the error.
pub fn stake_pool_ticket_fee(
    stake_diff: Amount,
    relay_fee: Amount,
    height: i32,
    pool_fee: f64,
    params: &Params,
    dcp0010_active: bool,
    dcp0012_active: bool,
) -> Result<Amount, Error> {
    StakePoolFees::new(params).ticket_fee(
        stake_diff,
        relay_fee,
        height,
        pool_fee,
        dcp0010_active,
        dcp0012_active,
    )
}

/// VSP fees on one network, with the subsidy cache kept between calls.
///
/// Go keeps one subsidy cache per process, built from the parameters of
/// the first call and used for every later call whatever parameters it is
/// passed. A value of this type is bound to the parameters it is created
/// with; a process that uses two networks gets each network's subsidy
/// (QK-0269).
pub struct StakePoolFees<'a> {
    cache: SubsidyCache<&'a Params>,
    /// `int(math.Ceil(float64(TicketPoolSize) /
    /// float64(SubsidyReductionInterval)))`, the subsidy reductions the fee
    /// anticipates.
    adjustments: i64,
}

impl<'a> StakePoolFees<'a> {
    /// The fee calculator for `params`.
    pub fn new(params: &'a Params) -> StakePoolFees<'a> {
        // Go converts both to float64 and takes the ceiling of the
        // quotient; an interval of 0 or below gives no adjustment (the
        // conversion of a NaN or an infinity is amd64's, see crate docs).
        let adjustments = amd64_f64_to_i64(
            (f64::from(params.ticket_pool_size) / params.subsidy_reduction_interval as f64).ceil(),
        );
        StakePoolFees {
            cache: SubsidyCache::new(params),
            adjustments,
        }
    }

    /// The VSP fee for a ticket of price `stake_diff`, with fee term
    /// `relay_fee`, voting after `height`, for the percentage `pool_fee`
    /// (`1.00` for 1%), under the subsidy split the two agenda flags select
    /// (DCP0012 wins over DCP0010).
    ///
    /// Every input gives Go's answer, overflow and negative values
    /// included, with one exception: where the denominator `(s + v) · 10000`
    /// is 0 Go's big-integer division panics, and this returns an error of
    /// kind [`Kind::Invalid`] instead (ADR-0012 Rule 2(c): the ticket price
    /// comes from the network backend). A NaN or out-of-range percentage is
    /// converted as amd64 does (ADR-0012 Rule 9(a)).
    pub fn ticket_fee(
        &mut self,
        stake_diff: Amount,
        relay_fee: Amount,
        height: i32,
        pool_fee: f64,
        dcp0010_active: bool,
        dcp0012_active: bool,
    ) -> Result<Amount, Error> {
        let pool_fee_int = amd64_f64_to_i64((pool_fee * 100.0).floor());
        let variant = if dcp0012_active {
            SubsidySplitVariant::Dcp0012
        } else if dcp0010_active {
            SubsidySplitVariant::Dcp0010
        } else {
            SubsidySplitVariant::Original
        };
        let mut subsidy = self
            .cache
            .calc_stake_vote_subsidy_v3(i64::from(height), variant);
        for _ in 0..self.adjustments.max(0) {
            subsidy = subsidy.wrapping_mul(100).wrapping_div(101);
        }
        fee_quotient(pool_fee_int, subsidy, stake_diff.0, relay_fee.0)
            .map(Amount)
            .ok_or_else(|| {
                e!(
                    Kind::Invalid,
                    "stake pool ticket fee: ticket price plus vote subsidy is zero"
                )
            })
    }
}

/// `int64(((p·s·(v+z)) << 64) / ((s+v)·10000) >> 64)` with Go's big.Int
/// semantics (Euclidean `Div`, flooring `Rsh`, and `Int64`, which keeps
/// the low 64 bits of the magnitude and applies the sign), or `None` where
/// the denominator is 0.
///
/// When the denominator is positive and the numerator fits in an `i128`,
/// the quotient is `floor(p·s·(v+z) / d)`: for a positive divisor the
/// Euclidean quotient is the floor, and flooring by 2^64 after flooring by
/// `d` is flooring by `d·2^64`. Otherwise the big-integer path computes it
/// step by step as Go does.
pub(crate) fn fee_quotient(p: i64, s: i64, v: i64, z: i64) -> Option<i64> {
    let v_plus_z = i128::from(v).wrapping_add(i128::from(z));
    let den = i128::from(s)
        .wrapping_add(i128::from(v))
        .wrapping_mul(10_000);
    if den == 0 {
        return None;
    }
    // |p·s| < 2^126, so only the last product can overflow; |s+v| < 2^64
    // and the den product above stays below 2^78.
    let ps = i128::from(p).wrapping_mul(i128::from(s));
    if den > 0
        && let Some(num) = ps.checked_mul(v_plus_z)
    {
        return Some(go_int64_of_i128(num.div_euclid(den)));
    }
    Some(fee_quotient_big(p, s, v, z))
}

/// Go's `big.Int.Int64` of an `i128`.
fn go_int64_of_i128(x: i128) -> i64 {
    let low = x.unsigned_abs() as u64 as i64;
    if x < 0 { low.wrapping_neg() } else { low }
}

/// The big-integer path of [`fee_quotient`], step for step as Go computes
/// it. The denominator is nonzero.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "arbitrary-precision integers: the operations cannot overflow"
)]
fn fee_quotient_big(p: i64, s: i64, v: i64, z: i64) -> i64 {
    let (s, v, z) = (BigInt::from(s), BigInt::from(v), BigInt::from(z));
    let num: BigInt = (BigInt::from(p) * &s * (&v + &z)) << 64u32;
    let den = (&s + &v) * BigInt::from(10_000);
    div_rsh_int64(&num, &den)
}

/// Go's `num.Div(num, den).Rsh(num, 64).Int64()`: the Euclidean quotient
/// (its remainder never negative), floored by 2^64, then the low 64 bits of
/// the magnitude with the sign applied. `den` is nonzero.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "arbitrary-precision integers: the operations cannot overflow, and the one \
              division has a nonzero divisor"
)]
fn div_rsh_int64(num: &BigInt, den: &BigInt) -> i64 {
    // Truncating division, then the Euclidean correction: a negative
    // remainder moves the quotient one step away from zero's side of the
    // divisor, down for a positive divisor and up for a negative one.
    let mut q = num / den;
    let r = num - &q * den;
    if r.sign() == Sign::Minus {
        if den.sign() == Sign::Plus {
            q -= 1;
        } else {
            q += 1;
        }
    }
    // Rsh rounds toward negative infinity: floor(q / 2^64).
    let q = floor_shr_64(&q);
    // Int64: the low 64 bits of the magnitude, with the sign applied.
    let low = q.magnitude().iter_u64_digits().next().unwrap_or(0) as i64;
    if q.sign() == Sign::Minus {
        low.wrapping_neg()
    } else {
        low
    }
}

/// `floor(q / 2^64)` for a big integer of either sign.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "arbitrary-precision integers: the operations cannot overflow"
)]
fn floor_shr_64(q: &BigInt) -> BigInt {
    let mag: &BigUint = q.magnitude();
    match q.sign() {
        Sign::Minus => {
            // -ceil(|q| / 2^64) = -((|q| + 2^64 - 1) >> 64).
            let rounded: BigUint = (mag + BigUint::from(u64::MAX)) >> 64u32;
            BigInt::from_biguint(Sign::Minus, rounded)
        }
        _ => BigInt::from_biguint(Sign::Plus, mag >> 64u32),
    }
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and small values"
)]
mod tests {
    use super::*;
    use purse_testutil::SplitMix64;

    fn pick(rng: &mut SplitMix64) -> i64 {
        match rng.below(6) {
            0 => [0, 1, -1, i64::MAX, i64::MIN, i64::MIN + 1][rng.below(6) as usize],
            1 => rng.next_u64() as i64,
            2 => (rng.next_u64() >> 20) as i64,
            3 => -((rng.next_u64() >> 20) as i64),
            4 => (rng.next_u64() >> 40) as i64,
            _ => rng.below(20_000) as i64 - 10_000,
        }
    }

    /// The fast path and the step-by-step path agree wherever the fast
    /// path applies, and the big path is used otherwise.
    #[test]
    fn fast_path_equals_big_path() {
        let mut rng = SplitMix64::from_env("purse-txrules fee_quotient");
        let mut fast = 0usize;
        for _ in 0..200_000 {
            let (p, s, v, z) = (
                pick(&mut rng),
                pick(&mut rng),
                pick(&mut rng),
                pick(&mut rng),
            );
            let den = (i128::from(s) + i128::from(v)) * 10_000;
            let got = fee_quotient(p, s, v, z);
            if den == 0 {
                assert_eq!(got, None);
                continue;
            }
            let num = (i128::from(p) * i128::from(s)).checked_mul(i128::from(v) + i128::from(z));
            if den > 0 && num.is_some() {
                fast += 1;
            }
            assert_eq!(got, Some(fee_quotient_big(p, s, v, z)), "{p} {s} {v} {z}");
        }
        assert!(fast > 10_000, "fast path taken {fast} times");
    }

    /// Go's `Div`, `Rsh` and `Int64` computed independently of
    /// `div_rsh_int64`: the Euclidean quotient as `sign(den) * floor(num /
    /// |den|)` over magnitudes, num-bigint's flooring shift, and the low 64
    /// bits of the magnitude.
    fn reference(num: &BigInt, den: &BigInt) -> i64 {
        let (n, d) = (num.magnitude(), den.magnitude());
        let floor: BigInt = if num.sign() == Sign::Minus {
            -BigInt::from((n + d - 1u8) / d)
        } else {
            BigInt::from(n / d)
        };
        let q = if den.sign() == Sign::Minus {
            -floor
        } else {
            floor
        };
        let q: BigInt = q >> 64u32;
        let low = q.magnitude().iter_u64_digits().next().unwrap_or(0) as i64;
        if q.sign() == Sign::Minus {
            low.wrapping_neg()
        } else {
            low
        }
    }

    /// Quotients that land next to a multiple of 2^64, where the Euclidean
    /// correction decides the floored result, for every sign of the
    /// numerator and the divisor: `num = k·den ± r` with `0 < r·2^64 <
    /// |den|`. The fee formula reaches these only with a divisor beyond
    /// 2^64 (a ticket price near ±2^62), which the oracle corpus engineers
    /// (`ticket_cases`); here any numerator is taken.
    #[test]
    fn euclidean_correction_at_the_shift_boundary() {
        let mut rng = SplitMix64::from_env("purse-txrules div_rsh_int64");
        let two64 = BigInt::from(1u8) << 64u32;
        let mut checked = 0;
        for _ in 0..2_000 {
            // |den| in (2^64, 2^78), as (s + v)·10000 can be.
            let mag: BigInt = BigInt::from(rng.next_u64() >> rng.below(14)) * 10_000 + &two64 + 1u8;
            let k = BigInt::from(rng.next_u64() >> rng.below(64)) << 64u32;
            let max_r = (&mag - 1u8) >> 64u32;
            let r = BigInt::from(1u8) + BigInt::from(rng.next_u64()) % &max_r;
            for den in [mag.clone(), -mag.clone()] {
                for num in [
                    &k * &den + &r,
                    &k * &den - &r,
                    -(&k * &den) + &r,
                    -(&k * &den) - &r,
                ] {
                    assert_eq!(
                        div_rsh_int64(&num, &den),
                        reference(&num, &den),
                        "{num} / {den}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 16_000);
    }

    /// The whole big path against the reference on random operands, both
    /// signs of the divisor included (the fast path covers only a positive
    /// one).
    #[test]
    fn big_path_equals_reference() {
        let mut rng = SplitMix64::from_env("purse-txrules fee_quotient_big");
        for _ in 0..50_000 {
            let (p, s, v, z) = (
                pick(&mut rng),
                pick(&mut rng),
                pick(&mut rng),
                pick(&mut rng),
            );
            let den: BigInt = (BigInt::from(s) + v) * 10_000;
            if den.sign() == Sign::NoSign {
                continue;
            }
            let num: BigInt = (BigInt::from(p) * s * (BigInt::from(v) + z)) << 64u32;
            assert_eq!(
                fee_quotient_big(p, s, v, z),
                reference(&num, &den),
                "{p} {s} {v} {z}"
            );
        }
    }

    /// The cases that showed the negative divisor's correction untested:
    /// mainnet, height 306144, the original subsidy split, a numerator
    /// `p·s·(v+z) = -(10000·|s+v| - 1)`. Go answers 1 for each (the oracle's
    /// `poolfee.ticket`); without the correction the quotient floors to 0.
    #[test]
    fn negative_divisor_boundary_cases() {
        let params = dcroxide_chaincfg::mainnet_params();
        for (diff, relay, fee_bits) in [
            (
                -2305843009331151880i64,
                2305638378031244171i64,
                0x3f8e_b851_eb85_1eb8u64,
            ),
            (
                -2305843009443834691,
                2305774799010528788,
                0x3fa1_eb85_1eb8_51ec,
            ),
            (
                -2305843009443834691,
                2305813776400989304,
                0x3fb3_3333_3333_3333,
            ),
        ] {
            let fee = stake_pool_ticket_fee(
                Amount(diff),
                Amount(relay),
                306_144,
                f64::from_bits(fee_bits),
                &params,
                false,
                false,
            )
            .expect("a nonzero denominator");
            assert_eq!(fee, Amount(1), "{diff} {relay} {fee_bits:#x}");
        }
    }

    #[test]
    fn floor_shift_of_negatives() {
        let two64 = BigInt::from(1u8) << 64u32;
        assert_eq!(floor_shr_64(&BigInt::from(-1)), BigInt::from(-1));
        assert_eq!(floor_shr_64(&-two64.clone()), BigInt::from(-1));
        assert_eq!(floor_shr_64(&(-two64.clone() - 1)), BigInt::from(-2));
        assert_eq!(floor_shr_64(&(two64.clone() - 1)), BigInt::from(0));
        assert_eq!(floor_shr_64(&two64), BigInt::from(1));
    }

    #[test]
    fn zero_denominator_is_an_error() {
        let params = dcroxide_chaincfg::mainnet_params();
        // Below stake validation the vote subsidy is 0.
        let err =
            stake_pool_ticket_fee(Amount(0), Amount(1), 0, 1.0, &params, false, false).unwrap_err();
        assert!(err.is_kind(Kind::Invalid));
        assert_eq!(
            err.render(":: "),
            "invalid operation: stake pool ticket fee: ticket price plus vote subsidy is zero"
        );
    }
}
