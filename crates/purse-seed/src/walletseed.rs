// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `walletseed` package: seed generation, the seed
//! mnemonic with its checksum word, and decoding of user-entered seeds.

use std::fmt;

use purse_errors::{AnyError, Error, Kind, Op, e};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{ERR_INVALID_SEED_LEN, HexError};
use crate::pgpwordlist::{byte_to_mnemonic, decode_words, go_trim_space, word_index};
use crate::secret::{MnemonicWords, SecretBytes, SecretString};

/// The operation of [`generate_random_seed`]'s errors (Go's `const op` in
/// `GenerateRandomSeed`, `walletseed/seed.go:22`).
pub const OP_GENERATE_RANDOM_SEED: Op = Op::new("walletseed.GenerateRandomSeed");

/// The operation of [`decode_user_input`]'s errors (Go's `const op` in
/// `DecodeUserInput`, `walletseed/seed.go:72`).
pub const OP_DECODE_USER_INPUT: Op = Op::new("walletseed.DecodeUserInput");

// The three seed lengths are dcrd hdkeychain's, repeated here (as `usize`)
// rather than taken from purse-hdkeychain, whose curve arithmetic this
// crate does not need; a test checks that they are equal.

/// The shortest seed accepted, in bytes (dcrd `hdkeychain.MinSeedBytes`,
/// 128 bits).
pub const MIN_SEED_BYTES: usize = 16;

/// The longest seed accepted, in bytes (dcrd `hdkeychain.MaxSeedBytes`, 512
/// bits).
pub const MAX_SEED_BYTES: usize = 64;

/// The recommended seed length, in bytes (dcrd
/// `hdkeychain.RecommendedSeedLen`, 256 bits).
pub const RECOMMENDED_SEED_LEN: usize = 32;

/// A failure of a [`RandomSource`]; the underlying error of the [`Error`]
/// that [`generate_random_seed_with`] then returns, held as a
/// [`purse_errors::Foreign`] leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomSourceError(String);

impl RandomSourceError {
    /// An error with the given message, which becomes the leaf text of the
    /// [`Error`] that [`generate_random_seed_with`] returns.
    pub fn new(message: impl Into<String>) -> Self {
        RandomSourceError(message.into())
    }
}

impl fmt::Display for RandomSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RandomSourceError {}

impl From<getrandom::Error> for RandomSourceError {
    fn from(e: getrandom::Error) -> Self {
        RandomSourceError(e.to_string())
    }
}

/// A source of random bytes for seed generation.
///
/// Implemented by [`OsRandom`] and by any
/// `FnMut(&mut [u8]) -> Result<(), RandomSourceError>` closure, so tests can
/// inject a deterministic source.
pub trait RandomSource {
    /// Fills all of `buf` with random bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot supply the bytes; `buf` may
    /// then be partly written.
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError>;
}

impl<F> RandomSource for F
where
    F: FnMut(&mut [u8]) -> Result<(), RandomSourceError>,
{
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        self(buf)
    }
}

/// The operating system's cryptographically secure random source, through
/// the `getrandom` crate.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        getrandom::fill(buf).map_err(RandomSourceError::from)
    }
}

/// Returns a new seed of `size` bytes from the operating system's random
/// source (Go's `GenerateRandomSeed`, `walletseed/seed.go:21-34`).
///
/// # Errors
///
/// As [`generate_random_seed_with`].
pub fn generate_random_seed(size: usize) -> Result<SecretBytes, Error> {
    generate_random_seed_with(size, &mut OsRandom)
}

/// Returns a new seed of `size` bytes read from `rng` (Go's
/// `GenerateRandomSeed` with the random source made injectable).
///
/// # Errors
///
/// Every error has op [`OP_GENERATE_RANDOM_SEED`]. A `size` of 255 or
/// more, or outside [`MIN_SEED_BYTES`]`..=`[`MAX_SEED_BYTES`], gives Go's
/// `errors.E(op, errors.Invalid, hdkeychain.ErrInvalidSeedLen)`: kind
/// [`Kind::Invalid`] wrapping [`ERR_INVALID_SEED_LEN`], which renders as
/// `walletseed.GenerateRandomSeed: invalid operation: seed length must be
/// between 128 and 512 bits`. A failing source gives Go's `errors.E(op,
/// err)`: kind [`Kind::Other`] with the [`RandomSourceError`] as a foreign
/// leaf (in Go the standard library's reader does not fail).
pub fn generate_random_seed_with<R: RandomSource + ?Sized>(
    size: usize,
    rng: &mut R,
) -> Result<SecretBytes, Error> {
    let invalid_len = || {
        e!(
            OP_GENERATE_RANDOM_SEED,
            Kind::Invalid,
            &*ERR_INVALID_SEED_LEN
        )
    };
    // Go checks `size >= 255` (its uint8 narrowing guard) before the range
    // check; both give the same error.
    if size >= usize::from(u8::MAX) {
        return Err(invalid_len());
    }
    if !(MIN_SEED_BYTES..=MAX_SEED_BYTES).contains(&size) {
        return Err(invalid_len());
    }
    let mut seed = SecretBytes::zeroed(size);
    rng.fill(seed.as_mut_bytes())
        .map_err(|err| e!(OP_GENERATE_RANDOM_SEED, AnyError::foreign(err)))?;
    Ok(seed)
}

/// The checksum byte at the end of the seed mnemonic: the first byte of the
/// double SHA-256 of the seed (`walletseed/seed.go:38-41`).
///
/// Both digests are written into zero-on-drop buffers, and sha2's `zeroize`
/// feature wipes the hasher states.
fn checksum_byte(data: &[u8]) -> u8 {
    let mut intermediate = Zeroizing::new([0u8; 32]);
    Sha256::new_with_prefix(data).finalize_into((&mut *intermediate).into());
    let mut outer = Zeroizing::new([0u8; 32]);
    Sha256::new_with_prefix(&intermediate[..]).finalize_into((&mut *outer).into());
    outer[0]
}

/// Encodes a seed as a mnemonic word list (Go's `EncodeMnemonicSlice`,
/// `walletseed/seed.go:44-52`): one word per byte, then the checksum word at
/// position `seed.len()`. The length is not validated.
pub fn encode_mnemonic_slice(seed: &[u8]) -> MnemonicWords {
    let mut words = MnemonicWords::with_capacity(seed.len().saturating_add(1));
    for (i, &b) in seed.iter().enumerate() {
        words.push_index(word_index(b, i));
    }
    words.push_index(word_index(checksum_byte(seed), seed.len()));
    words
}

/// Encodes a seed as a mnemonic separated by single spaces (Go's
/// `EncodeMnemonic`, `walletseed/seed.go:55-67`).
///
/// The space before the checksum word is written unconditionally, so for
/// an empty seed the result is a space followed by the checksum word, unlike
/// [`encode_mnemonic_slice`] joined. The length is not validated.
pub fn encode_mnemonic(seed: &[u8]) -> SecretString {
    let checksum_word = byte_to_mnemonic(checksum_byte(seed), seed.len());
    // Exact size, so the zero-on-drop buffer never reallocates: every word,
    // a space between seed words and one before the checksum word
    // (`max(n - 1, 0) + 1` spaces).
    let capacity = seed
        .iter()
        .enumerate()
        .map(|(i, &b)| byte_to_mnemonic(b, i).len())
        .fold(checksum_word.len(), usize::saturating_add)
        .saturating_add(seed.len().max(1));
    let mut out = Zeroizing::new(String::with_capacity(capacity));
    for (i, &b) in seed.iter().enumerate() {
        if i != 0 {
            out.push(' ');
        }
        out.push_str(byte_to_mnemonic(b, i));
    }
    out.push(' ');
    out.push_str(checksum_word);
    debug_assert_eq!(out.len(), capacity);
    SecretString::new(out)
}

/// Decodes a seed entered as hex or as a mnemonic (Go's `DecodeUserInput`,
/// `walletseed/seed.go:71-102`).
///
/// The input is trimmed of white space (Go's `strings.TrimSpace`) and split
/// on the ASCII space only. One token is decoded as hex (either case, even
/// length, no `0x` prefix); more tokens are decoded as a mnemonic whose last
/// word is the checksum (runs of spaces are tolerated, but tabs and
/// newlines between words are not separators). A single mnemonic word is
/// therefore read as hex. The seed must be
/// [`MIN_SEED_BYTES`]`..=`[`MAX_SEED_BYTES`] long.
///
/// # Errors
///
/// Every error is Go's `errors.E(op, errors.Encoding, err)`: op
/// [`OP_DECODE_USER_INPUT`], kind [`Kind::Encoding`], and as `err`:
///
/// - for hex input, the [`HexError`] of [`go_hex_decode`] as a foreign leaf
///   (an invalid byte is reported in preference to odd length);
/// - for a bad word, the [`crate::pgpwordlist::decode_mnemonics`] error, a
///   nested level (`E` neither collapses it nor changes its fields, since
///   its op differs), whose leaf quotes the word;
/// - the text `checksum mismatch` (a string argument, as in Go);
/// - [`ERR_INVALID_SEED_LEN`] for a decoded seed of the wrong length
///   (including empty input).
///
/// The error may quote a seed word. Render it with
/// [`SeedErrorExt::render_zeroizing`](crate::SeedErrorExt::render_zeroizing),
/// not `render(sep)`, whose `String` is no longer wiped; and log its
/// `Display` once rather than its `source()` chain, which repeats the word
/// at each of its three links (see "What is wiped" in [`crate::error`]).
pub fn decode_user_input(input: &str) -> Result<SecretBytes, Error> {
    let trimmed = go_trim_space(input);
    let word_count = trimmed.split(' ').count();
    let seed = if word_count == 1 {
        go_hex_decode(trimmed.as_bytes())
            .map_err(|err| e!(OP_DECODE_USER_INPUT, Kind::Encoding, AnyError::foreign(err)))?
    } else {
        let decoded = decode_words(trimmed.split(' '), word_count)
            .map_err(|err| e!(OP_DECODE_USER_INPUT, Kind::Encoding, err))?;
        match decoded.as_bytes().split_last() {
            // Need data and a checksum byte to check the checksum. Fewer
            // than two decoded words cannot follow from two or more
            // trimmed tokens, but Go handles it, by failing the length
            // check below.
            Some((&checksum, data)) if !data.is_empty() => {
                if checksum_byte(data) != checksum {
                    return Err(e!(
                        OP_DECODE_USER_INPUT,
                        Kind::Encoding,
                        "checksum mismatch"
                    ));
                }
                SecretBytes::from_slice(data)
            }
            _ => SecretBytes::default(),
        }
    };
    if !(MIN_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len()) {
        return Err(e!(
            OP_DECODE_USER_INPUT,
            Kind::Encoding,
            &*ERR_INVALID_SEED_LEN
        ));
    }
    Ok(seed)
}

/// Go's `hex.DecodeString` (and `hex.Decode`), exactly: digits of either
/// case, pairs checked left to right with the first byte of a pair before
/// the second, and for an odd length an invalid last byte reported instead
/// of the length. Empty input decodes to an empty result.
///
/// [`decode_user_input`] uses it for one-token input. It is public for
/// callers that, like dcrwallet's interactive `--create` prompt
/// (`internal/prompt/prompt.go:330-353`), call `hex.DecodeString` directly
/// and print the bare error.
///
/// # Errors
///
/// [`HexError::InvalidByte`] or [`HexError::OddLength`], whose `Display` is
/// Go's error text (`encoding/hex: invalid byte: U+0067 'g'`,
/// `encoding/hex: odd length hex string`), with no operation or kind, as
/// the prompt prints it.
pub fn go_hex_decode(src: &[u8]) -> Result<SecretBytes, HexError> {
    let mut out = SecretBytes::with_capacity(src.len() / 2);
    let (pairs, remainder) = src.as_chunks::<2>();
    for &[p, q] in pairs {
        let hi = hex_value(p).ok_or(HexError::InvalidByte(p))?;
        let lo = hex_value(q).ok_or(HexError::InvalidByte(q))?;
        out.push(hi.wrapping_shl(4) | lo);
    }
    if let &[last] = remainder {
        if hex_value(last).is_none() {
            return Err(HexError::InvalidByte(last));
        }
        return Err(HexError::OddLength);
    }
    Ok(out)
}

/// The value of an ASCII hex digit of either case.
fn hex_value(c: u8) -> Option<u8> {
    char::from(c)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

#[cfg(test)]
mod tests {
    use purse_errors::DAEMON_SEPARATOR;

    use super::*;
    use crate::SeedErrorExt;

    #[test]
    fn hex_error_precedence() {
        let err = |s: &str| decode_user_input(s).unwrap_err().render(DAEMON_SEPARATOR);
        let prefix = "walletseed.DecodeUserInput: invalid encoding: ";
        assert_eq!(
            err("abc"),
            format!("{prefix}encoding/hex: odd length hex string")
        );
        assert_eq!(
            err("abg"),
            format!("{prefix}encoding/hex: invalid byte: U+0067 'g'")
        );
        assert_eq!(
            err("agb"),
            format!("{prefix}encoding/hex: invalid byte: U+0067 'g'")
        );
        assert_eq!(
            err("0x00"),
            format!("{prefix}encoding/hex: invalid byte: U+0078 'x'")
        );
        assert_eq!(
            err("aé"),
            format!("{prefix}encoding/hex: invalid byte: U+00C3 'Ã'")
        );
        assert_eq!(
            err(""),
            format!("{prefix}seed length must be between 128 and 512 bits")
        );
    }

    #[test]
    fn injected_rng_and_bounds() {
        let counter = std::cell::Cell::new(0u8);
        let mut rng = |buf: &mut [u8]| {
            for b in buf {
                *b = counter.get();
                counter.set(counter.get().wrapping_add(1));
            }
            Ok(())
        };
        let seed = generate_random_seed_with(16, &mut rng).unwrap();
        assert_eq!(seed.as_bytes(), (0u8..16).collect::<Vec<_>>());
        assert_eq!(counter.get(), 16);
        for size in [0, 15, 65, 254, 255, 256, usize::MAX] {
            let e = generate_random_seed_with(size, &mut rng).unwrap_err();
            assert_eq!(counter.get(), 16, "size {size} consumed randomness");
            assert_eq!(e.kind, Kind::Invalid);
            assert_eq!(e.op, OP_GENERATE_RANDOM_SEED);
            assert!(e.is_invalid_seed_len());
            assert_eq!(
                e.render(DAEMON_SEPARATOR),
                "walletseed.GenerateRandomSeed: invalid operation: \
                 seed length must be between 128 and 512 bits"
            );
        }
        let mut failing = |_: &mut [u8]| Err(RandomSourceError::new("no entropy"));
        let e = generate_random_seed_with(32, &mut failing).unwrap_err();
        assert_eq!(e.kind, Kind::Other);
        assert_eq!(e.as_kind(), None);
        assert!(!e.is_invalid_seed_len());
        assert_eq!(
            e.find_foreign::<RandomSourceError>(),
            Some(&RandomSourceError::new("no entropy"))
        );
        assert_eq!(
            e.render(DAEMON_SEPARATOR),
            "walletseed.GenerateRandomSeed: no entropy"
        );
        for size in [MIN_SEED_BYTES, RECOMMENDED_SEED_LEN, MAX_SEED_BYTES] {
            assert_eq!(generate_random_seed(size).unwrap().len(), size);
        }
    }

    #[test]
    fn go_hex_decode_matches_go() {
        assert!(go_hex_decode(b"").unwrap().is_empty());
        assert_eq!(go_hex_decode(b"0aFf").unwrap().as_bytes(), [0x0a, 0xff]);
        let err = |s: &[u8]| go_hex_decode(s).unwrap_err().to_string();
        assert_eq!(err(b"abc"), "encoding/hex: odd length hex string");
        assert_eq!(err(b"abcg"), "encoding/hex: invalid byte: U+0067 'g'");
        assert_eq!(err(b"abg"), "encoding/hex: invalid byte: U+0067 'g'");
        assert_eq!(err(b"a\xff"), "encoding/hex: invalid byte: U+00FF '\u{ff}'");
        assert_eq!(err(b"\x80"), "encoding/hex: invalid byte: U+0080");
    }

    #[test]
    fn mnemonic_capacity_is_exact() {
        for len in [0usize, 1, 2, 16, 33, 64] {
            let seed: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let s = encode_mnemonic(&seed);
            assert_eq!(s.capacity(), s.len(), "len {len}");
            let joined = encode_mnemonic_slice(&seed).join();
            assert_eq!(joined.capacity(), joined.len(), "len {len}");
        }
    }

    #[test]
    fn empty_seed_mnemonic_has_leading_space() {
        let s = encode_mnemonic(&[]);
        let w = encode_mnemonic_slice(&[]);
        assert_eq!(w.len(), 1);
        assert_eq!(s.as_str(), format!(" {}", w.get(0).unwrap()));
    }
}
