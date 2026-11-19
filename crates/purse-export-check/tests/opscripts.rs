// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The operation-script runner (`tools/oracle/cmd/oprunner`, format
//! `purse-opscript 1`, `docs/adr/0002-operation-script-format.md`) against
//! dcrwallet's own udb:
//!
//! - the hand-written corpus under `tests/data/opscripts` (one script per
//!   operation family, together covering the whole vocabulary) and a few
//!   hundred generated scripts each run twice to byte-identical results and
//!   exports;
//! - a change of only the injected randomness, or only the injected clock,
//!   changes only the result values marked as depending on it, and only the
//!   volatile fields of the exports (and, for the clock, the clock fields:
//!   the exports then agree under the masked-clock profile);
//! - every export whose private passphrase was supplied has a `yes` verdict
//!   for every sealed secret (ADR-0001 section 8), which also holds the
//!   runner's passphrase bookkeeping to the database it snapshots;
//! - every error text is rendered with dcrwallet's daemon separator;
//! - every export the runner writes parses with this crate's strict parser
//!   and carries the digests the runner reported;
//! - the frozen vectors under `tests/data/opvectors` (scripts with Go's
//!   results and final exports, the first cross-implementation vectors)
//!   are still what Go produces, and are well formed without Go.
//!
//! `PURSE_OPSCRIPT_RANDOM` sets the number of generated scripts (default
//! 300); `PURSE_TEST_SEED` reproduces a generated corpus.
//! `PURSE_REGEN_OPVECTORS=1` rewrites the frozen vectors from Go.

// Test code: an overflow here is a test bug and panics loudly.
#![allow(clippy::arithmetic_side_effects)]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{data_dir, path_str};
use purse_export_check::{Export, Profile, parse, render_with};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{Oracle, SplitMix64, hex, oracle_or_skip};

const SCRIPT_FORMAT: &str = "purse-opscript 1";
const RESULTS_FORMAT: &str = "purse-opresults 1";
const VECTOR_MAGIC: &str = "# purse-opvector 1";

/// The scripts frozen as cross-implementation vectors.
const FROZEN: [&str; 6] = [
    "02-addresses",
    "08-unmined",
    "12-policies",
    "16-tolerated-errors",
    "17-resolution-errors",
    "18-wallet-addresses",
];

/// dcrwallet's error kinds, and the runner's kind for a Go panic.
const KINDS: [&str; 22] = [
    "Other",
    "Bug",
    "Invalid",
    "Permission",
    "IO",
    "Exist",
    "NotExist",
    "Encoding",
    "Crypto",
    "Locked",
    "Passphrase",
    "Seed",
    "WatchingOnly",
    "InsufficientBalance",
    "ScriptFailure",
    "Policy",
    "Consensus",
    "DoubleSpend",
    "Protocol",
    "NoPeers",
    "Deployment",
    "Panic",
];

fn opscripts_dir() -> PathBuf {
    data_dir().join("opscripts")
}

fn opvectors_dir() -> PathBuf {
    data_dir().join("opvectors")
}

/// A scratch directory under cargo's target tmpdir, emptied first.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-export-check")
        .join("opscripts")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The hand-written corpus, by file stem, in name order.
fn corpus() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(opscripts_dir()).expect("read opscripts") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            let stem = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned();
            let text = std::fs::read_to_string(&path).expect("read script");
            let script: Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            out.push((stem, script));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// What one run left behind.
struct Run {
    result: Value,
    results_text: String,
    /// Every export, by name ("final" and the checkpoints): its text.
    exports: Vec<(String, String)>,
}

fn run_script(o: &mut Oracle, script: &Value, dir: &Path) -> Run {
    let result = o
        .try_call("run", json!({"script": script, "dir": path_str(dir)}))
        .unwrap_or_else(|e| panic!("run in {}: {}", dir.display(), e.text));
    let results_text =
        std::fs::read_to_string(dir.join("results.jsonl")).expect("read results.jsonl");
    let mut exports = Vec::new();
    for name in result["exports"].as_object().expect("exports").keys() {
        let text = std::fs::read_to_string(dir.join(format!("{name}.pe"))).expect("read export");
        exports.push((name.clone(), text));
    }
    Run {
        result,
        results_text,
        exports,
    }
}

/// Checks the shape of results.jsonl and that every export parses with the
/// digests the runner reported.
fn check_run(what: &str, run: &Run) {
    let mut lines = run.results_text.lines();
    assert_eq!(lines.next(), Some(RESULTS_FORMAT), "{what}: results format");
    let records: Vec<Value> = lines
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{what}: {l}: {e}")))
        .collect();
    assert_eq!(
        Value::Array(records.clone()),
        run.result["results"],
        "{what}: results.jsonl and the returned results differ"
    );
    for (i, rec) in records.iter().enumerate() {
        assert_eq!(rec["i"], json!(i), "{what}: record index");
        check_record(what, rec);
    }
    let digest = sha2_hex(run.results_text.as_bytes());
    assert_eq!(run.result["results_sha256"], json!(digest), "{what}");
    for (name, text) in &run.exports {
        let ex = parse(text).unwrap_or_else(|e| panic!("{what}: export {name}: {e}"));
        let summary = &run.result["exports"][name];
        assert_eq!(summary["sha256~cr"], json!(ex.sha256), "{what}: {name}");
        assert_eq!(summary["msha256~c"], json!(ex.msha256), "{what}: {name}");
        assert_eq!(summary["file_sha256~cr"], json!(sha2_hex(text.as_bytes())));
        assert_eq!(ex.meta("exporter"), Some("tools/oracle/cmd/oprunner"));
        // [bad] and [orphans] may be populated: udb's public API can
        // leave rows its readers never reach (02-addresses records a
        // derived address under a public key that is not the derived one).
        let unknown = ex.section("unknown").expect("section");
        assert!(unknown.records.is_empty(), "{what}: {name} [unknown]");
        // The exporter got the passphrases the runner's bookkeeping holds:
        // they must open every sealed secret of the snapshot (a passphrase
        // change rolled back with its group must not leave the new one
        // here).
        if ex.meta("privcheck") == Some("1") {
            ex.check_sealed_secrets()
                .unwrap_or_else(|e| panic!("{what}: export {name}: {e}"));
        }
    }
}

fn check_record(what: &str, rec: &Value) {
    let obj = rec.as_object().expect("record object");
    assert!(obj["op"].is_string(), "{what}: {rec}");
    assert!(
        obj.contains_key("ok") != obj.contains_key("err"),
        "{what}: exactly one of ok and err: {rec}"
    );
    if let Some(err) = obj.get("err") {
        let kind = err["kind"].as_str().expect("kind");
        assert!(KINDS.contains(&kind), "{what}: kind {kind}");
        let text = err["text"].as_str().expect("text");
        // dcrwallet's daemon sets errors.Separator to ":: "
        // (dcrwallet.go:38); the package default is ":\n\t".
        assert!(
            !text.contains("\n\t"),
            "{what}: error text with the package's default separator: {rec}"
        );
    }
    if let Some(t) = obj.get("tolerated") {
        assert_eq!(t, &json!(true), "{what}: {rec}");
        assert!(
            obj.contains_key("err"),
            "{what}: tolerated without an error: {rec}"
        );
        assert_ne!(
            obj["err"]["kind"], "Panic",
            "{what}: a tolerated panic: {rec}"
        );
    }
    if let Some(ops) = obj.get("ops") {
        for (j, sub) in ops.as_array().expect("ops").iter().enumerate() {
            assert_eq!(sub["i"], json!(j));
            check_record(what, sub);
        }
    }
}

fn sha2_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(b))
}

/// Two runs of one script must leave byte-identical results and exports.
fn assert_identical(what: &str, a: &Run, b: &Run) {
    assert!(
        a.results_text == b.results_text,
        "{what}: results differ between two runs:\n{}",
        first_difference(&a.results_text, &b.results_text)
    );
    assert_eq!(a.exports.len(), b.exports.len(), "{what}");
    for ((na, ta), (nb, tb)) in a.exports.iter().zip(&b.exports) {
        assert_eq!(na, nb, "{what}");
        assert!(
            ta == tb,
            "{what}: export {na} differs between two runs:\n{}",
            first_difference(ta, tb)
        );
    }
    assert_eq!(a.result, b.result, "{what}: returned results differ");
}

fn first_difference(a: &str, b: &str) -> String {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return format!("line {}:\n- {la}\n+ {lb}", i + 1);
        }
    }
    format!("lengths {} and {}", a.len(), b.len())
}

/// Masks the result values that may depend on the injected source
/// `letter` ('r' randomness, 'c' clock): members whose key ends in "~"
/// followed by letters including `letter`, and (for randomness) the byte
/// counts drawn, which depend on rejection sampling.
fn mask(v: &Value, letter: char) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    let volatile = k
                        .rsplit_once('~')
                        .is_some_and(|(_, tag)| tag.contains(letter))
                        || (letter == 'r' && k == "drawn");
                    let x = if volatile {
                        json!("*")
                    } else {
                        mask(x, letter)
                    };
                    (k.clone(), x)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| mask(x, letter)).collect()),
        _ => v.clone(),
    }
}

/// The record lines of every section under `profile`.
fn section_lines(ex: &Export, profile: Profile) -> Vec<(String, Vec<String>)> {
    ex.sections
        .iter()
        .map(|s| {
            let lines = s
                .records
                .iter()
                .map(|r| render_with(&s.types[r.ty], &r.values, profile))
                .collect();
            (s.name.clone(), lines)
        })
        .collect()
}

/// With only the source `letter` changed, results must agree once its
/// dependent values are masked, and every export must agree under the
/// masked profile for the randomness (only `~` fields differ) or the
/// masked-clock profile for the clock (only `~` and `@` fields differ:
/// received and birthday times follow the injected clock, ADR-0001 section
/// 8). Returns whether anything marked as dependent changed, for the
/// caller's sanity check.
fn assert_only_volatile_changes(what: &str, letter: char, base: &Run, varied: &Run) -> bool {
    let a = mask(&base.result["results"], letter);
    let b = mask(&varied.result["results"], letter);
    if a != b {
        let (ra, rb) = (
            a.as_array().expect("results"),
            b.as_array().expect("results"),
        );
        let i = ra.iter().zip(rb).position(|(x, y)| x != y).unwrap_or(0);
        panic!(
            "{what}: changing '{letter}' changed op {i} beyond its marked values:\n- {}\n+ {}",
            ra[i], rb[i]
        );
    }
    let profile = if letter == 'c' {
        Profile::MASKED_CLOCK
    } else {
        Profile::MASKED
    };
    for ((name, ta), (_, tb)) in base.exports.iter().zip(&varied.exports) {
        let ea = parse(ta).expect("parse");
        let eb = parse(tb).expect("parse");
        assert_eq!(ea.metas, eb.metas, "{what}: {name} metas");
        for ((sec, la), (_, lb)) in section_lines(&ea, profile)
            .into_iter()
            .zip(section_lines(&eb, profile))
        {
            assert_eq!(
                la, lb,
                "{what}: export {name} [{sec}] differs outside the fields '{letter}' may change"
            );
        }
        assert_eq!(ea.digest(profile), eb.digest(profile), "{what}: {name}");
        if letter == 'r' {
            assert_eq!(ea.msha256, eb.msha256, "{what}: {name}");
        }
    }
    base.results_text != varied.results_text
        || base
            .exports
            .iter()
            .zip(&varied.exports)
            .any(|(a, b)| a.1 != b.1)
}

/// The script with its injected randomness key or clock changed. The
/// chain-time base is pinned first, since block timestamps are chain data,
/// not clock readings (ADR-0001), and default to the clock.
fn varied(script: &Value, letter: char) -> Value {
    let mut s = script.clone();
    let setup = s["setup"].as_object_mut().expect("setup");
    if !setup.contains_key("chain_time") {
        let clock = setup["clock"].clone();
        setup.insert("chain_time".to_owned(), clock);
    }
    match letter {
        'r' => {
            let key = setup["rand"].as_str().expect("rand").to_owned();
            let flipped: String = key
                .chars()
                .map(|c| if c == 'e' { 'f' } else { 'e' })
                .collect();
            setup.insert("rand".to_owned(), json!(flipped));
        }
        _ => {
            let clock = setup["clock"].as_i64().expect("clock");
            setup.insert("clock".to_owned(), json!(clock + 3 * 86_400 + 17));
        }
    }
    s
}

/// Operation outcomes of one run: (operations, errors, panics), groups
/// counted by their members.
fn tally(records: &Value) -> (usize, usize, usize) {
    let mut t = (0, 0, 0);
    for rec in records.as_array().expect("records") {
        if let Some(ops) = rec.get("ops") {
            let s = tally(ops);
            t = (t.0 + s.0, t.1 + s.1, t.2 + s.2);
            continue;
        }
        t.0 += 1;
        if let Some(err) = rec.get("err") {
            t.1 += 1;
            if err["kind"] == "Panic" {
                t.2 += 1;
            }
        }
    }
    t
}

/// Runs a script twice (and, with `vary`, once with other randomness and
/// once with another clock) and checks everything this file promises.
/// Returns (randomness changed something, clock changed something) and the
/// tally of the first run.
fn exercise(
    o: &mut Oracle,
    what: &str,
    script: &Value,
    dir: &Path,
    vary: bool,
) -> ((bool, bool), (usize, usize, usize)) {
    let a = run_script(o, script, &dir.join("a"));
    check_run(what, &a);
    let counts = tally(&a.result["results"]);
    let b = run_script(o, script, &dir.join("b"));
    assert_identical(what, &a, &b);
    let mut changed = (false, false);
    if vary {
        let r = run_script(o, &varied(script, 'r'), &dir.join("r"));
        check_run(what, &r);
        changed.0 = assert_only_volatile_changes(what, 'r', &a, &r);
        let c = run_script(o, &varied(script, 'c'), &dir.join("c"));
        check_run(what, &c);
        changed.1 = assert_only_volatile_changes(what, 'c', &a, &c);
    }
    let _ = std::fs::remove_dir_all(dir);
    (changed, counts)
}

fn ops_used(ops: &Value, out: &mut BTreeSet<String>) {
    for op in ops.as_array().expect("ops") {
        out.insert(op["op"].as_str().expect("op name").to_owned());
        if let Some(sub) = op.get("ops") {
            ops_used(sub, out);
        }
    }
}

#[test]
fn corpus_covers_the_vocabulary() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let vocab = o.call("vocabulary", Value::Null);
    assert_eq!(vocab["format"], SCRIPT_FORMAT);
    let names: BTreeSet<String> = vocab["ops"]
        .as_array()
        .expect("ops")
        .iter()
        .map(|o| o["op"].as_str().expect("op").to_owned())
        .collect();
    let mut used = BTreeSet::new();
    let scripts = corpus();
    assert!(scripts.len() >= 12, "at least a dozen hand-written scripts");
    for (_, s) in &scripts {
        assert_eq!(s["format"], SCRIPT_FORMAT);
        ops_used(&s["ops"], &mut used);
    }
    let missing: Vec<_> = names.difference(&used).collect();
    assert!(
        missing.is_empty(),
        "operations no corpus script runs: {missing:?}"
    );
    let unknown: Vec<_> = used.difference(&names).collect();
    assert!(
        unknown.is_empty(),
        "unknown operations in the corpus: {unknown:?}"
    );
}

#[test]
fn corpus_is_deterministic_and_varies_only_in_volatile_values() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let base = scratch("corpus");
    let mut changed = (false, false);
    for (name, script) in corpus() {
        // Mainnet's scrypt parameters make the mainnet script slow; it is
        // run for determinism only.
        let vary = script["setup"]["network"] == "simnet";
        let (c, _) = exercise(&mut o, &name, &script, &base.join(&name), vary);
        changed = (changed.0 || c.0, changed.1 || c.1);
    }
    assert!(changed.0, "changing the randomness changed nothing at all");
    assert!(changed.1, "changing the clock changed nothing at all");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn generated_scripts_are_deterministic_and_vary_only_in_volatile_values() {
    let Some(probe) = oracle_or_skip("oprunner") else {
        return;
    };
    drop(probe);
    let count: usize = std::env::var("PURSE_OPSCRIPT_RANDOM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let mut rng = SplitMix64::from_env("opscripts");
    let scripts: Vec<Value> = (0..count)
        .map(|i| Gen::new(SplitMix64(rng.next_u64()), i).script())
        .collect();
    let base = scratch("generated");
    let next = AtomicUsize::new(0);
    let changed = Mutex::new((false, false));
    let totals = Mutex::new((0_usize, 0_usize, 0_usize));
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 8);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let mut o = oracle_or_skip("oprunner").expect("oracle");
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(script) = scripts.get(i) else { break };
                    let what = format!("generated script {i}");
                    let dir = base.join(format!("g{i:04}"));
                    let (c, t) = exercise(&mut o, &what, script, &dir, i.is_multiple_of(8));
                    let mut all = changed.lock().expect("lock");
                    *all = (all.0 || c.0, all.1 || c.1);
                    let mut sum = totals.lock().expect("lock");
                    *sum = (sum.0 + t.0, sum.1 + t.1, sum.2 + t.2);
                }
            });
        }
    });
    let changed = changed.into_inner().expect("lock");
    if count >= 8 {
        assert!(changed.0 && changed.1, "variations changed nothing");
    }
    let (n_ops, n_err, n_panic) = totals.into_inner().expect("lock");
    eprintln!(
        "ran {count} generated scripts: {n_ops} operations, {n_err} returned errors, \
         {n_panic} of them Go panics"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// A frozen vector: the script, Go's results.jsonl and its final export.
struct Vector {
    comments: Vec<String>,
    script: Value,
    results: String,
    export_sha256: String,
    export_msha256: String,
}

fn read_vector(name: &str) -> (Vector, String) {
    let text = std::fs::read_to_string(opvectors_dir().join(format!("{name}.opv")))
        .unwrap_or_else(|e| panic!("{name}.opv: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(VECTOR_MAGIC), "{name}");
    let mut comments = Vec::new();
    let mut script = None;
    let mut results = String::new();
    let mut digests = None;
    for line in lines {
        if let Some(c) = line.strip_prefix("# ") {
            comments.push(c.to_owned());
        } else if let Some(s) = line.strip_prefix("script ") {
            script = Some(serde_json::from_str(s).expect("script JSON"));
        } else if let Some(r) = line.strip_prefix("result ") {
            results.push_str(r);
            results.push('\n');
        } else if let Some(d) = line.strip_prefix("export ") {
            let (a, b) = d.split_once(' ').expect("two digests");
            digests = Some((a.to_owned(), b.to_owned()));
        } else {
            panic!("{name}.opv: unexpected line {line:?}");
        }
    }
    let (export_sha256, export_msha256) = digests.expect("export line");
    let export = std::fs::read_to_string(opvectors_dir().join(format!("{name}.pe")))
        .unwrap_or_else(|e| panic!("{name}.pe: {e}"));
    (
        Vector {
            comments,
            script: script.expect("script line"),
            results,
            export_sha256,
            export_msha256,
        },
        export,
    )
}

/// The export text without its header comments (not digested).
fn without_comments(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn write_vector(name: &str, script: &Value, run: &Run) {
    let how = [
        format!(
            "Script tests/data/opscripts/{name}.json run by tools/oracle/cmd/oprunner (dcrwallet's"
        ),
        "wallet/udb at the parity pin with the seeded randomness stream, the injected clock"
            .to_owned(),
        "and processor count of docs/adr/0002-operation-script-format.md): each result line is"
            .to_owned(),
        "a line of its results.jsonl, export names the eof digests of the final export beside"
            .to_owned(),
        "this file. Regenerate with PURSE_REGEN_OPVECTORS=1 cargo test -p purse-export-check"
            .to_owned(),
        "--test opscripts frozen_vectors_match_go, never by hand.".to_owned(),
    ];
    let mut out = format!("{VECTOR_MAGIC}\n");
    for c in &how {
        out.push_str(&format!("# {c}\n"));
    }
    out.push_str(&format!(
        "script {}\n",
        serde_json::to_string(script).expect("script")
    ));
    for line in run.results_text.lines() {
        out.push_str(&format!("result {line}\n"));
    }
    let (_, final_text) = run
        .exports
        .iter()
        .find(|(n, _)| n == "final")
        .expect("final export");
    let ex = parse(final_text).expect("parse");
    out.push_str(&format!("export {} {}\n", ex.sha256, ex.msha256));
    std::fs::create_dir_all(opvectors_dir()).expect("create opvectors");
    std::fs::write(opvectors_dir().join(format!("{name}.opv")), out).expect("write vector");
    // The export with a header saying where it comes from (comments are
    // outside every digest).
    let mut lines = final_text.lines();
    let mut pe = format!("{}\n", lines.next().expect("magic"));
    pe.push_str(&format!(
        "# Final export of tests/data/opscripts/{name}.json run by tools/oracle/cmd/oprunner;\n"
    ));
    pe.push_str(&format!("# see {name}.opv beside this file.\n"));
    for l in lines {
        pe.push_str(l);
        pe.push('\n');
    }
    std::fs::write(opvectors_dir().join(format!("{name}.pe")), pe).expect("write export");
}

#[test]
fn frozen_vectors_are_well_formed() {
    let scripts = corpus();
    for name in FROZEN {
        let (v, export) = read_vector(name);
        assert!(
            v.comments.iter().any(|c| c.contains("oprunner")),
            "{name}: the header says how it was made"
        );
        let (_, current) = scripts
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is not in the corpus"));
        assert_eq!(
            &v.script, current,
            "{name}: the corpus script changed since it was frozen; regenerate"
        );
        let mut lines = v.results.lines();
        assert_eq!(lines.next(), Some(RESULTS_FORMAT));
        let n_ops = v.script["ops"].as_array().expect("ops").len();
        let mut count = 0;
        for (i, line) in lines.enumerate() {
            let rec: Value = serde_json::from_str(line).expect("record");
            assert_eq!(rec["i"], json!(i));
            check_record(name, &rec);
            count += 1;
        }
        assert_eq!(count, n_ops, "{name}: one result per operation");
        let ex = parse(&export).unwrap_or_else(|e| panic!("{name}.pe: {e}"));
        assert_eq!(ex.sha256, v.export_sha256, "{name}");
        assert_eq!(ex.msha256, v.export_msha256, "{name}");
        assert!(
            !ex.comments.is_empty(),
            "{name}.pe says where it comes from"
        );
    }
}

#[test]
fn frozen_vectors_match_go() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let regen = std::env::var("PURSE_REGEN_OPVECTORS").is_ok_and(|v| v == "1");
    let base = scratch("frozen");
    let scripts = corpus();
    for name in FROZEN {
        let (_, script) = scripts
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is not in the corpus"));
        let run = run_script(&mut o, script, &base.join(name));
        check_run(name, &run);
        if regen {
            write_vector(name, script, &run);
        }
        let (v, export) = read_vector(name);
        assert!(
            v.results == run.results_text,
            "{name}: Go's results changed:\n{}",
            first_difference(&v.results, &run.results_text)
        );
        let (_, final_text) = run
            .exports
            .iter()
            .find(|(n, _)| n == "final")
            .expect("final export");
        assert!(
            without_comments(&export) == without_comments(final_text),
            "{name}: Go's final export changed:\n{}",
            first_difference(&without_comments(&export), &without_comments(final_text))
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn script_faults_fail_the_run() {
    let Some(mut o) = oracle_or_skip("oprunner") else {
        return;
    };
    let base = scratch("faults");
    let (_, good) = corpus().into_iter().next().expect("a corpus script");
    let bad_ops = [
        json!({"op": "no.such_op"}),
        json!({"op": "mgr.new_account"}),
        json!({"op": "mgr.new_account", "name": "x", "nmae": "typo"}),
        json!({"op": "store.tx_details", "tx": "no-such-label"}),
        json!({"op": "view", "ops": [{"op": "mgr.new_account", "name": "x"}]}),
        json!({"op": "update", "ops": [{"op": "rt.checkpoint", "name": "x"}]}),
        json!({"op": "rt.checkpoint", "name": "final"}),
    ];
    let fail = "no such account";
    let bad_scripts = [
        // The wallet session's rules.
        json!([{"op": "wallet.new_external_address", "account": 0}]),
        json!([{"op": "wallet.close"}]),
        json!([{"op": "wallet.open"}, {"op": "mgr.lock"}]),
        json!([{"op": "wallet.open"}, {"op": "wallet.open"}]),
        json!([{"op": "wallet.open"}]),
        json!([{"op": "wallet.open"},
            {"op": "update", "ops": [{"op": "wallet.new_external_address", "account": 0}]}]),
        json!([{"op": "wallet.open"},
            {"op": "wallet.new_external_address", "account": 0, "policy": "sometimes"}]),
        // "tolerate" only on a group member, and only with real kinds.
        json!([{"op": "mgr.lookup_account", "name": fail, "tolerate": ["NotExist"]}]),
        json!([{"op": "update", "ops": [
            {"op": "mgr.lookup_account", "name": fail, "tolerate": ["Other"]}]}]),
        json!([{"op": "update", "ops": [
            {"op": "mgr.lookup_account", "name": fail, "tolerate": ["Panic"]}]}]),
        json!([{"op": "update", "ops": [
            {"op": "mgr.lookup_account", "name": fail, "tolerate": []}]}]),
        // A block label registered in a group that rolled back names
        // nothing afterwards.
        json!([{"op": "update", "ops": [
                {"op": "store.extend_main_chain", "label": "lost"},
                {"op": "mgr.lookup_account", "name": fail}]},
            {"op": "store.get_block_header", "block": "lost"}]),
        json!([{"op": "store.extend_main_chain", "again": "tip", "label": "x"}]),
    ];
    let singles = bad_ops.iter().map(|op| json!([op]));
    for (i, ops) in singles.chain(bad_scripts).enumerate() {
        let mut s = good.clone();
        s["ops"] = ops.clone();
        let dir = base.join(format!("f{i}"));
        let err = o
            .try_call("run", json!({"script": s, "dir": path_str(&dir)}))
            .expect_err("a script fault fails the run");
        assert!(!err.text.is_empty(), "{ops}");
    }
    let mut s = good.clone();
    s["format"] = json!("purse-opscript 2");
    let dir = base.join("format");
    assert!(
        o.try_call("run", json!({"script": s, "dir": path_str(&dir)}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------
// The script generator.
//
// It keeps just enough of a model to write scripts that are always valid
// as scripts (every label exists, every derived address names an existing
// account), while the operations themselves may fail in udb as they like:
// spends of rolled-back outputs, double spends, duplicate names, locked
// keys and the rest are recorded results, not faults.

/// An output a generated transaction may spend.
#[derive(Clone)]
struct Out {
    tx: String,
    index: u32,
    value: i64,
    tree: i8,
}

/// A wallet output of a built transaction, for its AddCredit.
#[derive(Clone)]
struct Credit {
    index: u32,
    account: u32,
    change: bool,
}

struct Pending {
    label: String,
    credits: Vec<Credit>,
}

struct Gen {
    rng: SplitMix64,
    id: usize,
    ops: Vec<Value>,
    unlocked: bool,
    accounts: u32,
    height: u32,
    blocks: Vec<String>,
    outs: Vec<Out>,
    unmined: Vec<Pending>,
    tickets: Vec<(String, i64)>,
    txs: Vec<String>,
    labels: u32,
    names: u32,
    checkpoints: u32,
    acct_pass: bool,
    /// The private passphrase Go's Manager holds in memory: a change
    /// rolled back with its group stays in memory until a reopen.
    mem_priv: &'static str,
}

impl Gen {
    fn new(rng: SplitMix64, id: usize) -> Gen {
        Gen {
            rng,
            id,
            ops: Vec::new(),
            unlocked: false,
            accounts: 1,
            height: 0,
            blocks: Vec::new(),
            outs: Vec::new(),
            unmined: Vec::new(),
            tickets: Vec::new(),
            txs: Vec::new(),
            labels: 0,
            names: 0,
            checkpoints: 0,
            acct_pass: false,
            mem_priv: "private",
        }
    }

    fn below(&mut self, n: u64) -> u64 {
        self.rng.below(n)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn label(&mut self, prefix: &str) -> String {
        self.labels += 1;
        format!("{prefix}{}", self.labels)
    }

    fn account(&mut self) -> u32 {
        self.below(u64::from(self.accounts)) as u32
    }

    fn derived(&mut self) -> (Value, u32, bool) {
        let acct = self.account();
        let branch = self.below(2) as u32;
        let index = self.below(40) as u32;
        (
            json!({"acct": acct, "branch": branch, "index": index}),
            acct,
            branch == 1,
        )
    }

    fn foreign_script(&mut self) -> String {
        format!("76a914{}88ac", hex(&self.rng.bytes(20)))
    }

    fn hash(&mut self) -> String {
        hex(&self.rng.bytes(32))
    }

    fn push(&mut self, op: Value) {
        self.ops.push(op);
    }

    fn script(mut self) -> Value {
        let clock = 1_700_000_000 + self.below(200_000_000) as i64;
        let setup = json!({
            "network": "simnet",
            "seed": hex(&self.rng.bytes(32)),
            "pubpass": "public",
            "privpass": "private",
            "rand": hex(&self.rng.bytes(32)),
            "clock": clock,
            "chain_time": clock - 1_000_000,
            "ncpu": 1 + self.below(4),
        });
        if self.chance(90) {
            self.push(json!({"op": "mgr.unlock", "pass": "private"}));
            self.unlocked = true;
        }
        let steps = 15 + self.below(45);
        for _ in 0..steps {
            self.step();
        }
        let id = self.id;
        json!({
            "format": SCRIPT_FORMAT,
            "name": format!("generated {id}"),
            "setup": setup,
            "ops": self.ops,
        })
    }

    fn step(&mut self) {
        match self.below(100) {
            0..=17 => self.mine(),
            18..=33 => self.spend(),
            34..=39 => self.address_op(),
            40..=43 => self.account_op(),
            44..=55 => self.query(),
            56..=61 => self.select(),
            62..=64 => self.rollback(),
            65..=66 => self.invalidate(),
            67..=69 => self.stake(),
            70..=73 => self.policy(),
            74..=77 => self.lock_op(),
            78 => self.passphrase(),
            79..=81 => {
                let s = 1 + self.below(100_000);
                self.push(json!({"op": "rt.clock_advance", "seconds": s}));
            }
            82 => {
                self.checkpoints += 1;
                let n = self.checkpoints;
                self.push(json!({"op": "rt.checkpoint", "name": format!("cp{n}")}));
            }
            83..=85 => self.unmined_op(),
            86..=87 => self.wallet_session(),
            _ => self.query(),
        }
    }

    /// A block: a coinbase to the wallet and, sometimes, one unmined
    /// transaction mined with it.
    fn mine(&mut self) {
        let cb = self.label("t");
        let (addr, acct, _) = self.derived();
        let value = 1 + self.below(5_000_000_000) as i64;
        self.push(json!({"op": "tx.build", "label": cb,
            "inputs": [{"prev": "coinbase", "value": value}],
            "outputs": [{"value": value, "pay": addr}]}));
        let mut filter = vec![json!(cb)];
        let mined = if !self.unmined.is_empty() && self.chance(50) {
            let i = self.below(self.unmined.len() as u64) as usize;
            let p = self.unmined.remove(i);
            filter.push(json!(p.label));
            Some(p)
        } else {
            None
        };
        let block = self.extend(1, filter);
        let mut group = Vec::new();
        if self.chance(20) {
            // connectBlock tolerates the Exist of a block it already
            // connected (wallet/chainntfns.go:46).
            group.push(
                json!({"op": "store.extend_main_chain", "again": "tip", "tolerate": ["Exist"]}),
            );
        }
        group.push(json!({"op": "store.insert_mined_tx", "tx": cb, "block": block}));
        group.push(
            json!({"op": "store.add_credit", "tx": cb, "block": block, "index": 0, "account": acct}),
        );
        if let Some(p) = mined {
            group.push(json!({"op": "store.insert_mined_tx", "tx": p.label, "block": block}));
            for c in &p.credits {
                group.push(
                    json!({"op": "store.add_credit", "tx": p.label, "block": block,
                    "index": c.index, "change": c.change, "account": c.account}),
                );
            }
        }
        self.push(json!({"op": "update", "ops": group}));
        self.outs.push(Out {
            tx: cb.clone(),
            index: 0,
            value,
            tree: 0,
        });
        self.txs.push(cb);
    }

    fn extend(&mut self, votebits: u32, filter: Vec<Value>) -> String {
        let label = self.label("b");
        self.push(json!({"op": "store.extend_main_chain", "label": label,
            "votebits": votebits, "filter": filter}));
        self.height += 1;
        self.blocks.truncate(self.height as usize - 1);
        self.blocks.push(label.clone());
        label
    }

    fn take_inputs(&mut self) -> Vec<Out> {
        let n = (1 + self.below(3)) as usize;
        let mut ins = Vec::new();
        for _ in 0..n {
            if self.outs.is_empty() {
                break;
            }
            let i = self.below(self.outs.len() as u64) as usize;
            ins.push(self.outs.swap_remove(i));
        }
        ins
    }

    /// A payment from wallet outputs, unmined or mined in a new block.
    fn spend(&mut self) {
        let ins = self.take_inputs();
        if ins.is_empty() {
            return self.mine();
        }
        let total: i64 = ins.iter().map(|o| o.value).sum();
        let label = self.label("t");
        let mut remaining = (total - 10_000).max(0);
        let n_out = 1 + self.below(3);
        let mut outputs = Vec::new();
        let mut credits = Vec::new();
        let mut spendable = Vec::new();
        for k in 0..n_out {
            let value = if k + 1 == n_out {
                remaining
            } else {
                let v = self.below(remaining as u64 + 1) as i64;
                remaining -= v;
                v
            };
            if self.chance(75) {
                let (addr, acct, change) = self.derived();
                outputs.push(json!({"value": value, "pay": addr}));
                credits.push(Credit {
                    index: k as u32,
                    account: acct,
                    change,
                });
                spendable.push(Out {
                    tx: label.clone(),
                    index: k as u32,
                    value,
                    tree: 0,
                });
            } else {
                let s = self.foreign_script();
                outputs.push(json!({"value": value, "script": s}));
            }
        }
        let inputs: Vec<Value> = ins
            .iter()
            .map(|o| json!({"prev": {"tx": o.tx, "index": o.index, "tree": o.tree}, "value": o.value}))
            .collect();
        let mut build =
            json!({"op": "tx.build", "label": label, "inputs": inputs, "outputs": outputs});
        if self.chance(30) {
            build["received"] = json!(1_700_000_000 + self.below(1_000_000) as i64);
        }
        if self.chance(15) {
            build["expiry"] = json!(self.height + self.below(4) as u32);
        }
        self.push(build);
        self.outs.extend(spendable);
        self.txs.push(label.clone());
        if self.chance(60) {
            let mut insert = json!({"op": "store.insert_mem_pool_tx", "tx": label});
            if self.chance(30) {
                // processTransactionRecord tolerates Exist (a transaction
                // already mined, wallet/chainntfns.go:450-455).
                insert["tolerate"] = json!(["Exist"]);
            }
            let mut group = vec![insert];
            for c in &credits {
                group.push(
                    json!({"op": "store.add_credit", "tx": label, "index": c.index,
                    "change": c.change, "account": c.account}),
                );
            }
            if self.chance(20) {
                group.push(json!({"op": "store.set_published", "tx": label, "published": false}));
            }
            self.push(json!({"op": "update", "ops": group}));
            self.unmined.push(Pending { label, credits });
        } else {
            let block = self.extend(1, vec![json!(label)]);
            let mut group =
                vec![json!({"op": "store.insert_mined_tx", "tx": label, "block": block})];
            for c in &credits {
                group.push(
                    json!({"op": "store.add_credit", "tx": label, "block": block,
                    "index": c.index, "change": c.change, "account": c.account}),
                );
            }
            self.push(json!({"op": "update", "ops": group}));
        }
    }

    fn address_op(&mut self) {
        let acct = self.account();
        let branch = self.below(2);
        let index = self.below(60);
        let op = match self.below(5) {
            0 => {
                json!({"op": "mgr.sync_account_to_addr_index", "account": acct, "index": index, "branch": branch})
            }
            1 => {
                json!({"op": "mgr.mark_used_child_index", "account": acct, "branch": branch, "child": index})
            }
            2 => {
                json!({"op": "mgr.mark_returned_child_index", "account": acct, "branch": branch, "child": index})
            }
            3 => {
                let (addr, _, _) = self.derived();
                json!({"op": "mgr.mark_used", "addr": addr})
            }
            _ => {
                json!({"op": "mgr.record_derived_address", "account": acct, "branch": branch, "child": index})
            }
        };
        self.push(op);
    }

    fn account_op(&mut self) {
        self.names += 1;
        let name = format!("acct {}", self.names);
        if self.unlocked && self.chance(60) {
            self.push(json!({"op": "mgr.new_account", "name": name}));
            self.accounts += 1;
        } else if self.chance(50) {
            let acct = self.account();
            self.push(json!({"op": "mgr.rename_account", "account": acct, "name": name}));
        } else {
            self.push(json!({"op": "mgr.lookup_account", "name": name}));
        }
    }

    fn some_tx(&mut self) -> Option<String> {
        if self.txs.is_empty() {
            None
        } else {
            let i = self.below(self.txs.len() as u64) as usize;
            Some(self.txs[i].clone())
        }
    }

    fn query(&mut self) {
        let h = i64::from(self.height);
        let op = match self.below(16) {
            0 => json!({"op": "store.account_balances", "minconf": self.below(4)}),
            1 => {
                let a = self.account();
                json!({"op": "store.account_balance", "minconf": self.below(3), "account": a})
            }
            2 => json!({"op": "store.unspent_outputs"}),
            3 => json!({"op": "store.for_each_unspent_outpoint"}),
            4 => json!({"op": "store.unspent_output_count"}),
            5 | 6 => match self.some_tx() {
                Some(t) => json!({"op": "store.tx_details", "tx": t}),
                None => json!({"op": "store.main_chain_tip"}),
            },
            7 => match self.some_tx() {
                Some(t) => json!({"op": "store.tx_block_height", "tx": t}),
                None => json!({"op": "store.unmined_tx_hashes"}),
            },
            8 => {
                let begin = self.below(h as u64 + 2) as i64 - 1;
                let end = self.below(h as u64 + 2) as i64 - 1;
                json!({"op": "store.range_transactions", "begin": begin, "end": end})
            }
            9 => json!({"op": "store.unmined_txs"}),
            10 => {
                let a = self.account();
                json!({"op": "mgr.account_properties", "account": a})
            }
            11 => {
                let a = self.account();
                json!({"op": "mgr.for_each_account_address", "account": a})
            }
            12 => json!({"op": "store.range_blocks", "begin": 0, "end": h}),
            13 => json!({"op": "store.unspent_tickets", "syncheight": h, "include_immature": true}),
            14 => match self.some_tx() {
                Some(t) => {
                    json!({"op": "store.unspent_output", "outpoint": {"tx": t, "index": 0}, "include_mempool": true})
                }
                None => json!({"op": "mgr.for_each_account"}),
            },
            _ => json!({"op": "view", "ops": [
                {"op": "store.main_chain_tip"},
                {"op": "store.account_balances", "minconf": 1},
                {"op": "mgr.last_account"}
            ]}),
        };
        self.push(op);
    }

    fn select(&mut self) {
        let h = self.height;
        if self.chance(25) {
            let minconf = 1 + self.below(3);
            self.push(json!({"op": "store.random_utxo", "minconf": minconf, "syncheight": h}));
            return;
        }
        let acct = self.account();
        let minconf = self.below(3);
        let mut targets = Vec::new();
        let mut t = 0_i64;
        for _ in 0..1 + self.below(3) {
            t += 1 + self.below(4_000_000_000) as i64;
            targets.push(t);
        }
        if self.chance(10) {
            targets = vec![0];
        }
        let mut op = json!({"op": "store.select_inputs", "account": acct, "minconf": minconf,
            "syncheight": h, "targets": targets});
        if !self.outs.is_empty() && self.chance(30) {
            let i = self.below(self.outs.len() as u64) as usize;
            let o = self.outs[i].clone();
            op["ignore"] = json!([{"tx": o.tx, "index": o.index, "tree": o.tree}]);
        }
        self.push(op);
    }

    fn rollback(&mut self) {
        if self.height == 0 {
            return self.mine();
        }
        let to = 1 + self.below(u64::from(self.height)) as u32;
        self.push(json!({"op": "store.rollback", "height": to}));
        self.height = to - 1;
        self.blocks.truncate(self.height as usize);
    }

    fn invalidate(&mut self) {
        self.extend(0, Vec::new());
    }

    /// A ticket from a wallet output, mined, with its commitment; or a
    /// vote on an earlier ticket.
    fn stake(&mut self) {
        if !self.tickets.is_empty() && self.chance(50) {
            let (ticket, price) = self.tickets.swap_remove(0);
            let label = self.label("t");
            let (addr, acct, _) = self.derived();
            self.push(json!({"op": "tx.build", "label": label, "inputs": [
                    {"prev": "stakebase", "value": 100_000_000},
                    {"prev": {"tx": ticket, "index": 0, "tree": 1}, "value": price}],
                "outputs": [
                    {"value": 0, "voteblock": "tip"},
                    {"value": 0, "votebits": "0100"},
                    {"value": price + 100_000_000, "pay": addr, "tag": "ssgen"}]}));
            let block = self.extend(1, vec![json!(label)]);
            self.push(json!({"op": "update", "ops": [
                {"op": "store.insert_mined_tx", "tx": label, "block": block},
                {"op": "store.add_credit", "tx": label, "block": block, "index": 2, "account": acct},
                {"op": "store.redeem_ticket_commitments", "tx": label, "block": block}]}));
            self.txs.push(label);
            return;
        }
        let Some(input) = self.take_inputs().into_iter().next() else {
            return self.mine();
        };
        let price = (input.value - 10_000).max(0);
        let label = self.label("t");
        let (vote_addr, acct, _) = self.derived();
        let (commit_addr, commit_acct, _) = self.derived();
        let change = self.foreign_script().replacen("76", "bd76", 1);
        self.push(json!({"op": "tx.build", "label": label,
            "inputs": [{"prev": {"tx": input.tx, "index": input.index, "tree": input.tree}, "value": input.value}],
            "outputs": [
                {"value": price, "pay": vote_addr, "tag": "sstx"},
                {"value": 0, "commitment": {"pay": commit_addr, "amount": input.value}},
                {"value": 0, "script": change}]}));
        let block = self.extend(1, vec![json!(label)]);
        self.push(json!({"op": "update", "ops": [
            {"op": "store.insert_mined_tx", "tx": label, "block": block},
            {"op": "store.add_credit", "tx": label, "block": block, "index": 0, "account": acct},
            {"op": "store.add_ticket_commitment", "tx": label, "index": 1, "account": commit_acct}]}));
        if self.chance(40) {
            self.push(json!({"op": "store.ticket_details", "tx": label}));
        }
        self.tickets.push((label.clone(), price));
        self.txs.push(label);
    }

    fn policy(&mut self) {
        let ticket = match self.some_tx() {
            Some(t) if self.chance(50) => json!(t),
            _ => json!(self.hash()),
        };
        let tspend = self.hash();
        let policy = self.below(3);
        let pikey = hex(&self.rng.bytes(33));
        let op = match self.below(10) {
            0 => json!({"op": "udb.set_tspend_policy", "tspend": tspend, "policy": policy}),
            1 => {
                json!({"op": "udb.set_vsp_tspend_policy", "ticket": ticket, "tspend": tspend, "policy": policy})
            }
            2 => json!({"op": "udb.set_treasury_key_policy", "pikey": pikey, "policy": policy}),
            3 => {
                json!({"op": "udb.set_vsp_treasury_key_policy", "ticket": ticket, "pikey": pikey, "policy": policy})
            }
            4 => {
                json!({"op": "udb.set_default_agenda_preference", "version": self.below(12),
                    "agenda": format!("agenda{}", self.below(4)), "choice": "yes"})
            }
            5 => {
                json!({"op": "udb.set_ticket_agenda_preference", "ticket": ticket, "version": self.below(12),
                    "agenda": format!("agenda{}", self.below(4)), "choice": "no"})
            }
            6 => {
                let fee = self.hash();
                json!({"op": "udb.set_vsp_ticket", "ticket": ticket, "fee_hash": fee,
                    "fee_tx_status": self.below(5), "host": format!("https://vsp{}.example.org", self.below(3)),
                    "pubkey": hex(&self.rng.bytes(4))})
            }
            7 => json!({"op": "udb.get_vsp_tickets_by_fee_status", "status": self.below(5)}),
            8 => json!({"op": "view", "ops": [
                {"op": "udb.tspend_policies"}, {"op": "udb.vsp_tspend_policies"},
                {"op": "udb.treasury_key_policies"}, {"op": "udb.vsp_treasury_key_policies"}]}),
            _ => json!({"op": "udb.get_vsp_ticket", "ticket": ticket}),
        };
        self.push(op);
    }

    fn lock_op(&mut self) {
        match self.below(4) {
            0 => {
                self.push(json!({"op": "mgr.lock"}));
                self.unlocked = false;
            }
            1 => {
                self.push(json!({"op": "mgr.unlock", "pass": "private"}));
                self.unlocked = self.mem_priv == "private";
            }
            2 => self.push(json!({"op": "mgr.unlock", "pass": "wrong"})),
            _ => {
                self.push(json!({"op": "rt.reopen"}));
                self.unlocked = false;
                self.mem_priv = "private";
            }
        }
        // A failed unlock locks the wallet (Manager.Unlock locks first).
        if self.ops.last().is_some_and(|o| o["pass"] == "wrong") {
            self.unlocked = false;
        }
    }

    /// A passphrase change, alone or in a group that fails after it (the
    /// change rolls back in the database, so the runner's bookkeeping must
    /// too, while Go's Manager keeps it in memory). At most one per-account
    /// passphrase per script, and only in some scripts: each one costs
    /// Argon2id derivations of 256 MiB.
    fn passphrase(&mut self) {
        let failing = json!({"op": "mgr.lookup_account", "name": "no such account"});
        if self.chance(50) {
            let private = self.chance(50);
            let (old, new) = if private {
                (self.mem_priv, "other")
            } else {
                ("public", "public2")
            };
            self.push(json!({"op": "update", "ops": [
                {"op": "mgr.change_passphrase", "old": old, "new": new, "private": private},
                failing]}));
            if private {
                self.mem_priv = "other";
            }
            return;
        }
        if self.acct_pass || !self.unlocked || self.accounts < 2 || !self.chance(30) {
            return self.query();
        }
        self.acct_pass = true;
        let acct = 1 + self.below(u64::from(self.accounts - 1));
        let set = json!({"op": "mgr.set_account_passphrase", "account": acct, "pass": "acct-pass"});
        if self.chance(50) {
            self.push(json!({"op": "update", "ops": [set, failing]}));
            return;
        }
        self.push(set);
        self.push(json!({"op": "mgr.lock_account", "account": acct}));
        self.push(json!({"op": "mgr.unlock_account", "account": acct, "pass": "acct-pass"}));
    }

    /// A wallet-package session issuing addresses under a small gap limit
    /// and every policy. The copied udb's Manager reopens locked after it.
    fn wallet_session(&mut self) {
        let gap = self.below(5);
        self.push(json!({"op": "wallet.open", "gap_limit": gap}));
        for _ in 0..1 + self.below(8) {
            let acct = if self.chance(90) {
                self.account()
            } else {
                self.accounts
            };
            let op = if self.chance(70) {
                "wallet.new_external_address"
            } else {
                "wallet.new_internal_address"
            };
            let mut call = json!({"op": op, "account": acct});
            match self.below(4) {
                0 => {}
                1 => call["policy"] = json!("error"),
                2 => call["policy"] = json!("ignore"),
                _ => call["policy"] = json!("wrap"),
            }
            self.push(call);
            if self.chance(20) {
                self.push(json!({"op": "wallet.branch_next_indexes", "account": acct}));
            }
        }
        self.push(json!({"op": "wallet.close"}));
        self.unlocked = false;
    }

    fn unmined_op(&mut self) {
        match self.below(3) {
            0 => self.push(json!({"op": "store.prune_unmined", "sdiff": 0})),
            1 if !self.unmined.is_empty() => {
                let i = self.below(self.unmined.len() as u64) as usize;
                let p = self.unmined.remove(i);
                self.push(json!({"op": "store.remove_unconfirmed", "tx": p.label}));
            }
            _ => self.push(json!({"op": "store.unmined_tx_hashes"})),
        }
    }
}
