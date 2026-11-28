// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Checks of the base58 codec's handling of secret strings (ADR-0012,
//! addendum of 2026-11-21, and its note of 2026-11-22): a disassembly
//! check of the test binary's own code (`purse-testutil`'s `disasm`), and
//! dudect's fixed-against-random t-test on `purse-testutil`'s harness
//! (`crates/purse-testutil/src/ct.rs`, after dcroxide's), with a positive
//! control that must be found. Every test is ignored and refuses a debug
//! build; run them one at a time in release mode, the timing targets
//! pinned to one processor, as CI does: `.github/workflows/constant-time.yml`
//! runs the timing targets at opt-levels 3 and "s" (on a change to the
//! paths that can move their result, and nightly), and
//! `.github/workflows/ci.yml`'s constant-time job runs `disassembly` at
//! opt-levels 3, "s", 1, 2 and "z" on every push (the two moved apart on
//! 2026-11-22, ADR-0012, the pointer of that date in the same note):
//!
//! ```text
//! cargo test --release -p purse-hdkeychain --lib --locked --no-run
//! taskset -c 3 cargo test --release -p purse-hdkeychain --lib --locked ct_check -- --ignored --test-threads=1 --nocapture
//! CARGO_PROFILE_RELEASE_OPT_LEVEL=s cargo test --release -p purse-hdkeychain --lib --locked ct_check -- --ignored --test-threads=1 --nocapture
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
//! and the whole decoder (`decode_extended_key`), which may branch on the
//! value's magnitude and call, and must have no division instruction. It
//! is the only check here that sees a lookup made a table index again
//! (the encoder's form before 2026-11-22, `ALPHABET[d]`) or a division
//! the optimiser kept (the encoder's `val / 58` and `val % 58` before
//! the same date, a `div` at opt-level "z"), and it sees a select
//! compiled into a branch at whatever opt-level it is built with. Its
//! rules and what they cannot see are in `disasm`'s documentation.
//!
//! The timing targets, each against a random class:
//!
//! - `digits`: the decoder's map of a 111-character string to digits
//!   (`map_digits`, which runs `digit_plus_one` on every character), on
//!   strings of random alphabet characters; fixed classes all `1` (digit 0,
//!   where the old search stopped first), all `z` (digit 57, where it
//!   stopped last), all `0` and all 0xff (outside the alphabet), and a
//!   placebo.
//! - `chars`: the encoder's map of 111 digits to characters (`map_chars`),
//!   on random digits; fixed classes all 0, all 57, and a placebo.
//! - `chars_diagnostic`, since 2026-11-24 (ADR-0012, addendum of that
//!   date): a diagnostic pass of `chars`, a test of its own run right after
//!   it, for the same budget, through `time_diagnostic`, printed and never
//!   judged, so that it cannot fail; `chars` itself is as it was. Its
//!   classes are the three of `chars` and two more: a fixed draw of digits,
//!   one input drawn once from the harness's generator and spread over the
//!   digits as the random class's are, and all 0 again under a name of its
//!   own. Each of its six classes (the random class among them) has a share
//!   of 1/6, where each of `chars`'s four has 1/4. The target failed once on
//!   GitHub with all 0 quicker than the random class. If a difference
//!   follows the repetition of one input, the fixed draw leans with all 0;
//!   if it follows the value, the two copies of all 0 lean together and the
//!   fixed draw does not. The pass times its own later window, with its own
//!   warm-up and thresholds, so it shows only a difference that lasts into
//!   that window.
//! - `decode`: `decode_extended_key` of a mainnet extended private key
//!   string whose public fields (version, depth, parent fingerprint, child
//!   number) are fixed and whose 69 secret bytes (chain code, key and
//!   checksum) are random; fixed classes all 0x00, all 0xff, one fixed
//!   draw, and a placebo.
//! - `encode`: `encode_extended_key` of the same payloads.
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
//!
//! `chars` copies its input onto the stack, maps the copy and returns it,
//! so that its timed region holds copies of the 111 bytes before and after
//! the call of `map_chars`. A form that mapped the harness's input in
//! place, with no copy, was built and measured on 2026-11-26 with leaks
//! planted in `map_chars`, and was not adopted. It removed the copies'
//! failed store-to-load forwards, and it traded one plant's detection for
//! another's. Here at opt-level 3, in five rebuilds that moved code (one
//! of them by changing the number of codegen units), the multiplication
//! plant (the one the note of 2026-11-25 is about) and the division
//! plant were found in 50 of 50 runs each in place against 21 and 27 of
//! 50 with the copy; but the branch plant here at 3 was found in 16 of
//! 50 against 44, and the division plant at "s" in 20 of 50 against 50,
//! with the same losses in the unperturbed binaries. Across the twelve
//! plant cells it narrowed how far a plant's cost moves between such
//! builds for some and widened it for others (ADR-0012, addendum of
//! 2026-11-26).

extern crate std;

use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;

use purse_testutil::SplitMix64;
use purse_testutil::ct::{PLACEBO, time_diagnostic, time_target};
#[cfg(target_arch = "x86_64")]
use purse_testutil::disasm::{Shape, assert_clean};

use crate::base58::{
    ALPHABET, char_of, decode_extended_key, digit_by_search, digit_plus_one, encode_extended_key,
    map_chars, map_digits,
};
use crate::{DECODED_DATA_LEN, HdVersions};

/// The length of an extended key string on mainnet: 82 bytes beginning
/// with `dprv` or `dpub`'s version bytes encode to 111 characters.
const KEY_STRING_LEN: usize = 111;

/// The payload's public fields: mainnet's private version, depth 0, parent
/// fingerprint 0, child number 0.
const PUBLIC_LEN: usize = 13;

/// The secret bytes after them: chain code (32), the key's pad byte and
/// key (33) and the checksum (4), which is a function of them.
const SECRET_LEN: usize = DECODED_DATA_LEN - PUBLIC_LEN;

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

fn string_classes() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("all 1", vec![b'1'; KEY_STRING_LEN]),
        ("all z", vec![b'z'; KEY_STRING_LEN]),
        ("all 0 (outside)", vec![b'0'; KEY_STRING_LEN]),
        ("all 0xff (outside)", vec![0xff; KEY_STRING_LEN]),
        (PLACEBO, Vec::new()),
    ]
}

fn to_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().expect("the class's length")
}

/// The payload of `secret` behind mainnet's public fields.
fn payload(secret: &[u8]) -> [u8; DECODED_DATA_LEN] {
    let mut p = [0u8; DECODED_DATA_LEN];
    p[..4].copy_from_slice(&HdVersions::MAINNET.private);
    p[PUBLIC_LEN..].copy_from_slice(secret);
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
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn digits() {
    refuse_debug();
    time_target(
        "hdkeychain digits",
        &string_classes(),
        |rng| random_chars(rng, KEY_STRING_LEN),
        to_array::<KEY_STRING_LEN>,
        |input| {
            let mut digits = [0u8; KEY_STRING_LEN];
            let valid = map_digits(input, &mut digits);
            (valid, digits)
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn chars() {
    refuse_debug();
    time_target(
        "hdkeychain chars",
        &[
            ("all 0", vec![0; KEY_STRING_LEN]),
            ("all 57", vec![57; KEY_STRING_LEN]),
            (PLACEBO, Vec::new()),
        ],
        |rng| {
            (0..KEY_STRING_LEN)
                .map(|_| u8::try_from(rng.below(58)).expect("below 58"))
                .collect()
        },
        to_array::<KEY_STRING_LEN>,
        |input| {
            let mut chars = *input;
            map_chars(&mut chars);
            chars
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn decode() {
    refuse_debug();
    time_target(
        "hdkeychain decode",
        &secret_classes(),
        |rng| rng.bytes(SECRET_LEN),
        |secret| {
            let s = encode_extended_key(&payload(secret));
            // Every class's string has the one length: the length is public,
            // and a class of another would time it.
            to_array::<KEY_STRING_LEN>(s.as_bytes())
        },
        |input| decode_extended_key(input).expect("82 bytes"),
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn encode() {
    refuse_debug();
    time_target(
        "hdkeychain encode",
        &secret_classes(),
        |rng| rng.bytes(SECRET_LEN),
        payload,
        |input| {
            let s = encode_extended_key(input);
            assert_eq!(s.len(), KEY_STRING_LEN);
            s
        },
    )
    .assert_constant_time();
}

#[test]
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn control() {
    refuse_debug();
    time_target(
        "hdkeychain control (linear search)",
        &[
            ("all 1", vec![b'1'; KEY_STRING_LEN]),
            ("all z", vec![b'z'; KEY_STRING_LEN]),
        ],
        |rng| random_chars(rng, KEY_STRING_LEN),
        to_array::<KEY_STRING_LEN>,
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
#[ignore = "disassembly check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn disassembly() {
    refuse_debug();
    // Called through opaque pointers, so that both probes are kept.
    let probes: [fn(u8) -> u8; 2] = black_box([digit_probe, char_probe]);
    assert_eq!(probes[0](b'z'), 58);
    assert_eq!(probes[1](57), b'z');
    assert_clean(&[
        (
            "purse_hdkeychain::ct_check::digit_probe",
            Shape::StraightLine,
        ),
        (
            "purse_hdkeychain::ct_check::char_probe",
            Shape::StraightLine,
        ),
        ("purse_hdkeychain::base58::map_digits", Shape::Loop),
        ("purse_hdkeychain::base58::map_chars", Shape::Loop),
        ("purse_hdkeychain::base58::value_digits", Shape::Branching),
        (
            "purse_hdkeychain::base58::decode_extended_key",
            Shape::Branching,
        ),
    ]);
}

/// `len` random digits, each below 58, drawn as the `chars` target's
/// random class draws its digits.
fn random_digits(rng: &mut SplitMix64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|_| u8::try_from(rng.below(58)).expect("below 58"))
        .collect()
}

/// The classes of the `chars` target's diagnostic pass (the module's
/// documentation): the target's three, as `chars` names them, then a fixed
/// draw of digits and all 0 again.
fn chars_diagnostic_classes() -> [Vec<(&'static str, Vec<u8>)>; 2] {
    let judged = vec![
        ("all 0", vec![0; KEY_STRING_LEN]),
        ("all 57", vec![57; KEY_STRING_LEN]),
        (PLACEBO, Vec::new()),
    ];
    let diagnostic = vec![
        (
            "fixed draw",
            random_digits(
                &mut SplitMix64::for_label("ct_check fixed digits", 0),
                KEY_STRING_LEN,
            ),
        ),
        ("all 0 again", vec![0; KEY_STRING_LEN]),
    ];
    [judged, diagnostic]
}

/// The diagnostic pass of `chars`, a test of its own that the timing
/// workflow runs right after `chars`, in the same step: printed and never
/// judged, so it cannot fail (`purse_testutil::ct`, "The diagnostic
/// pass"). It is kept out of `chars` itself, whose code is as it was before
/// the pass was added.
#[test]
#[ignore = "timing check: cargo test --release -p purse-hdkeychain --lib ct_check -- --ignored --test-threads=1 --nocapture"]
fn chars_diagnostic() {
    refuse_debug();
    let [judged, diagnostic] = chars_diagnostic_classes();
    time_diagnostic(
        "hdkeychain chars",
        &judged,
        &diagnostic,
        |rng| random_digits(rng, KEY_STRING_LEN),
        to_array::<KEY_STRING_LEN>,
        |input| {
            let mut chars = *input;
            map_chars(&mut chars);
            chars
        },
    )
    .print_not_judged();
}

/// The diagnostic pass's classes: the three judged ones of `chars`, then a
/// fixed draw of 111 digits, spread over the digits as a random draw is,
/// and all 0 under a second name.
#[test]
fn chars_diagnostic_classes_are_as_documented() {
    let [judged, diagnostic] = chars_diagnostic_classes();
    let names: Vec<&str> = judged.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["all 0", "all 57", PLACEBO]);
    assert_eq!(judged[0].1, vec![0; KEY_STRING_LEN]);
    assert_eq!(judged[1].1, vec![57; KEY_STRING_LEN]);
    assert!(judged[2].1.is_empty());
    let names: Vec<&str> = diagnostic.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["fixed draw", "all 0 again"]);
    let draw = &diagnostic[0].1;
    assert_eq!(draw.len(), KEY_STRING_LEN);
    assert!(draw.iter().all(|&d| d < 58));
    let mut seen = [0usize; 58];
    for &d in draw {
        seen[usize::from(d)] += 1;
    }
    assert!(seen.iter().all(|&n| n <= 8), "{seen:?}");
    assert!(seen.iter().filter(|&&n| n > 0).count() > 40, "{seen:?}");
    assert_eq!(diagnostic[1].1, judged[0].1);
}
