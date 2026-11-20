// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Streamlined NTRU Prime 4591^761, byte for byte as
//! `github.com/companyzero/sntrup4591761` (pseudo-version
//! `v0.0.0-20220309191932-9e0f3af2f07a`, the one dcrwallet 54b85b33 and
//! dcrd's `mixing` v0.7.5 link) computes it.
//!
//! This is the KEM of the 2017 NTRU Prime paper and of the round-1 NIST
//! submission (p = 761, q = 4591, weight 286, SHA-512 confirmation and
//! key), not the later `sntrup761`. dcrd's mixing key agreement uses it:
//! each peer's key exchange message carries a public key, and its
//! ciphertexts message one ciphertext per peer (spec 12 §4.8, §4.12). Keys
//! and ciphertexts are derived from the run's seeded ChaCha20 stream, and
//! every peer recomputes them from a revealed seed during blame
//! assignment, so the port must draw exactly Go's bytes and produce exactly
//! Go's output.
//!
//! | Go | Rust |
//! |---|---|
//! | `PublicKeySize`, `PrivateKeySize`, `CiphertextSize`, `SharedKeySize` | [`PUBLIC_KEY_SIZE`] (1218), [`PRIVATE_KEY_SIZE`] (1600), [`CIPHERTEXT_SIZE`] (1047), [`SHARED_KEY_SIZE`] (32) |
//! | `PublicKey`, `PrivateKey`, `Ciphertext`, `SharedKey` | [`PublicKey`], [`PrivateKey`], [`Ciphertext`], [`SharedKey`] |
//! | `GenerateKey(s io.Reader)` | [`generate_key`] |
//! | `Encapsulate(s io.Reader, pk)` | [`encapsulate`] |
//! | `Decapsulate(c, sk) (*SharedKey, int)` | [`decapsulate`], returning `(SharedKey, Choice)` |
//!
//! # Randomness
//!
//! Randomness comes from a [`RandomSource`], the counterpart of the
//! `io.Reader` Go takes; any `FnMut(&mut [u8]) -> Result<(), E>` closure is
//! one. Each random polynomial draws 3044 bytes (four per coefficient, read
//! as little-endian 32-bit words in coefficient order) in one
//! [`RandomSource::fill`] call. Go reads the same bytes in the same order,
//! in 761 four-byte reads, so a source that is a byte stream (dcrd's
//! ChaCha20 PRNG is one) yields identical output either way:
//!
//! | Operation | Draws, in order |
//! |---|---|
//! | [`generate_key`] | 3044 bytes for g, again for every g not invertible in R3; then 3044 bytes for f |
//! | [`encapsulate`] | 3044 bytes for r |
//! | [`decapsulate`] | nothing |
//!
//! A failing source's error is returned unchanged, as Go returns the
//! reader's. Bytes drawn before the failure are discarded, as in Go.
//!
//! The 3044 bytes of a t-small element (f, r) become coefficients through
//! a sort of the 761 words. The Go code sorts with
//! `subtle.ConstantTimeLessOrEq` applied outside its documented domain, so
//! what it computes depends on the Go release and the width of `int`
//! (spec 12 §5.10.2, Q40): built with Go 1.26 or later on a 64-bit target,
//! which is what the parity pin produces (dcrwallet's `go.mod` requires
//! `go 1.26.0`), it is an ascending sort of the words as unsigned
//! integers. **This crate implements that unsigned sort only**, as the
//! project owner decided Q40 on 2026-11-19. A Go 1.26+ build for a 32-bit
//! target sorts as signed integers, and Go 1.25 or earlier uses a
//! comparison that is not an order at all; keys and ciphertexts drawn from
//! the same bytes differ from those builds'.
//!
//! # Malformed input
//!
//! Every byte string of the right length is a [`PublicKey`],
//! [`Ciphertext`] or [`PrivateKey`], as every `[N]byte` is in Go, and each
//! operation gives Go's result on it: the decoders reproduce Go's wrapping
//! arithmetic on out-of-range encodings, and [`decapsulate`] of an invalid
//! ciphertext returns Go's key with `ok` false. Nothing panics on any
//! input. The decoders do not validate (Go's do not), with one consequence
//! an outside party can see: about 31% of valid ciphertexts have one or
//! two other encodings, differing in the last three bytes, that
//! decapsulate as valid to the same key (Quirk QK-0740). The four-byte
//! groups before them decode injectively.
//!
//! # Constant time and secrets
//!
//! Secret data (f, 1/g, g, r, the shared key, every intermediate product)
//! never selects a branch or a memory index, and no operation on it is a
//! division:
//!
//! - The multiplications (`rq::mult`, `r3::mult`) are dot products whose
//!   bounds and windows depend on the coefficient index alone, followed by
//!   multiply-and-shift reductions (a 64-bit multiply in the wide one,
//!   constant time on x86-64 and AArch64).
//! - The reciprocals run a fixed 1521 divsteps whatever the input; each
//!   step's swap is a mask and its scalar a mask selection.
//! - The sorting network runs a fixed sequence of compare-exchanges, each
//!   an arithmetic mask.
//! - Decapsulation combines its three checks with [`subtle`]'s
//!   constant-time comparisons, as Go does with `crypto/subtle`.
//!
//! Every mask passes through [`core::hint::black_box`] before it is used,
//! so the optimiser cannot turn the masked select back into a branch (a
//! conditional move LLVM may later convert to a jump); this is best effort,
//! as `black_box` documents.
//!
//! Where Go branches on secrets, and what this crate does instead:
//!
//! | Go | This crate |
//! |---|---|
//! | `GenerateKey` retries g until `r3.Reciprocal` succeeds: the running time reveals how many candidates were rejected (a random g has no inverse with probability about 3^−19, one in 1.2 billion: the modulus has a factor of degree 19 over GF(3)), nothing about the g kept | the same loop: the number of draws is part of the byte-compatible behaviour |
//! | `zx.minmax` is branch-free only because Go 1.26+ compiles `subtle.ConstantTimeLessOrEq` to a flag-setting instruction, outside the function's documented domain | an arithmetic borrow mask |
//! | nothing else: the rest of Go's arithmetic is the branch-free C reference's | — |
//!
//! Go's caller branches on `Decapsulate`'s `ok`, which is public (the run
//! blames the sender of an invalid ciphertext). Go's results are reproduced
//! byte for byte whatever the code path: the multiplications compute the
//! exact product Go's reductions give, and the reciprocals the unique
//! inverse Go's loop gives (see each function's documentation).
//!
//! The timing test `secret_independence` (`tests/timing.rs`, ignored) is a
//! dudect-style check with five class pairs: key generation from a fixed
//! against random streams (g and f), encapsulation with a fixed against
//! random r, and decapsulation of a fixed against random honest
//! ciphertexts under one key (the r it decrypts), of valid against
//! invalid ciphertexts under one key, and under a fixed against random
//! private keys, each with an honest ciphertext (f and 1/g). No class
//! differed (|t| ≤ 3.1 in three runs with 20,000 samples per class, 2,000
//! for key generation, against dudect's threshold of 4.5) on an x86_64
//! Linux desktop. The machine code was not audited instruction by
//! instruction.
//!
//! [`PrivateKey`] and [`SharedKey`] are held on the heap, wiped on drop
//! and printed by `Debug` as their length only; the random bytes and
//! words, polynomials, products, the reciprocals' working arrays, the
//! SHA-512 output and (through `sha2`'s `zeroize` feature) the hasher state
//! are wiped too. Go wipes none of them. Copies the compiler makes in
//! registers and spilled stack slots are not wiped; no Rust code can reach
//! them.
//!
//! # Speed
//!
//! Go reduces after every term of its schoolbook multiplications and runs
//! a quadratic Euclidean loop with a modular inversion per step for each
//! reciprocal. This crate sums each product coefficient as one vectorised
//! dot product and reduces once, and inverts by divsteps (bitsliced in
//! R3, Montgomery products in Rq), keeping Go's simple code as the test
//! model (`src/reference.rs`) the optimised functions are checked against.
//! Measured by `tests/timing.rs` (`speed_against_go`, release build, one
//! core of an x86_64 Linux desktop, Go 1.27.4), median times of 205 runs
//! (the direct port's from one earlier session with its own Go times,
//! 18.6 ms, 2.34 ms and 6.43 ms):
//!
//! | Operation | Go | Direct port | This crate | This crate / Go |
//! |---|---|---|---|---|
//! | [`generate_key`] | 18.7 ms | 8.43 ms | 1.40 ms | 0.075 |
//! | [`encapsulate`] | 2.38 ms | 2.26 ms | 70 µs | 0.029 |
//! | [`decapsulate`] | 6.54 ms | 6.49 ms | 149 µs | 0.023 |
//!
//! Most of key generation is the reciprocal in Rq (about 1.3 ms); most of
//! encapsulation is one multiplication (about 47 µs) and the sorting
//! network (about 20 µs).
//!
//! # Status
//!
//! Checked against Go by the oracle (`tools/oracle/cmd/sntrup`: the KEM,
//! and each exported `zx`, `rq` and `r3` function the KEM is built from),
//! by frozen oracle vectors, by the upstream Sage test (`tests/`,
//! `src/tests.rs`), by property tests (among them decapsulation's three
//! validity checks, each on inputs that fail it alone, which the oracle
//! also compares), by the cargo-fuzz target
//! `sntrup_decapsulate`, and, for the optimised arithmetic, against the
//! test model on arbitrary inputs, inputs with no inverse included.
//!
//! Not replayed, an open item: the known-answer file of the round-1 NIST
//! submission package. No copy of it is in this repository, in the Go
//! module the oracle builds from (which ships the Sage file alone), or in
//! any other local checkout or module cache, and it was not downloaded.
//! The Sage replay is the cross-check against an implementation other
//! than Go's meanwhile: it fixes key derivation from given f and g,
//! encapsulation from a given r, and decapsulation, but not the mapping
//! from random bytes to f and r, where the sort lives. Expected but not
//! verified: the file's decapsulations would match; its key generations
//! and encapsulations draw f and r through the C reference's own sort,
//! which, if it orders the words as signed integers as a 32-bit Go build
//! does, would not match the unsigned sort here.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod arith;
mod conv;
mod kem;
mod r3;
mod rq;
mod zx;

#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;

pub use kem::{
    CIPHERTEXT_SIZE, Ciphertext, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE, PrivateKey, PublicKey,
    SHARED_KEY_SIZE, SharedKey, decapsulate, encapsulate, generate_key,
};
pub use subtle::{Choice, ConstantTimeEq};

/// A source of random bytes: the counterpart of the `io.Reader` Go's
/// `GenerateKey` and `Encapsulate` take.
///
/// Implemented by every `FnMut(&mut [u8]) -> Result<(), E>` closure. Each
/// random polynomial is one `fill` of 3044 bytes; see the crate
/// documentation for what each operation draws.
pub trait RandomSource {
    /// The error a failed draw returns, passed through unchanged.
    type Error;

    /// Fills all of `buf` with random bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot supply the bytes; `buf` may
    /// then be partly written, and the operation discards it.
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), Self::Error>;
}

impl<F, E> RandomSource for F
where
    F: FnMut(&mut [u8]) -> Result<(), E>,
{
    type Error = E;

    fn fill(&mut self, buf: &mut [u8]) -> Result<(), E> {
        self(buf)
    }
}
