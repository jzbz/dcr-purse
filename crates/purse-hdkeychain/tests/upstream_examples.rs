// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2023 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrd's `hdkeychain` v3.1.3 `example_test.go`: its three examples, whose
//! `// Output:` lines `go test` checks, ported as tests with the same
//! expected output. The addresses are made as the example makes them
//! (`stdaddr.Hash160` of the serialized public key, then a version 0
//! P2PKH-ECDSA address), here with dcroxide-txscript's `stdaddr`. One more
//! test checks the address vectors of dcrwallet's own `TestAddresses`.

use dcroxide_txscript::stdaddr;
use purse_hdkeychain::{
    ExtendedKey, HARDENED_KEY_START, HdVersions, RECOMMENDED_SEED_LEN, generate_seed,
};
use purse_testutil::SplitMix64;

/// The master key both examples hard-code.
const MASTER: &str = "dprv3hCznBesA6jBushjx7y9NrfheE4ZshnaKYtsoLXefmLPzrXgEiXkd\
                      RMD6UngnmBYZzgNhdEd4K3PidxcaCiR6HC9hmpj8FcrP4Cv7zBwELA";

/// `Example_newMaster`: a seed at the recommended length makes a private
/// master key. (Go draws the seed from the operating system; any bytes do.)
#[test]
fn example_new_master() {
    let mut rng = SplitMix64::from_env("hdkeychain-example-new-master");
    let seed = generate_seed(RECOMMENDED_SEED_LEN, |b: &mut [u8]| {
        rng.fill(b);
        Ok::<(), std::convert::Infallible>(())
    })
    .expect("seed");
    let key = ExtendedKey::new_master(&seed, &HdVersions::MAINNET).expect("master");
    // Output: Private Extended Key?: true
    assert_eq!(
        format!("Private Extended Key?: {}", key.is_private()),
        "Private Extended Key?: true"
    );
}

/// `Example_defaultWalletLayout`: m/0'/0/10 and m/0'/1/0 of the hard-coded
/// master key, as mainnet P2PKH addresses.
#[test]
fn example_default_wallet_layout() {
    let net = dcroxide_chaincfg::mainnet_params();
    let master_key =
        ExtendedKey::new_key_from_string(MASTER, &HdVersions::MAINNET).expect("master");
    let acct0 = master_key.child(HARDENED_KEY_START).expect("m/0'");
    let acct0_ext = acct0.child(0).expect("m/0'/0");
    let acct0_int = acct0.child(1).expect("m/0'/1");
    let acct0_ext10 = acct0_ext.child(10).expect("m/0'/0/10");
    let acct0_int0 = acct0_int.child(0).expect("m/0'/1/0");

    let pub_key_hash_addr = |ext_key: &ExtendedKey| {
        let pk_hash = stdaddr::hash160(ext_key.serialized_pub_key());
        stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&pk_hash, &net)
            .expect("address")
            .to_string()
    };
    let out = format!(
        "Account 0 External Address 10: {}\nAccount 0 Internal Address 0: {}",
        pub_key_hash_addr(&acct0_ext10),
        pub_key_hash_addr(&acct0_int0)
    );
    assert_eq!(
        out,
        "Account 0 External Address 10: DshMmJ3bfvMDdk1mkXRD3x5xDuPwSxoYGfi\n\
         Account 0 Internal Address 0: DsoTyktAyEDkYpgKSex6zx5rrkFDi2gAsHr"
    );
}

/// `Example_audits`: the neutered master key.
#[test]
fn example_audits() {
    let master_key =
        ExtendedKey::new_key_from_string(MASTER, &HdVersions::MAINNET).expect("master");
    let master_pub_key = master_key.neuter();
    assert_eq!(
        format!("Audit key N(m/*): {}", *master_pub_key.string()),
        "Audit key N(m/*): dpubZ9169KDAEUnypHbWCe2Vu5TxGEcqJeNeX6XCYFU1fqw2iQZK7fsMhzsEFArbLmyU\
         dprUw9aXHneUNd92bjc31TqC6sUduMY6PK2z4JXDS8j"
    );
}

/// dcrwallet's `wallet/addresses_test.go` `TestAddresses` vectors: the
/// first five external and internal addresses of account 0 of a simnet
/// wallet created from the test seed (the legacy coin type 115, which a new
/// wallet uses until its coin type is upgraded; spec 02, `wallet/addresses_test.go`),
/// with each address's HASH160 and public key. Only the derivation is
/// checked here; the address-buffer behaviour of the same test belongs to
/// the wallet crates.
#[test]
fn dcrwallet_test_addresses_vectors() {
    const SEED: &str = "b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567";
    // (branch, address, HASH160, public key), in the upstream order.
    const EXPECTED: [(u32, &str, &str, &str); 10] = [
        (
            1,
            "SsrFKd8aX4KHabWSQfbmEaDv3BJCpSH2ySj",
            "f032b89ec075ab2847e2ec186ad000be16cf354b",
            "03d1ad44eeac8eb59e9598f7e530a1cbe2c1684c0aa5f45ab24d33d38a2102dd1a",
        ),
        (
            1,
            "SsW4roiFKWkbbhiAeEV5byet1pLKAP4xRks",
            "12d5a8e19b9a070d6d5e6e425b593c2c137285e3",
            "02cbcf5c1aa84bf8e6d04412d867eccbaa6cc12ebb792f3f1eaf4d2887f8e884f3",
        ),
        (
            1,
            "SscaK4A6V94dawc6ZBRCGUxPjdf7um1GJgD",
            "5a38638f09937214b07481c656d0c9c73020f8bf",
            "0392735a0eee9026425556ef5c5ae23ad3e54598132a1ca0d74dbcac7bfe31bfa4",
        ),
        (
            1,
            "Ssm4BeTKgwKGTqNR63WiGtP1FJaKCRJsN1S",
            "b73edb8f32957800e2e3b9424c3b659acac51b7f",
            "037c1e500884c6c3cb044390b52525d324fd67c031fdd9a47d742d0323fe5de73f",
        ),
        (
            1,
            "SssBoVxTkCUb6xs7vph3BHdPmki3weVvRsF",
            "fa8073fcb670ba7312a1ef0d908cfb05c59b70b9",
            "0327540e546f9cfac45f51699e2656732727507971060641ead554d78eeea88aa6",
        ),
        (
            0,
            "SsfPTmZmaXGkXfcNGjftRPmoGGCqtNPCHKx",
            "791376f67fb3becf392b071d25d7c99c82139ee3",
            "031634efb3e83c834a82cdc898000f85215a09dc742d5b3b82ace7221ca1bb0938",
        ),
        (
            0,
            "SsXhSHBiaEan7Ls36bvhLspZ3LC1NKzuwQz",
            "24b8b3d89f987bf3cd80a8c16d9368d683217fa4",
            "0280beb72c6ef42ce3133fd6d340fd5bedcfccaded5a6eabb6d2430e3958bf7c85",
        ),
        (
            0,
            "SspSfaWDNwc9TA31Q9iR2jot2eV1hk2ix6U",
            "dc67b3d95adb1789efe4aa73607d8a8c57eee2bb",
            "03b120e0e073a12a1957680251a1562c5c6e30e547797fb5411107eac19699f601",
        ),
        (
            0,
            "SsrSYTB9MQQ1czAfPmWE66ZFqv7NrwzqfQT",
            "f252015c8e0059c21cae623704d8588d12ca5c74",
            "0350822c9bd61f524f4d68fa605e850c34c5e8ccc9b5cf278782131c1e21dd261b",
        ),
        (
            0,
            "SsqBcGBre8SZrG61cd5M5e2GaMJbK2CMdEa",
            "e486d22d1244becac5a30b38dda1c8c4c1b3bdeb",
            "031c494068c9c454bef7de35fa4717f21c07dec4471bd8500650b133d57e49a81d",
        ),
    ];
    let params = dcroxide_chaincfg::simnet_params();
    let net = HdVersions::SIMNET;
    let seed = purse_testutil::unhex(SEED);
    let acct = ExtendedKey::new_master(&seed, &net)
        .and_then(|m| m.child(HARDENED_KEY_START | 44))
        .and_then(|p| p.child(HARDENED_KEY_START | params.legacy_coin_type))
        .and_then(|c| c.child(HARDENED_KEY_START))
        .expect("account 0");
    let xpub = ExtendedKey::new_key_from_string(&acct.neuter().string(), &net).expect("xpub");
    let mut next = [0u32; 2];
    for (branch, address, hash, pub_key) in EXPECTED {
        let index = next[branch as usize];
        next[branch as usize] = index.saturating_add(1);
        let child = xpub
            .child(branch)
            .and_then(|b| b.child(index))
            .expect("child");
        let h160 = stdaddr::hash160(child.serialized_pub_key());
        let addr =
            stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&h160, &params).expect("address");
        let at = format!("branch {branch} index {index}");
        assert_eq!(
            purse_testutil::hex(child.serialized_pub_key()),
            pub_key,
            "{at}"
        );
        assert_eq!(purse_testutil::hex(&h160), hash, "{at}");
        assert_eq!(addr.to_string(), address, "{at}");
    }
    assert_eq!(params.legacy_coin_type, 115);
}
