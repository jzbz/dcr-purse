// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The comparison of a Go run and a Rust run of one operation script
//! (ADR-0002 section 6): records and exports.

use std::path::Path;

use purse_export_check::{Export, Profile, parse, render_with};
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
    assert_eq!(comparable(g), comparable(r), "{what}: op {}", g["i"]);
}

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

/// [`compare_exports`] on parsed exports, for a test that first edits Go's
/// to state a recorded divergence.
pub fn compare_parsed(what: &str, go: &Export, rust: &Export) {
    check_privcheck(go).unwrap_or_else(|e| panic!("{what}: Go export: {e}"));
    check_privcheck(rust).unwrap_or_else(|e| panic!("{what}: Rust export: {e}"));
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
pub fn differential(o: &mut Oracle, what: &str, script: &Value) -> Vec<Value> {
    let dir = scratch("diff");
    let (go_results, go_pe) = go_run(o, script, &dir.join("go"));
    let rust = runner::run(script, &dir.join("rust"))
        .unwrap_or_else(|e| panic!("{what}: Rust runner: {e}"));
    assert_eq!(go_results.len(), rust.results.len(), "{what}: record count");
    for (g, r) in go_results.iter().zip(&rust.results) {
        check_record(what, g, r);
    }
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
