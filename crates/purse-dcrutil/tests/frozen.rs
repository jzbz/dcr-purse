// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays the frozen vectors in `tests/data`, generated from the Go oracle
//! (dcrd dcrutil v4.0.3) by `regenerate_frozen_vectors` in `tests/oracle.rs`,
//! so the comparison with Go is pinned on machines without Go. No expected
//! value in these files was written by hand.
//!
//! The answers are amd64 Go's. The crate gives amd64's answers on every
//! target, so these files hold on every platform.

#![allow(clippy::arithmetic_side_effects, reason = "test code: counters")]

mod common;

use std::collections::BTreeMap;

use common::*;

/// The entries each file must hold per command (checked against the body,
/// whose size and hash the file's header pins).
const AMOUNT_COUNTS: &[(&str, usize)] = &[
    ("amount.coin", 506),
    ("amount.format", 388),
    ("amount.mulf64", 477),
    ("amount.new", 1960),
    ("amount.unit", 878),
];
const WIF_COUNTS: &[(&str, usize)] = &[("wif.decode", 816), ("wif.new", 587)];
const TXSORT_COUNTS: &[(&str, usize)] = &[
    ("txsort.fields", 20),
    ("txsort.hex", 29),
    ("txsort.tagged", 199),
];

fn replay(file: &str, counts: &[(&str, usize)]) {
    let entries = read_vectors(file);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = Vec::new();
    for e in &entries {
        let cmd = e["cmd"].as_str().expect("cmd");
        *seen.entry(cmd.to_owned()).or_default() += 1;
        let rust = rust_answer(cmd, &e["in"]);
        if rust != e["out"] && failed.len() < 10 {
            failed.push(format!(
                "{cmd} {}\n  rust: {rust}\n  go:   {}",
                e["in"], e["out"]
            ));
        }
    }
    let want: BTreeMap<String, usize> = counts.iter().map(|&(c, n)| (c.to_owned(), n)).collect();
    assert_eq!(seen, want, "{file}: entries per command");
    assert!(failed.is_empty(), "{file}:\n{}", failed.join("\n"));
    eprintln!("{file}: {} entries agree", entries.len());
}

#[test]
fn amount_vectors() {
    replay(AMOUNT_FILE, AMOUNT_COUNTS);
}

#[test]
fn wif_vectors() {
    replay(WIF_FILE, WIF_COUNTS);
}

#[test]
fn txsort_vectors() {
    replay(TXSORT_FILE, TXSORT_COUNTS);
}
