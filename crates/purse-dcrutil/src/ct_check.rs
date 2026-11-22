// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Checks of the WIF base58 codec (ADR-0012, addendum of 2026-11-21, and
//! its note of 2026-11-22): purse-hdkeychain's `ct_check` at the lengths
//! of a WIF, a disassembly check of the test binary's own code
//! (`purse-testutil`'s `disasm`) and dudect's fixed-against-random t-test
//! on `purse-testutil`'s harness (`crates/purse-testutil/src/ct.rs`, after
//! dcroxide's), with a positive control that must be found. Every test is
//! ignored and refuses a debug build; run them one at a time in release
//! mode, the timing targets pinned to one processor, as
//! `.github/workflows/ci.yml`'s constant-time job does, which also runs
//! them all at opt-level "s" and `disassembly` at opt-levels 1, 2 and
//! "z":
//!
//! ```text
//! cargo test --release -p purse-dcrutil --lib --locked --no-run
//! taskset -c 3 cargo test --release -p purse-dcrutil --lib --locked ct_check -- --ignored --test-threads=1 --nocapture
//! CARGO_PROFILE_RELEASE_OPT_LEVEL=s cargo test --release -p purse-dcrutil --lib --locked ct_check -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `PURSE_CT_SECONDS` sets each timing target's budget (10 s when unset).
//!
//! `disassembly` (x86_64 only; it needs GNU binutils' `nm` and `objdump`)
//! disassembles six functions of the test binary: two probes, each one
//! lookup out of line with no loop (`digit_probe`, `char_probe`), which
//! must have no conditional jump, no call and no memory access but a
//! fixed address or an unindexed stack slot; the two loops the codec
//! calls (`map_digits`, `map_chars`), which must have no call, take no
//! table's address, and jump conditionally only backward or out of every
//! loop; and the big-number arithmetic, the encoder's (`value_digits`)
//! and the whole decoder (`decode_wif`), which may branch on the value's
//! magnitude and call, and must have no division instruction. It is the
//! only check here that sees a lookup made a table index again (the
//! encoder's form before 2026-11-22, `ALPHABET[d]`) or a division the
//! optimiser kept (the encoder's `val / 58` and `val % 58` before the
//! same date, a `div` at opt-level "z"), and it sees a select compiled
//! into a branch at whatever opt-level it is built with. Its rules and
//! what they cannot see are in `disasm`'s documentation.
//!
//! The timing targets, each against a random class:
//!
//! - `digits`: the decoder's map of a 53-character string to digits
//!   (`map_digits`), on strings of random alphabet characters; fixed
//!   classes all `1`, all `z`, all `0` and all 0xff (the last two outside
//!   the alphabet), and a placebo.
//! - `chars`: the encoder's map of 53 digits to characters (`map_chars`),
//!   on random digits; fixed classes all 0, all 57, and a placebo.
//! - `decode`: `decode_wif` of a mainnet ECDSA WIF string whose network and
//!   signature type bytes are fixed and whose 36 secret bytes (the key and
//!   the checksum) are random; fixed classes all 0x00, all 0xff, one fixed
//!   draw, and a placebo.
//! - `encode`: the encoder (`base58::encode`) of the same payloads.
//! - `control`: the linear search the codec used before, through an opaque
//!   alphabet, on the `digits` strings, with fixed classes all `1` (where
//!   it stops first) and all `z` (where it stops last): a leak must be
//!   found in one of them. Which way all `z` differs depends on the
//!   processor, since a random string's characters mispredict where the
//!   search stops and a repeated one's do not.
//!
//! What the timing targets can and cannot find. A branch on the secret is
//! what they see: at opt-level "s" the first constant-time lookup's
//! branch made `digits` fail by hundreds of nanoseconds an operation.
//! Only `digits` reliably finds a variable-time decoder lookup: whether
//! `decode` does depends on how the compiler lays the lookup out (the old
//! search, as it was written, passed it, and the same search written
//! another way failed it). `chars` and `encode` cannot see the encoder's
//! old form, a load from the 58-byte alphabet indexed by the digit, which
//! stays in the first-level cache; with that form restored, both passed.
//! For the rest see the harness's documentation: they are statistical,
//! they time one process's wall clock, and they fail only on a difference
//! of a nanosecond an operation or more. A pass is not a proof.

use std::hint::black_box;

use purse_testutil::SplitMix64;
use purse_testutil::ct::{PLACEBO, time_target};
#[cfg(target_arch = "x86_64")]
use purse_testutil::disasm::{Shape, assert_clean};

use crate::base58::{
    ALPHABET, WIF_LEN, char_of, decode_wif, digit_by_search, digit_plus_one, encode as encode_wif,
    map_chars, map_digits,
};

/// The length of a WIF string on mainnet: 39 bytes beginning with `22 de`
/// encode to 53 characters.
const WIF_STRING_LEN: usize = 53;

/// The payload's public bytes: mainnet's `PrivateKeyID` and signature
/// type 0.
const PUBLIC: [u8; 3] = [0x22, 0xde, 0x00];

/// The secret bytes after them: the key (32) and the checksum (4), which
/// is a function of it.
const SECRET_LEN: usize = WIF_LEN - PUBLIC.len();

fn refuse_debug() {
    if cfg!(debug_assertions) {
        panic!("the timing check is defined on a release build: run it with --release");
    }
}

/// `len` random alphabet characters.
fn random_chars(rng: &mut SplitMix64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|_| ALPHABET[usize::try_from(rng.below(58)).expect("below 58")])
        .collect()
}

fn to_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().expect("the class's length")
}

/// The payload of `secret` behind mainnet's public bytes.
fn payload(secret: &[u8]) -> [u8; WIF_LEN] {
    let mut p = [0u8; WIF_LEN];
    p[..PUBLIC.len()].copy_from_slice(&PUBLIC);
    p[PUBLIC.len()..].copy_from_slice(secret);
    p
}

fn secret_classes() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("all 0x00", vec![0; SECRET_LEN]),
        ("all 0xff", vec![0xff; SECRET_LEN]),
        (
            "fixed draw",
            SplitMix64::for_label("ct_check fixed secret", 0).bytes(SECRET_LEN),
        ),
        (PLACEBO, Vec::new()),
    ]
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn digits() {
    refuse_debug();
    time_target(
        "dcrutil digits",
        &[
            ("all 1", vec![b'1'; WIF_STRING_LEN]),
            ("all z", vec![b'z'; WIF_STRING_LEN]),
            ("all 0 (outside)", vec![b'0'; WIF_STRING_LEN]),
            ("all 0xff (outside)", vec![0xff; WIF_STRING_LEN]),
            (PLACEBO, Vec::new()),
        ],
        |rng| random_chars(rng, WIF_STRING_LEN),
        to_array::<WIF_STRING_LEN>,
        |input| {
            let mut digits = [0u8; WIF_STRING_LEN];
            let valid = map_digits(input, &mut digits);
            (valid, digits)
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn chars() {
    refuse_debug();
    time_target(
        "dcrutil chars",
        &[
            ("all 0", vec![0; WIF_STRING_LEN]),
            ("all 57", vec![57; WIF_STRING_LEN]),
            (PLACEBO, Vec::new()),
        ],
        |rng| {
            (0..WIF_STRING_LEN)
                .map(|_| u8::try_from(rng.below(58)).expect("below 58"))
                .collect()
        },
        to_array::<WIF_STRING_LEN>,
        |input| {
            let mut chars = *input;
            map_chars(&mut chars);
            chars
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn decode() {
    refuse_debug();
    time_target(
        "dcrutil decode",
        &secret_classes(),
        |rng| rng.bytes(SECRET_LEN),
        |secret| {
            let s = encode_wif(&payload(secret));
            // Every class's string has the one length: the length is public,
            // and a class of another would time it.
            to_array::<WIF_STRING_LEN>(s.as_bytes())
        },
        |input| decode_wif(input).expect("39 bytes"),
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn encode() {
    refuse_debug();
    time_target(
        "dcrutil encode",
        &secret_classes(),
        |rng| rng.bytes(SECRET_LEN),
        payload,
        |input| {
            let s = encode_wif(input);
            assert_eq!(s.len(), WIF_STRING_LEN);
            s
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn control() {
    refuse_debug();
    time_target(
        "dcrutil control (linear search)",
        &[
            ("all 1", vec![b'1'; WIF_STRING_LEN]),
            ("all z", vec![b'z'; WIF_STRING_LEN]),
        ],
        |rng| random_chars(rng, WIF_STRING_LEN),
        to_array::<WIF_STRING_LEN>,
        |input| input.map(digit_by_search),
    )
    .assert_leak_found();
}

/// One character's lookup, out of line: the code [`map_digits`] runs on
/// each character, without the loop around it, for [`disassembly`].
#[inline(never)]
fn digit_probe(c: u8) -> u8 {
    digit_plus_one(c)
}

/// One digit's lookup, out of line: the code [`map_chars`] runs on each
/// digit, without the loop around it, for [`disassembly`].
#[inline(never)]
fn char_probe(digit: u8) -> u8 {
    char_of(digit)
}

#[cfg(target_arch = "x86_64")]
#[test]
#[ignore = "disassembly check: cargo test --release -p purse-dcrutil --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn disassembly() {
    refuse_debug();
    // Called through opaque pointers, so that both probes are kept.
    let probes: [fn(u8) -> u8; 2] = black_box([digit_probe, char_probe]);
    assert_eq!(probes[0](b'z'), 58);
    assert_eq!(probes[1](57), b'z');
    assert_clean(&[
        ("purse_dcrutil::ct_check::digit_probe", Shape::StraightLine),
        ("purse_dcrutil::ct_check::char_probe", Shape::StraightLine),
        ("purse_dcrutil::base58::map_digits", Shape::Loop),
        ("purse_dcrutil::base58::map_chars", Shape::Loop),
        ("purse_dcrutil::base58::value_digits", Shape::Branching),
        ("purse_dcrutil::base58::decode_wif", Shape::Branching),
    ]);
}
