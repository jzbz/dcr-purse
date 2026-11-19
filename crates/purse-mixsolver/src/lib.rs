// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The StakeShuffle root solver: the distinct roots of the slot-reservation
//! polynomial over F = 2¹²⁷ − 1, computed in process ([ADR-0010]).
//!
//! In a mix, every peer publishes its slot-reservation (SR) messages as
//! padded power vectors; the pads cancel and the column sums are the power
//! sums of all `mtot` messages, which `mixing.Coefficients` turns into the
//! coefficients of a degree-`mtot` polynomial whose roots are exactly those
//! messages (spec 12 §5.9.3). The roots in ascending order are the slots.
//! dcrwallet obtains them from an external `csppsolver` process that calls
//! FLINT's `fmpz_mod_poly_factor`; dcr-purse calls [`roots`].
//!
//! # The function
//!
//! [`roots`] takes the coefficients as canonical residues in ascending
//! degree order — `coefficients[k]` multiplies `x^k`, the order of
//! `mixing.Coefficients` and of the solver RPC's `A` — and returns the
//! `degree` roots in ascending order, or an [`Error`]. It **accepts exactly
//! when the leading coefficient is nonzero and the polynomial has `degree`
//! distinct roots in F**, which is when the monic g = a / lead(a) divides
//! x^F − x = ∏ over r in F of (x − r). The answer, and the set of inputs
//! accepted, are fixed by that mathematics; how they are found is not
//! observable except through time.
//!
//! # Algorithm (ADR-0010, Decision 1)
//!
//! 1. Make g monic and compute h = x^((F−1)/2) mod g by square-and-multiply
//!    (125 modular squarings, each followed by a multiplication by x).
//! 2. Fail with [`Error::NotSplit`] unless x·h² ≡ x (mod g): x·h² is
//!    x^F mod g, and g divides x^F − x exactly when it is a product of
//!    distinct linear factors. A repeated root, or an irreducible factor of
//!    degree two or more, fails here, before any splitting.
//! 3. Split by Cantor-Zassenhaus: first gcd(g, h − 1), which separates the
//!    nonzero roots that are squares from the rest; then each factor f of
//!    degree two or more, recursively, by gcd(f, (x + δ)^((F−1)/2) − 1),
//!    which separates the roots r with r + δ a nonzero square from the
//!    rest. A factor of degree one, x + f₀, gives the root −f₀.
//! 4. Check every root by Horner evaluation of the *input* polynomial,
//!    check that they are distinct and that there are `degree` of them, and
//!    sort. A failure here would be a defect of this crate and is reported
//!    as [`Error::SelfCheckFailed`]: no wrong root set is ever returned.
//!
//! **The splitting values are deterministic.** The attempt counter starts
//! at 0 for each call and counts every δ drawn, across all factors, in the
//! order the factors are processed (a stack: the "square" side of each split
//! first). The δ of attempt c is the first 16 bytes, read big-endian and
//! reduced modulo F, of
//!
//! ```text
//! BLAKE-256( a_0 ‖ a_1 ‖ … ‖ a_degree ‖ c )
//! ```
//!
//! with each coefficient as 16 bytes big-endian (as given, before the
//! scaling to monic) and c as 8 bytes big-endian. So the same input takes
//! the same path, the same number of attempts and the same time on every
//! platform, and a failure can be replayed. The first split (δ = 0) draws
//! no value. ADR-0010 fixes only "the BLAKE-256 hash of the coefficients
//! and an attempt counter, reduced mod F"; the encodings and the use of the
//! first 16 digest bytes are this crate's (see *Departures from ADR-0010*).
//!
//! **The attempt bound.** One attempt on a factor with two or more roots
//! fails to split it only if all its roots land on the same side, which for
//! any pair of roots r ≠ s happens for at most about half of all δ (the
//! character sum Σ χ((x + r)(x + s)) over F is −1). A factor is given at
//! most [`MAX_ATTEMPTS_PER_SPLIT`] = 80 attempts, and a solve of degree n
//! has at most n − 1 factors to split, so an honest polynomial fails with
//! probability at most (n − 1)·2⁻⁸⁰ ≤ 2⁻⁷⁰ at n = [`MAX_DEGREE`]. The δ
//! sequence is public, but it is a hash of the coefficients: making a
//! polynomial whose roots the sequence cannot separate means grinding SR
//! messages, at an expected cost above 2⁷⁰ trials, beyond the 2⁶⁴ the
//! ADR asks for. A solve therefore draws at most 80·(n − 1) values, and
//! typically about 1.5·n (a median of 743 at degree 512, with no factor
//! needing more than 16 in 30 such solves).
//!
//! # Input bounds
//!
//! At most [`MAX_DEGREE`] = 1024 = `mixing.MaxMtot`
//! (`dcrd/mixing@v0.7.5/limits.go:34`): the client's size limiting never
//! lets `mtot` exceed it (`mixclient/client.go:1654`), and the wire format
//! caps an SR's inner dimension, which is `mtot`, at `MaxMixPeers` = 512
//! (`dcrd/wire@v1.8.0/msgmixslotreserve.go:81`,
//! `msgmixkeyexchange.go:22`; spec 12 §13 item 3). A larger input is
//! refused with [`Error::DegreeTooLarge`] before anything is allocated
//! for it. Every allocation is linear in the degree (the largest is
//! 2·degree accumulators of 32 bytes), so a solve at the limit needs well
//! under a megabyte.
//!
//! # Errors and the mixing client
//!
//! Every error is one outcome for the client: `Client.roots` treats any
//! non-nil error from `solverrpc.Roots` other than `ErrSolverProcessExited`,
//! and any root count other than `mtot`, alike — it sets `triggeredBlame` on
//! every local peer and returns `errTriggeredBlame`
//! (`mixclient/client.go:2162-2179`; spec 12 §5.11.1 step 3). ADR-0010 maps
//! every error of this crate to that path; `ErrSolverProcessExited`, which
//! clears the solver flag (`client.go:2163-2172`), has no counterpart: an
//! in-process solver cannot exit.
//!
//! | Error | Cause | What Go's solver does with the same input |
//! |---|---|---|
//! | [`Error::TooFewCoefficients`] | fewer than 2 coefficients (degree < 1) | `too few coefficients` (`cspp/v2@v2.4.0/solver/solver.go:42-44`): the same blame |
//! | [`Error::DegreeTooLarge`] | degree above [`MAX_DEGREE`] | factors it, and accepts one that splits: a divergence, unreachable from the client |
//! | [`Error::NonCanonical`] | a coefficient ≥ F | reduces it modulo F and answers for the reduced polynomial, accepting one that splits: a divergence, unreachable from the client |
//! | [`Error::ZeroLeadingCoefficient`] | the top coefficient is zero (includes the zero polynomial) | factors the shorter polynomial and reports fewer than `len − 1` values (none at all for the zero polynomial or a nonzero constant): the client's count check fails, the same blame |
//! | [`Error::NotSplit`] | a repeated root, or an irreducible factor of degree ≥ 2 | `repeated roots` (`solver.go:119-123`, `cmd/csppsolver/solver.go:64-68`) whenever a factor is repeated, else one value per factor, fewer than `mtot` (`solver.go:83-100`): the same blame |
//! | [`Error::AttemptsExhausted`] | a factor did not split within the bound (probability ≤ 2⁻⁷⁰ for any polynomial that splits) | FLINT succeeds: a divergence accepted by ADR-0010, never observed |
//! | [`Error::SelfCheckFailed`] | step 4 failed: a defect of this crate | none; Go does not check its roots (`client.go:2174-2196`) |
//!
//! Every row but the last two is checked against FLINT (*Parity with Go*).
//!
//! # Parity with Go
//!
//! **Checked by test** against `decred.org/cspp/v2@v2.4.0` over FLINT
//! 3.6.0, at dcrwallet 54b85b33, by `agrees_with_flint` in
//! `tests/oracle.rs` (live, where the oracle is built with FLINT: every
//! case both through the solver package in process and through
//! `solverrpc.Roots` and the `csppsolver` program, the client's own path,
//! which answer identically) and by the frozen vectors
//! `tests/data/flint_vectors.jsonl`, replayed without Go on every platform
//! by `tests/frozen.rs`. The comparison is on the client's resulting
//! action, by the rule in `tests/flint_cases/mod.rs`:
//!
//! - Go's answer and its form. `solver.RootFactors` loads the coefficients
//!   with `fmpz_mod_poly_set_coeff_fmpz`, calls `fmpz_mod_poly_factor`, and
//!   for *each* monic irreducible factor reports −(constant coefficient)
//!   mod F, a canonical `big.Int`, with its exponent, in FLINT's factor
//!   order, which is not ascending (`solver.go:41-103`); `Roots` turns any
//!   exponent other than 1 into `repeated roots` (`:113-126`; over RPC,
//!   `cmd/csppsolver/solver.go:59-74` and `solverrpc/rpc.go:138-168`).
//! - What is published. The client sorts the values by `big.Int.Cmp`
//!   (`client.go:2181-2183`) and puts each root's minimal big-endian bytes
//!   in one identical factored-polynomial message per local peer
//!   (`:2184-2195`). This crate returns the roots ascending, so the
//!   client's sort is the identity on its answer; the test compares the
//!   exact bytes the message would carry, in order, the root 0 (no bytes)
//!   included, and they are identical on every input both accept.
//! - Acceptance. On every well-formed input (canonical, at least two
//!   coefficients, nonzero leading coefficient, degree at most
//!   [`MAX_DEGREE`]: every input the client can build) the two accept the
//!   same inputs with the same roots, whatever the leading coefficient
//!   (monic, F − 1, random; spec 12 §13 item 6), and refuse the same ones:
//!   repeated roots (Go: `repeated roots`), irreducible quadratic and cubic
//!   factors and their mixtures (Go: one value per factor, or `repeated
//!   roots` when a factor repeats), random polynomials.
//!
//! **Pinned by reading** at dcrwallet 54b85b33, because no test can run the
//! client's session code around the solver:
//!
//! - What the client does with the answer: `Client.roots`
//!   (`client.go:2162-2195`), transcribed as `clientRule` in
//!   `tools/oracle/cmd/mixsolver/flint.go`; the test compares actions
//!   under that transcription. Every refusal, by either solver, sets
//!   `triggeredBlame` on every local peer (`:2173-2179`).
//! - The client's input. `Coefficients` sets `a[mtot] = F − 1` and reduces
//!   every other coefficient with `Mod`, which is Euclidean, so all are
//!   canonical (`dcrd/mixing@v0.7.5/dcnet.go:116-133`, from power sums
//!   reduced in `AddVectors`, `:88-98`; the construction itself is checked
//!   against Go for chosen roots by
//!   `client_coefficients_are_the_negated_product`), and there are
//!   `mtot + 1` of them (`client.go:1916-1919`) with `mtot` at most
//!   [`MAX_DEGREE`] (*Input bounds*). So the client only ever passes
//!   well-formed inputs, where the two solvers agree.
//! - FLINT releases other than 3.6.0. Everything the comparison uses is
//!   fixed by the mathematics; only FLINT's factor order may differ between
//!   releases, and neither the client nor this crate depends on it.
//!
//! **Divergences**, none reachable from the client, each exercised by the
//! differential and the frozen vectors:
//!
//! - A coefficient ≥ F: FLINT reduces it modulo F and accepts when the
//!   reduced polynomial splits (the client would publish), where this crate
//!   refuses with [`Error::NonCanonical`] (the client would blame). The
//!   input contract is canonical residues, which is all the client builds;
//!   reducing silently would accept inputs that no honest session makes.
//!   When the reduced polynomial does not split, or its leading coefficient
//!   reduces to 0, both refuse. (A negative or ≥ 2¹²⁸ coefficient, which
//!   Go's `big.Int` could carry, has no `u128` form.)
//! - A degree above [`MAX_DEGREE`]: FLINT factors it and accepts one that
//!   splits; this crate refuses with [`Error::DegreeTooLarge`], so that no
//!   input costs more than a degree-1024 solve. Both refuse one that does
//!   not split.
//! - The attempt bound above, and the step-4 check, which Go does not make.
//!
//! A zero leading coefficient and fewer than two coefficients are refused
//! by both, with the same client action, for different stated reasons.
//!
//! # Departures from ADR-0010
//!
//! 1. **The attempt bound is defined**: [`MAX_ATTEMPTS_PER_SPLIT`] = 80 per
//!    factor (the first split, δ = 0, draws no counter value), so an honest
//!    polynomial fails with probability at most 2⁻⁷⁰ and grinding costs
//!    more than the ADR's 2⁶⁴ trials.
//! 2. **The δ derivation is narrowed.** The ADR says each δ "is the
//!    BLAKE-256 hash of the coefficients and an attempt counter, reduced mod
//!    F" and fixes no encoding. This crate hashes each coefficient as given
//!    (before scaling to monic) as 16 bytes big-endian, then the counter as
//!    8 bytes big-endian, and reduces only the **first 16 bytes** of the
//!    32-byte digest, read big-endian, modulo F, where a literal reading
//!    would reduce all 256 bits; this keeps the derivation in `u128`
//!    arithmetic. The bias: 2¹²⁸ = 2F + 2, so the residues 0 and 1 have
//!    three 128-bit preimages and every other residue two, a statistical
//!    distance from uniform of about 2⁻¹²⁷ (exactly 6·2⁻¹²⁸ − 2/F), against
//!    which the 2⁻⁸⁰ per-factor bound and the 2⁶⁴ grinding cost are
//!    unchanged. The rule is pinned by `tests/data/self_vectors.txt`; the
//!    ADR should record it.
//! 3. **The FLINT oracle is gated differently.** The ADR has a test build
//!    the oracle under `PURSE_ORACLE_FLINT`; the shared harness builds
//!    oracles untagged, so the FLINT build is behind the Go build tag
//!    `flint`, a prebuilt binary is used through `PURSE_ORACLE_DIR` (with
//!    the reference `csppsolver` program beside it), and
//!    `PURSE_REQUIRE_FLINT=1` turns every FLINT skip into a failure.
//! 4. **[`Error::SelfCheckFailed`]** guards step 4.
//! 5. **Non-canonical coefficients and degrees above [`MAX_DEGREE`] are
//!    refused** (*Divergences*).
//! 6. **The property tests** also cover degrees 1 to 3, below the ADR's "4
//!    to 1024"; degrees above 128 run in an optimised build, or in a debug
//!    build under `PURSE_MIXSOLVER_HEAVY`, to keep the unoptimised CI legs
//!    fast.
//!
//! # Secrets and timing
//!
//! Nothing secret enters. The coefficients are a function of the summed SR
//! vectors every peer broadcasts, and the roots are published in every
//! solving peer's factored-polynomial message (`client.go:2188-2195`); which
//! root belongs to which peer is the secret, and the solver never learns
//! it: the wallet finds its own slots afterwards with a constant-time
//! search of its own (`client.go:2585-2606`). This crate is **not constant
//! time and does not try to be**: its running time and memory access depend
//! on the coefficients, on the roots (through the quadratic characters
//! that decide each split) and on the hash-derived δ values, all public. It
//! touches no randomness source, and so no run PRNG (spec 12 §4.8).
//!
//! # Performance
//!
//! A modular squaring at degree d costs about 1.5·d² multiply-accumulates,
//! each a 127×127-bit product on 64-bit limbs folded once and summed
//! without reduction; a solve costs about 2·125 such squarings at the full
//! degree (h, then the recursion's geometric series). ADR-0010's gate is a
//! median of 1 s at degree 512 on the slowest CI runner (arm64); the
//! `speed_gate` test in `tests/timing.rs` times its 21 seeded instances,
//! and `bench_table` prints medians and tails at every degree. On an x86-64
//! desktop (release build, one thread pinned to one core) the median is
//! about 0.25 s at degree 512 and 0.93 s at 1024, where FLINT takes about
//! 0.5 s and 1.1 s; the arm64 runner has not been measured.
//!
//! [ADR-0010]: https://github.com/jzbz/dcr-purse/blob/master/docs/adr/0010-mixing-root-solver.md

#![cfg_attr(not(test), no_std)]
// ADR-0010, Decision 1: no input may cause a panic.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

extern crate alloc;

mod field;
mod poly;
mod solve;

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "test code: indexing, unwrapping and plain arithmetic on test data"
)]
mod tests;

use alloc::vec::Vec;
use core::fmt;

/// The field prime F = 2¹²⁷ − 1 (`mixing.F`).
pub const FIELD_PRIME: u128 = field::P;

/// The largest degree accepted: `mixing.MaxMtot`, the most messages a
/// session can mix (`dcrd/mixing@v0.7.5/limits.go:34`).
pub const MAX_DEGREE: usize = 1024;

/// The most splitting values tried on any one factor before giving up with
/// [`Error::AttemptsExhausted`]; see the crate documentation for why 80.
pub const MAX_ATTEMPTS_PER_SPLIT: u32 = 80;

/// Why [`roots`] gave no answer. Every variant takes the mixing client's
/// triggered-blame path (see the crate documentation's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// Fewer than two coefficients: a polynomial of degree below one, which
    /// has no roots to find. Go's solver says `too few coefficients`.
    TooFewCoefficients {
        /// The number of coefficients given.
        len: usize,
    },
    /// The degree exceeds [`MAX_DEGREE`].
    DegreeTooLarge {
        /// The degree given (the number of coefficients minus one).
        degree: usize,
    },
    /// A coefficient is not a canonical residue (it is at least F).
    NonCanonical {
        /// The position of the first such coefficient.
        index: usize,
    },
    /// The leading (last) coefficient is zero, so the polynomial does not
    /// have the degree its length states. Includes the zero polynomial.
    ZeroLeadingCoefficient,
    /// The polynomial does not split into distinct linear factors over F:
    /// it has a repeated root or an irreducible factor of degree two or
    /// more (or both).
    NotSplit,
    /// A factor was not split within [`MAX_ATTEMPTS_PER_SPLIT`] attempts.
    AttemptsExhausted {
        /// The degree of the factor that would not split.
        degree: usize,
    },
    /// The roots found failed the final check. This is a defect of this
    /// crate, never a property of the input; it is reported rather than
    /// returning a wrong answer.
    SelfCheckFailed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooFewCoefficients { len } => {
                write!(f, "too few coefficients: {len}, need at least 2")
            }
            Error::DegreeTooLarge { degree } => {
                write!(f, "degree {degree} exceeds the maximum {MAX_DEGREE}")
            }
            Error::NonCanonical { index } => {
                write!(f, "coefficient {index} is not reduced modulo 2^127 - 1")
            }
            Error::ZeroLeadingCoefficient => f.write_str("leading coefficient is zero"),
            Error::NotSplit => f.write_str(
                "polynomial does not split into distinct linear factors \
                 (repeated roots or an irreducible factor)",
            ),
            Error::AttemptsExhausted { degree } => write!(
                f,
                "a factor of degree {degree} did not split within \
                 {MAX_ATTEMPTS_PER_SPLIT} attempts"
            ),
            Error::SelfCheckFailed => f.write_str("solver self-check failed"),
        }
    }
}

impl core::error::Error for Error {}

/// The roots, in ascending order, of the polynomial
/// `coefficients[0] + coefficients[1]·x + … + coefficients[n]·x^n` over
/// F = 2¹²⁷ − 1, or the reason there is no such answer.
///
/// On success the result holds exactly `n = coefficients.len() − 1`
/// distinct values, each below [`FIELD_PRIME`] and each a root; this
/// happens exactly when the polynomial has `n` distinct roots in F. The
/// checks are made in this order: the length ([`Error::TooFewCoefficients`],
/// [`Error::DegreeTooLarge`]), every coefficient ([`Error::NonCanonical`],
/// the first offender), the leading coefficient
/// ([`Error::ZeroLeadingCoefficient`]), then the polynomial itself.
///
/// The function is pure and deterministic: the same input gives the same
/// answer after the same work on every platform. It never panics.
///
/// ```
/// use purse_mixsolver::{Error, FIELD_PRIME, roots};
///
/// // (x − 2)(x − 3) = x² − 5x + 6, with leading coefficient F − 1 as
/// // mixing.Coefficients makes it: −x² + 5x − 6.
/// let f = FIELD_PRIME;
/// assert_eq!(roots(&[f - 6, 5, f - 1]), Ok(vec![2, 3]));
/// // x² + 1 has no roots: −1 is not a square modulo 2^127 − 1.
/// assert_eq!(roots(&[1, 0, 1]), Err(Error::NotSplit));
/// // (x − 2)²: a repeated root.
/// assert_eq!(roots(&[4, f - 4, 1]), Err(Error::NotSplit));
/// ```
pub fn roots(coefficients: &[u128]) -> Result<Vec<u128>, Error> {
    solve::solve(coefficients, MAX_ATTEMPTS_PER_SPLIT).map(|(roots, _)| roots)
}
