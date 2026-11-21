// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The driver's smoke mode against the Go side, in every test run: both
//! sides run every workload once at the reduced sizes, alternating, and
//! every output they cross-check must agree. Its times mean nothing (this
//! is an unoptimised build); it holds the two sides to the same workloads
//! and keeps the driver working.

use std::path::PathBuf;
use std::process::Command;

use purse_bench::report::{self, Side};
use purse_testutil::{is_reference_go, oracle_bin_or_skip};

fn out_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("purse-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn smoke_session_agrees_with_go() {
    let Some(go) = oracle_bin_or_skip("bench") else {
        return;
    };
    let out = out_dir("smoke");
    let status = Command::new(env!("CARGO_BIN_EXE_purse-bench"))
        .args(["drive", "--smoke", "--runs", "2", "--go-bin"])
        .arg(&go)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("run purse-bench");
    assert!(status.success(), "purse-bench drive --smoke failed");

    // Every workload ran on the Rust side, and on the Go side all but the
    // solver (the oracle's default build has no FLINT).
    let go_run = std::fs::read_to_string(out.join("run-1-go.txt")).expect("run-1-go.txt");
    let rust_run = std::fs::read_to_string(out.join("run-1-rust.txt")).expect("run-1-rust.txt");
    let go_out = report::parse(&go_run, Side::Go).expect("parse go");
    let rust_out = report::parse(&rust_run, Side::Rust).expect("parse rust");
    assert!(rust_out.skips.is_empty(), "{:?}", rust_out.skips);
    let rust_names: Vec<&str> = rust_out.results.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(rust_names.len(), 25, "{rust_names:?}");
    for (name, _) in &go_out.results {
        assert!(rust_names.contains(&name.as_str()), "{name}");
    }
    for name in &rust_names {
        let ran = go_out.results.iter().any(|(n, _)| n == name);
        assert!(
            ran || (name.starts_with("mix.") && go_out.skips.contains_key(*name)),
            "{name} neither ran nor was skipped on the Go side"
        );
    }
    assert!(rust_out.checks.len() >= 20, "{:?}", rust_out.checks);
    assert_eq!(rust_out.checks, go_out.checks);

    // The base58 workloads' string sets: the same strings on both sides,
    // and the same answers from each side's decoder (the sets' digests and
    // the digests of what each side decodes them to), the addresses at
    // their full size, the extended keys at smoke mode's.
    let set_checks = [
        (
            "crypto.base58check_set_count",
            Some(purse_bench::workloads::ADDRESS_SET_SIZE),
        ),
        ("crypto.base58check_set_sha256", None),
        ("crypto.base58check_set_decoded_sha256", None),
        (
            "hd.xpub_set_count",
            Some(purse_bench::workloads::xpub_set_size(true)),
        ),
        ("hd.xpub_set_sha256", None),
        ("hd.xpub_set_parsed_sha256", None),
    ];
    for (name, count) in set_checks {
        let rust = rust_out.checks.get(name);
        assert!(rust.is_some(), "no CHECK {name} on the Rust side");
        assert_eq!(rust, go_out.checks.get(name), "{name}");
        if let Some(count) = count {
            assert_eq!(rust, Some(&count.to_string()), "{name}");
        }
    }

    if std::env::var_os("PURSE_REQUIRE_ORACLE").is_some() {
        let version = go_out
            .meta
            .iter()
            .find(|(k, _)| k == "version")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert!(
            is_reference_go(version),
            "the Go side was built with {version:?}"
        );
    }

    let table = std::fs::read_to_string(out.join("table.md")).expect("table.md");
    assert!(table.contains("| hd.addr20k |"), "{table}");
    let _ = std::fs::remove_dir_all(&out);
}

/// The driver builds the Go side itself (as the CI gate does) into an
/// output directory given relative to the current directory. `go build`
/// runs in `tools/oracle`, and a relative `-o` once put the binary there,
/// leaving the runs unable to find it.
#[test]
fn drive_builds_the_go_side_into_a_relative_out_dir() {
    // Skips without Go, as the other oracle tests do.
    if oracle_bin_or_skip("bench").is_none() {
        return;
    }
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let rel = format!("purse-bench-relative-{}", std::process::id());
    let _ = std::fs::remove_dir_all(base.join(&rel));
    let output = Command::new(env!("CARGO_BIN_EXE_purse-bench"))
        .current_dir(&base)
        .args(["drive", "--smoke", "--runs", "1", "--only", "hd.master"])
        .arg("--out")
        .arg(&rel)
        .output()
        .expect("run purse-bench");
    let stray = purse_testutil::oracle_source_dir().join(&rel);
    let stray_found = stray.exists();
    let _ = std::fs::remove_dir_all(&stray);
    assert!(
        output.status.success(),
        "purse-bench drive with a relative --out failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!stray_found, "the Go side was built under tools/oracle");
    let go_bin = base
        .join(&rel)
        .join(format!("purse-bench-go{}", std::env::consts::EXE_SUFFIX));
    assert!(go_bin.is_file(), "no Go binary in the output directory");
    assert!(base.join(&rel).join("run-1-go.txt").is_file());
    let _ = std::fs::remove_dir_all(base.join(&rel));
}

#[test]
fn gate_baseline_names_measured_workloads() {
    let baseline = report::parse_baseline(include_str!("../gate-baseline.tsv")).expect("baseline");
    assert!(!baseline.is_empty());
    // Every baselined workload is one the harness runs (mix.* is not
    // baselined: CI's Go side has no FLINT).
    let mut out = Vec::new();
    let mut h = purse_bench::harness::Harness::new(
        purse_bench::harness::Config {
            smoke: true,
            only: baseline.iter().map(|(n, _)| n.clone()).collect(),
            ..Default::default()
        },
        &mut out,
    );
    purse_bench::workloads::run_all(&mut h);
    let _ = h.finish();
    let text = String::from_utf8(out).expect("utf-8");
    let run = report::parse(&text, Side::Rust).expect("parse");
    for (name, _) in &baseline {
        assert!(
            !name.starts_with("mix."),
            "{name}: the CI gate's Go side has no FLINT"
        );
        assert!(
            run.results.iter().any(|(n, _)| n == name),
            "{name} is in the baseline but not a workload"
        );
    }
}

/// The `bench-gate` job's selection (`--only` in `.github/workflows/ci.yml`)
/// reaches every baselined workload, and the "CI." paragraph of
/// `docs/PHASE1-EXIT.md` gives the number of workloads that job runs and
/// the number it gates. A workload added under a selected prefix without a
/// baseline (as `crypto.base58check_decode_one` was) changes the first
/// number and not the second.
#[test]
fn ci_gate_selection_matches_its_record() {
    let root = purse_testutil::skew::workspace_root();
    let ci =
        purse_testutil::skew::read_text(&root.join(".github").join("workflows").join("ci.yml"));
    let lists: Vec<&str> = ci
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--only "))
        .collect();
    assert_eq!(lists.len(), 1, "ci.yml's --only lists: {lists:?}");
    let only: Vec<String> = lists[0]
        .trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    let mut out = Vec::new();
    let mut h = purse_bench::harness::Harness::new(
        purse_bench::harness::Config {
            smoke: true,
            only,
            ..Default::default()
        },
        &mut out,
    );
    purse_bench::workloads::run_all(&mut h);
    let _ = h.finish();
    let text = String::from_utf8(out).expect("utf-8");
    let run = report::parse(&text, Side::Rust).expect("parse");
    let selected: Vec<&str> = run.results.iter().map(|(n, _)| n.as_str()).collect();

    let baseline = report::parse_baseline(include_str!("../gate-baseline.tsv")).expect("baseline");
    for (name, _) in &baseline {
        assert!(
            selected.contains(&name.as_str()),
            "{name} is baselined but CI's selection does not run it: {selected:?}"
        );
    }

    let record = purse_testutil::skew::read_text(&root.join("docs").join("PHASE1-EXIT.md"));
    let ci_paragraph = record
        .lines()
        .find(|l| l.starts_with("**CI.**"))
        .expect("PHASE1-EXIT.md has a \"CI.\" paragraph");
    let runs = format!("runs the same driver on {} micro workloads", selected.len());
    let gates = format!("any of the {} with a baseline", baseline.len());
    assert!(
        ci_paragraph.contains(&runs) && ci_paragraph.contains(&gates),
        "PHASE1-EXIT.md's CI paragraph should say {runs:?} and {gates:?} \
         (selected: {selected:?}):\n{ci_paragraph}"
    );
}

#[test]
fn smoke_refuses_a_disagreeing_go_side() {
    // A Go side that prints a different check is refused, with the check
    // named. Stands in for the Go binary with a script, so it runs only
    // where /bin/sh does.
    if !cfg!(unix) {
        return;
    }
    let out = out_dir("refuse");
    std::fs::create_dir_all(&out).expect("mkdir");
    let script = out.with_extension("sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'CHECK\\tgo\\thd.master_pub\\tnot-the-key\\n'\n",
    )
    .expect("write script");
    let _ = Command::new("chmod").arg("+x").arg(&script).status();
    let output = Command::new(env!("CARGO_BIN_EXE_purse-bench"))
        .args([
            "drive",
            "--smoke",
            "--runs",
            "1",
            "--only",
            "hd.master",
            "--go-bin",
        ])
        .arg(&script)
        .arg("--out")
        .arg(out.join("session"))
        .output()
        .expect("run purse-bench");
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("hd.master_pub"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_file(&script);
}
