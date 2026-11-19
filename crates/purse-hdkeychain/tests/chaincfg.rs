// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The `chaincfg` feature: dcroxide-chaincfg's parameters as
//! [`NetworkParams`], and the [`HdVersions`] constants equal to them.

#![cfg(feature = "chaincfg")]

use purse_hdkeychain::{ExtendedKey, HdVersions, NetworkParams};

#[test]
fn versions_equal_chaincfg() {
    let cases = [
        (HdVersions::MAINNET, dcroxide_chaincfg::mainnet_params()),
        (HdVersions::TESTNET3, dcroxide_chaincfg::testnet3_params()),
        (HdVersions::SIMNET, dcroxide_chaincfg::simnet_params()),
        (HdVersions::REGNET, dcroxide_chaincfg::regnet_params()),
    ];
    for (versions, params) in &cases {
        assert_eq!(HdVersions::from(params), *versions);
        assert_eq!(params.hd_priv_key_version(), versions.private);
        assert_eq!(params.hd_pub_key_version(), versions.public);
    }
}

#[test]
fn params_work_as_network_params() {
    let params = dcroxide_chaincfg::mainnet_params();
    let a = ExtendedKey::new_master(&[9u8; 32], &params).expect("master");
    let b = ExtendedKey::new_master(&[9u8; 32], &HdVersions::MAINNET).expect("master");
    assert_eq!(*a.string(), *b.string());
    let parsed = ExtendedKey::new_key_from_string(&a.neuter().string(), &params).expect("parse");
    assert_eq!(*parsed.string(), *b.neuter().string());
}
