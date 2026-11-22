// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Base58 (decred/base58 v1.0.6 `Encode`/`Decode`, whose outputs
//! `dcroxide-base58` gives) specialised to the 82-byte extended key, over
//! fixed buffers that are wiped, so neither direction leaves a private
//! key's bytes or digits in freed memory. Equivalence with
//! `dcroxide-base58` is tested over the parse corpus and random and mutated
//! strings.
//!
//! # Timing
//!
//! A character's digit ([`digit_plus_one`]) and a digit's character
//! ([`char_of`]) are computed from the bounds of the alphabet's six runs
//! of consecutive characters ([`RUNS`]) by subtractions, ORs, ANDs and
//! shifts, written with no comparison, no table and no read of the
//! alphabet (ADR-0012, addendum of 2026-11-21, and its note of
//! 2026-11-22). The masks that say which run a character is in (or which
//! runs a digit is past) are packed into one word and passed through
//! [`black_box`] before they select anything. Without that barrier the
//! optimiser sees each mask as a comparison and each masked AND as a
//! `select`, which it may compile into a conditional branch: this codec's
//! first constant-time lookup, a masked scan of the alphabet with only the
//! alphabet behind `black_box`, compiled to a branch on the secret
//! character at opt-level "s" on x86_64, and in the crate's own object
//! code before linking. The barrier also keeps the loops scalar, the shape
//! the disassembly check below knows. The compiler may still form a mask
//! with a compare and a conditional move or set-on-condition (it does at
//! "s"), which take the same time either way; what must not appear is a
//! conditional branch or an indexed load. `black_box` is documented as
//! best effort, so the property is checked, not assumed. decred/base58
//! looks each character up in a 256-entry table (`b58[r]`) and each
//! digit's character up in the alphabet (`alphabet[b]`), memory accesses
//! indexed by the secret; the outputs are the same, so this is no
//! divergence.
//!
//! What was checked, and where: on x86_64, the lookups compile to code
//! with no conditional branch and no memory access that depends on the
//! character or the digit, and the big-number arithmetic to code with no
//! division instruction, at opt-levels 1, 2, 3, "s" and "z", by
//! `ct_check`'s disassembly check of the test binary at each level (an
//! x86_64 Linux desktop, the pinned compiler); and dudect's timing test
//! found no difference at opt-levels 3 and "s" in runs on the same
//! desktop that are not recorded. No other architecture or compiler was
//! looked at. A consumer that builds this crate another way gets whatever
//! code its compiler emits, which these checks did not see.
//!
//! What still takes a time that depends on the input:
//!
//! - **The length** of the string or of the payload, which sets every
//!   loop's count. It is public.
//! - **Whether every character is in the alphabet.** The decoder maps the
//!   whole string first and then returns `None` if any character was not
//!   in it, before the big-number arithmetic: an early exit kept because
//!   the result (`None`) says the same. Where the invalid character stands
//!   is not shown by the time, only that there was one.
//! - **The count of leading `1` characters** (leading zero bytes), counted
//!   by a loop that stops at the first other one. They encode the payload's
//!   leading bytes, which in an extended key are its version, never zero
//!   for the four networks: the loop reads one digit.
//! - **The big-number arithmetic.** The decoder grows its limbs, and the
//!   encoder its digits, as the value read so far needs them, so the work
//!   follows the magnitude of each prefix of the value. That magnitude is
//!   set, but for a carry near a limb's or digit's boundary, by the
//!   payload's most significant bytes, the version, depth, parent
//!   fingerprint and child number, which are public; not by the chain code
//!   or the key, except through such carries. Its operations are
//!   additions, subtractions, shifts and multiplications, including the
//!   encoder's division by 58, which is written as a multiplication by a
//!   constant and a shift ([`divmod58`]) rather than left to the
//!   optimiser: at opt-level "z" rustc compiled this encoder's former
//!   `val / 58` and `val % 58` to a hardware divide (`div`) on x86_64,
//!   whose latency depends on its operands on several x86-64 and AArch64
//!   cores, on a dividend that follows every payload byte before it
//!   (levels 1, 2, 3 and "s" turned it into a multiplication). Those
//!   operations, in their 32- and 64-bit forms, take a time that does not
//!   depend on their operands on x86-64 and AArch64 cores; that is the
//!   cores' documented behaviour, not something these checks measured,
//!   and a 32-bit target's 64-bit multiplication (the decoder's) was not
//!   considered.
//!
//! `ct_check` (in this crate's `src/ct_check.rs`, ignored; release builds)
//! holds the checks. `disassembly` reads the test binary's own code for
//! one lookup in each direction and for the two loops ([`map_digits`],
//! [`map_chars`]); it is what catches a lookup made a table index again,
//! which no timing target can see (a 58-byte table stays in the
//! first-level cache), and a select made a branch, at whatever opt-level
//! the test is built with. It also reads the big-number arithmetic, the
//! encoder's ([`value_digits`]) and the whole decoder
//! ([`decode_extended_key`]), and fails on any division instruction
//! there; their branches on the magnitude, which the list above allows,
//! it does not judge. The timing targets run dudect's
//! fixed-against-random t-test with the linear search this codec used
//! before as a positive control; of them only `digits` reliably finds a
//! variable-time decoder lookup (whether `decode` does depends on how the
//! compiler lays the search out), and `chars` and `encode` time the
//! encoder's path without being able to see a table load.

use alloc::string::String;
use core::hint::black_box;

use zeroize::Zeroizing;

use crate::DECODED_DATA_LEN;

/// The Bitcoin base58 alphabet.
pub(crate) const ALPHABET: &[u8; 58] =
    b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// The longest input [`decode_extended_key`] accepts, the caller's own
/// limit (`MAX_KEY_LEN`, 113 characters).
pub(crate) const MAX_INPUT: usize = crate::MAX_KEY_LEN;

/// 113 base58 digits are below 58^113 < 2^662, so 21 32-bit limbs hold any
/// accepted input's value; 24 leave room.
const LIMBS: usize = 24;

/// The most digits an 82-byte input encodes to: `82 * 137 / 100 + 1`.
const MAX_DIGITS: usize = DECODED_DATA_LEN * 137 / 100 + 1;

/// One of the alphabet's runs of consecutive ASCII characters: its first
/// character, that character's digit, and its last character.
type Run = (u8, u8, u8);

/// The alphabet's six runs: `1`-`9`, `A`-`H`, `J`-`N`, `P`-`Z`, `a`-`k` and
/// `m`-`z`. The lookups compute with these bounds and never read the
/// alphabet; a compile-time assertion below checks that they spell
/// [`ALPHABET`], and `lookups_match_the_search_for_every_byte` checks the
/// lookups against the old search over every byte value.
const RUNS: [Run; 6] = [
    (b'1', 0, b'9'),
    (b'A', 9, b'H'),
    (b'J', 17, b'N'),
    (b'P', 22, b'Z'),
    (b'a', 33, b'k'),
    (b'm', 44, b'z'),
];

/// Whether [`RUNS`] spell [`ALPHABET`]: each run's characters, in order,
/// at the digits that follow the previous run's, and nothing else.
const fn runs_spell_alphabet() -> bool {
    let mut digit = 0usize;
    let mut run = 0usize;
    while run < RUNS.len() {
        let (first, start, last) = RUNS[run];
        if start as usize != digit || last < first {
            return false;
        }
        let mut c = first;
        while c <= last {
            if digit >= ALPHABET.len() || ALPHABET[digit] != c {
                return false;
            }
            digit = digit.wrapping_add(1);
            c = c.wrapping_add(1);
        }
        run = run.wrapping_add(1);
    }
    digit == ALPHABET.len()
}

const _: () = assert!(runs_spell_alphabet(), "RUNS must spell ALPHABET");

/// 0xff when `x`, a difference of two values below 2^8 taken in 32 bits,
/// is not negative (its bit 31 is clear), and 0 when it is: arithmetic
/// only, no comparison.
#[inline(always)]
fn nonnegative(x: u32) -> u64 {
    u64::from(((x >> 31) as u8).wrapping_sub(1))
}

/// 0xff when `x` (a byte's value) is in `run`, and 0 otherwise: both
/// `x - first` and `last - x` are not negative.
#[inline(always)]
fn inside(x: u32, (first, _, last): Run) -> u64 {
    nonnegative(x.wrapping_sub(u32::from(first)) | u32::from(last).wrapping_sub(x))
}

/// 0xff when the digit `x` is not below `run`'s first digit, and 0
/// otherwise.
#[inline(always)]
fn at_least(x: u32, (_, start, _): Run) -> u64 {
    nonnegative(x.wrapping_sub(u32::from(start)))
}

/// Byte `lane` (0 to 7) of `masks`.
#[inline(always)]
fn lane(masks: u64, lane: u32) -> u8 {
    (masks >> lane.wrapping_mul(8)) as u8
}

/// `c`'s digit plus one if `c` is in `run`; a meaningless value otherwise,
/// which the caller masks out.
#[inline(always)]
fn digit_plus_one_in(c: u8, (first, start, _): Run) -> u8 {
    c.wrapping_sub(first).wrapping_add(start).wrapping_add(1)
}

/// How many characters lie between `run` and the run before it (`I`, `O`
/// and `l` are left out of the alphabet, and the punctuation between the
/// digits, the capitals and the small letters).
const fn gap(run: usize) -> u8 {
    RUNS[run]
        .0
        .wrapping_sub(RUNS[run.wrapping_sub(1)].2)
        .wrapping_sub(1)
}

/// `c`'s digit plus one, or 0 when `c` is not in the alphabet.
///
/// Computed from the bounds of the alphabet's runs ([`RUNS`]) by
/// subtractions, an OR and shifts, with no comparison, no branch and no
/// memory read that depends on `c`. The six masks that say which run `c`
/// is in are packed into one word and passed through [`black_box`] before
/// they select anything: without that barrier the optimiser recognises
/// each mask as a comparison and the masked select as a `select`, which it
/// may compile into a conditional branch on `c`, as it did to this codec's
/// first constant-time lookup (a masked scan of the alphabet) at opt-level
/// "s" on x86_64. The barrier hides only the masks' values; the masks are
/// computed from `c` with no branch either side of it.
#[inline(always)]
pub(crate) fn digit_plus_one(c: u8) -> u8 {
    let x = u32::from(c);
    let masks = inside(x, RUNS[0])
        | inside(x, RUNS[1]) << 8
        | inside(x, RUNS[2]) << 16
        | inside(x, RUNS[3]) << 24
        | inside(x, RUNS[4]) << 32
        | inside(x, RUNS[5]) << 40;
    let masks = black_box(masks);
    (lane(masks, 0) & digit_plus_one_in(c, RUNS[0]))
        | (lane(masks, 1) & digit_plus_one_in(c, RUNS[1]))
        | (lane(masks, 2) & digit_plus_one_in(c, RUNS[2]))
        | (lane(masks, 3) & digit_plus_one_in(c, RUNS[3]))
        | (lane(masks, 4) & digit_plus_one_in(c, RUNS[4]))
        | (lane(masks, 5) & digit_plus_one_in(c, RUNS[5]))
}

/// The character of `digit`, or 0 when `digit` is 58 or more (no caller
/// passes one).
///
/// `digit + '1'` plus the [`gap`] before each run whose first digit is not
/// above `digit`, each gap selected by a mask computed by a subtraction
/// and a shift, with no comparison, no branch and no memory read that
/// depends on the digit. The masks pass through [`black_box`] before they
/// select, for the reason [`digit_plus_one`] gives.
#[inline(always)]
pub(crate) fn char_of(digit: u8) -> u8 {
    let x = u32::from(digit);
    let masks = at_least(x, RUNS[1])
        | at_least(x, RUNS[2]) << 8
        | at_least(x, RUNS[3]) << 16
        | at_least(x, RUNS[4]) << 24
        | at_least(x, RUNS[5]) << 32
        | nonnegative(57u32.wrapping_sub(x)) << 40;
    let masks = black_box(masks);
    let c = digit
        .wrapping_add(RUNS[0].0)
        .wrapping_add(lane(masks, 0) & const { gap(1) })
        .wrapping_add(lane(masks, 1) & const { gap(2) })
        .wrapping_add(lane(masks, 2) & const { gap(3) })
        .wrapping_add(lane(masks, 3) & const { gap(4) })
        .wrapping_add(lane(masks, 4) & const { gap(5) });
    c & lane(masks, 5)
}

/// Writes the digit of each character of `input` to the first
/// `input.len()` entries of `digits` (which must have room for them) and
/// returns whether every character is in the alphabet. Every character is
/// mapped by [`digit_plus_one`], whatever came before it; the caller
/// branches only on the result, after the whole string. Kept out of line
/// (one call a string), so that `ct_check`'s disassembly check finds it.
#[inline(never)]
pub(crate) fn map_digits(input: &[u8], digits: &mut [u8]) -> bool {
    let mut missing = 0u8;
    for (digit, &c) in digits.iter_mut().zip(input) {
        let found = digit_plus_one(c);
        // 1 when `found` is 0, whose 0 - 1 sets bit 31; 0 otherwise.
        missing |= (u32::from(found).wrapping_sub(1) >> 31) as u8;
        // An invalid character's 0xff is never read: the caller stops.
        *digit = found.wrapping_sub(1);
    }
    missing == 0
}

/// Replaces each digit in `digits` (each below 58) with its character by
/// [`char_of`]. Kept out of line (one call a string), so that `ct_check`'s
/// disassembly check finds it.
#[inline(never)]
pub(crate) fn map_chars(digits: &mut [u8]) {
    for d in digits {
        *d = char_of(*d);
    }
}

/// The digit of `c` by a linear search of the alphabet that stops at the
/// match, which this codec used until 2026-11-22 and whose time grows with
/// the digit: the reference of the exhaustive test and the positive
/// control of `ct_check`. The alphabet goes through [`black_box`], so that
/// the compiler keeps the search a loop and the control a leak.
#[cfg(test)]
pub(crate) fn digit_by_search(c: u8) -> Option<u8> {
    black_box(ALPHABET)
        .iter()
        .position(|&a| a == c)
        .and_then(|i| u8::try_from(i).ok())
}

/// What `dcroxide_base58::decode(input)` returns when that is exactly 82
/// bytes, and `None` otherwise (an invalid character, which makes the
/// general decoder return nothing, or any other decoded length). Inputs
/// longer than [`MAX_INPUT`] give `None`; the caller rejects them first.
///
/// Kept out of line (one call a string), so that `ct_check`'s disassembly
/// check finds its big-number arithmetic, which must not divide.
#[inline(never)]
pub(crate) fn decode_extended_key(input: &[u8]) -> Option<Zeroizing<[u8; DECODED_DATA_LEN]>> {
    if input.len() > MAX_INPUT {
        return None;
    }
    // Every character's digit, before any arithmetic: an invalid character
    // anywhere makes the general decoder return nothing.
    let mut digits = Zeroizing::new([0u8; MAX_INPUT]);
    if !map_digits(input, &mut digits[..]) {
        return None;
    }
    let digits = &digits[..input.len()];
    // Leading '1' characters (digit 0) are leading zero bytes.
    let nlz = digits.iter().take_while(|&&d| d == 0).count();

    // The value of the remaining digits, as little-endian 32-bit limbs.
    let mut limbs = Zeroizing::new([0u32; LIMBS]);
    let mut used = 0usize;
    for &d in &digits[nlz..] {
        let mut val = u64::from(d);
        for limb in limbs[..used].iter_mut() {
            // val < 2^32 on entry and limb * 58 < 58 * 2^32, so the sum
            // stays below 59 * 2^32: no wrap.
            val = val.wrapping_add(u64::from(*limb).wrapping_mul(58));
            *limb = val as u32;
            val >>= 32;
        }
        if val > 0 {
            // More limbs than fit means a value of more than 82 bytes.
            *limbs.get_mut(used)? = val as u32;
            used = used.wrapping_add(1);
        }
    }

    // The value's minimal big-endian length: the general decoder trims to
    // the most significant nonzero byte of the top limb.
    let value_len = match used.checked_sub(1) {
        None => 0,
        Some(top) => {
            let top_zero_bytes = usize::try_from(limbs[top].leading_zeros() / 8).ok()?;
            used.checked_mul(4)?.checked_sub(top_zero_bytes)?
        }
    };
    if nlz.checked_add(value_len)? != DECODED_DATA_LEN {
        return None;
    }

    let mut out = Zeroizing::new([0u8; DECODED_DATA_LEN]);
    // The value's bytes, least significant first, into the tail of `out`.
    let value_bytes = limbs[..used].iter().flat_map(|limb| limb.to_le_bytes());
    for (slot, byte) in out.iter_mut().rev().zip(value_bytes.take(value_len)) {
        *slot = byte;
    }
    Some(out)
}

/// 18,079 = ceil(2^20 / 58): `(val * DIV58_MAGIC) >> 20` is `val / 58` for
/// every `val` below [`DIV58_BOUND`]. `val * DIV58_MAGIC / 2^20` exceeds
/// `val / 58` by `val * 6 / (58 * 2^20)`, less than `1 / 58` while `val`
/// is below 2^20 / 6, which cannot carry the quotient past the next
/// integer; and the product stays below 2^32.
const DIV58_MAGIC: u32 = 18_079;

/// The bound below which [`divmod58`] is exact, 2^17; the first value at
/// which it is not is 174,811. `divmod58_matches_division` checks every
/// value below it.
const DIV58_BOUND: u32 = 1 << 17;

// The encoder's largest dividend, a carry of at most 255 plus a digit of
// at most 57 shifted up a byte, is in range.
const _: () = assert!(255 + 57 * 256 < DIV58_BOUND, "divmod58's range");

/// `(val / 58, val % 58)` for `val` below [`DIV58_BOUND`], by a
/// multiplication by a constant, a shift, a multiplication and a
/// subtraction, with no division instruction. Written out, not left to the
/// optimiser: at opt-level "z" rustc compiled this encoder's former
/// `val / 58` and `val % 58` to a hardware divide (`div`) on x86_64, whose
/// latency depends on its operands on several x86-64 and AArch64 cores,
/// and whose dividend here depends on every payload byte before it.
#[inline(always)]
fn divmod58(val: u32) -> (u32, u8) {
    let q = val.wrapping_mul(DIV58_MAGIC) >> 20;
    // q * 58 <= val: the remainder is below 58.
    (q, val.wrapping_sub(q.wrapping_mul(58)) as u8)
}

/// Writes the base-58 digits of the big-endian value of `input`, least
/// significant first, to the start of `digits` (which must have room for
/// them) and returns how many it wrote: none for a value of 0, and none
/// for `input`'s leading zero bytes, which are the caller's. The encoder's
/// big-number arithmetic, with every division by 58 through [`divmod58`].
/// Kept out of line (one call a string), so that `ct_check`'s disassembly
/// check finds it.
#[inline(never)]
pub(crate) fn value_digits(input: &[u8], digits: &mut [u8]) -> usize {
    let mut index = 0usize;
    for &r in input {
        let mut val = u32::from(r);
        for d in digits[..index].iter_mut() {
            // *d < 58 and val <= 255 on entry, so val <= 255 + 57 * 256,
            // in divmod58's range, and the quotient is at most 255.
            let (q, rem) = divmod58(val.wrapping_add(u32::from(*d) << 8));
            *d = rem;
            val = q;
        }
        while val > 0 {
            // The caller's buffer holds every digit, so the store is in
            // bounds (and bounds-checked).
            let (q, rem) = divmod58(val);
            digits[index] = rem;
            index = index.wrapping_add(1);
            val = q;
        }
    }
    index
}

/// `dcroxide_base58::encode(input)` for an 82-byte input. The digits are
/// built in a wiped buffer and written into a string allocated once at its
/// final length, which is returned wrapped in [`Zeroizing`].
pub(crate) fn encode_extended_key(input: &[u8; DECODED_DATA_LEN]) -> Zeroizing<String> {
    let mut digits = Zeroizing::new([0u8; MAX_DIGITS]);
    // 82 bytes encode to at most MAX_DIGITS digits.
    let mut index = value_digits(input, &mut digits[..]);
    // Leading zero bytes are leading '1's (digit 0).
    for &r in input {
        if r != 0 {
            break;
        }
        digits[index] = 0;
        index = index.wrapping_add(1);
    }

    // Each digit's character by `char_of` (through `map_chars`), not by
    // indexing the alphabet with the digit, in place in the wiped buffer.
    map_chars(&mut digits[..index]);
    let mut out = Zeroizing::new(String::with_capacity(index));
    for &c in digits[..index].iter().rev() {
        out.push(char::from(c));
    }
    out
}

#[cfg(test)]
mod tests {
    //! Equivalence with `dcroxide_base58` (decred/base58's outputs): the
    //! decoder must accept exactly the strings whose general decoding is
    //! 82 bytes, with the same bytes, and the encoder must give the same
    //! string.

    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: index arithmetic on small, bounded values"
    )]

    use super::*;
    use crate::{ExtendedKey, HdVersions};
    use alloc::format;
    use alloc::vec::Vec;
    use purse_testutil::SplitMix64;

    fn reference(s: &str) -> Option<[u8; DECODED_DATA_LEN]> {
        dcroxide_base58::decode(s).as_slice().try_into().ok()
    }

    /// Checks one string; returns whether it decoded.
    fn check(s: &str) -> bool {
        let got = decode_extended_key(s.as_bytes()).map(|z| *z);
        if s.len() > MAX_INPUT {
            assert_eq!(got, None, "{s:?}");
            return false;
        }
        assert_eq!(got, reference(s), "{s:?}");
        got.is_some()
    }

    fn valid_strings(rng: &mut SplitMix64) -> Vec<String> {
        let nets = [
            HdVersions::MAINNET,
            HdVersions::TESTNET3,
            HdVersions::SIMNET,
            HdVersions::REGNET,
        ];
        let mut out = Vec::new();
        for net in nets {
            for _ in 0..8 {
                let m = ExtendedKey::new_master(&rng.bytes(32), &net).expect("master");
                let c = m.child(0x8000_002c).expect("child");
                for k in [&m, &c] {
                    out.push(String::from(k.string().as_str()));
                    out.push(String::from(k.neuter().string().as_str()));
                }
            }
        }
        // Payloads with leading zero bytes (leading '1's), all 82 lengths.
        for zeros in 0..=DECODED_DATA_LEN {
            let mut p = [0u8; DECODED_DATA_LEN];
            rng.fill(&mut p[zeros..]);
            if zeros < DECODED_DATA_LEN && p[zeros] == 0 {
                p[zeros] = 1;
            }
            out.push(dcroxide_base58::encode(&p));
        }
        out
    }

    /// The constant-time lookups against the linear search they replaced,
    /// over every byte value: the same digit for each of the 58 alphabet
    /// characters, "none" (0) for the other 198 bytes, and the alphabet's
    /// character for each digit below 58 (0 above, which no caller passes),
    /// one value at a time and through the slice maps the codec calls.
    #[test]
    fn lookups_match_the_search_for_every_byte() {
        let every_byte: Vec<u8> = (0..=u8::MAX).collect();
        let mut digits = [0xaau8; 256];
        assert!(
            !map_digits(&every_byte, &mut digits),
            "198 bytes are invalid"
        );
        let mut found = 0usize;
        for b in 0..=u8::MAX {
            let search = digit_by_search(b);
            assert_eq!(
                digit_plus_one(b).checked_sub(1),
                search,
                "character {b:#04x}"
            );
            found += usize::from(search.is_some());
            let mut digit = [0xaa];
            assert_eq!(map_digits(&[b], &mut digit), search.is_some(), "{b:#04x}");
            if let Some(d) = search {
                assert_eq!(digit[0], d, "{b:#04x}");
                assert_eq!(digits[usize::from(b)], d, "{b:#04x} in a string");
            }
            let expected = ALPHABET.get(usize::from(b)).copied().unwrap_or(0);
            assert_eq!(char_of(b), expected, "digit {b}");
        }
        assert_eq!(found, 58);
        let mut chars: Vec<u8> = (0..58).collect();
        map_chars(&mut chars);
        assert_eq!(chars, ALPHABET);
        assert!(map_digits(ALPHABET, &mut digits));
        assert!(digits[..58].iter().copied().eq(0..58));
        // One invalid character anywhere in a string fails the whole map,
        // and the digits before and after it are still written.
        let mut digits = [0u8; 5];
        assert!(!map_digits(b"2z0z2", &mut digits));
        assert_eq!([digits[0], digits[1], digits[3], digits[4]], [1, 57, 57, 1]);
        assert!(map_digits(b"2z1z2", &mut digits));
        assert_eq!(digits, [1, 57, 0, 57, 1]);
    }

    /// The encoder's division by 58 against the division it replaced, over
    /// every value of its range, and the first value past the range, where
    /// the multiplication's error shows.
    #[test]
    fn divmod58_matches_division() {
        for val in 0..DIV58_BOUND {
            let (q, r) = divmod58(val);
            assert_eq!((q, u32::from(r)), (val / 58, val % 58), "{val}");
        }
        assert_ne!(divmod58(174_811).0, 174_811 / 58);
        assert!((DIV58_BOUND..174_811).all(|val| divmod58(val).0 == val / 58));
    }

    #[test]
    fn decoder_matches_general_decoder() {
        let mut rng = SplitMix64::from_env("hdkeychain-base58-decode");
        let valid = valid_strings(&mut rng);
        let mut checked = 0usize;
        let mut decoded = 0usize;
        let mut run = |s: &str| {
            checked += 1;
            decoded += usize::from(check(s));
        };
        for s in &valid {
            run(s);
        }

        // Lengths and repeated characters around the limits.
        for n in 0..=MAX_INPUT + 2 {
            for c in ["1", "2", "z", "Z"] {
                run(&c.repeat(n));
            }
            run(&format!("{}{}", "1".repeat(n / 2), "z".repeat(n - n / 2)));
        }

        // Mutations of valid strings.
        let invalid = ['0', 'O', 'I', 'l', ' ', '+', '/', '\u{e9}', '\u{20ac}'];
        let alphabet: &[u8] = ALPHABET;
        for round in 0..100_000usize {
            let s = valid[round % valid.len()].as_bytes();
            let mut m: Vec<u8> = s.to_vec();
            match rng.below(7) {
                0 => {
                    let i = rng.below(m.len() as u64) as usize;
                    m[i] = alphabet[rng.below(58) as usize];
                }
                1 => {
                    let i = rng.below(m.len() as u64) as usize;
                    m.remove(i);
                }
                2 => {
                    let i = rng.below(m.len() as u64 + 1) as usize;
                    m.insert(i, alphabet[rng.below(58) as usize]);
                }
                3 => {
                    let n = rng.below(m.len() as u64 + 1) as usize;
                    m.truncate(n);
                }
                4 => {
                    for _ in 0..rng.below(4) + 1 {
                        m.insert(0, b'1');
                    }
                }
                5 => {
                    let n = rng.below(MAX_INPUT as u64 + 1) as usize;
                    m = (0..n).map(|_| alphabet[rng.below(58) as usize]).collect();
                }
                _ => {
                    let i = rng.below(m.len() as u64) as usize;
                    let c = invalid[rng.below(invalid.len() as u64) as usize];
                    let mut st = String::from_utf8(m).expect("ascii");
                    st.replace_range(i..i + 1, c.encode_utf8(&mut [0; 4]));
                    m = st.into_bytes();
                }
            }
            run(core::str::from_utf8(&m).expect("utf-8"));
        }
        assert!(checked > 100_000, "{checked}");
        // Both outcomes are exercised in quantity.
        assert!(
            decoded > 1_000 && checked - decoded > 10_000,
            "{decoded}/{checked}"
        );
    }

    #[test]
    fn encoder_matches_general_encoder() {
        let mut rng = SplitMix64::from_env("hdkeychain-base58-encode");
        for round in 0..100_000usize {
            let mut p = [0u8; DECODED_DATA_LEN];
            let zeros = if round % 4 == 0 {
                rng.below(DECODED_DATA_LEN as u64 + 1) as usize
            } else {
                0
            };
            rng.fill(&mut p[zeros..]);
            let got = encode_extended_key(&p);
            assert_eq!(got.as_str(), dcroxide_base58::encode(&p), "{p:?}");
            assert_eq!(got.capacity(), got.len(), "allocated once, at its length");
        }
    }
}
