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
use support::generator::{GEN_OPS, Shape, generate};
use support::runner::{self, SUPPORTED};
use support::scratch;

#[test]
fn hand_written_scripts_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    for (name, script) in corpus() {
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
/// implements, and every implemented `mgr.` operation but the one that
/// needs a watching-only wallet is drawn.
#[test]
fn generated_scripts_draw_every_ported_operation() {
    for op in GEN_OPS {
        assert!(SUPPORTED.contains(op), "{op} is drawn but not ported");
    }
    for op in SUPPORTED.iter().filter(|op| op.starts_with("mgr.")) {
        if ["mgr.import_public_key", "mgr.chain_params"].contains(op) {
            continue;
        }
        assert!(GEN_OPS.contains(op), "{op} is ported but never drawn");
    }
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
    for (field, value, want) in [
        (
            "privpass",
            "",
            "invalid operation: private passphrase may not be empty",
        ),
        (
            "seed",
            "000102030405060708090a0b0c0d0e",
            "seed length must be between 128 and 512 bits",
        ),
    ] {
        let mut s = base.clone();
        s["setup"][field] = json!(value);
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
