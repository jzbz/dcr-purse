// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests through the public API, needing no reference
//! implementation: round trips, tampering, wrong keys, ciphertexts that
//! fail exactly one of decapsulation's three checks (built with the test
//! model of Go's createCipher, itself checked against `encapsulate`),
//! determinism and randomness consumption, error pass-through, and the
//! secret types' formatting. That the values equal Go's is `tests/oracle.rs`'s and
//! `tests/frozen.rs`'s job.
//!
//! `PURSE_SNTRUP_CASES` multiplies the number of random instances;
//! `PURSE_TEST_SEED` picks the seed. An optimised build (or
//! `PURSE_SNTRUP_HEAVY`) runs more.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

mod common;

use common::{
    MOVED_COEFFICIENTS, POLY_BYTES, Stream, alias_stats, env_count, heavy, model_create_cipher,
    model_random_tsmall, move_coefficient, rejection_streams, small_of_weight, tail_aliases,
};
use purse_testutil::SplitMix64;
use sntrup4591761::{
    CIPHERTEXT_SIZE, Ciphertext, ConstantTimeEq, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE, PrivateKey,
    PublicKey, SHARED_KEY_SIZE, decapsulate, encapsulate, generate_key,
};

fn cases(debug: usize, release: usize) -> usize {
    env_count("PURSE_SNTRUP_CASES", 1) * if heavy() { release } else { debug }
}

fn keypair(rng: &mut SplitMix64) -> (PublicKey, PrivateKey, usize) {
    let mut s = Stream::new(rng.bytes(4 * POLY_BYTES));
    let (pk, sk) = generate_key(&mut s.source()).expect("four polynomials suffice");
    (pk, sk, s.consumed())
}

/// A g that is a multiple of a factor of the modulus over GF(3) has no
/// inverse in R3 and is rejected: key generation draws a fresh candidate,
/// and the key pair is the one the stream without the rejected candidates
/// gives.
#[test]
fn non_invertible_g_is_rejected() {
    let mut rng = SplitMix64::from_env("non_invertible_g_is_rejected");
    let rest = rng.bytes(2 * POLY_BYTES);
    let mut s = Stream::new(rest.clone());
    let (pk, sk) = generate_key(&mut s.source()).expect("two polynomials");
    assert_eq!(s.consumed(), 2 * POLY_BYTES);
    for (label, stream, rejected) in rejection_streams(&mut rng, &rest) {
        let mut s = Stream::new(stream);
        let (pk2, sk2) = generate_key(&mut s.source()).expect("enough bytes");
        assert_eq!(s.consumed(), (2 + rejected) * POLY_BYTES, "{label}");
        assert_eq!(pk2, pk, "{label}");
        assert!(bool::from(sk2.ct_eq(&sk)), "{label}");
    }
}

#[test]
fn sizes() {
    assert_eq!(PUBLIC_KEY_SIZE, 1218);
    assert_eq!(PRIVATE_KEY_SIZE, 1600);
    assert_eq!(CIPHERTEXT_SIZE, 1047);
    assert_eq!(SHARED_KEY_SIZE, 32);
}

/// decapsulate(encapsulate(pk)) gives the encapsulated key, valid; the
/// private key carries the public key; keys come from exactly two draws.
#[test]
fn round_trip() {
    let mut rng = SplitMix64::from_env("round_trip");
    for _ in 0..cases(3, 24) {
        let (pk, sk, used) = keypair(&mut rng);
        assert_eq!(used, 2 * POLY_BYTES, "one g candidate (invertible) and f");
        assert_eq!(sk.public_key(), pk);
        assert_eq!(&sk.as_bytes()[382..], &pk.as_bytes()[..]);
        for _ in 0..3 {
            let mut s = Stream::new(rng.bytes(POLY_BYTES + 5));
            let (ct, k) = encapsulate(&mut s.source(), &pk).expect("enough bytes");
            assert_eq!(s.consumed(), POLY_BYTES);
            let (k2, ok) = decapsulate(&ct, &sk);
            assert!(bool::from(ok));
            assert!(bool::from(k.ct_eq(&k2)));
        }
    }
}

/// The same stream gives the same keys and ciphertexts; another stream
/// gives others.
#[test]
fn deterministic_from_the_stream() {
    let mut rng = SplitMix64::from_env("deterministic_from_the_stream");
    let stream = rng.bytes(3 * POLY_BYTES);
    let a = generate_key(&mut Stream::new(stream.clone()).source()).expect("keygen");
    let b = generate_key(&mut Stream::new(stream.clone()).source()).expect("keygen");
    assert_eq!(a.0, b.0);
    assert!(bool::from(a.1.ct_eq(&b.1)));
    let c = generate_key(&mut Stream::new(rng.bytes(3 * POLY_BYTES)).source()).expect("keygen");
    assert_ne!(a.0, c.0);

    let r = rng.bytes(POLY_BYTES);
    let (ct1, k1) = encapsulate(&mut Stream::new(r.clone()).source(), &a.0).expect("encap");
    let (ct2, k2) = encapsulate(&mut Stream::new(r).source(), &a.0).expect("encap");
    assert_eq!(ct1, ct2);
    assert!(bool::from(k1.ct_eq(&k2)));
}

/// Any change to the 32-byte confirmation, and any single bit flip in the
/// body before its last three bytes, makes the ciphertext invalid; a flip
/// in the last three bytes does too unless it lands on one of Go's aliases
/// (QK-0740), which decapsulate to the same key. A ciphertext under
/// another key is invalid.
#[test]
fn tampering_is_detected() {
    let mut rng = SplitMix64::from_env("tampering_is_detected");
    let (pk, sk, _) = keypair(&mut rng);
    let (_, other, _) = keypair(&mut rng);
    for _ in 0..cases(2, 8) {
        let mut s = Stream::new(rng.bytes(POLY_BYTES));
        let (ct, k) = encapsulate(&mut s.source(), &pk).expect("encap");
        assert!(
            !bool::from(decapsulate(&ct, &other).1),
            "wrong key accepted"
        );
        let flips = cases(12, 120);
        for f in 0..flips {
            let mut t = *ct.as_bytes();
            let i = match f % 3 {
                0 => rng.below(32) as usize,
                1 => 32 + rng.below(1012) as usize,
                _ => 1044 + rng.below(3) as usize,
            };
            t[i] ^= 1 << rng.below(8);
            let (k2, ok) = decapsulate(&Ciphertext::from_bytes(t), &sk);
            if i < 1044 {
                assert!(!bool::from(ok), "a flip at byte {i} was accepted");
            } else if bool::from(ok) {
                assert!(
                    tail_aliases(ct.as_bytes()).contains(&t),
                    "accepted, not an alias"
                );
                assert!(bool::from(k.ct_eq(&k2)));
            }
        }
        // Every alias is accepted, with the same key.
        for a in tail_aliases(ct.as_bytes()) {
            let (k2, ok) = decapsulate(&Ciphertext::from_bytes(a), &sk);
            assert!(bool::from(ok));
            assert!(bool::from(k.ct_eq(&k2)));
        }
    }
}

/// The test model of Go's createCipher (`tests/common`), which builds the
/// single-check ciphertexts below and in the oracle corpus, gives exactly
/// what `encapsulate` gives for the same random bytes.
#[test]
fn model_create_cipher_is_encapsulate() {
    let mut rng = SplitMix64::from_env("model_create_cipher_is_encapsulate");
    for _ in 0..cases(2, 8) {
        let (pk, _, _) = keypair(&mut rng);
        for _ in 0..2 {
            let stream = rng.bytes(POLY_BYTES);
            let (ct, k) =
                encapsulate(&mut Stream::new(stream.clone()).source(), &pk).expect("enough bytes");
            let (mct, mk) = model_create_cipher(&model_random_tsmall(&stream), &pk);
            assert_eq!(&mct, ct.as_bytes());
            assert_eq!(&mk, k.as_bytes());
        }
    }
}

/// A ciphertext built honestly from an r whose weight is not 286 is
/// invalid, as in Go: decryption gives that r back (the returned key is
/// its hash), it re-encrypts to c and matches the confirmation, so the
/// weight check alone rejects it. Weight 286 through the same path is
/// valid, the control.
#[test]
fn wrong_weight_is_rejected() {
    let mut rng = SplitMix64::from_env("wrong_weight_is_rejected");
    for _ in 0..cases(1, 4) {
        let (pk, sk, _) = keypair(&mut rng);
        for w in [0, 1, 2, 100, 285, 286, 287, 288, 289, 300, 400] {
            let r = small_of_weight(&mut rng, w);
            let (ct, key) = model_create_cipher(&r, &pk);
            let (k, ok) = decapsulate(&Ciphertext::from_bytes(ct), &sk);
            assert_eq!(k.as_bytes(), &key, "weight {w}: r was not decrypted");
            assert_eq!(bool::from(ok), w == 286, "weight {w}");
        }
    }
}

/// An honest ciphertext with one body coefficient moved by ±3 is invalid,
/// as in Go: r is unchanged (the returned key is the honest one), so the
/// weight and confirmation checks pass and the re-encryption comparison at
/// that coefficient alone rejects it. An optimised build moves every
/// coefficient of each ciphertext; a debug build, the positions the corpus
/// uses and a random sample.
#[test]
fn moved_coefficient_is_rejected() {
    let mut rng = SplitMix64::from_env("moved_coefficient_is_rejected");
    let (pk, sk, _) = keypair(&mut rng);
    for _ in 0..cases(1, 2) {
        let (ct, k) =
            encapsulate(&mut Stream::new(rng.bytes(POLY_BYTES)).source(), &pk).expect("encap");
        let positions: Vec<usize> = if heavy() {
            (0..761).collect()
        } else {
            let mut p = MOVED_COEFFICIENTS.to_vec();
            p.extend((0..8).map(|_| rng.below(761) as usize));
            p
        };
        for i in positions {
            for delta in [1, -1] {
                let Some(moved) = move_coefficient(ct.as_bytes(), i, delta) else {
                    continue;
                };
                let (k2, ok) = decapsulate(&Ciphertext::from_bytes(moved), &sk);
                assert!(bool::from(k.ct_eq(&k2)), "coefficient {i}: r changed");
                assert!(
                    !bool::from(ok),
                    "coefficient {i} moved by {delta}·3: accepted"
                );
            }
        }
    }
}

/// The alias structure of the last three bytes, from the model of Go's
/// decoding in `tests/common`: of the 1531² valid tails, these many have
/// one or two other 24-bit encodings that decode to the same coefficients.
/// `tests/oracle.rs` confirms with Go that the aliases it meets are
/// accepted; `src/tests.rs` checks the model against the crate's decoder
/// and that the four-byte groups have no aliases.
#[test]
fn tail_alias_counts() {
    let (tails, aliases, one, two) = alias_stats();
    assert_eq!(
        (tails, aliases, one, two),
        (724_163, 964_530, 483_796, 240_367)
    );
}

/// Decapsulation never fails outright: random ciphertexts and private keys
/// give a key and `ok` false, deterministically.
#[test]
fn malformed_input_is_answered() {
    let mut rng = SplitMix64::from_env("malformed_input_is_answered");
    for _ in 0..cases(3, 20) {
        let mut ct = [0u8; CIPHERTEXT_SIZE];
        rng.fill(&mut ct);
        let mut sk = [0u8; PRIVATE_KEY_SIZE];
        rng.fill(&mut sk);
        let (ct, sk) = (Ciphertext::from_bytes(ct), PrivateKey::from_bytes(&sk));
        let (k1, ok1) = decapsulate(&ct, &sk);
        let (k2, ok2) = decapsulate(&ct, &sk);
        assert!(!bool::from(ok1) && !bool::from(ok2));
        assert!(bool::from(k1.ct_eq(&k2)));

        let mut pk = [0u8; PUBLIC_KEY_SIZE];
        rng.fill(&mut pk);
        encapsulate(
            &mut Stream::new(rng.bytes(POLY_BYTES)).source(),
            &PublicKey::from(pk),
        )
        .expect("any public key is accepted");
    }
}

/// A failing source's error comes back unchanged, after exactly the draws
/// that preceded it.
#[test]
fn source_errors_pass_through() {
    #[derive(Debug, PartialEq)]
    struct Boom(usize);
    let mut calls = Vec::new();
    let mut src = |buf: &mut [u8]| {
        calls.push(buf.len());
        if calls.len() == 2 {
            Err(Boom(calls.len()))
        } else {
            buf.fill(0x20);
            Ok(())
        }
    };
    // 0x20202020 makes every g coefficient 0: never invertible, so the
    // second draw is a second g candidate.
    assert_eq!(generate_key(&mut src).err(), Some(Boom(2)));
    assert_eq!(calls, [POLY_BYTES, POLY_BYTES]);

    let mut n = 0;
    let mut src = |_: &mut [u8]| -> Result<(), &'static str> {
        n += 1;
        Err("no randomness")
    };
    let pk = PublicKey::from_bytes([0; PUBLIC_KEY_SIZE]);
    assert_eq!(encapsulate(&mut src, &pk).err(), Some("no randomness"));
    assert_eq!(n, 1);
}

#[test]
fn secrets_are_not_formatted() {
    let mut rng = SplitMix64::from_env("secrets_are_not_formatted");
    let (pk, sk, _) = keypair(&mut rng);
    let (ct, k) =
        encapsulate(&mut Stream::new(rng.bytes(POLY_BYTES)).source(), &pk).expect("encap");
    assert_eq!(format!("{sk:?}"), "PrivateKey(<1600 bytes redacted>)");
    assert_eq!(format!("{k:?}"), "SharedKey(<32 bytes redacted>)");
    let pk_dbg = format!("{pk:?}");
    assert!(pk_dbg.starts_with("PublicKey(") && pk_dbg.ends_with("…, 1218 bytes)"));
    assert!(format!("{ct:?}").ends_with("…, 1047 bytes)"));
}
