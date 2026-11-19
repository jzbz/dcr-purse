// SPDX-License-Identifier: ISC
//! Seed input fuzz target: the decoders behind gRPC `SeedService.DecodeSeed`
//! and the `--create` prompt.
//!
//! Arbitrary text must never panic `decode_user_input`, `decode_mnemonics`
//! or the Go-exact hex decoder.  Anything `decode_user_input` accepts is a
//! seed of legal length, and encoding it as a mnemonic and decoding that
//! gives the same bytes back.  The first byte picks the mode: raw text, or a
//! seed built from the remaining bytes and sent through the round trip, which
//! mutation alone would almost never reach past the checksum word.

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_seed::pgpwordlist::decode_mnemonics;
use purse_seed::walletseed::{
    MAX_SEED_BYTES, MIN_SEED_BYTES, decode_user_input, encode_mnemonic, encode_mnemonic_slice,
    go_hex_decode,
};

fn round_trip(seed: &[u8]) {
    let mnemonic = encode_mnemonic(seed);
    let words = encode_mnemonic_slice(seed);
    if seed.is_empty() {
        // Go's EncodeMnemonic puts a space before the checksum word even
        // when no word precedes it; the slice form has no such space.
        assert_eq!(mnemonic.as_str(), format!(" {}", words.join().as_str()));
    } else {
        assert_eq!(words.join().as_str(), mnemonic.as_str());
    }
    let legal = (MIN_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len());
    match decode_user_input(mnemonic.as_str()) {
        Ok(decoded) => {
            assert!(legal, "accepted a seed of {} bytes", seed.len());
            assert_eq!(decoded.as_bytes(), seed);
        }
        Err(_) => assert!(!legal, "refused its own mnemonic for {} bytes", seed.len()),
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        round_trip(rest);
        return;
    }
    let _ = go_hex_decode(rest);
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    if let Ok(seed) = decode_user_input(text) {
        assert!((MIN_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len()));
        round_trip(seed.as_bytes());
    }
    let words: Vec<&str> = text.split(' ').collect();
    if let Ok(bytes) = decode_mnemonics(&words) {
        // Every accepted word maps to one byte.
        assert!(bytes.len() <= words.len());
    }
});
