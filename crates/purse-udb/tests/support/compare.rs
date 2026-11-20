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

/// The lines of `section` under the masked observable profile, keeping
/// only `types` when given.
fn masked_lines(ex: &Export, section: &str, types: Option<&[&str]>) -> Vec<String> {
    let s = ex.section(section).expect("section present");
    s.records
        .iter()
        .filter(|r| types.is_none_or(|t| t.contains(&s.types[r.ty].name.as_str())))
        .map(|r| render_with(&s.types[r.ty], &r.values, Profile::MASKED.observable()))
        .collect()
}

/// Checks the two exports equal over what the Rust producer covers.
pub fn compare_exports(what: &str, go_pe: &str, rust_pe: &str) {
    let go = parse(go_pe).unwrap_or_else(|e| panic!("{what}: Go export: {e}"));
    let rust = parse(rust_pe).unwrap_or_else(|e| panic!("{what}: Rust export: {e}\n{rust_pe}"));
    go.check_sealed_secrets()
        .unwrap_or_else(|e| panic!("{what}: Go export: {e}"));
    rust.check_sealed_secrets()
        .unwrap_or_else(|e| panic!("{what}: Rust export: {e}"));
    for meta in ["network", "chain", "decrypt", "derived", "privcheck"] {
        assert_eq!(go.meta(meta), rust.meta(meta), "{what}: @{meta}");
    }
    for name in COVERED {
        let types: Option<&[&str]> = (name == "derived").then_some(&COVERED_DERIVED[..]);
        let g = masked_lines(&go, name, types);
        let r = masked_lines(&rust, name, types);
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
        assert_eq!(comparable(g), comparable(r), "{what}: op {}", g["i"]);
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
