// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Test utilities shared by the dcr-purse crates.
//!
//! The main one is the **oracle harness**. `tools/oracle` is a Go module
//! pinned to dcrwallet at the parity pin; each directory under its `cmd/` is
//! one binary that links dcrwallet's own packages and answers requests over
//! stdin/stdout, one JSON object per line. Tests build a binary on demand
//! with `go build` into `<target-dir>/<profile>/oracle/` (or take a prebuilt
//! one from `PURSE_ORACLE_DIR`) and compare the Rust code's answers with Go's.
//!
//! Without a Go toolchain a differential test skips with a notice, unless
//! `PURSE_REQUIRE_ORACLE` is set (as in CI), which makes it a failure, so
//! differential coverage cannot silently disappear.

use std::env;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

pub use serde_json;
use serde_json::Value;

/// The dcrwallet commit every parity claim names (master, 2.2.0-pre).
pub const DCRWALLET_PARITY_COMMIT: &str = "54b85b336077587eaf20112dc695355ce06fc76e";

/// Lower-case hex of `b`.
pub fn hex(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().saturating_mul(2));
    for byte in b {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Decodes hex, panicking on malformed input (test data only).
pub fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd-length hex: {s:?}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i.saturating_add(2)], 16).expect("hex digit"))
        .collect()
}

/// SplitMix64: a small deterministic PRNG for generating test inputs.
///
/// Not a cryptographic generator and never used outside tests.
#[derive(Clone, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// A generator seeded from `PURSE_TEST_SEED` (decimal or `0x` hex) when
    /// set, and from a fixed per-label default otherwise. The seed is printed
    /// so a failing run can be reproduced.
    pub fn from_env(label: &str) -> Self {
        let seed = env::var("PURSE_TEST_SEED")
            .ok()
            .and_then(|v| parse_seed(&v))
            .unwrap_or_else(|| {
                label.bytes().fold(0x9e37_79b9_7f4a_7c15_u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            });
        eprintln!("{label}: PURSE_TEST_SEED={seed:#x}");
        SplitMix64(seed)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n` must be nonzero; slightly biased, fine for tests).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64().checked_rem(n).expect("n must be nonzero")
    }

    /// Fills `buf` with pseudo-random bytes.
    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }

    /// `len` pseudo-random bytes.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        self.fill(&mut out);
        out
    }
}

/// Parses a seed given as decimal or `0x`-prefixed hex.
pub fn parse_seed(value: &str) -> Option<u64> {
    let value = value.trim();
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}

/// Whether a Go toolchain is on the path.
pub fn go_available() -> bool {
    Command::new("go")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The `tools/oracle` directory of the workspace the running test belongs to.
///
/// Found from `CARGO_MANIFEST_DIR` as cargo sets it when it runs a test (the
/// directory of the crate under test, two levels below the workspace root),
/// not from the value compiled into this crate: with a target directory
/// shared between copies of the workspace, the compiled-in path can be
/// another copy's.
pub fn oracle_source_dir() -> PathBuf {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    manifest_dir
        .join("../../tools/oracle")
        .canonicalize()
        .unwrap_or_else(|e| {
            panic!(
                "no tools/oracle two levels above {}: {e}",
                manifest_dir.display()
            )
        })
}

/// The directory oracle binaries are built into: `oracle/` in the profile
/// directory the running test binary lives under, so it follows whatever
/// target directory cargo was given and every test binary shares one copy.
///
/// Cargo has put test binaries at `<profile>/deps/<bin>` and, with the newer
/// build-directory layout, at `<profile>/build/<pkg>/<hash>/out/<bin>`; the
/// profile directory is the parent of the nearest `deps` or `build` ancestor.
fn oracle_out_dir() -> PathBuf {
    let exe = env::current_exe().expect("current_exe");
    let profile_dir = exe
        .ancestors()
        .skip(1)
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name == "deps" || name == "build")
        })
        .and_then(Path::parent)
        .or_else(|| exe.parent())
        .expect("test binary has a parent directory");
    profile_dir.join("oracle")
}

/// Serializes `go build` within one test process. Separate test processes
/// may build the same binary at once; each links to a name of its own and
/// renames it into place, so none ever runs a half-written file.
static BUILD_LOCK: Mutex<()> = Mutex::new(());

/// FNV-1a over the files an oracle binary is built from: `go.mod`, `go.sum`,
/// everything under `internal/` and everything under `cmd/<name>/`, each
/// with its path, in sorted order.
fn oracle_source_hash(name: &str) -> Result<u64, String> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|e| format!("read {}: {e}", dir.display()))?
                .path();
            if path.is_dir() {
                collect(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let root = oracle_source_dir();
    let mut files = vec![root.join("go.mod"), root.join("go.sum")];
    collect(&root.join("internal"), &mut files)?;
    collect(&root.join("cmd").join(name), &mut files)?;
    files.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for file in &files {
        let relative = file.strip_prefix(&root).unwrap_or(file);
        feed(relative.to_string_lossy().replace('\\', "/").as_bytes());
        feed(&[0]);
        let contents = std::fs::read(file).map_err(|e| format!("read {}: {e}", file.display()))?;
        feed(&(contents.len() as u64).to_le_bytes());
        feed(&contents);
    }
    Ok(hash)
}

/// Why no oracle binary is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OracleUnavailable {
    /// There is no oracle to run: no Go toolchain on the path. A test may
    /// skip on this, unless `PURSE_REQUIRE_ORACLE` is set.
    Missing(String),
    /// An oracle should exist and does not work: `go build` failed, a
    /// directory could not be written, or `PURSE_ORACLE_DIR` names no such
    /// binary. Never a reason to skip: a broken oracle would otherwise turn
    /// every differential test into a silent pass.
    Broken(String),
}

/// Builds the oracle binary `tools/oracle/cmd/<name>`, unless one built from
/// the same sources is already there, and returns its path.
///
/// When `PURSE_ORACLE_DIR` is set, nothing is built: the binary
/// `purse-oracle-<name>` in that directory is used as it is, so a run can be
/// pointed at oracles built elsewhere.
pub fn build_oracle(name: &str) -> Result<PathBuf, OracleUnavailable> {
    use OracleUnavailable::{Broken, Missing};
    let file_name = format!("purse-oracle-{name}{}", env::consts::EXE_SUFFIX);
    if let Some(dir) = env::var_os("PURSE_ORACLE_DIR") {
        let bin = Path::new(&dir).join(&file_name);
        return if bin.is_file() {
            Ok(bin)
        } else {
            Err(Broken(format!(
                "PURSE_ORACLE_DIR is set but {} does not exist",
                bin.display()
            )))
        };
    }
    if !go_available() {
        return Err(Missing("no Go toolchain on the path".to_owned()));
    }
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out_dir = oracle_out_dir();
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| Broken(format!("create {}: {e}", out_dir.display())))?;
    // The name carries a hash of the sources, so a copy of this workspace
    // sharing the target directory (a worktree, a scratch copy under test)
    // with different oracle sources never runs, or replaces, this one's
    // binary; and an unchanged oracle is built once, not once per test run.
    let hash = oracle_source_hash(name).map_err(Broken)?;
    let bin = out_dir.join(format!(
        "purse-oracle-{name}-{hash:016x}{}",
        env::consts::EXE_SUFFIX
    ));
    if bin.is_file() {
        return Ok(bin);
    }
    let staged = out_dir.join(format!("{file_name}.{}.tmp", std::process::id()));
    let output = Command::new("go")
        .arg("build")
        .arg("-o")
        .arg(&staged)
        .arg(format!("./cmd/{name}"))
        .current_dir(oracle_source_dir())
        .output()
        .map_err(|e| Broken(format!("run go build: {e}")))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&staged);
        return Err(Broken(format!(
            "go build ./cmd/{name} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if let Err(e) = std::fs::rename(&staged, &bin) {
        // Where a running binary cannot be replaced (Windows), another test
        // process is using the copy it just built from the same source.
        let _ = std::fs::remove_file(&staged);
        if !bin.is_file() {
            return Err(Broken(format!("install {}: {e}", bin.display())));
        }
    }
    Ok(bin)
}

/// Builds the oracle binary `name`, or returns `None` (the test should
/// return early) when there is no Go toolchain to build it with. Panics
/// instead when `PURSE_REQUIRE_ORACLE` is set, and always panics when the
/// oracle is broken rather than missing.
pub fn oracle_bin_or_skip(name: &str) -> Option<PathBuf> {
    match build_oracle(name) {
        Ok(bin) => Some(bin),
        Err(OracleUnavailable::Broken(why)) => panic!("oracle {name}: {why}"),
        Err(OracleUnavailable::Missing(why)) => {
            assert!(
                env::var_os("PURSE_REQUIRE_ORACLE").is_none(),
                "PURSE_REQUIRE_ORACLE is set but {why}"
            );
            eprintln!("skipping: {why} (set PURSE_REQUIRE_ORACLE to make this an error)");
            None
        }
    }
}

/// Spawns the oracle binary `name`, or returns `None` as
/// [`oracle_bin_or_skip`] does.
pub fn oracle_or_skip(name: &str) -> Option<Oracle> {
    oracle_bin_or_skip(name).map(|bin| Oracle::spawn_at(&bin))
}

/// A running oracle process.
#[derive(Debug)]
pub struct Oracle {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    line: String,
}

/// An error an oracle command returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleError {
    /// The Go error's text.
    pub text: String,
    /// The machine-readable kind, when the command supplies one.
    pub kind: Option<String>,
}

impl Oracle {
    /// Spawns the oracle binary at `bin`.
    pub fn spawn_at(bin: &Path) -> Self {
        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Oracle {
            child,
            stdin,
            stdout,
            line: String::new(),
        }
    }

    /// Sends `cmd` with `args` and returns the command's result, or the
    /// error Go returned. Panics if the oracle process dies (a Go panic) or
    /// breaks the protocol.
    pub fn try_call(&mut self, cmd: &str, args: Value) -> Result<Value, OracleError> {
        let request = serde_json::json!({ "cmd": cmd, "args": args });
        let stdin = self.stdin.as_mut().expect("oracle stdin open");
        let mut line = serde_json::to_string(&request).expect("serialize request");
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .unwrap_or_else(|e| panic!("oracle {cmd}: write: {e}"));
        self.line.clear();
        let n = self
            .stdout
            .read_line(&mut self.line)
            .unwrap_or_else(|e| panic!("oracle {cmd}: read: {e}"));
        assert!(n != 0, "oracle exited while serving {cmd} (a Go panic?)");
        let mut response: Value = serde_json::from_str(&self.line)
            .unwrap_or_else(|e| panic!("oracle {cmd}: bad response {:?}: {e}", self.line));
        if response.get("failed").and_then(Value::as_bool) == Some(true) {
            return Err(OracleError {
                text: response["error"].as_str().unwrap_or_default().to_owned(),
                kind: response
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        Ok(response
            .get_mut("result")
            .map(Value::take)
            .unwrap_or(Value::Null))
    }

    /// As [`Oracle::try_call`], panicking on a Go error.
    pub fn call(&mut self, cmd: &str, args: Value) -> Value {
        self.try_call(cmd, args)
            .unwrap_or_else(|e| panic!("oracle {cmd} failed: {} ({:?})", e.text, e.kind))
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        // Closing stdin makes the oracle exit at EOF.
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let mut rng = SplitMix64(7);
        let bytes = rng.bytes(33);
        assert_eq!(unhex(&hex(&bytes)), bytes);
    }

    #[test]
    fn oracle_pin_matches_go_mod() {
        let go_mod = std::fs::read_to_string(oracle_source_dir().join("go.mod")).expect("go.mod");
        let short = &DCRWALLET_PARITY_COMMIT[..12];
        assert!(
            go_mod
                .lines()
                .any(|l| l.contains("decred.org/dcrwallet/v5 ") && l.ends_with(short)),
            "tools/oracle/go.mod does not pin dcrwallet at {short}"
        );
    }
}
