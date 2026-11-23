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
//! - Per-account passphrases follow a model of Go's rules through random
//!   sets, changes, removals, unlocks, locks, wallet locks and reopens on
//!   BIP0044 and voting accounts; rolled-back changes leave store and
//!   memory as they were, and a snapshot from before a change does not put
//!   the old key material back in the cache (QK-0733). Voting account
//!   numbers never wrap onto account 0 (QK-0041).
//! - Passphrase changes follow a model of Go's rules through random changes
//!   of either passphrase (right and wrong old passphrases, empty and
//!   repeated new ones), locks, unlocks and reopens: the passphrases the
//!   wallet accepts are the model's, the lock state is kept, every key the
//!   crypto keys seal opens as before, a failed change leaves the store as
//!   it was, and a successful one rewrites only its half's two rows; an
//!   account its own passphrase unlocked stays unlocked across a private
//!   change. A refused change, like a refused watching-only creation from
//!   an extended private key, draws nothing from the random source.
//! - Conversions to watching-only: no value the conversion removes is left
//!   in any row or in a backup of the converted store (the live file may
//!   keep earlier copies in freed pages, QK-0761), and an account row whose
//!   key material does not decode is left as Go leaves it.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

mod support;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// [`rng_source`], adding the bytes it hands out to `drawn`, so a test can
/// show that an operation drew nothing, or exactly what it should.
fn counted_source(rng: &mut SplitMix64, drawn: &Arc<AtomicU64>) -> BoxedRandom {
    let mut r = SplitMix64(rng.next_u64());
    let drawn = Arc::clone(drawn);
    Box::new(move |buf: &mut [u8]| -> Result<(), RandomSourceError> {
        r.fill(buf);
        drawn.fetch_add(u64::try_from(buf.len()).expect("small"), Ordering::Relaxed);
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
        let path = dir.join(purse_store::STORE_FILE_NAME);
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
        let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
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
    let (_, store, mgr, chain) = wallet_file(rng, label);
    (store, mgr, chain)
}

/// [`wallet`], with the path of its store file.
fn wallet_file(
    rng: &mut SplitMix64,
    label: &str,
) -> (std::path::PathBuf, Store, Manager, ChainParams) {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    loop {
        let seed = rng.bytes(32);
        if hd_keys_from_seed(&seed, &chain).is_err() {
            continue;
        }
        let dir = scratch(label);
        let path = dir.join(purse_store::STORE_FILE_NAME);
        let store = Store::create(&path).expect("create store");
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
        return (path, store, mgr, chain);
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
    let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
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

/// QK-0767 (ADR-0012 Rule 7; the report decided by the project owner on
/// 2026-11-22): over random wallets whose account 0 has no passphrase of its
/// own, has one, had one removed, or had one set in a transaction that
/// rolled back, the SLIP0044 upgrade returns `true` exactly when account 0
/// had its own passphrase, in a transaction that rolls back as in one that
/// commits. A rolled-back upgrade leaves the passphrase as it was. After a
/// committed one account 0 has no passphrase of its own, live and after a
/// reopen, and the wallet passphrase opens its key; a second upgrade is
/// refused and writes nothing.
#[test]
fn coin_type_upgrade_reports_a_dropped_passphrase() {
    let mut rng = SplitMix64::from_env("purse-udb-upgrade-report");
    let mut seen = BTreeSet::new();
    for case in 0..cases().div_ceil(2).max(8) {
        let (store, mgr, chain) = wallet(&mut rng, "prop-upgrade-report");
        mgr.unlock(b"private").expect("unlock");
        let state = case % 4;
        match state {
            1 => {
                update(&store, |tx| mgr.set_account_passphrase(tx, 0, b"zero")).expect("set");
            }
            2 => {
                update(&store, |tx| mgr.set_account_passphrase(tx, 0, b"zero")).expect("set");
                update(&store, |tx| mgr.set_account_passphrase(tx, 0, b"")).expect("remove");
            }
            3 => {
                let r = update(&store, |tx| {
                    mgr.set_account_passphrase(tx, 0, b"zero")?;
                    Err::<(), _>(AnyError::new("roll back"))
                });
                assert!(r.is_err());
            }
            _ => {}
        }
        if (case / 4) % 2 == 1 {
            mgr.lock().expect("lock");
        }
        let had = state == 1;
        seen.insert((had, mgr.is_locked()));
        let what = format!("case {case} (state {state}, locked {})", mgr.is_locked());
        let has = |m: &Manager| {
            let tx = store.read().expect("read");
            m.account_has_passphrase(&mut &tx, 0).0
        };
        assert_eq!(has(&mgr), had, "{what}");
        // Rolled back: the report is the same, and nothing changes.
        let r = update(&store, |tx| {
            let dropped = mgr.upgrade_to_slip0044_coin_type(tx)?;
            assert_eq!(dropped, had, "{what}: inside a transaction that rolls back");
            Err::<(), _>(AnyError::new("roll back"))
        });
        assert!(r.is_err());
        assert_eq!(has(&mgr), had, "{what}: after the rollback");
        let dropped = update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        assert_eq!(dropped, had, "{what}");
        assert!(!has(&mgr), "{what}: after the upgrade");
        // A second upgrade is refused and writes nothing.
        let before = change_rows(&store);
        let err = update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx))
            .expect_err("a second upgrade");
        assert!(err.is_kind(Kind::Invalid), "{what}: {err}");
        assert_eq!(
            change_rows(&store),
            before,
            "{what}: a refused upgrade wrote"
        );
        drop(mgr);
        let tx = store.read().expect("read");
        let reopened = Manager::open(
            &mut &tx,
            &chain,
            b"public",
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open");
        drop(tx);
        assert!(!has(&reopened), "{what}: after a reopen");
        reopened.unlock(b"private").expect("unlock");
        let tx = store.read().expect("read");
        reopened
            .account_extended_priv_key(&mut &tx, 0)
            .unwrap_or_else(|e| panic!("{what}: the wallet passphrase opens account 0: {e}"));
    }
    assert_eq!(seen.len(), 4, "reports and lock states: {seen:?}");
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
/// deferred to Phase 5 by the project owner on 2026-11-22): no legacy
/// address is recognised after the
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

/// Opens a value of `store` sealed by the private crypto key, deriving the
/// private master key from `pass` as the store's parameters say (what a
/// holder of the file and the passphrase can do).
fn open_with_private_crypto_key(store: &Store, pass: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    use purse_snacl::{CryptoKey, Parameters, SecretKey};
    use purse_store::wallet::{CryptoKeyPriv, MasterKeyPriv};
    let tx = store.read().expect("read");
    let p = tx.wallet_row::<MasterKeyPriv>().expect("read")?;
    let sealed_key = tx.wallet_row::<CryptoKeyPriv>().expect("read")?;
    let mut master = SecretKey::from_parameters(Parameters {
        salt: p.salt,
        digest: p.digest,
        n: p.n.cast_signed(),
        r: p.r.cast_signed(),
        p: p.p.cast_signed(),
    });
    master.derive_key_with(pass, &Limits::DEFAULT).ok()?;
    let plain = master.decrypt(&sealed_key.0).ok()?;
    let key: [u8; 32] = plain.as_bytes().try_into().ok()?;
    let crypto = CryptoKey::from_bytes(&key);
    crypto.decrypt(sealed).ok().map(|b| b.as_bytes().to_vec())
}

/// A wallet created from an account's extended public key
/// (`create_watch_only`) answers every public question about that account
/// as the seed's wallet does, on all four networks and both coin types:
/// the account and branch keys, the recorded addresses and their order,
/// lookups and `HavePrivateKey` (true for a derived address, QK-0037),
/// before and after a reopen; it never gives a private key. Its stored
/// private master key derives from the public passphrase and opens account
/// 0's private field to the account key's own string (QK-0037, reproduced),
/// and a public passphrase change leaves it so: the creation's passphrase
/// still opens that field, the new one does not. The account's extended
/// private key is refused, and the refusal writes nothing and draws
/// nothing from the random source, where the creation from the public key
/// draws in Go's sizes (QK-0762).
#[test]
fn watching_only_wallet_answers_as_the_seed_wallet() {
    use purse_store::{AccountKey, Accounts};
    let mut rng = SplitMix64::from_env("purse-udb-watching-xpub");
    let mut plan: Vec<&str> = vec!["regnet", "testnet3", "mainnet"];
    plan.extend(std::iter::repeat_n("simnet", cases()));
    let mut seen = BTreeSet::new();
    for (case, net) in plan.into_iter().enumerate() {
        let len = 16 + usize::try_from(rng.below(49)).expect("small");
        let seed = rng.bytes(len);
        let Some((store, mgr, chain)) = wallet_on(&mut rng, "prop-wo-seed", net, &seed) else {
            continue;
        };
        let slip = rng.below(2) == 0;
        if slip {
            update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        }
        seen.insert((net, slip));
        let what = format!("case {case} ({net}, SLIP0044 {slip})");
        let keys = hd_keys_from_seed(&seed, &chain).expect("seed");
        let acct = if slip {
            &keys.account0_slip0044
        } else {
            &keys.account0_legacy
        };
        let xpub = acct.neuter().string();
        let pub_pass = bytes(&mut rng, 12);
        let dir = scratch("prop-wo");
        let wstore = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
        let mut src = rng_source(&mut rng);
        // QK-0762 (ADR-0012 Rules 6 and 11, decided 2026-11-22): the account's
        // extended private key is refused, and the refusal writes nothing,
        // even in a transaction that then commits.
        // It also comes before any draw from the random source.
        let mut drawn = 0usize;
        let mut counting = |buf: &mut [u8]| -> Result<(), RandomSourceError> {
            drawn += buf.len();
            src.fill(buf)
        };
        let refused = wstore
            .update(|tx| -> Result<_, purse_store::Error> {
                Ok(Manager::create_watch_only(
                    tx,
                    &chain,
                    acct.string().as_bytes(),
                    &pub_pass,
                    &mut counting,
                    &Limits::DEFAULT,
                ))
            })
            .expect("commit")
            .expect_err("an extended private key");
        assert_eq!(drawn, 0, "{what}: the refusal drew");
        assert!(refused.is_kind(Kind::Invalid), "{what}: {refused}");
        assert_eq!(
            refused.to_string(),
            "invalid operation: extended key must be an xpub"
        );
        assert_eq!(
            wstore
                .read()
                .expect("read")
                .iter::<Accounts>()
                .expect("accounts")
                .count(),
            0,
            "{what}: the refusal wrote an account"
        );
        assert_eq!(
            change_rows(&wstore),
            ChangeRows {
                private: (None, None),
                public: (None, None),
                rest: vec!["None".to_owned(); 6],
            },
            "{what}: the refusal wrote a row"
        );
        // The creation from the public key draws in Go's sizes: two salts
        // and two crypto keys of 32 bytes, four nonces of 24.
        let mut drawn = 0usize;
        let mut counting = |buf: &mut [u8]| -> Result<(), RandomSourceError> {
            drawn += buf.len();
            src.fill(buf)
        };
        update(&wstore, |tx| {
            Manager::create_watch_only(
                tx,
                &chain,
                xpub.as_bytes(),
                &pub_pass,
                &mut counting,
                &Limits::DEFAULT,
            )
        })
        .expect("create_watch_only");
        assert_eq!(drawn, 4 * 32 + 4 * 24, "{what}: the creation's draws");
        // A second creation of either kind is refused and changes nothing.
        let err = update(&wstore, |tx| {
            Manager::create_watch_only(
                tx,
                &chain,
                xpub.as_bytes(),
                &pub_pass,
                &mut *src,
                &Limits::DEFAULT,
            )
        })
        .expect_err("second creation");
        assert_eq!(
            err.to_string(),
            "item already exists: address manager already exists"
        );
        let err = update(&wstore, |tx| {
            Manager::create(tx, &chain, &seed, b"p", b"q", &mut *src, &Limits::DEFAULT)
        })
        .expect_err("second creation");
        assert!(err.is_kind(Kind::Exist), "{err}");
        let open = |rng: &mut SplitMix64| {
            let tx = wstore.read().expect("read");
            Manager::open(
                &mut &tx,
                &chain,
                &pub_pass,
                rng_source(rng),
                Limits::DEFAULT,
            )
            .expect("open")
        };
        let mut wmgr = open(&mut rng);
        assert!(wmgr.watching_only() && wmgr.is_locked(), "{what}");

        // QK-0037: the "private" field is the account key's string under
        // a private crypto key the public passphrase opens.
        let row = {
            let tx = wstore.read().expect("read");
            tx.get::<Accounts>(&AccountKey(0))
                .expect("read")
                .expect("row")
        };
        let sealed = AccountKeys::decode(&row.keys).expect("keys").enc_priv;
        assert_eq!(
            open_with_private_crypto_key(&wstore, &pub_pass, &sealed).as_deref(),
            Some(xpub.as_bytes()),
            "{what}"
        );

        mgr.unlock(b"private").expect("unlock");
        let syncs = [
            (0, u32::try_from(rng.below(6)).expect("small")),
            (1, u32::try_from(rng.below(6)).expect("small")),
        ];
        for (branch, index) in syncs {
            for m in [&mgr, &wmgr] {
                let s = if std::ptr::eq(m, &mgr) {
                    &store
                } else {
                    &wstore
                };
                update(s, |tx| m.sync_account_to_addr_index(tx, 0, index, branch)).expect("sync");
            }
        }
        for reopened in [false, true] {
            if reopened {
                drop(wmgr);
                wmgr = open(&mut rng);
            }
            let what = format!("{what}, reopened {reopened}");
            let (tx, wtx) = (store.read().expect("read"), wstore.read().expect("read"));
            assert_eq!(
                mgr.account_extended_pub_key(&mut &tx, 0)
                    .expect("xpub")
                    .string(),
                wmgr.account_extended_pub_key(&mut &wtx, 0)
                    .expect("xpub")
                    .string(),
                "{what}"
            );
            for b in [0, 1] {
                assert_eq!(
                    mgr.account_branch_extended_pub_key(&mut &tx, 0, b)
                        .expect("branch")
                        .string(),
                    wmgr.account_branch_extended_pub_key(&mut &wtx, 0, b)
                        .expect("branch")
                        .string(),
                    "{what}"
                );
            }
            let list = |m: &Manager, t: &purse_store::ReadTx| {
                let mut out = Vec::new();
                m.for_each_account_address(&mut &*t, 0, |a| {
                    out.push(a);
                    Ok(())
                })
                .expect("for each");
                out
            };
            let (seed_list, watch_list) = (list(&mgr, &tx), list(&wmgr, &wtx));
            assert_eq!(seed_list.len(), watch_list.len(), "{what}");
            for (s, w) in seed_list.iter().zip(&watch_list) {
                assert_eq!(s.address(), w.address(), "{what}");
                assert_eq!(s.pub_key(), w.pub_key(), "{what}");
                assert_eq!((s.index(), s.internal()), (w.index(), w.internal()));
                let found = wmgr.address(&mut &wtx, w.address()).expect("address");
                assert_eq!(found.pub_key(), s.pub_key(), "{what}");
                assert!(wmgr.have_private_key(&mut &wtx, w.address()).expect("have"));
                mgr.private_key(&mut &tx, s.address())
                    .expect("seed wallet key");
                let err = wmgr
                    .private_key(&mut &wtx, w.address())
                    .expect_err("no private key");
                assert!(err.is_kind(Kind::Locked), "{what}: {err}");
            }
            let err = wmgr.coin_type(&mut &wtx).expect_err("no coin type");
            assert!(err.is_kind(Kind::WatchingOnly), "{what}");
            let err = wmgr.unlock(&pub_pass).expect_err("unlock");
            assert!(err.is_kind(Kind::WatchingOnly), "{what}");
        }
        // QK-0037: a public change rewrites only the public half, so the
        // creation's passphrase still opens account 0's private field and
        // the new one does not; the new one opens the wallet.
        let new_pass = [pub_pass.as_slice(), b"!"].concat();
        wmgr.change_passphrase(&wstore, &pub_pass, &new_pass, false)
            .expect("public change");
        assert_eq!(
            open_with_private_crypto_key(&wstore, &pub_pass, &sealed).as_deref(),
            Some(xpub.as_bytes()),
            "{what}: after a public change"
        );
        assert_eq!(
            open_with_private_crypto_key(&wstore, &new_pass, &sealed),
            None,
            "{what}: the new public passphrase"
        );
        drop(wmgr);
        let tx = wstore.read().expect("read");
        Manager::open(
            &mut &tx,
            &chain,
            &new_pass,
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open with the new public passphrase");
        drop(tx);
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(seen.len() >= 5, "networks and coin types: {seen:?}");
}

/// What a manager answers about a wallet without a private key: for every
/// account, its public key, properties and recorded addresses (with each
/// address's lookup), the redeem scripts, and the wallet-wide reads.
fn public_answers(m: &Manager, store: &Store) -> Vec<String> {
    let tx = store.read().expect("read");
    let mut out = vec![
        format!(
            "coin {:?}",
            m.coin_type(&mut &tx).map_err(|e| e.to_string())
        ),
        format!(
            "last {:?}",
            m.last_account(&mut &tx).map_err(|e| e.to_string())
        ),
        format!(
            "last imported {:?}",
            m.last_imported_account(&mut &tx).map_err(|e| e.to_string())
        ),
    ];
    for n in m.for_each_account(&mut &tx).expect("accounts") {
        out.push(format!(
            "{n} xpub {:?}",
            m.account_extended_pub_key(&mut &tx, n)
                .map(|k| k.string().to_string())
                .map_err(|e| e.to_string())
        ));
        out.push(format!(
            "{n} props {:?}",
            m.account_properties(&mut &tx, n).map_err(|e| e.to_string())
        ));
        let mut listed = Vec::new();
        m.for_each_account_address(&mut &tx, n, |a| {
            listed.push(a);
            Ok(())
        })
        .expect("for each");
        for a in listed {
            let found = m
                .address(&mut &tx, a.address())
                .map(|f| (f.account(), f.pub_key().map(<[u8]>::to_vec), f.index()));
            out.push(format!(
                "{n} {} {:?} {:?} {:?}",
                a.address().encode(),
                a.pub_key(),
                found.map_err(|e| e.to_string()),
                m.redeem_script(&mut &tx, a.address()).ok()
            ));
        }
    }
    out
}

/// `ConvertToWatchingOnly` over random wallets (both coin types, accounts,
/// some with their own passphrase, imported keys, scripts, xpub and voting
/// accounts, addresses, the wallet locked or unlocked): afterwards the
/// store holds no private master key parameters, no sealed private crypto
/// key, no coin type private key and no imported key's private half; an
/// account whose key the private crypto key seals keeps it as Go keeps it
/// (QK-0761, reproduced), a BIP0044 account with its own passphrase has its
/// sealed key emptied and its Argon2id parameters kept, and a voting
/// account keeps its key (QK-0761, QK-0766, decided 2026-11-22); every
/// public answer, `AccountHasPassphrase` included, is the one before the
/// conversion (locked), live and after a reopen; the manager refuses to
/// unlock and every private key request fails. A second conversion changes
/// nothing.
#[test]
fn conversion_keeps_public_answers_and_removes_private_keys() {
    use purse_store::wallet::{
        CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, MasterKeyPriv, WatchOnly,
    };
    use purse_store::{AccountKey, Accounts, AddrByHash, AddrRow};
    use purse_udb::addrmgr::StoreRead;
    use support::generator::{import_key, import_pub_key};
    let mut rng = SplitMix64::from_env("purse-udb-conversion");
    let mut seen = BTreeSet::new();
    let mut own_seen = 0usize;
    for case in 0..cases() {
        let (store, mgr, chain) = wallet(&mut rng, "prop-convert");
        let slip = rng.below(2) == 0;
        if slip {
            update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        }
        mgr.unlock(b"private").expect("unlock");
        let accounts = u32::try_from(rng.below(3)).expect("small");
        for n in 0..accounts {
            update(&store, |tx| mgr.new_account(tx, format!("a{n}").as_bytes())).expect("new");
        }
        let mut imported = Vec::new();
        for _ in 0..rng.below(4) {
            let k = import_key(&mut rng);
            let wif = purse_udb::addrmgr::Wif::new(
                &k,
                chain.ids.private_key,
                purse_dcrutil::SignatureType::ECDSA_SECP256K1,
            )
            .expect("wif");
            if let Ok(m) = update(&store, |tx| mgr.import_private_key(tx, &wif)) {
                imported.push(m.address().clone());
            }
        }
        for _ in 0..rng.below(3) {
            let script = format!(
                "5121{}51ae",
                purse_testutil::hex(&import_pub_key(&import_key(&mut rng)))
            );
            let _ = update(&store, |tx| {
                mgr.import_script(tx, &purse_testutil::unhex(&script))
            });
        }
        if rng.below(2) == 0 {
            let x = ExtendedKey::new_master(&rng.bytes(32), &chain.hd)
                .expect("master")
                .neuter();
            update(&store, |tx| mgr.import_xpub_account(tx, b"xpub", &x)).expect("xpub");
        }
        for n in 0..=accounts {
            let (b, i) = (
                u32::try_from(rng.below(2)).expect("small"),
                u32::try_from(rng.below(5)).expect("small"),
            );
            update(&store, |tx| mgr.sync_account_to_addr_index(tx, n, i, b)).expect("sync");
        }
        // Fill the cache with private keys, then lock or not.
        {
            let tx = store.read().expect("read");
            for n in 0..=accounts {
                mgr.account_extended_priv_key(&mut &tx, n).expect("xpriv");
            }
        }
        // Accounts with their own passphrase (left unlocked by it, cached
        // or not) and, in some wallets, a voting account.
        let mut own = BTreeSet::new();
        for n in 0..=accounts {
            if rng.below(3) == 0 {
                update(&store, |tx| mgr.set_account_passphrase(tx, n, b"own")).expect("set");
                own.insert(n);
                if rng.below(2) == 0 {
                    let tx = store.read().expect("read");
                    mgr.account_extended_priv_key(&mut &tx, n)
                        .expect("unlocked");
                }
            }
        }
        if rng.below(3) == 0 {
            let x = ExtendedKey::new_master(&rng.bytes(32), &chain.hd)
                .expect("master")
                .child(0x8000_0000)
                .expect("child");
            update(&store, |tx| {
                mgr.import_voting_account(tx, &x, b"v", b"vote")
            })
            .expect("voting");
        }
        let locked = rng.below(2) == 0;
        if locked {
            mgr.lock().expect("lock");
        }
        seen.insert((slip, locked));
        if !own.is_empty() {
            own_seen += 1;
        }
        // The conversion locks every account; so the public answers are
        // compared with the accounts locked by their own passphrase.
        {
            let tx = store.read().expect("read");
            for n in &own {
                let _ = mgr.lock_account(&mut &tx, *n);
            }
        }
        let what = format!("case {case} (SLIP0044 {slip}, locked {locked}, own {own:?})");
        let before = public_answers(&mgr, &store);
        // What AccountHasPassphrase answers after the conversion: the same
        // passphrases, all locked.
        let has_before: Vec<(bool, bool)> = {
            let tx = store.read().expect("read");
            let mut nums = StoreRead::account_numbers(&mut &tx).expect("numbers");
            nums.sort_unstable();
            nums.into_iter()
                .map(|n| (mgr.account_has_passphrase(&mut &tx, n).0, false))
                .collect()
        };
        let acct_keys = |s: &Store| -> Vec<(u32, u8, AccountKeys)> {
            let tx = s.read().expect("read");
            let mut nums = StoreRead::account_numbers(&mut &tx).expect("numbers");
            nums.sort_unstable();
            nums.into_iter()
                .map(|n| {
                    let row = tx
                        .get::<Accounts>(&AccountKey(n))
                        .expect("read")
                        .expect("row");
                    (
                        n,
                        row.account_type,
                        AccountKeys::decode(&row.keys).expect("keys"),
                    )
                })
                .collect()
        };
        let keys_before = acct_keys(&store);
        let has_after = |m: &Manager| -> Vec<(bool, bool)> {
            let tx = store.read().expect("read");
            let mut nums = StoreRead::account_numbers(&mut &tx).expect("numbers");
            nums.sort_unstable();
            nums.into_iter()
                .map(|n| m.account_has_passphrase(&mut &tx, n))
                .collect()
        };

        mgr.convert_to_watching_only(&store).expect("convert");
        mgr.convert_to_watching_only(&store)
            .expect("second convert");

        let check = |m: &Manager, when: &str| {
            assert!(m.watching_only() && m.is_locked(), "{what}: {when}");
            assert!(
                m.unlock(b"private")
                    .expect_err("unlock")
                    .is_kind(Kind::WatchingOnly)
            );
            assert!(m.lock().expect_err("lock").is_kind(Kind::WatchingOnly));
            assert!(
                m.encrypt_private(b"x")
                    .expect_err("encrypt")
                    .is_kind(Kind::Locked)
            );
            assert_eq!(public_answers(m, &store), before, "{what}: {when}");
            assert_eq!(has_after(m), has_before, "{what}: {when}: passphrases");
            let tx = store.read().expect("read");
            assert!(
                m.coin_type_priv_key(&mut &tx)
                    .expect_err("ct")
                    .is_kind(Kind::Locked)
            );
            for n in 0..=accounts {
                let err = m.account_extended_priv_key(&mut &tx, n).expect_err("xpriv");
                assert!(err.is_kind(Kind::Locked), "{what}: {when}: {err}");
            }
            for a in &imported {
                assert!(!m.have_private_key(&mut &tx, a).expect("have"), "{what}");
                assert!(m.private_key(&mut &tx, a).is_err(), "{what}");
            }
        };
        check(&mgr, "live");
        {
            let tx = store.read().expect("read");
            assert_eq!(tx.wallet_row::<WatchOnly>().expect("read"), Some(true));
            assert!(tx.wallet_row::<MasterKeyPriv>().expect("read").is_none());
            assert!(tx.wallet_row::<CryptoKeyPriv>().expect("read").is_none());
            for k in [
                tx.wallet_row::<CoinTypeLegacy>().expect("read"),
                tx.wallet_row::<CoinTypeSlip0044>().expect("read"),
            ]
            .into_iter()
            .flatten()
            {
                assert!(k.enc_priv.is_none() && k.enc_pub.is_some(), "{what}");
            }
            for (_, row) in StoreRead::rows_with_prefix::<AddrByHash>(&mut &tx, &[]).expect("rows")
            {
                if let AddrRow::Imported { data, .. } = row
                    && let ImportedData::Key { enc_priv, .. } =
                        ImportedData::decode(&data).expect("decode")
                {
                    assert!(enc_priv.is_empty(), "{what}");
                }
            }
        }
        // QK-0761: what the private crypto key seals stays; QK-0761 and
        // QK-0766 (decided 2026-11-22): what a BIP0044 account's own
        // passphrase seals goes, its parameters kept; voting accounts keep
        // theirs.
        let keys_after = acct_keys(&store);
        assert_eq!(keys_after.len(), keys_before.len(), "{what}");
        for ((n, ty, was), (n2, ty2, now)) in keys_before.iter().zip(&keys_after) {
            assert_eq!((n, ty), (n2, ty2), "{what}");
            assert_eq!(now.enc_pub, was.enc_pub, "{what}: account {n}");
            assert_eq!(now.kdf, was.kdf, "{what}: account {n}");
            let emptied = *ty == purse_udb::addrmgr::ACCOUNT_TYPE_BIP0044 && was.kdf.is_some();
            assert_eq!(emptied, own.contains(n), "{what}: account {n}");
            if emptied {
                assert!(now.enc_priv.is_empty(), "{what}: account {n}: QK-0766");
            } else {
                assert_eq!(now.enc_priv, was.enc_priv, "{what}: account {n}: QK-0761");
            }
        }
        drop(mgr);
        let tx = store.read().expect("read");
        let reopened = Manager::open(
            &mut &tx,
            &chain,
            b"public",
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open");
        drop(tx);
        check(&reopened, "reopened");
    }
    if cases() >= 16 {
        assert_eq!(seen.len(), 4, "coin types and lock states: {seen:?}");
        assert!(own_seen > 0, "no wallet had an account passphrase");
    }
}

/// A conversion whose transaction fails rolls back whole: the store keeps
/// every private key and is not watching-only, and the manager is not
/// watching-only either (QK-0733: memory follows the store). The failure
/// here, an imported row that does not decode, comes before the manager is
/// locked, so it stays unlocked; a failure of the commit itself would
/// leave it locked, and it unlocks again either way.
///
/// The imported rows are walked after the rows of BIP0044 accounts with
/// their own passphrase are rewritten (QK-0766, decided 2026-11-22), so in
/// the cases where account 1 has one, left unlocked by setting it or
/// locked again, the failure comes after that rewrite ran inside the
/// transaction: account 1's row bytes, what `AccountHasPassphrase` says of
/// it and its private key behind its own passphrase all survive. With the
/// bad row removed the same conversion then empties that key, which shows
/// the rewrite was reached.
#[test]
fn a_failed_conversion_changes_nothing() {
    use purse_store::wallet::{CryptoKeyPriv, MasterKeyPriv, WatchOnly};
    use purse_store::{AccountKey, Accounts, AddrByHash, AddrRow, Hash160};
    let mut rng = SplitMix64::from_env("purse-udb-conversion-fails");
    // 0: no account passphrase; 1: account 1 has its own, unlocked;
    // 2: account 1 has its own, locked.
    for case in 0..3 {
        let (store, mgr, chain) = wallet(&mut rng, "prop-convert-fails");
        mgr.unlock(b"private").expect("unlock");
        let wif = purse_udb::addrmgr::Wif::new(
            &[7u8; 32],
            chain.ids.private_key,
            purse_dcrutil::SignatureType::ECDSA_SECP256K1,
        )
        .expect("wif");
        let good = update(&store, |tx| mgr.import_private_key(tx, &wif)).expect("import");
        update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
        if case >= 1 {
            update(&store, |tx| mgr.set_account_passphrase(tx, 1, b"own")).expect("set");
            let tx = store.read().expect("read");
            if case == 1 {
                assert!(
                    mgr.account_extended_priv_key(&mut &tx, 1)
                        .expect("unlocked by setting it")
                        .is_private()
                );
            } else {
                mgr.lock_account(&mut &tx, 1).expect("lock account 1");
            }
        }
        let read_row = |store: &Store| {
            let tx = store.read().expect("read");
            tx.get::<Accounts>(&AccountKey(1))
                .expect("read")
                .expect("account 1")
        };
        let row_before = read_row(&store);
        let has_before = {
            let tx = store.read().expect("read");
            mgr.account_has_passphrase(&mut &tx, 1)
        };
        assert_eq!(has_before, (case >= 1, case == 1), "case {case}");
        let bad = Hash160([0x55; 20]);
        store
            .update(|tx| {
                tx.put::<AddrByHash>(
                    &bad,
                    &AddrRow::Imported {
                        account: purse_udb::addrmgr::IMPORTED_ADDR_ACCOUNT,
                        data: vec![9, 9, 9],
                    },
                )
            })
            .expect("malformed row");
        let err = mgr
            .convert_to_watching_only(&store)
            .expect_err("a malformed row fails the conversion");
        assert!(err.is_kind(Kind::Io), "case {case}: {err}");
        assert!(!mgr.watching_only());
        assert!(!mgr.is_locked());
        assert_eq!(read_row(&store), row_before, "case {case}: account 1's row");
        {
            let tx = store.read().expect("read");
            assert_eq!(
                mgr.account_has_passphrase(&mut &tx, 1),
                has_before,
                "case {case}"
            );
        }
        mgr.lock().expect("lock");
        let tx = store.read().expect("read");
        assert_eq!(tx.wallet_row::<WatchOnly>().expect("read"), Some(false));
        assert!(tx.wallet_row::<MasterKeyPriv>().expect("read").is_some());
        assert!(tx.wallet_row::<CryptoKeyPriv>().expect("read").is_some());
        assert!(
            mgr.have_private_key(&mut &tx, good.address())
                .expect("have")
        );
        drop(tx);
        mgr.unlock(b"private").expect("unlocks again");
        {
            let tx = store.read().expect("read");
            mgr.private_key(&mut &tx, good.address())
                .expect("the key is still there");
            if case >= 1 {
                mgr.unlock_account(&mut &tx, 1, b"own")
                    .expect("account 1 unlocks with its own passphrase");
            }
            assert!(
                mgr.account_extended_priv_key(&mut &tx, 1)
                    .expect("account 1's private key")
                    .is_private(),
                "case {case}"
            );
        }

        // Without the bad row the conversion goes through, and what it
        // rewrites is what the failed one had rewritten and rolled back.
        store
            .update(|tx| tx.delete::<AddrByHash>(&bad).map(|_| ()))
            .expect("remove the malformed row");
        mgr.convert_to_watching_only(&store).expect("convert");
        let was = AccountKeys::decode(&row_before.keys).expect("decode");
        let now = AccountKeys::decode(&read_row(&store).keys).expect("decode");
        assert_eq!(now.enc_pub, was.enc_pub, "case {case}");
        assert_eq!(now.kdf, was.kdf, "case {case}");
        assert!(!was.enc_priv.is_empty(), "case {case}");
        if case >= 1 {
            assert!(now.enc_priv.is_empty(), "case {case}: QK-0766");
        } else {
            assert_eq!(now.enc_priv, was.enc_priv, "case {case}: QK-0761");
        }
        let tx = store.read().expect("read");
        assert_eq!(
            mgr.account_has_passphrase(&mut &tx, 1),
            (case >= 1, false),
            "case {case}"
        );
    }
}

/// An account row whose key material does not decode is left as it is by
/// the conversion, as Go leaves it (Go's `deletePrivateKeys` reads no key
/// field of a current account row): the conversion succeeds, that row's
/// bytes are unchanged, and the rest of the store is converted.
#[test]
fn a_conversion_leaves_an_account_row_that_does_not_decode() {
    use purse_store::wallet::{CryptoKeyPriv, MasterKeyPriv, WatchOnly};
    use purse_store::{AccountKey, Accounts};
    let mut rng = SplitMix64::from_env("purse-udb-conversion-undecodable");
    for case in 0..4 {
        let (store, mgr, _chain) = wallet(&mut rng, "prop-convert-undecodable");
        mgr.unlock(b"private").expect("unlock");
        update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
        if case % 2 == 1 {
            update(&store, |tx| mgr.set_account_passphrase(tx, 1, b"own")).expect("set");
        }
        let bad = [vec![0xff], vec![], rng.bytes(7), vec![0u8; 3]];
        let keys = bad[case].clone();
        let row = {
            let tx = store.read().expect("read");
            let mut row = tx
                .get::<Accounts>(&AccountKey(1))
                .expect("read")
                .expect("account 1");
            row.keys = keys;
            row
        };
        assert!(AccountKeys::decode(&row.keys).is_err(), "case {case}");
        store
            .update(|tx| tx.put::<Accounts>(&AccountKey(1), &row))
            .expect("a row that does not decode");
        mgr.convert_to_watching_only(&store)
            .unwrap_or_else(|e| panic!("case {case}: {e}"));
        assert!(mgr.watching_only(), "case {case}");
        let tx = store.read().expect("read");
        assert_eq!(
            tx.get::<Accounts>(&AccountKey(1)).expect("read"),
            Some(row),
            "case {case}: the row is left as it was"
        );
        assert_eq!(tx.wallet_row::<WatchOnly>().expect("read"), Some(true));
        assert!(tx.wallet_row::<MasterKeyPriv>().expect("read").is_none());
        assert!(tx.wallet_row::<CryptoKeyPriv>().expect("read").is_none());
    }
}

/// What a conversion removes (QK-0761, QK-0766; ADR-0012 Rule 6) is gone
/// from every row of every table and from every byte of a backup of the
/// converted store (ADR-0018: rows copied into a new file), over random
/// wallets (both coin types, accounts with their own passphrase, imported
/// keys): the private master key's salt and digest, the sealed private
/// crypto key, the coin types' sealed private keys, the imported keys'
/// sealed private halves and the keys accounts' own passphrases seal. The
/// live store file is scanned too and is not held to it: redb frees the
/// pages of a changed row without clearing them, as bbolt does for Go, so
/// earlier copies can stay in it until the pages are reused; the count
/// found is printed, not asserted.
#[test]
fn a_conversion_leaves_no_removed_value_in_rows_or_a_backup() {
    use purse_store::wallet::{CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, MasterKeyPriv};
    use purse_store::{
        AccountKey, Accounts, AddrByHash, AddrRow, BackupInfo, CoinTypeKeys, TableId,
    };
    use std::ops::Bound;
    use support::generator::import_key;
    fn holds(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }
    let mut rng = SplitMix64::from_env("purse-udb-conversion-pages");
    let mut values = 0usize;
    let mut live = 0usize;
    for case in 0..cases().div_ceil(4).max(4) {
        let (path, store, mgr, chain) = wallet_file(&mut rng, "prop-convert-pages");
        mgr.unlock(b"private").expect("unlock");
        if rng.below(2) == 0 {
            update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        }
        update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
        update(&store, |tx| mgr.new_account(tx, b"two")).expect("new account");
        update(&store, |tx| mgr.set_account_passphrase(tx, 1, b"one-pass")).expect("set");
        if rng.below(2) == 0 {
            update(&store, |tx| mgr.set_account_passphrase(tx, 2, b"two-pass")).expect("set");
        }
        for _ in 0..=rng.below(2) {
            let wif = purse_udb::addrmgr::Wif::new(
                &import_key(&mut rng),
                chain.ids.private_key,
                purse_dcrutil::SignatureType::ECDSA_SECP256K1,
            )
            .expect("wif");
            let _ = update(&store, |tx| mgr.import_private_key(tx, &wif));
        }
        if rng.below(2) == 0 {
            mgr.lock().expect("lock");
        }
        // The values the conversion removes, each long enough to be found
        // only where it was written.
        let mut removed: Vec<(String, Vec<u8>)> = Vec::new();
        {
            let tx = store.read().expect("read");
            let m = tx
                .wallet_row::<MasterKeyPriv>()
                .expect("read")
                .expect("private master key");
            removed.push(("private master key salt".into(), m.salt.to_vec()));
            removed.push(("private master key digest".into(), m.digest.to_vec()));
            let c = tx
                .wallet_row::<CryptoKeyPriv>()
                .expect("read")
                .expect("sealed private crypto key");
            removed.push(("sealed private crypto key".into(), c.0));
            for (what, ct) in [
                ("legacy", tx.wallet_row::<CoinTypeLegacy>().expect("read")),
                (
                    "SLIP0044",
                    tx.wallet_row::<CoinTypeSlip0044>().expect("read"),
                ),
            ] {
                if let Some(CoinTypeKeys {
                    enc_priv: Some(p), ..
                }) = ct
                {
                    removed.push((format!("{what} coin type private key"), p));
                }
            }
            for r in tx.iter::<Accounts>().expect("accounts") {
                let (AccountKey(n), row) = r.expect("account row");
                let k = AccountKeys::decode(&row.keys).expect("keys");
                if row.account_type == purse_udb::addrmgr::ACCOUNT_TYPE_BIP0044 && k.kdf.is_some() {
                    removed.push((format!("account {n}'s own key"), k.enc_priv));
                }
            }
            for r in tx.iter::<AddrByHash>().expect("addresses") {
                let (_, row) = r.expect("address row");
                if let AddrRow::Imported { data, .. } = row
                    && let Ok(ImportedData::Key { enc_priv, .. }) = ImportedData::decode(&data)
                    && !enc_priv.is_empty()
                {
                    removed.push(("an imported key's private half".into(), enc_priv));
                }
            }
        }
        assert!(
            removed.iter().any(|(w, _)| w == "account 1's own key"),
            "case {case}: {removed:?}"
        );
        assert!(removed.iter().all(|(_, v)| v.len() >= 32), "case {case}");
        values += removed.len();

        mgr.convert_to_watching_only(&store).expect("convert");

        let tx = store.read().expect("read");
        for &id in TableId::ALL {
            for e in tx
                .range_raw(id, Bound::Unbounded, Bound::Unbounded)
                .expect("table")
            {
                let (k, v) = e.expect("row");
                for (what, needle) in &removed {
                    assert!(
                        !holds(&k, needle) && !holds(&v, needle),
                        "case {case}: {what} is still in a row of {}",
                        id.name()
                    );
                }
            }
        }
        drop(tx);
        let dir = scratch("prop-convert-pages-backup");
        let info = BackupInfo {
            network: "simnet",
            version: "2.2.0-pre+dcr-purse.0.1.0",
            time: 1_794_990_000_123_456_789,
        };
        let backup = store.backup(&dir, "converted", &info).expect("backup");
        let file = std::fs::read(&backup.path).expect("read the backup");
        for (what, needle) in &removed {
            assert!(
                !holds(&file, needle),
                "case {case}: {what} is in the backup's file"
            );
        }
        let file = std::fs::read(&path).expect("read the store file");
        live += removed.iter().filter(|(_, n)| holds(&file, n)).count();
        drop(mgr);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }
    eprintln!(
        "removed values: {values}; still in the live store files' freed pages (not asserted): {live}"
    );
}

/// [`reopen_equals_in_memory_state`] for generated watching-only scripts:
/// a manager created from an extended key, or converted in this process
/// (its cached accounts' sealed keys dropped, Go's in-memory conversion),
/// answers every read as a manager opened again on the same store.
#[test]
fn watching_only_reopen_equals_in_memory_state() {
    use purse_testutil::serde_json::{Value, json};
    use support::generator::{Shape, generate_watching, watch_kind};
    use support::runner;

    let mut rng = SplitMix64::from_env("purse-udb-watching-reopen");
    let mut sweep = vec![
        json!({"op": "mgr.watching_only"}),
        json!({"op": "mgr.is_locked"}),
        json!({"op": "mgr.coin_type"}),
        json!({"op": "mgr.coin_type_priv_key"}),
        json!({"op": "mgr.last_account"}),
        json!({"op": "mgr.last_imported_account"}),
        json!({"op": "mgr.for_each_account"}),
        json!({"op": "mgr.for_each_active_address"}),
        json!({"op": "mgr.unlock", "pass": "x"}),
        json!({"op": "mgr.encrypt", "key": "private", "data": "00"}),
    ];
    for a in [0u32, 1, 2, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001] {
        for op in [
            "mgr.account_name",
            "mgr.account_properties",
            "mgr.account_has_passphrase",
            "mgr.account_extended_pub_key",
            "mgr.account_extended_priv_key",
            "mgr.for_each_account_address",
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
    let mut seen = std::collections::BTreeMap::new();
    for case in 0..cases() {
        let script = generate_watching(&mut rng, case, shape);
        *seen.entry(watch_kind(&script)).or_insert(0usize) += 1;
        let base = script["ops"].as_array().expect("ops").clone();
        let mut live = script.clone();
        live["ops"] = Value::Array(base.iter().chain(&sweep).cloned().collect());
        let dir = scratch("prop-wo-reopen");
        let a = runner::run(&live, &dir.join("live")).expect("live run");
        let mut ops = base.clone();
        ops.push(json!({"op": "rt.reopen"}));
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
        let tail_a = &a.results[base.len()..];
        assert_eq!(tail_a[0]["ok"], json!(true), "case {case}: watching-only");
        assert_eq!(
            strip(tail_a),
            strip(&b.results[skip..]),
            "case {case}: {script}"
        );
        assert_eq!(a.export, b.export, "case {case}: exports");
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!("kind -> cases: {seen:?}");
    if cases() >= 24 {
        assert_eq!(seen.len(), 4, "every kind: {seen:?}");
    }
}

/// What the model of [`account_passphrases_follow_gos_state_machine`] keeps
/// of one account: its own passphrase, if any, and whether that passphrase
/// has unlocked it.
#[derive(Clone, Debug)]
struct PassModel {
    account: u32,
    xpriv: String,
    pass: Option<&'static [u8]>,
    unlocked: bool,
}

/// What [`pass_answers`] reports of one account: `AccountHasPassphrase`,
/// the private key or the error kind, and whether the row stores Argon2id
/// parameters.
type PassAnswer = ((bool, bool), Result<String, Kind>, bool);

/// The manager's answers about the accounts of the model, read from a
/// snapshot.
fn pass_answers(m: &Manager, store: &Store, accounts: &[PassModel]) -> Vec<PassAnswer> {
    let tx = store.read().expect("read");
    accounts
        .iter()
        .map(|a| {
            let has = m.account_has_passphrase(&mut &tx, a.account);
            let key = m
                .account_extended_priv_key(&mut &tx, a.account)
                .map(|k| k.string().as_str().to_owned())
                .map_err(|e| e.as_kind().expect("a kind"));
            let row = tx
                .get::<purse_store::Accounts>(&purse_store::AccountKey(a.account))
                .expect("get")
                .expect("row");
            let kdf = AccountKeys::decode(&row.keys).expect("keys").kdf.is_some();
            (has, key, kdf)
        })
        .collect()
}

/// Per-account passphrases follow a model of Go's rules (spec 02 sections
/// 5.7 and 5.15; spec 02 section 9 item 17), through random sequences of
/// `SetAccountPassphrase` (set, change, remove), `UnlockAccount`,
/// `LockAccount`, the wallet's lock and unlock (right and wrong
/// passphrase) and reopens, on BIP0044 accounts 0 and 1 and an imported
/// voting account: a passphrase can be set only on an unlocked account and
/// leaves it unlocked; it is removed only while the wallet is unlocked;
/// every wallet lock, and a wrong wallet passphrase, locks every account;
/// unlocking the wallet unlocks none of them; an unlocked account refuses a
/// different passphrase and stays unlocked. A `SetAccountPassphrase` whose
/// transaction rolls back changes nothing, in the store or in memory
/// (QK-0733, not reproduced); an `UnlockAccount` in a transaction that
/// rolls back, which writes nothing, still unlocks. After every step each
/// account's `AccountHasPassphrase`, private key (the right key, or the
/// model's error kind) and stored parameters agree with the model.
#[test]
fn account_passphrases_follow_gos_state_machine() {
    let mut rng = SplitMix64::from_env("purse-udb-account-passphrases");
    const PASSES: [&[u8]; 3] = [b"p", b"q", b""];
    for case in 0..cases().div_ceil(4) {
        let (store, mut mgr, chain) = wallet(&mut rng, "prop-acctpass");
        mgr.unlock(b"private").expect("unlock");
        update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
        let voting = ExtendedKey::new_master(&rng.bytes(32), &chain.hd)
            .expect("master")
            .child(0x8000_002a)
            .expect("child");
        let v = update(&store, |tx| {
            mgr.import_voting_account(tx, &voting, b"v", b"vote")
        })
        .expect("import voting");
        assert_eq!(v, 0x8000_0000, "case {case}");
        let tx = store.read().expect("read");
        let key = |n: u32| {
            mgr.account_extended_priv_key(&mut &tx, n)
                .expect("xpriv")
                .string()
                .as_str()
                .to_owned()
        };
        let mut model = vec![
            PassModel {
                account: 0,
                xpriv: key(0),
                pass: None,
                unlocked: false,
            },
            PassModel {
                account: 1,
                xpriv: key(1),
                pass: None,
                unlocked: false,
            },
            PassModel {
                account: v,
                xpriv: voting.string().as_str().to_owned(),
                pass: Some(b"v"),
                unlocked: false,
            },
        ];
        drop(tx);
        let mut locked = false;
        let lock_all = |model: &mut Vec<PassModel>| {
            for a in model.iter_mut() {
                a.unlocked = false;
            }
        };
        let mut seen = [0usize; 6];
        for step in 0..60 {
            let what = format!("case {case} step {step}");
            let i = usize::try_from(rng.below(3)).expect("small");
            let a = model[i].account;
            let avail = |m: &PassModel, locked: bool| match m.pass {
                Some(_) => m.unlocked,
                None => !locked,
            };
            match rng.below(10) {
                0 => {
                    let r = mgr.unlock(b"private");
                    assert!(r.is_ok(), "{what}: unlock");
                    locked = false;
                }
                1 => {
                    assert!(mgr.unlock(b"wrong").is_err(), "{what}");
                    locked = true;
                    lock_all(&mut model);
                }
                2 => {
                    // On a locked wallet Lock fails and changes nothing:
                    // an account its own passphrase unlocked stays so.
                    assert_eq!(mgr.lock().is_ok(), !locked, "{what}: lock");
                    if !locked {
                        locked = true;
                        lock_all(&mut model);
                    }
                }
                3 | 4 => {
                    let p = PASSES[usize::try_from(rng.below(3)).expect("small")];
                    let r = update(&store, |tx| mgr.set_account_passphrase(tx, a, p));
                    let ok = avail(&model[i], locked) && (!p.is_empty() || !locked);
                    assert_eq!(r.is_ok(), ok, "{what}: set {a} {p:?}: {r:?}");
                    if ok {
                        seen[0] += 1;
                        model[i].pass = (!p.is_empty()).then_some(p);
                        model[i].unlocked = !p.is_empty();
                    }
                }
                5 => {
                    // Rolled back: nothing changes.
                    let p = PASSES[usize::try_from(rng.below(2)).expect("small")];
                    let r = update(&store, |tx| {
                        mgr.set_account_passphrase(tx, a, p)?;
                        Err::<(), _>(AnyError::new("roll back"))
                    });
                    assert!(r.is_err(), "{what}");
                    seen[1] += 1;
                }
                6 | 7 => {
                    let p: &'static [u8] = if rng.below(3) == 0 {
                        b"r"
                    } else {
                        model[i].pass.unwrap_or(b"p")
                    };
                    let in_write = rng.below(4) == 0;
                    let r = if in_write {
                        // In a write transaction that rolls back.
                        let mut got = None;
                        let _ = update(&store, |tx| {
                            got = Some(mgr.unlock_account(tx, a, p));
                            Err::<(), _>(AnyError::new("roll back"))
                        });
                        got.expect("ran")
                    } else {
                        let tx = store.read().expect("read");
                        mgr.unlock_account(&mut &tx, a, p)
                    };
                    let ok = model[i].pass == Some(p);
                    assert_eq!(r.is_ok(), ok, "{what}: unlock {a} {p:?}: {r:?}");
                    if ok {
                        seen[2] += 1;
                        model[i].unlocked = true;
                    }
                }
                8 => {
                    let tx = store.read().expect("read");
                    let r = mgr.lock_account(&mut &tx, a);
                    let ok = model[i].pass.is_some() && model[i].unlocked;
                    assert_eq!(r.is_ok(), ok, "{what}: lock {a}: {r:?}");
                    if ok {
                        seen[3] += 1;
                        model[i].unlocked = false;
                    }
                }
                _ => {
                    drop(mgr);
                    let tx = store.read().expect("read");
                    mgr = Manager::open(
                        &mut &tx,
                        &chain,
                        b"public",
                        rng_source(&mut rng),
                        Limits::DEFAULT,
                    )
                    .expect("open");
                    locked = true;
                    lock_all(&mut model);
                    seen[4] += 1;
                }
            }
            assert_eq!(mgr.is_locked(), locked, "{what}: wallet lock");
            let got = pass_answers(&mgr, &store, &model);
            for (m, (has, key, kdf)) in model.iter().zip(got) {
                let a = m.account;
                assert_eq!(
                    has,
                    (m.pass.is_some(), m.pass.is_some() && m.unlocked),
                    "{what}: AccountHasPassphrase({a})"
                );
                assert_eq!(kdf, m.pass.is_some(), "{what}: account {a}'s parameters");
                if avail(m, locked) {
                    assert_eq!(key.as_deref(), Ok(m.xpriv.as_str()), "{what}: key {a}");
                } else {
                    // Go's AccountExtendedPrivKey: Invalid above the
                    // reserved account (QK-0041), Locked below it.
                    let want = if a > 0x7FFF_FFFF {
                        Kind::Invalid
                    } else {
                        Kind::Locked
                    };
                    assert_eq!(key, Err(want), "{what}: key {a}");
                }
                if m.pass.is_some() && m.unlocked {
                    seen[5] += 1;
                }
            }
        }
        eprintln!(
            "case {case}: sets {}, rolled back {}, unlocks {}, locks {}, reopens {}, \
             unlocked account-steps {}",
            seen[0], seen[1], seen[2], seen[3], seen[4], seen[5]
        );
    }
}

/// A snapshot taken before a `SetAccountPassphrase` commits, read after it,
/// answers as the store did then, and does not put the account's old key
/// material back in the cache: a later snapshot sees the new passphrase,
/// with the account unlocked as `SetAccountPassphrase` leaves it. The same
/// for a removal.
#[test]
fn a_snapshot_from_before_a_passphrase_change_keeps_the_cache_current() {
    let mut rng = SplitMix64::from_env("purse-udb-acctpass-snapshot");
    let (store, mgr, _) = wallet(&mut rng, "prop-acctpass-snapshot");
    mgr.unlock(b"private").expect("unlock");
    update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
    for (pass, want) in [
        (&b"p"[..], (true, true)),
        (b"", (false, false)),
        (b"q", (true, true)),
    ] {
        let old = store.read().expect("read");
        let before = mgr.account_has_passphrase(&mut &old, 1);
        update(&store, |tx| mgr.set_account_passphrase(tx, 1, pass)).expect("set");
        // The old snapshot first, so it is the one that would cache.
        assert_eq!(mgr.account_has_passphrase(&mut &old, 1), before, "{pass:?}");
        mgr.account_extended_pub_key(&mut &old, 1).expect("xpub");
        drop(old);
        let tx = store.read().expect("read");
        assert_eq!(mgr.account_has_passphrase(&mut &tx, 1), want, "{pass:?}");
        mgr.account_extended_priv_key(&mut &tx, 1)
            .expect("unlocked, by the wallet or its own passphrase");
    }
    // Locking the wallet locks the account; its passphrase unlocks it.
    mgr.lock().expect("lock");
    let tx = store.read().expect("read");
    assert_eq!(mgr.account_has_passphrase(&mut &tx, 1), (true, false));
    mgr.unlock_account(&mut &tx, 1, b"q")
        .expect("unlock account");
    assert_eq!(mgr.account_has_passphrase(&mut &tx, 1), (true, true));
}

/// QK-0041 (ADR-0012 Rule 7): Go's `importAccount` takes the account after
/// the last imported one with no wraparound check, so after an imported
/// account 0xFFFFFFFF it imports over account 0. dcr-purse refuses with
/// `ImportXpubAccount`'s text and writes nothing. No input reaches 2^31
/// imported accounts in practice; Go's behaviour is read from its source
/// (`wallet/udb/addressmanager.go:2278-2282`), not run.
#[test]
fn voting_account_numbers_never_wrap_onto_account_0() {
    let mut rng = SplitMix64::from_env("purse-udb-voting-wrap");
    let (store, mgr, chain) = wallet(&mut rng, "prop-voting-wrap");
    let xpriv = ExtendedKey::new_master(&[3u8; 32], &chain.hd)
        .expect("master")
        .child(0x8000_0001)
        .expect("child");
    update(&store, |tx| {
        tx.put_wallet_row::<purse_store::wallet::LastImportedAccount>(&purse_store::AccountNumber(
            u32::MAX,
        ))
        .map_err(|e| AnyError::from(e.to_purse_error()))
    })
    .expect("last imported account");
    let before = {
        let tx = store.read().expect("read");
        mgr.account_extended_pub_key(&mut &tx, 0)
            .expect("xpub")
            .string()
    };
    let err = update(&store, |tx| {
        mgr.import_voting_account(tx, &xpriv, b"v", b"wrapped")
    })
    .expect_err("refused");
    assert!(err.is_kind(Kind::Invalid), "{err}");
    assert_eq!(
        err.render(purse_errors::DAEMON_SEPARATOR),
        "invalid operation: exhausted possible imported accounts"
    );
    let tx = store.read().expect("read");
    assert_eq!(
        mgr.account_extended_pub_key(&mut &tx, 0)
            .expect("xpub")
            .string(),
        before
    );
    assert!(mgr.lookup_account(&mut &tx, b"wrapped").is_err());
}

/// The rows a passphrase change may rewrite, and everything else, as read
/// from one snapshot.
#[derive(Debug, PartialEq)]
struct ChangeRows {
    private: (
        Option<purse_store::MasterKeyParams>,
        Option<purse_store::SealedKey>,
    ),
    public: (
        Option<purse_store::MasterKeyParams>,
        Option<purse_store::SealedKey>,
    ),
    rest: Vec<String>,
}

fn change_rows(store: &Store) -> ChangeRows {
    use purse_store::wallet::{
        CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, CryptoKeyPub, LastAccount,
        LastImportedAccount, MasterKeyPriv, MasterKeyPub, WatchOnly,
    };
    use purse_store::{Accounts, AddrByHash};
    let tx = store.read().expect("read");
    let mut rest = vec![
        format!("{:?}", tx.wallet_row::<CoinTypeLegacy>().expect("read")),
        format!("{:?}", tx.wallet_row::<CoinTypeSlip0044>().expect("read")),
        format!("{:?}", tx.wallet_row::<Slip0044Account0>().expect("read")),
        format!("{:?}", tx.wallet_row::<WatchOnly>().expect("read")),
        format!("{:?}", tx.wallet_row::<LastAccount>().expect("read")),
        format!(
            "{:?}",
            tx.wallet_row::<LastImportedAccount>().expect("read")
        ),
    ];
    for r in tx.iter::<Accounts>().expect("accounts") {
        rest.push(format!("{:?}", r.expect("row")));
    }
    for r in tx.iter::<AddrByHash>().expect("addresses") {
        rest.push(format!("{:?}", r.expect("row")));
    }
    ChangeRows {
        private: (
            tx.wallet_row::<MasterKeyPriv>().expect("read"),
            tx.wallet_row::<CryptoKeyPriv>().expect("read"),
        ),
        public: (
            tx.wallet_row::<MasterKeyPub>().expect("read"),
            tx.wallet_row::<CryptoKeyPub>().expect("read"),
        ),
        rest,
    }
}

/// `ChangePassphrase` against a model of Go's rules (spec 02 section 5.8)
/// over random simnet wallets (both coin types, an extra account with its
/// own passphrase, an imported key): random changes of either passphrase,
/// with the current old passphrase or another and a random new one (empty
/// and equal to the old included), wallet locks, unlocks with the current
/// or an earlier passphrase, account unlocks, and reopens with the current
/// public passphrase. After every step:
///
/// - a change succeeds exactly when its old passphrase is the current one
///   and it is not a change of the private passphrase to the empty one
///   (refused with creation's error, QK-0768, where Go accepts it), leaves
///   the lock state as it was, and rewrites exactly its half's master key
///   parameters and sealed crypto key (both, with the network's scrypt
///   cost), nothing else, drawing from the manager's random source in
///   Go's sizes; a failed or refused one rewrites nothing and draws
///   nothing;
/// - `Unlock` and `UnlockedWithPassphrase` accept exactly the current
///   private passphrase, `Manager::open` exactly the current public one;
/// - the crypto keys never change: a value sealed before the first change
///   opens with the private crypto key after every unlock, and account 0's
///   extended private key, the coin type's and the imported key's private
///   key are those of the start;
/// - account 1, unlocked by its own passphrase, stays unlocked across
///   private changes until a wallet lock, and `UnlockAccount` with its
///   passphrase is accepted as already unlocked (the account passphrases'
///   hasher is not replaced with the wallet's).
#[test]
fn passphrase_changes_follow_gos_rules() {
    let mut rng = SplitMix64::from_env("purse-udb-change-passphrase");
    let passes: [&[u8]; 5] = [b"private", b"public", b"", b"p\xc3\xa4ss", b"x"];
    let mut changes = [0usize; 2];
    let mut refused = 0usize;
    for case in 0..cases() {
        let (store, mgr, chain) = wallet(&mut rng, "prop-change");
        // The manager again, on a source that counts its draws.
        drop(mgr);
        let drawn = Arc::new(AtomicU64::new(0));
        let mut mgr = {
            let tx = store.read().expect("read");
            Manager::open(
                &mut &tx,
                &chain,
                b"public",
                counted_source(&mut rng, &drawn),
                Limits::DEFAULT,
            )
            .expect("open")
        };
        mgr.unlock(b"private").expect("unlock");
        if rng.below(2) == 0 {
            update(&store, |tx| mgr.upgrade_to_slip0044_coin_type(tx)).expect("upgrade");
        }
        update(&store, |tx| mgr.new_account(tx, b"one")).expect("new account");
        update(&store, |tx| mgr.set_account_passphrase(tx, 1, b"acct")).expect("set");
        let wif = purse_udb::addrmgr::Wif::new(
            &[0x21u8; 32],
            chain.ids.private_key,
            purse_dcrutil::SignatureType::ECDSA_SECP256K1,
        )
        .expect("wif");
        let imported = update(&store, |tx| mgr.import_private_key(tx, &wif)).expect("import");
        let sealed = mgr.encrypt_private(b"sealed before").expect("seal");
        let (xpriv0, coin_priv) = {
            let tx = store.read().expect("read");
            (
                mgr.account_extended_priv_key(&mut &tx, 0)
                    .expect("xpriv")
                    .string()
                    .to_string(),
                mgr.coin_type_priv_key(&mut &tx)
                    .expect("coin type")
                    .string()
                    .to_string(),
            )
        };
        let mut privpass: Vec<u8> = b"private".to_vec();
        let mut pubpass: Vec<u8> = b"public".to_vec();
        let mut earlier: Vec<Vec<u8>> = Vec::new();
        let mut locked = false;
        // Whether account 1 is unlocked by its own passphrase (setting it
        // leaves the account unlocked, as in Go).
        let mut acct1 = true;
        let check_private = |mgr: &Manager, what: &str| {
            let tx = store.read().expect("read");
            assert_eq!(
                mgr.decrypt_private(&sealed).expect("open").as_bytes(),
                b"sealed before",
                "{what}"
            );
            assert_eq!(
                mgr.account_extended_priv_key(&mut &tx, 0)
                    .expect("xpriv")
                    .string()
                    .as_str(),
                xpriv0,
                "{what}"
            );
            assert_eq!(
                mgr.coin_type_priv_key(&mut &tx)
                    .expect("coin type")
                    .string()
                    .as_str(),
                coin_priv,
                "{what}"
            );
            mgr.private_key(&mut &tx, imported.address())
                .expect("imported key");
        };
        for step in 0..12 {
            let what = format!("case {case} step {step}");
            match rng.below(6) {
                0..=2 => {
                    let private = rng.below(2) == 0;
                    let current = if private { &privpass } else { &pubpass };
                    let old: Vec<u8> = if rng.below(3) == 0 {
                        passes[usize::try_from(rng.below(5)).expect("small")].to_vec()
                    } else {
                        current.clone()
                    };
                    let new: Vec<u8> = if rng.below(6) == 0 {
                        old.clone()
                    } else {
                        passes[usize::try_from(rng.below(5)).expect("small")].to_vec()
                    };
                    let before = change_rows(&store);
                    let drawn_before = drawn.load(Ordering::Relaxed);
                    let r = mgr.change_passphrase(&store, &old, &new, private);
                    let drew = drawn.load(Ordering::Relaxed) - drawn_before;
                    let after = change_rows(&store);
                    assert_eq!(mgr.is_locked(), locked, "{what}: lock state");
                    assert_eq!(before.rest, after.rest, "{what}: other rows");
                    if old == *current && private && new.is_empty() {
                        // QK-0768 (decided 2026-11-22): Go accepts this;
                        // dcr-purse refuses it with creation's error and
                        // writes nothing.
                        let err = r.expect_err("an empty new private passphrase");
                        assert!(err.is_kind(Kind::Invalid), "{what}: {err}");
                        assert_eq!(
                            err.to_string(),
                            "invalid operation: private passphrase may not be empty"
                        );
                        assert_eq!(before, after, "{what}: a refused change wrote");
                        assert_eq!(drew, 0, "{what}: a refused change drew");
                        refused += 1;
                    } else if old == *current {
                        r.unwrap_or_else(|e| panic!("{what}: {e}"));
                        changes[usize::from(private)] += 1;
                        // Go's sizes: the salt (32), for the private
                        // passphrase the hasher key (32), the nonce (24).
                        let want = if private { 32 + 32 + 24 } else { 32 + 24 };
                        assert_eq!(drew, want, "{what}: a change's draws");
                        let (kept, changed, was) = if private {
                            (&before.public, &after.private, &before.private)
                        } else {
                            (&before.private, &after.public, &before.public)
                        };
                        assert_eq!(
                            kept,
                            if private {
                                &after.public
                            } else {
                                &after.private
                            },
                            "{what}: the other half"
                        );
                        assert_ne!(changed.0, was.0, "{what}: new parameters");
                        assert_ne!(changed.1, was.1, "{what}: new sealed key");
                        let p = changed.0.expect("parameters");
                        assert_eq!((p.n, p.r, p.p), (2, 1, 1), "{what}: simnet's cost");
                        let cur = if private { &mut privpass } else { &mut pubpass };
                        earlier.push(std::mem::replace(cur, new));
                    } else {
                        let err = r.expect_err("a wrong old passphrase");
                        assert!(err.is_kind(Kind::Passphrase), "{what}: {err}");
                        assert_eq!(err.to_string(), "secretkey.DeriveKey: invalid passphrase");
                        assert_eq!(before, after, "{what}: a failed change wrote");
                        assert_eq!(drew, 0, "{what}: a failed change drew");
                    }
                    if !locked {
                        mgr.unlocked_with_passphrase(&privpass)
                            .unwrap_or_else(|e| panic!("{what}: {e}"));
                        check_private(&mgr, &what);
                    }
                    let tx = store.read().expect("read");
                    assert_eq!(
                        mgr.account_has_passphrase(&mut &tx, 1),
                        (true, acct1),
                        "{what}: account 1"
                    );
                    if acct1 {
                        mgr.unlock_account(&mut &tx, 1, b"acct")
                            .unwrap_or_else(|e| panic!("{what}: account 1 again: {e}"));
                    }
                }
                3 => {
                    // An unlock with the current passphrase or an earlier
                    // one (which fails, and locks an unlocked wallet).
                    let pass = if earlier.is_empty() || rng.below(2) == 0 {
                        privpass.clone()
                    } else {
                        earlier[usize::try_from(rng.below(earlier.len() as u64)).expect("small")]
                            .clone()
                    };
                    let r = mgr.unlock(&pass);
                    if pass == privpass {
                        r.unwrap_or_else(|e| panic!("{what}: {e}"));
                        locked = false;
                        check_private(&mgr, &what);
                    } else {
                        assert!(r.expect_err("an old passphrase").is_kind(Kind::Passphrase));
                        locked = true;
                        acct1 = false;
                    }
                }
                4 => {
                    if locked {
                        let tx = store.read().expect("read");
                        mgr.unlock_account(&mut &tx, 1, b"acct")
                            .unwrap_or_else(|e| panic!("{what}: {e}"));
                        acct1 = true;
                    } else {
                        mgr.lock().expect("lock");
                        locked = true;
                        acct1 = false;
                    }
                }
                _ => {
                    for old in earlier.iter().filter(|p| **p != pubpass) {
                        let tx = store.read().expect("read");
                        let err = Manager::open(
                            &mut &tx,
                            &chain,
                            old,
                            rng_source(&mut rng),
                            Limits::DEFAULT,
                        )
                        .map(|_| ())
                        .expect_err("an earlier passphrase");
                        // An earlier private passphrase may equal the
                        // current public one only by the filter above.
                        assert!(err.is_kind(Kind::Passphrase), "{what}: {err}");
                    }
                    let tx = store.read().expect("read");
                    mgr = Manager::open(
                        &mut &tx,
                        &chain,
                        &pubpass,
                        counted_source(&mut rng, &drawn),
                        Limits::DEFAULT,
                    )
                    .unwrap_or_else(|e| panic!("{what}: reopen: {e}"));
                    locked = true;
                    acct1 = false;
                }
            }
        }
        // The passphrases the model ends with are the store's.
        drop(mgr);
        let tx = store.read().expect("read");
        let mgr = Manager::open(
            &mut &tx,
            &chain,
            &pubpass,
            rng_source(&mut rng),
            Limits::DEFAULT,
        )
        .expect("open with the current public passphrase");
        drop(tx);
        mgr.unlock(&privpass).expect("unlock with the current one");
        check_private(&mgr, &format!("case {case}: end"));
    }
    // Both halves were changed.
    assert!(changes.iter().all(|n| *n > 0), "{changes:?}");
    eprintln!(
        "successful changes (public, private): {changes:?}; empty new private passphrases \
         refused: {refused}"
    );
}
