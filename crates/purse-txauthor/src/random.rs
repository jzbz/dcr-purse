// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2026 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Change-position randomization (Go `RandomizeOutputPosition` and
//! `RandomizeChangePosition`, `wallet/txauthor/author.go:168-181`) over
//! dcroxide's port of dcrd's `crypto/rand`, whose `Int32N` draws what Go's
//! draws.

use dcroxide_crypto::rand::Prng;
use dcroxide_wire::TxOut;
use purse_errors::{Error, Kind, e};

use crate::author::AuthoredTx;

/// Swaps `outputs[index]` with an output at a uniformly random position,
/// possibly itself, and returns that position (Go
/// `RandomizeOutputPosition`), drawing from `dcroxide-crypto`'s
/// process-wide generator as Go draws from dcrd's, with the generator's
/// lock held for the whole bounded draw as Go holds it.
///
/// # Errors
///
/// As [`randomize_output_position_with`].
pub fn randomize_output_position(outputs: &mut [TxOut], index: i64) -> Result<i64, Error> {
    randomize(outputs, index, dcroxide_crypto::rand::int32n)
}

/// [`randomize_output_position`] drawing from `rng`.
///
/// The position is `rand.Int32N(int32(len(outputs)))`: the count is
/// narrowed to 32 bits as Go's conversion does, and the draw is dcroxide's
/// port of dcrd's `Int32N` (one 64-bit draw per attempt), so a [`Prng`]
/// keyed with [`Prng::from_seed`] gives the positions the Go oracle's
/// generator keyed alike gives (docs/specs/23 §5.6.2). It is drawn before
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
pub fn randomize_output_position_with(
    outputs: &mut [TxOut],
    index: i64,
    rng: &mut Prng,
) -> Result<i64, Error> {
    randomize(outputs, index, |n| rng.int32n(n))
}

/// The randomization over `int32n`, which is called once, with a positive
/// bound, and returns a value below it.
fn randomize(
    outputs: &mut [TxOut],
    index: i64,
    int32n: impl FnOnce(i32) -> i32,
) -> Result<i64, Error> {
    // Go `int32(len(outputs))`: the low 32 bits.
    let n = outputs.len() as i32;
    if n <= 0 {
        // Checked here, before any draw: dcroxide's Int32N panics, as Go's
        // does, on a bound that is not positive.
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
    // The draw is in [0, n), so nonnegative: the cast is exact.
    let r = int32n(n) as usize;
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
        self.change_index = randomize_output_position(&mut self.tx.tx_out, self.change_index)?;
        Ok(())
    }

    /// [`randomize_change_position`](Self::randomize_change_position)
    /// drawing from `rng`.
    ///
    /// # Errors
    ///
    /// As [`randomize_change_position`](Self::randomize_change_position).
    pub fn randomize_change_position_with(&mut self, rng: &mut Prng) -> Result<(), Error> {
        self.change_index =
            randomize_output_position_with(&mut self.tx.tx_out, self.change_index, rng)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    reason = "test code: reference arithmetic on bounded values"
)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    fn outs(n: usize) -> Vec<TxOut> {
        (0..n)
            .map(|i| TxOut {
                value: i as i64,
                ..TxOut::default()
            })
            .collect()
    }

    /// The 64-bit draws a generator keyed with `SEED` yields after `skip`
    /// of them.
    fn stream_after(skip: usize) -> u64 {
        let mut p = Prng::from_seed(SEED);
        for _ in 0..skip {
            p.uint64();
        }
        p.uint64()
    }

    #[test]
    fn no_outputs_draws_nothing() {
        let mut p = Prng::from_seed(SEED);
        let err = randomize_output_position_with(&mut [], 0, &mut p).unwrap_err();
        assert!(err.is_kind(Kind::Invalid));
        assert_eq!(p.uint64(), stream_after(0), "a draw was taken");
    }

    #[test]
    fn bad_index_draws_then_fails_unchanged() {
        let mut o = outs(5);
        let mut p = Prng::from_seed(SEED);
        let err = randomize_output_position_with(&mut o, -1, &mut p).unwrap_err();
        assert!(err.is_kind(Kind::Invalid), "{err}");
        assert_eq!(p.uint64(), stream_after(1), "not exactly one draw");
        assert_eq!(o, outs(5));
    }

    #[test]
    fn swaps_with_the_drawn_position() {
        // Go's Int32N(5) on the stream's first draw x, which for this key
        // is not rejected (the low half of x * 5 is not below 2^64 mod 5):
        // the high half of the product.
        let n = 5u64;
        let x = stream_after(0);
        let product = u128::from(x) * u128::from(n);
        // Go's threshold, -n % n: 2^64 mod n.
        let thresh = n.wrapping_neg() % n;
        assert!(product as u64 >= thresh, "the first draw is rejected");
        let want = (product >> 64) as usize;

        let mut o = outs(5);
        let mut p = Prng::from_seed(SEED);
        let index = (want + 1) % 5;
        assert_eq!(
            randomize_output_position_with(&mut o, index as i64, &mut p).unwrap(),
            want as i64
        );
        let mut values: Vec<i64> = (0..5).collect();
        values.swap(want, index);
        assert_eq!(o.iter().map(|t| t.value).collect::<Vec<_>>(), values);
        assert_eq!(p.uint64(), stream_after(1), "not exactly one draw");
    }

    #[test]
    fn the_global_generator_draws_in_range() {
        for n in [1usize, 2, 3, 7, 255] {
            let mut o = outs(n);
            let r = randomize_output_position(&mut o, 0).unwrap();
            assert!((0..n as i64).contains(&r), "{r} of {n}");
        }
        let err = randomize_output_position(&mut [], 0).unwrap_err();
        assert!(err.is_kind(Kind::Invalid));
    }
}
