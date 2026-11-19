// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2026 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Change-position randomization (Go `RandomizeOutputPosition` and
//! `RandomizeChangePosition`, `wallet/txauthor/author.go:168-181`) over an
//! injectable random source that draws exactly what Go's `rand.Int32N`
//! draws.

use dcroxide_crypto::rand::Prng;
use dcroxide_wire::TxOut;
use purse_errors::{Error, Kind, e};

use crate::author::AuthoredTx;

/// A source of uniform 64-bit draws, the one primitive dcrd's
/// `crypto/rand` derives its bounded integers from.
///
/// [`GlobalRandom`] is the process-wide generator Go's package functions
/// use; [`Prng`] can be injected, and [`Prng::from_seed`] gives tests the
/// ChaCha20 stream the Go oracle's seeded generator produces
/// (docs/specs/23 §5.6.2).
pub trait RandomSource {
    /// A uniform random `u64`, little-endian from eight stream bytes (dcrd
    /// `PRNG.Uint64`).
    fn uint64(&mut self) -> u64;
}

impl RandomSource for Prng {
    fn uint64(&mut self) -> u64 {
        Prng::uint64(self)
    }
}

/// The process-wide generator of `dcroxide-crypto` (dcrd `crypto/rand`'s
/// package global, which Go's `rand.Int32N` locks and draws from).
///
/// Go holds the global lock across a whole `Int32N`, including any
/// rejection redraw; this takes it per 64-bit draw. Another thread's draw
/// can then fall between two draws of one call, which changes which
/// uniform value each call gets but no distribution.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalRandom;

impl RandomSource for GlobalRandom {
    fn uint64(&mut self) -> u64 {
        dcroxide_crypto::rand::uint64()
    }
}

/// The high and low halves of the 64-bit product of two `u32` (Go
/// `bits.Mul32`).
#[inline]
fn mul32(x: u32, y: u32) -> (u32, u32) {
    // The product of two values below 2^32 is below 2^64: no wrap.
    let p = u64::from(x).wrapping_mul(u64::from(y));
    ((p >> 32) as u32, p as u32)
}

/// A uniform `u32` in `[0, n)` without modulo bias (dcrd `PRNG.Uint32N`,
/// `crypto/rand/uniform.go:61-97`), drawing one `u64` per attempt.
///
/// This is Go's code, which computes the high word of the 96-bit product
/// of a draw and `n` from 32-bit halves and redraws while the low 64 bits
/// fall below `2^64 mod n`, so the draw count is Go's. Go runs this form on
/// 64-bit machines too. `n` is nonzero: Go's only caller here, `Int32N`,
/// refuses `n <= 0` first.
fn uint32n<R: RandomSource + ?Sized>(rng: &mut R, n: u32) -> u32 {
    if n & n.wrapping_sub(1) == 0 {
        // A power of two: the low bits of one draw.
        return (rng.uint64() as u32) & n.wrapping_sub(1);
    }
    let product = |x: u64| {
        let (lo1a, lo0) = mul32(x as u32, n);
        let (hi, lo1b) = mul32((x >> 32) as u32, n);
        let (lo1, carry) = lo1a.overflowing_add(lo1b);
        (hi.wrapping_add(u32::from(carry)), lo1, lo0)
    };
    let (mut hi, mut lo1, mut lo0) = product(rng.uint64());
    if lo1 == 0 && lo0 < n {
        // Go's `uint32(-n64 % n64)`: 2^64 mod n, below n.
        let n64 = u64::from(n);
        let thresh = n64.wrapping_neg().checked_rem(n64).unwrap_or(0) as u32;
        while lo1 == 0 && lo0 < thresh {
            (hi, lo1, lo0) = product(rng.uint64());
        }
    }
    hi
}

/// Swaps `outputs[index]` with an output at a uniformly random position,
/// possibly itself, and returns that position (Go
/// `RandomizeOutputPosition`), drawing from the process-wide generator as
/// Go does.
///
/// # Errors
///
/// As [`randomize_output_position_with`].
pub fn randomize_output_position(outputs: &mut [TxOut], index: i64) -> Result<i64, Error> {
    randomize_output_position_with(outputs, index, &mut GlobalRandom)
}

/// [`randomize_output_position`] drawing from `rng`.
///
/// The position is `rand.Int32N(int32(len(outputs)))`: the count is
/// narrowed to 32 bits as Go's conversion does, and the draws are Go's
/// (one `u64` per attempt, see [`RandomSource`]). It is drawn before
/// `index` is checked, as in Go, where the swap's bounds check comes after
/// the draw.
///
/// # Errors
///
/// Where Go panics (QK-0263, QK-0492), an error of kind `Invalid` with
/// dcr-purse's own text (ADR-0012 Rule 2(c)):
///
/// - when the narrowed count is not positive, that is `outputs` is empty
///   (Go's `rand: invalid argument to Int32N`), before any draw;
/// - when `index` is not a position in `outputs`, as `-1` is for an
///   authored transaction without change (Go's `index out of range`),
///   after the draw. `outputs` is then unchanged.
pub fn randomize_output_position_with<R>(
    outputs: &mut [TxOut],
    index: i64,
    rng: &mut R,
) -> Result<i64, Error>
where
    R: RandomSource + ?Sized,
{
    // Go `int32(len(outputs))`: the low 32 bits.
    let n = outputs.len() as i32;
    if n <= 0 {
        return Err(if outputs.is_empty() {
            e!(Kind::Invalid, "randomize output position: no outputs")
        } else {
            e!(
                Kind::Invalid,
                format!(
                    "randomize output position: {} outputs do not fit an int32",
                    outputs.len()
                )
            )
        });
    }
    // n is positive, and the draw is below it, so both casts are exact.
    let r = uint32n(rng, n as u32) as usize;
    let Some(i) = usize::try_from(index).ok().filter(|&i| i < outputs.len()) else {
        return Err(e!(
            Kind::Invalid,
            format!(
                "randomize output position: index {index} out of range for {} outputs",
                outputs.len()
            )
        ));
    };
    outputs.swap(r, i);
    Ok(r as i64)
}

impl AuthoredTx {
    /// Moves the change output to a uniformly random position among the
    /// outputs (Go `RandomizeChangePosition`), drawing from the
    /// process-wide generator. Do this before signing.
    ///
    /// # Errors
    ///
    /// As [`randomize_output_position_with`]; in particular when there is
    /// no change output (`change_index` -1), where Go panics. The index is
    /// then unchanged.
    pub fn randomize_change_position(&mut self) -> Result<(), Error> {
        self.randomize_change_position_with(&mut GlobalRandom)
    }

    /// [`randomize_change_position`](Self::randomize_change_position)
    /// drawing from `rng`.
    ///
    /// # Errors
    ///
    /// As [`randomize_change_position`](Self::randomize_change_position).
    pub fn randomize_change_position_with<R>(&mut self, rng: &mut R) -> Result<(), Error>
    where
        R: RandomSource + ?Sized,
    {
        self.change_index =
            randomize_output_position_with(&mut self.tx.tx_out, self.change_index, rng)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and reference arithmetic on bounded values"
)]
mod tests {
    use super::*;

    /// Answers a fixed list of draws, then panics.
    struct Script(Vec<u64>, usize);

    impl RandomSource for Script {
        fn uint64(&mut self) -> u64 {
            let x = self.0[self.1];
            self.1 += 1;
            x
        }
    }

    /// The 64-bit form of the same reduction (dcroxide's port of dcrd's
    /// `Uint64N`), which Go's comment says `Uint32N` reproduces.
    fn reference(draws: &[u64], n: u32) -> (u32, usize) {
        let mut used = 0;
        let mut next = || {
            let x = draws[used];
            used += 1;
            x
        };
        let n64 = u64::from(n);
        if n & (n - 1) == 0 {
            return ((next() & (n64 - 1)) as u32, 1);
        }
        let wide = |x: u64| u128::from(x) * u128::from(n64);
        let mut p = wide(next());
        if (p as u64) < n64 {
            let thresh = n64.wrapping_neg() % n64;
            while (p as u64) < thresh {
                p = wide(next());
            }
        }
        (((p >> 64) as u32), used)
    }

    /// The inverse of odd `n` modulo 2^64 (Newton's iteration).
    fn inverse(n: u32) -> u64 {
        let n = u64::from(n);
        let mut inv = n;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(n.wrapping_mul(inv)));
        }
        assert_eq!(n.wrapping_mul(inv), 1);
        inv
    }

    /// Draws whose product with n has a low half below `thresh` (2^64 mod
    /// n), which Go rejects: 0 for any n, and for odd n the draw whose
    /// product's low half is `thresh - 1`, through n's inverse mod 2^64.
    fn rejected(n: u32, thresh: u64) -> Vec<u64> {
        let mut v = vec![0];
        if n % 2 == 1 {
            v.push((thresh - 1).wrapping_mul(inverse(n)));
        }
        v
    }

    #[test]
    fn uint32n_matches_the_64_bit_reduction() {
        let mut state = 0x0123_4567_89ab_cdefu64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        let bounds = [
            1u32,
            2,
            3,
            5,
            7,
            8,
            1000,
            0x7fff_ffff,
            0x4000_0000,
            0x7fff_fffe,
            0x5555_5555,
        ];
        for &n in &bounds {
            for _ in 0..2_000 {
                let draws: Vec<u64> = (0..4).map(|_| next()).collect();
                let want = reference(&draws, n);
                let mut s = Script(draws, 0);
                assert_eq!((uint32n(&mut s, n), s.1), want, "n {n}");
            }
        }
        // The rejection loop, which a random draw reaches with probability
        // below 2^-33: a rejected draw then an accepted one.
        for &n in &[3u32, 1000, 0x7fff_ffff, 0x5555_5555] {
            let thresh = (u64::MAX % u64::from(n) + 1) % u64::from(n);
            if thresh == 0 {
                continue;
            }
            for bad in rejected(n, thresh) {
                assert!(
                    bad.wrapping_mul(u64::from(n)) < thresh,
                    "n {n}: not rejected"
                );
                let draws = vec![bad, bad, 12345];
                let want = reference(&draws, n);
                assert_eq!(want.1, 3);
                let mut s = Script(draws, 0);
                assert_eq!((uint32n(&mut s, n), s.1), want, "n {n}");
            }
            // The boundary: for odd n, the draw whose product's low half is
            // exactly `thresh` enters the loop's test (low 64 bits below n)
            // and is accepted (Go's `lo0 < thresh` is strict), after one
            // draw; the second draw would be taken by `<=`.
            if n % 2 == 1 {
                let edge = thresh.wrapping_mul(inverse(n));
                assert_eq!(edge.wrapping_mul(u64::from(n)), thresh, "n {n}");
                let draws = vec![edge, 0, 12345];
                let want = reference(&draws, n);
                assert_eq!(want, (((u128::from(edge) * u128::from(n)) >> 64) as u32, 1));
                let mut s = Script(draws, 0);
                assert_eq!((uint32n(&mut s, n), s.1), want, "n {n} at thresh");
            }
        }
    }

    fn outs(n: usize) -> Vec<TxOut> {
        (0..n)
            .map(|i| TxOut {
                value: i as i64,
                ..TxOut::default()
            })
            .collect()
    }

    #[test]
    fn no_outputs_draws_nothing() {
        let mut s = Script(Vec::new(), 0);
        let err = randomize_output_position_with(&mut [], 0, &mut s).unwrap_err();
        assert!(err.is_kind(Kind::Invalid));
        assert_eq!(s.1, 0);
    }

    #[test]
    fn bad_index_draws_then_fails_unchanged() {
        let mut o = outs(5);
        let mut s = Script(vec![u64::MAX], 0);
        let err = randomize_output_position_with(&mut o, -1, &mut s).unwrap_err();
        assert!(err.is_kind(Kind::Invalid), "{err}");
        assert_eq!(s.1, 1);
        assert_eq!(o, outs(5));
    }

    #[test]
    fn swaps_with_the_drawn_position() {
        let mut o = outs(5);
        // hi of u64::MAX * 5 is 4.
        let mut s = Script(vec![u64::MAX], 0);
        assert_eq!(
            randomize_output_position_with(&mut o, 1, &mut s).unwrap(),
            4
        );
        let values: Vec<i64> = o.iter().map(|t| t.value).collect();
        assert_eq!(values, [0, 4, 2, 3, 1]);
    }
}
