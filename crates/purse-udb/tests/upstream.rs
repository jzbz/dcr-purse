// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Upstream tests of dcrwallet at the parity pin that the address manager
//! reaches so far, ported:
//!
//! - `wallet/udb/cointype_test.go` `TestCoinTypes`;
//! - `wallet/udb/addressmanager_test.go` `testManagerAPI` (`testLocking`,
//!   `testNewAccount`, `testLookupAccount`, `testForEachAccount`,
//!   `testRenameAccount`), run as `TestManager` runs it: on testnet3 with
//!   the seed and passphrases of `wallet/udb/common_test.go`, in one write
//!   transaction, once on the newly created manager and once after opening
//!   it again (the watching-only half waits for `ConvertToWatchingOnly`, a
//!   later slice);
//! - `TestManagerImports` (`testImportPrivateKey`, `testImportScript`) in
//!   its create mode, with upstream's WIF and P2SH vectors (its
//!   watching-only mode returns before importing anything);
//! - `wallet/addresses_test.go` `TestAddresses`' address vectors: the first
//!   five children of both branches of account 0 of upstream's simnet
//!   wallet with the same seed (address, HASH160, public key), through
//!   `SyncAccountToAddrIndex` and `Address`, locked and unlocked; the
//!   address issuance it also checks belongs to the wallet crate.
//!
//! - `cointype_test.go` `TestCoinTypeUpgrade`, plus a reopen after it;
//! - `TestManagerEncryptDecrypt` (`testEncryptDecryptErrors`,
//!   `testEncryptDecrypt`), but its two checks of an invalid key type, a
//!   value the Rust API cannot express.
//!
//! `TestImportVotingAccount` and `TestImportAccount` (both import an
//! account from an extended private key, the voting-account path),
//! `TestChangePassphrase` and `TestManagerWatchingOnly` wait for the
//! slices that port those methods; `TestUpgrades` exercises the database
//! upgrades, which are not ported (ADR-0013); the transaction store's
//! tests wait for the transaction store.

mod support;

use dcroxide_txscript::stdaddr;
use purse_dcrutil::Wif;
use purse_errors::{AnyError, Kind};
use purse_snacl::{Limits, OsRandom};
use purse_store::{Store, WriteTx};
use purse_testutil::{hex, unhex};
use purse_udb::Manager;
use purse_udb::addrmgr::{
    ChainParams, ExtendedKey, IMPORTED_ADDR_ACCOUNT, IMPORTED_ADDR_ACCOUNT_NAME, ManagedAddress,
    coin_types, derive_account_key, derive_coin_type_key,
};
use support::scratch;

/// `common_test.go`'s seed.
const SEED: &str = "b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567";
/// `common_test.go`'s `pubPassphrase`.
const PUB_PASSPHRASE: &[u8] = b"_DJr{fL4H0O}*-0\n:V1izc)(6BomK";
/// `common_test.go`'s `privPassphrase`.
const PRIV_PASSPHRASE: &[u8] = b"81lUHXnOMZ@?XXd7O9xyDIWIbXX-lj";

#[test]
fn coin_types_match_upstream() {
    for (net, legacy, slip0044) in [("mainnet", 20, 42), ("testnet3", 11, 1), ("simnet", 115, 1)] {
        let params = ChainParams::by_name(net).expect("network");
        assert_eq!(coin_types(&params), (legacy, slip0044), "{net}");
    }
}

/// Creates a wallet in a file store and returns the store and its opened
/// manager.
fn create(net: &str, pub_pass: &[u8], priv_pass: &[u8]) -> (Store, Manager, ChainParams) {
    let chain = ChainParams::by_name(net).expect("network");
    let dir = scratch("upstream");
    let store = Store::create(&dir.join("wallet.redb")).expect("create store");
    let mut created = None;
    store
        .update(|tx| {
            created = Some(Manager::create(
                tx,
                &chain,
                &unhex(SEED),
                pub_pass,
                priv_pass,
                &mut OsRandom,
                &Limits::DEFAULT,
            ));
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
    created.expect("ran").expect("create");
    let mgr = open(&store, &chain, pub_pass);
    (store, mgr, chain)
}

fn open(store: &Store, chain: &ChainParams, pub_pass: &[u8]) -> Manager {
    let tx = store.read().expect("read");
    Manager::open(
        &mut &tx,
        chain,
        pub_pass,
        Box::new(OsRandom),
        Limits::DEFAULT,
    )
    .expect("open")
}

/// `testLocking` (`addressmanager_test.go:147-218`) for a manager that is
/// not watching-only.
fn test_locking(mgr: &Manager) {
    assert!(
        mgr.is_locked(),
        "IsLocked: returned false on locked manager"
    );
    let err = mgr.lock().expect_err("locking a locked manager");
    assert!(err.is_kind(Kind::Locked), "Lock: unexpected error: {err}");

    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
    assert!(
        !mgr.is_locked(),
        "IsLocked: returned true on unlocked manager"
    );

    // Unlocking the manager again is allowed.
    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock again");
    assert!(
        !mgr.is_locked(),
        "IsLocked: returned true on unlocked manager"
    );

    // An invalid passphrase must give an error and a locked manager.
    let err = mgr
        .unlock(b"invalidpassphrase")
        .expect_err("unlock with an invalid passphrase");
    assert!(
        err.is_kind(Kind::Passphrase),
        "Unlock: unexpected error: {err}"
    );
    assert!(
        mgr.is_locked(),
        "IsLocked: manager is unlocked after failed unlock attempt"
    );
}

#[test]
fn locking_matches_upstream_in_create_and_open_modes() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    test_locking(&mgr);
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    test_locking(&mgr);
    // The other public passphrase of common_test.go opens nothing.
    let tx = store.read().expect("read");
    let err = Manager::open(
        &mut &tx,
        &chain,
        b"-0NV4P~VSJBWbunw}%<Z]fuGpbN[ZI",
        Box::new(OsRandom),
        Limits::DEFAULT,
    )
    .expect_err("a wrong public passphrase");
    assert!(err.is_kind(Kind::Passphrase));
    assert_eq!(err.to_string(), "invalid passphrase");
}

/// `addresses_test.go`'s expected public keys, internal branch then
/// external branch, children 0 to 4 of account 0 (simnet, legacy coin
/// type 115).
const INTERNAL: [&str; 5] = [
    "03d1ad44eeac8eb59e9598f7e530a1cbe2c1684c0aa5f45ab24d33d38a2102dd1a",
    "02cbcf5c1aa84bf8e6d04412d867eccbaa6cc12ebb792f3f1eaf4d2887f8e884f3",
    "0392735a0eee9026425556ef5c5ae23ad3e54598132a1ca0d74dbcac7bfe31bfa4",
    "037c1e500884c6c3cb044390b52525d324fd67c031fdd9a47d742d0323fe5de73f",
    "0327540e546f9cfac45f51699e2656732727507971060641ead554d78eeea88aa6",
];
const EXTERNAL: [&str; 5] = [
    "031634efb3e83c834a82cdc898000f85215a09dc742d5b3b82ace7221ca1bb0938",
    "0280beb72c6ef42ce3133fd6d340fd5bedcfccaded5a6eabb6d2430e3958bf7c85",
    "03b120e0e073a12a1957680251a1562c5c6e30e547797fb5411107eac19699f601",
    "0350822c9bd61f524f4d68fa605e850c34c5e8ccc9b5cf278782131c1e21dd261b",
    "031c494068c9c454bef7de35fa4717f21c07dec4471bd8500650b133d57e49a81d",
];

#[test]
fn account0_addresses_match_upstream() {
    // wallet/setup_test.go: the public passphrase "public".
    let (store, mgr, _) = create("simnet", b"public", PRIV_PASSPHRASE);
    let tx = store.read().expect("read");
    for unlocked in [false, true] {
        if unlocked {
            mgr.unlock(PRIV_PASSPHRASE).expect("unlock");
        }
        for (branch, want) in [(1, INTERNAL), (0, EXTERNAL)] {
            let bk = mgr
                .account_branch_extended_pub_key(&mut &tx, 0, branch)
                .expect("branch key");
            for (i, w) in want.iter().enumerate() {
                let child = bk.child(u32::try_from(i).expect("small")).expect("child");
                assert_eq!(
                    purse_testutil::hex(child.serialized_pub_key()),
                    *w,
                    "branch {branch} child {i} (unlocked: {unlocked})"
                );
            }
        }
    }
}

/// Runs `f` in one write transaction, committed (as `walletdb.Update`).
fn update(store: &Store, f: impl FnOnce(&mut WriteTx<'_>)) {
    store
        .update(|tx| {
            f(tx);
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
}

/// `testNewAccount` (`addressmanager_test.go:817-874`), create mode.
fn test_new_account(mgr: &Manager, tx: &mut WriteTx<'_>) {
    let err = mgr
        .new_account(tx, b"test")
        .expect_err("NewAccount on a locked manager");
    assert!(
        err.is_kind(Kind::Locked),
        "NewAccount: expected Locked, got {err}"
    );
    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
    let account = mgr.new_account(tx, b"acct-create").expect("NewAccount");
    assert_eq!(account, 1, "NewAccount account mismatch");
    let err = mgr
        .new_account(tx, b"acct-create")
        .expect_err("duplicate name");
    assert!(
        err.is_kind(Kind::Exist),
        "NewAccount: expected Exist, got {err}"
    );
    for name in [&b""[..], b"imported"] {
        let err = mgr.new_account(tx, name).expect_err("invalid name");
        assert!(
            err.is_kind(Kind::Invalid),
            "NewAccount: expected Invalid, got {err}"
        );
    }
}

/// `testLookupAccount` (`addressmanager_test.go:876-912`).
fn test_lookup_account(mgr: &Manager, tx: &mut WriteTx<'_>) {
    for (name, want) in [
        (&b"default"[..], 0),
        (IMPORTED_ADDR_ACCOUNT_NAME.as_bytes(), IMPORTED_ADDR_ACCOUNT),
    ] {
        assert_eq!(mgr.lookup_account(tx, name).expect("LookupAccount"), want);
    }
    let err = mgr
        .lookup_account(tx, b"non existent account")
        .expect_err("missing account");
    assert!(
        err.is_kind(Kind::NotExist),
        "LookupAccount: unexpected error: {err}"
    );
    assert_eq!(mgr.last_account(tx).expect("LastAccount"), 1);
}

/// `testForEachAccount` (`addressmanager_test.go:950-974`).
fn test_for_each_account(mgr: &Manager, tx: &mut WriteTx<'_>) {
    assert_eq!(
        mgr.for_each_account(tx).expect("ForEachAccount"),
        [0, 1, 2_147_483_647]
    );
}

/// `testRenameAccount` (`addressmanager_test.go:914-948`) on account 1.
fn test_rename_account(mgr: &Manager, tx: &mut WriteTx<'_>) {
    let name = mgr.account_name(tx, 1).expect("AccountName");
    let mut renamed = name.clone();
    renamed.extend_from_slice(b"-renamed");
    mgr.rename_account(tx, 1, &renamed).expect("RenameAccount");
    assert_eq!(mgr.account_name(tx, 1).expect("AccountName"), renamed);
    let err = mgr
        .rename_account(tx, 1, &renamed)
        .expect_err("duplicate name");
    assert!(
        err.is_kind(Kind::Exist),
        "RenameAccount: unexpected error: {err}"
    );
    let err = mgr.lookup_account(tx, &name).expect_err("old name");
    assert!(
        err.is_kind(Kind::NotExist),
        "LookupAccount: unexpected error: {err}"
    );
}

/// `testManagerAPI` (`addressmanager_test.go:1120-1144`): everything in
/// one write transaction.
fn test_manager_api(store: &Store, mgr: &Manager, create: bool) {
    update(store, |tx| {
        test_locking(mgr);
        if create {
            test_new_account(mgr, tx);
        }
        test_lookup_account(mgr, tx);
        test_for_each_account(mgr, tx);
        test_rename_account(mgr, tx);
    });
}

#[test]
fn manager_api_matches_upstream_in_create_and_open_modes() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    test_manager_api(&store, &mgr, true);
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    test_manager_api(&store, &mgr, false);
    let tx = store.read().expect("read");
    assert_eq!(
        mgr.account_name(&mut &tx, 1).expect("AccountName"),
        b"acct-create-renamed-renamed"
    );
}

/// `testAddress` (`addressmanager_test.go:97-145`) for an expected
/// address.
fn check_address(
    got: &ManagedAddress,
    account: u32,
    address: &str,
    hash: &str,
    internal: bool,
    imported: bool,
    pub_key: Option<&str>,
) {
    assert_eq!(got.account(), account, "{address}: Account");
    assert_eq!(got.address().encode(), address, "EncodeAddress");
    assert_eq!(hex(&got.addr_hash()), hash, "{address}: AddrHash");
    assert_eq!(got.internal(), internal, "{address}: Internal");
    assert_eq!(got.imported(), imported, "{address}: Imported");
    if let Some(p) = pub_key {
        assert_eq!(
            got.pub_key().map(hex).as_deref(),
            Some(p),
            "{address}: PubKey"
        );
    }
}

#[test]
fn imports_match_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    // testImportPrivateKey's vector.
    let wif = Wif::decode(
        "PtWUqkS3apLoZUevFtG3Bwt6uyX8LQfYttycGkt2XCzgxquPATQgG",
        chain.ids.private_key,
    )
    .expect("DecodeWIF");
    let (address, hash) = (
        "TsSYVKf24LcrxyWHBqj4oBcU542PcjH1iA2",
        "10b601a41d2320527c95eb4cdae2c75b45ae45e1",
    );
    let pub_key = "03df8852b90ce8da7de6bcbacd26b78534ad9e46dc1b62a01dcf43f5837d7f9f5e";
    let priv_key = "ac4cb1a53c4f04a71fffbff26d4500c8a95443936deefd1b6ed89727a6858e08";
    // testImportScript's vector.
    let script =
        unhex("51210373c717acda38b5aa4c00c33932e059cdbc11deceb5f00490a9101704cc444c5151ae");
    let (p2sh, p2sh_hash) = (
        "TcsXPUraiDWZoeQBEbw7T7LSgrvD7dar9DA",
        "db7e6d507e3e291a5ab2fac10107f4479c1f4f9c",
    );
    update(&store, |tx| {
        mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
        let got = mgr.import_private_key(tx, &wif).expect("ImportPrivateKey");
        check_address(
            &got,
            IMPORTED_ADDR_ACCOUNT,
            address,
            hash,
            false,
            true,
            Some(pub_key),
        );
        let utility = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&unhex(hash), &chain)
            .expect("address");
        let p2sh_addr = stdaddr::new_address_script_hash_v0(&script, &chain).expect("p2sh");
        for unlocked in [true, false] {
            let got = mgr.address(tx, &utility).expect("Address");
            check_address(
                &got,
                IMPORTED_ADDR_ACCOUNT,
                address,
                hash,
                false,
                true,
                Some(pub_key),
            );
            // Not in upstream's test: the private key, which opens only
            // while unlocked.
            match mgr.private_key(tx, &utility) {
                Ok(k) => {
                    assert!(unlocked);
                    assert_eq!(hex(&k.serialize()[..]), priv_key);
                }
                Err(e) => {
                    assert!(!unlocked);
                    assert!(e.is_kind(Kind::Crypto), "{e}");
                }
            }
            if unlocked {
                mgr.lock().expect("Lock");
            }
        }
        // testImportScript: imported while unlocked, then read locked and
        // unlocked.
        mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
        let got = mgr.import_script(tx, &script).expect("ImportScript");
        check_address(
            &got,
            IMPORTED_ADDR_ACCOUNT,
            p2sh,
            p2sh_hash,
            false,
            true,
            None,
        );
        assert_eq!(hex(&got.addr_hash()), hex(&stdaddr::hash160(&script)));
        for _ in 0..2 {
            let got = mgr.address(tx, &p2sh_addr).expect("Address");
            check_address(
                &got,
                IMPORTED_ADDR_ACCOUNT,
                p2sh,
                p2sh_hash,
                false,
                true,
                None,
            );
            assert_eq!(got.redeem_script(), Some((0, &script[..])));
            if !mgr.is_locked() {
                mgr.lock().expect("Lock");
            }
        }
    });
}

/// `addresses_test.go`'s expected addresses and HASH160s, internal branch
/// then external branch, children 0 to 4 of account 0.
const INTERNAL_ADDRS: [(&str, &str); 5] = [
    (
        "SsrFKd8aX4KHabWSQfbmEaDv3BJCpSH2ySj",
        "f032b89ec075ab2847e2ec186ad000be16cf354b",
    ),
    (
        "SsW4roiFKWkbbhiAeEV5byet1pLKAP4xRks",
        "12d5a8e19b9a070d6d5e6e425b593c2c137285e3",
    ),
    (
        "SscaK4A6V94dawc6ZBRCGUxPjdf7um1GJgD",
        "5a38638f09937214b07481c656d0c9c73020f8bf",
    ),
    (
        "Ssm4BeTKgwKGTqNR63WiGtP1FJaKCRJsN1S",
        "b73edb8f32957800e2e3b9424c3b659acac51b7f",
    ),
    (
        "SssBoVxTkCUb6xs7vph3BHdPmki3weVvRsF",
        "fa8073fcb670ba7312a1ef0d908cfb05c59b70b9",
    ),
];
const EXTERNAL_ADDRS: [(&str, &str); 5] = [
    (
        "SsfPTmZmaXGkXfcNGjftRPmoGGCqtNPCHKx",
        "791376f67fb3becf392b071d25d7c99c82139ee3",
    ),
    (
        "SsXhSHBiaEan7Ls36bvhLspZ3LC1NKzuwQz",
        "24b8b3d89f987bf3cd80a8c16d9368d683217fa4",
    ),
    (
        "SspSfaWDNwc9TA31Q9iR2jot2eV1hk2ix6U",
        "dc67b3d95adb1789efe4aa73607d8a8c57eee2bb",
    ),
    (
        "SsrSYTB9MQQ1czAfPmWE66ZFqv7NrwzqfQT",
        "f252015c8e0059c21cae623704d8588d12ca5c74",
    ),
    (
        "SsqBcGBre8SZrG61cd5M5e2GaMJbK2CMdEa",
        "e486d22d1244becac5a30b38dda1c8c4c1b3bdeb",
    ),
];

#[test]
fn account0_address_records_match_upstream() {
    let (store, mgr, chain) = create("simnet", b"public", PRIV_PASSPHRASE);
    update(&store, |tx| {
        for branch in [0, 1] {
            mgr.sync_account_to_addr_index(tx, 0, 4, branch)
                .expect("SyncAccountToAddrIndex");
        }
    });
    let tx = store.read().expect("read");
    for unlocked in [false, true] {
        if unlocked {
            mgr.unlock(PRIV_PASSPHRASE).expect("unlock");
        }
        for (branch, addrs, keys) in [
            (1u32, INTERNAL_ADDRS, INTERNAL),
            (0, EXTERNAL_ADDRS, EXTERNAL),
        ] {
            for (i, ((address, hash), key)) in addrs.iter().zip(keys).enumerate() {
                let decoded = stdaddr::decode_address(address, &chain).expect("decode");
                let got = mgr.address(&mut &tx, &decoded).expect("Address");
                check_address(&got, 0, address, hash, branch == 1, false, Some(key));
                assert_eq!(got.index(), Some(u32::try_from(i).expect("small")));
                assert_eq!(
                    mgr.addr_account(&mut &tx, &decoded).expect("AddrAccount"),
                    0
                );
                let have: Result<bool, AnyError> = mgr.have_private_key(&mut &tx, &decoded);
                assert!(have.expect("HavePrivateKey"));
            }
        }
    }
}

/// `cointype_test.go` `TestCoinTypeUpgrade`: a testnet3 wallet of
/// upstream's seed upgrades from the legacy coin type to SLIP0044 in one
/// write transaction, after which the coin type key, account 0's xpub and
/// the first address of accounts 0 and 1 are the SLIP0044 derivations, and
/// a second upgrade is refused with `Invalid`.
#[test]
fn coin_type_upgrade_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    let (legacy, slip0044) = coin_types(&chain);
    let master = ExtendedKey::new_master(&unhex(SEED), &chain.hd).expect("NewMaster");
    let legacy_key = derive_coin_type_key(&master, legacy).expect("legacy coin type key");
    let slip_key = derive_coin_type_key(&master, slip0044).expect("SLIP0044 coin type key");
    let acct0 = derive_account_key(&slip_key, 0)
        .expect("account 0")
        .neuter();
    let acct1 = derive_account_key(&slip_key, 1)
        .expect("account 1")
        .neuter();
    // deriveChildAddress(key, 0, 0): branch 0, child 0, P2PKH.
    let first = |k: &ExtendedKey| -> [u8; 20] {
        let child = k.child(0).expect("branch").child(0).expect("child");
        stdaddr::hash160(child.serialized_pub_key())
    };
    let (addr0, addr1) = (first(&acct0), first(&acct1));
    update(&store, |tx| {
        mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
        assert_eq!(
            mgr.coin_type(tx).expect("CoinType"),
            legacy,
            "initialized database has wrong coin type"
        );
        assert_eq!(
            mgr.coin_type_priv_key(tx)
                .expect("CoinTypePrivKey")
                .string(),
            legacy_key.string(),
            "initialized database has wrong coin type key"
        );
        mgr.upgrade_to_slip0044_coin_type(tx)
            .expect("UpgradeToSLIP0044CoinType");
        assert_eq!(
            mgr.coin_type(tx).expect("CoinType"),
            slip0044,
            "upgraded database has wrong coin type"
        );
        assert_eq!(
            mgr.coin_type_priv_key(tx)
                .expect("CoinTypePrivKey")
                .string(),
            slip_key.string(),
            "upgraded database has wrong coin type key"
        );
        assert_eq!(
            mgr.account_extended_pub_key(tx, 0)
                .expect("AccountExtendedPubKey")
                .string(),
            acct0.string(),
            "upgraded database has wrong account xpub"
        );
        mgr.sync_account_to_addr_index(tx, 0, 1, 0)
            .expect("SyncAccountToAddrIndex");
        assert!(
            mgr.exists_hash160(tx, &addr0).expect("ExistsHash160"),
            "upgraded database does not record SLIP0044-derived account 0 branch 0 address 0"
        );
        mgr.new_account(tx, b"account-1").expect("NewAccount");
        assert_eq!(
            mgr.account_extended_pub_key(tx, 1)
                .expect("AccountExtendedPubKey")
                .string(),
            acct1.string(),
            "upgraded database derived wrong account xpub"
        );
        mgr.sync_account_to_addr_index(tx, 1, 1, 0)
            .expect("SyncAccountToAddrIndex");
        assert!(
            mgr.exists_hash160(tx, &addr1).expect("ExistsHash160"),
            "upgraded database does not record SLIP0044-derived account 1 branch 0 address 0"
        );
        let err = mgr
            .upgrade_to_slip0044_coin_type(tx)
            .expect_err("a second upgrade");
        assert!(
            err.is_kind(Kind::Invalid),
            "upgrade database did not refuse second upgrade with errors.Invalid: {err}"
        );
    });
    // Beyond upstream: the committed upgrade is what a reopened manager
    // reads, and account 0's counters are the zeros QK-0027 describes.
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    let tx = store.read().expect("read");
    assert_eq!(mgr.coin_type(&mut &tx).expect("CoinType"), slip0044);
    assert_eq!(
        mgr.account_extended_pub_key(&mut &tx, 0)
            .expect("xpub")
            .string(),
        acct0.string()
    );
    let p = mgr.account_properties(&mut &tx, 0).expect("props");
    assert_eq!(
        (
            p.last_used_external_index,
            p.last_used_internal_index,
            p.last_returned_external_index,
            p.last_returned_internal_index
        ),
        (0, 0, 0, 0),
        "account 0 after the upgrade (syncing marks nothing used or returned)"
    );
}

/// `addressmanager_test.go` `TestManagerEncryptDecrypt`
/// (`testEncryptDecryptErrors`, `testEncryptDecrypt`). Go's first two
/// checks pass an invalid `CryptoKeyType`, which this API cannot express
/// (one method per key, ADR-0012 Rule 4: the value can only come from Go
/// code). `TstRunWithFailingCryptoKeyPriv` is replaced by a random source
/// that fails after open, the only way sealing can fail here.
#[test]
fn encrypt_decrypt_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    // testEncryptDecryptErrors.
    assert!(mgr.is_locked(), "Manager should be locked at this point.");
    let err = mgr.encrypt_private(&[]).expect_err("locked");
    assert!(
        err.is_kind(Kind::Locked),
        "encryption with private key should fail when manager is locked: {err}"
    );
    let err = mgr.decrypt_private(&[]).expect_err("locked");
    assert!(
        err.is_kind(Kind::Locked),
        "decryption with private key should fail when manager is locked: {err}"
    );
    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
    mgr.lock().expect("Lock");

    // testEncryptDecrypt.
    let plain = b"this is a plaintext";
    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
    for private in [false, true] {
        let sealed = if private {
            mgr.encrypt_private(plain)
        } else {
            mgr.encrypt_public(plain)
        }
        .expect("Failed to encrypt plaintext");
        let opened = if private {
            mgr.decrypt_private(&sealed)
        } else {
            mgr.decrypt_public(&sealed)
        }
        .expect("Failed to decrypt plaintext");
        assert_eq!(opened.as_bytes(), plain);
    }
    mgr.lock().expect("Lock");
    drop(mgr);

    // The Crypto path of Encrypt: sealing fails when the nonce cannot be
    // drawn.
    let mut draws = 0u32;
    let failing = move |buf: &mut [u8]| -> Result<(), purse_snacl::RandomSourceError> {
        draws = draws.saturating_add(1);
        if draws > 1 {
            return Err(purse_snacl::RandomSourceError::new("no entropy"));
        }
        buf.fill(7);
        Ok(())
    };
    let tx = store.read().expect("read");
    let mgr = Manager::open(
        &mut &tx,
        &chain,
        PUB_PASSPHRASE,
        Box::new(failing),
        Limits::DEFAULT,
    )
    .expect("open");
    drop(tx);
    mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
    for private in [false, true] {
        let err = if private {
            mgr.encrypt_private(&[])
        } else {
            mgr.encrypt_public(&[])
        }
        .expect_err("failed encryption");
        assert!(err.is_kind(Kind::Crypto), "failed encryption: {err}");
    }
}
