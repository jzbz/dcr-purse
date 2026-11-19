// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays the FLINT vectors `tests/data/flint_vectors.jsonl` without Go or
//! FLINT, by the comparison rule of `tests/flint_cases/mod.rs`, on every
//! platform.
//!
//! The file was written by `regenerate_flint_vectors` (`tests/oracle.rs`)
//! from the oracle built with FLINT; its header names the FLINT release,
//! the `decred.org/cspp/v2` version and the Go it was built with. The replay
//! refuses a file that is truncated, edited or stale: the body hash and the
//! entry count in the header must match, the dcrwallet commit must be the
//! parity pin, and the inputs must be exactly the corpus the generator
//! produces today from the fixed seed, in order (so a change to the corpus
//! without regenerating fails here, not silently).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

mod common;
mod flint_cases;

use std::collections::BTreeMap;
use std::path::PathBuf;

use dcroxide_crypto::blake256;
use flint_cases::{
    CATEGORIES, FROZEN_PATH, FROZEN_PLAN, FROZEN_SEED, Tally, Verdict, cases, check,
    parse_frozen_line,
};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, SplitMix64, hex};

#[test]
fn flint_vectors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FROZEN_PATH);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {FROZEN_PATH}: {e} (it is checked in)"));
    let header = |key: &str| {
        text.lines()
            .filter(|l| l.starts_with('#'))
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{FROZEN_PATH}: no {key:?} header"))
            .to_owned()
    };
    assert_eq!(
        header("#   dcrwallet"),
        format!("{DCRWALLET_PARITY_COMMIT}."),
        "{FROZEN_PATH} names another dcrwallet commit"
    );
    assert!(
        text.contains("decred.org/cspp/v2 v2.4.0, FLINT "),
        "{FROZEN_PATH}: the header does not name cspp v2.4.0 and a FLINT release"
    );
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        header("# body-blake256:"),
        hex(&blake256::sum256(body.join("\n").as_bytes())),
        "{FROZEN_PATH}: body hash"
    );
    assert_eq!(
        header("# entries:").parse::<usize>().ok(),
        Some(body.len()),
        "{FROZEN_PATH}: entry count"
    );

    let expected = cases(&mut SplitMix64(FROZEN_SEED), FROZEN_PLAN);
    assert_eq!(
        body.len(),
        expected.len(),
        "{FROZEN_PATH} holds {} entries, the corpus has {}: regenerate it",
        body.len(),
        expected.len()
    );
    let mut tally = Tally::default();
    let mut per_category: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, (line, case)) in body.iter().zip(&expected).enumerate() {
        let (category, coeffs, go) = parse_frozen_line(line);
        assert!(
            category == case.category && coeffs == case.coeffs,
            "{FROZEN_PATH}: entry {i} ({category}) is not the corpus's ({}): regenerate it",
            case.category
        );
        let checked = check(case, &go);
        tally.add(case, &go, &checked);
        *per_category.entry(category).or_default() += 1;
    }
    for &(category, _) in &CATEGORIES {
        assert!(
            category == "max-degree" || per_category.get(category).is_some_and(|&n| n > 0),
            "{FROZEN_PATH}: no {category} vectors"
        );
    }
    assert!(tally.count(Verdict::SameAccept) > 0 && tally.count(Verdict::SameReject) > 0);
    eprintln!(
        "{FROZEN_PATH}: {} vectors checked: {} accepted alike, {} refused alike, {} documented \
         divergences",
        body.len(),
        tally.count(Verdict::SameAccept),
        tally.count(Verdict::SameReject),
        tally.count(Verdict::Divergence),
    );
    for line in tally.lines() {
        eprintln!("  {line}");
    }
}
