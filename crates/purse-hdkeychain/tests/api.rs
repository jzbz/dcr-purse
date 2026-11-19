// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! API properties outside upstream's tests: secrets stay out of `Debug`,
//! zeroing, error identities, and value semantics where Go aliases.

use purse_hdkeychain::{Error, ExtendedKey, HARDENED_KEY_START, HdVersions, PubKeyErrorKind};
use purse_testutil::hex;

fn master() -> ExtendedKey {
    ExtendedKey::new_master(&[0x5a; 32], &HdVersions::MAINNET).expect("master")
}

#[test]
fn debug_reveals_no_secrets() {
    let k = master().child(HARDENED_KEY_START).expect("child");
    let priv_hex = hex(k.serialized_priv_key().expect("private"));
    let debug = format!("{k:?} {:?}", k.neuter());
    assert!(!debug.contains(&priv_hex), "{debug}");
    assert!(!debug.contains(&*k.string()), "{debug}");
    assert!(!debug.contains(&*k.neuter().string()), "{debug}");
    assert!(
        !debug.contains("dprv") && !debug.contains("dpub"),
        "{debug}"
    );
    // Neither the private key nor the chain code, in any common rendering.
    let payload = dcroxide_base58::decode(&k.string());
    let chain_code = &payload[13..45];
    assert!(!debug.contains(&hex(chain_code)), "{debug}");
    assert!(
        !debug.contains(&format!("{:?}", &chain_code[..4])),
        "{debug}"
    );
}

#[test]
fn keys_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ExtendedKey>();
    assert_send_sync::<Error>();
}

#[test]
fn zeroed_key_behaves_as_go() {
    let mut k = master().child(HARDENED_KEY_START).expect("child");
    k.zero();
    assert!(!k.is_private());
    assert_eq!(
        (k.depth(), k.child_num(), k.parent_fingerprint()),
        (0, 0, 0)
    );
    assert_eq!(*k.string(), "zeroed extended key");
    assert_eq!(*k.neuter().string(), "zeroed extended key");
    assert_eq!(
        k.child(HARDENED_KEY_START).err(),
        Some(Error::DeriveHardFromPublic)
    );
    let err = k.child(0).expect_err("empty public key");
    assert_eq!(err.to_string(), "malformed public key: invalid length: 0");
    assert!(matches!(&err, Error::PubKey(e) if e.kind() == PubKeyErrorKind::InvalidLen));
    assert!(std::error::Error::source(&err).is_some());
    // The version bytes are kept (and zeroing twice is harmless).
    k.zero();
    assert_eq!(*k.string(), "zeroed extended key");
}

/// Go's `Neuter` aliases the private key's buffers, so zeroing one clears
/// the other; here each key owns its bytes (a documented divergence).
#[test]
fn zeroing_does_not_reach_other_keys() {
    let mut k = master();
    let neutered = k.neuter();
    let child = k.child(1).expect("child");
    let before = (neutered.string(), child.string());
    k.zero();
    assert_eq!((neutered.string(), child.string()), before);
}

#[test]
fn parsed_private_key_keeps_all_32_bytes_and_derived_key_strips() {
    // dcrd BIP32 vector 4: m/0' is 00d948e9...
    let net = HdVersions::MAINNET;
    let seed =
        purse_testutil::unhex("3ddd5602285899a946114506157c7997e5444528f3003f6134712147db19b678");
    let derived = ExtendedKey::new_master(&seed, &net)
        .and_then(|m| m.child(HARDENED_KEY_START))
        .expect("child");
    let stored = derived.serialized_priv_key().expect("private");
    assert_eq!(stored.len(), 31);
    assert_eq!(stored[0], 0xd9);
    let parsed = ExtendedKey::new_key_from_string(&derived.string(), &net).expect("parse");
    let padded = parsed.serialized_priv_key().expect("private");
    assert_eq!(padded.len(), 32);
    assert_eq!(&padded[1..], stored);
    assert_eq!(*parsed.string(), *derived.string());
    // The padded form is the same for both provenances.
    assert_eq!(&derived.priv_key_bytes().expect("private")[..], padded);
    assert_eq!(&parsed.priv_key_bytes().expect("private")[..], padded);
    assert_eq!(
        derived.neuter().priv_key_bytes().err(),
        Some(Error::NotPrivExtKey)
    );
}

#[test]
fn error_texts_are_go_s() {
    // Texts of dcrd hdkeychain v3.1.3 (also checked against the oracle).
    let cases = [
        (
            Error::DeriveHardFromPublic,
            "cannot derive a hardened key from a public key",
        ),
        (
            Error::NotPrivExtKey,
            "unable to create private keys from a public extended key",
        ),
        (
            Error::InvalidChild,
            "the extended key at this index is invalid",
        ),
        (Error::UnusableSeed, "unusable seed"),
        (
            Error::InvalidSeedLen,
            "seed length must be between 128 and 512 bits",
        ),
        (Error::BadChecksum, "bad extended key checksum"),
        (
            Error::InvalidKeyLen,
            "the provided serialized extended key length is invalid",
        ),
        (
            Error::WrongNetwork,
            "the provided serialized extended key is for the wrong network",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(e.to_string(), text);
        assert!(std::error::Error::source(&e).is_none());
    }
    assert_eq!(
        PubKeyErrorKind::NotOnCurve.to_string(),
        "ErrPubKeyNotOnCurve"
    );
}
