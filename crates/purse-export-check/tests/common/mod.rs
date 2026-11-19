// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Helpers shared by the integration tests.

#![allow(dead_code)] // each test binary uses a subset

use std::path::{Path, PathBuf};

use purse_testutil::serde_json::{Value, json};
use purse_testutil::{Oracle, oracle_or_skip};

/// The seed every generated wallet is created from.
pub const SEED: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
/// The scenario's clock and chain-time base (2026-11-19).
pub const TIME: i64 = 1_794_990_000;
/// Passphrases of the generated wallets (tools/oracle/cmd/walletgen).
pub const PUBPASS: &str = "public";
pub const PRIVPASS: &str = "private";
pub const ACCT1_PASS: &str = "account-one-pass";
pub const VOTING_PASS: &str = "voting-pass";
/// Account number of the scenario's voting account (the second imported
/// account number).
pub const VOTING_ACCOUNT: &str = "2147483649";

/// This crate's `tests/data` directory.
pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// dcrwallet's fixture databases, kept by purse-boltread.
pub fn upstream_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../purse-boltread/tests/data/upstream")
}

/// A fresh, empty scratch directory under cargo's target tmpdir.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-export-check")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

pub fn path_str(p: &Path) -> &str {
    p.to_str().expect("UTF-8 path")
}

/// One raw write for [`Oracles::mutate`]: `(op, bucket path, key, value)`.
pub type MutateOp<'a> = (&'a str, &'a [&'a [u8]], &'a [u8], &'a [u8]);

/// The two oracle binaries.
pub struct Oracles {
    pub gen_: Oracle,
    pub exp: Oracle,
}

pub fn oracles() -> Option<Oracles> {
    let gen_ = oracle_or_skip("walletgen")?;
    let exp = oracle_or_skip("walletexport")?;
    Some(Oracles { gen_, exp })
}

impl Oracles {
    /// Creates a simnet wallet from [`SEED`] with the given scenario.
    pub fn create(&mut self, path: &Path, scenario: &str, time: i64, chain_time: i64) -> Value {
        self.gen_.call(
            "create",
            json!({
                "path": path_str(path), "network": "simnet", "seed": SEED,
                "pubpass": PUBPASS, "privpass": PRIVPASS, "scenario": scenario,
                "time": time, "chain_time": chain_time,
            }),
        )
    }

    /// Upgrades an upstream fixture to version 26 at `out`.
    pub fn upgrade(&mut self, fixture: &str, out: &Path) -> Value {
        self.gen_.call(
            "upgrade",
            json!({
                "gz": path_str(&upstream_dir().join(fixture)),
                "out": path_str(out),
            }),
        )
    }

    /// Opens and closes `path` as dcrwallet does at start-up.
    pub fn reopen(&mut self, path: &Path, network: &str) {
        self.gen_.call(
            "reopen",
            json!({"path": path_str(path), "network": network, "pubpass": PUBPASS}),
        );
    }

    /// Applies raw bbolt writes to a generated database (walletgen
    /// `mutate`): each op is `(op, bucket path, key, value)`.
    pub fn mutate(&mut self, path: &Path, ops: &[MutateOp<'_>]) -> Value {
        let ops: Vec<Value> = ops
            .iter()
            .map(|(op, bpath, key, value)| {
                json!({
                    "op": op,
                    "path": bpath.iter().map(|c| purse_testutil::hex(c)).collect::<Vec<_>>(),
                    "key": purse_testutil::hex(key),
                    "value": purse_testutil::hex(value),
                })
            })
            .collect();
        self.gen_
            .call("mutate", json!({"path": path_str(path), "ops": ops}))
    }

    /// Exports `db` to `out` and returns the result and the text.
    pub fn export(
        &mut self,
        db: &Path,
        out: &Path,
        network: &str,
        extra: Value,
    ) -> (Value, String) {
        let mut args = json!({
            "path": path_str(db), "out": path_str(out), "network": network,
            "pubpass": PUBPASS,
        });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                args[k] = v;
            }
        }
        let res = self.exp.call("export", args);
        let text = std::fs::read_to_string(out).expect("read export");
        (res, text)
    }

    /// Exports a scenario wallet with every private check.
    pub fn export_private(&mut self, db: &Path, out: &Path) -> (Value, String) {
        self.export(
            db,
            out,
            "simnet",
            json!({
                "privpass": PRIVPASS,
                "acctpass": {"1": ACCT1_PASS, VOTING_ACCOUNT: VOTING_PASS},
            }),
        )
    }
}
