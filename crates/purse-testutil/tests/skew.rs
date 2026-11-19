// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The mechanical checks of ADR-0009 (decision item 3) on the version-skew
//! ledger `docs/SKEW.md`:
//!
//! - (a) the ledger has one row per dcrd module dcrwallet's `go.mod`
//!   requires, the two neighbours and the Go toolchain, and no other; its
//!   dcrwallet column equals the requirements of
//!   `tools/oracle/dcrwallet.go.mod`, dcrwallet's `go.mod` at the pin (the Go
//!   test `tools/oracle/internal/deps` holds that copy to `go.sum`); every
//!   cell is filled in, Delta and Disposition from their fixed classes;
//! - (b) every module the oracle binaries link is at the version that copy
//!   requires, and none is a dcrd module dcrwallet does not require; one
//!   binary's `build.info` must equal `go list -deps` for it, and `go list
//!   -deps` gives the others'; a ledger row whose module no oracle binary
//!   links must say "no differential yet";
//! - (c) the ledger's header matches `DCRWALLET_PARITY_COMMIT`, the dcroxide
//!   `rev` of the root `Cargo.toml` and every dcroxide commit `Cargo.lock`
//!   builds, `REFERENCE_GO` and CI's `GO_VERSION`, and, at that rev,
//!   dcroxide's own dcrd pin (its CI's full `DCRD_COMMIT`, and the prefixes
//!   its other files carry) and `GO_VERSION`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use purse_testutil::serde_json::{self, Value};
use purse_testutil::skew::{
    GoMod, Ledger, dcroxide_lock_commits, dcroxide_revs, first_code_span, is_dcrd_module,
    parse_go_mod, parse_ledger, read_text, workflow_env, workflow_go_version, workspace_root,
};
use purse_testutil::{
    DCRWALLET_PARITY_COMMIT, Oracle, OracleUnavailable, REFERENCE_GO, build_oracle, go_available,
    is_reference_go, oracle_source_dir,
};

/// The header keys of `docs/SKEW.md`.
const H_DCRWALLET: &str = "dcrwallet";
const H_DCROXIDE_REV: &str = "dcroxide rev";
const H_DCRD_PIN: &str = "dcroxide's dcrd pin";
const H_GO_PURSE: &str = "Reference Go, dcr-purse";
const H_GO_DCROXIDE: &str = "Reference Go, dcroxide";

/// The ledger's row for the Go toolchain.
const TOOLCHAIN_ROW: &str = "Go toolchain";

/// The two neighbouring libraries ADR-0009 gives rows of their own.
const NEIGHBOURS: [&str; 2] = ["github.com/jessevdk/go-flags", "github.com/jrick/logrotate"];

/// How a Delta cell may begin (ADR-0009 item 1; the classes of the ledger).
const DELTA_CLASSES: [&str; 3] = ["none", "comments or tests only", "code"];

/// How a Disposition cell may begin.
const DISPOSITIONS: [&str; 5] = [
    "reuse as is",
    "parameterise in dcroxide",
    "wallet-side constant or shim",
    "own port",
    "not linked",
];

/// A Differential cell for a module no oracle binary links.
const NO_DIFFERENTIAL: &str = "no differential yet";

/// Appended to a failure that compares something with `dcrwallet.go.mod`:
/// these tests take the copy as correct, and only the Go test holds it to
/// `go.sum`.
const GO_MOD_HINT: &str = " (or tools/oracle/dcrwallet.go.mod was edited: run `go test \
                           ./internal/deps` in tools/oracle, which holds the copy to go.sum)";

fn ledger() -> Ledger {
    let path = workspace_root().join("docs").join("SKEW.md");
    parse_ledger(&read_text(&path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn dcrwallet_go_mod() -> GoMod {
    let go_mod = parse_go_mod(&read_text(&oracle_source_dir().join("dcrwallet.go.mod")));
    assert_eq!(
        go_mod.module, "decred.org/dcrwallet/v5",
        "tools/oracle/dcrwallet.go.mod is not dcrwallet's go.mod{GO_MOD_HINT}"
    );
    go_mod
}

fn header<'a>(ledger: &'a Ledger, key: &str) -> &'a str {
    ledger
        .header
        .get(key)
        .unwrap_or_else(|| panic!("docs/SKEW.md has no header line \"- {key}: `...`\""))
}

/// The root of cargo's checkout of dcroxide at the pinned rev.
///
/// Cargo checks a git dependency out whole under
/// `$CARGO_HOME/git/checkouts/<name>-<hash>/<short rev>/`, and building the
/// workspace's dcroxide crates leaves it there. Where that directory cannot
/// be found (a `CARGO_HOME` this process does not see), `cargo metadata`
/// names the manifest of `dcroxide-wire`, three levels below the root; it
/// is the fallback only, since offline it needs every platform's packages
/// downloaded.
fn dcroxide_checkout(rev: &str) -> PathBuf {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        });
    if let Some(checkouts) = cargo_home.map(|h| h.join("git").join("checkouts"))
        && let Ok(repos) = std::fs::read_dir(&checkouts)
    {
        for repo in repos.filter_map(Result::ok).map(|e| e.path()) {
            let is_dcroxide = repo
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dcroxide-"));
            if !is_dcroxide {
                continue;
            }
            let Ok(revs) = std::fs::read_dir(&repo) else {
                continue;
            };
            for dir in revs.filter_map(Result::ok).map(|e| e.path()) {
                let at_rev = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.len() >= 7 && rev.starts_with(n));
                if at_rev && dir.join("crates").join("dcroxide-wire").is_dir() {
                    return dir;
                }
            }
        }
    }

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
        .current_dir(workspace_root())
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "no dcroxide checkout at {rev} under CARGO_HOME, and cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("cargo metadata JSON");
    let wire = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| {
            p["name"] == "dcroxide-wire"
                && p["source"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("git+") && s.ends_with(&format!("#{rev}")))
        })
        .unwrap_or_else(|| panic!("cargo metadata lists no dcroxide-wire at rev {rev}"));
    let manifest = PathBuf::from(wire["manifest_path"].as_str().expect("manifest_path"));
    manifest
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| panic!("no checkout root above {}", manifest.display()))
}

/// (c) The header names the pins every other file names.
#[test]
fn ledger_header_matches_the_pins() {
    let ledger = ledger();
    let root = workspace_root();

    assert_eq!(
        header(&ledger, H_DCRWALLET),
        DCRWALLET_PARITY_COMMIT,
        "SKEW.md's dcrwallet commit"
    );

    let rev = header(&ledger, H_DCROXIDE_REV);
    let revs = dcroxide_revs(&read_text(&root.join("Cargo.toml")));
    assert!(
        !revs.is_empty(),
        "the root Cargo.toml names no dcroxide rev"
    );
    for r in &revs {
        assert_eq!(r, rev, "Cargo.toml's dcroxide rev against SKEW.md's");
    }
    // Cargo.lock records the commit every dcroxide package is built from,
    // whichever manifest declared it.
    let commits = dcroxide_lock_commits(&read_text(&root.join("Cargo.lock")));
    assert!(
        !commits.is_empty(),
        "Cargo.lock has no package from the dcroxide repository"
    );
    for c in &commits {
        assert_eq!(
            c, rev,
            "Cargo.lock builds a dcroxide package at {c}, SKEW.md's dcroxide rev is {rev}"
        );
    }

    let go_purse = header(&ledger, H_GO_PURSE);
    assert_eq!(
        go_purse, REFERENCE_GO,
        "SKEW.md's dcr-purse Go against REFERENCE_GO"
    );
    let ci = read_text(&root.join(".github").join("workflows").join("ci.yml"));
    assert_eq!(
        workflow_go_version(&ci).as_deref(),
        Some(go_purse),
        "SKEW.md's dcr-purse Go against ci.yml's GO_VERSION"
    );

    // dcroxide's own files at the rev the workspace links.
    let dcroxide = dcroxide_checkout(rev);
    let go_dcroxide = header(&ledger, H_GO_DCROXIDE);
    let dcroxide_ci = read_text(&dcroxide.join(".github").join("workflows").join("ci.yml"));
    assert_eq!(
        workflow_go_version(&dcroxide_ci).as_deref(),
        Some(go_dcroxide),
        "SKEW.md's dcroxide Go against dcroxide's ci.yml GO_VERSION at {rev}"
    );
    let pin = header(&ledger, H_DCRD_PIN);
    assert!(
        pin.len() == 40 && pin.bytes().all(|b| b.is_ascii_hexdigit()),
        "SKEW.md's dcrd pin is not a full commit hash: {pin}"
    );
    // The one full hash dcroxide records: the commit its CI builds dcrd at.
    // The other three sources below carry prefixes of it.
    assert_eq!(
        workflow_env(&dcroxide_ci, "DCRD_COMMIT").as_deref(),
        Some(pin),
        "SKEW.md's \"{H_DCRD_PIN}\" against dcroxide's ci.yml DCRD_COMMIT at {rev}"
    );
    let parity = read_text(&dcroxide.join("docs").join("PARITY.md"));
    assert!(
        parity.contains(&format!("master `{}`", &pin[..8])),
        "dcroxide's docs/PARITY.md at {rev} does not name dcrd master {}",
        &pin[..8]
    );
    let oracle_go_mod = read_text(&dcroxide.join("tools").join("oracle").join("go.mod"));
    assert!(
        oracle_go_mod.contains(&format!("-{}\n", &pin[..12])),
        "dcroxide's tools/oracle/go.mod at {rev} has no pseudo-version at {}",
        &pin[..12]
    );
    let testutil = read_text(
        &dcroxide
            .join("crates")
            .join("dcroxide-testutil")
            .join("src")
            .join("lib.rs"),
    );
    let pinned = testutil
        .lines()
        .find_map(|l| {
            let rest = l
                .trim()
                .strip_prefix("pub const DCRD_PARITY_COMMIT: &str = \"")?;
            rest.split_once('"').map(|(c, _)| c.to_owned())
        })
        .expect("dcroxide-testutil defines DCRD_PARITY_COMMIT");
    assert!(
        !pinned.is_empty() && pin.starts_with(&pinned),
        "dcroxide-testutil's DCRD_PARITY_COMMIT {pinned} at {rev} against SKEW.md's {pin}"
    );

    // The toolchain row repeats the two releases.
    let row = ledger
        .rows
        .iter()
        .find(|r| r.module == TOOLCHAIN_ROW)
        .expect("SKEW.md has no Go toolchain row");
    assert_eq!(
        first_code_span(row.cell("dcrwallet")),
        Some(go_purse),
        "SKEW.md's Go toolchain row, dcrwallet column, against the header's \"{H_GO_PURSE}\""
    );
    assert_eq!(
        first_code_span(row.cell("dcroxide")),
        Some(go_dcroxide),
        "SKEW.md's Go toolchain row, dcroxide column, against the header's \"{H_GO_DCROXIDE}\""
    );

    eprintln!(
        "skew header: dcrwallet {}, dcroxide {rev} (dcrd {pin}), {go_purse} and {go_dcroxide}: \
         match the pins",
        &DCRWALLET_PARITY_COMMIT[..8]
    );
}

/// (a) One row per dcrd module dcrwallet requires, the two neighbours and
/// the toolchain, each at dcrwallet's version, with a recognised delta class
/// and disposition.
#[test]
fn ledger_rows_match_dcrwallet_go_mod() {
    let ledger = ledger();
    let go_mod = dcrwallet_go_mod();

    let mut seen = BTreeSet::new();
    for row in &ledger.rows {
        assert!(
            seen.insert(row.module.clone()),
            "two rows for {}",
            row.module
        );
        let delta = row.cell("Delta");
        assert!(
            DELTA_CLASSES.iter().any(|c| delta.starts_with(c)),
            "{}: the Delta cell does not start with one of {DELTA_CLASSES:?}: {delta}",
            row.module
        );
        let disposition = row.cell("Disposition");
        assert!(
            DISPOSITIONS.iter().any(|d| disposition.starts_with(d)),
            "{}: the Disposition cell does not start with one of {DISPOSITIONS:?}: {disposition}",
            row.module
        );
        for column in [
            "dcrwallet",
            "dcroxide",
            "Diff",
            "Observable",
            "Differential",
        ] {
            assert!(
                !row.cell(column).is_empty(),
                "{}: the {column} cell is empty",
                row.module
            );
        }
        if row.module == TOOLCHAIN_ROW {
            continue;
        }
        assert!(
            is_dcrd_module(&row.module) || NEIGHBOURS.contains(&row.module.as_str()),
            "SKEW.md has a row for {}, which is neither a dcrd module, one of the neighbours \
             {NEIGHBOURS:?} nor the {TOOLCHAIN_ROW} (ADR-0009 item 1)",
            row.module
        );
        let required = go_mod.require.get(&row.module).unwrap_or_else(|| {
            panic!(
                "SKEW.md has a row for {}, which dcrwallet.go.mod does not require{GO_MOD_HINT}",
                row.module
            )
        });
        assert_eq!(
            first_code_span(row.cell("dcrwallet")),
            Some(required.as_str()),
            "{}: SKEW.md's dcrwallet column against dcrwallet.go.mod{GO_MOD_HINT}",
            row.module
        );
    }

    let dcrd: Vec<&str> = go_mod
        .require
        .keys()
        .map(String::as_str)
        .filter(|p| is_dcrd_module(p))
        .collect();
    for &module in dcrd.iter().chain(NEIGHBOURS.iter()) {
        assert!(
            seen.contains(module),
            "SKEW.md has no row for {module}{GO_MOD_HINT}"
        );
    }
    assert!(
        seen.contains(TOOLCHAIN_ROW),
        "SKEW.md has no Go toolchain row"
    );
    eprintln!(
        "skew rows: {} rows ({} dcrd modules, {} neighbours, the toolchain) at dcrwallet.go.mod's \
         versions",
        ledger.rows.len(),
        dcrd.len(),
        NEIGHBOURS.len()
    );
}

/// The oracle binaries: every `cmd/<name>` whose sources serve the
/// line-JSON protocol, and so answer `build.info`.
fn oracle_binaries() -> Vec<String> {
    let cmd = oracle_source_dir().join("cmd");
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&cmd).unwrap_or_else(|e| panic!("read {}: {e}", cmd.display())) {
        let dir = entry.expect("read cmd entry").path();
        if !dir.is_dir() {
            continue;
        }
        let serves = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "go"))
            .any(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains("linejson.Serve(")));
        if serves && let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    names
}

/// The modules a binary links, by path: the selected version and, where a
/// `replace` directive substituted the module, the replacement's path.
type LinkedSet = BTreeMap<String, (String, Option<String>)>;

/// The modules `tools/oracle/cmd/<cmd>` links, from `go list -deps`, which
/// reads the module's one build list without compiling anything: the set
/// `debug.ReadBuildInfo` reports in the built binary. `-e` lets a package
/// that does not load (a command being edited) still list what it imports;
/// its build failure is its own tests' to report, not a version skew.
fn go_list_linked(cmd: &str) -> LinkedSet {
    let output = Command::new("go")
        .args([
            "list",
            "-e",
            "-deps",
            "-f",
            "{{with .Module}}{{if not .Main}}{{.Path}}\t{{.Version}}\t\
             {{with .Replace}}{{.Path}}{{end}}{{end}}{{end}}",
        ])
        .arg(format!("./cmd/{cmd}"))
        .current_dir(oracle_source_dir())
        .output()
        .unwrap_or_else(|e| panic!("run go list: {e}"));
    assert!(
        output.status.success(),
        "go list -deps ./cmd/{cmd} in tools/oracle failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut linked = LinkedSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split('\t');
        if let (Some(path), Some(version), Some(replace)) =
            (fields.next(), fields.next(), fields.next())
        {
            let replace = (!replace.is_empty()).then(|| replace.to_owned());
            linked.insert(path.to_owned(), (version.to_owned(), replace));
        }
    }
    linked
}

/// The first of `binaries` that builds (or that `PURSE_ORACLE_DIR` holds),
/// spawned, or `None` where there is no Go toolchain and
/// `PURSE_REQUIRE_ORACLE` is unset. A binary that does not build is passed
/// over with a notice; only when none builds is that a failure here.
fn first_oracle(binaries: &[String]) -> Option<(String, Oracle)> {
    let require = std::env::var_os("PURSE_REQUIRE_ORACLE").is_some();
    let mut broken = Vec::new();
    for name in binaries {
        match build_oracle(name) {
            Ok(bin) => {
                let mut oracle = Oracle::spawn_at(&bin);
                if require {
                    let built_with = oracle.go_version();
                    assert!(
                        is_reference_go(&built_with),
                        "PURSE_REQUIRE_ORACLE is set but oracle {name} was built with \
                         {built_with:?}, not the reference {REFERENCE_GO}"
                    );
                }
                for why in &broken {
                    eprintln!("skew oracle: passed over {why}");
                }
                return Some((name.clone(), oracle));
            }
            Err(OracleUnavailable::Missing(why)) => {
                assert!(!require, "PURSE_REQUIRE_ORACLE is set but {why}");
                eprintln!("skipping: {why} (set PURSE_REQUIRE_ORACLE to make this an error)");
                return None;
            }
            Err(OracleUnavailable::Broken(why)) => broken.push(format!("oracle {name}: {why}")),
        }
    }
    panic!(
        "no oracle binary under tools/oracle/cmd builds:\n{}",
        broken.join("\n")
    );
}

/// (b) Every module an oracle binary links is at the version dcrwallet's
/// go.mod requires; no dcrd module outside that go.mod is linked; nothing
/// is replaced. Ledger rows whose module no binary links must carry no
/// differential.
///
/// The oracle requires dcrwallet, so minimal version selection never picks
/// a version below dcrwallet's requirement: "not above" it is equality,
/// and equality is what is checked.
///
/// Selection covers the whole module, so every binary links its modules at
/// the versions of one build list. One binary is built and its `build.info`
/// must equal what `go list -deps` gives for it; the modules of every oracle
/// binary are then read with `go list -deps`, without building them, so a
/// command another change has left unbuildable does not fail this check.
#[test]
fn oracle_links_dcrwallet_versions() {
    let go_mod = dcrwallet_go_mod();
    let ledger = ledger();
    let binaries = oracle_binaries();
    assert!(
        !binaries.is_empty(),
        "no oracle binary under tools/oracle/cmd"
    );
    if !go_available() {
        assert!(
            std::env::var_os("PURSE_REQUIRE_ORACLE").is_none(),
            "PURSE_REQUIRE_ORACLE is set but there is no Go toolchain on the path"
        );
        eprintln!(
            "skipping: no Go toolchain on the path (set PURSE_REQUIRE_ORACLE to make this an error)"
        );
        return;
    }

    let Some((probe, mut oracle)) = first_oracle(&binaries) else {
        return;
    };
    let info = oracle.build_info();
    assert!(
        !info.deps.is_empty(),
        "oracle {probe}'s build.info reports no modules (main {:?})",
        info.main
    );
    let reported: LinkedSet = info
        .deps
        .iter()
        .map(|d| {
            let replace = d.replace.as_ref().map(|r| r.path.clone());
            (d.path.clone(), (d.version.clone(), replace))
        })
        .collect();
    assert_eq!(
        reported,
        go_list_linked(&probe),
        "oracle {probe}'s build.info against `go list -deps ./cmd/{probe}` in tools/oracle: \
         the binary was not built from these sources (a stale PURSE_ORACLE_DIR?)"
    );

    // module path -> (version, binaries linking it)
    let mut linked: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for name in &binaries {
        for (path, (version, replace)) in go_list_linked(name) {
            assert!(
                replace.is_none(),
                "oracle {name} links {path} replaced by {replace:?}"
            );
            let entry = linked
                .entry(path.clone())
                .or_insert_with(|| (version.clone(), Vec::new()));
            assert_eq!(
                entry.0, version,
                "{path} at two versions across the oracle binaries"
            );
            entry.1.push(name.clone());
        }
    }

    let mut failures = Vec::new();
    let mut outside = Vec::new();
    let mut dcrd_linked = 0usize;
    for (path, (version, by)) in &linked {
        if *path == go_mod.module {
            // dcrwallet itself, at the parity commit's pseudo-version.
            if !version.ends_with(&format!("-{}", &DCRWALLET_PARITY_COMMIT[..12])) {
                failures.push(format!(
                    "{path} resolves to {version} (in {}), not the parity commit {}",
                    by.join(", "),
                    &DCRWALLET_PARITY_COMMIT[..12]
                ));
            }
            continue;
        }
        match go_mod.require.get(path) {
            Some(required) if required == version => {
                if is_dcrd_module(path) {
                    dcrd_linked = dcrd_linked.saturating_add(1);
                }
            }
            Some(required) => failures.push(format!(
                "{path} resolves to {version} (in {}), dcrwallet requires {required}",
                by.join(", ")
            )),
            None if is_dcrd_module(path) => failures.push(format!(
                "{path} {version} (in {}) is a dcrd module dcrwallet's go.mod does not require",
                by.join(", ")
            )),
            None => outside.push(format!("{path} {version}")),
        }
    }
    assert!(
        failures.is_empty(),
        "the oracle links off dcrwallet's versions:\n{}\n{}",
        failures.join("\n"),
        GO_MOD_HINT.trim_start()
    );

    let mut no_differential = Vec::new();
    for row in ledger.rows.iter().filter(|r| r.module != TOOLCHAIN_ROW) {
        if !linked.contains_key(&row.module) {
            assert!(
                row.cell("Differential").starts_with(NO_DIFFERENTIAL),
                "{}: no oracle binary links it, but SKEW.md names a differential: {}",
                row.module,
                row.cell("Differential")
            );
            no_differential.push(row.module.as_str());
        }
    }
    if !outside.is_empty() {
        eprintln!(
            "skew oracle: linked, outside dcrwallet's go.mod (not dcrd modules): {}",
            outside.join(", ")
        );
    }
    eprintln!(
        "skew oracle: absent from the oracle's graph, no differential yet: {}",
        no_differential.join(", ")
    );
    eprintln!(
        "skew oracle: {probe}'s build.info equals go list's; {} binaries link {} modules, \
         {dcrd_linked} of them dcrd's, all at dcrwallet.go.mod's versions",
        binaries.len(),
        linked.len()
    );
}
