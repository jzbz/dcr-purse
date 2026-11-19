// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays the frozen vectors in `tests/data`, generated from the Go oracle
//! (dcrwallet `wallet/txsizes` and `wallet/txrules` at the parity pin) by
//! `regenerate_frozen_vectors` in `tests/oracle.rs`, so the comparison with
//! Go is pinned on machines without Go. No expected value in the file was
//! written by hand.
//!
//! The answers are amd64 Go's. The crate gives amd64's answers on every
//! target, so the file holds on every platform.

#![allow(clippy::arithmetic_side_effects, reason = "test code: counters")]

mod common;

use std::collections::BTreeMap;

use common::*;

/// The entries the file must hold per command (checked against the body,
/// whose size and hash the file's header pins).
const COUNTS: &[(&str, usize)] = &[
    ("check.output", 605),
    ("dust.amount", 406),
    ("dust.output", 605),
    ("fee", 373),
    ("highfees", 160),
    ("highfees.tx", 104),
    ("poolfee.ticket", 971),
    ("poolfee.valid", 130),
    ("size.estimate", 120),
    ("size.input", 70),
    ("size.output", 70),
    ("size.scripts", 120),
    ("stake.subtype", 21),
];

#[test]
fn frozen_vectors_agree() {
    let entries = read_vectors(VECTORS_FILE);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut panics = 0;
    let mut failed = Vec::new();
    for e in &entries {
        let cmd = e["cmd"].as_str().expect("cmd");
        *seen.entry(cmd.to_owned()).or_default() += 1;
        if e["out"].get("panic").is_some() {
            panics += 1;
        }
        let rust = rust_answer(cmd, &e["in"]);
        if rust != e["out"] && failed.len() < 10 {
            failed.push(format!(
                "{cmd} {}\n  rust: {rust}\n  go:   {}",
                e["in"], e["out"]
            ));
        }
    }
    let want: BTreeMap<String, usize> = COUNTS.iter().map(|&(c, n)| (c.to_owned(), n)).collect();
    assert_eq!(seen, want, "{VECTORS_FILE}: entries per command");
    assert!(failed.is_empty(), "{VECTORS_FILE}:\n{}", failed.join("\n"));
    // Both divergences at Go's panics are in the file.
    assert!(panics >= 2, "{panics} Go panics in the file");
    eprintln!(
        "{VECTORS_FILE}: {} entries agree, {panics} of them at Go panics",
        entries.len()
    );
}
