// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Readers for the version-skew checks of ADR-0009 (decision item 3):
//! `go.mod` files, the ledger `docs/SKEW.md`, and the small pieces of CI
//! and Cargo configuration its header is held to.
//!
//! Every reader normalizes CRLF to LF first, so a Windows checkout with
//! `core.autocrlf` parses as the LF original does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The workspace root: two levels above `tools/oracle`.
pub fn workspace_root() -> PathBuf {
    let oracle = crate::oracle_source_dir();
    oracle
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| panic!("no workspace root above {}", oracle.display()))
}

/// Reads a text file with CRLF line endings normalized to LF.
pub fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The parts of a `go.mod` the checks use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoMod {
    /// The module path.
    pub module: String,
    /// The `go` directive's version, without the `go ` prefix.
    pub go: String,
    /// Every `require`d module and its version, direct and indirect.
    pub require: BTreeMap<String, String>,
}

/// Parses the `module`, `go` and `require` directives of a `go.mod`, in
/// both the single-line and the block form. Comments are dropped; other
/// directives (`tool`, `replace`, `exclude`, ...) are skipped.
pub fn parse_go_mod(text: &str) -> GoMod {
    let mut out = GoMod::default();
    let mut block: Option<String> = None;
    for raw in text.replace("\r\n", "\n").lines() {
        let line = raw.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = &block {
            if line == ")" {
                block = None;
            } else if name == "require" {
                add_requirement(&mut out, line);
            }
            continue;
        }
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        if rest == "(" {
            block = Some(verb.to_owned());
            continue;
        }
        match verb {
            "module" => out.module = rest.trim_matches('"').to_owned(),
            "go" => out.go = rest.to_owned(),
            "require" => add_requirement(&mut out, rest),
            _ => {}
        }
    }
    out
}

fn add_requirement(out: &mut GoMod, spec: &str) {
    let mut fields = spec.split_whitespace();
    if let (Some(path), Some(version)) = (fields.next(), fields.next()) {
        out.require.insert(path.to_owned(), version.to_owned());
    }
}

/// Whether `path` is one of dcrd's modules: those under
/// `github.com/decred/dcrd/`, and `github.com/decred/base58`, which left
/// dcrd's tree for a repository of its own and which dcroxide ports as one
/// of them (ADR-0009 counts it among dcrwallet's four indirect dcrd
/// modules).
pub fn is_dcrd_module(path: &str) -> bool {
    path.starts_with("github.com/decred/dcrd/") || path == "github.com/decred/base58"
}

/// The text inside the first pair of backticks in `s`.
pub fn first_code_span(s: &str) -> Option<&str> {
    let (_, rest) = s.split_once('`')?;
    let (code, _) = rest.split_once('`')?;
    Some(code)
}

/// One row of the ledger's table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    /// The module path (the first code span of the `Module` cell), or the
    /// cell's text where it has none (`Go toolchain`).
    pub module: String,
    /// Every cell, by its column's header, trimmed.
    pub cells: BTreeMap<String, String>,
}

impl LedgerRow {
    /// The cell under `column`, or the empty string.
    pub fn cell(&self, column: &str) -> &str {
        self.cells.get(column).map_or("", String::as_str)
    }
}

/// The ledger: its header list and its table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    /// The ``- key: `value` `` lines before the table, value by key.
    pub header: BTreeMap<String, String>,
    /// The table's rows, in order.
    pub rows: Vec<LedgerRow>,
}

/// The columns the ledger's table must have, in order (ADR-0009 item 1).
pub const LEDGER_COLUMNS: [&str; 8] = [
    "Module",
    "dcrwallet",
    "dcroxide",
    "Diff",
    "Delta",
    "Observable",
    "Disposition",
    "Differential",
];

fn split_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_owned()).collect()
}

/// Parses `docs/SKEW.md`: the header list (each ``- key: `value` `` line
/// before the table) and the one table, whose header row must be
/// [`LEDGER_COLUMNS`].
pub fn parse_ledger(text: &str) -> Result<Ledger, String> {
    let text = text.replace("\r\n", "\n");
    let mut ledger = Ledger::default();
    let mut lines = text.lines().peekable();
    let mut columns: Option<Vec<String>> = None;
    while let Some(line) = lines.next() {
        if line.starts_with('|') {
            let header = split_row(line);
            if header != LEDGER_COLUMNS {
                return Err(format!(
                    "the table's columns are {header:?}, not {LEDGER_COLUMNS:?}"
                ));
            }
            let separator = lines.next().unwrap_or_default();
            if !separator.starts_with("|---") {
                return Err(format!("no separator row under the header: {separator:?}"));
            }
            columns = Some(header);
            break;
        }
        if let Some(item) = line.strip_prefix("- ")
            && let Some((key, value)) = item.split_once(": ")
            && let Some(code) = first_code_span(value)
            && value.trim() == format!("`{code}`")
        {
            ledger.header.insert(key.trim().to_owned(), code.to_owned());
        }
    }
    let columns = columns.ok_or("no table")?;
    for line in lines {
        if !line.starts_with('|') {
            break;
        }
        let cells = split_row(line);
        if cells.len() != columns.len() {
            return Err(format!(
                "a row has {} cells, not {}: {line}",
                cells.len(),
                columns.len()
            ));
        }
        let first = cells.first().cloned().unwrap_or_default();
        let module = first_code_span(&first).map_or(first.clone(), str::to_owned);
        ledger.rows.push(LedgerRow {
            module,
            cells: columns.iter().cloned().zip(cells).collect(),
        });
    }
    if ledger.rows.is_empty() {
        return Err("the table has no rows".to_owned());
    }
    Ok(ledger)
}

/// The first value of an environment entry `<key>: <value>` in a GitHub
/// Actions workflow (quotes removed), such as `DCRD_COMMIT`.
pub fn workflow_env(workflow: &str, key: &str) -> Option<String> {
    workflow.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(key)?.strip_prefix(':')?;
        let v = rest.trim().trim_matches('"');
        (!v.is_empty()).then(|| v.to_owned())
    })
}

/// The value of `GO_VERSION: "<v>"` in a GitHub Actions workflow, with the
/// `go` prefix `runtime.Version` uses.
pub fn workflow_go_version(workflow: &str) -> Option<String> {
    workflow_env(workflow, "GO_VERSION").map(|v| format!("go{v}"))
}

/// Every `rev = "<rev>"` a `dcroxide-*` dependency names in a Cargo
/// manifest's lines.
pub fn dcroxide_revs(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("dcroxide-"))
        .filter_map(|line| {
            let (_, rest) = line.split_once("rev = \"")?;
            let (rev, _) = rest.split_once('"')?;
            Some(rev.to_owned())
        })
        .collect()
}

/// The commit of every package `Cargo.lock` takes from a git repository
/// named `dcroxide`: the `<commit>` of each
/// `source = "git+<url>/dcroxide[.git]?<query>#<commit>"`, as cargo
/// resolved it, whichever manifest declared the dependency.
pub fn dcroxide_lock_commits(lock: &str) -> Vec<String> {
    lock.lines()
        .filter_map(|line| {
            let source = line
                .trim()
                .strip_prefix("source = \"git+")?
                .strip_suffix('"')?;
            let (url, commit) = source.rsplit_once('#')?;
            let repo = url.split(['?', '#']).next().unwrap_or_default();
            let name = repo.trim_end_matches('/').rsplit('/').next()?;
            (name.trim_end_matches(".git") == "dcroxide").then(|| commit.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_mod_blocks_lines_and_crlf() {
        let text = "module example.com/m\r\n\r\ngo 1.26.0\r\n\r\nrequire (\r\n\
                    \tgithub.com/decred/dcrd/wire v1.8.0\r\n\
                    \tgithub.com/decred/base58 v1.0.6 // indirect\r\n)\r\n\r\n\
                    require golang.org/x/sys v0.48.0\r\n\r\n\
                    tool (\r\n\tgolang.org/x/tools/cmd/stringer\r\n)\r\n";
        let m = parse_go_mod(text);
        assert_eq!(m.module, "example.com/m");
        assert_eq!(m.go, "1.26.0");
        assert_eq!(m.require.len(), 3);
        assert_eq!(m.require["github.com/decred/dcrd/wire"], "v1.8.0");
        assert_eq!(m.require["github.com/decred/base58"], "v1.0.6");
        assert_eq!(m.require["golang.org/x/sys"], "v0.48.0");
    }

    #[test]
    fn dcrd_modules() {
        assert!(is_dcrd_module("github.com/decred/dcrd/wire"));
        assert!(is_dcrd_module("github.com/decred/base58"));
        assert!(!is_dcrd_module("github.com/decred/slog"));
        assert!(!is_dcrd_module("github.com/decred/dcrdx"));
    }

    #[test]
    fn ledger_header_and_rows() {
        let text = "# L\r\n\r\n- dcrwallet: `abc`\r\n- note: not `code` only\r\n\r\n\
                    | Module | dcrwallet | dcroxide | Diff | Delta | Observable | Disposition | Differential |\r\n\
                    |---|---|---|---|---|---|---|---|\r\n\
                    | `github.com/decred/dcrd/wire` | `v1.8.0` | x | `git diff a b -- wire` | none | — | reuse as is | no differential yet |\r\n\
                    | Go toolchain | `go1.27.4` | `go1.27.3` | d | code: x | no | reuse as is | t |\r\n\
                    \r\n## Notes\r\n";
        let l = parse_ledger(text).expect("parses");
        assert_eq!(l.header.len(), 1);
        assert_eq!(l.header["dcrwallet"], "abc");
        assert_eq!(l.rows.len(), 2);
        assert_eq!(l.rows[0].module, "github.com/decred/dcrd/wire");
        assert_eq!(first_code_span(l.rows[0].cell("dcrwallet")), Some("v1.8.0"));
        assert_eq!(l.rows[1].module, "Go toolchain");
        assert_eq!(l.rows[1].cell("Delta"), "code: x");
    }

    #[test]
    fn ledger_rejects_other_columns() {
        let text = "| Module | dcrwallet |\n|---|---|\n| a | b |\n";
        assert!(parse_ledger(text).is_err());
    }

    #[test]
    fn workflow_and_manifest_values() {
        assert_eq!(
            workflow_go_version(
                "env:\r\n  GO_VERSION: \"1.27.4\"\r\n"
                    .replace("\r\n", "\n")
                    .as_str()
            ),
            Some("go1.27.4".to_owned())
        );
        assert_eq!(
            workflow_env("  DCRD_COMMIT: 46e9\n  DCRD_COMMIT_X: no\n", "DCRD_COMMIT").as_deref(),
            Some("46e9")
        );
        assert_eq!(workflow_env("  DCRD_COMMITS: 46e9\n", "DCRD_COMMIT"), None);
        let manifest = "dcroxide-a = { git = \"u\", rev = \"r1\" }\nother = { rev = \"x\" }\n";
        assert_eq!(dcroxide_revs(manifest), ["r1"]);
        let lock = "source = \"git+https://example.com/o/dcroxide?rev=r1#c1\"\n\
                    source = \"git+https://example.com/o/dcroxide.git?branch=b#c2\"\n\
                    source = \"git+https://example.com/o/dcroxide-x?rev=r3#c3\"\n\
                    source = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
        assert_eq!(dcroxide_lock_commits(lock), ["c1", "c2"]);
    }
}
