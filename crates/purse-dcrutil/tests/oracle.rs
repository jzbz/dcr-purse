// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrd's `dcrutil/v4` v4.0.3 through the Go
//! oracle `tools/oracle/cmd/dcrutil`, and the generator of the frozen
//! vector files.
//!
//! Inputs come from `SplitMix64::from_env`; set `PURSE_TEST_SEED` to vary or
//! reproduce them, and `PURSE_DCRUTIL_SCALE` (default 10) to size the
//! random parts of the corpora. Without Go the tests skip (fail under
//! `PURSE_REQUIRE_ORACLE`).
//!
//! # Architecture
//!
//! Go's `NewAmount` and `MulF64` give different answers on amd64 and arm64
//! (see `src/amount.rs`), and purse-dcrutil gives amd64's on every target.
//! So every comparison here is made against a model of Go on the oracle's
//! own architecture (`common::Arch`): the test checks that Go's answer is
//! the model's for that architecture (which pins down the model, and the
//! claim about Go, on each CI runner), and that the crate's answer is the
//! amd64 model's. On amd64 the two checks together are a direct comparison
//! with Go; on arm64 the inputs whose answers differ are counted and
//! reported, and the test requires the corpus to contain some, so the
//! arm64 runner exercises the difference rather than skipping it. Any
//! other architecture fails: there is no model for it. That includes
//! amd64 at GOAMD64 v3 or v4, where Go fuses the multiply-add of the
//! non-negative branch (`VFMADD231SD`) and so matches neither model, and
//! riscv64, 386 and arm, whose answers `src/amount.rs` describes but no
//! model here computes.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and lengths of small, bounded values"
)]

mod common;

use std::fmt::Write as _;

use common::*;
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, oracle_or_skip};

/// Collects mismatches and reports the first few.
#[derive(Default)]
struct Diffs {
    checked: usize,
    failed: usize,
    report: String,
}

impl Diffs {
    fn check(&mut self, what: &str, input: &Value, got: &Value, want: &Value) {
        self.checked = self.checked.saturating_add(1);
        if got != want {
            self.failed = self.failed.saturating_add(1);
            if self.failed <= 10 {
                let _ = writeln!(self.report, "{what} {input}\n  got:  {got}\n  want: {want}");
            }
        }
    }

    fn finish(self, label: &str) {
        eprintln!("{label}: {} comparisons", self.checked);
        assert!(
            self.failed == 0,
            "{label}: {} of {} differ:\n{}",
            self.failed,
            self.checked,
            self.report
        );
    }
}

fn scale() -> usize {
    std::env::var("PURSE_DCRUTIL_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
}

const BATCH: usize = 20_000;

/// Sends `items` under `key` in batches and returns Go's answers in order.
fn call_batched(o: &mut Oracle, cmd: &str, key: &str, items: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(items.len());
    for chunk in batches(items, BATCH) {
        let answer = o.call(cmd, json!({ key: chunk }));
        let answers = answer.as_array().expect("an array answer");
        assert_eq!(answers.len(), chunk.len(), "{cmd}: answer count");
        out.extend(answers.iter().cloned());
    }
    out
}

fn new_amount_inputs(fs: &[f64]) -> Vec<Value> {
    fs.iter().map(|&f| json!(bits_hex(f))).collect()
}

#[test]
fn constants_and_texts_match_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let c = o.call("consts", Value::Null);
    assert_eq!(c["atoms_per_cent"], json!(purse_dcrutil::ATOMS_PER_CENT));
    assert_eq!(c["atoms_per_coin"], json!(purse_dcrutil::ATOMS_PER_COIN));
    assert_eq!(c["max_amount"], json!(purse_dcrutil::MAX_AMOUNT));
    use purse_dcrutil::AmountUnit as U;
    let units = [
        U::MEGA_COIN,
        U::KILO_COIN,
        U::COIN,
        U::MILLI_COIN,
        U::MICRO_COIN,
        U::ATOM,
    ];
    let values: Vec<i64> = units.iter().map(|u| u.0).collect();
    let names: Vec<String> = units.iter().map(ToString::to_string).collect();
    assert_eq!(c["unit_values"], json!(values));
    assert_eq!(c["unit_names"], json!(names));
    use purse_dcrutil::SignatureType as St;
    assert_eq!(
        c["signature_types"],
        json!([
            St::ECDSA_SECP256K1.0,
            St::ED25519.0,
            St::SCHNORR_SECP256K1.0
        ])
    );
    assert_eq!(
        c["err_amount"],
        json!(purse_dcrutil::InvalidCoinAmount.to_string())
    );
    use purse_dcrutil::WifError as E;
    assert_eq!(
        c["err_malformed"],
        json!(E::MalformedPrivateKey.to_string())
    );
    assert_eq!(c["err_checksum"], json!(E::ChecksumMismatch.to_string()));
    assert_eq!(
        c["err_wrong_net_sample"],
        json!(E::WrongNetwork([0x22, 0xde]).to_string())
    );
    for (name, id) in NETS {
        assert_eq!(
            c["private_key_ids"][name],
            json!(purse_testutil::hex(&id)),
            "{name}"
        );
    }
}

#[test]
fn new_amount_matches_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let arch = Arch::of(&mut o);
    let mut rng = SplitMix64::from_env("dcrutil-new-amount");
    let fs = new_amount_corpus(&mut rng, scale());
    let go = call_batched(&mut o, "amount.new", "bits", &new_amount_inputs(&fs));
    let mut diffs = Diffs::default();
    let mut arch_dependent = 0usize;
    let mut fused_only = 0usize;
    for (f, go) in fs.iter().zip(&go) {
        let input = json!(bits_hex(*f));
        diffs.check("Go vs its model", &input, go, &go_new_amount(arch, *f));
        diffs.check(
            "Rust vs amd64 Go",
            &input,
            &rust_new_amount(*f),
            &go_new_amount(Arch::Amd64, *f),
        );
        if arch == Arch::Amd64 {
            diffs.check("Rust vs Go", &input, &rust_new_amount(*f), go);
        }
        if go_new_amount(Arch::Amd64, *f) != go_new_amount(Arch::Arm64, *f) {
            arch_dependent += 1;
        }
        if f.is_finite() {
            fused_only += usize::from(in_range_difference(
                go_round_product(Arch::Amd64, *f, 1e8),
                go_round_product(Arch::Arm64, *f, 1e8),
            ));
        }
    }
    eprintln!(
        "NewAmount on {arch:?}: {arch_dependent} inputs where amd64 and arm64 Go differ, \
         {fused_only} of them in range (the fused rounding alone)"
    );
    assert!(
        arch_dependent >= 50,
        "the corpus lost its architecture-dependent inputs"
    );
    assert!(
        fused_only >= 10,
        "the corpus lost its in-range fused-rounding inputs"
    );
    diffs.finish("NewAmount");
}

#[test]
fn mul_f64_matches_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let arch = Arch::of(&mut o);
    let mut rng = SplitMix64::from_env("dcrutil-mulf64");
    let atoms = atoms_corpus(&mut rng, scale());
    let cases = mul_corpus(&mut rng, &atoms, scale());
    let inputs: Vec<Value> = cases
        .iter()
        .map(|&(a, f)| json!([a, bits_hex(f)]))
        .collect();
    let go = call_batched(&mut o, "amount.mulf64", "cases", &inputs);
    let mut diffs = Diffs::default();
    let mut arch_dependent = 0usize;
    let mut fused_only = 0usize;
    for ((&(a, f), input), go) in cases.iter().zip(&inputs).zip(&go) {
        diffs.check("Go vs its model", input, go, &json!(go_mul_f64(arch, a, f)));
        let rust = rust_mul_f64(a, f);
        diffs.check(
            "Rust vs amd64 Go",
            input,
            &rust,
            &json!(go_mul_f64(Arch::Amd64, a, f)),
        );
        if arch == Arch::Amd64 {
            diffs.check("Rust vs Go", input, &rust, go);
        }
        if go_mul_f64(Arch::Amd64, a, f) != go_mul_f64(Arch::Arm64, a, f) {
            arch_dependent += 1;
        }
        fused_only += usize::from(in_range_difference(
            go_mul_f64(Arch::Amd64, a, f),
            go_mul_f64(Arch::Arm64, a, f),
        ));
    }
    eprintln!(
        "MulF64 on {arch:?}: {arch_dependent} inputs where amd64 and arm64 Go differ, \
         {fused_only} of them in range (the fused rounding alone)"
    );
    assert!(
        arch_dependent >= 25,
        "the corpus lost its architecture-dependent inputs"
    );
    assert!(
        fused_only >= 10,
        "the corpus lost its in-range fused-rounding inputs"
    );
    diffs.finish("MulF64");
}

#[test]
fn to_coin_and_string_match_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let arch = Arch::of(&mut o);
    let mut rng = SplitMix64::from_env("dcrutil-coin");
    let atoms = atoms_corpus(&mut rng, scale());
    let inputs: Vec<Value> = atoms.iter().map(|&a| json!(a)).collect();
    let go = call_batched(&mut o, "amount.coin", "atoms", &inputs);
    let mut diffs = Diffs::default();
    for ((&a, input), go) in atoms.iter().zip(&inputs).zip(&go) {
        let rust = rust_coin(a);
        diffs.check(
            "ToCoin/String",
            input,
            go,
            &go_quotient_answer(arch, &rust, "to_coin"),
        );
    }
    diffs.finish("ToCoin and String");
}

#[test]
fn format_and_to_unit_match_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let arch = Arch::of(&mut o);
    let mut rng = SplitMix64::from_env("dcrutil-format");
    let atoms = atoms_corpus(&mut rng, scale());
    let cases = format_corpus(&mut rng, &atoms, &units_corpus());
    let inputs: Vec<Value> = cases.iter().map(|&(a, u)| json!([a, u])).collect();
    let go = call_batched(&mut o, "amount.format", "cases", &inputs);
    let mut diffs = Diffs::default();
    for ((&(a, u), input), go) in cases.iter().zip(&inputs).zip(&go) {
        let rust = rust_format(a, u);
        diffs.check(
            "Format/ToUnit",
            input,
            go,
            &go_quotient_answer(arch, &rust, "to_unit"),
        );
    }
    diffs.finish("Format and ToUnit");
}

#[test]
fn unit_names_match_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let mut rng = SplitMix64::from_env("dcrutil-units");
    let units = unit_names_corpus(&mut rng);
    let inputs: Vec<Value> = units.iter().map(|&u| json!(u)).collect();
    let go = call_batched(&mut o, "amount.unit", "units", &inputs);
    let mut diffs = Diffs::default();
    for ((&u, input), go) in units.iter().zip(&inputs).zip(&go) {
        diffs.check("AmountUnit.String", input, &rust_unit(u), go);
    }
    diffs.finish("AmountUnit.String");
}

#[test]
fn wif_new_matches_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let mut rng = SplitMix64::from_env("dcrutil-wif-new");
    let cases = wif_new_corpus(&mut rng, scale());
    let go = call_batched(&mut o, "wif.new", "cases", &cases);
    let mut diffs = Diffs::default();
    let mut decode_back = Vec::new();
    for (case, go) in cases.iter().zip(&go) {
        diffs.check("NewWIF", case, &rust_wif_new(case), go);
        if let Some(s) = go["string"].as_str() {
            decode_back.push(json!({ "wif": s, "net": case["net"] }));
        }
    }
    // Every WIF made decodes the same way on both sides (a key that is not
    // 32 bytes gives a string DecodeWIF refuses).
    let go = call_batched(&mut o, "wif.decode", "cases", &decode_back);
    for (case, go) in decode_back.iter().zip(&go) {
        diffs.check("DecodeWIF(NewWIF)", case, &rust_wif_decode(case), go);
    }
    diffs.finish("NewWIF");
}

#[test]
fn wif_decode_matches_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let mut rng = SplitMix64::from_env("dcrutil-wif-decode");
    let cases = wif_decode_corpus(&mut rng, scale());
    let go = call_batched(&mut o, "wif.decode", "cases", &cases);
    let mut diffs = Diffs::default();
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    for (case, go) in cases.iter().zip(&go) {
        diffs.check("DecodeWIF", case, &rust_wif_decode(case), go);
        let key = go["error"].as_str().map_or_else(
            || format!("ok dsa {}", go["dsa"]),
            |e| e.split(" 0x").next().unwrap_or(e).to_owned(),
        );
        *outcomes.entry(key).or_default() += 1;
    }
    eprintln!("DecodeWIF outcomes: {outcomes:?}");
    // Every outcome DecodeWIF has is in the corpus.
    for want in [
        "malformed private key",
        "checksum mismatch",
        "WIF is not for the network identified by",
        "not on subgroup (>N)",
        "zero or negative scalar",
        "ok dsa 0",
        "ok dsa 1",
        "ok dsa 2",
    ] {
        assert!(outcomes.contains_key(want), "no {want:?} case");
    }
    diffs.finish("DecodeWIF");
}

#[test]
fn txsort_serialized_matches_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let mut rng = SplitMix64::from_env("dcrutil-txsort-hex");
    let txs: Vec<Value> = txsort_hex_corpus(&mut rng, 50 * scale(), 20)
        .into_iter()
        .map(Value::String)
        .collect();
    let go = call_batched(&mut o, "txsort.hex", "txs", &txs);
    let mut diffs = Diffs::default();
    for (tx, go) in txs.iter().zip(&go) {
        let rust = rust_txsort_hex(tx.as_str().expect("hex"));
        diffs.check("txsort", tx, &rust, go);
    }
    diffs.finish("txsort (serialized)");
}

#[test]
fn txsort_fields_match_go() {
    let Some(mut o) = oracle_or_skip("dcrutil") else {
        return;
    };
    let mut rng = SplitMix64::from_env("dcrutil-txsort-fields");
    let txs = txsort_fields_corpus(&mut rng, scale());
    let go = call_batched(&mut o, "txsort.fields", "txs", &txs);
    let mut diffs = Diffs::default();
    let mut tie_orders = 0usize;
    for (tx, go) in txs.iter().zip(&go) {
        diffs.check("txsort", tx, &rust_txsort_fields(tx), go);
        assert_eq!(
            go["sort_agrees"],
            json!(true),
            "Go's Sort and InPlaceSort disagree"
        );
        // Count answers in which equal inputs ended up out of their input
        // order: where a stable sort would have differed from Go's.
        let inputs = go["inputs"].as_array().expect("inputs");
        tie_orders += usize::from(inputs.windows(2).any(|w| {
            w[0]["hash"] == w[1]["hash"]
                && w[0]["tree"] == w[1]["tree"]
                && w[0]["index"] == w[1]["index"]
                && w[0]["sequence"].as_u64() > w[1]["sequence"].as_u64()
        }));
    }
    eprintln!("txsort: {tie_orders} transactions where Go reordered equal inputs");
    assert!(
        tie_orders > 10,
        "no transaction shows sort.Sort's tie order"
    );
    diffs.finish("txsort (fields)");
}

// ---------------------------------------------------------------------
// Frozen vector generation.
// ---------------------------------------------------------------------

/// `text` as `# ` comment lines of at most 78 characters.
fn comment(text: &str) -> String {
    let mut out = String::new();
    let mut line = String::from("#");
    for word in text.split_whitespace() {
        if line.len() + 1 + word.len() > 78 && line.len() > 1 {
            out.push_str(&line);
            out.push('\n');
            line = String::from("#");
        }
        line.push(' ');
        line.push_str(word);
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// The upstream notice the txsort file carries: its `txsort.hex` entries
/// include dcrd's txsort testdata byte for byte.
fn txsort_testdata_notice() -> String {
    format!(
        "#\n{}#\n#     Copyright (c) 2015-2016 The btcsuite developers\n#     \
         Copyright (c) 2017-2021 The Decred developers\n",
        comment(
            "The txsort.hex entries include, byte for byte, the transactions of dcrd's \
             github.com/decred/dcrd/dcrutil/v4 v4.0.3 txsort/testdata (also in \
             tests/data/txsort; see its README.md), distributed under dcrd's ISC license:"
        )
    )
}

/// Header lines common to every frozen file, with `notice` (comment lines,
/// or empty) after the description.
fn header(title: &str, recipe: &str, notice: &str, entries_body: &str) -> String {
    let about = format!(
        "Every \"out\" is the answer of the Go oracle tools/oracle/cmd/dcrutil \
         (github.com/decred/dcrd/dcrutil/v4 v4.0.3 and its txsort package, as linked by \
         dcrwallet {}), built for amd64 (GOAMD64 v1): where Go's answer depends on the \
         architecture, these are amd64's, which purse-dcrutil gives on every target. \
         Corpus: {recipe}. Written by `PURSE_DCRUTIL_REGEN=1 cargo test -p purse-dcrutil \
         --test oracle -- --ignored regenerate_frozen_vectors`, which also checked the Rust \
         crate equal to every answer. Each line: {{\"cmd\": oracle command, \"in\": one \
         element of its batch, \"out\": Go's answer to it}}; txsort.tagged is the compact \
         form of txsort.fields for a tagged transaction (tests/common/mod.rs `tagged_tx`, \
         `tagged_answer`), projected from Go's txsort.fields answer.",
        &DCRWALLET_PARITY_COMMIT[..12]
    );
    format!(
        "# {title}\n#\n{}{notice}{}",
        comment(&about),
        integrity_header(entries_body)
    )
}

/// Queries Go for one command over `inputs`, checks the crate agrees, and
/// appends the entries to `body`.
fn freeze(o: &mut Oracle, body: &mut String, cmd: &str, key: &str, inputs: &[Value]) -> usize {
    let go = call_batched(o, cmd, key, inputs);
    for (input, out) in inputs.iter().zip(&go) {
        push_entry(body, cmd, input, out);
    }
    inputs.len()
}

/// Checks the crate's answer to one entry and appends the entry.
fn push_entry(body: &mut String, cmd: &str, input: &Value, out: &Value) {
    let rust = rust_answer(cmd, input);
    assert_eq!(
        &rust, out,
        "{cmd} {input}: the crate differs from Go; not freezing"
    );
    let line = json!({ "cmd": cmd, "in": input, "out": out });
    body.push_str(&line.to_string());
    body.push('\n');
}

fn write_file(name: &str, title: &str, recipe: &str, notice: &str, body: &str) {
    let text = format!("{}{body}", header(title, recipe, notice, body));
    std::fs::write(data_dir().join(name), text).expect("write frozen vectors");
}

#[test]
#[ignore = "regenerates tests/data; run with PURSE_DCRUTIL_REGEN=1"]
fn regenerate_frozen_vectors() {
    assert!(
        std::env::var_os("PURSE_DCRUTIL_REGEN").is_some(),
        "set PURSE_DCRUTIL_REGEN=1 to rewrite tests/data"
    );
    let mut o = oracle_or_skip("dcrutil").expect("regeneration needs the Go oracle");
    assert_eq!(
        Arch::of(&mut o),
        Arch::Amd64,
        "the frozen vectors pin amd64's answers; regenerate on amd64"
    );

    // Amounts.
    let mut rng = SplitMix64(AMOUNT_SEED);
    let mut body = String::new();
    let mut counts = Vec::new();
    let fs = new_amount_corpus(&mut rng, 1);
    let mut picked: Vec<f64> = fs.iter().take(26).copied().collect();
    picked.extend(fs.iter().skip(26).step_by(8));
    picked.extend(fs.iter().rev().take(100));
    counts.push(freeze(
        &mut o,
        &mut body,
        "amount.new",
        "bits",
        &new_amount_inputs(&picked),
    ));
    let atoms = atoms_corpus(&mut rng, 0);
    let inputs: Vec<Value> = atoms.iter().map(|&a| json!(a)).collect();
    counts.push(freeze(&mut o, &mut body, "amount.coin", "atoms", &inputs));
    let units: Vec<i64> = units_corpus()
        .into_iter()
        .enumerate()
        .filter(|&(i, u)| (-12..=20).contains(&u) || !(-340..=320).contains(&u) || i % 7 == 0)
        .map(|(_, u)| u)
        .collect();
    let named = [6i64, 3, 0, -3, -6, -8];
    let mut cases: Vec<Value> = units
        .iter()
        .map(|&u| json!([atoms[rng.below(atoms.len() as u64) as usize], u]))
        .collect();
    for &a in atoms.iter().step_by(2) {
        cases.push(json!([a, named[rng.below(named.len() as u64) as usize]]));
    }
    counts.push(freeze(&mut o, &mut body, "amount.format", "cases", &cases));
    let muls: Vec<Value> = mul_corpus(&mut rng, &atoms, 1)
        .into_iter()
        .step_by(3)
        .map(|(a, f)| json!([a, bits_hex(f)]))
        .collect();
    counts.push(freeze(&mut o, &mut body, "amount.mulf64", "cases", &muls));
    let names: Vec<Value> = unit_names_corpus(&mut rng)
        .into_iter()
        .map(|u| json!(u))
        .collect();
    counts.push(freeze(&mut o, &mut body, "amount.unit", "units", &names));
    write_file(
        AMOUNT_FILE,
        "Amount: NewAmount, MulF64, ToCoin and String, ToUnit and Format, AmountUnit.String.",
        &format!(
            "tests/common/mod.rs with SplitMix64({AMOUNT_SEED:#x}): new_amount_corpus(1) (its \
             specials, every eighth of the rest and its last 100, the fused multiply-add \
             cases), atoms_corpus(0), the units of units_corpus from -12 to 20, beyond -340 \
             and 320, and every seventh other, each with a random amount, every other amount \
             in a named unit, every third of mul_corpus(1), unit_names_corpus; entries per \
             command (new, coin, format, mulf64, unit) {counts:?}"
        ),
        "",
        &body,
    );

    // WIFs.
    let mut rng = SplitMix64(WIF_SEED);
    let mut body = String::new();
    let mut counts = vec![freeze(
        &mut o,
        &mut body,
        "wif.new",
        "cases",
        &wif_new_corpus(&mut rng, 0),
    )];
    counts.push(freeze(
        &mut o,
        &mut body,
        "wif.decode",
        "cases",
        &wif_decode_corpus(&mut rng, 0),
    ));
    write_file(
        WIF_FILE,
        "WIF: NewWIF and DecodeWIF.",
        &format!(
            "tests/common/mod.rs with SplitMix64({WIF_SEED:#x}): wif_new_corpus(0), \
             wif_decode_corpus(0) (its 54-character block included); entries per command (new, decode) {counts:?}"
        ),
        "",
        &body,
    );

    // txsort.
    let mut rng = SplitMix64(TXSORT_SEED);
    let mut body = String::new();
    let mut counts = Vec::new();
    let txs: Vec<Value> = txsort_hex_corpus(&mut rng, 20, 8)
        .into_iter()
        .map(Value::String)
        .collect();
    counts.push(freeze(&mut o, &mut body, "txsort.hex", "txs", &txs));
    let fields = random_fields_txs(&mut rng, 20, 8);
    counts.push(freeze(&mut o, &mut body, "txsort.fields", "txs", &fields));
    let keys: Vec<Vec<u32>> = sort_key_corpus(&mut rng, 1)
        .into_iter()
        .filter(|k| k.len() <= 257)
        .collect();
    let tagged: Vec<Value> = keys.iter().map(|k| tx_fields(&tagged_tx(k))).collect();
    let go = call_batched(&mut o, "txsort.fields", "txs", &tagged);
    for (k, out) in keys.iter().zip(&go) {
        push_entry(&mut body, "txsort.tagged", &json!(k), &tagged_answer(out));
    }
    counts.push(keys.len());
    write_file(
        TXSORT_FILE,
        "txsort: Sort, InPlaceSort and IsSorted.",
        &format!(
            "tests/common/mod.rs with SplitMix64({TXSORT_SEED:#x}): txsort_hex_corpus(20, 8) \
             (the upstream testdata and 20 random transactions), random_fields_txs(20, 8), \
             the key sequences of sort_key_corpus(1) of at most 257 keys (its \
             shortestShifting block of 49 and 50 keys with ties included) as tagged \
             transactions; entries per command (hex, fields, tagged) {counts:?}"
        ),
        &txsort_testdata_notice(),
        &body,
    );
}
