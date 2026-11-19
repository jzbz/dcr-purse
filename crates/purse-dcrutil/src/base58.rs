// Copyright (c) 2013-2015 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Base58 (decred/base58 v1.0.6 `Encode`/`Decode`, the algorithm of
//! `dcroxide-base58`) over buffers that are wiped, so neither direction
//! leaves a WIF's private key bytes or digits in freed memory. The decoder
//! is specialised to the 39 bytes of a WIF; the encoder takes any length,
//! since Go's `WIF.String` encodes a private key of whatever length
//! `NewWIF` was given. Equivalence with `dcroxide-base58` is tested over
//! random and mutated strings and payloads.
//!
//! The algorithm is purse-hdkeychain's codec (its `src/base58.rs`) with the
//! lengths changed.

use zeroize::Zeroizing;

/// The Bitcoin base58 alphabet.
const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// The decoded length of a WIF (`decodedDataLen`, `wif.go:113`).
pub(crate) const WIF_LEN: usize = 39;

/// The longest WIF string `DecodeWIF` decodes (`maxWIFLen`, `wif.go:114`).
pub(crate) const MAX_WIF_STRING_LEN: usize = 54;

/// 54 base58 digits are below 58^54 < 2^317, so 10 32-bit limbs hold any
/// accepted input's value; 12 leave room.
const LIMBS: usize = 12;

fn digit_of(c: u8) -> Option<u8> {
    ALPHABET
        .iter()
        .position(|&a| a == c)
        .and_then(|i| u8::try_from(i).ok())
}

/// What `dcroxide_base58::decode(input)` returns when that is exactly
/// [`WIF_LEN`] bytes, and `None` otherwise (an invalid character, which
/// makes the general decoder return nothing, or any other decoded length).
/// Inputs longer than [`MAX_WIF_STRING_LEN`] give `None`; the caller rejects
/// them first, as Go does.
pub(crate) fn decode_wif(input: &[u8]) -> Option<Zeroizing<[u8; WIF_LEN]>> {
    if input.len() > MAX_WIF_STRING_LEN {
        return None;
    }
    // Leading '1' characters are leading zero bytes.
    let nlz = input.iter().take_while(|&&c| c == ALPHABET[0]).count();

    // The value of the remaining digits, as little-endian 32-bit limbs.
    let mut limbs = Zeroizing::new([0u32; LIMBS]);
    let mut used = 0usize;
    for &c in input.get(nlz..)? {
        let mut val = u64::from(digit_of(c)?);
        for limb in limbs.get_mut(..used)?.iter_mut() {
            // val < 2^32 on entry and limb * 58 < 58 * 2^32, so the sum
            // stays below 59 * 2^32: no wrap.
            val = val.wrapping_add(u64::from(*limb).wrapping_mul(58));
            *limb = val as u32;
            val >>= 32;
        }
        if val > 0 {
            *limbs.get_mut(used)? = val as u32;
            used = used.wrapping_add(1);
        }
    }

    // The value's minimal big-endian length: the general decoder trims to
    // the most significant nonzero byte of the top limb.
    let value_len = match used.checked_sub(1) {
        None => 0,
        Some(top) => {
            let top_zero_bytes = usize::try_from(limbs.get(top)?.leading_zeros() / 8).ok()?;
            used.checked_mul(4)?.checked_sub(top_zero_bytes)?
        }
    };
    if nlz.checked_add(value_len)? != WIF_LEN {
        return None;
    }

    let mut out = Zeroizing::new([0u8; WIF_LEN]);
    // The value's bytes, least significant first, into the tail of `out`.
    let value_bytes = limbs
        .get(..used)?
        .iter()
        .flat_map(|limb| limb.to_le_bytes());
    for (slot, byte) in out.iter_mut().rev().zip(value_bytes.take(value_len)) {
        *slot = byte;
    }
    Some(out)
}

/// `dcroxide_base58::encode(input)`. The digits are built in a wiped
/// buffer allocated once at the most digits `input` can need, and written
/// into a string allocated once at its final length, which is returned
/// wrapped in [`Zeroizing`].
pub(crate) fn encode(input: &[u8]) -> Zeroizing<String> {
    // n bytes need at most ceil(n * log58(256)) digits for the value, one
    // per leading zero byte, and log58(256) < 1.37: n * 137 / 100 + 1.
    let max_digits = input
        .len()
        .saturating_mul(137)
        .checked_div(100)
        .unwrap_or(0)
        .saturating_add(1);
    let mut digits: Zeroizing<Vec<u8>> = Zeroizing::new(vec![0u8; max_digits]);
    let mut index = 0usize;
    for &r in input {
        let mut val = u32::from(r);
        for d in digits.iter_mut().take(index) {
            // *d < 58 and val <= 255 + 57 * 256 on entry: no wrap.
            val = val.wrapping_add(u32::from(*d) << 8);
            *d = val.wrapping_rem(58) as u8;
            val = val.wrapping_div(58);
        }
        while val > 0 {
            // The bound above makes the store in bounds (and it is
            // bounds-checked).
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
    for &d in digits.iter().take(index).rev() {
        out.push(char::from(ALPHABET[usize::from(d)]));
    }
    out
}

#[cfg(test)]
mod tests {
    //! Equivalence with `dcroxide_base58` (decred/base58's algorithm).

    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: index arithmetic on small, bounded values"
    )]

    use super::*;
    use purse_testutil::SplitMix64;

    fn reference(s: &str) -> Option<[u8; WIF_LEN]> {
        dcroxide_base58::decode(s).as_slice().try_into().ok()
    }

    /// Checks one string; returns whether it decoded.
    fn check(s: &str) -> bool {
        let got = decode_wif(s.as_bytes()).map(|z| *z);
        if s.len() > MAX_WIF_STRING_LEN {
            assert_eq!(got, None, "{s:?}");
            return false;
        }
        assert_eq!(got, reference(s), "{s:?}");
        got.is_some()
    }

    #[test]
    fn decoder_matches_general_decoder() {
        let mut rng = SplitMix64::from_env("dcrutil-base58-decode");
        let mut valid = Vec::new();
        for zeros in 0..=WIF_LEN {
            let mut p = [0u8; WIF_LEN];
            rng.fill(&mut p[zeros..]);
            if zeros < WIF_LEN && p[zeros] == 0 {
                p[zeros] = 1;
            }
            valid.push(dcroxide_base58::encode(&p));
        }
        for _ in 0..200 {
            valid.push(dcroxide_base58::encode(&rng.bytes(WIF_LEN)));
        }
        let mut checked = 0usize;
        let mut decoded = 0usize;
        let mut run = |s: &str| {
            checked += 1;
            decoded += usize::from(check(s));
        };
        for s in &valid {
            run(s);
        }
        for n in 0..=MAX_WIF_STRING_LEN + 2 {
            for c in ["1", "2", "z", "Z"] {
                run(&c.repeat(n));
            }
            run(&format!("{}{}", "1".repeat(n / 2), "z".repeat(n - n / 2)));
        }
        let alphabet: &[u8] = ALPHABET;
        for round in 0..100_000usize {
            let mut m: Vec<u8> = valid[round % valid.len()].as_bytes().to_vec();
            match rng.below(6) {
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
                    for _ in 0..rng.below(4) + 1 {
                        m.insert(0, b'1');
                    }
                }
                4 => {
                    let n = rng.below(MAX_WIF_STRING_LEN as u64 + 3) as usize;
                    m = (0..n).map(|_| alphabet[rng.below(58) as usize]).collect();
                }
                _ => {
                    let i = rng.below(m.len() as u64) as usize;
                    m[i] = b"0OIl+/ "[rng.below(7) as usize];
                }
            }
            run(std::str::from_utf8(&m).expect("ascii"));
        }
        assert!(checked > 100_000, "{checked}");
        assert!(
            decoded > 1_000 && checked - decoded > 10_000,
            "{decoded}/{checked}"
        );
    }

    #[test]
    fn encoder_matches_general_encoder() {
        let mut rng = SplitMix64::from_env("dcrutil-base58-encode");
        for round in 0..50_000usize {
            let len = rng.below(80) as usize;
            let mut p = vec![0u8; len];
            let zeros = if round % 4 == 0 {
                rng.below(len as u64 + 1) as usize
            } else {
                0
            };
            rng.fill(&mut p[zeros..]);
            let got = encode(&p);
            assert_eq!(got.as_str(), dcroxide_base58::encode(&p), "{p:?}");
            assert_eq!(got.capacity(), got.len(), "allocated once, at its length");
        }
    }
}
