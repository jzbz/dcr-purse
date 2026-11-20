// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests of the address manager, over random seeds,
//! passphrases and operation sequences (`PURSE_TEST_SEED` moves them).
//!
//! - Creation and opening: account 0's keys are those `hd_keys_from_seed`
//!   gives under the legacy coin type, before and after a reopen; the
//!   parked SLIP0044 account 0 opens to the SLIP0044 account key; every
//!   other public passphrase fails to open with the bare `invalid
//!   passphrase`; a second creation in the same store is refused and
//!   leaves it unchanged.
//! - The lock follows a model of Go's state machine (spec 02 section 5.7)
//!   through random sequences of lock, unlock (right and wrong passphrase)
//!   and private key requests.
//! - The account key-material codec round-trips and refuses every
//!   truncation and every trailing length but the Argon2id parameters'.
//! - The imported-address codec round-trips, refuses every truncation and
//!   trailing byte, and never panics.
//! - The used and returned counters follow a model of Go's rules (spec 02
//!   section 5.19), written over 64-bit integers, through random marks.
//! - Recorded addresses: after random syncs and records, each account's
//!   address set and the whole address list are in the order of SHA-256 of
//!   the HASH160, every derived address is the child its row names, and the
//!   set holds exactly what was written.
//! - Memory follows the store: what the manager loads inside a write
//!   transaction that rolls back is not cached (QK-0733).
//! - The SLIP0044 upgrade leaves no address signed for with another
//!   address's key, across a restart (QK-0760, ADR-0012 Rule 7).
//! - `Unlock` names the lowest-numbered of two failing cached accounts
//!   (QK-0755, ADR-0012 Rule 3(a)).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

mod support;

use std::collections::BTreeSet;

use dcroxide_txscript::stdaddr;
use purse_errors::{AnyError, Kind};
use purse_snacl::{Argon2idParams, Limits, RandomSourceError};
use purse_store::Store;
use purse_store::wallet::Slip0044Account0;
use purse_testutil::SplitMix64;
use purse_udb::Manager;
use purse_udb::addrmgr::{
    AccountKeys, AccountKeysError, BoxedRandom, ChainParams, ExtendedKey, ImportedData,
    ImportedDataError, ManagedAddress, hd_keys_from_seed,
};
use sha2::{Digest, Sha256};
use support::scratch;

fn rng_source(rng: &mut SplitMix64) -> BoxedRandom {
    let mut r = SplitMix64(rng.next_u64());
    Box::new(move |buf: &mut [u8]| -> Result<(), RandomSourceError> {
        r.fill(buf);
        Ok(())
    })
}

fn bytes(rng: &mut SplitMix64, max: u64) -> Vec<u8> {
    let n = usize::try_from(rng.below(max + 1)).expect("small");
    rng.bytes(n)
}

fn cases() -> usize {
    std::env::var("PURSE_UDB_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
}

#[test]
fn create_open_and_reopen_give_the_seeds_keys() {
    let mut rng = SplitMix64::from_env("purse-udb-create-open");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    for case in 0..cases() {
        let seed_len = 16 + usize::try_from(rng.below(49)).expect("small");
        let seed = rng.bytes(seed_len);
        let pub_pass = bytes(&mut rng, 20);
        let mut priv_pass = bytes(&mut rng, 20);
        priv_pass.push(b'!');
        let Ok(want) = hd_keys_from_seed(&seed, &chain) else {
            continue; // an unusable seed: not a creation case
        };
        let dir = scratch("prop-create");
        let path = dir.join("wallet.redb");
        let store = Store::create(&path).expect("create store");
        let mut src = rng_source(&mut rng);
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &seed,
                    &pub_pass,
                    &priv_pass,
                    &mut *src,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        // A second creation is refused and writes nothing.
        let before = store
            .read()
            .expect("read")
            .len(purse_store::TableId::Wallet)
            .expect("len");
        let again: Result<(), purse_errors::AnyError> = store
            .update(|tx| {
                Ok::<_, purse_store::Error>(Manager::create(
                    tx,
                    &chain,
                    &seed,
                    &pub_pass,
                    &priv_pass,
                    &mut *src,
                    &Limits::DEFAULT,
                ))
            })
            .expect("commit");
        let err = again.expect_err("second creation");
        assert!(err.is_kind(Kind::Exist), "case {case}: {err}");
        assert_eq!(
            err.to_string(),
            "item already exists: address manager already exists"
        );
        let after = store
            .read()
            .expect("read")
            .len(purse_store::TableId::Wallet)
            .expect("len");
        assert_eq!(before, after);

        for round in 0..2 {
            let tx = store.read().expect("read");
            let mut r = &tx;
            let m = Manager::open(
                &mut r,
                &chain,
                &pub_pass,
                rng_source(&mut rng),
                Limits::DEFAULT,
            )
            .expect("open");
            assert_eq!(
                m.account_extended_pub_key(&mut r, 0)
                    .expect("xpub")
                    .string(),
                want.account0_legacy.neuter().string(),
                "case {case} round {round}"
            );
            m.unlock(&priv_pass).expect("unlock");
            assert_eq!(
                m.account_extended_priv_key(&mut r, 0)
                    .expect("xpriv")
                    .string(),
                want.account0_legacy.string(),
                "case {case} round {round}"
            );
            // The parked SLIP0044 account 0 holds the SLIP0044 keys.
            let parked = tx
                .wallet_row::<Slip0044Account0>()
                .expect("row")
                .expect("parked");
            assert_eq!(
                m.decrypt_public(&parked.enc_pub).expect("open").as_bytes(),
                want.account0_slip0044.neuter().string().as_bytes()
            );
            assert_eq!(
                m.decrypt_private(&parked.enc_priv)
                    .expect("open")
                    .as_bytes(),
                want.account0_slip0044.string().as_bytes()
            );
            // Another public passphrase opens nothing.
            let mut wrong = pub_pass.clone();
            wrong.push(b'x');
            let err = Manager::open(
                &mut r,
                &chain,
                &wrong,
                rng_source(&mut rng),
                Limits::DEFAULT,
            )
            .expect_err("wrong public passphrase");
            assert_eq!(err.to_string(), "invalid passphrase");
            assert!(err.is_kind(Kind::Passphrase));
            drop(m);
            drop(tx);
        }
        drop(store);
        let store = Store::open(&path).expect("reopen store");
        let tx = store.read().expect("read");
        let mut r = &tx;
        let m = Manager::open(
            &mut r,
            &chain,
            &pub_pass,
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open after the file is reopened");
        assert_eq!(
            m.account_extended_pub_key(&mut r, 0)
                .expect("xpub")
                .string(),
            want.account0_legacy.neuter().string()
        );
        drop(tx);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The model of the wallet lock (spec 02 section 5.7): unlocked or not,
/// and with which passphrase.
#[test]
fn the_lock_follows_gos_state_machine() {
    let mut rng = SplitMix64::from_env("purse-udb-lock-model");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    for case in 0..cases() {
        let seed = rng.bytes(32);
        let priv_pass = format!("pass-{case}").into_bytes();
        let dir = scratch("prop-lock");
        let store = Store::create(&dir.join("wallet.redb")).expect("create store");
        let mut src = rng_source(&mut rng);
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &seed,
                    b"public",
                    &priv_pass,
                    &mut *src,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        let tx = store.read().expect("read");
        let mut r = &tx;
        let m = Manager::open(
            &mut r,
            &chain,
            b"public",
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open");
        let mut unlocked = false;
        for step in 0..40 {
            let what = format!("case {case} step {step}");
            match rng.below(5) {
                0 => match m.lock() {
                    Ok(()) => {
                        assert!(unlocked, "{what}: locked a locked wallet");
                        unlocked = false;
                    }
                    Err(e) => {
                        assert!(!unlocked, "{what}");
                        assert_eq!(e.to_string(), "wallet locked", "{what}");
                    }
                },
                1 => {
                    m.unlock(&priv_pass).expect("the right passphrase");
                    unlocked = true;
                }
                2 => {
                    let e = m.unlock(b"wrong").expect_err("a wrong passphrase");
                    let want = if unlocked {
                        "invalid passphrase"
                    } else {
                        "secretkey.DeriveKey: invalid passphrase"
                    };
                    assert_eq!(e.to_string(), want, "{what}");
                    unlocked = false;
                }
                3 => {
                    let r2 = m.unlocked_with_passphrase(&priv_pass);
                    assert_eq!(r2.is_ok(), unlocked, "{what}");
                    if !unlocked {
                        assert!(r2.expect_err("locked").is_kind(Kind::Locked));
                    }
                }
                _ => {
                    let k = m.account_extended_priv_key(&mut r, 0);
                    assert_eq!(k.is_ok(), unlocked, "{what}");
                    if let Err(e) = k {
                        assert_eq!(
                            e.to_string(),
                            "wallet locked: unable to access account extended privkey",
                            "{what}"
                        );
                    }
                }
            }
            assert_eq!(m.is_locked(), !unlocked, "{what}");
        }
        drop(m);
        drop(tx);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn account_keys_codec_round_trips_and_refuses_malformed_values() {
    let mut rng = SplitMix64::from_env("purse-udb-account-keys");
    for _ in 0..2000 {
        let keys = AccountKeys {
            enc_pub: bytes(&mut rng, 200),
            enc_priv: bytes(&mut rng, 200),
            kdf: (rng.below(2) == 0).then(|| Argon2idParams {
                salt: rng.bytes(16).try_into().expect("16"),
                time: u32::try_from(rng.below(1 << 32)).expect("32 bits"),
                memory: u32::try_from(rng.below(1 << 32)).expect("32 bits"),
                threads: u8::try_from(rng.below(256)).expect("8 bits"),
            }),
        };
        let enc = keys.encode();
        assert_eq!(AccountKeys::decode(&enc), Ok(keys.clone()));
        // Every strict prefix that cuts a length field or a key is refused;
        // a prefix ending right after the private key is the same keys
        // without parameters.
        let bare = enc.len() - if keys.kdf.is_some() { 25 } else { 0 };
        for cut in 0..enc.len() {
            let r = AccountKeys::decode(&enc[..cut]);
            if cut == bare {
                assert_eq!(r.map(|k| k.kdf), Ok(None));
            } else if cut > bare {
                assert_eq!(r, Err(AccountKeysError::Trailing(cut - bare)));
            } else {
                assert!(
                    r.is_err(),
                    "a prefix of {cut} of {} bytes decoded",
                    enc.len()
                );
            }
        }
        let mut longer = enc.clone();
        longer.push(0);
        assert!(AccountKeys::decode(&longer).is_err());
    }
}

#[test]
fn account_keys_decoder_never_panics_on_random_input() {
    let mut rng = SplitMix64::from_env("purse-udb-account-keys-random");
    for _ in 0..20_000 {
        let b = bytes(&mut rng, 64);
        let _ = AccountKeys::decode(&b);
    }
}

#[test]
fn imported_data_codec_round_trips_and_refuses_malformed_values() {
    let mut rng = SplitMix64::from_env("purse-udb-imported-data");
    for _ in 0..cases() * 20 {
        let (a, b) = (bytes(&mut rng, 80), bytes(&mut rng, 80));
        let v = if rng.below(2) == 0 {
            ImportedData::Key {
                enc_pub: a,
                enc_priv: b,
            }
        } else {
            ImportedData::Script {
                enc_hash: a,
                script: b,
            }
        };
        let enc = v.encode();
        assert_eq!(ImportedData::decode(&enc), Ok(v));
        for cut in 0..enc.len() {
            let got = ImportedData::decode(&enc[..cut]);
            let want = if cut == 0 {
                ImportedDataError::Kind
            } else {
                ImportedDataError::Truncated
            };
            assert_eq!(got, Err(want), "cut at {cut}");
        }
        let mut longer = enc.clone();
        longer.extend_from_slice(&bytes(&mut rng, 5));
        if longer.len() > enc.len() {
            assert_eq!(
                ImportedData::decode(&longer),
                Err(ImportedDataError::Trailing(longer.len() - enc.len()))
            );
        }
        let mut other = enc;
        other[0] = [0u8, 3, 0xff][usize::try_from(rng.below(3)).expect("small")];
        assert_eq!(ImportedData::decode(&other), Err(ImportedDataError::Kind));
    }
    for _ in 0..cases() * 200 {
        let raw = bytes(&mut rng, 40);
        if let Ok(v) = ImportedData::decode(&raw) {
            assert_eq!(v.encode(), raw);
        }
    }
}

/// A seeded wallet on simnet in a file store, opened, with its private
/// passphrase.
fn wallet(rng: &mut SplitMix64, label: &str) -> (Store, Manager, ChainParams) {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    loop {
        let seed = rng.bytes(32);
        if hd_keys_from_seed(&seed, &chain).is_err() {
            continue;
        }
        let dir = scratch(label);
        let store = Store::create(&dir.join("wallet.redb")).expect("create store");
        let mut src = rng_source(rng);
        update(&store, |tx| {
            Manager::create(
                tx,
                &chain,
                &seed,
                b"public",
                b"private",
                &mut *src,
                &Limits::DEFAULT,
            )
        })
        .expect("create");
        let tx = store.read().expect("read");
        let mgr = Manager::open(
            &mut &tx,
            &chain,
            b"public",
            rng_source(rng),
            Limits::DEFAULT,
        )
        .expect("open");
        drop(tx);
        return (store, mgr, chain);
    }
}

/// Runs `f` in a write transaction, committed on success.
fn update<T>(
    store: &Store,
    f: impl FnOnce(&mut purse_store::WriteTx<'_>) -> Result<T, AnyError>,
) -> Result<T, AnyError> {
    let mut out = None;
    let r = store.update(|tx| match f(tx) {
        Ok(v) => {
            out = Some(v);
            Ok(())
        }
        Err(e) => Err(Rollback(e)),
    });
    match r {
        Ok(()) => Ok(out.expect("ran")),
        Err(Rollback(e)) => Err(e),
    }
}

/// The error that rolls a test's write transaction back.
struct Rollback(AnyError);

impl From<purse_store::Error> for Rollback {
    fn from(e: purse_store::Error) -> Self {
        Rollback(e.to_purse_error().into())
    }
}

#[test]
fn counters_follow_gos_rules() {
    /// Go's MarkUsedChildIndex and MarkReturnedChildIndex over u64, the
    /// "+1" comparisons taken modulo 2^32.
    fn plus1(v: u32) -> u64 {
        (u64::from(v) + 1) % (1 << 32)
    }
    let mut rng = SplitMix64::from_env("purse-udb-counters");
    let (store, mgr, _) = wallet(&mut rng, "prop-counters");
    // (used, returned) per account (0 and the reserved account) and branch.
    let mut model = [[(u32::MAX, u32::MAX); 2], [(0, 0); 2]];
    for _ in 0..cases() * 10 {
        let (slot, acct) = if rng.below(3) == 0 {
            (1, 0x7FFF_FFFF)
        } else {
            (0, 0)
        };
        let branch = u32::try_from(rng.below(3)).expect("small");
        let child = match rng.below(6) {
            0 => u32::MAX,
            1 => 0x8000_0000 | u32::try_from(rng.below(4)).expect("small"),
            _ => u32::try_from(rng.below(40)).expect("small"),
        };
        if rng.below(2) == 0 {
            let r = update(&store, |tx| {
                mgr.mark_used_child_index(tx, acct, branch, child)
            });
            if branch > 1 {
                assert_eq!(
                    r.expect_err("bad branch").to_string(),
                    format!("invalid operation: account branch {branch}")
                );
                continue;
            }
            r.expect("mark used");
            let (used, ret) = &mut model[slot][branch as usize];
            if plus1(child) > plus1(*used) {
                *used = child;
                if plus1(*ret) < plus1(child) {
                    *ret = child;
                }
            }
        } else {
            update(&store, |tx| {
                mgr.mark_returned_child_index(tx, acct, branch, child)
            })
            .expect("mark returned");
            let b = usize::from(branch == 1);
            let (_, ret) = &mut model[slot][b];
            if child > *ret || *ret == u32::MAX {
                *ret = child;
            }
        }
        let tx = store.read().expect("read");
        let p = mgr.account_properties(&mut &tx, 0).expect("props");
        assert_eq!(
            (
                (p.last_used_external_index, p.last_returned_external_index),
                (p.last_used_internal_index, p.last_returned_internal_index)
            ),
            (model[0][0], model[0][1]),
        );
        let row = tx
            .get::<purse_store::Accounts>(&purse_store::AccountKey(0x7FFF_FFFF))
            .expect("read")
            .expect("row");
        assert_eq!(
            [
                (row.last_used[0], row.last_returned[0]),
                (row.last_used[1], row.last_returned[1])
            ],
            model[1]
        );
    }
}

#[test]
fn recorded_addresses_are_listed_in_dcrwallets_order() {
    let mut rng = SplitMix64::from_env("purse-udb-addresses");
    for _ in 0..cases().div_ceil(4) {
        let (store, mgr, chain) = wallet(&mut rng, "prop-addrs");
        if rng.below(2) == 0 {
            mgr.unlock(b"private").expect("unlock");
        }
        let mut written: BTreeSet<(u32, u8, u32)> = BTreeSet::new();
        for _ in 0..1 + rng.below(6) {
            let branch = u32::try_from(rng.below(2)).expect("small");
            let index = u32::try_from(rng.below(12)).expect("small");
            if rng.below(3) == 0 {
                // A recorded address at a path, with the path's key.
                let tx = store.read().expect("read");
                let key = mgr
                    .account_branch_extended_pub_key(&mut &tx, 0, branch)
                    .expect("branch")
                    .child(index)
                    .expect("child");
                drop(tx);
                update(&store, |tx| {
                    mgr.record_derived_address(tx, 0, branch, index, key.serialized_pub_key())
                })
                .expect("record");
                written.insert((0, u8::from(branch == 1), index));
            } else {
                update(&store, |tx| {
                    mgr.sync_account_to_addr_index(tx, 0, index, branch)
                })
                .expect("sync");
                // Go writes down from index to the first recorded child.
                let mut c = index;
                loop {
                    if !written.insert((0, u8::from(branch == 1), c)) || c == 0 {
                        break;
                    }
                    c -= 1;
                }
            }
        }
        let tx = store.read().expect("read");
        let mut listed = Vec::new();
        mgr.for_each_account_address(&mut &tx, 0, |m| {
            listed.push(m);
            Ok(())
        })
        .expect("for each");
        let mut active = Vec::new();
        mgr.for_each_active_address(&mut &tx, |a| {
            active.push(a);
            Ok(())
        })
        .expect("for each active");
        let order = |m: &ManagedAddress| -> [u8; 32] { Sha256::digest(m.addr_hash()).into() };
        assert!(
            listed.windows(2).all(|w| order(&w[0]) < order(&w[1])),
            "sha order"
        );
        assert_eq!(
            active,
            listed
                .iter()
                .map(|m| m.address().clone())
                .collect::<Vec<_>>()
        );
        let got: BTreeSet<(u32, u8, u32)> = listed
            .iter()
            .map(|m| {
                (
                    m.account(),
                    u8::from(m.internal()),
                    m.index().expect("pubkey"),
                )
            })
            .collect();
        assert_eq!(got, written);
        for m in &listed {
            let b = u32::from(m.internal());
            let key: ExtendedKey = mgr
                .account_branch_extended_pub_key(&mut &tx, 0, b)
                .expect("branch")
                .child(m.index().expect("index"))
                .expect("child");
            assert_eq!(m.pub_key(), Some(key.serialized_pub_key()));
            let want = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                &stdaddr::hash160(key.serialized_pub_key()),
                &chain,
            )
            .expect("p2pkh");
            assert_eq!(m.address(), &want);
            assert_eq!(mgr.addr_account(&mut &tx, &want).expect("account"), 0);
        }
    }
}

#[test]
fn loads_in_a_rolled_back_transaction_are_not_cached() {
    let mut rng = SplitMix64::from_env("purse-udb-rollback");
    let (store, mgr, chain) = wallet(&mut rng, "prop-rollback");
    mgr.unlock(b"private").expect("unlock");
    // A new account, loaded inside its own transaction, which rolls back.
    let r = update(&store, |tx| {
        let n = mgr.new_account(tx, b"gone")?;
        mgr.account_extended_priv_key(tx, n)?;
        mgr.account_properties(tx, n)?;
        Err::<(), _>(AnyError::new("roll back"))
    });
    assert_eq!(r.expect_err("rolled back").to_string(), "roll back");
    let tx = store.read().expect("read");
    let err = mgr
        .account_extended_pub_key(&mut &tx, 1)
        .expect_err("account 1 was rolled back");
    assert_eq!(err.to_string(), "item does not exist: no account 1");
    drop(tx);
    // An imported xpub account rolled back, then another key imported at
    // the same number: its key, not the rolled-back one, is reported.
    let xpub = |n: u32| {
        ExtendedKey::new_master(&[7u8; 32], &chain.hd)
            .expect("master")
            .child(n | 0x8000_0000)
            .expect("child")
            .neuter()
    };
    let r = update(&store, |tx| {
        mgr.import_xpub_account(tx, b"x", &xpub(1))?;
        mgr.account_extended_pub_key(tx, 0x8000_0000)?;
        Err::<(), _>(AnyError::new("roll back"))
    });
    assert!(r.is_err());
    update(&store, |tx| mgr.import_xpub_account(tx, b"x", &xpub(2))).expect("import");
    let tx = store.read().expect("read");
    assert_eq!(
        mgr.account_extended_pub_key(&mut &tx, 0x8000_0000)
            .expect("xpub")
            .string()
            .as_str(),
        xpub(2).string().as_str()
    );
    // A renamed account: its name is read from the store, never cached.
    drop(tx);
    let r = update(&store, |tx| {
        mgr.rename_account(tx, 0, b"renamed")?;
        let p = mgr.account_properties(tx, 0)?;
        assert_eq!(p.account_name, b"renamed");
        Err::<(), _>(AnyError::new("roll back"))
    });
    assert!(r.is_err());
    let tx = store.read().expect("read");
    assert_eq!(
        mgr.account_properties(&mut &tx, 0)
            .expect("props")
            .account_name,
        b"default"
    );
    let err = mgr
        .lookup_account(&mut &tx, b"renamed")
        .expect_err("rolled back");
    assert!(err.is_kind(Kind::NotExist));
}

/// A seeded wallet on `net` in a file store, opened.
fn wallet_on(
    rng: &mut SplitMix64,
    label: &str,
    net: &str,
    seed: &[u8],
) -> Option<(Store, Manager, ChainParams)> {
    let chain = ChainParams::by_name(net).expect("network");
    hd_keys_from_seed(seed, &chain).ok()?;
    let dir = scratch(label);
    let store = Store::create(&dir.join("wallet.redb")).expect("create store");
    let mut src = rng_source(rng);
    update(&store, |tx| {
        Manager::create(
            tx,
            &chain,
            seed,
            b"public",
            b"private",
            &mut *src,
            &Limits::DEFAULT,
        )
    })
    .expect("create");
    let tx = store.read().expect("read");
    let mgr = Manager::open(
        &mut &tx,
        &chain,
        b"public",
        rng_source(rng),
        Limits::DEFAULT,
    )
    .expect("open");
    drop(tx);
    Some((store, mgr, chain))
}

/// Seeds whose purpose key or legacy coin type key `Child` stores
/// stripped of a leading zero byte (QK-0028) on `chain`.
fn stripped_seed(rng: &mut SplitMix64, chain: &ChainParams) -> Vec<u8> {
    loop {
        let seed = rng.bytes(32);
        let Ok(master) = ExtendedKey::new_master(&seed, &chain.hd) else {
            continue;
        };
        let Ok(p) = master.child(44 | 0x8000_0000) else {
            continue;
        };
        let Ok(c) = p.child(chain.legacy_coin_type | 0x8000_0000) else {
            continue;
        };
        if p.serialized_priv_key().expect("private").len() < 32
            || c.serialized_priv_key().expect("private").len() < 32
        {
            return seed;
        }
    }
}

/// A private key's scalar as 32 bytes (a stripped key is left-padded).
fn scalar(k: &ExtendedKey) -> [u8; 32] {
    let b = k.serialized_priv_key().expect("private");
    let mut out = [0u8; 32];
    out[32 - b.len()..].copy_from_slice(b);
    out
}

/// Derivation agrees with purse-hdkeychain on all four networks and both
/// coin types: account keys, the coin type key, recorded addresses, their
/// public keys and private keys are those `ExtendedKey` derives from the
/// seed by BIP0044 paths, account 0 from the master key in memory and
/// every later account from the coin type key parsed from its string, as
/// dcrwallet derives them (QK-0028). Seeds with a stripped purpose or coin
/// type key are included on simnet.
#[test]
fn derivation_agrees_with_purse_hdkeychain() {
    let mut rng = SplitMix64::from_env("purse-udb-derivation");
    let simnet = ChainParams::by_name("simnet").expect("simnet");
    let mut plan: Vec<(&str, Vec<u8>)> = Vec::new();
    for _ in 0..cases().div_ceil(2) {
        let len = 16 + usize::try_from(rng.below(49)).expect("small");
        plan.push(("simnet", rng.bytes(len)));
    }
    for _ in 0..2 {
        plan.push(("simnet", stripped_seed(&mut rng, &simnet)));
    }
    for net in ["regnet", "testnet3", "mainnet"] {
        plan.push((net, rng.bytes(32)));
    }
    let h = 0x8000_0000u32;
    for (case, (net, seed)) in plan.into_iter().enumerate() {
        let Some((store, mgr, chain)) = wallet_on(&mut rng, "prop-derive", net, &seed) else {
            continue;
        };
        let what = format!("case {case} ({net})");
        let upgraded = rng.below(2) == 0;
        if upgraded {
            update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        }
        let unlocked = rng.below(3) != 0;
        let accounts = 1 + u32::try_from(rng.below(3)).expect("small");
        mgr.unlock(b"private").expect("unlock");
        for n in 1..accounts {
            let got = update(&store, |tx| mgr.new_account(tx, format!("a{n}").as_bytes()))
                .expect("new account");
            assert_eq!(got, n);
        }
        if !unlocked {
            mgr.lock().expect("lock");
        }
        let (legacy, slip) = (chain.legacy_coin_type, chain.slip0044_coin_type);
        let coin = if upgraded { slip } else { legacy };
        let master = ExtendedKey::new_master(&seed, &chain.hd).expect("master");
        let ct = master
            .child(44 | h)
            .expect("purpose")
            .child(coin | h)
            .expect("coin type");
        let ct_parsed =
            ExtendedKey::new_key_from_string(ct.string().as_str(), &chain.hd).expect("parse");
        let tx = store.read().expect("read");
        assert_eq!(mgr.coin_type(&mut &tx).expect("coin type"), coin, "{what}");
        if unlocked {
            assert_eq!(
                mgr.coin_type_priv_key(&mut &tx)
                    .expect("coin type key")
                    .string(),
                ct.string(),
                "{what}"
            );
        }
        drop(tx);
        for n in 0..accounts {
            let acct = if n == 0 { &ct } else { &ct_parsed }
                .child(n | h)
                .expect("account");
            let tx = store.read().expect("read");
            assert_eq!(
                mgr.account_extended_pub_key(&mut &tx, n)
                    .expect("xpub")
                    .string(),
                acct.neuter().string(),
                "{what}: account {n}"
            );
            if unlocked {
                assert_eq!(
                    mgr.account_extended_priv_key(&mut &tx, n)
                        .expect("xpriv")
                        .string(),
                    acct.string(),
                    "{what}: account {n}"
                );
            }
            drop(tx);
            let branch = u32::try_from(rng.below(2)).expect("small");
            let index = u32::try_from(rng.below(6)).expect("small");
            update(&store, |tx| {
                mgr.sync_account_to_addr_index(tx, n, index, branch)
            })
            .expect("sync");
            let tx = store.read().expect("read");
            let mut listed = Vec::new();
            mgr.for_each_account_address(&mut &tx, n, |m| {
                listed.push(m);
                Ok(())
            })
            .expect("for each");
            assert_eq!(listed.len(), usize::try_from(index + 1).expect("small"));
            for m in listed {
                let i = m.index().expect("derived");
                let child = acct.child(branch).expect("branch").child(i).expect("child");
                let want = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                    &stdaddr::hash160(child.serialized_pub_key()),
                    &chain,
                )
                .expect("p2pkh");
                assert_eq!(m.address(), &want, "{what}: {n}/{branch}/{i}");
                assert_eq!(m.pub_key(), Some(child.serialized_pub_key()));
                let found = mgr.address(&mut &tx, &want).expect("address");
                assert_eq!(found.pub_key(), Some(child.serialized_pub_key()));
                let k = mgr.private_key(&mut &tx, &want);
                if unlocked {
                    let k = k.expect("private key");
                    assert_eq!(&k.serialize()[..], &scalar(&child)[..], "{what}");
                } else {
                    assert!(k.expect_err("locked").is_kind(Kind::Locked), "{what}");
                }
            }
        }
    }
}

/// The read side of the manager after a random generated script: what the
/// live manager answers equals what a manager opened again on the same
/// store answers (unlocked again when the live one was unlocked), and the
/// two runs' final exports are byte-identical. Memory holds nothing the
/// store does not.
#[test]
fn reopen_equals_in_memory_state() {
    use purse_testutil::serde_json::{Value, json};
    use support::generator::{Shape, generate};
    use support::runner;

    let mut rng = SplitMix64::from_env("purse-udb-reopen");
    let accounts = [
        0u32,
        1,
        2,
        3,
        0x7FFF_FFFF,
        0x8000_0000,
        0x8000_0001,
        0x8000_0002,
    ];
    let mut sweep = vec![
        json!({"op": "mgr.is_locked"}),
        json!({"op": "mgr.coin_type"}),
        json!({"op": "mgr.coin_type_priv_key"}),
        json!({"op": "mgr.last_account"}),
        json!({"op": "mgr.last_imported_account"}),
        json!({"op": "mgr.for_each_account"}),
        json!({"op": "mgr.for_each_active_address"}),
    ];
    for a in accounts {
        for op in [
            "mgr.account_name",
            "mgr.account_properties",
            "mgr.account_has_passphrase",
            "mgr.account_extended_pub_key",
            "mgr.account_extended_priv_key",
            "mgr.for_each_account_address",
            "mgr.for_each_active_account_address",
        ] {
            sweep.push(json!({"op": op, "account": a}));
        }
        for (b, i) in [(0, 0), (1, 3)] {
            sweep.push(json!({"op": "mgr.address", "addr": {"acct": a, "branch": b, "index": i}}));
            sweep.push(
                json!({"op": "mgr.private_key", "addr": {"acct": a, "branch": b, "index": i}}),
            );
        }
    }
    let strip = |r: &[Value]| -> Vec<Value> {
        r.iter()
            .map(|v| {
                let mut v = v.clone();
                v.as_object_mut().expect("record").remove("i");
                v
            })
            .collect()
    };
    let shape = Shape {
        network: Some("simnet"),
        ..Shape::default()
    };
    // (unlocked at the end, upgraded to SLIP0044) -> cases.
    let mut seen = std::collections::BTreeMap::new();
    for case in 0..cases() {
        let script = generate(&mut rng, case, shape);
        let base = script["ops"].as_array().expect("ops").clone();
        let mut live = script.clone();
        live["ops"] = Value::Array(base.iter().chain(&sweep).cloned().collect());
        let dir = scratch("prop-reopen");
        let a = runner::run(&live, &dir.join("live")).expect("live run");
        let tail_a = &a.results[base.len()..];
        let unlocked = tail_a[0]["ok"] == json!(false);
        let upgraded = tail_a[1]["ok"] == json!(1);
        *seen.entry((unlocked, upgraded)).or_insert(0usize) += 1;
        let mut ops = base.clone();
        ops.push(json!({"op": "rt.reopen"}));
        if unlocked {
            ops.push(json!({"op": "mgr.unlock", "pass": script["setup"]["privpass"]}));
        }
        let skip = ops.len();
        ops.extend(sweep.iter().cloned());
        let mut reopened = script.clone();
        reopened["ops"] = Value::Array(ops);
        let b = runner::run(&reopened, &dir.join("reopened")).expect("reopened run");
        assert_eq!(
            strip(&a.results[..base.len()]),
            strip(&b.results[..base.len()]),
            "case {case}: the script itself"
        );
        assert_eq!(
            b.results[base.len()]["ok"],
            Value::Null,
            "case {case}: reopen"
        );
        if unlocked {
            assert_eq!(
                b.results[base.len() + 1]["ok"],
                Value::Null,
                "case {case}: unlock"
            );
        }
        assert_eq!(
            strip(tail_a),
            strip(&b.results[skip..]),
            "case {case}: {script}"
        );
        assert_eq!(a.export, b.export, "case {case}: exports");
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!("(unlocked, upgraded) -> cases: {seen:?}");
    if cases() >= 24 {
        assert_eq!(seen.len(), 4, "every lock state and coin type: {seen:?}");
    }
}

/// The coin type upgrade replaces account 0's keys, the one change to
/// cached key material: an upgrade that rolls back leaves the legacy keys
/// in use, and a snapshot taken before an upgrade commits, read after it,
/// sees the legacy keys without putting them back in the cache.
#[test]
fn coin_type_upgrade_keeps_memory_with_the_store() {
    let mut rng = SplitMix64::from_env("purse-udb-upgrade-cache");
    for case in 0..cases().div_ceil(4) {
        let (store, mgr, chain) = wallet(&mut rng, "prop-upgrade");
        if rng.below(2) == 0 {
            mgr.unlock(b"private").expect("unlock");
        }
        let unlocked = !mgr.is_locked();
        let tx = store.read().expect("read");
        let legacy = mgr
            .account_extended_pub_key(&mut &tx, 0)
            .expect("xpub")
            .string();
        drop(tx);
        // Rolled back: nothing changes.
        let r = update(&store, |tx| {
            mgr.upgrade_to_slip0044_coin_type(tx)?;
            assert_ne!(
                mgr.account_extended_pub_key(tx, 0)?.string(),
                legacy,
                "case {case}: inside the transaction"
            );
            Err::<(), _>(AnyError::new("roll back"))
        });
        assert!(r.is_err());
        let tx = store.read().expect("read");
        assert_eq!(
            mgr.account_extended_pub_key(&mut &tx, 0)
                .expect("xpub")
                .string(),
            legacy,
            "case {case}: after the rollback"
        );
        assert_eq!(
            mgr.coin_type(&mut &tx).expect("coin type"),
            chain.legacy_coin_type
        );
        drop(tx);
        // Committed, with a snapshot from before the commit still open.
        let old = store.read().expect("read");
        update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        assert_eq!(
            mgr.account_extended_pub_key(&mut &old, 0)
                .expect("xpub")
                .string(),
            legacy,
            "case {case}: the old snapshot"
        );
        drop(old);
        let tx = store.read().expect("read");
        let slip = mgr
            .account_extended_pub_key(&mut &tx, 0)
            .expect("xpub")
            .string();
        assert_ne!(slip, legacy, "case {case}: after the commit");
        // A manager opened afresh on the store reads the same key.
        let fresh = Manager::open(
            &mut &tx,
            &chain,
            b"public",
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open");
        assert_eq!(
            fresh
                .account_extended_pub_key(&mut &tx, 0)
                .expect("xpub")
                .string(),
            slip,
            "case {case}"
        );
        if unlocked {
            assert_eq!(
                mgr.account_extended_priv_key(&mut &tx, 0)
                    .expect("xpriv")
                    .neuter()
                    .string(),
                slip
            );
        }
    }
}

/// QK-0760 (ADR-0012 Rule 7): after the SLIP0044 upgrade of a wallet
/// whose account 0 recorded addresses under the legacy coin type, and a
/// restart, every address the wallet claims for account 0 is signed for
/// with its own key; the legacy addresses are no addresses of the wallet,
/// and the SLIP0044 children at the recorded paths are. An entry another
/// account's set held for a replaced row (QK-0757) is removed with it, so
/// that account's walk still succeeds; a row at a hardened child is
/// removed; a SLIP0044 child already recorded as an imported key keeps
/// its imported row. Go's answer, a legacy address
/// claimed and given another address's key, is pinned by
/// `coin_type_upgrade_rederives_legacy_rows` in `tests/oracle.rs`.
///
/// The test also pins the case the fix leaves (QK-0760, residual case,
/// open for the owner): no legacy address is recognised after the
/// upgrade, so a later payment to one is neither recognised nor
/// spendable until the seed is restored again, as in Go, whose filter
/// misses it too. A decision to keep the legacy keys and rows changes
/// the `NotExist` assertions below.
#[test]
fn coin_type_upgrade_leaves_no_address_with_another_key() {
    let mut rng = SplitMix64::from_env("purse-udb-upgrade-rows");
    for case in 0..cases().div_ceil(4) {
        let (store, mgr, chain) = wallet(&mut rng, "prop-upgrade-rows");
        mgr.unlock(b"private").expect("unlock");
        let p2pkh = |k: &ExtendedKey| {
            stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                &stdaddr::hash160(k.serialized_pub_key()),
                &chain,
            )
            .expect("p2pkh")
        };
        let children = |m: &Manager, tx: &purse_store::ReadTx, branch: u32, upto: u32| {
            let b = m
                .account_branch_extended_pub_key(&mut &*tx, 0, branch)
                .expect("branch");
            (0..=upto)
                .map(|i| p2pkh(&b.child(i).expect("child")))
                .collect::<Vec<_>>()
        };
        let (ext, int) = (
            u32::try_from(rng.below(8)).expect("small"),
            u32::try_from(rng.below(4)).expect("small"),
        );
        let tx = store.read().expect("read");
        let legacy: Vec<_> = children(&mgr, &tx, 0, ext)
            .into_iter()
            .chain(children(&mgr, &tx, 1, int))
            .collect();
        drop(tx);
        update(&store, |tx| {
            mgr.sync_account_to_addr_index(tx, 0, ext, 0)?;
            mgr.sync_account_to_addr_index(tx, 0, int, 1)
        })
        .expect("sync");
        // An imported key recorded again as account 0's child 50: its
        // entry stays in the imported account's set (QK-0757).
        let shadowed = rng.below(2) == 0;
        if shadowed {
            let mut k = [0u8; 32];
            k[31] = 1 + u8::try_from(rng.below(200)).expect("small");
            let wif = purse_udb::addrmgr::Wif::new(
                &k,
                chain.ids.private_key,
                purse_dcrutil::SignatureType::ECDSA_SECP256K1,
            )
            .expect("wif");
            update(&store, |tx| {
                let m = mgr.import_private_key(tx, &wif)?;
                let pk = m.pub_key().expect("pubkey").to_vec();
                mgr.record_derived_address(tx, 0, 0, 50, &pk)
            })
            .expect("import and record");
        }
        // A row at a hardened child, which the new public key cannot
        // derive: removed, not replaced.
        let tx = store.read().expect("read");
        let hardened_pk = mgr
            .account_branch_extended_pub_key(&mut &tx, 0, 1)
            .expect("branch")
            .child(99)
            .expect("child")
            .serialized_pub_key()
            .to_vec();
        drop(tx);
        update(&store, |tx| {
            mgr.record_derived_address(tx, 0, 1, 0x8000_0002, &hardened_pk)
        })
        .expect("record hardened");
        // The SLIP0044 external child 0's private key imported before the
        // upgrade (the key read in an upgrade that rolls back): that row is
        // kept, and the child is not recorded over it.
        let pre_imported = rng.below(2) == 0;
        if pre_imported {
            let r = update(&store, |tx| {
                mgr.upgrade_to_slip0044_coin_type(tx)?;
                let k = mgr
                    .account_extended_priv_key(tx, 0)?
                    .child(0)
                    .and_then(|b| b.child(0))
                    .map_err(AnyError::foreign)?;
                let hex = purse_testutil::hex(k.serialized_priv_key().map_err(AnyError::foreign)?);
                Err::<(), _>(AnyError::new(hex))
            });
            let mut key = [0u8; 32];
            let raw = purse_testutil::unhex(&r.expect_err("rolled back").to_string());
            key[32 - raw.len()..].copy_from_slice(&raw);
            let wif = purse_udb::addrmgr::Wif::new(
                &key,
                chain.ids.private_key,
                purse_dcrutil::SignatureType::ECDSA_SECP256K1,
            )
            .expect("wif");
            update(&store, |tx| mgr.import_private_key(tx, &wif)).expect("import key");
        }
        update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        drop(mgr);

        let tx = store.read().expect("read");
        let mgr = Manager::open(
            &mut &tx,
            &chain,
            b"public",
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("reopen");
        mgr.unlock(b"private").expect("unlock");
        for a in &legacy {
            let err = mgr.address(&mut &tx, a).expect_err("legacy address");
            assert!(err.is_kind(Kind::NotExist), "case {case}: {err}");
            assert!(
                !mgr.exists_hash160(&mut &tx, &a.hash160().expect("p2pkh")[..])
                    .expect("exists"),
                "case {case}"
            );
        }
        let mut want: BTreeSet<String> = children(&mgr, &tx, 0, ext)
            .into_iter()
            .chain(children(&mgr, &tx, 1, int))
            .map(|a| a.encode())
            .collect();
        let b0 = mgr
            .account_branch_extended_pub_key(&mut &tx, 0, 0)
            .expect("branch");
        if shadowed {
            want.insert(p2pkh(&b0.child(50).expect("child")).encode());
        }
        assert!(
            !mgr.exists_hash160(&mut &tx, &stdaddr::hash160(&hardened_pk))
                .expect("exists"),
            "case {case}: the hardened row is removed"
        );
        if pre_imported {
            let slip0 = p2pkh(&b0.child(0).expect("child"));
            assert!(want.remove(&slip0.encode()));
            let m = mgr.address(&mut &tx, &slip0).expect("imported");
            assert!(m.imported(), "case {case}: the imported row is kept");
        }
        let mut listed = BTreeSet::new();
        mgr.for_each_account_address(&mut &tx, 0, |m| {
            listed.insert(m.address().encode());
            Ok(())
        })
        .expect("walk account 0");
        assert_eq!(listed, want, "case {case}");
        for s in &want {
            let a = stdaddr::decode_address(s, &chain).expect("decode");
            let m = mgr.address(&mut &tx, &a).expect("recorded");
            assert_eq!(m.address(), &a, "case {case}");
            let k = mgr.private_key(&mut &tx, &a).expect("private key");
            assert_eq!(
                stdaddr::hash160(&k.public_point().serialize_compressed()),
                m.addr_hash(),
                "case {case}: {s} is signed for with its own key"
            );
        }
        let mut imported = 0;
        mgr.for_each_account_address(&mut &tx, 0x7FFF_FFFF, |_| {
            imported += 1;
            Ok(())
        })
        .expect("walk the imported account");
        assert_eq!(imported, usize::from(pre_imported), "case {case}");
    }
}

/// QK-0755 (ADR-0012 Rule 3(a)): with two cached accounts whose sealed
/// private keys do not open, `Unlock` names the lower account number, and
/// leaves the wallet locked. Accounts 1 and 256 are chosen so that the
/// store's little-endian key order (256 first) and the numeric order
/// differ. Go ranges over a map, so its choice is not fixed; no input at
/// the pin reaches two failing accounts, so there is no oracle run.
#[test]
fn unlock_names_the_lowest_failing_account() {
    let mut rng = SplitMix64::from_env("purse-udb-unlock-order");
    let (store, mgr, _chain) = wallet(&mut rng, "prop-unlock-order");
    update(&store, |tx| {
        let mut row = tx
            .get::<purse_store::Accounts>(&purse_store::AccountKey(0))
            .map_err(|e| AnyError::from(e.to_purse_error()))?
            .expect("account 0");
        let mut keys = AccountKeys::decode(&row.keys).expect("keys");
        // A sealed box whose tag no longer matches.
        let last = keys.enc_priv.last_mut().expect("sealed");
        *last ^= 1;
        row.keys = keys.encode();
        for (n, name) in [(256u32, &b"two-five-six"[..]), (1, b"one")] {
            row.name = name.to_vec();
            tx.put::<purse_store::Accounts>(&purse_store::AccountKey(n), &row)
                .map_err(|e| AnyError::from(e.to_purse_error()))?;
        }
        Ok(())
    })
    .expect("write accounts");
    // Cache both through a snapshot, 256 first.
    let tx = store.read().expect("read");
    for n in [256, 1] {
        mgr.account_extended_pub_key(&mut &tx, n).expect("xpub");
    }
    drop(tx);
    let err = mgr.unlock(b"private").expect_err("unlock");
    assert!(err.is_kind(Kind::Crypto), "{err}");
    assert!(
        err.to_string().contains("decrypt account 1 privkey: "),
        "{err}"
    );
    assert!(mgr.is_locked());
}
