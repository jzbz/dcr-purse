// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays `tests/data/snacl_vectors.txt`, generated from the Go oracle (see
//! its header), so the crate keeps pinned coverage where no Go toolchain is
//! available. No expected value in it was written by hand.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use purse_snacl::{Argon2idParams, Parameters};
use purse_testutil::serde_json::Value;
use purse_testutil::{DCRWALLET_PARITY_COMMIT, unhex};

#[test]
fn replay_frozen_vectors() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    par_map(&entries, check_entry);
}

#[test]
fn header_names_the_pin() {
    for name in [VECTORS, FIXTURE_VECTORS, GENERATED_EXPECT] {
        let text = std::fs::read_to_string(data_dir().join(name)).expect("read");
        assert!(
            text.lines()
                .take_while(|l| l.starts_with('#'))
                .any(|l| l.contains(DCRWALLET_PARITY_COMMIT)),
            "{name} does not name dcrwallet {DCRWALLET_PARITY_COMMIT}"
        );
    }
}

#[test]
fn every_command_is_covered() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    let cmds: BTreeSet<&str> = entries.iter().map(|e| e.cmd.as_str()).collect();
    let want: BTreeSet<&str> = [
        "new-secret-key",
        "marshal",
        "unmarshal",
        "derive-key",
        "generate-crypto-key",
        "crypto-encrypt",
        "secret-encrypt",
        "crypto-decrypt",
        "secret-decrypt",
        "argon2-new-params",
        "argon2-marshal",
        "argon2-unmarshal",
        "argon2-derive",
        "seal",
        "unseal",
        "account-seal",
        "account-unseal",
    ]
    .into_iter()
    .collect();
    assert_eq!(cmds, want);
}

/// Every error exit of the ported code, and every Go panic turned into an
/// error, is reached by some frozen entry.
#[test]
fn every_error_exit_is_reached() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    let texts: BTreeSet<String> = entries
        .iter()
        .filter_map(|e| {
            e.outcome
                .get("err")
                .map(|o| &o["text"])
                .or_else(|| e.outcome.get("panic"))
                .and_then(|t| t.as_str())
                .map(str::to_owned)
        })
        .collect();
    for want in [
        "cryptokey.Decrypt: invalid operation: missing nonce",
        "cryptokey.Decrypt: encryption/decryption error",
        "secretkey.Decrypt: invalid operation:: cryptokey.Decrypt: missing nonce",
        "secretkey.Decrypt: encryption/decryption error:: cryptokey.Decrypt",
        "secretkey.Unmarshal: invalid encoding: bad marshalled data len ",
        "secretkey.DeriveKey: invalid passphrase",
        "secretkey.DeriveKey: scrypt: N must be > 1 and a power of 2",
        "secretkey.DeriveKey: scrypt: parameters must be > 0",
        "secretkey.DeriveKey: scrypt: parameters are too large",
        "snacl.NewSecretKey: scrypt: N must be > 1 and a power of 2",
        "snacl.NewSecretKey: scrypt: parameters must be > 0",
        "snacl.NewSecretKey: scrypt: parameters are too large",
        "invalid marshaled Argon2id parameters",
        "injected reader: exhausted",
        "argon2: number of rounds too small",
        "argon2: parallelism degree too low",
        "encryption/decryption error: ciphertext too short (len ",
        "invalid passphrase",
    ] {
        assert!(
            texts
                .iter()
                .any(|t| t.starts_with(want) && (want.ends_with(' ') || t == want)),
            "no frozen entry reaches {want:?}"
        );
    }
}

#[test]
fn divergences_are_pinned() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    let n = entries.iter().filter(|e| e.divergence.is_some()).count();
    assert_eq!(n, 6, "argon2 memory below 8 KiB per thread");
}

#[test]
#[should_panic(expected = "body hash")]
fn a_changed_body_fails_the_integrity_check() {
    let text = std::fs::read_to_string(data_dir().join(VECTORS)).expect("read");
    let changed = text.replacen("\"cmd\":\"unmarshal\"", "\"cmd\":\"unmarshal\" ", 1);
    verify_vectors("changed", &changed);
}

#[test]
#[should_panic(expected = "no \"# body-sha256:\" header")]
fn a_missing_header_fails_the_integrity_check() {
    let text = std::fs::read_to_string(data_dir().join(VECTORS)).expect("read");
    let stripped: String = text
        .lines()
        .filter(|l| !l.starts_with("# body-sha256:"))
        .map(|l| format!("{l}\n"))
        .collect();
    verify_vectors("stripped", &stripped);
}

fn hex_arg(e: &Entry, key: &str) -> Vec<u8> {
    unhex(e.args[key].as_str().expect("hex argument"))
}

fn err_text(e: &Entry) -> &str {
    e.outcome["err"]["text"].as_str().unwrap_or_default()
}

/// Fails both of scrypt's first two checks (`N` not a power of two above 1,
/// and `r` or `p` not positive).
fn fails_both_scrypt_checks(n: i64, r: i64, p: i64) -> bool {
    (n <= 1 || n & n.wrapping_sub(1) != 0) && (r <= 0 || p <= 0)
}

/// Costs that fail both of Go's first scrypt checks are in the corpus, for
/// new keys and for stored parameters, so the order of the checks (N first)
/// is pinned.
#[test]
fn scrypt_check_order_is_pinned() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    let int = |v: &Value| v.as_i64().expect("integer");
    let new_keys = entries
        .iter()
        .filter(|e| e.cmd == "new-secret-key")
        .filter(|e| {
            fails_both_scrypt_checks(int(&e.args["n"]), int(&e.args["r"]), int(&e.args["p"]))
        })
        .inspect(|e| {
            assert!(
                err_text(e).ends_with("N must be > 1 and a power of 2"),
                "{e:?}"
            )
        })
        .count();
    assert!(new_keys >= 4, "{new_keys}");
    let stored = entries
        .iter()
        .filter(|e| e.cmd == "derive-key")
        .filter_map(|e| {
            Parameters::unmarshal(&hex_arg(e, "params"))
                .ok()
                .map(|p| (e, p))
        })
        .filter(|(_, p)| fails_both_scrypt_checks(p.n, p.r, p.p))
        .inspect(|(e, _)| {
            assert!(
                err_text(e).ends_with("N must be > 1 and a power of 2"),
                "{e:?}"
            )
        })
        .count();
    assert!(stored >= 2, "{stored}");
}

/// For one stored key, the right passphrase is tried against every byte of
/// its digest flipped, and each is refused: the whole digest is compared.
#[test]
fn every_digest_byte_is_pinned() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    // (params, pass) of each successful derivation.
    let good: BTreeMap<(Vec<u8>, Vec<u8>), ()> = entries
        .iter()
        .filter(|e| e.cmd == "derive-key" && e.outcome.get("ok").is_some())
        .map(|e| ((hex_arg(e, "params"), hex_arg(e, "pass")), ()))
        .collect();
    let mut flipped: BTreeMap<(Vec<u8>, Vec<u8>), BTreeSet<usize>> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.cmd == "derive-key") {
        let (params, pass) = (hex_arg(e, "params"), hex_arg(e, "pass"));
        for ((good_params, good_pass), ()) in &good {
            if *good_pass != pass || good_params.len() != params.len() {
                continue;
            }
            let diff: Vec<usize> = (0..params.len())
                .filter(|&i| params[i] != good_params[i])
                .collect();
            if let [at] = diff[..] {
                assert!((32..64).contains(&at), "{e:?}");
                assert_eq!(
                    err_text(e),
                    "secretkey.DeriveKey: invalid passphrase",
                    "{e:?}"
                );
                flipped
                    .entry((good_params.clone(), pass.clone()))
                    .or_default()
                    .insert(at);
            }
        }
    }
    let all: BTreeSet<usize> = (32..64).collect();
    assert!(
        flipped.values().any(|at| *at == all),
        "no key has every digest byte flipped"
    );
    assert!(
        flipped.len() > 1,
        "single-byte digest flips on one key only"
    );
}

/// Argon2id at the least memory Go and the argon2 crate both accept (8 KiB
/// per thread), for one to four threads, derives the same key in both (not a
/// divergence), for argon2-derive and account-seal alike.
#[test]
fn argon2_memory_boundary_is_pinned() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    for cmd in ["argon2-derive", "account-seal"] {
        let threads: BTreeSet<u8> = entries
            .iter()
            .filter(|e| e.cmd == cmd && e.divergence.is_none())
            .filter_map(|e| {
                Argon2idParams::unmarshal_binary(&hex_arg(e, "params"))
                    .ok()
                    .map(|p| (e, p))
            })
            .filter(|(_, p)| p.time > 0 && p.threads > 0 && p.memory == 8 * u32::from(p.threads))
            .inspect(|(e, _)| assert!(e.outcome.get("ok").is_some(), "{e:?}"))
            .map(|(_, p)| p.threads)
            .collect();
        assert_eq!(threads, (1..=4).collect(), "{cmd}");
    }
}

/// Every Argon2 params blob a derivation, seal or unseal request carries is
/// one Go encoded (an earlier argon2-marshal answer), not this crate's encoding.
#[test]
fn argon2_params_are_gos_encoding() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    let mut encoded: BTreeSet<String> = BTreeSet::new();
    let mut checked = 0;
    for e in &entries {
        match e.cmd.as_str() {
            "argon2-marshal" => {
                encoded.insert(e.outcome["ok"]["data"].as_str().expect("data").to_owned());
            }
            "argon2-derive" | "account-seal" | "account-unseal" => {
                let params = e.args["params"].as_str().expect("params");
                if params.len() == 50 {
                    assert!(encoded.contains(params), "{e:?}");
                    checked += 1;
                }
            }
            _ => {}
        }
    }
    assert!(checked > 60, "{checked}");
}
