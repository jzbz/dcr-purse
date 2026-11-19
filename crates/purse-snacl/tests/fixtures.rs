// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's own fixture databases (copied into purse-boltread's test
//! data from `wallet/udb/testdata`), read with purse-boltread: their master
//! key parameters derived with the passphrases the generators name, their
//! crypto keys and every sealed extended key opened, and the wrong
//! passphrases refused with Go's exact error. Each fixture's requests are
//! checked against the frozen Go outcomes in `tests/data/fixture_vectors.txt`
//! and, when the oracle is available, against Go live.
//!
//! The fixtures are testnet databases, so every derivation is scrypt at
//! `N = 2^18, r = 8, p = 1` (256 MiB), dcrwallet's real cost.

mod common;

use common::*;
use purse_testutil::oracle_or_skip;
use purse_testutil::serde_json::json;
use purse_testutil::unhex;

fn run(file: &str, wrong: bool) {
    let flow = fixture_flow(file, wrong);

    // Frozen Go outcomes for exactly these requests.
    let frozen: Vec<Entry> = read_vectors(FIXTURE_VECTORS, FIXTURE_VECTORS_ENTRIES)
        .into_iter()
        .filter(|e| e.fixture.as_deref() == Some(file))
        .collect();
    check_flow_against(&flow, &frozen, file);

    // The public passphrase opens the public master key, the private one the
    // private key; each crypto key opens the extended keys it sealed.
    let derives: Vec<&Entry> = flow.iter().filter(|e| e.cmd == "derive-key").collect();
    assert!(
        derives[0].outcome.get("ok").is_some(),
        "{file}: public passphrase"
    );
    assert!(
        derives[1].outcome.get("ok").is_some(),
        "{file}: private passphrase"
    );
    if wrong {
        for e in &derives[2..] {
            assert_eq!(
                e.outcome,
                json!({"err": {
                    "text": "secretkey.DeriveKey: invalid passphrase",
                    "text_lib": "secretkey.DeriveKey: invalid passphrase",
                    "kind": 10,
                    "op": "secretkey.DeriveKey",
                }}),
                "{file}: wrong passphrase"
            );
        }
    }
    let decrypts: Vec<&Entry> = flow.iter().filter(|e| e.cmd == "secret-decrypt").collect();
    for e in &decrypts[..2] {
        let ck = unhex(e.outcome["ok"]["plaintext"].as_str().expect("crypto key"));
        assert_eq!(ck.len(), 32);
    }
    for e in &decrypts[2..] {
        assert_eq!(
            e.outcome["err"]["text"],
            json!("secretkey.Decrypt: encryption/decryption error:: cryptokey.Decrypt"),
            "{file}: a master key opens only its own crypto key"
        );
    }
    let crypto: Vec<&Entry> = flow.iter().filter(|e| e.cmd == "crypto-decrypt").collect();
    let (opened, refused): (Vec<&&Entry>, Vec<&&Entry>) =
        crypto.iter().partition(|e| e.outcome.get("ok").is_some());
    assert_eq!(
        refused.len(),
        1,
        "{file}: the public crypto key opens no private blob"
    );
    let mut prefixes = std::collections::BTreeSet::new();
    for e in opened {
        let pt = unhex(e.outcome["ok"]["plaintext"].as_str().expect("plaintext"));
        let s = String::from_utf8(pt).expect("an extended key string");
        prefixes.insert(s[..4].to_owned());
    }
    assert_eq!(
        prefixes.into_iter().collect::<Vec<_>>(),
        ["tprv", "tpub"],
        "{file}: testnet extended keys"
    );

    // Go, live, for the same requests.
    if let Some(mut oracle) = oracle_or_skip("snacl") {
        let go: Vec<Entry> = flow
            .iter()
            .map(|e| Entry {
                outcome: oracle.call(&e.cmd, e.args.clone()),
                ..e.clone()
            })
            .collect();
        check_flow_against(&flow, &go, file);
    }
}

#[test]
fn v24_newest_fixture() {
    run(FIXTURES[0].0, FIXTURES[0].1);
}

#[test]
fn v1_oldest_fixture() {
    run(FIXTURES[1].0, FIXTURES[1].1);
}

#[test]
fn fixture_vectors_cover_exactly_the_fixtures() {
    let entries = read_vectors(FIXTURE_VECTORS, FIXTURE_VECTORS_ENTRIES);
    let names: std::collections::BTreeSet<&str> = entries
        .iter()
        .filter_map(|e| e.fixture.as_deref())
        .collect();
    assert_eq!(names.len(), FIXTURES.len());
    assert!(entries.iter().all(|e| e.fixture.is_some()));
}

/// Re-sealing every opened blob with its stored nonce reproduces the
/// fixture's bytes: what dcrwallet wrote, Rust writes.
#[test]
fn v24_blobs_are_reproduced() {
    use purse_snacl::{CryptoKey, SecretKey};
    let db = open_db(upstream_fixture(FIXTURES[0].0));
    let blobs = fixture_blobs(&db);
    let mut mpub = SecretKey::unmarshal(&blobs.mpub).expect("unmarshal");
    assert_eq!(mpub.marshal().to_vec(), blobs.mpub);
    mpub.derive_key(FIXTURE_PUB_PASS)
        .expect("public passphrase");
    let ck = mpub.decrypt(&blobs.cpub).expect("crypto key");
    let mut nonce = Replay::new(blobs.cpub[..24].to_vec());
    assert_eq!(
        mpub.encrypt_with(ck.as_bytes(), &mut nonce).expect("seal"),
        blobs.cpub
    );
    let ck = CryptoKey::from_slice(ck.as_bytes()).expect("32 bytes");
    for (place, blob) in &blobs.public {
        let pt = ck.decrypt(blob).expect("open");
        let mut nonce = Replay::new(blob[..24].to_vec());
        assert_eq!(
            &ck.encrypt_with(pt.as_bytes(), &mut nonce).expect("seal"),
            blob,
            "{place}"
        );
    }
}
