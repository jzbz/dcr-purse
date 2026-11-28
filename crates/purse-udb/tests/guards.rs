// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Guards of the operation-script differential (`tests/oracle.rs`) added
//! at Phase 3's exit (2026-11-26):
//!
//! - a count of the `unspent` table after an earlier member of the same
//!   `update` group wrote it (QK-0003: Go's `KeyN` counts committed pages
//!   only): `a_count_after_an_unspent_write_in_a_group_is_gos_committed_count`
//!   pins Go's answer beside dcr-purse's, and
//!   `generated_group_scripts_never_count_after_an_unspent_write` holds the
//!   generator to the rule that keeps the sequence out of the generated
//!   group scripts (PARITY.md, `purse-udb` transaction store, item (26);
//!   crate `txstore`, choice 15);
//! - the export sections no comparison compares (`[bad]`, `[unknown]`,
//!   `[residue]`; ADR-0001 section 11, addendum of 2026-11-26): every
//!   differential checks them with `compare::check_uncompared`, and
//!   `uncompared_sections_refuse_what_the_rule_does_not_name` shows each
//!   record the rule refuses being refused;
//! - the upstream transaction store tests replayed through the oracle
//!   (`38-upstream-tx-tests`): `the_coinbase_port_asserts_gos_balances`
//!   checks that the balances the `TestCoinbases` port asserts are the
//!   ones Go returned for the same steps (QK-0650);
//! - `rt.checkpoint` in the Rust runner:
//!   `checkpoint_names_are_refused_as_go_refuses_them` holds it to Go's
//!   naming rules and place, and to the one name Go's file layout refuses
//!   (`wallet`; the exports themselves are compared by every differential
//!   and frozen replay, `compare::compare_checkpoints`);
//! - `QUIRKS.md` cites records of some corpus scripts by index (the `i` of
//!   a result record, which is the operation's index):
//!   `quirk_citations_by_record_index_are_pinned` fails when such a
//!   script gains, loses or reorders an operation, so the citations are
//!   checked again (the checkpoints added to `35-txstore-commitments` and
//!   `37-txstore-selection` moved the records QK-0060, QK-0096, QK-0105
//!   and QK-0788 cite).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: small counts and indexes"
)]

mod support;

use purse_export_check::{Export, Record, parse};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, oracle_or_skip};
use support::compare::{check_record, check_uncompared, compare_exports, corpus, go_run};
use support::txgen::{
    counts_after_unspent_writes, generate_txstore, generate_txstore_cms, generate_txstore_credits,
    generate_txstore_select, group_txstore,
};
use support::{data_dir, runner, scratch};

/// The selection key of the pinned selections.
const ZERO_RAND: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The count-in-group script: a transaction of four outputs to the wallet
/// mined in block 1 with its first output credited, then `update` groups
/// in which a member counts the `unspent` table after a member wrote it
/// (`AddCredit` with a block adding an entry, `InsertMinedTx` of a spender
/// deleting one), each followed by the same count at the top level, and
/// one group counting before its write.
fn count_script() -> Value {
    let pay = |index: u32| json!({"acct": 0, "branch": 0, "index": index});
    let select = json!({"op": "store.select_inputs", "account": 0, "minconf": 1,
                        "syncheight": 2, "targets": [1_000_000], "rand": ZERO_RAND});
    let count = json!({"op": "store.unspent_output_count"});
    json!({
        "format": "purse-opscript 1",
        "name": "count of unspent in an update group after a write",
        "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                  "pubpass": "public", "privpass": "private",
                  "rand": "3737373737373737373737373737373737373737373737373737373737373737",
                  "clock": 1_795_300_000},
        "ops": [
            {"op": "store.extend_main_chain", "label": "b1"},
            {"op": "store.extend_main_chain", "label": "b2"},
            {"op": "tx.build", "label": "t",
             "inputs": [{"prev": {"tx": "1111111111111111111111111111111111111111111111111111111111111111", "index": 0}, "value": 30000}],
             "outputs": [{"value": 5000, "pay": pay(0)}, {"value": 6000, "pay": pay(1)},
                         {"value": 7000, "pay": pay(2)}, {"value": 8000, "pay": pay(3)}]},
            // 3: the credit slice's mined group; 4: one entry.
            {"op": "update", "ops": [
                {"op": "store.insert_mined_tx", "tx": "t", "block": "b1"},
                {"op": "store.add_credit", "tx": "t", "index": 0, "account": 0, "block": "b1"},
            ]},
            count.clone(),
            // 5: a selection after a credit in its group, then at the top
            // level (two entries committed).
            {"op": "update", "ops": [
                {"op": "store.add_credit", "tx": "t", "index": 1, "account": 0, "block": "b1"},
                select.clone(),
            ]},
            select,
            // 7: a count after a credit in its group (three entries).
            {"op": "update", "ops": [
                {"op": "store.add_credit", "tx": "t", "index": 2, "account": 0, "block": "b1"},
                count.clone(),
            ]},
            count.clone(),
            // 9, 10: a count after a spend of output 0 mined in its group
            // (two entries).
            {"op": "tx.build", "label": "s",
             "inputs": [{"prev": {"tx": "t", "index": 0}, "value": 5000}],
             "outputs": [{"value": 4000, "pay": pay(5)}]},
            {"op": "update", "ops": [
                {"op": "store.insert_mined_tx", "tx": "s", "block": "b2"},
                count.clone(),
            ]},
            count.clone(),
            // 12: a count before the group's write: equal (three entries
            // after it).
            {"op": "update", "ops": [
                count.clone(),
                {"op": "store.add_credit", "tx": "t", "index": 3, "account": 0, "block": "b1"},
            ]},
            count,
        ],
    })
}

/// QK-0003 in the differential (PARITY.md, `purse-udb` transaction store,
/// item (26); crate `txstore`, choice 15, pending owner): in one `update`
/// group, a member counting the `unspent` table after a member that wrote
/// it. Go's `UnspentOutputCount` and `MakeInputSource` read bbolt's
/// `KeyN`, which counts the committed pages, so Go answers with the count
/// as the group began; the Rust runner, which runs the group in one write
/// transaction of dcr-purse's store, which counts a transaction's own
/// writes, answers with the group's writes counted (the same answers as
/// when it committed each member on its own, before 2026-11-27). Pinned
/// on both sides: after a credit, Go counts 2 and dcr-purse 3; after a
/// spend, Go 3 and dcr-purse 2; and a
/// selection (minimum depth 1, a target above the total) after a credit
/// makes no random try in Go (one committed entry, `randTries <
/// numUnspent/2` false) and one in dcr-purse (two entries), so the bytes
/// drawn differ. Each answer equals the one its side gives at the top
/// level: Go's the count before the group, dcr-purse's the count after
/// it. A count before the group's write, every other record and the final
/// exports are equal. No caller at the pin counts after such a write in
/// one transaction (QK-0003: `txToMultisig` counts first), and the
/// generated group scripts never do (`group_txstore`).
#[test]
fn a_count_after_an_unspent_write_in_a_group_is_gos_committed_count() {
    let script = count_script();
    // The places the generator's rule keeps out of generated scripts.
    let divergent = [(5, 1), (7, 1), (10, 1)];
    assert_eq!(counts_after_unspent_writes(&script), divergent);
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let dir = scratch("group-counts");
    let (go, go_pe) = go_run(&mut o, &script, &dir.join("go"));
    let rust = runner::run(&script, &dir.join("rust")).expect("Rust runner");
    let rust_records = &rust.results;
    assert_eq!(go.len(), rust_records.len(), "record count");
    let member = |v: &[Value], i: usize, j: usize| v[i]["ops"][j].clone();
    for (i, (g, r)) in go.iter().zip(rust_records).enumerate() {
        match divergent.iter().find(|d| d.0 == i) {
            Some(&(_, j)) => {
                // The group's other members and its own record are equal.
                let strip = |v: &Value| {
                    let mut v = v.clone();
                    v["ops"].as_array_mut().expect("members").remove(j);
                    v
                };
                check_record("count script", &strip(g), &strip(r));
            }
            None => check_record("count script", g, r),
        }
    }
    // A count: Go's as the group began, dcr-purse's with the group's write.
    assert_eq!(go[4]["ok"], json!(1), "{}", go[4]);
    assert_eq!(member(&go, 7, 1)["ok"], json!(2), "Go: {}", go[7]);
    assert_eq!(
        member(rust_records, 7, 1)["ok"],
        json!(3),
        "Rust: {}",
        rust_records[7]
    );
    assert_eq!(go[8]["ok"], json!(3), "{}", go[8]);
    assert_eq!(member(&go, 10, 1)["ok"], json!(3), "Go: {}", go[10]);
    assert_eq!(
        member(rust_records, 10, 1)["ok"],
        json!(2),
        "Rust: {}",
        rust_records[10]
    );
    assert_eq!(go[11]["ok"], json!(2), "{}", go[11]);
    assert_eq!(member(&go, 12, 0)["ok"], json!(2), "{}", go[12]);
    assert_eq!(go[13]["ok"], json!(3), "{}", go[13]);
    // A selection: Go makes no random try (one committed entry), dcr-purse
    // one (two), as it does at the top level (op 6, equal on both sides).
    let (g5, r5) = (member(&go, 5, 1), member(rust_records, 5, 1));
    eprintln!("Go's selection in the group: {g5}");
    eprintln!("dcr-purse's selection in the group: {r5}");
    eprintln!("both at the top level: {}", go[6]);
    let strip_i = |v: &Value| {
        let mut v = v.clone();
        v.as_object_mut().expect("record").remove("i");
        v
    };
    assert_eq!(
        strip_i(&r5),
        strip_i(&go[6]),
        "dcr-purse answers as at the top level"
    );
    assert_ne!(
        strip_i(&g5),
        strip_i(&go[6]),
        "Go answers from one committed entry"
    );
    assert_eq!(g5["drawn"], json!(SELECT_DRAWN_GO), "Go: {g5}");
    assert_eq!(r5["drawn"], json!(SELECT_DRAWN_RUST), "Rust: {r5}");
    compare_exports("count script", &go_pe, &rust.export);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bytes Go's selection in group 5 draws: no random try, then the
/// shuffle of the remaining outputs. (Both sides select the same two
/// outputs in the same order here; only the bytes drawn differ.)
const SELECT_DRAWN_GO: u64 = 8;
/// The bytes dcr-purse's draws: one random try first.
const SELECT_DRAWN_RUST: u64 = 41;

/// A transaction store script generator.
type Generate = fn(&mut SplitMix64, usize, Option<&'static str>) -> Value;

/// No generated group script counts the `unspent` table after a write in
/// the same `update` group once an output is credited
/// (`counts_after_unspent_writes`), for every family, from
/// `PURSE_TEST_SEED` (`PURSE_UDB_GROUP_COUNT_SCRIPTS` scripts of each,
/// default 300); the rule's cuts are counted (an `update` group opening
/// with a count right after an `update` group holding a write), so a run
/// that never meets the sequence shows. The hand-written scripts have no
/// such group either.
#[test]
fn generated_group_scripts_never_count_after_an_unspent_write() {
    let count: usize = std::env::var("PURSE_UDB_GROUP_COUNT_SCRIPTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let families: [(&str, Generate); 4] = [
        ("txstore", generate_txstore),
        ("credit", generate_txstore_credits),
        ("cms", generate_txstore_cms),
        ("select", generate_txstore_select),
    ];
    let mut cuts = 0;
    for (family, generate) in families {
        let mut rng = SplitMix64::from_env(&format!("purse-udb-group-counts-{family}"));
        for id in 0..count {
            let script = generate(&mut rng, id, None);
            let script = group_txstore(&mut rng, script);
            assert_eq!(
                counts_after_unspent_writes(&script),
                Vec::<(usize, usize)>::new(),
                "{family} group script {id}: {script}"
            );
            let ops = script["ops"].as_array().expect("ops");
            for w in ops.windows(2) {
                let opens_with_count = w[1]["op"] == "update"
                    && support::txgen::UNSPENT_COUNTERS
                        .contains(&w[1]["ops"][0]["op"].as_str().unwrap_or(""));
                let wrote = w[0]["op"] == "update"
                    && w[0]["ops"].as_array().is_some_and(|m| {
                        m.iter().any(|x| {
                            support::txgen::UNSPENT_WRITERS
                                .contains(&x["op"].as_str().unwrap_or(""))
                        })
                    });
                if opens_with_count && wrote {
                    cuts += 1;
                }
            }
        }
    }
    eprintln!("{count} group scripts of each family; {cuts} groups cut before a count");
    assert!(cuts > 0, "the rule never met its sequence");
    for (name, script) in corpus() {
        assert_eq!(
            counts_after_unspent_writes(&script),
            Vec::<(usize, usize)>::new(),
            "{name}"
        );
    }
}

/// The Go export of `name` (a frozen hand-written vector) and a stand-in
/// for the Rust export of the same state: Go's, without the records the
/// Rust producer never writes.
fn exports(name: &str) -> (Export, Export) {
    let text = std::fs::read_to_string(data_dir().join("opvectors").join(format!("{name}.pe")))
        .expect("frozen export");
    let go = parse(&text).expect("Go export parses");
    let mut rust = go.clone();
    for s in &mut rust.sections {
        if ["bad", "unknown", "residue"].contains(&s.name.as_str()) {
            s.records.clear();
        }
    }
    (go, rust)
}

/// The section named `name` of `ex`, to edit.
fn section_mut<'a>(ex: &'a mut Export, name: &str) -> &'a mut purse_export_check::Section {
    ex.sections
        .iter_mut()
        .find(|s| s.name == name)
        .expect("section")
}

/// A `[bad]` record.
fn bad(path: &str, key: &str, reason: &str) -> Record {
    Record {
        ty: 0,
        values: vec![
            format!("\"{path}\""),
            key.to_owned(),
            reason.to_owned(),
            "-".to_owned(),
        ],
    }
}

/// The rule of `check_uncompared` (ADR-0001 section 11, addendum of
/// 2026-11-26) on `06-addresses`, whose Go export holds three `unreached`
/// records and one `api` record, each of an address row both sides write
/// with `-` identity and address, and the dead `mgrver`: accepted as it
/// is; refused with a record in the Rust export's `[bad]`, `[unknown]` or
/// `[residue]`, a record in Go's `[unknown]`, another dead item in Go's
/// `[residue]`, a `[bad]` reason other than the two, an `unreached` record
/// of a row the API returned, an `api` record of an account with no such
/// row, a value present, or a row the Rust side writes with its identity.
#[test]
fn uncompared_sections_refuse_what_the_rule_does_not_name() {
    let (go, rust) = exports("06-addresses");
    check_uncompared(&go, &rust).expect("the frozen export passes");
    let refused = |go: &Export, rust: &Export, what: &str| {
        let e = check_uncompared(go, rust).expect_err(what);
        eprintln!("{what}: {e}");
    };
    for name in ["bad", "unknown", "residue"] {
        let mut r = rust.clone();
        let rec = section_mut(&mut go.clone(), name).records.first().cloned();
        let rec = rec.unwrap_or_else(|| Record {
            ty: 0,
            values: vec!["\"wtxmgr\"".to_owned(), "00".to_owned(), "-".to_owned()],
        });
        section_mut(&mut r, name).records.push(rec);
        refused(&go, &r, &format!("a Rust [{name}] record"));
    }
    let mut g = go.clone();
    section_mut(&mut g, "unknown").records.push(Record {
        ty: 0,
        values: vec!["\"wtxmgr\"".to_owned(), "00".to_owned(), "-".to_owned()],
    });
    refused(&g, &rust, "a Go [unknown] record");
    let mut g = go.clone();
    section_mut(&mut g, "residue").records[0].values[0] = "\"wstakemgr\"".to_owned();
    refused(&g, &rust, "another dead item in Go's [residue]");
    let unreached = "421405614c683be3116296458fb04e64e83d317941b8161e47a901dde2ac32c9";
    let reached = "19c1ae5e84c70bbdadd1972108de98bba50d9dab69ed43c274e24fbf6a1b4bc2";
    for (rec, what) in [
        (bad("waddrmgr/addr", unreached, "len"), "reason len"),
        (
            bad("waddrmgr/addr", reached, "unreached"),
            "unreached of a reached row",
        ),
        (
            bad("waddrmgr/addracctidx", "01000000", "api"),
            "api of an account with no such row",
        ),
        (
            bad("waddrmgr/addracctidx", "000000", "api"),
            "api of a 3-byte key",
        ),
        (
            bad("waddrmgr/acctid", unreached, "unreached"),
            "unreached on another path",
        ),
    ] {
        let mut g = go.clone();
        section_mut(&mut g, "bad").records.push(rec);
        refused(&g, &rust, what);
    }
    let mut g = go.clone();
    section_mut(&mut g, "bad").records[0].values[3] = "00".to_owned();
    refused(&g, &rust, "a value present");
    // The Rust side writing the unreached row's identity breaks the tie.
    let mut r = rust.clone();
    let addrs = section_mut(&mut r, "addresses");
    let id = addrs.types[0].field_index("id").expect("id");
    let key = addrs.types[0].field_index("key").expect("key");
    let row = addrs
        .records
        .iter_mut()
        .find(|x| x.values[key] == unreached)
        .expect("the unreached row");
    row.values[id] = "00".repeat(20);
    refused(&go, &r, "an identity on the Rust side");
}

/// Go's records of the hand-written script `name`, as frozen in
/// `tests/data/addrmgr_opvectors.txt` (whose digests and counts
/// `tests/frozen.rs` checks).
fn frozen_records(name: &str) -> Vec<Value> {
    let text =
        std::fs::read_to_string(data_dir().join("addrmgr_opvectors.txt")).expect("frozen vectors");
    let mut current = None;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (kind, rest) = line.split_once(' ').expect("tagged line");
        match kind {
            "name" => current = Some(rest),
            "result" if current == Some(name) => {
                out.push(purse_testutil::serde_json::from_str(rest).expect("result JSON"));
            }
            _ => {}
        }
    }
    assert!(!out.is_empty(), "no frozen records of {name}");
    out
}

/// The literals of the `TestCoinbases` port (`tests/txstore.rs`,
/// `coinbases`; `support::UPSTREAM_COINBASE_BALANCES`) are Go's: the
/// `store.account_balance` members of `38-upstream-tx-tests`' coinbase
/// group, the upstream test's steps run by dcrwallet's own udb, return
/// the same immature and spendable amounts at each step. Upstream never
/// compares the spendable ones (QK-0650).
#[test]
fn the_coinbase_port_asserts_gos_balances() {
    let records = frozen_records("38-upstream-tx-tests");
    let group = records
        .iter()
        .find(|r| {
            r["op"] == "update"
                && r["ops"][0]["op"] == "tx.build"
                && r["ops"][1]["op"] == "store.extend_main_chain"
                && r["ops"][2]["op"] == "store.insert_mined_tx"
        })
        .expect("the coinbase group");
    assert!(group.get("err").is_none(), "{group}");
    let got: Vec<(i64, i64)> = group["ops"]
        .as_array()
        .expect("members")
        .iter()
        .filter(|m| m["op"] == "store.account_balance")
        .map(|m| {
            let b = &m["ok"];
            (
                b["immaturecoinbaserewards"].as_i64().expect("immature"),
                b["spendable"].as_i64().expect("spendable"),
            )
        })
        .collect();
    let want: Vec<(i64, i64)> = support::UPSTREAM_COINBASE_BALANCES
        .iter()
        .map(|(_, immature, spendable)| (*immature, *spendable))
        .collect();
    assert_eq!(got, want, "Go's balances at TestCoinbases' steps");
}

/// `rt.checkpoint` takes Go's names and place: a name that is `final`,
/// empty, holds `/`, `\` or `.`, or repeats an earlier one, an argument
/// other than `name`, and a checkpoint inside a group are script faults on
/// both sides (Go's command fails; the Rust run returns an error), and a
/// well-formed checkpoint passes on both. The name `wallet` is refused on
/// both sides too, Go's for its file layout rather than its name check:
/// its snapshot `wallet.db` would be its own database, so `exportNow`'s
/// exclusive create fails with "file exists".
#[test]
fn checkpoint_names_are_refused_as_go_refuses_them() {
    const GO_DATABASE_CASE: &str = "the stem of Go's database";
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let cp = |name: &str| json!({"op": "rt.checkpoint", "name": name});
    let script = |ops: Vec<Value>| {
        json!({
            "format": "purse-opscript 1",
            "name": "checkpoint names",
            "setup": {"network": "simnet", "seed": "000102030405060708090a0b0c0d0e0f",
                      "pubpass": "public", "privpass": "private",
                      "rand": "3838383838383838383838383838383838383838383838383838383838383838",
                      "clock": 1_795_800_000},
            "ops": ops,
        })
    };
    let cases = [
        ("final", vec![cp("final")]),
        ("empty", vec![cp("")]),
        ("a slash", vec![cp("a/b")]),
        ("a backslash", vec![cp("a\\b")]),
        ("a dot", vec![cp("a.b")]),
        ("repeated", vec![cp("x"), cp("x")]),
        (
            "an unknown argument",
            vec![json!({"op": "rt.checkpoint", "name": "x", "label": "y"})],
        ),
        (
            "in a group",
            vec![json!({"op": "update", "ops": [cp("x")]})],
        ),
        (GO_DATABASE_CASE, vec![cp(runner::GO_DATABASE_STEM)]),
    ];
    for (what, ops) in cases {
        let s = script(ops);
        let dir = scratch("cp-names");
        let go = o.try_call(
            "run",
            json!({"script": s, "dir": dir.join("go").to_str().expect("utf-8 path")}),
        );
        assert!(go.is_err(), "{what}: Go ran the script");
        if what == GO_DATABASE_CASE {
            let text = go.err().map(|e| e.text).unwrap_or_default();
            assert!(
                text.contains("wallet.db") && text.contains("file exists"),
                "{what}: Go refused it otherwise: {text}"
            );
        }
        assert!(
            runner::run(&s, &dir.join("rust")).is_err(),
            "{what}: the Rust runner ran the script"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    let s = script(vec![cp("x"), cp("y-1_Z")]);
    let dir = scratch("cp-names");
    let (results, _) = go_run(&mut o, &s, &dir.join("go"));
    let rust = runner::run(&s, &dir.join("rust")).expect("Rust runner");
    for (g, r) in results.iter().zip(&rust.results) {
        check_record("checkpoint names", g, r);
    }
    assert_eq!(rust.checkpoints.len(), 2);
    assert!(dir.join("rust").join("y-1_Z.pe").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The corpus scripts `docs/QUIRKS.md` cites by record index, each with
/// its operation count and the SHA-256 of its operations' names (one per
/// line, each ending in a newline), as of the citations' last check.
/// When this test fails, the script's operations moved: find each
/// citation of the script in `QUIRKS.md` (its name in backticks on a line
/// citing `record N`), map its indexes onto the new script through
/// `tests/data/addrmgr_opvectors.txt` (the records keep their content and
/// change only `i`), correct the citations, then update the pin.
const CITED_BY_INDEX: &[(&str, usize, &str)] = &[
    (
        "35-txstore-commitments",
        129,
        "7864d1607256ea028ef9881674cabeba0be731df91cf39b1fe9a38d1c68dec73",
    ),
    (
        "36-txstore-multisig",
        138,
        "084c14c7404bf1b1cd8d71284aff84f435def2df25c7dcbc0111330c048d761e",
    ),
    (
        "37-txstore-selection",
        115,
        "d6c8fe2f497a44201db1ca46c7ddfa4cc3267e6dd8f8e0ada6b48f8a25417435",
    ),
];

/// Whether a line cites a record by number (`record 7`, `records 75 to
/// 81`, `Records 61`).
fn cites_a_record_index(line: &str) -> bool {
    ["record ", "records ", "Record ", "Records "]
        .iter()
        .any(|key| {
            line.match_indices(key).any(|(at, _)| {
                line[at + key.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
        })
}

/// A record cited by index in `QUIRKS.md` stays the record cited: every
/// corpus script named on a line that cites a record by number (the line
/// naming that script alone) is pinned in [`CITED_BY_INDEX`], and each pin
/// still holds, so a change that moves such a script's operations fails
/// here until the citations are checked again. No oracle: it reads the
/// scripts and the document.
#[test]
fn quirk_citations_by_record_index_are_pinned() {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::fmt::Write as _;

    let scripts = data_dir().join("opscripts");
    let quirks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/QUIRKS.md"),
    )
    .expect("read docs/QUIRKS.md");
    let mut cited = BTreeSet::new();
    for line in quirks.lines().filter(|l| cites_a_record_index(l)) {
        let named: BTreeSet<&str> = line
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|t| scripts.join(format!("{t}.json")).is_file())
            .collect();
        if named.len() == 1 {
            cited.extend(named);
        }
    }
    let pinned: BTreeSet<&str> = CITED_BY_INDEX.iter().map(|(n, _, _)| *n).collect();
    assert_eq!(
        cited, pinned,
        "QUIRKS.md cites records of these scripts by index; pin each in CITED_BY_INDEX"
    );
    for (name, count, digest) in CITED_BY_INDEX {
        let text = std::fs::read_to_string(scripts.join(format!("{name}.json")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let script: Value =
            purse_testutil::serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let ops = script["ops"].as_array().expect("ops");
        let mut h = Sha256::new();
        for op in ops {
            h.update(op["op"].as_str().expect("op name").as_bytes());
            h.update(b"\n");
        }
        let got = h.finalize().iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        assert_eq!(
            (ops.len(), got.as_str()),
            (*count, *digest),
            "{name}: its operations moved; check every record QUIRKS.md cites in it \
             (CITED_BY_INDEX says how), then update the pin"
        );
    }
}
