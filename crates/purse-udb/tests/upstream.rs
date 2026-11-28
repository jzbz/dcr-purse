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
//!   it again;
//! - `TestManagerWatchingOnly`: `testManagerAPI` in create mode, then, after
//!   opening the manager again and converting it with
//!   `ConvertToWatchingOnly`, in watching-only mode on the converted manager
//!   and once more after opening it again;
//! - `TestManagerImports` (`testImportPrivateKey`, `testImportScript`) in
//!   its create mode, with upstream's WIF and P2SH vectors (its
//!   watching-only mode returns before importing anything, so there is
//!   nothing of it to port);
//! - `wallet/addresses_test.go` `TestAddresses`' address vectors: the first
//!   five children of both branches of account 0 of upstream's simnet
//!   wallet with the same seed (address, HASH160, public key), through
//!   `SyncAccountToAddrIndex` and `Address`, locked and unlocked, on the
//!   seed's wallet and, as its watching-only runs do, on that wallet
//!   converted with `ConvertToWatchingOnly` (unlocked first or not) before
//!   any address is derived; the address issuance it also checks belongs to
//!   the wallet crate.
//!
//! - `cointype_test.go` `TestCoinTypeUpgrade`, plus a reopen after it;
//! - `TestManagerEncryptDecrypt` (`testEncryptDecryptErrors`,
//!   `testEncryptDecrypt`), but its two checks of an invalid key type, a
//!   value the Rust API cannot express.
//!
//! - `TestImportVotingAccount`: its six cases on one testnet3 wallet, each
//!   in its own write transaction, with upstream's extended private key;
//!   after each import the account is unlocked in the same transaction and
//!   its type and private key checked, and (beyond upstream) whether it is
//!   still unlocked once committed;
//! - `TestImportAccount`'s cases through `ImportVotingAccount`: its
//!   `importAccount` with the BIP0044 type has no caller but that test (the
//!   one production caller passes `importedVoting`, spec 02 section 3.4),
//!   and the steps it checks (numbering, names, the duplicate-key check
//!   after `SyncAccountToAddrIndex`) are the voting import's.
//!
//! - `TestChangePassphrase` (`testChangePassphrase`) on a testnet3 wallet
//!   with upstream's passphrases, each call committing on its own (the
//!   Rust method runs its own transaction), and its watching-only branches,
//!   which upstream never runs, on the same wallet converted.
//!
//! - `wallet/udb/vsp_test.go`: `TestSetGetDeleteVSPTicket` over
//!   `purse_udb::vsp` on a store of its own, each step in its own
//!   transaction, the record read back equal to the one written with the
//!   host id `SetVSPTicket` assigned (upstream's `DeepEqual` passes because
//!   Go writes that id into the record, QK-0122); and its three codec
//!   round trips (`TestVSPSerializeVSPHost`, `TestVSPSerializeVSPPubKey`,
//!   `TestVSPSerializeVSPTicket`) on the store's codecs of the same
//!   records, with upstream's values. `treasury.go` and `agendas.go` have
//!   no upstream test.
//!
//! `TestUpgrades` exercises the database upgrades, which are not ported
//! (ADR-0013); the transaction store's tests are in `tests/txstore.rs`.

mod support;

use dcroxide_txscript::stdaddr;
use purse_dcrutil::Wif;
use purse_errors::{AnyError, Kind};
use purse_snacl::{Limits, OsRandom};
use purse_store::{Store, WriteTx};
use purse_testutil::{hex, unhex};
use purse_udb::Manager;
use purse_udb::addrmgr::{
    BoxedRandom, ChainParams, ExtendedKey, IMPORTED_ADDR_ACCOUNT, IMPORTED_ADDR_ACCOUNT_NAME,
    ManagedAddress, coin_types, derive_account_key, derive_coin_type_key,
};
use support::scratch;

/// `common_test.go`'s seed.
const SEED: &str = "b46bc6502a30beb92f0aebc776403c3dbf11bfb68305967c36dac9ef8d641567";
/// `common_test.go`'s `pubPassphrase`.
const PUB_PASSPHRASE: &[u8] = b"_DJr{fL4H0O}*-0\n:V1izc)(6BomK";
/// `common_test.go`'s `privPassphrase`.
const PRIV_PASSPHRASE: &[u8] = b"81lUHXnOMZ@?XXd7O9xyDIWIbXX-lj";
/// `common_test.go`'s `pubPassphrase2`.
const PUB_PASSPHRASE2: &[u8] = b"-0NV4P~VSJBWbunw}%<Z]fuGpbN[ZI";
/// `common_test.go`'s `privPassphrase2`.
const PRIV_PASSPHRASE2: &[u8] = b"~{<]08%6!-?2s<$(8$8:f(5[4/!/{Y";

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
    let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
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
    test_locking_as(mgr, false);
}

/// `testLocking` with its `watchingOnly` flag: a watching-only manager
/// refuses `Lock` and every `Unlock` with `WatchingOnly` and stays locked.
fn test_locking_as(mgr: &Manager, watching_only: bool) {
    assert!(
        mgr.is_locked(),
        "IsLocked: returned false on locked manager"
    );
    let err = mgr.lock().expect_err("locking a locked manager");
    if watching_only {
        assert!(
            err.is_kind(Kind::WatchingOnly),
            "Lock: unexpected error: {err}"
        );
        for pass in [PRIV_PASSPHRASE, PRIV_PASSPHRASE, b"invalidpassphrase"] {
            let err = mgr
                .unlock(pass)
                .expect_err("unlocking a watching-only manager");
            assert!(
                err.is_kind(Kind::WatchingOnly),
                "Unlock: unexpected error: {err}"
            );
            assert!(mgr.is_locked(), "IsLocked: watching-only manager unlocked");
        }
        return;
    }
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

/// The same vectors from a watching-only wallet created from account 0's
/// extended public key (`InitializeWatchOnly`, as `dcrwallet --create`
/// with a watching-only key makes it), through the branch keys and
/// through `SyncAccountToAddrIndex` and `Address`.
#[test]
fn account0_addresses_match_upstream_from_the_account_xpub() {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let master = ExtendedKey::new_master(&unhex(SEED), &chain.hd).expect("master");
    let (legacy, _) = coin_types(&chain);
    let xpub = derive_account_key(
        &derive_coin_type_key(&master, legacy).expect("coin type"),
        0,
    )
    .expect("account")
    .neuter();
    let dir = scratch("upstream-wo");
    let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
    update(&store, |tx| {
        Manager::create_watch_only(
            tx,
            &chain,
            xpub.string().as_bytes(),
            b"public",
            &mut OsRandom,
            &Limits::DEFAULT,
        )
        .expect("create_watch_only");
    });
    let mgr = open(&store, &chain, b"public");
    assert!(mgr.watching_only());
    update(&store, |tx| {
        for branch in [0, 1] {
            mgr.sync_account_to_addr_index(tx, 0, 4, branch)
                .expect("SyncAccountToAddrIndex");
        }
    });
    let tx = store.read().expect("read");
    for (branch, want) in [(1, INTERNAL), (0, EXTERNAL)] {
        let bk = mgr
            .account_branch_extended_pub_key(&mut &tx, 0, branch)
            .expect("branch key");
        for (i, w) in want.iter().enumerate() {
            let child = bk.child(u32::try_from(i).expect("small")).expect("child");
            assert_eq!(
                hex(child.serialized_pub_key()),
                *w,
                "branch {branch} child {i}"
            );
            let addr = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                &stdaddr::hash160(child.serialized_pub_key()),
                &chain,
            )
            .expect("p2pkh");
            let got = mgr.address(&mut &tx, &addr).expect("Address");
            assert_eq!(got.pub_key().map(hex).as_deref(), Some(*w));
            let err = mgr
                .private_key(&mut &tx, &addr)
                .expect_err("a watching-only wallet has no private key");
            assert!(err.is_kind(Kind::Locked), "{err}");
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
    test_manager_api_as(store, mgr, create, false);
}

/// `testManagerAPI` with its `watchingOnly` flag (upstream never runs a
/// watching-only manager in create mode, where `testNewAccount` would
/// expect `WatchingOnly`).
fn test_manager_api_as(store: &Store, mgr: &Manager, create: bool, watching_only: bool) {
    update(store, |tx| {
        test_locking_as(mgr, watching_only);
        if create && watching_only {
            let err = mgr.new_account(tx, b"test").expect_err("NewAccount");
            assert!(
                err.is_kind(Kind::WatchingOnly),
                "NewAccount: expected WatchingOnly, got {err}"
            );
        } else if create {
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

/// `TestManagerWatchingOnly` (`addressmanager_test.go:1146-1209`): the API
/// tests in create mode, then on the manager opened again and converted to
/// watching-only, then on the converted manager opened again. Each run
/// renames account 1 once more.
#[test]
fn watching_only_manager_api_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    test_manager_api_as(&store, &mgr, true, false);
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    mgr.convert_to_watching_only(&store)
        .expect("failed to convert manager to watching-only");
    assert!(mgr.watching_only());
    test_manager_api_as(&store, &mgr, false, true);
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    assert!(mgr.watching_only());
    test_manager_api_as(&store, &mgr, false, true);
    let tx = store.read().expect("read");
    assert_eq!(
        mgr.account_name(&mut &tx, 1).expect("AccountName"),
        b"acct-create-renamed-renamed-renamed"
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

/// `TestAddresses`' watching-only runs: upstream's simnet wallet, unlocked
/// first or left locked, converted with `ConvertToWatchingOnly` before any
/// address of it is derived, then the same vectors through
/// `SyncAccountToAddrIndex` and `Address` (upstream asks the wallet's
/// `NewInternalAddress` and `NewExternalAddress`, which the wallet crate
/// ports; the address manager's part is the derivation from account 0's
/// public key and the record it leaves). Beyond upstream: the converted
/// manager is locked and watching-only, keeps answering `HavePrivateKey`
/// true for a derived address (Go's rule is the account number, QK-0037),
/// gives no private key, and answers the same after a reopen.
#[test]
fn account0_addresses_match_upstream_after_a_conversion() {
    for unlock in [false, true] {
        let (store, mgr, chain) = create("simnet", b"public", PRIV_PASSPHRASE);
        if unlock {
            mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
        }
        mgr.convert_to_watching_only(&store)
            .expect("ConvertToWatchingOnly");
        assert!(mgr.watching_only() && mgr.is_locked(), "unlock {unlock}");
        update(&store, |tx| {
            for branch in [0, 1] {
                mgr.sync_account_to_addr_index(tx, 0, 4, branch)
                    .expect("SyncAccountToAddrIndex");
            }
        });
        let reopened = open(&store, &chain, b"public");
        for (m, again) in [(&mgr, false), (&reopened, true)] {
            assert!(m.watching_only() && m.is_locked(), "unlock {unlock}");
            let tx = store.read().expect("read");
            for (branch, addrs, keys) in [
                (1u32, INTERNAL_ADDRS, INTERNAL),
                (0, EXTERNAL_ADDRS, EXTERNAL),
            ] {
                for (i, ((address, hash), key)) in addrs.iter().zip(keys).enumerate() {
                    let what = format!("unlock {unlock}, reopened {again}: {address}");
                    let decoded = stdaddr::decode_address(address, &chain).expect("decode");
                    let got = m.address(&mut &tx, &decoded).expect("Address");
                    check_address(&got, 0, address, hash, branch == 1, false, Some(key));
                    assert_eq!(
                        got.index(),
                        Some(u32::try_from(i).expect("small")),
                        "{what}"
                    );
                    assert_eq!(
                        m.addr_account(&mut &tx, &decoded).expect("AddrAccount"),
                        0,
                        "{what}"
                    );
                    let have: Result<bool, AnyError> = m.have_private_key(&mut &tx, &decoded);
                    assert!(have.expect("HavePrivateKey"), "{what}");
                    let err = m
                        .private_key(&mut &tx, &decoded)
                        .expect_err("a converted wallet gives no private key");
                    assert!(err.is_kind(Kind::Locked), "{what}: {err}");
                }
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

/// `TestImportVotingAccount`'s and `TestImportAccount`'s extended private
/// key (testnet3).
const VOTING_XPRIV: &str = "tprvZUo1ZuEfLLFWg9ktEaXHf3HKq2EdfwY5pXFZ2rbg6HFzeaoJDp1qFZnuuuN6iUAG9EyNF4sH4RmJb395XWYpdqQoXRoKkV88HZwgq95KfiK";

/// One case of [`import_voting_account_matches_upstream`]: its name, the
/// passphrase, the account name, the wanted account or error kind, and
/// whether the wallet is unlocked first.
type VotingCase = (
    &'static str,
    &'static [u8],
    &'static [u8],
    Result<u32, Kind>,
    bool,
);

/// One case of [`import_account_matches_upstream`]: its name, the account
/// name, the wanted account or error kind, and whether an address of the
/// key is recorded first.
type ImportCase = (&'static str, &'static [u8], Result<u32, Kind>, bool);

/// `TestImportVotingAccount` (`addressmanager_test.go:474-604`).
#[test]
fn import_voting_account_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    let xpriv = ExtendedKey::new_key_from_string(VOTING_XPRIV, &chain.hd).expect("xpriv");
    // (name, passphrase, account name, wanted account or error kind, unlock)
    let cases: [VotingCase; 6] = [
        (
            "ok watching only",
            b"abc",
            b"account 1",
            Ok(IMPORTED_ADDR_ACCOUNT + 1),
            false,
        ),
        (
            "ok locked",
            b"abc",
            b"account 2",
            Ok(IMPORTED_ADDR_ACCOUNT + 2),
            false,
        ),
        (
            "ok unlocked",
            b"abc",
            b"account 3",
            Ok(IMPORTED_ADDR_ACCOUNT + 3),
            true,
        ),
        ("name taken", b"abc", b"account 1", Err(Kind::Exist), false),
        (
            "no password",
            b"",
            b"account 4",
            Err(Kind::Passphrase),
            false,
        ),
        ("no name", b"abc", b"", Err(Kind::Invalid), false),
    ];
    for (name, pass, acct_name, want, unlock) in cases {
        let mut imported = None;
        update(&store, |tx| {
            if unlock {
                mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
            }
            let got = mgr.import_voting_account(tx, &xpriv, pass, acct_name);
            match (got, want) {
                (Ok(got), Ok(want)) => {
                    assert_eq!(got, want, "{name}");
                    mgr.unlock_account(tx, got, pass)
                        .unwrap_or_else(|e| panic!("{name}: UnlockAccount: {e}"));
                    let props = mgr.account_properties(tx, got).expect("properties");
                    assert_eq!(props.account_type, 2, "{name}: importedVoting");
                    let key = mgr.account_extended_priv_key(tx, got).expect("xpriv");
                    assert_eq!(key.string().as_str(), VOTING_XPRIV, "{name}");
                    imported = Some(got);
                }
                (Err(e), Err(kind)) => assert!(e.is_kind(kind), "{name}: {e}"),
                (got, want) => panic!("{name}: got {got:?}, wanted {want:?}"),
            }
            if unlock {
                mgr.lock().expect("Lock");
            }
        });
        // Beyond upstream: the account unlocked in the transaction stays
        // unlocked once it commits (Go's cache keeps the key it opened),
        // unless the wallet was locked after it.
        if let Some(acct) = imported {
            let tx = store.read().expect("read");
            assert_eq!(
                mgr.account_has_passphrase(&mut &tx, acct),
                (true, !unlock),
                "{name}"
            );
        }
    }
}

/// `TestImportAccount` (`addressmanager_test.go:606-708`), through
/// `ImportVotingAccount` (see the file's documentation): the account's key
/// is checked after unlocking it with its own passphrase, where upstream's
/// BIP0044 import leaves it under the wallet passphrase.
#[test]
fn import_account_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    let xpriv = ExtendedKey::new_key_from_string(VOTING_XPRIV, &chain.hd).expect("xpriv");
    // (name, account name, wanted account or error kind, discover usage)
    let cases: [ImportCase; 5] = [
        (
            "ok watching only",
            b"account 1",
            Ok(IMPORTED_ADDR_ACCOUNT + 1),
            false,
        ),
        ("ok", b"account 2", Ok(IMPORTED_ADDR_ACCOUNT + 2), false),
        ("name taken", b"account 1", Err(Kind::Exist), false),
        ("address exists", b"account 3", Err(Kind::Exist), true),
        ("no name", b"", Err(Kind::Invalid), false),
    ];
    for (name, acct_name, want, discover) in cases {
        update(&store, |tx| {
            mgr.unlock(PRIV_PASSPHRASE).expect("Unlock");
            if discover {
                mgr.sync_account_to_addr_index(tx, IMPORTED_ADDR_ACCOUNT + 1, 0, 0)
                    .expect("SyncAccountToAddrIndex");
            }
            match (
                mgr.import_voting_account(tx, &xpriv, b"pass", acct_name),
                want,
            ) {
                (Ok(got), Ok(want)) => {
                    assert_eq!(got, want, "{name}");
                    mgr.unlock_account(tx, got, b"pass").expect("UnlockAccount");
                    let key = mgr.account_extended_priv_key(tx, got).expect("xpriv");
                    assert_eq!(key.string().as_str(), VOTING_XPRIV, "{name}");
                }
                (Err(e), Err(kind)) => {
                    assert!(e.is_kind(kind), "{name}: {e}");
                    if discover {
                        assert_eq!(
                            e.render(purse_errors::DAEMON_SEPARATOR),
                            "item already exists: address belonging to this key already \
                             exists in the database"
                        );
                    }
                }
                (got, want) => panic!("{name}: got {got:?}, wanted {want:?}"),
            }
            mgr.lock().expect("Lock");
        });
    }
}

/// A random source that fails while its switch is on: the stand-in for
/// upstream's `TstRunWithReplacedNewSecretKey`, which replaces the secret
/// key constructor. Go's random reads cannot fail, so upstream injects the
/// failure there; here the new key's salt draw fails.
fn switchable_source(fail: std::sync::Arc<std::sync::atomic::AtomicBool>) -> BoxedRandom {
    Box::new(
        move |buf: &mut [u8]| -> Result<(), purse_snacl::RandomSourceError> {
            if fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(purse_snacl::RandomSourceError::new("injected failure"));
            }
            purse_snacl::RandomSource::fill(&mut OsRandom, buf)
        },
    )
}

/// Upstream's `TstCheckPublicPassphrase`: whether `pass` derives the
/// stored public master key. Here, whether a manager opens with it.
fn public_passphrase_opens(store: &Store, chain: &ChainParams, pass: &[u8]) -> bool {
    let tx = store.read().expect("read");
    match Manager::open(&mut &tx, chain, pass, Box::new(OsRandom), Limits::DEFAULT) {
        Ok(_) => true,
        Err(err) => {
            assert!(err.is_kind(Kind::Passphrase), "open: {err}");
            false
        }
    }
}

/// `testChangePassphrase` (`addressmanager_test.go:721-813`) with its
/// `watchingOnly` flag.
fn test_change_passphrase(store: &Store, mgr: &Manager, chain: &ChainParams, watching: bool) {
    // Upstream's "ChangePassphrase (public) with invalid new secret key"
    // is `invalid_new_secret_key_changes_nothing` below, which needs a
    // manager with a failing source.

    let err = mgr
        .change_passphrase(store, b"bogus", PUB_PASSPHRASE2, false)
        .expect_err("ChangePassphrase (public) with invalid old passphrase");
    assert!(err.is_kind(Kind::Passphrase), "unexpected error: {err}");

    mgr.change_passphrase(store, PUB_PASSPHRASE, PUB_PASSPHRASE2, false)
        .expect("ChangePassphrase (public)");
    assert!(
        public_passphrase_opens(store, chain, PUB_PASSPHRASE2),
        "ChangePassphrase (public): passphrase does not match"
    );
    assert!(!public_passphrase_opens(store, chain, PUB_PASSPHRASE));

    mgr.change_passphrase(store, PUB_PASSPHRASE2, PUB_PASSPHRASE, false)
        .expect("ChangePassphrase (public) back");

    let err = mgr
        .change_passphrase(store, b"bogus", PRIV_PASSPHRASE2, true)
        .expect_err("ChangePassphrase (private) with invalid old passphrase");
    let want = if watching {
        Kind::WatchingOnly
    } else {
        Kind::Passphrase
    };
    assert!(err.is_kind(want), "unexpected error: {err}");
    if watching {
        return;
    }

    mgr.change_passphrase(store, PRIV_PASSPHRASE, PRIV_PASSPHRASE2, true)
        .expect("ChangePassphrase (private)");
    mgr.unlock(PRIV_PASSPHRASE2)
        .expect("failed to unlock with new private passphrase");

    // Back again while unlocked: the manager stays unlocked.
    mgr.change_passphrase(store, PRIV_PASSPHRASE2, PRIV_PASSPHRASE, true)
        .expect("ChangePassphrase (private) while unlocked");
    assert!(!mgr.is_locked(), "manager is locked");
    mgr.lock().expect("Lock");
}

#[test]
fn change_passphrase_matches_upstream() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    test_change_passphrase(&store, &mgr, &chain, false);
    // Beyond upstream: the passphrases it ends with are the ones it began
    // with, after a reopen too.
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    mgr.unlock(PRIV_PASSPHRASE).expect("unlock after a reopen");
    mgr.lock().expect("lock");
    // The watching-only branches, which no upstream test runs (spec 02
    // section 11): the private change is refused before the old
    // passphrase is checked, the public one is not.
    mgr.convert_to_watching_only(&store).expect("convert");
    test_change_passphrase(&store, &mgr, &chain, true);
    drop(mgr);
    let mgr = open(&store, &chain, PUB_PASSPHRASE);
    test_change_passphrase(&store, &mgr, &chain, true);
}

/// Upstream's first case: a public change whose new secret key cannot be
/// made fails and changes nothing. Upstream's injected constructor returns
/// kind `Crypto`; the Rust failure is the source's, which snacl reports
/// with kind `Other` under `snacl.NewSecretKey`, as for creation.
#[test]
fn invalid_new_secret_key_changes_nothing() {
    let (store, mgr, chain) = create("testnet3", PUB_PASSPHRASE, PRIV_PASSPHRASE);
    drop(mgr);
    let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mgr = {
        let tx = store.read().expect("read");
        Manager::open(
            &mut &tx,
            &chain,
            PUB_PASSPHRASE,
            switchable_source(fail.clone()),
            Limits::DEFAULT,
        )
        .expect("open")
    };
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    for private in [false, true] {
        let old = if private {
            PRIV_PASSPHRASE
        } else {
            PUB_PASSPHRASE
        };
        let err = mgr
            .change_passphrase(&store, old, b"new", private)
            .expect_err("a failing source");
        // Kind `Other`, which `errors.Is` never matches, as in Go.
        assert!(
            !err.is_kind(Kind::Crypto) && !err.is_kind(Kind::Passphrase),
            "{err}"
        );
        assert_eq!(err.to_string(), "snacl.NewSecretKey: injected failure");
    }
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(public_passphrase_opens(&store, &chain, PUB_PASSPHRASE));
    mgr.unlock(PRIV_PASSPHRASE)
        .expect("the private passphrase is unchanged");
}

/// A hash from dcrd's display form (`chainhash.NewHashFromStr`).
fn display_hash(s: &str) -> purse_store::Hash32 {
    let mut b: [u8; 32] = unhex(s).try_into().expect("32 bytes");
    b.reverse();
    purse_store::Hash32(b)
}

/// `vsp_test.go` `TestSetGetDeleteVSPTicket`.
#[test]
fn set_get_delete_vsp_ticket() {
    use purse_udb::vsp::{self, VspTicket};
    let dir = scratch("upstream-vsp");
    let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
    let ticket_hash =
        display_hash("0a472047bfcf1a19580412536570e351d2ac3f30638491c1c1268d6ada514858");
    let fee_hash = display_hash("679c508625503e5cd00ba8d35badc40ce040df8986c11321758db831b78f94ec");
    let mut ticket = VspTicket {
        fee_hash,
        fee_tx_status: 123,
        vsp_host_id: 321,
        host: b"example.com".to_vec(),
        pub_key: b"not-a-real-key".to_vec(),
    };
    let view = |f: &mut dyn FnMut(&mut purse_store::ReadTx)| {
        let mut tx = store.read().expect("read");
        f(&mut tx);
    };

    // Non-existent tickets should return an error.
    view(&mut |tx| {
        let err = vsp::get_vsp_ticket(tx, &ticket_hash).expect_err("missing");
        assert!(err.is_kind(Kind::NotExist), "{err}");
    });

    // Insert into DB.
    let id = store
        .update(|tx| Ok::<_, purse_store::Error>(vsp::set_vsp_ticket(tx, &ticket_hash, &ticket)))
        .expect("commit")
        .expect("Inserting VSP ticket");
    // Go writes the assigned host id into the record.
    ticket.vsp_host_id = id;
    assert_eq!(id, 1);

    // Retrieve from DB and check retrieved data matches inserted data.
    view(&mut |tx| {
        let got = vsp::get_vsp_ticket(tx, &ticket_hash).expect("Retrieving VSP ticket");
        assert_eq!(got, ticket, "Retrieved data does not match inserted data");
    });

    // Delete from DB.
    store
        .update(|tx| Ok::<_, purse_store::Error>(vsp::delete_vsp_ticket(tx, &ticket_hash)))
        .expect("commit")
        .expect("Deleting VSP ticket");

    // Deleted data should no longer be retrievable.
    view(&mut |tx| {
        let err = vsp::get_vsp_ticket(tx, &ticket_hash).expect_err("deleted");
        assert!(err.is_kind(Kind::NotExist), "{err}");
    });

    // Deleting non-existent ticket should not return an error.
    store
        .update(|tx| Ok::<_, purse_store::Error>(vsp::delete_vsp_ticket(tx, &ticket_hash)))
        .expect("commit")
        .expect("deleting a non-existent ticket");
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `vsp_test.go` `TestVSPSerializeVSPHost`, `TestVSPSerializeVSPPubKey`
/// and `TestVSPSerializeVSPTicket`, on the store's codecs of the same
/// records (the host is the value of `vsp_hosts`; the public key row drops
/// Go's length prefix; the ticket row holds neither host nor key).
#[test]
fn vsp_record_codecs_round_trip_upstreams_values() {
    use purse_store::{Codec, VspHost, VspPubKeyRow, VspTicketRow};
    for host in [&b"my host"[..], b""] {
        let after = VspHost::decode(&VspHost(host.to_vec()).encode()).expect("host");
        assert_eq!(after.0, host);
    }
    for (id, pk) in [
        (69, &b"my public key"[..]),
        (0, b"my public key"),
        (69, b""),
    ] {
        let before = VspPubKeyRow {
            host_id: id,
            pubkey: pk.to_vec(),
        };
        assert_eq!(
            VspPubKeyRow::decode(&before.encode()).expect("pubkey"),
            before
        );
    }
    let fee = display_hash("679c508625503e5cd00ba8d35badc40ce040df8986c11321758db831b78f94ec");
    for (fee_hash, fee_status, host_id) in [
        (fee, 123, 321),
        (purse_store::Hash32::default(), 123, 321),
        (fee, 0, 321),
        (fee, 123, 0),
    ] {
        let before = VspTicketRow {
            fee_hash,
            fee_status,
            host_id,
        };
        let bytes = before.encode();
        assert_eq!(bytes.len(), 40, "dcrwallet's 40-byte record");
        assert_eq!(VspTicketRow::decode(&bytes).expect("ticket"), before);
    }
}
