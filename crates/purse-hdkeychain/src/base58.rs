// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Base58 (decred/base58 v1.0.6 `Encode`/`Decode`, the algorithm of
//! `dcroxide-base58`) specialised to the 82-byte extended key, over fixed
//! buffers that are wiped, so neither direction leaves a private key's
//! bytes or digits in freed memory. Equivalence with `dcroxide-base58` is
//! tested over the parse corpus and random and mutated strings.

use alloc::string::String;

use zeroize::Zeroizing;

use crate::DECODED_DATA_LEN;

/// The Bitcoin base58 alphabet.
const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// The longest input [`decode_extended_key`] accepts, the caller's own
/// limit (`MAX_KEY_LEN`, 113 characters).
pub(crate) const MAX_INPUT: usize = crate::MAX_KEY_LEN;

/// 113 base58 digits are below 58^113 < 2^662, so 21 32-bit limbs hold any
/// accepted input's value; 24 leave room.
const LIMBS: usize = 24;

/// The most digits an 82-byte input encodes to: `82 * 137 / 100 + 1`.
const MAX_DIGITS: usize = DECODED_DATA_LEN * 137 / 100 + 1;

fn digit_of(c: u8) -> Option<u8> {
    ALPHABET
        .iter()
        .position(|&a| a == c)
        .and_then(|i| u8::try_from(i).ok())
}

/// What `dcroxide_base58::decode(input)` returns when that is exactly 82
/// bytes, and `None` otherwise (an invalid character, which makes the
/// general decoder return nothing, or any other decoded length). Inputs
/// longer than [`MAX_INPUT`] give `None`; the caller rejects them first.
pub(crate) fn decode_extended_key(input: &[u8]) -> Option<Zeroizing<[u8; DECODED_DATA_LEN]>> {
    if input.len() > MAX_INPUT {
        return None;
    }
    // Leading '1' characters are leading zero bytes.
    let nlz = input.iter().take_while(|&&c| c == ALPHABET[0]).count();

    // The value of the remaining digits, as little-endian 32-bit limbs.
    let mut limbs = Zeroizing::new([0u32; LIMBS]);
    let mut used = 0usize;
    for &c in &input[nlz..] {
        let mut val = u64::from(digit_of(c)?);
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

/// `dcroxide_base58::encode(input)` for an 82-byte input. The digits are
/// built in a wiped buffer and written into a string allocated once at its
/// final length, which is returned wrapped in [`Zeroizing`].
pub(crate) fn encode_extended_key(input: &[u8; DECODED_DATA_LEN]) -> Zeroizing<String> {
    let mut digits = Zeroizing::new([0u8; MAX_DIGITS]);
    let mut index = 0usize;
    for &r in input {
        let mut val = u32::from(r);
        for d in digits[..index].iter_mut() {
            // *d < 58 and val <= 255 + 57 * 256 on entry: no wrap.
            val = val.wrapping_add(u32::from(*d) << 8);
            *d = val.wrapping_rem(58) as u8;
            val = val.wrapping_div(58);
        }
        while val > 0 {
            // 82 bytes encode to at most MAX_DIGITS digits, so the store is
            // in bounds (and bounds-checked).
            digits[index] = val.wrapping_rem(58) as u8;
            index = index.wrapping_add(1);
            val = val.wrapping_div(58);
        }
    }
    // Leading zero bytes are leading '1's (digit 0).
    for &r in input {
        if r != 0 {
            break;
        }
        digits[index] = 0;
        index = index.wrapping_add(1);
    }

    let mut out = Zeroizing::new(String::with_capacity(index));
    for &d in digits[..index].iter().rev() {
        out.push(char::from(ALPHABET[usize::from(d)]));
    }
    out
}

#[cfg(test)]
mod tests {
    //! Equivalence with `dcroxide_base58` (decred/base58's algorithm): the
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
