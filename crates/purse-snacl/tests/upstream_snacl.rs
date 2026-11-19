// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2014 The btcsuite developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `wallet/internal/snacl/snacl_test.go`.
//!
//! The Go tests share package-level state and depend on running in source
//! order (QUIRKS QK-0658), so they are one test here, step by step, with
//! the same password, message and costs (`DefaultN`, `DefaultR`,
//! `DefaultP`). They have no fixed vectors: salts and nonces are random.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: an index into a blob known to be longer"
)]

use purse_errors::Kind;
use purse_snacl::{DEFAULT_N, DEFAULT_P, DEFAULT_R, SecretKey};

#[test]
fn snacl_test_go_in_source_order() {
    let password = b"sikrit";
    let message = b"this is a secret message of sorts";

    // TestNewSecretKey
    let mut key =
        SecretKey::new(password, DEFAULT_N, DEFAULT_R, DEFAULT_P).expect("TestNewSecretKey");

    // TestMarshalSecretKey
    let params = key.marshal();

    // TestUnmarshalSecretKey
    let mut sk = SecretKey::unmarshal(&params).expect("unexpected unmarshal error");
    sk.derive_key(password).expect("unexpected DeriveKey error");
    assert_eq!(sk.key().as_bytes(), key.key().as_bytes(), "keys not equal");

    // TestUnmarshalSecretKeyInvalid
    let mut sk = SecretKey::unmarshal(&params).expect("unexpected unmarshal error");
    let err = sk
        .derive_key(b"wrong password")
        .expect_err("wrong password didn't fail");
    assert!(err.is_kind(Kind::Passphrase), "wrong password didn't fail");

    // TestEncrypt
    let mut blob = key.encrypt(message).expect("TestEncrypt");

    // TestDecrypt
    let decrypted = key.decrypt(&blob).expect("TestDecrypt");
    assert_eq!(decrypted.as_bytes(), message, "decryption failed");

    // TestDecryptCorrupt
    let at = blob.len() - 15;
    blob[at] = blob[at].wrapping_add(1);
    assert!(key.decrypt(&blob).is_err(), "corrupt message decrypted");

    // TestZero
    key.zero();
    assert_eq!(key.key().as_bytes(), &[0u8; 32], "zero key failed");

    // TestDeriveKey
    key.derive_key(password)
        .expect("unexpected DeriveKey key failure");

    // TestDeriveKeyInvalid
    let err = key
        .derive_key(b"bogus")
        .expect_err("bogus passphrase accepted");
    assert!(
        err.is_kind(Kind::Passphrase),
        "unexpected DeriveKey key failure: {err}"
    );
}
