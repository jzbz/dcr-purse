// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Replays `tests/data/seed_vectors.json`, vectors generated from the Go
//! oracle (see the file's header), so the crate keeps pinned coverage on a
//! machine without Go.

mod common;

use common::{
    FROZEN_PATH, rust_decode_mnemonics, rust_decode_user_input, rust_encode, rust_generate,
    rust_generate_injected,
};
use purse_errors::MULTILINE_SEPARATOR;
use purse_seed::WORD_LIST;
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, unhex};

fn vectors() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FROZEN_PATH);
    let text = std::fs::read_to_string(&path).expect("read frozen vectors");
    serde_json::from_str(&text).expect("parse frozen vectors")
}

/// A section's entries, checked against the count the generator recorded.
fn entries<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    let entries = v[key].as_array().map(Vec::as_slice).unwrap_or_default();
    let count = v["counts"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("no recorded count for {key}"));
    assert_eq!(
        u64::try_from(entries.len()).ok(),
        Some(count),
        "{key}: entry count differs from the recorded one"
    );
    assert!(!entries.is_empty(), "no {key} vectors");
    entries
}

#[test]
fn pinned_commit_and_wordlist() {
    let v = vectors();
    assert_eq!(v["dcrwallet_commit"], json!(DCRWALLET_PARITY_COMMIT));
    assert_eq!(v["wordlist"], json!(WORD_LIST.to_vec()));
}

#[test]
fn encode() {
    let v = vectors();
    for e in entries(&v, "encode") {
        let seed = unhex(e["seed"].as_str().expect("seed"));
        let mut rust = rust_encode(&seed);
        rust["seed"] = e["seed"].clone();
        assert_eq!(&rust, e);
    }
}

/// Checks one decode entry with both separators.
fn check_decode(e: &Value, decode: impl Fn(Option<&str>) -> Value) {
    assert_eq!(decode(None), e["outcome"], "{e}");
    let library = e.get("outcome_library_separator").unwrap_or(&e["outcome"]);
    assert_eq!(&decode(Some(MULTILINE_SEPARATOR)), library, "{e}");
}

#[test]
fn decode_user_input() {
    let v = vectors();
    for e in entries(&v, "decode_user_input") {
        let input = e["input"].as_str().expect("input");
        check_decode(e, |sep| rust_decode_user_input(input, sep));
    }
}

#[test]
fn decode_mnemonics() {
    let v = vectors();
    for e in entries(&v, "decode_mnemonics") {
        let words: Vec<String> = serde_json::from_value(e["words"].clone()).expect("words");
        check_decode(e, |sep| rust_decode_mnemonics(&words, sep));
    }
}

#[test]
fn generate_random_seed() {
    let v = vectors();
    for e in entries(&v, "generate_random_seed") {
        let size = e["size"].as_u64().expect("size");
        assert_eq!(rust_generate(size), e["outcome"], "{e}");
    }
}

#[test]
fn generate_random_seed_injected() {
    let v = vectors();
    for e in entries(&v, "generate_random_seed_injected") {
        let size = e["size"].as_u64().expect("size");
        let start = e["start"]
            .as_u64()
            .and_then(|s| u8::try_from(s).ok())
            .expect("start");
        assert_eq!(rust_generate_injected(size, start), e["outcome"], "{e}");
    }
}

#[test]
fn every_section_is_counted() {
    let v = vectors();
    let counts = v["counts"].as_object().expect("counts");
    let sections = [
        "encode",
        "decode_user_input",
        "decode_mnemonics",
        "generate_random_seed",
        "generate_random_seed_injected",
    ];
    assert_eq!(counts.len(), sections.len());
    for key in sections {
        entries(&v, key);
    }
}
