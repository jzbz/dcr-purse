// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrwallet's own `pgpwordlist` and `walletseed`
//! through the Go oracle `tools/oracle/cmd/seed`, and the generator of the
//! frozen vector file.
//!
//! Inputs come from `SplitMix64::from_env`; set `PURSE_TEST_SEED` to vary or
//! reproduce them. Without Go the tests skip (fail under
//! `PURSE_REQUIRE_ORACLE`).

mod common;

use std::fmt::Write as _;

use common::{
    FROZEN_PATH, FROZEN_RNG_SEED, decode_args, generate_sizes, injected_cases, mnemonics_corpus,
    oracle_outcome, rust_decode_mnemonics, rust_decode_user_input, rust_encode, rust_generate,
    rust_generate_injected, user_input_corpus,
};
use purse_errors::MULTILINE_SEPARATOR;
use purse_seed::WORD_LIST;
use purse_seed::walletseed::encode_mnemonic_slice;
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip};

/// Collects mismatches and reports the first few.
#[derive(Default)]
struct Diffs {
    checked: usize,
    failed: usize,
    report: String,
}

impl Diffs {
    fn check(&mut self, what: &str, input: &Value, rust: &Value, go: &Value) {
        self.checked = self.checked.saturating_add(1);
        if rust != go {
            self.failed = self.failed.saturating_add(1);
            if self.failed <= 10 {
                let _ = writeln!(self.report, "{what} {input}\n  rust: {rust}\n  go:   {go}");
            }
        }
    }

    fn finish(self, label: &str) {
        eprintln!("{label}: {} comparisons", self.checked);
        assert!(
            self.failed == 0,
            "{label}: {} of {} differ from Go:\n{}",
            self.failed,
            self.checked,
            self.report
        );
    }
}

fn oracle_decode_user_input(o: &mut Oracle, input: &str, separator: Option<&str>) -> Value {
    oracle_outcome(o.try_call(
        "decode-user-input",
        decode_args("input", json!(input), separator),
    ))
}

fn oracle_decode_mnemonics(o: &mut Oracle, words: &[String], separator: Option<&str>) -> Value {
    oracle_outcome(o.try_call(
        "decode-mnemonics",
        decode_args("words", json!(words), separator),
    ))
}

#[test]
fn wordlist_matches_go() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let go = o.call("wordlist", Value::Null);
    assert_eq!(go, json!(WORD_LIST.to_vec()));
}

/// Random seeds of every length 0..=100 (60 of each legal length 16..=64)
/// through encode and back through both decoders, in Rust and in Go.
#[test]
fn random_seeds_round_trip() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-seed random_seeds_round_trip");
    let mut diffs = Diffs::default();
    for len in 0usize..=100 {
        let count = if (16..=64).contains(&len) { 60 } else { 5 };
        for _ in 0..count {
            let seed = rng.bytes(len);
            let input = json!(hex(&seed));
            let go = o.call("encode", json!({ "seed": hex(&seed) }));
            diffs.check("encode", &input, &rust_encode(&seed), &go);

            let mnemonic = go["mnemonic"].as_str().unwrap_or_default().to_owned();
            let rust = rust_decode_user_input(&mnemonic, None);
            let go = oracle_decode_user_input(&mut o, &mnemonic, None);
            diffs.check("decode-user-input", &json!(mnemonic), &rust, &go);

            let h = hex(&seed);
            let rust = rust_decode_user_input(&h, None);
            let go = oracle_decode_user_input(&mut o, &h, None);
            diffs.check("decode-user-input", &json!(h), &rust, &go);

            let words: Vec<String> = encode_mnemonic_slice(&seed)
                .iter()
                .map(str::to_owned)
                .collect();
            let rust = rust_decode_mnemonics(&words, None);
            let go = oracle_decode_mnemonics(&mut o, &words, None);
            diffs.check("decode-mnemonics", &json!(words), &rust, &go);
        }
    }
    diffs.finish("random_seeds_round_trip");
}

/// The mutation corpus through `DecodeUserInput`, rendered with both
/// separators.
#[test]
fn decode_user_input_mutations() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-seed decode_user_input_mutations");
    let mut diffs = Diffs::default();
    for input in user_input_corpus(&mut rng, 2000) {
        for sep in [None, Some(MULTILINE_SEPARATOR)] {
            let rust = rust_decode_user_input(&input, sep);
            let go = oracle_decode_user_input(&mut o, &input, sep);
            diffs.check("decode-user-input", &json!(input), &rust, &go);
        }
    }
    diffs.finish("decode_user_input_mutations");
}

/// The word-list corpus through `DecodeMnemonics`, rendered with both
/// separators.
#[test]
fn decode_mnemonics_mutations() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-seed decode_mnemonics_mutations");
    let mut diffs = Diffs::default();
    for words in mnemonics_corpus(&mut rng, 1000) {
        for sep in [None, Some(MULTILINE_SEPARATOR)] {
            let rust = rust_decode_mnemonics(&words, sep);
            let go = oracle_decode_mnemonics(&mut o, &words, sep);
            diffs.check("decode-mnemonics", &json!(words), &rust, &go);
        }
    }
    diffs.finish("decode_mnemonics_mutations");
}

/// `GenerateRandomSeed`'s accepted sizes and errors (the bytes themselves
/// are random on both sides).
#[test]
fn generate_random_seed_bounds() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let mut diffs = Diffs::default();
    for size in generate_sizes() {
        let go = oracle_outcome(o.try_call("generate-random-seed", json!({ "size": size })));
        diffs.check(
            "generate-random-seed",
            &json!(size),
            &rust_generate(size),
            &go,
        );
    }
    diffs.finish("generate_random_seed_bounds");
}

/// `GenerateRandomSeed`'s bytes and consumption: the oracle swaps
/// `crypto/rand.Reader` for a counting reader, the Rust side injects the same
/// stream into `generate_random_seed_with`.
#[test]
fn generate_random_seed_injected() {
    let Some(mut o) = oracle_or_skip("seed") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-seed generate_random_seed_injected");
    let mut diffs = Diffs::default();
    for (size, start) in injected_cases(&mut rng) {
        let go = o.call(
            "generate-random-seed-injected",
            json!({ "size": size, "start": start }),
        );
        let input = json!({ "size": size, "start": start });
        diffs.check(
            "generate-random-seed-injected",
            &input,
            &rust_generate_injected(size, start),
            &go,
        );
    }
    diffs.finish("generate_random_seed_injected");
}

/// Writes `tests/data/seed_vectors.json` from the Go oracle. Run it after
/// changing the corpus or moving the parity pin:
///
/// `cargo test -p purse-seed --test oracle regenerate_frozen_vectors -- --ignored`
#[test]
#[ignore = "writes tests/data/seed_vectors.json from the Go oracle"]
fn regenerate_frozen_vectors() {
    let mut o = oracle_or_skip("seed").expect("the oracle is required to regenerate vectors");
    let mut rng = SplitMix64(FROZEN_RNG_SEED);

    let mut seeds: Vec<Vec<u8>> = vec![
        purse_testutil::unhex("e58294f2e9a227486e8b061b31cc528fd7fa3f19"),
        purse_testutil::unhex("d1d464c004f00fb5c9a4c8d8e433e7fb7ff56256"),
        purse_testutil::unhex("e34cd132128c1929ec96865ced5c4d0bf40a5d021fcef58d27dbfee371d210"),
        vec![0; 31],
        vec![0xff; 64],
    ];
    seeds.extend((0usize..=80).map(|len| rng.bytes(len)));
    let encode: Vec<Value> = seeds
        .iter()
        .map(|seed| {
            let mut v = o.call("encode", json!({ "seed": hex(seed) }));
            v["seed"] = json!(hex(seed));
            v
        })
        .collect();

    // A decode entry stores the daemon-separator outcome, and the
    // library-separator one only where it differs.
    let mut decode_entry = |key: &str, input: Value, cmd: &str| {
        let daemon = oracle_outcome(o.try_call(cmd, decode_args(key, input.clone(), None)));
        let library = oracle_outcome(o.try_call(
            cmd,
            decode_args(key, input.clone(), Some(MULTILINE_SEPARATOR)),
        ));
        let mut entry = json!({ key: input, "outcome": daemon });
        if library != entry["outcome"] {
            entry["outcome_library_separator"] = library;
        }
        entry
    };
    let mut decode_user_input = Vec::new();
    for input in user_input_corpus(&mut rng, 250) {
        decode_user_input.push(decode_entry("input", json!(input), "decode-user-input"));
    }
    let mut decode_mnemonics = Vec::new();
    for words in mnemonics_corpus(&mut rng, 120) {
        decode_mnemonics.push(decode_entry("words", json!(words), "decode-mnemonics"));
    }
    let generate: Vec<Value> = generate_sizes()
        .into_iter()
        .map(|size| {
            let outcome =
                oracle_outcome(o.try_call("generate-random-seed", json!({ "size": size })));
            json!({ "size": size, "outcome": outcome })
        })
        .collect();
    let generate_injected: Vec<Value> = injected_cases(&mut rng)
        .into_iter()
        .map(|(size, start)| {
            let args = json!({ "size": size, "start": start });
            let outcome = o.call("generate-random-seed-injected", args);
            json!({ "size": size, "start": start, "outcome": outcome })
        })
        .collect();
    let wordlist = o.call("wordlist", Value::Null);

    let comment = [
        "Frozen vectors for purse-seed, generated by the Go oracle tools/oracle/cmd/seed,",
        "which calls dcrwallet's own pgpwordlist and walletseed packages at the commit below.",
        "Regenerate with:",
        "  cargo test -p purse-seed --test oracle regenerate_frozen_vectors -- --ignored",
        "Inputs come from the corpus in tests/common/mod.rs with a fixed PRNG seed; every",
        "expected value is the oracle's answer. Do not edit by hand.",
        "Error texts use the daemon separator ':: ' (errors.Separator as dcrwallet's main",
        "sets it); outcome_library_separator is the default ':\\n\\t' rendering, present only",
        "where it differs. generate_random_seed records only lengths and errors;",
        "generate_random_seed_injected runs GenerateRandomSeed with crypto/rand.Reader",
        "replaced by a reader serving start, start+1, ... (mod 256) and records the seed and",
        "the bytes served. counts holds each section's entry count.",
    ];
    let mut out = String::from("{\n");
    let line = |v: &Value| serde_json::to_string(v).expect("serialize");
    let _ = writeln!(out, "\"comment\": {},", line(&json!(comment)));
    let _ = writeln!(
        out,
        "\"dcrwallet_commit\": {},",
        line(&json!(DCRWALLET_PARITY_COMMIT))
    );
    let _ = writeln!(out, "\"wordlist\": {},", line(&wordlist));
    let sections: [(&str, &[Value]); 5] = [
        ("encode", &encode),
        ("decode_user_input", &decode_user_input),
        ("decode_mnemonics", &decode_mnemonics),
        ("generate_random_seed", &generate),
        ("generate_random_seed_injected", &generate_injected),
    ];
    let counts: serde_json::Map<String, Value> = sections
        .iter()
        .map(|(name, entries)| ((*name).to_owned(), json!(entries.len())))
        .collect();
    let _ = writeln!(out, "\"counts\": {},", line(&Value::Object(counts)));
    for (i, (name, entries)) in sections.iter().enumerate() {
        let _ = writeln!(out, "\"{name}\": [");
        for (j, e) in entries.iter().enumerate() {
            let comma = if j.saturating_add(1) == entries.len() {
                ""
            } else {
                ","
            };
            let _ = writeln!(out, "{}{comma}", line(e));
        }
        let comma = if i.saturating_add(1) == sections.len() {
            ""
        } else {
            ","
        };
        let _ = writeln!(out, "]{comma}");
    }
    out.push_str("}\n");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FROZEN_PATH);
    std::fs::write(&path, out).expect("write frozen vectors");
    eprintln!("wrote {}", path.display());
}
