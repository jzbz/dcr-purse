// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2013, 2014 The btcsuite developers
// Copyright (c) 2015-2023 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrd's `dcrutil/v4@v4.0.3/wif_test.go` (`TestWIF`) with all its
//! vectors.

use purse_dcrutil::{SignatureType, Wif, WifError};
use purse_testutil::unhex;

const MAIN_NET: [u8; 2] = [0x22, 0xde]; // starts with Pm
const TEST_NET: [u8; 2] = [0x23, 0x0e]; // starts with Pt
const SIM_NET: [u8; 2] = [0x23, 0x07]; // starts with Ps
const REG_NET: [u8; 2] = [0x22, 0xfe]; // starts with Pr
const PRIV1: &str = "0c28fca386c7a227600b2fe50b7cae11ec86d3bf1fbe471be89827e19d72aa1d";
const PRIV2: &str = "dda35a1488fb97b6eb3fe6e9ef2a25814e396fb5dc295fe994b96789b21a0398";
const PRIV3: &str = "0ca35a1488fb97b6eb3fe6e9ef2a25814e396fb5dc295fe994b96789b21a0398";
const PUB1: &str = "02d0de0aaeaefad02b8bdc8a01a1b8b11c696bd3d66a2c5f10780d95b7df42645c";
const PUB2: &str = "02eec2540661b0c39d271570742413bd02932dd0093493fd0beced0b7f93addec4";
const PUB3: &str = "544e3399a8df7b99a0454bfd9691050c8bba58f8d685e98cb72277032a0cf3ac";

struct Case {
    name: &'static str,
    priv_key: &'static str,
    net: [u8; 2],
    dsa: SignatureType,
    skip_make: bool,
    make_err: Option<WifError>,
    wif: &'static str,
    decode_err: Option<WifError>,
    pub_key: &'static str,
}

const BASE: Case = Case {
    name: "",
    priv_key: "",
    net: MAIN_NET,
    dsa: SignatureType::ECDSA_SECP256K1,
    skip_make: false,
    make_err: None,
    wif: "",
    decode_err: None,
    pub_key: "",
};

fn cases() -> Vec<Case> {
    use SignatureType as St;
    vec![
        // Misc construction error tests.
        Case {
            name: "unsupported signature type",
            priv_key: PRIV1,
            net: MAIN_NET,
            dsa: SignatureType(99999),
            make_err: Some(WifError::UnsupportedSignatureType(SignatureType(99999))),
            ..BASE
        },
        Case {
            name: "invalid ed25519 privkey via constructor",
            priv_key: PRIV2,
            net: MAIN_NET,
            dsa: St::ED25519,
            make_err: Some(WifError::ScalarAboveOrder),
            ..BASE
        },
        // Misc decoding error tests.
        Case {
            name: "encoded wif too long",
            skip_make: true,
            net: MAIN_NET,
            wif: "2jV1H5Abfm1gV6GxGxNAvVfuSJPApQ9mr5vNzqJctmzbA6Km9vsBDvW",
            decode_err: Some(WifError::MalformedPrivateKey),
            ..BASE
        },
        Case {
            name: "bad checksum",
            skip_make: true,
            net: MAIN_NET,
            wif: "PmQdMn8xafwaQouk8ngs1CccRCB1ZmsqQxBaxNR4vhQi5a5QB5717",
            decode_err: Some(WifError::ChecksumMismatch),
            ..BASE
        },
        Case {
            name: "bad decoded data length",
            skip_make: true,
            net: MAIN_NET,
            wif: "6A9ymjvs2UDRijKt1s2qfC5i9cvQiZmBDbzmDukTfT3jZS4mStxo",
            decode_err: Some(WifError::MalformedPrivateKey),
            ..BASE
        },
        Case {
            name: "wif for wrong network",
            skip_make: true,
            net: MAIN_NET,
            wif: "PtWVDUidYaiiNT5e2Sfb1Ah4evbaSopZJkkpFBuzkJYcYteugvdFg",
            decode_err: Some(WifError::WrongNetwork(MAIN_NET)),
            ..BASE
        },
        Case {
            name: "invalid ed25519 privkey via decode",
            skip_make: true,
            net: MAIN_NET,
            wif: "PmQgtn9ZpDCASuNpjbwDkxHqskf3kTPFFpLZ4SMWtda6PmFyvM5aG",
            decode_err: Some(WifError::ScalarAboveOrder),
            ..BASE
        },
        // Positive path tests.
        Case {
            name: "mainnet ecdsa secp256k1 priv1",
            priv_key: PRIV1,
            net: MAIN_NET,
            dsa: St::ECDSA_SECP256K1,
            wif: "PmQdMn8xafwaQouk8ngs1CccRCB1ZmsqQxBaxNR4vhQi5a5QB5716",
            pub_key: PUB1,
            ..BASE
        },
        Case {
            name: "testnet ecdsa secp256k1 priv2",
            priv_key: PRIV2,
            net: TEST_NET,
            dsa: St::ECDSA_SECP256K1,
            wif: "PtWVDUidYaiiNT5e2Sfb1Ah4evbaSopZJkkpFBuzkJYcYteugvdFg",
            pub_key: PUB2,
            ..BASE
        },
        Case {
            name: "simnet ecdsa secp256k1 priv2",
            priv_key: PRIV2,
            net: SIM_NET,
            dsa: St::ECDSA_SECP256K1,
            wif: "PsURoUb7FMeJQdTYea8pkbUQFBZAsxtfDcfTLGja5sCLZvLZWRtjK",
            pub_key: PUB2,
            ..BASE
        },
        Case {
            name: "regnet ecdsa secp256k1 priv2",
            priv_key: PRIV2,
            net: REG_NET,
            dsa: St::ECDSA_SECP256K1,
            wif: "Pr9D8L8s9nG4AroRjbTGiRuYrweN1T8Dg9grAEeTEStZAPMnjxwCT",
            pub_key: PUB2,
            ..BASE
        },
        Case {
            name: "mainnet ed25519 priv1",
            priv_key: PRIV1,
            net: MAIN_NET,
            dsa: St::ED25519,
            wif: "PmQfJXKC2ho1633ZiVbSdCZw1y68BVXYFpyE2UfDcbQN5xa3DByDn",
            pub_key: "667c1c28ec2d59ceb33673a94165a5e51d5cbcef620aec663cd83638643bba3b",
            ..BASE
        },
        Case {
            name: "testnet ed25519 priv3",
            priv_key: PRIV3,
            net: TEST_NET,
            dsa: St::ED25519,
            wif: "PtWVaBGeCfbFQfgqFew8YvdrSH5TH439K7rvpo3aWnSfDvyK8ijbK",
            pub_key: PUB3,
            ..BASE
        },
        Case {
            name: "simnet ed25519 priv3",
            priv_key: PRIV3,
            net: SIM_NET,
            dsa: St::ED25519,
            wif: "PsUSAB97uSWqSr4jsnQNJMRC2Y33iD7FDymZuss9rM6PExexSPyTQ",
            pub_key: PUB3,
            ..BASE
        },
        Case {
            name: "regnet ed25519 priv3",
            priv_key: PRIV3,
            net: REG_NET,
            dsa: St::ED25519,
            wif: "Pr9DV2gsos8bD5QcxoipGBrLeJ8EqhLogWnxjqn2zvnbqRg9fZMHs",
            pub_key: PUB3,
            ..BASE
        },
        Case {
            name: "mainnet schnorr secp256k1 priv2",
            priv_key: PRIV1,
            net: MAIN_NET,
            dsa: St::SCHNORR_SECP256K1,
            wif: "PmQhFGVRUjeRmGBPJCW2FCXFck1EoDBF6hks6auNJVQ26M4h73W9W",
            pub_key: PUB1,
            ..BASE
        },
        Case {
            name: "testnet schnorr secp256k1 priv2",
            priv_key: PRIV2,
            net: TEST_NET,
            dsa: St::SCHNORR_SECP256K1,
            wif: "PtWZ6y56SeRZiuMHBrUkFAbhrURogF7xzWL6PQQJ86XvZfeE3jf1a",
            pub_key: PUB2,
            ..BASE
        },
        Case {
            name: "simnet schnorr secp256k1 priv2",
            priv_key: PRIV2,
            net: SIM_NET,
            dsa: St::SCHNORR_SECP256K1,
            wif: "PsUVgxwa9RM9m5jBoywyzbP3SjPQ7QC4uNEjUVDsTfBeahKkmETvQ",
            pub_key: PUB2,
            ..BASE
        },
        Case {
            name: "regnet schnorr secp256k1 priv2",
            priv_key: PRIV2,
            net: REG_NET,
            dsa: St::SCHNORR_SECP256K1,
            wif: "Pr9H1pVL3qxuXK54u1GRxRpC4VUbEtRdMuG8JT8kcEssBALyTRM7v",
            pub_key: PUB2,
            ..BASE
        },
    ]
}

/// The error texts the upstream test compares by text (its `isErr`).
#[test]
fn error_texts() {
    let texts = [
        (
            WifError::UnsupportedSignatureType(SignatureType(99999)),
            "unsupported signature type '99999'",
        ),
        (WifError::ScalarAboveOrder, "not on subgroup (>N)"),
        (WifError::MalformedPrivateKey, "malformed private key"),
        (WifError::ChecksumMismatch, "checksum mismatch"),
    ];
    for (e, text) in texts {
        assert_eq!(e.to_string(), text);
    }
}

#[test]
fn wif() {
    for test in cases() {
        let priv_key = unhex(test.priv_key);
        let pub_key = unhex(test.pub_key);
        let check = |w: &Wif| {
            assert_eq!(w.string().as_str(), test.wif, "{}: encoding", test.name);
            assert_eq!(
                w.priv_key(),
                priv_key.as_slice(),
                "{}: private key",
                test.name
            );
            assert_eq!(w.pub_key(), pub_key.as_slice(), "{}: public key", test.name);
            assert_eq!(w.dsa(), test.dsa, "{}: DSA", test.name);
        };
        if !test.skip_make {
            match (Wif::new(&priv_key, test.net, test.dsa), test.make_err) {
                (Ok(w), None) => check(&w),
                (Err(e), Some(want)) => {
                    assert_eq!(e, want, "{}", test.name);
                    continue;
                }
                (got, want) => panic!("{}: got {got:?}, want error {want:?}", test.name),
            }
        }
        match (Wif::decode(test.wif, test.net), test.decode_err) {
            (Ok(w), None) => check(&w),
            (Err(e), Some(want)) => assert_eq!(e, want, "{}", test.name),
            (got, want) => panic!("{}: got {got:?}, want error {want:?}", test.name),
        }
    }
}
