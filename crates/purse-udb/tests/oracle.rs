// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The live differential of the address manager against dcrwallet
//! (ADR-0002 section 6): each operation script runs through Go's
//! `tools/oracle/cmd/oprunner` (dcrwallet's own udb on bbolt) and through
//! this crate's runner (`tests/support/runner.rs`, over `purse-store`), and
//!
//! 1. every operation's record must be equal: the same value, or an error
//!    of the same kind and text (`drawn` aside: the two sides' random
//!    streams are not aligned yet, ADR-0002 open item 2);
//! 2. the two final logical exports must be equal, section by section,
//!    under the masked observable profile (ADR-0001 section 8: sealed
//!    values, salts and digests masked, write-only fields projected out),
//!    for every section the Rust producer covers, with every `[privcheck]`
//!    verdict `yes` on both sides, which is what makes the masked profile
//!    a proof that the sealed secrets are the same keys.
//!
//! The hand-written scripts are `tests/data/opscripts/*.json`; the generated
//! ones (`tests/support/generator.rs`) vary the network (all four), the coin type
//! (half of them upgrade to SLIP0044 first), seed (16 to 64 bytes),
//! passphrases and the order of operations, from `PURSE_TEST_SEED`
//! (`PURSE_UDB_SCRIPTS` sets how many). They draw account, address and
//! import arguments from small sets, so that names collide, imported keys
//! are recorded again as derived addresses, and lookups hit and miss; they
//! never pass an account Go's marking methods panic on
//! (`divergences_from_go_are_as_recorded` covers those), and never upgrade
//! the coin type after recording an address of account 0
//! (`coin_type_upgrade_rederives_legacy_rows` covers that, QK-0760).
//! Generated watching-only scripts, generated passphrase scripts (per-
//! account passphrases and imported voting accounts) and generated
//! passphrase-change scripts run the same way (`PURSE_UDB_WO_SCRIPTS`,
//! `PURSE_UDB_PASS_SCRIPTS`, `PURSE_UDB_CHANGE_SCRIPTS`), each family
//! excluding exactly the inputs of the divergences its pinning tests cover.
//!
//! The transaction store's first slice runs the same way: its hand-written
//! scripts (`28` to `31`), generated scripts (`tests/support/txgen.rs`,
//! `PURSE_UDB_TX_SCRIPTS`) and the same with operations in `update` and
//! `view` groups (`PURSE_UDB_TX_GROUP_SCRIPTS`), never tolerating a kind
//! dcr-purse returns where Go panics, which the Rust runner refuses
//! (`a_group_member_may_not_tolerate_a_panic_stand_in`);
//! `an_unmined_coinbase_conflicts_with_votes` pins QK-0780 (coinbases and
//! treasury spends) and QK-0087's dropped treasury spend, and
//! `a_transaction_mined_twice_and_rolled_back_is_mined_and_unmined`
//! QK-0781. The credit slice adds the hand-written scripts `32` to `34`,
//! generated scripts that draw its operations too
//! (`generate_txstore_credits`, `PURSE_UDB_TX_CREDIT_SCRIPTS`, and in
//! groups `PURSE_UDB_TX_CREDIT_GROUP_SCRIPTS`), which avoid by construction
//! the sequences of its divergences (QK-0082 and QK-0081 with QK-0782,
//! decided; QK-0101 and QK-0783, pending owner), four pinning tests of
//! those divergences,
//! `a_same_block_spend_chain_invalidated_leaves_no_dangling_unspent`
//! (QK-0082), `credits_off_the_main_chain_or_without_a_record_are_refused`
//! (QK-0783), `invalidated_credits_of_a_detached_block_do_not_come_back`
//! (QK-0782) and
//! `a_spender_below_its_funder_is_refused_where_go_fabricates_a_credit`
//! (QK-0101), and two of Go defects it reproduces, every record and the
//! export equal: `a_revalidated_transaction_also_unmined_is_counted_twice`
//! (QK-0734, QK-0736) and
//! `a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again`
//! (QK-0785).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts, lengths and generator sizes"
)]

mod support;

use std::collections::{BTreeMap, BTreeSet};

use dcroxide_txscript::stdaddr;
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, oracle_or_skip};
use purse_udb::addrmgr::ChainParams;
use support::compare::{comparable, corpus, differential, go_run};
use support::generator::{
    CHANGE_OPS, GEN_OPS, PASS_OPS, Shape, WATCH_OPS, change_kind, generate, generate_change,
    generate_passphrase, generate_watching, watch_kind,
};
use support::runner::{self, SUPPORTED};
use support::scratch;
use support::txgen::{
    CREDIT_OPS, TX_OPS, generate_txstore, generate_txstore_credits, group_txstore,
};

#[test]
fn hand_written_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    // PURSE_UDB_SCRIPT, when set, runs only the scripts whose name holds
    // it (a debugging aid; the full corpus runs without it).
    let only = std::env::var("PURSE_UDB_SCRIPT").ok();
    for (name, script) in corpus() {
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let n = differential(&mut o, &name, &script).len();
        eprintln!("{name}: {n} operations equal");
    }
}

/// Every operation the Rust runner implements is one of Go's, and the
/// hand-written corpus runs every one of them.
#[test]
fn supported_operations_are_gos_and_all_exercised() {
    let mut used = BTreeSet::new();
    for (_, s) in corpus() {
        for op in s["ops"].as_array().expect("ops") {
            used.insert(op["op"].as_str().expect("op").to_owned());
        }
    }
    for op in SUPPORTED {
        assert!(used.contains(*op), "no hand-written script runs {op}");
    }
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let vocab = o.call("vocabulary", Value::Null);
    let go: BTreeSet<String> = vocab["ops"]
        .as_array()
        .expect("ops")
        .iter()
        .map(|v| v["op"].as_str().expect("op").to_owned())
        .collect();
    for op in SUPPORTED {
        assert!(go.contains(*op), "{op} is not an operation of oprunner");
    }
    let left: Vec<&String> = go
        .iter()
        .filter(|op| op.starts_with("mgr.") || op.starts_with("udb."))
        .filter(|op| !SUPPORTED.contains(&op.as_str()))
        .collect();
    eprintln!(
        "address manager and udb operations not yet ported ({}): {left:?}",
        left.len()
    );
}

#[test]
fn generated_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_UDB_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let mut rng = SplitMix64::from_env("purse-udb-addrmgr-generated");
    let mut ops = 0;
    // (network, coin type at the end) -> scripts.
    let mut tally: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for id in 0..count {
        let script = generate(&mut rng, id, Shape::default());
        let results = differential(&mut o, &format!("generated {id}: {script}"), &script);
        ops += results.len();
        let upgraded = results
            .iter()
            .any(|r| r["op"] == "mgr.upgrade_to_slip0044_coin_type" && r.get("ok").is_some());
        let net = script["setup"]["network"]
            .as_str()
            .expect("network")
            .to_owned();
        *tally
            .entry((net, if upgraded { "slip0044" } else { "legacy" }))
            .or_default() += 1;
    }
    eprintln!(
        "{count} generated scripts, {ops} operations equal; by network and final coin type: {tally:?}"
    );
}

/// Every operation a generated script draws is one the Rust runner
/// implements, and every implemented `mgr.` operation but `chain_params`
/// is drawn: by the seed scripts, by the watching-only ones (public key
/// imports, which only a watching-only wallet accepts, and conversion), by
/// the passphrase ones (per-account passphrases and voting accounts), or by
/// the passphrase-change ones.
#[test]
fn generated_scripts_draw_every_ported_operation() {
    let lists = [GEN_OPS, WATCH_OPS, PASS_OPS, CHANGE_OPS];
    for op in lists.iter().copied().flatten() {
        assert!(SUPPORTED.contains(op), "{op} is drawn but not ported");
    }
    for op in SUPPORTED.iter().filter(|op| op.starts_with("mgr.")) {
        if *op == "mgr.chain_params" {
            continue;
        }
        assert!(
            lists.iter().any(|l| l.contains(op)),
            "{op} is ported but never drawn"
        );
    }
}

/// Passphrase changes: generated scripts on all four networks, on wallets
/// created from a seed (either coin type, with per-account passphrases), from
/// an account extended public key, or from a seed and converted
/// (`PURSE_UDB_CHANGE_SCRIPTS` sets how many).
#[test]
fn generated_change_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_UDB_CHANGE_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let mut rng = SplitMix64::from_env("purse-udb-addrmgr-change");
    let mut ops = 0;
    let mut tally = BTreeMap::new();
    // Changes that succeeded, by half, so a campaign shows it reached them
    // and not only their refusals.
    let mut changed: BTreeMap<&str, usize> = BTreeMap::new();
    for id in 0..count {
        let script = generate_change(&mut rng, id, Shape::default());
        let results = differential(&mut o, &format!("generated {id}: {script}"), &script);
        ops += results.len();
        let script_ops = script["ops"].as_array().expect("ops");
        for r in &results {
            if r["op"] == "mgr.change_passphrase" && r.get("err").is_none() {
                let i = usize::try_from(r["i"].as_u64().expect("i")).expect("index");
                let half = if script_ops[i]["private"] == true {
                    "private"
                } else {
                    "public"
                };
                *changed.entry(half).or_default() += 1;
            }
        }
        let net = script["setup"]["network"]
            .as_str()
            .expect("network")
            .to_owned();
        *tally.entry((net, change_kind(&script))).or_insert(0usize) += 1;
    }
    eprintln!(
        "{count} generated passphrase-change scripts, {ops} operations equal; by network and \
         kind: {tally:?}; successful changes: {changed:?}"
    );
}

/// Per-account passphrases and imported voting accounts: generated
/// scripts on all four networks, on wallets created from a seed (either
/// coin type) or from an account extended public key
/// (`PURSE_UDB_PASS_SCRIPTS` sets how many).
#[test]
fn generated_passphrase_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_UDB_PASS_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);
    let mut rng = SplitMix64::from_env("purse-udb-addrmgr-passphrase");
    let mut ops = 0;
    let mut tally: BTreeMap<(String, &str), usize> = BTreeMap::new();
    // Successful calls of each passphrase operation, so a campaign shows it
    // reached them and not only their refusals.
    let mut succeeded: BTreeMap<String, usize> = BTreeMap::new();
    for id in 0..count {
        let script = generate_passphrase(&mut rng, id, Shape::default());
        let results = differential(&mut o, &format!("generated {id}: {script}"), &script);
        ops += results.len();
        for r in &results {
            let op = r["op"].as_str().expect("op");
            if PASS_OPS.contains(&op) && r.get("err").is_none() {
                *succeeded.entry(op.to_owned()).or_default() += 1;
            }
        }
        let net = script["setup"]["network"]
            .as_str()
            .expect("network")
            .to_owned();
        let kind = if script["setup"]["xpub"].is_null() {
            "seed"
        } else {
            "xpub"
        };
        *tally.entry((net, kind)).or_default() += 1;
    }
    eprintln!(
        "{count} generated passphrase scripts, {ops} operations equal; by network and wallet: \
         {tally:?}; successful passphrase operations: {succeeded:?}"
    );
}

/// Watching-only wallets: generated scripts of every kind (created from
/// an account extended public key of either coin type, or created from a
/// seed on either coin type and converted) on all four networks
/// (`PURSE_UDB_WO_SCRIPTS` sets how many).
#[test]
fn generated_watching_only_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var("PURSE_UDB_WO_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let mut rng = SplitMix64::from_env("purse-udb-addrmgr-watching");
    let mut ops = 0;
    let mut tally = BTreeMap::new();
    for id in 0..count {
        let script = generate_watching(&mut rng, id, Shape::default());
        let results = differential(&mut o, &format!("generated {id}: {script}"), &script);
        ops += results.len();
        let net = script["setup"]["network"]
            .as_str()
            .expect("network")
            .to_owned();
        *tally.entry((net, watch_kind(&script))).or_insert(0usize) += 1;
    }
    eprintln!(
        "{count} generated watching-only scripts, {ops} operations equal; by network and kind: {tally:?}"
    );
}

/// A setup Go refuses fails both runs, with the same text.
#[test]
fn refused_setups_fail_both_runs_alike() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let base = json!({
        "format": "purse-opscript 1",
        "name": "refused setup",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "0505050505050505050505050505050505050505050505050505050505050505",
                  "clock": 1_794_990_000},
        "ops": [],
    });
    // An extended key passed unparsed to InitializeWatchOnly: empty,
    // malformed, a character outside base58's alphabet, another network's,
    // a bad checksum.
    let raw = |k: &str| json!({"raw": k});
    let mainnet_xpub = "dpubZF4LSCdF9YKZfNzTVYhz4RBxsjYXqms8AQnMBHXZ8GUKoRSigG7kQnKiJt5pzk93Q8FxcdVBEkQZruSXduGtWnkwXzGnjbSovQ97dCxqaXc";
    let bad_checksum = "sprvZGFP8jrEcU3JA2qJYKSGTFf9oVXhfnc7invEQZuLUbuztXS6MnzkzQYse5N1mn1E7mn9vvG6BkSP3AkR9JzSKLKqMmL4498yABFQffCcCVa";
    for (field, value, want) in [
        (
            "privpass",
            json!(""),
            "invalid operation: private passphrase may not be empty",
        ),
        (
            "seed",
            json!("000102030405060708090a0b0c0d0e"),
            "seed length must be between 128 and 512 bits",
        ),
        (
            "xpub",
            raw(""),
            "the provided serialized extended key length is invalid",
        ),
        (
            "xpub",
            raw("xyz"),
            "the provided serialized extended key length is invalid",
        ),
        (
            "xpub",
            raw("\u{20ac}"),
            "the provided serialized extended key length is invalid",
        ),
        (
            "xpub",
            raw(mainnet_xpub),
            "the provided serialized extended key is for the wrong network",
        ),
        ("xpub", raw(bad_checksum), "bad extended key checksum"),
    ] {
        let mut s = base.clone();
        if field == "xpub" {
            s["setup"].as_object_mut().expect("setup").remove("seed");
        }
        s["setup"][field] = value;
        let dir = scratch("refused");
        let go = o
            .try_call(
                "run",
                json!({"script": s, "dir": dir.join("go").to_str().expect("utf-8")}),
            )
            .expect_err("Go refuses the setup");
        assert!(go.text.contains(want), "Go: {}", go.text);
        let rust = runner::run(&s, &dir.join("rust")).expect_err("Rust refuses the setup");
        assert!(rust.contains(want), "Rust: {rust}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// QK-0762 (ADR-0012 Rules 6 and 11, decided by the project owner on 2026-11-22;
/// PARITY.md, `purse-udb`): a watching-only wallet created from an
/// account's extended private key, on every network and for both coin
/// types' account 0. Go's `InitializeWatchOnly` accepts it, and its
/// `AccountExtendedPubKey` returns the private key's string, before and
/// after a reopen, while the wallet calls itself watching-only and still
/// answers `AccountExtendedPrivKey` and `PrivateKey` with `Locked`;
/// dcr-purse refuses the key with `Invalid` `extended key must be an xpub`
/// and creates nothing.
/// The same script from the neutered key runs on both sides and is equal.
/// (These were corpus script `17-watching-only-private-key`, testnet3's
/// case, until the refusal: a setup one side refuses is no corpus script.)
#[test]
fn watching_only_creation_refuses_an_extended_private_key() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let seed =
        purse_testutil::unhex("1717171717171717171717171717171717171717171717171717171717171717");
    let ops = json!([
        {"op": "mgr.watching_only"},
        {"op": "mgr.account_extended_pub_key", "account": 0},
        {"op": "mgr.account_extended_priv_key", "account": 0},
        {"op": "mgr.account_branch_extended_pub_key", "account": 0, "branch": 0},
        {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 1, "branch": 0},
        {"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 1}},
        {"op": "mgr.private_key", "addr": {"acct": 0, "branch": 0, "index": 1}},
        {"op": "rt.reopen"},
        {"op": "mgr.account_extended_pub_key", "account": 0},
    ]);
    let mut cases = 0;
    for net in ["mainnet", "testnet3", "simnet", "regnet"] {
        let chain = ChainParams::by_name(net).expect("network");
        let keys = purse_udb::addrmgr::hd_keys_from_seed(&seed, &chain).expect("seed");
        for xpriv in [&keys.account0_legacy, &keys.account0_slip0044] {
            let private = xpriv.string().to_string();
            let public = xpriv.neuter().string().to_string();
            let script = |key: &str| {
                json!({
                    "format": "purse-opscript 1",
                    "name": format!("QK-0762: {net} watching-only wallet from {}", &key[..4]),
                    "setup": {"network": net, "xpub": {"raw": key}, "pubpass": "",
                              "privpass": "",
                              "rand": "1717171717171717171717171717171717171717171717171717171717171717",
                              "clock": 1_794_990_000},
                    "ops": ops,
                })
            };
            let dir = scratch("qk0762");
            let (go, _) = go_run(&mut o, &script(&private), &dir.join("go"));
            let rust = runner::run(&script(&private), &dir.join("rust"))
                .expect_err("dcr-purse refuses an extended private key");
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(
                rust, "setup: invalid operation: extended key must be an xpub",
                "{net}"
            );
            assert_eq!(go[0]["ok"], true, "{net}: Go: {}", go[0]);
            for i in [1, 8] {
                assert_eq!(go[i]["ok"], json!(private), "{net}: Go op {i}: {}", go[i]);
            }
            assert_eq!(go[2]["err"]["kind"], "Locked", "{net}: Go: {}", go[2]);
            assert_eq!(go[6]["err"]["kind"], "Locked", "{net}: Go: {}", go[6]);
            let same = differential(&mut o, &format!("{net} {public}"), &script(&public));
            assert_eq!(same[1]["ok"], json!(public), "{net}");
            cases += 1;
        }
    }
    assert_eq!(cases, 8);
}

/// Seeds whose purpose key or legacy coin type key `Child` stores with a
/// leading zero byte stripped, the case where the Decred derivation and
/// plain BIP32 part (QK-0028): about one seed in 256 for each key.
fn stripped_seeds(rng: &mut SplitMix64, want: usize) -> Vec<(Vec<u8>, &'static str)> {
    use purse_udb::addrmgr::ExtendedKey;
    let chain = purse_udb::addrmgr::ChainParams::by_name("simnet").expect("simnet");
    let (mut purpose, mut coin) = (Vec::new(), Vec::new());
    while purpose.len() < want || coin.len() < want {
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
        if p.serialized_priv_key().expect("private").len() < 32 && purpose.len() < want {
            purpose.push((seed, "purpose key stripped"));
        } else if c.serialized_priv_key().expect("private").len() < 32 && coin.len() < want {
            coin.push((seed, "coin type key stripped"));
        }
    }
    purpose.into_iter().chain(coin).collect()
}

#[test]
fn seeds_with_stripped_parents_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-udb-addrmgr-stripped");
    for (seed, what) in stripped_seeds(&mut rng, 4) {
        let script = json!({
            "format": "purse-opscript 1",
            "name": what,
            "setup": {"network": "simnet", "seed": hex(&seed), "pubpass": "public",
                      "privpass": "private", "rand": hex(&rng.bytes(32)),
                      "clock": 1_794_990_000},
            "ops": [
                {"op": "udb.hd_keys_from_seed", "seed": hex(&seed)},
                {"op": "mgr.account_extended_pub_key", "account": 0},
                {"op": "mgr.unlock", "pass": "private"},
                {"op": "mgr.account_extended_priv_key", "account": 0},
                {"op": "mgr.account_branch_extended_pub_key", "account": 0, "branch": 1},
                // QK-0028's other half: NewAccount derives account 1 from
                // the coin type key parsed from its string.
                {"op": "mgr.new_account", "name": "second"},
                {"op": "mgr.account_extended_priv_key", "account": 1},
                {"op": "mgr.sync_account_to_addr_index", "account": 1, "index": 2, "branch": 0},
                {"op": "mgr.for_each_account_address", "account": 1},
            ],
        });
        differential(&mut o, &format!("{what}: {}", hex(&seed)), &script);
    }
}

/// The divergences recorded in PARITY.md (`purse-udb`) where Go panics or
/// records what the store cannot hold: each operation gives Go's result
/// (a recovered panic, recorded by `oprunner` with kind `Panic`) and
/// dcr-purse's, both pinned; every other operation of the script is equal.
/// The exports are not compared: Go's holds the row of a branch-2 address
/// that dcr-purse refuses.
#[test]
fn divergences_from_go_are_as_recorded() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let g = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let g_addr = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
        &stdaddr::hash160(&purse_testutil::unhex(g)),
        &chain,
    )
    .expect("p2pkh")
    .encode();
    let script = json!({
        "format": "purse-opscript 1",
        "name": "divergences",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.mark_used_child_index", "account": 7, "branch": 0, "child": 1},
            {"op": "mgr.mark_returned_child_index", "account": 7, "branch": 1, "child": 1},
            {"op": "mgr.record_derived_address", "account": 99, "branch": 0, "child": 1, "pubkey": g},
            {"op": "mgr.mark_used", "addr": g_addr},
            {"op": "mgr.record_derived_address", "account": 0, "branch": 2, "child": 1,
             "pubkey": "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"},
            {"op": "mgr.account_properties", "account": 0},
            {"op": "mgr.for_each_account_address", "account": 99},
        ],
    });
    // (index, dcr-purse's kind and text); Go's is a panic, or success for
    // the branch-2 record.
    let divergent: [(usize, &str, &str); 4] = [
        (0, "IO", "I/O error: no account 7"),
        (1, "IO", "I/O error: no account 7"),
        (3, "IO", "I/O error: no account 99"),
        (4, "Invalid", "invalid operation: account branch 2"),
    ];
    let dir = support::scratch("divergences");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    assert_eq!(go.len(), rust.results.len());
    for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
        match divergent.iter().find(|d| d.0 == i) {
            Some((_, kind, text)) => {
                if i == 4 {
                    assert_eq!(g["ok"], Value::Null, "Go records any branch: {g}");
                    assert!(g.get("err").is_none(), "Go records any branch: {g}");
                } else {
                    assert_eq!(g["err"]["kind"], "Panic", "Go panics: {g}");
                }
                assert_eq!(r["err"]["kind"], *kind, "op {i}: {r}");
                assert_eq!(r["err"]["text"], *text, "op {i}: {r}");
            }
            None => assert_eq!(comparable(g), comparable(r), "op {i}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// QK-0763 and QK-0764 (PARITY.md, `purse-udb`): after
/// `ConvertToWatchingOnly`, and until a reopen, Go's `PrivateKey` answers
/// from memory that no longer matches its store. For an imported key it
/// dereferences the nil private crypto key and panics (recorded by
/// `oprunner` with kind `Panic`; ADR-0012 Rule 2(a)); for an account it had
/// cached it finds the sealed private key it emptied in memory, which the
/// store keeps (QK-0761), and fails with `no private key for
/// <name>/<branch>/<index>`, where an account it had not cached fails with
/// the locked account's text. dcr-purse answers from the store, as Go does
/// after a reopen. Every other operation and the exports are equal.
#[test]
fn private_keys_after_a_conversion_answer_from_the_store() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0763, QK-0764: PrivateKey after a conversion",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "6363636363636363636363636363636363636363636363636363636363636363",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.new_account", "name": "second"},
            {"op": "mgr.account_extended_priv_key", "account": 1},
            {"op": "mgr.sync_account_to_addr_index", "account": 1, "index": 2, "branch": 0},
            {"op": "mgr.import_private_key",
             "key": "0000000000000000000000000000000000000000000000000000000000000002",
             "label": "k"},
            {"op": "mgr.import_script", "script": "51", "label": "s"},
            {"op": "mgr.convert_to_watching_only"},
            {"op": "mgr.private_key", "addr": {"ref": "k"}},
            {"op": "mgr.import_private_key",
             "key": "0000000000000000000000000000000000000000000000000000000000000003",
             "label": "k3"},
            {"op": "mgr.private_key", "addr": {"ref": "k3"}},
            {"op": "mgr.private_key", "addr": {"ref": "s"}},
            {"op": "mgr.private_key", "addr": {"acct": 1, "branch": 0, "index": 2}},
            // Account 0 is first loaded (and, by Go, cached) after the
            // conversion.
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 0, "branch": 0},
            {"op": "mgr.private_key", "addr": {"acct": 0, "branch": 0, "index": 0}},
            {"op": "rt.reopen"},
            {"op": "mgr.private_key", "addr": {"ref": "k"}},
            {"op": "mgr.private_key", "addr": {"ref": "k3"}},
            {"op": "mgr.private_key", "addr": {"acct": 1, "branch": 0, "index": 2}},
        ],
    });
    let dir = support::scratch("qk0763");
    let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    let r = &rust.results;
    let missing_nonce = "encryption/decryption error: decrypt imported privkey: \
                         cryptokey.Decrypt: invalid operation: missing nonce";
    // QK-0763: an imported key, imported before and after the conversion.
    for i in [7, 9] {
        assert_eq!(go[i]["err"]["kind"], "Panic", "Go panics: {}", go[i]);
        assert_eq!(r[i]["err"]["kind"], "Crypto", "{}", r[i]);
        assert_eq!(r[i]["err"]["text"], missing_nonce);
    }
    // QK-0764: account 1, cached before the conversion.
    assert_eq!(go[11]["err"]["kind"], "Other", "Go: {}", go[11]);
    assert_eq!(go[11]["err"]["text"], "no private key for second/0/2");
    assert_eq!(r[11]["err"]["kind"], "Locked", "{}", r[11]);
    assert_eq!(r[11]["err"]["text"], "wallet locked: second");
    // Account 0, which Go had not cached, and everything after the reopen.
    assert_eq!(
        go[13]["err"]["text"], "wallet locked: default",
        "Go: {}",
        go[13]
    );
    for i in [15, 16] {
        assert_eq!(go[i]["err"]["text"], missing_nonce, "Go: {}", go[i]);
    }
    assert_eq!(
        go[17]["err"]["text"], "wallet locked: second",
        "Go: {}",
        go[17]
    );
    for (i, (g, rr)) in go.iter().zip(r).enumerate() {
        if [7, 9, 11].contains(&i) {
            continue;
        }
        assert_eq!(comparable(g), comparable(rr), "op {i}");
    }
    support::compare::compare_exports("QK-0763, QK-0764", &go_pe, &rust.export);
}

/// QK-0760 (ADR-0012 Rule 7; PARITY.md, `purse-udb`): the SLIP0044 upgrade
/// of a wallet whose account 0 recorded addresses under the legacy coin
/// type, then a restart. Go keeps the legacy rows, so it still claims each
/// legacy address and answers for it with the SLIP0044 child at the same
/// path, another address with another key, while the SLIP0044 children at
/// the recorded paths are no addresses of the wallet; a later sync records
/// them a second time in account 0's set, so its walk lists them twice.
/// dcr-purse derives the recorded rows again from the SLIP0044 account key
/// in the upgrade's transaction. Both answers are pinned for each
/// divergent operation; every other operation is equal, and account 0's
/// walk lists the same addresses on both sides. The exports are not
/// compared: Go's holds the legacy rows.
#[test]
fn coin_type_upgrade_rederives_legacy_rows() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let seed = "0f1e2d3c4b5a69788796a5b4c3d2e1f00112233445566778";
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let keys =
        purse_udb::addrmgr::hd_keys_from_seed(&purse_testutil::unhex(seed), &chain).expect("seed");
    let path_key = |acct: &purse_udb::addrmgr::ExtendedKey, branch: u32, index: u32| {
        acct.neuter()
            .child(branch)
            .and_then(|b| b.child(index))
            .expect("child")
    };
    let p2pkh = |k: &purse_udb::addrmgr::ExtendedKey| {
        stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
            &stdaddr::hash160(k.serialized_pub_key()),
            &chain,
        )
        .expect("p2pkh")
        .encode()
    };
    let legacy_ext1 = path_key(&keys.account0_legacy, 0, 1);
    let legacy_int0 = path_key(&keys.account0_legacy, 1, 0);
    let slip_ext1 = p2pkh(&path_key(&keys.account0_slip0044, 0, 1));
    let slip_int0 = path_key(&keys.account0_slip0044, 1, 0);
    let g = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0760: SLIP0044 upgrade after legacy addresses were recorded",
        "setup": {"network": "simnet", "seed": seed, "pubpass": "", "privpass": "private",
                  "rand": "1111111111111111111111111111111111111111111111111111111111111111",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 3, "branch": 0},
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 1, "branch": 1},
            {"op": "mgr.import_private_key",
             "key": "0000000000000000000000000000000000000000000000000000000000000001"},
            // QK-0757: the imported account's set keeps its entry.
            {"op": "mgr.record_derived_address", "account": 0, "branch": 0, "child": 50,
             "pubkey": g},
            {"op": "mgr.for_each_account_address", "account": 0},
            {"op": "mgr.upgrade_to_slip0044_coin_type"},
            {"op": "rt.reopen"},
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.address", "addr": p2pkh(&legacy_ext1)},
            {"op": "mgr.private_key", "addr": p2pkh(&legacy_ext1)},
            {"op": "mgr.addr_account", "addr": p2pkh(&legacy_int0)},
            {"op": "mgr.exists_hash160",
             "hash160": hex(&stdaddr::hash160(legacy_ext1.serialized_pub_key()))},
            {"op": "mgr.exists_hash160",
             "hash160": hex(&stdaddr::hash160(&purse_testutil::unhex(g)))},
            {"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 1}},
            {"op": "mgr.private_key", "addr": {"acct": 0, "branch": 1, "index": 0}},
            {"op": "mgr.for_each_account_address", "account": 0},
            {"op": "mgr.for_each_account_address", "account": 0x7FFF_FFFF},
            {"op": "mgr.account_properties", "account": 0x7FFF_FFFF},
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 5, "branch": 0},
            {"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 5}},
            {"op": "mgr.address", "addr": {"acct": 0, "branch": 0, "index": 4}},
            {"op": "mgr.for_each_account_address", "account": 0},
        ],
    });
    let dir = support::scratch("qk0760");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    let r = &rust.results;
    let not_exist = |v: &Value, text: &str| {
        assert_eq!(v["err"]["kind"], "NotExist", "{v}");
        assert_eq!(v["err"]["text"], text, "{v}");
    };
    let addrs = |v: &Value| -> Vec<String> {
        v["ok"]
            .as_array()
            .unwrap_or_else(|| panic!("a listing: {v}"))
            .iter()
            .map(|m| m["addr"].as_str().expect("addr").to_owned())
            .collect()
    };
    let key_hash = |v: &Value| {
        let pk = v["ok"]["pubkey"]
            .as_str()
            .unwrap_or_else(|| panic!("a key: {v}"));
        hex(&stdaddr::hash160(&purse_testutil::unhex(pk)))
    };
    // 9: Go answers for the legacy address with the SLIP0044 child.
    assert_eq!(go[9]["ok"]["addr"], slip_ext1.as_str(), "Go: {}", go[9]);
    not_exist(
        &r[9],
        &format!("item does not exist: no address {}", p2pkh(&legacy_ext1)),
    );
    // 10: Go gives the legacy address another address's key.
    let legacy_hash = hex(&stdaddr::hash160(legacy_ext1.serialized_pub_key()));
    assert_ne!(key_hash(&go[10]), legacy_hash, "Go: {}", go[10]);
    not_exist(
        &r[10],
        &format!("item does not exist: no address with id {legacy_hash}"),
    );
    // 11-13: Go claims the legacy address and the shadowed key's HASH160.
    assert_eq!(go[11]["ok"], 0, "Go: {}", go[11]);
    not_exist(
        &r[11],
        &format!("item does not exist: no address {}", p2pkh(&legacy_int0)),
    );
    for i in [12, 13] {
        assert_eq!(go[i]["ok"], true, "Go: {}", go[i]);
        assert_eq!(r[i]["ok"], false, "op {i}: {}", r[i]);
    }
    // 14-15: the SLIP0044 children at recorded paths are the wallet's here.
    assert_eq!(go[14]["err"]["kind"], "NotExist", "Go: {}", go[14]);
    assert_eq!(r[14]["ok"]["addr"], slip_ext1.as_str(), "{}", r[14]);
    assert_eq!(go[15]["err"]["kind"], "NotExist", "Go: {}", go[15]);
    assert_eq!(
        key_hash(&r[15]),
        hex(&stdaddr::hash160(slip_int0.serialized_pub_key())),
        "{}",
        r[15]
    );
    // 16: the same addresses, in each side's set order.
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(sorted(addrs(&go[16])), sorted(addrs(&r[16])));
    // 17-18: the imported account's stale entry (QK-0757) is gone here.
    assert_eq!(addrs(&go[17]).len(), 1, "Go: {}", go[17]);
    assert_eq!(addrs(&r[17]).len(), 0, "{}", r[17]);
    assert_eq!(go[18]["ok"]["importedkeys"], 1, "Go: {}", go[18]);
    assert_eq!(r[18]["ok"]["importedkeys"], 0, "{}", r[18]);
    // 22: Go lists the children recorded by both rows twice.
    let (g22, r22) = (addrs(&go[22]), addrs(&r[22]));
    let unique = |v: &[String]| v.iter().collect::<BTreeSet<_>>().len();
    assert_eq!(unique(&r22), r22.len(), "{}", r[22]);
    assert!(unique(&g22) < g22.len(), "Go: {}", go[22]);
    assert_eq!(
        g22.iter().collect::<BTreeSet<_>>(),
        r22.iter().collect::<BTreeSet<_>>()
    );
    for (i, (g, rr)) in go.iter().zip(r).enumerate() {
        if [9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 22].contains(&i) {
            continue;
        }
        assert_eq!(comparable(g), comparable(rr), "op {i}");
    }
}

/// QK-0765 and QK-0766 (PARITY.md, `purse-udb`): per-account keys after
/// `ConvertToWatchingOnly`, until a reopen. Go locks the manager only if
/// the wallet is unlocked, so a key `UnlockAccount` opened while the wallet
/// was locked (a BIP0044 account with its own passphrase, a voting account)
/// stays in memory and keeps answering; it empties the sealed private key
/// of every cached account, so a cached voting account then fails to
/// unlock (`ciphertext too short (len 0)`) while the store keeps its key
/// (QK-0761); and `ImportVotingAccount` seals with the nil private crypto
/// key and panics (recorded by `oprunner` with kind `Panic`). dcr-purse
/// locks every account at a conversion and answers from the store, as Go
/// does after a reopen. Every other operation is equal; the exports are
/// not compared (dcr-purse's holds the account Go's panic did not import).
#[test]
fn account_keys_after_a_conversion_answer_from_the_store() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let key = |i: u32| {
        json!({"seed": "2525252525252525252525252525252525252525252525252525252525252525",
                              "path": ["44'", "115'", format!("{i}'")]})
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0765, QK-0766: account keys after a conversion",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "6565656565656565656565656565656565656565656565656565656565656565",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.new_account", "name": "one"},
            {"op": "mgr.set_account_passphrase", "account": 1, "pass": "one-pass"},
            {"op": "mgr.import_voting_account", "name": "vote", "pass": "abc", "xpriv": key(1)},
            {"op": "mgr.import_voting_account", "name": "cached", "pass": "c", "xpriv": key(2)},
            // Caches account 0x80000001 (locked) on both sides.
            {"op": "mgr.account_has_passphrase", "account": 0x8000_0001_u32},
            {"op": "mgr.lock"},
            {"op": "mgr.unlock_account", "account": 1, "pass": "one-pass"},
            {"op": "mgr.unlock_account", "account": 0x8000_0000_u32, "pass": "abc"},
            {"op": "mgr.convert_to_watching_only"},
            {"op": "mgr.account_has_passphrase", "account": 1},
            {"op": "mgr.account_extended_priv_key", "account": 1},
            {"op": "mgr.account_has_passphrase", "account": 0x8000_0000_u32},
            {"op": "mgr.account_extended_priv_key", "account": 0x8000_0000_u32},
            {"op": "mgr.unlock_account", "account": 0x8000_0001_u32, "pass": "c"},
            {"op": "mgr.import_voting_account", "name": "late", "pass": "l", "xpriv": key(3)},
            {"op": "rt.reopen"},
            {"op": "mgr.account_has_passphrase", "account": 1},
            {"op": "mgr.account_extended_priv_key", "account": 1},
            {"op": "mgr.account_has_passphrase", "account": 0x8000_0000_u32},
            {"op": "mgr.unlock_account", "account": 0x8000_0001_u32, "pass": "c"},
        ],
    });
    let dir = support::scratch("qk0766");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    let r = &rust.results;
    let locked_pair = json!({"has": true, "unlocked": false});
    let unlocked_pair = json!({"has": true, "unlocked": true});
    // QK-0766: the keys UnlockAccount opened while the wallet was locked.
    for i in [10, 12] {
        assert_eq!(go[i]["ok"], unlocked_pair, "Go: {}", go[i]);
        assert_eq!(r[i]["ok"], locked_pair, "{}", r[i]);
    }
    for i in [11, 13] {
        assert!(
            go[i]["ok"].as_str().is_some_and(|k| k.starts_with("sprv")),
            "Go: {}",
            go[i]
        );
    }
    assert_eq!(r[11]["err"]["kind"], "Locked", "{}", r[11]);
    assert_eq!(
        r[11]["err"]["text"],
        "wallet locked: unable to access account extended privkey"
    );
    assert_eq!(r[13]["err"]["kind"], "Invalid", "{}", r[13]);
    assert_eq!(
        r[13]["err"]["text"],
        "invalid operation: imported xpub account has no extended privkey"
    );
    // QK-0766: the cached voting account's emptied sealed key.
    assert_eq!(go[14]["err"]["kind"], "Crypto", "Go: {}", go[14]);
    assert_eq!(
        go[14]["err"]["text"],
        "encryption/decryption error: ciphertext too short (len 0) to encode nonce and MAC tag"
    );
    assert_eq!(r[14]["ok"], Value::Null, "{}", r[14]);
    assert!(r[14].get("err").is_none(), "{}", r[14]);
    // QK-0765: the nil private crypto key.
    assert_eq!(go[15]["err"]["kind"], "Panic", "Go panics: {}", go[15]);
    assert_eq!(r[15]["ok"], 0x8000_0002_u32, "{}", r[15]);
    // After the reopen Go answers as dcr-purse did.
    assert_eq!(go[17]["ok"], locked_pair, "Go: {}", go[17]);
    assert_eq!(go[19]["ok"], locked_pair, "Go: {}", go[19]);
    assert_eq!(go[20]["ok"], Value::Null, "Go: {}", go[20]);
    for (i, (g, rr)) in go.iter().zip(r).enumerate() {
        if (10..=15).contains(&i) {
            continue;
        }
        assert_eq!(comparable(g), comparable(rr), "op {i}");
    }
}

/// Account `num`'s record in an export's `[accounts]`: its `encpriv` and
/// Argon2id parameters (`kdfsalt`, `kdftime`, `kdfmem`, `kdfthreads`, each
/// `None` when absent) and its decrypted `xpub`.
fn account_record(
    ex: &purse_export_check::Export,
    num: u32,
) -> (String, [Option<String>; 4], String) {
    let s = ex.section("accounts").expect("[accounts]");
    let r = s
        .records_of("account")
        .find(|r| s.value(r, "num") == Some(num.to_string().as_str()))
        .unwrap_or_else(|| panic!("no account {num}"));
    let v = |f: &str| s.value(r, f).filter(|v| *v != "-").map(str::to_owned);
    (
        s.value(r, "encpriv").expect("encpriv").to_owned(),
        [v("kdfsalt"), v("kdftime"), v("kdfmem"), v("kdfthreads")],
        s.value(r, "xpub")
            .expect("xpub")
            .trim_matches('"')
            .to_owned(),
    )
}

/// QK-0761 and QK-0766 (ADR-0012 Rules 6 and 11, decided by the project
/// owner on 2026-11-22; PARITY.md, `purse-udb`): a conversion of a wallet
/// with a BIP0044 account given its own passphrase and an imported voting
/// account (corpus script `23-voting-convert` as it stood before the
/// decision). Go's converted file keeps account 1's private key sealed by
/// that passphrase, which opens it from Go's export alone, though no call
/// of a watching-only wallet can; dcr-purse's has it removed, the Argon2id
/// parameters kept, so every reply is Go's: all operations are equal. The
/// voting account keeps its key on both sides, as a watching-only wallet
/// unlocks it by design. The exports are equal once Go's account 1
/// `encpriv` is replaced by dcr-purse's (and Go's `[privcheck]` verdict for
/// that account, which dcr-purse's exporter, holding no sealed key, does
/// not write, removed).
#[test]
fn a_conversion_removes_keys_bip0044_account_passphrases_seal() {
    use purse_export_check::parse;
    use purse_snacl::{Argon2idParams, Limits};
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let mut script: Value = serde_json_from(
        &std::fs::read_to_string(support::data_dir().join("opscripts/23-voting-convert.json"))
            .expect("corpus script 23"),
    );
    script["name"] = json!("QK-0761, QK-0766: a BIP0044 account passphrase across a conversion");
    // Corpus script 23 without its account passphrase: put it back.
    let ops = script["ops"].as_array_mut().expect("ops");
    assert_eq!(
        ops[3]["op"], "mgr.convert_to_watching_only",
        "corpus script 23 changed"
    );
    ops.insert(
        3,
        json!({"op": "mgr.set_account_passphrase", "account": 1, "pass": "one-pass"}),
    );
    let dir = support::scratch("qk0761");
    let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
        assert_eq!(comparable(g), comparable(r), "op {i}");
    }
    let mut go_ex = parse(&go_pe).expect("Go export");
    let rust_ex = parse(&rust.export).expect("Rust export");
    let (go_priv, go_kdf, xpub) = account_record(&go_ex, 1);
    let (rust_priv, rust_kdf, _) = account_record(&rust_ex, 1);
    // Go's export alone opens account 1's private key with its passphrase.
    let kdf = Argon2idParams {
        salt: purse_testutil::unhex(go_kdf[0].as_deref().expect("salt"))
            .try_into()
            .expect("16 bytes"),
        time: go_kdf[1].as_deref().expect("time").parse().expect("time"),
        memory: go_kdf[2]
            .as_deref()
            .expect("memory")
            .parse()
            .expect("memory"),
        threads: go_kdf[3]
            .as_deref()
            .expect("threads")
            .parse()
            .expect("threads"),
    };
    let key = kdf
        .derive_key_with(b"one-pass", &Limits::DEFAULT)
        .expect("derive");
    let plain = key
        .unseal(&purse_testutil::unhex(&go_priv))
        .expect("Go's key opens");
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let xpriv = purse_udb::addrmgr::ExtendedKey::new_key_from_string(
        std::str::from_utf8(plain.as_bytes()).expect("a key string"),
        &chain.hd,
    )
    .expect("an extended key");
    assert!(xpriv.is_private());
    assert_eq!(xpriv.neuter().string().to_string(), xpub);
    // dcr-purse's holds no key for it, and keeps its parameters.
    // `_`, the export's empty value.
    assert_eq!(rust_priv, "_", "dcr-purse's account 1 encpriv");
    assert_ne!(go_priv, "_", "Go's account 1 encpriv");
    assert!(rust_kdf.iter().all(Option::is_some), "{rust_kdf:?}");
    assert_eq!(rust_kdf[1..], go_kdf[1..], "time, memory, threads");
    // The voting account keeps its key on both sides.
    for ex in [&go_ex, &rust_ex] {
        assert_ne!(account_record(ex, 0x8000_0000).0, "_");
    }
    // Otherwise the exports are equal.
    for sec in &mut go_ex.sections {
        match sec.name.as_str() {
            "accounts" => {
                for r in &mut sec.records {
                    let ty = &sec.types[r.ty];
                    if ty.name == "account" && r.values[ty.field_index("num").expect("num")] == "1"
                    {
                        let f = ty.field_index("encpriv").expect("encpriv");
                        r.values[f].clone_from(&rust_priv);
                    }
                }
            }
            "privcheck" => sec.records.retain(|r| {
                let ty = &sec.types[r.ty];
                !(ty.name == "acctok" && r.values[ty.field_index("num").expect("num")] == "1")
            }),
            _ => {}
        }
    }
    support::compare::compare_parsed("QK-0761, QK-0766", &go_ex, &rust_ex);
}

fn serde_json_from(text: &str) -> Value {
    purse_testutil::serde_json::from_str(text).expect("JSON")
}

/// QK-0767 (ADR-0012 Rule 7; PARITY.md, `purse-udb`): the SLIP0044 upgrade
/// of a wallet whose account 0 has its own passphrase. Go writes the
/// SLIP0044 account 0 row, sealed by the private crypto key, and leaves the
/// account's Argon2id parameters, so account 0 still claims its own
/// passphrase while no passphrase opens its key: `UnlockAccount` fails,
/// the wallet's unlock skips it, the passphrase cannot be removed, and
/// after a restart nothing signs for account 0's addresses. dcr-purse's
/// upgrade writes the row without the parameters: account 0 is protected by
/// the wallet passphrase, as the parked row was, and reports the drop
/// (`{"passphrase_dropped": true}`, decided by the project owner on
/// 2026-11-22), which `oprunner` states from Go's state before the call, so
/// the upgrade's own record is equal. Both answers are pinned for each
/// divergent operation; every other operation is equal. The exports are
/// not compared: Go's keeps account 0's parameters. A second script, the
/// same upgrade after the passphrase was removed again, reports nothing on
/// either side.
#[test]
fn coin_type_upgrade_drops_account_0s_own_passphrase() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0767: SLIP0044 upgrade of an account 0 with its own passphrase",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "6666666666666666666666666666666666666666666666666666666666666666",
                  "clock": 1_794_990_000},
        "export": {"privcheck": false},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.set_account_passphrase", "account": 0, "pass": "zero"},
            {"op": "mgr.upgrade_to_slip0044_coin_type"},
            {"op": "mgr.account_has_passphrase", "account": 0},
            {"op": "mgr.account_extended_priv_key", "account": 0},
            {"op": "mgr.lock"},
            {"op": "mgr.unlock_account", "account": 0, "pass": "zero"},
            {"op": "rt.reopen"},
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.account_has_passphrase", "account": 0},
            {"op": "mgr.account_extended_priv_key", "account": 0},
            {"op": "mgr.unlock_account", "account": 0, "pass": "zero"},
            {"op": "mgr.sync_account_to_addr_index", "account": 0, "index": 0, "branch": 0},
            {"op": "mgr.private_key", "addr": {"acct": 0, "branch": 0, "index": 0}},
            {"op": "mgr.set_account_passphrase", "account": 0, "pass": ""},
        ],
    });
    let dir = support::scratch("qk0767");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    let r = &rust.results;
    let none = json!({"has": false, "unlocked": false});
    let not_unique =
        "encryption/decryption error: account is not encrypted with a unique passphrase";
    // 2: the upgrade succeeds on both sides, and reports the drop.
    for (side, rec) in [("Go", &go[2]), ("dcr-purse", &r[2])] {
        assert_eq!(
            rec["ok"],
            json!({"passphrase_dropped": true}),
            "{side}: {rec}"
        );
    }
    // 3: live, Go's cache keeps the parameters and the opened key.
    assert_eq!(
        go[3]["ok"],
        json!({"has": true, "unlocked": true}),
        "Go: {}",
        go[3]
    );
    assert_eq!(r[3]["ok"], none, "{}", r[3]);
    // 6 and 11: no passphrase opens Go's key; dcr-purse's has none.
    for i in [6, 11] {
        assert_eq!(go[i]["err"]["kind"], "Passphrase", "Go: {}", go[i]);
        assert_eq!(r[i]["err"]["kind"], "Crypto", "{}", r[i]);
        assert_eq!(r[i]["err"]["text"], not_unique);
    }
    // 9 and 10: after the restart Go's account 0 is locked for good.
    assert_eq!(
        go[9]["ok"],
        json!({"has": true, "unlocked": false}),
        "Go: {}",
        go[9]
    );
    assert_eq!(r[9]["ok"], none, "{}", r[9]);
    assert_eq!(go[10]["err"]["kind"], "Locked", "Go: {}", go[10]);
    assert_eq!(r[10]["ok"], go[4]["ok"], "{}", r[10]);
    // 13: Go cannot sign for account 0's address.
    assert_eq!(go[13]["err"]["kind"], "Locked", "Go: {}", go[13]);
    assert_eq!(
        go[13]["err"]["text"],
        "wallet locked: account with unique passphrase is locked"
    );
    assert!(r[13]["ok"]["key"].is_string(), "{}", r[13]);
    // 14: nor remove the passphrase.
    assert_eq!(go[14]["err"]["kind"], "Locked", "Go: {}", go[14]);
    assert_eq!(r[14]["ok"], Value::Null, "{}", r[14]);
    assert!(r[14].get("err").is_none(), "{}", r[14]);
    for (i, (g, rr)) in go.iter().zip(r).enumerate() {
        if [3, 6, 9, 10, 11, 13, 14].contains(&i) {
            continue;
        }
        assert_eq!(comparable(g), comparable(rr), "op {i}");
    }
    // A passphrase set and removed before the upgrade: nothing to report,
    // and every operation and the exports are equal.
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0767: SLIP0044 upgrade after account 0's passphrase was removed",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "6666666666666666666666666666666666666666666666666666666666666666",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.set_account_passphrase", "account": 0, "pass": "zero"},
            {"op": "mgr.set_account_passphrase", "account": 0, "pass": ""},
            {"op": "mgr.upgrade_to_slip0044_coin_type"},
            {"op": "mgr.account_has_passphrase", "account": 0},
        ],
    });
    let records = differential(&mut o, "QK-0767: no passphrase to drop", &script);
    assert_eq!(records[3]["ok"], Value::Null, "{}", records[3]);
    assert!(records[3].get("err").is_none(), "{}", records[3]);
}

/// Opens account 0's public (`which` = `pub`) or private (`priv`) field
/// from a logical export with `pass`, as a holder of the wallet file would:
/// the master key of that half from its stored parameters, then the sealed
/// crypto key, then the field. `None` when `pass` does not derive the
/// master key.
fn open_account0_field(pe: &str, which: &str, pass: &[u8]) -> Option<String> {
    use purse_snacl::{CryptoKey, Limits, Parameters, SecretKey};
    let ex = purse_export_check::parse(pe).expect("export parses");
    let row = |section: &str, kind: &str, key: (&str, &str), fields: &[&str]| -> Vec<String> {
        let s = ex.section(section).expect(section);
        let r = s
            .records_of(kind)
            .find(|r| s.value(r, key.0) == Some(key.1))
            .unwrap_or_else(|| panic!("[{section}] {kind} {}={}", key.0, key.1));
        fields
            .iter()
            .map(|f| s.value(r, f).expect("field").to_owned())
            .collect()
    };
    let m = row(
        "masterkeys",
        "masterkey",
        ("which", which),
        &["salt", "digest", "n", "r", "p"],
    );
    let int = |v: &str| v.parse::<i64>().expect("integer");
    let mut master = SecretKey::from_parameters(Parameters {
        salt: purse_testutil::unhex(&m[0]).try_into().expect("salt"),
        digest: purse_testutil::unhex(&m[1]).try_into().expect("digest"),
        n: int(&m[2]),
        r: int(&m[3]),
        p: int(&m[4]),
    });
    master.derive_key_with(pass, &Limits::DEFAULT).ok()?;
    let sealed_crypto = row("cryptokeys", "cryptokey", ("which", which), &["sealed"]);
    let crypto = master
        .decrypt(&purse_testutil::unhex(&sealed_crypto[0]))
        .expect("crypto key opens under its master key");
    let crypto = CryptoKey::from_bytes(crypto.as_bytes().try_into().expect("32 bytes"));
    let field = if which == "priv" { "encpriv" } else { "encpub" };
    let sealed = row("accounts", "account", ("num", "0"), &[field]);
    let plain = crypto
        .decrypt(&purse_testutil::unhex(&sealed[0]))
        .expect("account 0's field opens under its crypto key");
    Some(String::from_utf8(plain.as_bytes().to_vec()).expect("a key string"))
}

/// QK-0037 (reproduced) and QK-0762 (not reproduced, ADR-0012 Rules 6 and 11,
/// decided by the project owner on 2026-11-22): a watching-only wallet's
/// private master key derives from the public passphrase given at
/// creation, and a public `ChangePassphrase` rewrites only the public
/// master key and crypto key. After two public changes the creation's
/// passphrase no longer opens the wallet's public half, but it still opens
/// the private crypto key and with it account 0's private field, which
/// holds the key string the wallet was created from. For a wallet made from
/// the account's `spub` that string is public, and both sides show it from
/// their exports; the operations and the masked exports are equal. For one
/// made from the account's `sprv`, which only Go creates, Go's export gives
/// the `sprv` itself under the creation's passphrase (the copy no call can
/// re-protect, which is why dcr-purse refuses the key; shown here from
/// Go's side, dcr-purse's refusal beside it).
#[test]
fn public_change_leaves_account_0_open_to_the_creation_passphrase() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let keys = purse_udb::addrmgr::hd_keys_from_seed(
        &purse_testutil::unhex("000102030405060708090a0b0c0d0e0f"),
        &chain,
    )
    .expect("seed");
    let sprv = keys.account0_legacy.string().to_string();
    let spub = keys.account0_legacy.neuter().string().to_string();
    assert!(sprv.starts_with("sprv") && spub.starts_with("spub"));
    for key in [&sprv, &spub] {
        let script = json!({
            "format": "purse-opscript 1",
            "name": "QK-0037, QK-0762: a public change leaves account 0's private field",
            "setup": {"network": "simnet", "xpub": {"raw": key},
                      "pubpass": "public", "privpass": "",
                      "rand": "7676767676767676767676767676767676767676767676767676767676767676",
                      "clock": 1_794_990_000},
            "ops": [
                {"op": "mgr.change_passphrase", "old": "public",
                 "new": "a strong public passphrase", "private": false},
                {"op": "mgr.change_passphrase", "old": "a strong public passphrase",
                 "new": "another", "private": false},
                {"op": "rt.reopen"},
                {"op": "mgr.watching_only"},
                {"op": "mgr.account_extended_pub_key", "account": 0},
                {"op": "mgr.change_passphrase", "old": "public", "new": "x", "private": false},
                {"op": "mgr.change_passphrase", "old": "another", "new": "x", "private": true},
            ],
        });
        let dir = support::scratch("qk0037");
        let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
        let rust = runner::run(&script, &dir.join("rust"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(go[4]["ok"], json!(key), "AccountExtendedPubKey: {}", go[4]);
        assert_eq!(go[5]["err"]["kind"], "Passphrase", "{}", go[5]);
        assert_eq!(go[6]["err"]["kind"], "WatchingOnly", "{}", go[6]);
        let mut sides = vec![("Go", go_pe.clone())];
        if key == &sprv {
            // QK-0762: dcr-purse creates no wallet from the private key.
            let err = rust.expect_err("dcr-purse refuses the sprv");
            assert_eq!(
                err,
                "setup: invalid operation: extended key must be an xpub"
            );
        } else {
            let rust = rust.expect("Rust runner");
            assert_eq!(go.len(), rust.results.len());
            for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
                assert_eq!(comparable(g), comparable(r), "op {i}");
            }
            support::compare::compare_exports("QK-0037", &go_pe, &rust.export);
            sides.push(("dcr-purse", rust.export));
        }
        for (side, pe) in &sides {
            assert_eq!(
                open_account0_field(pe, "pub", b"another").as_deref(),
                Some(key.as_str()),
                "{side}: the public field under the current passphrase"
            );
            assert_eq!(
                open_account0_field(pe, "pub", b"public"),
                None,
                "{side}: the creation's passphrase no longer opens the public half"
            );
            assert_eq!(
                open_account0_field(pe, "priv", b"public").as_deref(),
                Some(key.as_str()),
                "{side}: the private field under the creation's passphrase"
            );
            for pass in [&b"another"[..], b"a strong public passphrase"] {
                assert_eq!(
                    open_account0_field(pe, "priv", pass),
                    None,
                    "{side}: a later public passphrase does not open the private half"
                );
            }
        }
    }
}

/// QK-0768 (ADR-0012 Rules 6 and 11, decided by the project owner on
/// 2026-11-22; PARITY.md, `purse-udb`): a change of the private passphrase
/// to the empty one. Go accepts it, after which the empty passphrase
/// unlocks the wallet and the old one does not, across a reopen; dcr-purse
/// refuses it with creation's error and changes nothing, so the old
/// passphrase still unlocks it. Both answers are pinned for each divergent
/// operation, and dcr-purse's refusal draws nothing from the random
/// source where Go's acceptance draws (the records' `drawn`). An empty new
/// private passphrase Go refuses first (a wrong old one) gives Go's error
/// on both sides, as does a public change to the empty passphrase, which
/// both accept; every other operation, and the masked exports (each side's
/// exporter unlocking with its own current passphrase), are equal.
#[test]
fn an_empty_new_private_passphrase_is_refused() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0768: an empty new private passphrase",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "6868686868686868686868686868686868686868686868686868686868686868",
                  "clock": 1_794_990_000},
        "ops": [
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.change_passphrase", "old": "wrong", "new": "", "private": true},
            {"op": "mgr.change_passphrase", "old": "private", "new": "", "private": true},
            {"op": "mgr.unlocked_with_passphrase", "pass": "private"},
            {"op": "mgr.is_locked"},
            {"op": "mgr.lock"},
            {"op": "rt.reopen"},
            {"op": "mgr.unlock", "pass": ""},
            {"op": "mgr.lock"},
            {"op": "mgr.unlock", "pass": "private"},
            {"op": "mgr.change_passphrase", "old": "public", "new": "", "private": false},
            {"op": "rt.reopen"},
            {"op": "mgr.is_locked"},
        ],
    });
    let dir = support::scratch("qk0768");
    let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(go.len(), rust.results.len());
    let r = &rust.results;
    let wrong = "secretkey.DeriveKey: invalid passphrase";
    // 1: a wrong old passphrase is Go's refusal on both sides.
    assert_eq!(go[1]["err"]["kind"], "Passphrase", "Go: {}", go[1]);
    // 2: Go accepts the empty passphrase; dcr-purse refuses it.
    assert_eq!(go[2]["ok"], Value::Null, "Go: {}", go[2]);
    assert!(go[2].get("err").is_none(), "Go accepts: {}", go[2]);
    assert_eq!(r[2]["err"]["kind"], "Invalid", "{}", r[2]);
    assert_eq!(
        r[2]["err"]["text"],
        "invalid operation: private passphrase may not be empty"
    );
    // The refusal comes before any draw: Go's accepted change draws (the
    // salt, the hasher key and the nonce), dcr-purse's refusal nothing.
    // The Rust runner counts what the manager's source hands out, as its
    // draws for the public change (10) show; a wrong old passphrase (1)
    // draws nothing on either side.
    assert!(
        go[2]["drawn"].as_u64().is_some_and(|n| n > 0),
        "Go draws: {}",
        go[2]
    );
    assert!(r[2].get("drawn").is_none(), "dcr-purse drew: {}", r[2]);
    assert!(
        r[10]["drawn"].as_u64().is_some_and(|n| n > 0),
        "the runner counts draws: {}",
        r[10]
    );
    for (side, rec) in [("Go", &go[1]), ("dcr-purse", &r[1])] {
        assert!(rec.get("drawn").is_none(), "{side} drew: {rec}");
    }
    // 3: Go's unlocked wallet now recognises only the empty passphrase.
    assert_eq!(go[3]["err"]["kind"], "Passphrase", "Go: {}", go[3]);
    assert_eq!(r[3]["ok"], Value::Null, "{}", r[3]);
    assert!(r[3].get("err").is_none(), "{}", r[3]);
    // 7 and 9, after a reopen: the empty passphrase opens Go's wallet, the
    // old one dcr-purse's (and 7's failure leaves dcr-purse's locked).
    assert!(
        go[7].get("err").is_none(),
        "Go unlocks with \"\": {}",
        go[7]
    );
    assert_eq!(r[7]["err"]["kind"], "Passphrase", "{}", r[7]);
    // 8: so Go has a wallet to lock and dcr-purse an already locked one.
    assert!(go[8].get("err").is_none(), "Go: {}", go[8]);
    assert_eq!(r[8]["err"]["kind"], "Locked", "{}", r[8]);
    assert_eq!(go[9]["err"]["kind"], "Passphrase", "Go: {}", go[9]);
    assert_eq!(go[9]["err"]["text"], wrong, "Go: {}", go[9]);
    assert!(r[9].get("err").is_none(), "{}", r[9]);
    for (i, (g, rr)) in go.iter().zip(r).enumerate() {
        if [2, 3, 7, 8, 9].contains(&i) {
            continue;
        }
        assert_eq!(comparable(g), comparable(rr), "op {i}");
    }
    support::compare::compare_exports("QK-0768", &go_pe, &rust.export);
}

/// The processor count new Argon2id parameters record (Go's
/// `runtime.NumCPU()`, the script's `setup.ncpu`, `Manager::with_num_cpu`):
/// the masked comparison masks `kdfthreads`, so here each account's
/// parameters are compared unmasked but for the salt. At 256 processors
/// Go's `uint8(min(NumCPU, 256))` stores 0 threads and its Argon2id panics
/// (QK-0630, recorded by `oprunner` with kind `Panic`, the transaction
/// rolled back); dcr-purse returns Argon2id's refusal and writes nothing
/// (ADR-0012 Rule 2(a)). Every other operation and the exports are equal.
#[test]
fn argon2_parameters_record_the_processor_count() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let kdf = |pe: &str| -> Vec<(String, String, String, String)> {
        let ex = purse_export_check::parse(pe).expect("export parses");
        let s = ex.section("accounts").expect("[accounts]");
        s.records_of("account")
            .map(|r| {
                let v = |f: &str| s.value(r, f).unwrap_or_default().to_owned();
                (v("num"), v("kdftime"), v("kdfmem"), v("kdfthreads"))
            })
            .collect()
    };
    for ncpu in [1u32, 3, 255, 256, 300] {
        let script = json!({
            "format": "purse-opscript 1",
            "name": format!("Argon2id parameters at {ncpu} processors"),
            "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "6767676767676767676767676767676767676767676767676767676767676767",
                      "clock": 1_794_990_000, "ncpu": ncpu},
            "ops": [
                {"op": "mgr.unlock", "pass": "private"},
                {"op": "mgr.set_account_passphrase", "account": 0, "pass": "zero"},
                {"op": "mgr.import_voting_account", "name": "vote", "pass": "abc",
                 "xpriv": {"seed": "2525252525252525252525252525252525252525252525252525252525252525",
                           "path": ["44'", "115'", "1'"]}},
                {"op": "mgr.account_has_passphrase", "account": 0},
                {"op": "mgr.last_imported_account"},
            ],
        });
        let dir = support::scratch("ncpu");
        let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
        let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
        let _ = std::fs::remove_dir_all(&dir);
        let r = &rust.results;
        let zero_threads = ncpu >= 256;
        for (i, (g, rr)) in go.iter().zip(r).enumerate() {
            if zero_threads && (i == 1 || i == 2) {
                assert_eq!(g["err"]["kind"], "Panic", "Go panics: {g}");
                assert_eq!(rr["err"]["kind"], "Other", "{rr}");
                assert_eq!(rr["err"]["text"], "argon2: parallelism degree too low");
                continue;
            }
            assert_eq!(comparable(g), comparable(rr), "ncpu {ncpu}: op {i}");
        }
        support::compare::compare_exports(&format!("ncpu {ncpu}"), &go_pe, &rust.export);
        let (gk, rk) = (kdf(&go_pe), kdf(&rust.export));
        assert_eq!(gk, rk, "ncpu {ncpu}: Argon2id parameters");
        let threads: Vec<&str> = gk
            .iter()
            .filter(|k| k.1 != "-")
            .map(|k| k.3.as_str())
            .collect();
        if zero_threads {
            assert!(threads.is_empty(), "ncpu {ncpu}: {gk:?}");
        } else {
            let want = ncpu.to_string();
            assert_eq!(threads, [want.as_str(), want.as_str()], "ncpu {ncpu}");
        }
    }
}

/// Generated transaction store scripts (`tests/support/txgen.rs`) on all
/// four networks, about one in four with the chain sections in digest
/// mode (`PURSE_UDB_TX_SCRIPTS` sets how many; the first four are forced
/// onto each network in turn).
#[test]
fn generated_txstore_scripts_match_go() {
    generated_campaign(
        "PURSE_UDB_TX_SCRIPTS",
        24,
        "purse-udb-txstore-generated",
        generate_txstore,
    );
}

/// Generated transaction store scripts that draw the credit slice's
/// operations too (`generate_txstore_credits`: `AddCredit` unmined and in
/// the block an `InsertMinedTx` of the same group just recorded the
/// transaction in, the UTXO queries, `PreviousPkScripts`, balances and
/// `UnspentTickets`), on all four networks (`PURSE_UDB_TX_CREDIT_SCRIPTS`
/// sets how many).
#[test]
fn generated_txstore_credit_scripts_match_go() {
    generated_campaign(
        "PURSE_UDB_TX_CREDIT_SCRIPTS",
        12,
        "purse-udb-txstore-credits",
        generate_txstore_credits,
    );
}

/// Runs `count` (from `var`, else `default`) scripts of `generate` through
/// both runners, every record and the final exports equal.
fn generated_campaign(
    var: &str,
    default: usize,
    stream: &str,
    generate: fn(&mut SplitMix64, usize, Option<&'static str>) -> Value,
) {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    let mut rng = SplitMix64::from_env(stream);
    let nets = ["simnet", "regnet", "testnet3", "mainnet"];
    let mut ops = 0;
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for id in 0..count {
        let script = generate(&mut rng, id, nets.get(id).copied());
        let results = differential(
            &mut o,
            &format!("{} {id}: {script}", script["name"].as_str().unwrap_or("?")),
            &script,
        );
        *tally
            .entry(
                script["setup"]["network"]
                    .as_str()
                    .expect("network")
                    .to_owned(),
            )
            .or_default() += 1;
        let mut tally_one = |r: &Value| {
            let k = match r.get("err") {
                Some(e) => format!(
                    "{} err {}",
                    r["op"].as_str().unwrap_or("?"),
                    e["kind"].as_str().unwrap_or("?")
                ),
                None => format!("{} ok", r["op"].as_str().unwrap_or("?")),
            };
            *kinds.entry(k).or_default() += 1;
        };
        for r in &results {
            ops += 1;
            tally_one(r);
            for m in r["ops"].as_array().into_iter().flatten() {
                ops += 1;
                tally_one(m);
            }
        }
    }
    eprintln!(
        "{count} generated scripts ({stream}), {ops} operations equal; by network: {tally:?}"
    );
    eprintln!("outcomes: {kinds:?}");
}

/// Generated transaction store scripts with runs of their operations in
/// `update` and `view` groups, some members tolerating `Exist`,
/// `DoubleSpend` or `NotExist` (`group_txstore`), on all four networks
/// (`PURSE_UDB_TX_GROUP_SCRIPTS` sets how many). Go runs a group in one
/// transaction and keeps a tolerated error's writes, the Rust runner one
/// transaction per member, putting the store back when the group rolls
/// back; equal exports show that every tolerated failure wrote nothing in
/// Go either, and that the group's rollback matches.
#[test]
fn generated_txstore_group_scripts_match_go() {
    group_campaign(
        "PURSE_UDB_TX_GROUP_SCRIPTS",
        8,
        "purse-udb-txstore-groups",
        generate_txstore,
    );
}

/// The same with the credit scripts (`PURSE_UDB_TX_CREDIT_GROUP_SCRIPTS`).
#[test]
fn generated_txstore_credit_group_scripts_match_go() {
    group_campaign(
        "PURSE_UDB_TX_CREDIT_GROUP_SCRIPTS",
        4,
        "purse-udb-txstore-credit-groups",
        generate_txstore_credits,
    );
}

fn group_campaign(
    var: &str,
    default: usize,
    stream: &str,
    generate: fn(&mut SplitMix64, usize, Option<&'static str>) -> Value,
) {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let count: usize = std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    let mut rng = SplitMix64::from_env(stream);
    let nets = ["simnet", "regnet", "testnet3", "mainnet"];
    let (mut ops, mut groups, mut tolerated, mut rolled_back) = (0, 0, 0, 0);
    for id in 0..count {
        let script = generate(&mut rng, id, nets.get(id).copied());
        let script = group_txstore(&mut rng, script);
        let results = differential(
            &mut o,
            &format!("txstore group script {id} ({stream}): {script}"),
            &script,
        );
        for r in &results {
            ops += 1;
            if let Some(members) = r["ops"].as_array() {
                groups += 1;
                ops += members.len();
                tolerated += members.iter().filter(|m| m["tolerated"] == true).count();
                if r.get("err").is_some() {
                    rolled_back += 1;
                }
            }
        }
    }
    eprintln!(
        "{count} generated group scripts ({stream}), {ops} operations equal: {groups} groups, \
         {rolled_back} ended by an error, {tolerated} tolerated errors"
    );
}

/// Every operation a generated transaction store script draws is one the
/// Rust runner implements or one of the generator's own spellings of
/// `store.extend_main_chain` and `tx.build`.
#[test]
fn txstore_generator_draws_only_ported_operations() {
    for op in TX_OPS.iter().chain(CREDIT_OPS) {
        assert!(
            SUPPORTED.contains(op)
                || op.starts_with("extend.")
                || op.starts_with("build.")
                || op.starts_with("credit."),
            "{op}"
        );
    }
}

/// QK-0780 (reproduced): `InsertMemPoolTx` records the null previous
/// outpoint of an unmined coinbase (any single-input transaction with a
/// null previous outpoint, treasury spends included, QK-0087) in the
/// unmined-inputs index, and looks up every input of a vote, its
/// stakebase included, so while one is unmined a vote is refused as a
/// double spend of it, as is a second one; a vote mined into a block
/// removes it from the unmined set as a double spend. The second script
/// runs the same with treasury spends (version 3, a signature and key
/// pushed before `OP_TSPEND` on the null input, an `OP_RETURN` and a
/// `tgen`-tagged payment): a vote and a second treasury spend refused, the
/// mined vote removing the first, and QK-0087's `Rollback` dropping a
/// mined treasury spend instead of returning it to the unmined set.
/// Both sides on all four networks, each result pinned.
#[test]
fn an_unmined_coinbase_conflicts_with_votes() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    for net in ["simnet", "regnet", "testnet3", "mainnet"] {
        let ticket = json!({"op": "tx.build", "label": "tk",
            "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1100}],
            "outputs": [{"value": 1000, "pay": {"acct": 0, "branch": 0, "index": 0}, "tag": "sstx"},
                        {"value": 0, "commitment": {"pay": {"acct": 0, "branch": 0, "index": 1}, "amount": 1100}},
                        {"value": 0, "pay": {"acct": 0, "branch": 0, "index": 2}, "tag": "sstxchange"}]});
        // (A signature script is not part of a transaction's hash, so the
        // two coinbases differ in their output.)
        let coinbase = |label: &str, index: u32| {
            json!({"op": "tx.build", "label": label,
                "inputs": [{"prev": "coinbase", "value": 50_000, "script": "0000"}],
                "outputs": [{"value": 50_000, "pay": {"acct": 0, "branch": 0, "index": index}}]})
        };
        let vote = json!({"op": "tx.build", "label": "v", "version": 1,
            "inputs": [{"prev": "stakebase", "value": 300}, {"prev": {"tx": "tk", "index": 0, "tree": 1}, "value": 1000}],
            "outputs": [{"value": 0, "voteblock": "b1"}, {"value": 0, "votebits": "0100"},
                        {"value": 1300, "pay": {"acct": 0, "branch": 0, "index": 4}, "tag": "ssgen"}]});
        let script = json!({
            "format": "purse-opscript 1",
            "name": format!("unmined coinbase and votes, {net}"),
            "setup": {"network": net, "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
                      "clock": 1_795_300_000},
            "ops": [
                {"op": "store.extend_main_chain", "label": "b1"},
                ticket,
                {"op": "store.insert_mined_tx", "tx": "tk", "block": "b1"},
                coinbase("cb", 3),
                {"op": "store.insert_mem_pool_tx", "tx": "cb"},
                vote,
                {"op": "store.insert_mem_pool_tx", "tx": "v"},
                coinbase("cb2", 5),
                {"op": "store.insert_mem_pool_tx", "tx": "cb2"},
                {"op": "store.unmined_tx_hashes"},
                {"op": "store.insert_mined_tx", "tx": "v", "block": "b1"},
                {"op": "store.unmined_tx_hashes"},
                {"op": "store.insert_mem_pool_tx", "tx": "cb2"},
                {"op": "store.unmined_tx_hashes"},
            ],
        });
        let go = differential(&mut o, &format!("unmined coinbase, {net}"), &script);
        let kind = |i: usize| go[i]["err"]["kind"].as_str().map(str::to_owned);
        let count = |i: usize| go[i]["ok"].as_array().map(Vec::len);
        assert_eq!(kind(4), None, "{net}: the coinbase is recorded: {}", go[4]);
        assert_eq!(kind(6).as_deref(), Some("DoubleSpend"), "{net}: {}", go[6]);
        assert_eq!(kind(8).as_deref(), Some("DoubleSpend"), "{net}: {}", go[8]);
        assert_eq!(
            count(9),
            Some(1),
            "{net}: only the first coinbase: {}",
            go[9]
        );
        assert_eq!(kind(10), None, "{net}: the vote is mined: {}", go[10]);
        assert_eq!(
            count(11),
            Some(0),
            "{net}: the mined vote removed it: {}",
            go[11]
        );
        assert_eq!(kind(12), None, "{net}: {}", go[12]);
        assert_eq!(count(13), Some(1), "{net}: {}", go[13]);

        // The same with a treasury spend paying a wallet address.
        let tspend = |label: &str, index: u32| {
            json!({"op": "tx.build", "label": label, "version": 3, "expiry": 100,
                "inputs": [{"prev": "coinbase", "value": 2000,
                            "script": format!("40{}21{}c2", "01".repeat(64),
                                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")}],
                "outputs": [{"value": 0, "opreturn": "00".repeat(32)},
                            {"value": 2000, "pay": {"acct": 0, "branch": 0, "index": index}, "tag": "tgen"}]})
        };
        let script = json!({
            "format": "purse-opscript 1",
            "name": format!("unmined treasury spend and votes, {net}"),
            "setup": {"network": net, "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c",
                      "clock": 1_795_300_000},
            "ops": [
                {"op": "store.extend_main_chain", "label": "b1"},
                script["ops"][1].clone(),
                {"op": "store.insert_mined_tx", "tx": "tk", "block": "b1"},
                tspend("ts", 3),
                {"op": "store.insert_mem_pool_tx", "tx": "ts"},
                script["ops"][5].clone(),
                {"op": "store.insert_mem_pool_tx", "tx": "v"},
                {"op": "store.unmined_tx_hashes"},
                tspend("ts2", 5),
                {"op": "store.insert_mem_pool_tx", "tx": "ts2"},
                {"op": "store.insert_mined_tx", "tx": "v", "block": "b1"},
                {"op": "store.unmined_tx_hashes"},
                {"op": "store.insert_mined_tx", "tx": "ts", "block": "b1"},
                {"op": "store.rollback", "height": 1},
                {"op": "store.unmined_tx_hashes"},
            ],
        });
        let go = differential(&mut o, &format!("unmined treasury spend, {net}"), &script);
        let kind = |i: usize| go[i]["err"]["kind"].as_str().map(str::to_owned);
        let hashes = |i: usize| go[i]["ok"].as_array().cloned().unwrap_or_default();
        assert_eq!(go[3]["ok"]["type"], "tspend", "{net}: {}", go[3]);
        let ts = go[3]["ok"]["hash"].clone();
        assert_eq!(
            kind(4),
            None,
            "{net}: the treasury spend is recorded: {}",
            go[4]
        );
        assert_eq!(kind(6).as_deref(), Some("DoubleSpend"), "{net}: {}", go[6]);
        assert_eq!(hashes(7), vec![ts.clone()], "{net}: {}", go[7]);
        assert_eq!(go[8]["ok"]["type"], "tspend", "{net}: {}", go[8]);
        assert_eq!(
            kind(9).as_deref(),
            Some("DoubleSpend"),
            "{net}: a second treasury spend: {}",
            go[9]
        );
        assert_eq!(kind(10), None, "{net}: the vote is mined: {}", go[10]);
        assert!(
            hashes(11).is_empty(),
            "{net}: the mined vote removed it: {}",
            go[11]
        );
        assert_eq!(kind(12), None, "{net}: {}", go[12]);
        assert_eq!(kind(13), None, "{net}: {}", go[13]);
        assert_eq!(
            hashes(14).len(),
            2,
            "{net}: the vote and the ticket return: {}",
            go[14]
        );
        assert!(
            !hashes(14).contains(&ts),
            "{net}: the mined treasury spend is dropped: {}",
            go[14]
        );
    }
}

/// QK-0781 (reproduced): `InsertMinedTx` keys a mined record by
/// transaction and block, so one transaction is recorded mined in two
/// blocks when it is inserted into each, and a `Rollback` of the higher
/// block returns it to the unmined set while the lower block keeps its
/// record: mined and unmined at once (spec 03 I7 does not hold). Both
/// sides on all four networks, each result pinned.
#[test]
fn a_transaction_mined_twice_and_rolled_back_is_mined_and_unmined() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    for net in ["simnet", "regnet", "testnet3", "mainnet"] {
        let script = json!({
            "format": "purse-opscript 1",
            "name": format!("mined twice, {net}"),
            "setup": {"network": net, "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c",
                      "clock": 1_795_300_000},
            "ops": [
                {"op": "store.extend_main_chain", "label": "b1"},
                {"op": "store.extend_main_chain", "label": "b2"},
                {"op": "tx.build", "label": "t",
                 "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1000}],
                 "outputs": [{"value": 900, "pay": {"acct": 0, "branch": 0, "index": 0}}]},
                {"op": "store.insert_mined_tx", "tx": "t", "block": "b1"},
                {"op": "store.insert_mined_tx", "tx": "t", "block": "b2"},
                {"op": "store.range_transactions", "begin": 0, "end": -1},
                {"op": "store.rollback", "height": 2},
                {"op": "store.exists_tx_mined_or_unmined", "tx": "t"},
                {"op": "store.tx_block_height", "tx": "t"},
                {"op": "store.unmined_tx_hashes"},
                {"op": "store.range_transactions", "begin": 0, "end": -1},
                {"op": "store.insert_mem_pool_tx", "tx": "t"},
            ],
        });
        let go = differential(&mut o, &format!("mined twice, {net}"), &script);
        let batches = |i: usize| go[i]["ok"].as_array().map(Vec::len);
        assert_eq!(
            batches(5),
            Some(2),
            "{net}: a record in each block: {}",
            go[5]
        );
        assert_eq!(
            go[7]["ok"],
            json!({"mined": false, "unmined": true}),
            "{net}: {}",
            go[7]
        );
        assert_eq!(go[8]["ok"], json!(-1), "{net}: {}", go[8]);
        assert_eq!(
            go[9]["ok"].as_array().map(Vec::len),
            Some(1),
            "{net}: {}",
            go[9]
        );
        assert_eq!(
            batches(10),
            Some(2),
            "{net}: block 1's record and the unmined one: {}",
            go[10]
        );
        assert_eq!(go[11]["err"]["kind"], "Exist", "{net}: {}", go[11]);
    }
}

/// A group member whose operation returns an error where Go panics may
/// not tolerate that error's kind: Go never tolerates a panic, so its
/// group ends at the member and rolls back, where a Rust group tolerating
/// the stand-in error would go on (PARITY.md, `purse-udb` transaction
/// store, item (2)). The Rust runner refuses such a script as a script
/// fault, for each operation and kind it lists; Go's run of the first
/// shows the group it would end. The refusal needs no oracle; Go's side
/// runs when one is available.
#[test]
fn a_group_member_may_not_tolerate_a_panic_stand_in() {
    let script = |member: Value| {
        json!({
            "format": "purse-opscript 1",
            "name": "tolerated panic stand-in",
            "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b",
                      "clock": 1_795_300_000},
            "ops": [
                {"op": "store.extend_main_chain", "label": "b1"},
                {"op": "update", "ops": [
                    {"op": "store.extend_main_chain", "label": "b2"},
                    member,
                    {"op": "store.extend_main_chain", "label": "b3"},
                ]},
                {"op": "store.main_chain_tip"},
            ],
        })
    };
    let rollback = json!({"op": "store.rollback", "height": 9, "tolerate": ["Invalid"]});
    for (member, kind) in [
        (rollback.clone(), "Invalid"),
        (
            json!({"op": "store.rollback", "height": 9, "tolerate": ["NotExist", "IO"]}),
            "IO",
        ),
        (
            json!({"op": "store.get_main_chain_block_hashes", "start": "b1", "inclusive": true, "count": 3, "tolerate": ["Invalid"]}),
            "Invalid",
        ),
        (
            json!({"op": "store.main_chain_tip", "tolerate": ["IO"]}),
            "IO",
        ),
    ] {
        let dir = scratch("tolerate");
        let err = runner::run(&script(member.clone()), &dir.join("rust"))
            .expect_err("the Rust runner refuses the script");
        assert!(
            err.contains("never tolerates a panic") && err.contains(&format!("tolerate: {kind} ")),
            "{member}: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    // A kind no stand-in takes is accepted on the same operation: the
    // group runs, the member's error ending it.
    let dir = scratch("tolerate");
    let ok = json!({"op": "store.rollback", "height": 9, "tolerate": ["NotExist"]});
    let rust = runner::run(&script(ok), &dir.join("rust")).expect("Rust runner");
    assert_eq!(rust.results[1]["ops"].as_array().map(Vec::len), Some(2));
    let _ = std::fs::remove_dir_all(&dir);

    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let dir = scratch("tolerate");
    let (go, _) = go_run(&mut o, &script(rollback), &dir.join("go"));
    let members = go[1]["ops"].as_array().cloned().unwrap_or_default();
    assert_eq!(members.len(), 2, "Go's group ends at the panic: {}", go[1]);
    assert_eq!(members[1]["err"]["kind"], "Panic", "{}", go[1]);
    assert_eq!(members[1].get("tolerated"), None, "{}", go[1]);
    assert_eq!(go[2]["ok"]["height"], 1, "rolled back to b1: {}", go[2]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// QK-0082, fixed under ADR-0012 Rule 7 (PARITY.md, `purse-udb`): when a
/// block whose regular transactions hold a wallet spend chain (A paying
/// the wallet, B spending A's credit, A recorded first) is stake
/// invalidated, Go moves A's spent credit to the invalidated table and then
/// restores an unspent entry for it from B's debit, an entry whose credit
/// is in no live table: its unspent count is one more and `AccountBalances`
/// fails with `IO` until the block is validated again. dcr-purse restores
/// no unspent entry for a credit that is not live. Both sides are shown;
/// every other record is equal, and once the block is validated again the
/// records and the final exports are equal.
#[test]
fn a_same_block_spend_chain_invalidated_leaves_no_dangling_unspent() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let pay = |i: u32| json!({"acct": 0, "branch": 0, "index": i});
    let script = json!({
        "format": "purse-opscript 1",
        "name": "same-block spend chain invalidated",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1000}],
             "outputs": [{"value": 900, "pay": pay(0)}]},
            {"op": "tx.build", "label": "b",
             "inputs": [{"prev": {"tx": "a", "index": 0}, "value": 900}],
             "outputs": [{"value": 800, "pay": pay(1)}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b2"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b2"},
            {"op": "store.insert_mined_tx", "tx": "b", "block": "b2"},
            {"op": "store.add_credit", "tx": "b", "index": 0, "account": 0, "block": "b2"},
            {"op": "store.extend_main_chain", "label": "b3", "votebits": 0},
            {"op": "store.unspent_output_count"},
            {"op": "store.account_balances", "minconf": 1},
            {"op": "store.tx_details", "tx": "a"},
            {"op": "store.tx_details", "tx": "b"},
            {"op": "store.rollback", "height": 3},
            {"op": "store.extend_main_chain", "label": "b3x", "votebits": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.account_balances", "minconf": 1},
            {"op": "store.tx_details", "tx": "a"},
            {"op": "store.tx_details", "tx": "b"},
        ],
    });
    let dir = scratch("qk0082");
    let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    assert_eq!(go.len(), rust.results.len());
    // The invalidated state: Go's dangling entry.
    assert_eq!(go[9]["ok"], json!(1), "Go: {}", go[9]);
    assert_eq!(rust.results[9]["ok"], json!(0), "Rust: {}", rust.results[9]);
    assert_eq!(go[10]["err"]["kind"], "IO", "Go: {}", go[10]);
    assert!(
        go[10]["err"]["text"]
            .as_str()
            .is_some_and(|t| t.ends_with("missing credit for unspent output")),
        "Go: {}",
        go[10]
    );
    assert_eq!(
        rust.results[10]["ok"],
        json!([]),
        "Rust: {}",
        rust.results[10]
    );
    for (i, (g, r)) in go.iter().zip(&rust.results).enumerate() {
        if i != 9 && i != 10 {
            support::compare::check_record("QK-0082", g, r);
        }
    }
    // Validated again: the same rows on both sides.
    assert_eq!(go[15]["ok"], json!(1), "Go: {}", go[15]);
    support::compare::compare_exports("QK-0082", &go_pe, &rust.export);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `AddCredit` where Go writes a credit the store's key cannot hold
/// (PARITY.md, `purse-udb`): a block meta naming a block that is not the
/// main-chain block at its height (Go reads no header for a transaction
/// that is not regular, and keys the credit by the hash given), and a block
/// in which the transaction is not recorded. Go records both and answers
/// nil; dcr-purse refuses with `Invalid` and writes nothing. Records before
/// the divergence are equal.
#[test]
fn credits_off_the_main_chain_or_without_a_record_are_refused() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let pay = |i: u32| json!({"acct": 0, "branch": 0, "index": i});
    let script = json!({
        "format": "purse-opscript 1",
        "name": "credits Go keys off the main chain",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "tx.build", "label": "k",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1100}],
             "outputs": [{"value": 1000, "pay": pay(0), "tag": "sstx"},
                         {"value": 0, "commitment": {"pay": pay(1), "amount": 1100}},
                         {"value": 0, "pay": pay(2), "tag": "sstxchange"}]},
            {"op": "store.insert_mined_tx", "tx": "k", "block": "b2"},
            // A ticket's credit keyed by a block hash that is no block.
            {"op": "store.add_credit", "tx": "k", "index": 0, "account": 0,
             "block": {"hash": "2222222222222222222222222222222222222222222222222222222222222222",
                       "height": 2, "time": 0, "votebits": 1}},
            // A ticket's credit in a main-chain block that has no record of it.
            {"op": "store.add_credit", "tx": "k", "index": 2, "account": 0, "block": "b1"},
        ],
    });
    let dir = scratch("offchain");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    for (g, r) in go.iter().zip(&rust.results).take(4) {
        support::compare::check_record("off-chain credits", g, r);
    }
    for i in [4, 5] {
        assert_eq!(go[i]["ok"], Value::Null, "Go writes it: {}", go[i]);
        assert!(go[i].get("err").is_none(), "Go: {}", go[i]);
        assert_eq!(
            rust.results[i]["err"]["kind"], "Invalid",
            "Rust: {}",
            rust.results[i]
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// QK-0782, a consequence of QK-0081's decided divergence (ADR-0003
/// Decision item 5, PARITY.md, `purse-udb`): a credit invalidated with its
/// block is left by Go's `Rollback` under the block's hash; once the same
/// block is connected again, the transaction mined in it again, and the
/// block disapproved and then approved, Go's stake validation brings the
/// old credit back with an unspent entry. dcr-purse deleted the row at the
/// rollback, so the transaction comes back without a credit. Every record
/// before the revival is equal; after it, both sides are shown.
#[test]
fn invalidated_credits_of_a_detached_block_do_not_come_back() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "invalidated credits of a detached block",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1000}],
             "outputs": [{"value": 900, "pay": {"acct": 0, "branch": 0, "index": 0}}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b2"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b2"},
            // b2 disapproved: a's credit moves to the invalidated table.
            {"op": "store.extend_main_chain", "label": "b3", "votebits": 0},
            // b2 and b3 detached: Go leaves the invalidated credit.
            {"op": "store.rollback", "height": 2},
            {"op": "store.unspent_output_count"},
            // The same b2 again, a mined in it again, b2 disapproved again.
            {"op": "store.extend_main_chain", "again": "b2"},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b2"},
            {"op": "store.extend_main_chain", "label": "b3x", "votebits": 0},
            {"op": "store.tx_details", "tx": "a"},
            // b2 approved: Go's stake validation revives the old credit.
            {"op": "store.rollback", "height": 3},
            {"op": "store.extend_main_chain", "label": "b3y", "votebits": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.account_balances", "minconf": 1},
            {"op": "store.tx_details", "tx": "a"},
        ],
    });
    let dir = scratch("qk0782");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    assert_eq!(go.len(), rust.results.len());
    for (g, r) in go.iter().zip(&rust.results).take(14) {
        support::compare::check_record("QK-0782", g, r);
    }
    assert_eq!(go[7]["ok"], json!(0), "Go: {}", go[7]);
    assert_eq!(go[13]["ok"]["height"], json!(3), "Go: {}", go[13]);
    // The revived credit: an unspent output, a balance and a credit in
    // Go's details; none here.
    assert_eq!(go[14]["ok"], json!(1), "Go: {}", go[14]);
    assert_eq!(
        rust.results[14]["ok"],
        json!(0),
        "Rust: {}",
        rust.results[14]
    );
    assert_eq!(go[15]["ok"][0]["total"], json!(900), "Go: {}", go[15]);
    assert_eq!(
        rust.results[15]["ok"],
        json!([]),
        "Rust: {}",
        rust.results[15]
    );
    let credits = |v: &Value| v["ok"]["credits"].as_array().map_or(0, Vec::len);
    assert_eq!(credits(&go[16]), 1, "Go: {}", go[16]);
    assert_eq!(credits(&rust.results[16]), 0, "Rust: {}", rust.results[16]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// QK-0101 (PARITY.md, `purse-udb`, item (14); pending owner, crate
/// `txstore`, choice 8): a spender recorded in a block below
/// its funding transaction's, the one way to a debit whose credit is not
/// recorded. Rolling back the funding block leaves the debit naming a
/// deleted credit; rolling back the spender's block then fails on both
/// sides with `IO` "missing credit", whose 72-byte key holds Go's block
/// hash of the credit and here the main-chain block's at its height, zero
/// with none (the store's debit keeps the credit's height, not its block's
/// hash). Disapproving the spender's block makes Go restore an unspent
/// entry for the missing credit, which dcr-purse does not (QK-0082's
/// rule); approving it again makes Go write a zero-amount spent credit,
/// where dcr-purse refuses the extension with `IO` and writes nothing.
/// Every other record up to that refusal is equal.
#[test]
fn a_spender_below_its_funder_is_refused_where_go_fabricates_a_credit() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let pay = |i: u32| json!({"acct": 0, "branch": 0, "index": i});
    let script = json!({
        "format": "purse-opscript 1",
        "name": "a spender below its funder",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1000}],
             "outputs": [{"value": 900, "pay": pay(0)}]},
            {"op": "tx.build", "label": "b",
             "inputs": [{"prev": {"tx": "a", "index": 0}, "value": 900}],
             "outputs": [{"value": 800, "pay": pay(1)}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b2"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b2"},
            // b spends a's credit from the block below a's.
            {"op": "store.insert_mined_tx", "tx": "b", "block": "b1"},
            {"op": "store.add_credit", "tx": "b", "index": 0, "account": 0, "block": "b1"},
            // a's block detached: b's debit names a deleted credit.
            {"op": "store.rollback", "height": 2},
            {"op": "store.unspent_output_count"},
            {"op": "store.tx_details", "tx": "b"},
            // 11: the missing credit.
            {"op": "store.rollback", "height": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.tx_details", "tx": "b"},
            // 14: b1 disapproved; 16, 19, 20: Go's unspent entry without a
            // credit.
            {"op": "store.extend_main_chain", "label": "b2x", "votebits": 0},
            {"op": "store.extend_main_chain", "label": "b3x", "votebits": 0},
            {"op": "store.unspent_output_count"},
            // 21: b1 approved again: Go fabricates, dcr-purse refuses.
            {"op": "store.rollback", "height": 2},
            {"op": "store.tx_details", "tx": "b"},
            {"op": "store.account_balances", "minconf": 0},
            {"op": "store.unspent_output_count"},
            {"op": "store.extend_main_chain", "label": "b2y", "votebits": 1},
            {"op": "store.unspent_output_count"},
            // A disapproving block (at height 2 here, 3 in Go), so that
            // dcr-purse's export, which names a credit's block by the main
            // chain at its height, can write b's dangling debit.
            {"op": "store.extend_main_chain", "label": "b2z", "votebits": 0},
        ],
    });
    let dir = scratch("qk0101");
    let (go, _) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    assert_eq!(go.len(), rust.results.len());
    for (i, (g, r)) in go.iter().zip(&rust.results).enumerate().take(21) {
        if ![11, 16, 19, 20].contains(&i) {
            support::compare::check_record("QK-0101", g, r);
        }
    }
    // The missing credit: Go's text, but for the block hash in the key.
    let mut b2 = purse_testutil::unhex(go[1]["ok"]["hash"].as_str().expect("hash"));
    b2.reverse();
    let b2 = hex(&b2);
    let go_text = go[11]["err"]["text"].as_str().expect("Go: IO");
    assert_eq!(go[11]["err"]["kind"], "IO", "Go: {}", go[11]);
    assert!(
        go_text.contains("missing credit") && go_text.contains(&b2),
        "Go: {}",
        go[11]
    );
    assert_eq!(
        rust.results[11]["err"]["kind"], "IO",
        "Rust: {}",
        rust.results[11]
    );
    assert_eq!(
        rust.results[11]["err"]["text"].as_str(),
        Some(go_text.replace(&b2, &"0".repeat(64)).as_str()),
        "Rust: {}",
        rust.results[11]
    );
    // Disapproved: Go's unspent entry for the missing credit, which its
    // balance query then fails on; here a's unmined credit alone.
    assert_eq!(
        (&go[16]["ok"], &rust.results[16]["ok"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(go[19]["err"]["kind"], "IO", "Go: {}", go[19]);
    assert!(
        go[19]["err"]["text"]
            .as_str()
            .is_some_and(|t| t.ends_with("missing credit for unspent output")),
        "Go: {}",
        go[19]
    );
    assert_eq!(
        rust.results[19]["ok"][0]["total"],
        json!(900),
        "Rust: {}",
        rust.results[19]
    );
    assert_eq!(
        (&go[20]["ok"], &rust.results[20]["ok"]),
        (&json!(1), &json!(0))
    );
    // Approved: Go's zero-amount credit, dcr-purse's refusal.
    assert_eq!(go[21]["ok"]["height"], json!(2), "Go: {}", go[21]);
    assert_eq!(
        rust.results[21]["err"]["kind"], "IO",
        "Rust: {}",
        rust.results[21]
    );
    assert!(
        rust.results[21]["err"]["text"]
            .as_str()
            .is_some_and(|t| t.contains("stake validation") && t.ends_with("which is not recorded")),
        "Rust: {}",
        rust.results[21]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// QK-0734 and QK-0736 (reproduced; QK-0734's disposition pending owner,
/// crate `txstore`, choice 10): a transaction mined in block 3, whose block
/// 4 disapproves block 3, keeps its mined record, so `InsertMemPoolTx`
/// refuses it as already mined (QK-0736); mined again in block 5 and its
/// credit added there, then blocks 4 and 5 rolled back and block 3
/// approved by another block 4, it is unmined (from block 5) and mined
/// (block 3's revalidated record) at once. Its one output is then an
/// unspent output and an unmined credit: `UnspentOutputs` lists it twice,
/// both times as unmined, and `AccountBalances` counts it twice (QK-0734).
/// Every record and the final exports are equal; Go's answers are pinned.
#[test]
fn a_revalidated_transaction_also_unmined_is_counted_twice() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0734: revalidated and unmined",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "store.extend_main_chain", "label": "b3"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 1000}],
             "outputs": [{"value": 900, "pay": {"acct": 0, "branch": 0, "index": 0}}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b3"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b3"},
            // 6: b3 disapproved; 7: a keeps its mined record (QK-0736).
            {"op": "store.extend_main_chain", "label": "b4", "votebits": 0},
            {"op": "store.insert_mem_pool_tx", "tx": "a"},
            // a mined again in b5, its credit added there.
            {"op": "store.extend_main_chain", "label": "b5"},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b5"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b5"},
            {"op": "store.account_balances", "minconf": 0},
            // 12: b4 and b5 detached (a unmined again); 13: b3 approved.
            {"op": "store.rollback", "height": 4},
            {"op": "store.extend_main_chain", "label": "b4x", "votebits": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.unspent_outputs"},
            {"op": "store.account_balances", "minconf": 0},
            {"op": "store.account_balances", "minconf": 1},
            {"op": "store.exists_tx_mined_or_unmined", "tx": "a"},
            {"op": "store.insert_mem_pool_tx", "tx": "a"},
        ],
    });
    let go = differential(&mut o, "QK-0734", &script);
    let exist = |r: &Value| {
        r["err"]["kind"] == "Exist"
            && r["err"]["text"]
                .as_str()
                .is_some_and(|t| t.ends_with("transaction already exists mined"))
    };
    assert!(exist(&go[7]), "QK-0736: {}", go[7]);
    assert_eq!(go[14]["ok"], json!(1), "one unspent row: {}", go[14]);
    let outs = go[15]["ok"].as_array().expect("UnspentOutputs");
    assert_eq!(outs.len(), 2, "the outpoint twice: {}", go[15]);
    for out in outs {
        assert_eq!(out["outpoint"], outs[0]["outpoint"], "{}", go[15]);
        assert_eq!(out["block"]["height"], json!(-1), "as unmined: {}", go[15]);
    }
    let bal = |i: usize, field: &str| go[i]["ok"][0][field].clone();
    assert_eq!(
        (bal(16, "spendable"), bal(16, "total")),
        (json!(1800), json!(1800)),
        "minconf 0: {}",
        go[16]
    );
    assert_eq!(
        (
            bal(17, "spendable"),
            bal(17, "unconfirmed"),
            bal(17, "total")
        ),
        (json!(900), json!(900), json!(1800)),
        "minconf 1: {}",
        go[17]
    );
    assert_eq!(go[18]["ok"], json!({"mined": false, "unmined": true}));
    assert!(exist(&go[19]), "{}", go[19]);
}

/// QK-0785 (reproduced, pending owner; crate `txstore`, choice 11): a
/// ticket mined in block 3 spends a regular credit of block 3, a state no
/// consensus-valid chain holds. Block 4 disapproves block 3: the credit,
/// its spender kept, moves to the invalidated table and the ticket's debit
/// stays live (stake invalidation moves only regular transactions' rows).
/// `Rollback` over block 3 then fails on both sides with `IO` "missing
/// credit" (Go's `Rollback` reads only the live credits, QK-0081) until
/// block 3 is approved again, the texts equal, since the credit's block is
/// still the main-chain block at its height. Once another block 4 approves
/// block 3, stake validation moves the credit back with an unspent entry
/// although it is still spent: `UnspentOutputs` lists it, `AccountBalances`
/// counts it as spendable and `TxDetails` reports it spent. `Rollback` over
/// block 3 then succeeds. The second script makes the same unspent entry
/// with a regular spender recorded in block 2, below the credit's block
/// (QK-0101's state), whose debit invalidation of block 3 leaves live as
/// well. Every record and the final exports are equal; Go's answers are
/// pinned.
#[test]
fn a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let pay = |i: u32| json!({"acct": 0, "branch": 0, "index": i});
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0785: a revalidated credit a ticket of its block spends",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "store.extend_main_chain", "label": "b3"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "6666666666666666666666666666666666666666666666666666666666666666", "index": 0}, "value": 5000}],
             "outputs": [{"value": 1100, "pay": pay(0)}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b3"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b3"},
            {"op": "tx.build", "label": "t",
             "inputs": [{"prev": {"tx": "a", "index": 0}, "value": 1100}],
             "outputs": [{"value": 1000, "pay": pay(1), "tag": "sstx"},
                         {"value": 0, "commitment": {"pay": pay(2), "amount": 1100}},
                         {"value": 50, "pay": pay(3), "tag": "sstxchange"}]},
            {"op": "store.insert_mined_tx", "tx": "t", "block": "b3"},
            {"op": "store.add_credit", "tx": "t", "index": 0, "account": 0, "block": "b3"},
            // 9: b3 disapproved; 10: the ticket's output alone.
            {"op": "store.extend_main_chain", "label": "b4", "votebits": 0},
            {"op": "store.unspent_output_count"},
            // 11: the missing credit; 12: b4 alone detaches.
            {"op": "store.rollback", "height": 3},
            {"op": "store.rollback", "height": 4},
            // 13: b3 approved; 14 to 17: a's spent credit unspent again.
            {"op": "store.extend_main_chain", "label": "b4x", "votebits": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.unspent_outputs"},
            {"op": "store.account_balances", "minconf": 0},
            {"op": "store.tx_details", "tx": "a"},
            // 18: b3 detaches now.
            {"op": "store.rollback", "height": 3},
            {"op": "store.unspent_output_count"},
            {"op": "store.account_balances", "minconf": 0},
        ],
    });
    let go = differential(&mut o, "QK-0785", &script);
    let a = go[3]["ok"]["hash"].as_str().expect("a's hash").to_owned();
    let t = go[6]["ok"]["hash"].as_str().expect("t's hash").to_owned();
    assert_eq!(go[10]["ok"], json!(1), "{}", go[10]);
    let mut b3 = purse_testutil::unhex(go[2]["ok"]["hash"].as_str().expect("hash"));
    b3.reverse();
    let text = go[11]["err"]["text"].as_str().unwrap_or_default();
    assert_eq!(go[11]["err"]["kind"], "IO", "{}", go[11]);
    assert!(
        text.contains("missing credit")
            && text.contains(&hex(&b3))
            && text.ends_with(&format!("spent by {t}")),
        "{}",
        go[11]
    );
    assert_eq!(go[12]["ok"], json!([]), "{}", go[12]);
    assert_eq!(go[14]["ok"], json!(2), "{}", go[14]);
    let outs = go[15]["ok"].as_array().expect("UnspentOutputs");
    assert!(
        outs.iter()
            .any(|o| o["outpoint"]["hash"] == json!(a) && o["amount"] == json!(1100)),
        "a's spent output listed: {}",
        go[15]
    );
    assert_eq!(
        (
            &go[16]["ok"][0]["spendable"],
            &go[16]["ok"][0]["votingauthority"]
        ),
        (&json!(1100), &json!(1000)),
        "{}",
        go[16]
    );
    assert_eq!(
        go[17]["ok"]["credits"][0]["spent"],
        json!(true),
        "{}",
        go[17]
    );
    assert_eq!(
        go[18]["ok"][0]["txs"].as_array().map(Vec::len),
        Some(2),
        "b3's two records detached: {}",
        go[18]
    );

    // A regular spender recorded in b2, below its funder's block b3.
    let script = json!({
        "format": "purse-opscript 1",
        "name": "QK-0785: a revalidated credit a lower block's transaction spends",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "store.extend_main_chain", "label": "b3"},
            {"op": "tx.build", "label": "a",
             "inputs": [{"prev": {"tx": "6666666666666666666666666666666666666666666666666666666666666666", "index": 0}, "value": 5000}],
             "outputs": [{"value": 1100, "pay": pay(0)}]},
            {"op": "tx.build", "label": "s",
             "inputs": [{"prev": {"tx": "a", "index": 0}, "value": 1100}],
             "outputs": [{"value": 1000, "pay": pay(1)}]},
            {"op": "store.insert_mined_tx", "tx": "a", "block": "b3"},
            {"op": "store.add_credit", "tx": "a", "index": 0, "account": 0, "block": "b3"},
            {"op": "store.insert_mined_tx", "tx": "s", "block": "b2"},
            {"op": "store.add_credit", "tx": "s", "index": 0, "account": 0, "block": "b2"},
            // 9: b3 disapproved; 10: b4 alone detaches; 11: b3 approved.
            {"op": "store.extend_main_chain", "label": "b4", "votebits": 0},
            {"op": "store.rollback", "height": 4},
            {"op": "store.extend_main_chain", "label": "b4x", "votebits": 1},
            {"op": "store.unspent_output_count"},
            {"op": "store.unspent_outputs"},
            {"op": "store.account_balances", "minconf": 0},
            {"op": "store.tx_details", "tx": "a"},
        ],
    });
    let go = differential(&mut o, "QK-0785, a spender below its funder", &script);
    let a = go[3]["ok"]["hash"].as_str().expect("a's hash").to_owned();
    assert_eq!(go[12]["ok"], json!(2), "{}", go[12]);
    let outs = go[13]["ok"].as_array().expect("UnspentOutputs");
    assert!(
        outs.iter()
            .any(|o| o["outpoint"]["hash"] == json!(a) && o["amount"] == json!(1100)),
        "a's spent output listed: {}",
        go[13]
    );
    assert_eq!(go[14]["ok"][0]["spendable"], json!(2100), "{}", go[14]);
    assert_eq!(
        go[15]["ok"]["credits"][0]["spent"],
        json!(true),
        "{}",
        go[15]
    );
}
