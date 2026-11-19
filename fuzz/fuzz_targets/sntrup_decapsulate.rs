// SPDX-License-Identifier: ISC
//! sntrup4591761 fuzz target: the decoders, through decapsulation and
//! encapsulation, on arbitrary keys and ciphertexts, and the round trip
//! with tampering.
//!
//! Public keys and ciphertexts come from other mixing peers; every byte
//! string of the right length must be answered as Go answers it, so nothing
//! may panic. The first byte picks the mode:
//!
//! - 0: arbitrary ciphertext and private key (the input, zero-padded):
//!   `decapsulate` answers, the same way twice; and `encapsulate` to an
//!   arbitrary public key, with randomness from the input, succeeds.
//! - 1: a key pair and a ciphertext from randomness expanded from the
//!   input, which must decapsulate as valid to the encapsulated key; then
//!   one byte of the ciphertext changed by the input. A change in the first
//!   1044 bytes must make it invalid (the four-byte groups decode
//!   injectively, so the decoded ciphertext changes and the re-encryption
//!   or the confirmation fails, except with negligible probability); in the
//!   last three it may land on one of Go's aliases (QK-0740), which must
//!   then give the same key.
//! - 2: key generation and encapsulation from the input itself as a short
//!   stream: an error once it runs out, never a panic.
//! - 3: key generation whose first g candidate is chosen by the input (one
//!   coefficient per byte, mod 3), followed by randomness expanded from
//!   the input. If the candidate is rejected (it has no inverse in R3) the
//!   key pair must be the one the stream without it gives; either way it
//!   must decapsulate what is encapsulated to it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sntrup4591761::{
    CIPHERTEXT_SIZE, Ciphertext, ConstantTimeEq, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE, PrivateKey,
    PublicKey, decapsulate, encapsulate, generate_key,
};

/// Bytes one random polynomial draws.
const POLY: usize = 3044;

fn padded<const N: usize>(data: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = data.len().min(N);
    out[..n].copy_from_slice(&data[..n]);
    out
}

/// An endless stream expanded from `seed` by SplitMix64.
fn expander(seed: &[u8]) -> impl FnMut(&mut [u8]) -> Result<(), ()> {
    let mut state = seed
        .iter()
        .fold(0x9e37_79b9_7f4a_7c15_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
    move |buf: &mut [u8]| {
        for chunk in buf.chunks_mut(8) {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
        }
        Ok(())
    }
}

/// The input itself as a finite stream.
fn finite(data: &[u8]) -> impl FnMut(&mut [u8]) -> Result<(), ()> + '_ {
    let mut pos = 0;
    move |buf: &mut [u8]| {
        if data.len() - pos >= buf.len() {
            buf.copy_from_slice(&data[pos..pos + buf.len()]);
            pos += buf.len();
            Ok(())
        } else {
            pos = data.len();
            Err(())
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    match mode % 4 {
        0 => {
            let ct = Ciphertext::from_bytes(padded::<CIPHERTEXT_SIZE>(rest));
            let sk_bytes = padded::<PRIVATE_KEY_SIZE>(rest.get(CIPHERTEXT_SIZE..).unwrap_or(&[]));
            let sk = PrivateKey::from_bytes(&sk_bytes);
            let (k1, ok1) = decapsulate(&ct, &sk);
            let (k2, ok2) = decapsulate(&ct, &sk);
            assert_eq!(bool::from(ok1), bool::from(ok2));
            assert!(bool::from(k1.ct_eq(&k2)));

            let pk = PublicKey::from_bytes(padded::<PUBLIC_KEY_SIZE>(rest));
            encapsulate(&mut expander(rest), &pk).expect("any public key is accepted");
        }
        1 => {
            let mut rng = expander(rest);
            let (pk, sk) = generate_key(&mut rng).expect("an endless stream");
            let (ct, k) = encapsulate(&mut rng, &pk).expect("an endless stream");
            let (k2, ok) = decapsulate(&ct, &sk);
            assert!(bool::from(ok), "an honest ciphertext was rejected");
            assert!(bool::from(k.ct_eq(&k2)));

            let (at, delta) = match rest {
                [a, b, d, ..] => (usize::from(u16::from_le_bytes([*a, *b])) % CIPHERTEXT_SIZE, *d),
                _ => return,
            };
            if delta == 0 {
                return;
            }
            let mut t = *ct.as_bytes();
            t[at] ^= delta;
            let (k3, ok) = decapsulate(&Ciphertext::from_bytes(t), &sk);
            if at < CIPHERTEXT_SIZE - 3 {
                assert!(!bool::from(ok), "a change at byte {at} was accepted");
            } else if bool::from(ok) {
                assert!(bool::from(k.ct_eq(&k3)), "an alias gave another key");
            }
        }
        3 => {
            // ((w & 0x3fffffff)·3 >> 30) − 1 is −1, 0, 1 for these words.
            const WORDS: [u32; 3] = [0x0000_0000, 0x2000_0000, 0x3000_0000];
            let mut stream = Vec::with_capacity(3 * POLY);
            for i in 0..761 {
                let b = rest.get(i).copied().unwrap_or(1);
                stream.extend_from_slice(&WORDS[usize::from(b % 3)].to_le_bytes());
            }
            let mut tail = vec![0u8; 2 * POLY];
            expander(rest)(&mut tail).expect("endless");
            stream.extend_from_slice(&tail);
            stream.extend_from_slice(&tail[..POLY]);
            let mut src = finite(&stream);
            let (pk, sk) = generate_key(&mut src).expect("enough for a rejected g");
            let mut consumed = finite(&stream);
            // Replay to count what was drawn: two polynomials, or three
            // when the chosen g was rejected.
            let mut drawn = 0usize;
            let mut count = |buf: &mut [u8]| {
                drawn += buf.len();
                consumed(buf)
            };
            generate_key(&mut count).expect("the same stream");
            if drawn == 3 * POLY {
                let (pk2, sk2) = generate_key(&mut finite(&tail)).expect("two polynomials");
                assert_eq!(pk, pk2, "a rejected g changed the key");
                assert!(bool::from(sk.ct_eq(&sk2)));
            } else {
                assert_eq!(drawn, 2 * POLY);
            }
            let (ct, k) = encapsulate(&mut expander(&stream[..64]), &pk).expect("endless");
            let (k2, ok) = decapsulate(&ct, &sk);
            assert!(bool::from(ok) && bool::from(k.ct_eq(&k2)));
        }
        _ => {
            let rest = &rest[..rest.len().min(2 * POLY + 8)];
            let mut src = finite(rest);
            match generate_key(&mut src) {
                Ok((pk, _)) => {
                    assert!(rest.len() >= 2 * POLY);
                    let _ = encapsulate(&mut src, &pk);
                }
                // Too short for g and f, or for a rejected g and then f.
                Err(()) => assert!(rest.len() < 3 * POLY),
            }
        }
    }
});
