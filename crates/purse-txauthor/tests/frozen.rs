// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays the frozen vectors in `tests/data`, generated from the Go oracle
//! (dcrwallet `wallet/txauthor` at the parity pin) by
//! `regenerate_frozen_vectors` in `tests/oracle.rs`, so the comparison with
//! Go is pinned on machines without Go. No expected value in the file was
//! written by hand.
//!
//! Nothing here depends on the architecture: the loop is integer
//! arithmetic, the randomization draws from a ChaCha20 stream, and the
//! signatures are deterministic.

#![allow(clippy::arithmetic_side_effects, reason = "test code: counters")]

mod common;

use std::collections::BTreeMap;

use common::*;

/// The entries the file must hold per command (checked against the body,
/// whose size and hash the file's header pins).
const COUNTS: &[(&str, usize)] = &[("author", 308), ("randomize", 60), ("sign", 249)];

#[test]
fn frozen_vectors_agree() {
    let entries = read_vectors(VECTORS_FILE);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let (mut panics, mut crashes) = (0, 0);
    let mut failed = Vec::new();
    for e in &entries {
        let cmd = e["cmd"].as_str().expect("cmd");
        *seen.entry(cmd.to_owned()).or_default() += 1;
        let out = &e["out"];
        let calls = out["results"]
            .as_array()
            .map_or(0, |r| r.iter().filter(|c| c.get("panic").is_some()).count());
        panics += usize::from(out.get("panic").is_some()) + calls;
        crashes += usize::from(out.get("crash").is_some());
        let rust = rust_answer(cmd, &e["in"]);
        if &rust != out && failed.len() < 10 {
            failed.push(format!("{cmd} {}\n  rust: {rust}\n  go:   {out}", e["in"]));
        }
    }
    let want: BTreeMap<String, usize> = COUNTS.iter().map(|&(c, n)| (c.to_owned(), n)).collect();
    assert_eq!(seen, want, "{VECTORS_FILE}: entries per command");
    assert!(failed.is_empty(), "{VECTORS_FILE}:\n{}", failed.join("\n"));
    // Every divergence at a Go panic or crash is in the file.
    assert!(
        panics >= 3 && crashes >= 1,
        "{panics} Go panics, {crashes} crashes"
    );
    eprintln!(
        "{VECTORS_FILE}: {} entries agree, {panics} at Go panics, {crashes} at Go crashes",
        entries.len()
    );
}
