// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The comparison of a Go run and a Rust run of one operation script
//! (ADR-0002 section 6): records and exports.

use std::path::Path;

use purse_export_check::{
    Export, Profile, Record, Section, decode_hex, encode_str, parse, render_with,
};
use purse_testutil::Oracle;
use purse_testutil::serde_json::{self, Value, json};
use purse_udb::export::{COVERED, COVERED_DERIVED};

use super::data_dir;
use super::runner;
use super::scratch;

/// Runs `script` through Go; returns the results and the final export.
pub fn go_run(o: &mut Oracle, script: &Value, dir: &Path) -> (Vec<Value>, String) {
    let res = o
        .try_call(
            "run",
            json!({"script": script, "dir": dir.to_str().expect("utf-8 path")}),
        )
        .unwrap_or_else(|e| panic!("oprunner failed: {} ({:?})", e.text, e.kind));
    let results = res["results"].as_array().expect("results").clone();
    let pe = std::fs::read_to_string(dir.join("final.pe")).expect("final.pe");
    (results, pe)
}

/// The exports of the `rt.checkpoint` records among Go's `results` of a
/// run in `dir`, in script order: each checkpoint's name and the text of
/// the `<name>.pe` Go's runner wrote there.
pub fn go_checkpoints(results: &[Value], dir: &Path) -> Vec<(String, String)> {
    results
        .iter()
        .filter(|r| r["op"] == "rt.checkpoint")
        .map(|r| {
            let file = r["ok"]["file"].as_str().expect("a checkpoint's file");
            let name = file.strip_suffix(".pe").expect("a .pe file").to_owned();
            let text = std::fs::read_to_string(dir.join(file))
                .unwrap_or_else(|e| panic!("checkpoint {file}: {e}"));
            (name, text)
        })
        .collect()
}

/// Checks the checkpoint exports of a Go run and a Rust run: the same
/// checkpoints in the same order, each pair equal as [`compare_exports`]
/// compares the final ones.
pub fn compare_checkpoints(what: &str, go: &[(String, String)], rust: &[(String, String)]) {
    let names = |v: &[(String, String)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(go), names(rust), "{what}: checkpoints");
    for ((name, g), (_, r)) in go.iter().zip(rust) {
        compare_exports(&format!("{what}: checkpoint {name}"), g, r);
    }
}

/// A record without the members that follow each side's random stream:
/// `drawn` is removed, and every value under a member whose name ends in
/// `~r` or `~cr` (ADR-0002 section 4, volatility markers) is replaced by
/// its shape, the length of a string or `null`, so a sealed value still
/// has to be as long as Go's.
pub fn comparable(rec: &Value) -> Value {
    fn mask(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if k.ends_with("~r") || k.ends_with("~cr") {
                        *x = match x {
                            Value::String(s) => json!(format!("<masked {} chars>", s.len())),
                            _ => Value::Null,
                        };
                    } else {
                        mask(x);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(mask),
            _ => {}
        }
    }
    let mut r = rec.clone();
    if let Some(m) = r.as_object_mut() {
        m.remove("drawn");
    }
    mask(&mut r);
    r
}

/// Checks one Rust record against Go's: equal under [`comparable`], except
/// that a Go panic (kind `Panic`, ADR-0002 section 3) is matched by any
/// error, as dcr-purse returns an error where Go panics (ADR-0012 Rule 2;
/// each case is a PARITY.md line).
pub fn check_record(what: &str, g: &Value, r: &Value) {
    // A group: its members one by one (a member's panic ends the group with
    // it), then the group's own record without them.
    if let (Some(gm), Some(rm)) = (g["ops"].as_array(), r["ops"].as_array()) {
        assert_eq!(gm.len(), rm.len(), "{what}: op {}: group members", g["i"]);
        for (x, y) in gm.iter().zip(rm) {
            check_record(&format!("{what}: op {}", g["i"]), x, y);
        }
        let strip = |v: &Value| {
            let mut v = v.clone();
            if let Some(m) = v.as_object_mut() {
                m.remove("ops");
            }
            v
        };
        return check_record(what, &strip(g), &strip(r));
    }
    if g["op"] == "rt.checkpoint" && g.get("ok").is_some() {
        // An ExportSummary: each side's digests and counts are of its own
        // export, which holds what the other's producer does not write (the
        // exporter, the sections no profile compares, aux sections not yet
        // covered), so the record is compared by its file and its sections'
        // names, and the exports themselves by `compare_checkpoints`.
        assert_eq!(g["i"], r["i"], "{what}: record order");
        assert_eq!(g["op"], r["op"], "{what}: op {}", g["i"]);
        let names = |v: &Value| {
            v["ok"]["sections"]
                .as_array()
                .map(|s| s.iter().map(|x| x["name"].clone()).collect::<Vec<_>>())
        };
        assert_eq!(g["ok"]["file"], r["ok"]["file"], "{what}: op {}", g["i"]);
        assert_eq!(names(g), names(r), "{what}: op {}: sections", g["i"]);
        return;
    }
    if g["err"]["kind"] == "Panic" {
        assert_eq!(g["i"], r["i"], "{what}: record order");
        assert_eq!(g["op"], r["op"], "{what}: op {}", g["i"]);
        assert!(
            r.get("err").is_some(),
            "{what}: op {}: Go panics ({}), Rust returned {}",
            g["i"],
            g["err"]["text"],
            r
        );
        return;
    }
    if RANDOM_OPS.contains(&g["op"].as_str().unwrap_or_default()) {
        // The store's random selections draw what Go draws (the runner keys
        // dcrd's generator by the operation's `rand`, or continues the last
        // one's stream): their `~r` values and the bytes drawn are compared
        // as they are.
        assert_eq!(g, r, "{what}: op {} (exact)", g["i"]);
        return;
    }
    assert_eq!(comparable(g), comparable(r), "{what}: op {}", g["i"]);
}

/// The transaction store's operations that draw from the random stream,
/// whose records the Rust runner makes equal to Go's, `~r` values and
/// `drawn` included (`runner.rs`).
pub const RANDOM_OPS: &[&str] = &["store.random_utxo", "store.select_inputs"];

/// The lines of `section` under the masked observable profile, keeping
/// only `types` when given.
///
/// In `[credits]` and `[debits]` the invalidated records of a detached
/// block (one whose header is a `sideheader`) are left out: dcrwallet's
/// `Rollback` leaves them in `ic` and `id` (QK-0081), dcr-purse deletes
/// them (ADR-0003 Decision item 5; PARITY.md, `purse-udb`).
fn masked_lines(ex: &Export, section: &str, types: Option<&[&str]>) -> Vec<String> {
    let s = ex.section(section).expect("section present");
    let detached: std::collections::BTreeSet<&str> = if matches!(section, "credits" | "debits") {
        let h = ex.section("headers").expect("[headers] present");
        h.records_of("sideheader")
            .filter_map(|r| h.value(r, "hash"))
            .collect()
    } else {
        std::collections::BTreeSet::new()
    };
    s.records
        .iter()
        .filter(|r| types.is_none_or(|t| t.contains(&s.types[r.ty].name.as_str())))
        .filter(|r| {
            detached.is_empty()
                || s.value(r, "inv") != Some("1")
                || s.value(r, "block").is_none_or(|b| !detached.contains(b))
        })
        .map(|r| render_with(&s.types[r.ty], &r.values, Profile::MASKED.observable()))
        .collect()
}

/// The `[privcheck]` an export must carry: on a wallet that is not
/// watching-only, every verdict `yes` (`check_sealed_secrets`), which is
/// what makes the masked comparison a proof that the sealed secrets are
/// the same keys. A watching-only wallet can never be unlocked, so no
/// sealed private key is opened: one created from an extended key carries
/// no `[privcheck]` at all (no private passphrase exists, `@privcheck=0`),
/// and one converted from a seed's wallet has the master key's verdict
/// `error` (`Unlock` refuses) and every other verdict `nopass`. There the
/// masked comparison proves the public keys equal through the decrypted
/// `xpub` fields, and the private keys are checked by the property tests.
pub fn check_privcheck(ex: &Export) -> Result<(), String> {
    let wallet = ex.section("wallet").ok_or("no [wallet]")?;
    let watching = wallet
        .records
        .first()
        .is_some_and(|r| wallet.value(r, "watchonly") == Some("1"));
    if !watching {
        return ex.check_sealed_secrets();
    }
    let pc = ex.section("privcheck").ok_or("no [privcheck]")?;
    if ex.meta("privcheck") != Some("1") {
        return if pc.records.is_empty() {
            Ok(())
        } else {
            Err("verdicts without a private passphrase".to_owned())
        };
    }
    for r in &pc.records {
        let ty = pc.types[r.ty].name.as_str();
        let want = if ty == "mkeyok" { "error" } else { "nopass" };
        if pc.value(r, "ok") != Some(want) {
            return Err(format!("watching-only wallet: {ty} verdict not {want}"));
        }
    }
    if pc.records_of("mkeyok").count() == 1 {
        Ok(())
    } else {
        Err("watching-only wallet: no master key verdict".to_owned())
    }
}

/// Checks the two exports equal over what the Rust producer covers.
pub fn compare_exports(what: &str, go_pe: &str, rust_pe: &str) {
    let go = parse(go_pe).unwrap_or_else(|e| panic!("{what}: Go export: {e}"));
    let rust = parse(rust_pe).unwrap_or_else(|e| panic!("{what}: Rust export: {e}\n{rust_pe}"));
    compare_parsed(what, &go, &rust);
}

/// The dead items of ADR-0001 section 9 that a version-26 database made at
/// the pin can hold in Go's `[residue]`: `waddrmgr/main/mgrver`, written at
/// creation and never deleted. The section's other dead items reach a
/// version-26 database only from a migrated or older one, which no script
/// makes, so meeting one fails the comparison.
pub const GO_RESIDUE: &[&str] = &["waddrmgr/main/mgrver"];

/// Whether `ex`'s `[addresses]` holds a row matching `pick` whose
/// identity and address are absent (`-`), which Go's export writes for a
/// row the address API did not return.
fn unreached_row(ex: &Export, pick: impl Fn(&Section, &Record) -> bool) -> bool {
    ex.section("addresses").is_some_and(|s| {
        s.records_of("address")
            .any(|r| pick(s, r) && s.value(r, "id") == Some("-") && s.value(r, "addr") == Some("-"))
    })
}

/// Checks the sections no comparison compares (ADR-0001 section 11,
/// addendum of 2026-11-26): the Rust export's `[bad]`, `[unknown]` and
/// `[residue]` are empty; Go's `[unknown]` is empty; Go's `[residue]`
/// holds only [`GO_RESIDUE`]; and Go's `[bad]` holds only records the
/// address API left (`value` absent): `unreached` on `waddrmgr/addr` for a
/// key whose `[addresses]` row is written with `-` identity and address in
/// both exports, and `api` on `waddrmgr/addracctidx` for an account (the
/// key, LE32) one of whose `[addresses]` rows is written so in both.
/// `[orphans]` is compared with the covered sections.
pub fn check_uncompared(go: &Export, rust: &Export) -> Result<(), String> {
    check_rust_uncompared(rust)?;
    let unknown = go.section("unknown").ok_or("Go export: no [unknown]")?;
    if !unknown.records.is_empty() {
        return Err(format!(
            "Go export: [unknown] is not empty: {:?}",
            unknown.lines(false)
        ));
    }
    let residue = go.section("residue").ok_or("Go export: no [residue]")?;
    for r in &residue.records {
        let path = residue.value(r, "path").unwrap_or("");
        if !GO_RESIDUE.iter().any(|p| encode_str(p.as_bytes()) == path) {
            return Err(format!(
                "Go export: [residue] holds {path}, not a dead item a database made at the pin holds"
            ));
        }
    }
    let bad = go.section("bad").ok_or("Go export: no [bad]")?;
    for (r, line) in bad.records.iter().zip(bad.lines(false)) {
        let path = bad.value(r, "path").unwrap_or("");
        let key = bad.value(r, "key").unwrap_or("");
        let reason = bad.value(r, "reason").unwrap_or("");
        let tied = bad.value(r, "value") == Some("-")
            && if path == encode_str(b"waddrmgr/addr") && reason == "unreached" {
                let row = |s: &Section, a: &Record| s.value(a, "key") == Some(key);
                unreached_row(go, row) && unreached_row(rust, row)
            } else if path == encode_str(b"waddrmgr/addracctidx") && reason == "api" {
                match decode_hex(key).and_then(|k| <[u8; 4]>::try_from(k).ok()) {
                    Some(k) => {
                        let account = u32::from_le_bytes(k).to_string();
                        let row = |s: &Section, a: &Record| {
                            s.value(a, "account") == Some(account.as_str())
                        };
                        unreached_row(go, row) && unreached_row(rust, row)
                    }
                    None => false,
                }
            } else {
                false
            };
        if !tied {
            return Err(format!(
                "Go export: [bad] holds {line}, not a record of an address row both exports \
                 write with `-` fields"
            ));
        }
    }
    Ok(())
}

/// The Rust half of [`check_uncompared`], for a replay that holds only a
/// digest of Go's export: `[bad]`, `[unknown]` and `[residue]` empty.
pub fn check_rust_uncompared(rust: &Export) -> Result<(), String> {
    for name in ["bad", "unknown", "residue"] {
        let s = rust
            .section(name)
            .ok_or_else(|| format!("Rust export: no [{name}]"))?;
        if !s.records.is_empty() {
            return Err(format!(
                "Rust export: [{name}] is not empty: {:?}",
                s.lines(false)
            ));
        }
    }
    Ok(())
}

/// [`compare_exports`] on parsed exports, for a test that first edits Go's
/// to state a recorded divergence.
pub fn compare_parsed(what: &str, go: &Export, rust: &Export) {
    check_privcheck(go).unwrap_or_else(|e| panic!("{what}: Go export: {e}"));
    check_privcheck(rust).unwrap_or_else(|e| panic!("{what}: Rust export: {e}"));
    check_uncompared(go, rust).unwrap_or_else(|e| panic!("{what}: {e}"));
    for meta in ["network", "chain", "decrypt", "derived", "privcheck"] {
        assert_eq!(go.meta(meta), rust.meta(meta), "{what}: @{meta}");
    }
    for name in COVERED {
        let types: Option<&[&str]> = (name == "derived").then_some(&COVERED_DERIVED[..]);
        let g = masked_lines(go, name, types);
        let r = masked_lines(rust, name, types);
        assert_eq!(g, r, "{what}: [{name}] differs (masked observable profile)");
    }
}

/// The SHA-256 (hex) of what [`compare_exports`] compares: the metas it
/// checks, then each covered section's name and its lines under the
/// masked observable profile. Equal digests are equal comparisons.
pub fn covered_digest(pe: &str) -> String {
    use sha2::{Digest, Sha256};
    let ex = parse(pe).unwrap_or_else(|e| panic!("export: {e}"));
    let mut h = Sha256::new();
    for meta in ["network", "chain", "decrypt", "derived", "privcheck"] {
        h.update(format!("@{meta} {:?}\n", ex.meta(meta)).as_bytes());
    }
    for name in COVERED {
        let types: Option<&[&str]> = (name == "derived").then_some(&COVERED_DERIVED[..]);
        h.update(format!("[{name}]\n").as_bytes());
        for l in masked_lines(&ex, name, types) {
            h.update(l.as_bytes());
            h.update(b"\n");
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs one script on both sides and compares; returns Go's records.
/// Every checkpoint export is compared, then the final one.
pub fn differential(o: &mut Oracle, what: &str, script: &Value) -> Vec<Value> {
    let dir = scratch("diff");
    let (go_results, go_pe) = go_run(o, script, &dir.join("go"));
    let rust = runner::run(script, &dir.join("rust"))
        .unwrap_or_else(|e| panic!("{what}: Rust runner: {e}"));
    assert_eq!(go_results.len(), rust.results.len(), "{what}: record count");
    for (g, r) in go_results.iter().zip(&rust.results) {
        check_record(what, g, r);
    }
    let go_cps = go_checkpoints(&go_results, &dir.join("go"));
    compare_checkpoints(what, &go_cps, &rust.checkpoints);
    compare_exports(what, &go_pe, &rust.export);
    let _ = std::fs::remove_dir_all(&dir);
    go_results
}

/// The hand-written scripts, by file name.
pub fn corpus() -> Vec<(String, Value)> {
    let dir = data_dir().join("opscripts");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("opscripts")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("read script");
            let v: Value = serde_json::from_str(&text).expect("script JSON");
            (
                p.file_stem().expect("stem").to_string_lossy().into_owned(),
                v,
            )
        })
        .collect()
}
