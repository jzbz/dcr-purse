// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against the Go oracle `tools/oracle/cmd/snacl` (a
//! verbatim copy of dcrwallet's snacl, dcrwallet's kdf, a verbatim copy of
//! udb's seal/unseal, udb's public API), over inputs generated from
//! `PURSE_TEST_SEED`; the replay of every frozen file in `tests/data`
//! against Go; and their regeneration:
//!
//! ```text
//! PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl \
//!     --test oracle -- --ignored regenerate_frozen_vectors
//! ```
//!
//! Regenerating `snacl_vectors.txt`, `generated_db.txt` and
//! `generated-simnet.db.gz` does not reproduce them byte for byte: Go draws
//! salts, nonces and keys from dcrd's `crypto/rand`, seeded from the
//! operating system, which the oracle cannot steer, and later requests are
//! built from earlier answers. So the `frozen_*_against_go` tests send every
//! frozen entry, and the frozen database, back to Go on each run with the
//! oracle, which proves them Go's without regenerating them.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters on small, bounded values"
)]

mod common;

use std::collections::BTreeSet;
use std::io::Write as _;

use common::*;
use purse_snacl::{Argon2idParams, NONCE_SIZE, OVERHEAD, XCHACHA20POLY1305_OVERHEAD};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip};
use sha2::{Digest, Sha256};

fn ask(oracle: &mut Oracle) -> impl FnMut(&str, &Value) -> Value + '_ {
    |cmd, args| oracle.call(cmd, args.clone())
}

#[test]
fn differential_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-snacl differential");
    let entries = build_corpus(&mut rng, &mut ask(&mut oracle));
    par_map(&entries, check_entry);
    let cmds: BTreeSet<&str> = entries.iter().map(|e| e.cmd.as_str()).collect();
    assert_eq!(cmds.len(), 17, "{cmds:?}");
    eprintln!("{} requests agree with Go", entries.len());
}

#[test]
fn generated_database_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let num_cpu = oracle.call("info", Value::Null)["num_cpu"]
        .as_u64()
        .expect("num_cpu");
    let dir = scratch("generated-live");
    let path = dir.join("wallet.db");
    let mut request = make_db_request("simnet");
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("make-db", request.clone());
    let expect = outcome
        .get("ok")
        .unwrap_or_else(|| panic!("make-db failed: {outcome}"));
    let bytes = std::fs::read(&path).expect("read the database");
    let walk = check_generated_db(bytes, &request, expect);
    // kdf.NewArgon2idParams stored the oracle machine's CPU count.
    let threads = Argon2idParams::threads_for_cpus(usize::try_from(num_cpu).expect("fits"));
    assert_eq!(walk.unique_threads, vec![threads, threads]);
}

#[test]
fn frozen_vectors_against_go() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    if !replay_against_go(&entries) {
        return;
    }
    eprintln!("{} frozen entries replayed against Go", entries.len());
}

#[test]
fn frozen_fixture_vectors_against_go() {
    let entries = read_vectors(FIXTURE_VECTORS, FIXTURE_VECTORS_ENTRIES);
    assert!(
        entries
            .iter()
            .all(|e| DETERMINISTIC.contains(&e.cmd.as_str()))
    );
    replay_against_go(&entries);
}

/// The checked-in database, opened by Go through udb's public API with the
/// recorded passphrases: every member of the recorded result is read back
/// from it (open-db), and must equal what make-db reported when it wrote it.
#[test]
fn frozen_generated_database_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let line = read_generated_expect();
    let bytes = gunzip(&data_dir().join(GENERATED_DB));
    assert_eq!(
        line["db_sha256"].as_str(),
        Some(hex(&Sha256::digest(&bytes)).as_str()),
        "{GENERATED_DB} is not the database the expectations were recorded from"
    );
    // A copy: opening a database may write to it.
    let path = scratch("generated-frozen-replay").join("wallet.db");
    std::fs::write(&path, &bytes).expect("write the database copy");
    let mut request = line["request"].clone();
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("open-db", request);
    assert_eq!(
        outcome,
        json!({ "ok": line["result"] }),
        "Go reads back what it recorded"
    );
}

/// The commands whose outcome depends on nothing but their arguments.
/// argon2-new-params is among them (its reader is the request's stream),
/// except for the machine's CPU count, which its answer reports.
const DETERMINISTIC: [&str; 11] = [
    "marshal",
    "unmarshal",
    "derive-key",
    "crypto-decrypt",
    "secret-decrypt",
    "unseal",
    "account-unseal",
    "argon2-marshal",
    "argon2-unmarshal",
    "argon2-derive",
    "argon2-new-params",
];

/// Replays frozen entries against Go, on several oracle processes at once;
/// false when there is no oracle (the test skips).
fn replay_against_go(entries: &[Entry]) -> bool {
    if oracle_or_skip("snacl").is_none() {
        return false;
    }
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let chunk = entries.len().div_ceil(workers).max(1);
    std::thread::scope(|s| {
        for part in entries.chunks(chunk) {
            s.spawn(move || {
                let mut oracle = oracle_or_skip("snacl").expect("the oracle was available");
                for e in part {
                    replay_entry(&mut oracle, e);
                }
            });
        }
    });
    true
}

/// Checks one frozen entry against Go: a deterministic request must give
/// the recorded outcome again; a request that drew randomness is checked by
/// its inverse, since Go cannot be made to draw the same bytes again.
fn replay_entry(oracle: &mut Oracle, e: &Entry) {
    let ctx = format!("{} {}", e.cmd, e.args);
    let a = &e.args;
    let ok = e.outcome.get("ok");
    let mut call = |cmd: &str, args: Value| oracle.call(cmd, args);
    match (e.cmd.as_str(), ok) {
        ("argon2-new-params", _) => {
            let live = call(&e.cmd, a.clone());
            assert_eq!(
                without_cpu_count(&live),
                without_cpu_count(&e.outcome),
                "{ctx}"
            );
        }
        (cmd, _) if DETERMINISTIC.contains(&cmd) => {
            assert_eq!(call(cmd, a.clone()), e.outcome, "{ctx}");
        }
        ("new-secret-key", Some(ok)) => {
            let params = &ok["params"];
            let u = call("unmarshal", json!({ "data": params }));
            for k in ["n", "r", "p"] {
                assert_eq!(u["ok"][k], a[k], "{ctx}: {k}");
            }
            assert_eq!(
                call("derive-key", json!({"params": params, "pass": a["pass"]})),
                json!({"ok": {"key": ok["key"], "key_after": ok["key"]}}),
                "{ctx}: the key derived again from the recorded parameters"
            );
        }
        ("generate-crypto-key", Some(ok)) => {
            let key = &ok["key"];
            assert_eq!(key.as_str().map(str::len), Some(64), "{ctx}");
            let pt = hex(b"a crypto key round trip");
            let sealed = call("crypto-encrypt", json!({"key": key, "plaintext": pt}));
            assert_eq!(
                call(
                    "crypto-decrypt",
                    json!({"key": key, "sealed": sealed["ok"]["sealed"]})
                ),
                json!({"ok": {"plaintext": pt}}),
                "{ctx}"
            );
        }
        (cmd @ ("crypto-encrypt" | "secret-encrypt" | "seal"), Some(ok)) => {
            let open = match cmd {
                "crypto-encrypt" => "crypto-decrypt",
                "secret-encrypt" => "secret-decrypt",
                _ => "unseal",
            };
            let sealed = ok["sealed"].as_str().expect("sealed");
            let pt = a["plaintext"].as_str().expect("plaintext");
            let overhead = if cmd == "seal" {
                XCHACHA20POLY1305_OVERHEAD
            } else {
                NONCE_SIZE + OVERHEAD
            };
            assert_eq!(sealed.len(), pt.len() + 2 * overhead, "{ctx}");
            assert_eq!(
                call(open, json!({"key": a["key"], "sealed": sealed})),
                json!({"ok": {"plaintext": pt}}),
                "{ctx}: the recorded box opens to the recorded plaintext"
            );
        }
        ("account-seal", Some(ok)) => {
            let sealed = ok["sealed"].as_str().expect("sealed");
            let pt = a["plaintext"].as_str().expect("plaintext");
            assert_eq!(
                sealed.len(),
                pt.len() + 2 * XCHACHA20POLY1305_OVERHEAD,
                "{ctx}"
            );
            assert_eq!(
                call(
                    "account-unseal",
                    json!({"params": a["params"], "pass": a["pass"], "sealed": sealed})
                ),
                json!({"ok": {"key": ok["key"], "plaintext": pt}}),
                "{ctx}: the recorded blob opens to the recorded plaintext"
            );
        }
        // A request that failed drew nothing visible: the failure is
        // deterministic.
        (
            "new-secret-key"
            | "generate-crypto-key"
            | "crypto-encrypt"
            | "secret-encrypt"
            | "seal"
            | "account-seal",
            None,
        ) => {
            assert_eq!(call(&e.cmd, a.clone()), e.outcome, "{ctx}");
        }
        (cmd, _) => panic!("no Go replay for {cmd}"),
    }
}

/// An argon2-new-params outcome without what depends on the machine: the
/// CPU count it reports, and the thread count (the last byte of the
/// parameters) set from it, which must be the count Go's truncation gives.
fn without_cpu_count(outcome: &Value) -> Value {
    let mut o = outcome.clone();
    let member = if o.get("ok").is_some() { "ok" } else { "err" };
    let body = o
        .get_mut(member)
        .and_then(Value::as_object_mut)
        .expect("ok or err");
    let cpus = body
        .remove("num_cpu")
        .and_then(|v| v.as_u64())
        .expect("num_cpu");
    if let Some(params) = body.get("params").and_then(Value::as_str) {
        let mut bytes = purse_testutil::unhex(params);
        let threads = bytes.pop().expect("25 bytes");
        let cpus = usize::try_from(cpus).expect("fits");
        assert_eq!(threads, Argon2idParams::threads_for_cpus(cpus), "{outcome}");
        body.insert("params".to_owned(), json!(hex(&bytes)));
    }
    o
}

#[test]
#[ignore = "writes tests/data from the Go oracle; set PURSE_SNACL_REGEN=1"]
fn regenerate_frozen_vectors() {
    if !regen_requested() {
        eprintln!("skipping: set PURSE_SNACL_REGEN=1 to regenerate tests/data");
        return;
    }
    let mut oracle = oracle_or_skip("snacl").expect("regeneration needs the oracle");
    // The release alone: a distribution's build appends its own tags.
    let go = oracle
        .go_version()
        .split(['-', ' ', '+'])
        .next()
        .unwrap_or_default()
        .to_owned();

    // The general corpus.
    let mut rng = SplitMix64(FROZEN_RNG_SEED);
    let entries = build_corpus(&mut rng, &mut ask(&mut oracle));
    par_map(&entries, check_entry);
    eprintln!("{NOT_REPRODUCIBLE}");
    let mut header = provenance("General vectors for purse-snacl", &go);
    header.extend(not_reproducible_note());
    header.extend([
        format!(
            "Inputs: tests/common/mod.rs build_corpus with SplitMix64({FROZEN_RNG_SEED:#x}); later requests"
        ),
        "use earlier answers. Each line is {cmd, args, outcome}: the request as sent and Go's".to_owned(),
        "outcome, in the shapes tools/oracle/cmd/snacl/main.go documents. Lines with a".to_owned(),
        "\"divergence\" member pin a recorded difference (see the crate docs).".to_owned(),
        String::new(),
    ]);
    write_vectors(VECTORS, &header, &entries);

    // The upstream fixtures.
    let mut fixture_entries = Vec::new();
    for (file, wrong) in FIXTURES {
        let flow = fixture_flow(file, wrong);
        let go_flow: Vec<Entry> = flow
            .iter()
            .map(|e| Entry {
                outcome: oracle.call(&e.cmd, e.args.clone()),
                ..e.clone()
            })
            .collect();
        check_flow_against(&flow, &go_flow, file);
        fixture_entries.extend(go_flow);
    }
    let mut header = provenance("Upstream fixture vectors for purse-snacl", &go);
    header.extend([
        "Inputs: the master key parameters and sealed keys of dcrwallet's own fixture".to_owned(),
        "databases (crates/purse-boltread/tests/data/upstream, from wallet/udb/testdata),"
            .to_owned(),
        "opened with the passphrases their generators name (\"public\", \"private\"), as"
            .to_owned(),
        "tests/common/mod.rs fixture_flow requests them. The \"fixture\" member names the"
            .to_owned(),
        "database. key_after is Go's diagnostic, not compared for these.".to_owned(),
        "These requests draw nothing at random: regeneration reproduces this file byte for"
            .to_owned(),
        "byte, and frozen_fixture_vectors_against_go replays it against Go on every oracle run."
            .to_owned(),
        String::new(),
    ]);
    write_vectors(FIXTURE_VECTORS, &header, &fixture_entries);

    // The generated database.
    let dir = scratch("generated-regen");
    let path = dir.join("wallet.db");
    let mut request = make_db_request("simnet");
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("make-db", request.clone());
    let expect = outcome.get("ok").expect("make-db").clone();
    let bytes = std::fs::read(&path).expect("read the database");
    check_generated_db(bytes.clone(), &request, &expect);
    let gz = std::fs::File::create(data_dir().join(GENERATED_DB)).expect("create db.gz");
    let mut enc = flate2::write::GzEncoder::new(gz, flate2::Compression::best());
    enc.write_all(&bytes).expect("compress");
    enc.finish().expect("finish gzip");
    if let Some(o) = request.as_object_mut() {
        o.remove("path");
    }
    let line = serde_json::to_string(&json!({
        "request": request,
        "result": expect,
        "db_sha256": hex(&Sha256::digest(&bytes)),
    }))
    .expect("serialize");
    let mut text = String::new();
    let mut header = provenance("The generated database for purse-snacl", &go);
    header.extend(not_reproducible_note());
    for h in header {
        text.push_str(&format!("# {h}\n").replace("# \n", "#\n"));
    }
    for h in [
        format!("{GENERATED_DB} was written by the oracle's make-db through udb's public API"),
        "(Initialize, then a second account given its own passphrase with".to_owned(),
        "SetAccountPassphrase, an imported voting account, an imported private key and an"
            .to_owned(),
        "imported script); the line below holds the request (path omitted), Go's result and"
            .to_owned(),
        "the SHA-256 of the decompressed database. Argon2id threads are the generating".to_owned(),
        "machine's CPU count, as dcrwallet stores them.".to_owned(),
    ] {
        text.push_str(&format!("# {h}\n"));
    }
    text.push_str("# entries: 1\n");
    text.push_str(&format!("# body-sha256: {}\n", body_sha256(&line)));
    text.push_str(&line);
    text.push('\n');
    std::fs::write(data_dir().join(GENERATED_EXPECT), text).expect("write expectations");
    eprintln!("wrote tests/data/{GENERATED_DB} and tests/data/{GENERATED_EXPECT}");
}

/// What the regeneration prints, and the headers of the files it concerns
/// say.
const NOT_REPRODUCIBLE: &str = "note: snacl_vectors.txt, generated_db.txt and generated-simnet.db.gz \
     will not regenerate byte for byte: Go draws salts, nonces and keys from dcrd's crypto/rand, \
     which the oracle cannot steer, and later requests use earlier answers";

/// The header lines of the files whose regeneration is not reproducible.
fn not_reproducible_note() -> Vec<String> {
    [
        "Regeneration does not reproduce this file byte for byte: Go draws salts, nonces and",
        "keys from github.com/decred/dcrd/crypto/rand, a generator seeded from the operating",
        "system that the oracle cannot steer, and later requests are built from earlier",
        "answers. So tests/oracle.rs replays every entry (and the database) against Go on",
        "every run with the oracle: deterministic requests must give the recorded outcome",
        "again, and each random draw is checked by its inverse (the key derived again from",
        "the recorded parameters, the recorded box opened to the recorded plaintext).",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The header lines every frozen file starts with.
fn provenance(title: &str, go: &str) -> Vec<String> {
    vec![
        title.to_owned(),
        String::new(),
        "Every expected value is the output of the Go oracle tools/oracle/cmd/snacl, built with"
            .to_owned(),
        format!("{go}, which links dcrwallet {DCRWALLET_PARITY_COMMIT}'s"),
        "kdf and udb packages, a verbatim copy of its wallet/internal/snacl and a verbatim copy"
            .to_owned(),
        "of udb's seal/unseal. Regenerate with:".to_owned(),
        "  PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl --test oracle \\"
            .to_owned(),
        "      -- --ignored regenerate_frozen_vectors".to_owned(),
        "which also checks the Rust crate agrees. Do not edit by hand.".to_owned(),
    ]
}
