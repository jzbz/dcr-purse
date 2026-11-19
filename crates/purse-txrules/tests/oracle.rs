// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrwallet's `wallet/txsizes` and
//! `wallet/txrules` through the Go oracle `tools/oracle/cmd/txrules`, and
//! the generator of the frozen vector file.
//!
//! Inputs come from `SplitMix64::from_env`; set `PURSE_TEST_SEED` to vary or
//! reproduce them, and `PURSE_TXRULES_SCALE` (default 10) to size the random
//! parts of the corpora. Without Go the tests skip (fail under
//! `PURSE_REQUIRE_ORACLE`).
//!
//! Where the crate deliberately does not panic and Go does, `common`'s
//! `rust_answer` reports Go's panic text after checking the crate's own
//! answer, so these comparisons require the divergence at exactly Go's
//! panics (see `tests/common/mod.rs`).
//!
//! # Architecture
//!
//! `int64(math.Floor(poolFee * 100))` of a NaN or out-of-range percentage
//! is architecture-dependent in Go; the crate gives amd64's answer
//! everywhere (ADR-0012 Rule 9(a)). Against an amd64 oracle every case is
//! compared; against an arm64 oracle the VSP-fee cases with such a
//! percentage are excluded and counted, and `poolfee_int_model` checks
//! arm64's conversion separately. Any other architecture fails.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and lengths of small, bounded values"
)]

mod common;

use std::fmt::Write as _;

use common::*;
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, oracle_or_skip};
use purse_txrules::txsizes::*;
use purse_txrules::{DEFAULT_RELAY_FEE_PER_KB, stake_pool_ticket_fee};

/// Collects mismatches and reports the first few.
#[derive(Default)]
struct Diffs {
    checked: usize,
    failed: usize,
    report: String,
}

impl Diffs {
    fn check(&mut self, what: &str, input: &Value, got: &Value, want: &Value) {
        self.checked += 1;
        if got != want {
            self.failed += 1;
            if self.failed <= 10 {
                let mut shown = input.to_string();
                shown.truncate(400);
                let _ = writeln!(self.report, "{what} {shown}\n  rust: {got}\n  go:   {want}");
            }
        }
    }

    fn finish(self, label: &str) {
        eprintln!("{label}: {} comparisons", self.checked);
        assert!(self.checked > 0, "{label}: nothing compared");
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
    std::env::var("PURSE_TXRULES_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
}

const BATCH: usize = 5_000;

/// Sends `items` under `key` in batches and returns Go's answers in order.
fn call_batched(o: &mut Oracle, cmd: &str, key: &str, items: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(items.len());
    for chunk in items.chunks(BATCH) {
        let answer = o.call(cmd, json!({ key: chunk }));
        let answers = answer.as_array().expect("an array answer");
        assert_eq!(answers.len(), chunk.len(), "{cmd}: answer count");
        out.extend(answers.iter().cloned());
    }
    out
}

/// Go's answers to `poolfee.ticket` on `net`, from a process that has
/// served no other network (the package's subsidy cache is bound to the
/// first network a process serves).
fn ticket_fees(o: &mut Oracle, net: &str, cases: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(cases.len());
    for chunk in cases.chunks(BATCH) {
        let answer = o.call("poolfee.ticket", json!({ "net": net, "cases": chunk }));
        assert_eq!(
            answer["bound"],
            json!(net),
            "oracle bound to another network"
        );
        let fees = answer["fees"].as_array().expect("fees");
        assert_eq!(fees.len(), chunk.len(), "poolfee.ticket: answer count");
        out.extend(fees.iter().cloned());
    }
    out
}

/// The `poolfee.ticket` frozen/`rust_answer` element for a case on `net`.
fn with_net(net: &str, case: &Value) -> Value {
    let mut v = vec![json!(net)];
    v.extend(case.as_array().expect("case").iter().cloned());
    Value::Array(v)
}

/// The oracle's architecture, which must be one this file knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arch {
    Amd64,
    Arm64,
}

fn arch(o: &mut Oracle) -> Arch {
    let a = o.call("arch", Value::Null);
    match a["goarch"].as_str().expect("goarch") {
        "amd64" => Arch::Amd64,
        "arm64" => Arch::Arm64,
        other => panic!(
            "oracle built for {other}: Go's float-to-int conversion there is not \
             modelled; the differential knows amd64 and arm64"
        ),
    }
}

fn compare(o: &mut Oracle, label: &str, cmd: &str, key: &str, inputs: &[Value]) {
    let go = call_batched(o, cmd, key, inputs);
    let mut d = Diffs::default();
    for (input, want) in inputs.iter().zip(&go) {
        d.check(cmd, input, &rust_answer(cmd, input), want);
    }
    d.finish(label);
}

#[test]
fn constants_match_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let c = o.call("consts", Value::Null);
    let want = json!({
        "redeem_p2pk_sig_script_size": REDEEM_P2PK_SIG_SCRIPT_SIZE,
        "redeem_p2pkh_sig_script_size": REDEEM_P2PKH_SIG_SCRIPT_SIZE,
        "redeem_p2sh_sig_script_size": REDEEM_P2SH_SIG_SCRIPT_SIZE,
        "redeem_p2pkh_input_size": REDEEM_P2PKH_INPUT_SIZE,
        "p2pkh_pk_script_size": P2PKH_PK_SCRIPT_SIZE,
        "p2pkh_pk_treasury_script_size": P2PKH_PK_TREASURY_SCRIPT_SIZE,
        "p2sh_pk_script_size": P2SH_PK_SCRIPT_SIZE,
        "ticket_commitment_script_size": TICKET_COMMITMENT_SCRIPT_SIZE,
        "p2pkh_output_size": P2PKH_OUTPUT_SIZE,
        "tspend_input_size": TSPEND_INPUT_SIZE,
        "default_relay_fee_per_kb": DEFAULT_RELAY_FEE_PER_KB.0,
        "max_amount": MAX_AMOUNT,
    });
    assert_eq!(c, want);
}

#[test]
fn sizes_match_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules sizes");
    let n = 40 * scale();
    let sizes: Vec<Value> = size_values(&mut rng, 50 * scale())
        .into_iter()
        .map(|s| json!(s))
        .collect();
    compare(&mut o, "size.input", "size.input", "sizes", &sizes);
    compare(&mut o, "size.output", "size.output", "sizes", &sizes);
    compare(
        &mut o,
        "size.estimate",
        "size.estimate",
        "cases",
        &estimate_cases(&mut rng, n),
    );
    compare(
        &mut o,
        "size.scripts",
        "size.scripts",
        "cases",
        &scripts_cases(&mut rng, n),
    );
}

#[test]
fn fees_match_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules fees");
    compare(
        &mut o,
        "fee",
        "fee",
        "cases",
        &fee_cases(&mut rng, 500 * scale()),
    );
}

#[test]
fn dust_matches_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules dust");
    let amounts = dust_amount_cases(&mut rng, 500 * scale());
    // The zero-cost script size is in the corpus, so the divergence at
    // Go's divide-by-zero panic is compared, not skipped.
    assert!(amounts.iter().any(|c| c[1] == json!(-184)));
    compare(&mut o, "dust.amount", "dust.amount", "cases", &amounts);
    let outputs = output_cases(&mut rng, 100 * scale());
    compare(&mut o, "dust.output", "dust.output", "cases", &outputs);
    compare(&mut o, "check.output", "check.output", "cases", &outputs);
}

#[test]
fn high_fees_match_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules highfees");
    compare(
        &mut o,
        "highfees",
        "highfees",
        "cases",
        &highfees_cases(&mut rng, 50 * scale()),
    );
    compare(
        &mut o,
        "highfees.tx",
        "highfees.tx",
        "txs",
        &highfees_tx_cases(&mut rng, 50 * scale()),
    );
}

#[test]
fn stake_sub_script_types_match_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let types: Vec<Value> = (0..SCRIPT_TYPES.len()).map(|n| json!(n)).collect();
    compare(&mut o, "stake.subtype", "stake.subtype", "types", &types);
}

#[test]
fn valid_pool_fee_rate_matches_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules poolfee.valid");
    let bits: Vec<Value> = pool_fee_values(&mut rng, 500 * scale())
        .into_iter()
        .map(|f| json!(bits_hex(f)))
        .collect();
    compare(&mut o, "poolfee.valid", "poolfee.valid", "bits", &bits);
}

/// The model of Go's `int64(math.Floor(f * 100))` the crate's amd64 answer
/// and the arm64 exclusion rest on, checked against the oracle's own
/// architecture.
#[test]
fn poolfee_int_model() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let arch = arch(&mut o);
    let mut rng = SplitMix64::from_env("purse-txrules poolfee.int");
    let fs = pool_fee_values(&mut rng, 500 * scale());
    let bits: Vec<Value> = fs.iter().map(|&f| json!(bits_hex(f))).collect();
    let go = call_batched(&mut o, "poolfee.int", "bits", &bits);
    let mut d = Diffs::default();
    let mut dependent = 0;
    for ((f, input), want) in fs.iter().zip(&bits).zip(&go) {
        let x = (f * 100.0).floor();
        if pool_fee_int_is_arch_dependent(*f) {
            dependent += 1;
        }
        let model = match arch {
            Arch::Amd64 if pool_fee_int_is_arch_dependent(*f) => i64::MIN,
            // In range both truncate; out of range arm64 saturates and
            // gives 0 for a NaN, which is Rust's `as`.
            _ => x as i64,
        };
        d.check("poolfee.int", input, &json!(model), want);
    }
    assert!(dependent > 0, "no architecture-dependent percentage");
    d.finish(&format!("poolfee.int ({arch:?})"));
}

#[test]
fn stake_pool_ticket_fee_matches_go() {
    let Some(probe) = oracle_or_skip("txrules").map(|mut o| arch(&mut o)) else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txrules poolfee.ticket");
    let mut d = Diffs::default();
    let mut excluded = 0;
    let mut panics = 0;
    for net in NETS {
        let cases = ticket_cases(&mut rng, net, 200 * scale());
        // A process per network: Go's subsidy cache is bound to the first.
        let mut o = oracle_or_skip("txrules").expect("oracle");
        let go = ticket_fees(&mut o, net, &cases);
        for (case, want) in cases.iter().zip(&go) {
            let bits = case[3].as_str().expect("bits");
            if probe == Arch::Arm64 && pool_fee_int_is_arch_dependent(hex_bits(bits)) {
                excluded += 1;
                continue;
            }
            if want.get("panic").is_some() {
                panics += 1;
            }
            let input = with_net(net, case);
            d.check(
                "poolfee.ticket",
                &input,
                &rust_answer("poolfee.ticket", &input),
                want,
            );
        }
    }
    assert!(
        panics >= NETS.len(),
        "the zero-denominator cases are compared"
    );
    eprintln!(
        "poolfee.ticket: {panics} Go panics matched by errors; {excluded} \
         architecture-dependent percentages excluded ({probe:?})"
    );
    d.finish("poolfee.ticket");
}

/// QK-0269: Go's process-wide subsidy cache answers a later network with
/// the first network's subsidy (and the later network's own reduction
/// count); the crate uses the parameters it is given.
#[test]
fn go_subsidy_cache_is_bound_to_the_first_network() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    // Height 200,000 votes on both networks, with different subsidies.
    let (v, z, height, pct) = (15 * 100_000_000i64, 5_000_000i64, 200_000i64, 2.59f64);
    let case = json!([v, z, height, bits_hex(pct), true, true]);
    let first = ticket_fees(&mut o, "mainnet", std::slice::from_ref(&case));
    let answer = o.call(
        "poolfee.ticket",
        json!({ "net": "testnet3", "cases": [case] }),
    );
    assert_eq!(answer["bound"], json!("mainnet"));
    let go_testnet = answer["fees"][0].as_i64().expect("fee");
    let fee = |net| {
        stake_pool_ticket_fee(
            Amount(v),
            Amount(z),
            height as i32,
            pct,
            &params(net),
            true,
            true,
        )
        .expect("fee")
        .0
    };
    assert_eq!(first[0], json!(fee("mainnet")));
    assert_ne!(
        go_testnet,
        fee("testnet3"),
        "Go does not use testnet3's subsidy"
    );
    // Go's answer: mainnet's vote subsidy, reduced as many times as
    // testnet3's parameters say (ceil(1024 / 2048) = 1), in the fee formula.
    let mainnet = params("mainnet");
    let testnet = params("testnet3");
    let mut s = dcroxide_standalone::SubsidyCache::new(&mainnet)
        .calc_stake_vote_subsidy_v3(height, dcroxide_standalone::SubsidySplitVariant::Dcp0012);
    let adjs = (f64::from(testnet.ticket_pool_size) / testnet.subsidy_reduction_interval as f64)
        .ceil() as i64;
    assert_eq!(adjs, 1);
    for _ in 0..adjs {
        s = s * 100 / 101;
    }
    let p = (pct * 100.0).floor() as i128;
    let (s, v, z) = (i128::from(s), i128::from(v), i128::from(z));
    let model = p * s * (v + z) / ((s + v) * 10_000);
    assert_eq!(i128::from(go_testnet), model);
}

// ---------------------------------------------------------------------
// Frozen vectors.
// ---------------------------------------------------------------------

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

#[test]
#[ignore = "regenerates tests/data; run with PURSE_TXRULES_REGEN=1"]
fn regenerate_frozen_vectors() {
    assert!(
        std::env::var_os("PURSE_TXRULES_REGEN").is_some(),
        "set PURSE_TXRULES_REGEN=1 to rewrite tests/data"
    );
    let mut o = oracle_or_skip("txrules").expect("regeneration needs the Go oracle");
    assert_eq!(
        arch(&mut o),
        Arch::Amd64,
        "the frozen vectors pin amd64's answers; regenerate on amd64"
    );
    let mut rng = SplitMix64(VECTORS_SEED);
    let mut body = String::new();
    let mut counts = Vec::new();
    let sizes: Vec<Value> = size_values(&mut rng, 40)
        .into_iter()
        .map(|s| json!(s))
        .collect();
    counts.push(freeze(&mut o, &mut body, "size.input", "sizes", &sizes));
    counts.push(freeze(&mut o, &mut body, "size.output", "sizes", &sizes));
    let cases = estimate_cases(&mut rng, 120);
    counts.push(freeze(&mut o, &mut body, "size.estimate", "cases", &cases));
    let cases = scripts_cases(&mut rng, 120);
    counts.push(freeze(&mut o, &mut body, "size.scripts", "cases", &cases));
    let cases: Vec<Value> = fee_cases(&mut rng, 300)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % 3 == 0)
        .map(|(_, c)| c)
        .collect();
    counts.push(freeze(&mut o, &mut body, "fee", "cases", &cases));
    let cases: Vec<Value> = dust_amount_cases(&mut rng, 300)
        .into_iter()
        .enumerate()
        .filter(|(i, c)| i % 2 == 0 || c[1] == json!(-184))
        .map(|(_, c)| c)
        .collect();
    counts.push(freeze(&mut o, &mut body, "dust.amount", "cases", &cases));
    let outputs: Vec<Value> = output_cases(&mut rng, 200)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % 3 == 0)
        .map(|(_, c)| c)
        .collect();
    counts.push(freeze(&mut o, &mut body, "dust.output", "cases", &outputs));
    counts.push(freeze(&mut o, &mut body, "check.output", "cases", &outputs));
    let cases: Vec<Value> = highfees_cases(&mut rng, 160);
    counts.push(freeze(&mut o, &mut body, "highfees", "cases", &cases));
    let cases = highfees_tx_cases(&mut rng, 100);
    counts.push(freeze(&mut o, &mut body, "highfees.tx", "txs", &cases));
    let types: Vec<Value> = (0..SCRIPT_TYPES.len()).map(|n| json!(n)).collect();
    counts.push(freeze(&mut o, &mut body, "stake.subtype", "types", &types));
    let bits: Vec<Value> = pool_fee_values(&mut rng, 100)
        .into_iter()
        .map(|f| json!(bits_hex(f)))
        .collect();
    counts.push(freeze(&mut o, &mut body, "poolfee.valid", "bits", &bits));
    drop(o);
    let mut tickets = 0;
    for net in NETS {
        let cases = ticket_cases(&mut rng, net, 60);
        let mut o = oracle_or_skip("txrules").expect("oracle");
        let go = ticket_fees(&mut o, net, &cases);
        for (case, out) in cases.iter().zip(&go) {
            push_entry(&mut body, "poolfee.ticket", &with_net(net, case), out);
        }
        tickets += cases.len();
    }
    counts.push(tickets);

    let about = format!(
        "Every \"out\" is the answer of the Go oracle tools/oracle/cmd/txrules \
         (decred.org/dcrwallet/v5 wallet/txsizes and wallet/txrules at {}), built for \
         amd64 (GOAMD64 v1): where Go's answer depends on the architecture (the VSP fee of \
         a NaN or out-of-range percentage), these are amd64's, which purse-txrules gives on \
         every target. An \"out\" of {{\"panic\": text}} is a Go panic; the crate answers \
         there as docs/PARITY.md records, which tests/common/mod.rs `rust_answer` checks \
         before reporting Go's text. Corpus: tests/common/mod.rs with SplitMix64({:#x}): \
         size_values(40) for size.input and size.output, estimate_cases(120), \
         scripts_cases(120), every third of fee_cases(300), every other of \
         dust_amount_cases(300) and every case of script size -184, every third of \
         output_cases(200) for dust.output and check.output, highfees_cases(160), \
         highfees_tx_cases(100), the 21 script types, pool_fee_values(100), and \
         ticket_cases(net, 60) for mainnet, testnet3, simnet and regnet, each from a fresh \
         oracle process (poolfee.ticket's \"in\" is the network followed by the case); \
         entries per command (size.input, size.output, size.estimate, size.scripts, fee, \
         dust.amount, dust.output, check.output, highfees, highfees.tx, stake.subtype, \
         poolfee.valid, poolfee.ticket) {counts:?}. Written by `PURSE_TXRULES_REGEN=1 \
         cargo test -p purse-txrules --test oracle -- --ignored regenerate_frozen_vectors`, \
         which also checked the crate equal to every answer. Each line: {{\"cmd\": oracle \
         command, \"in\": one element of its batch, \"out\": Go's answer to it}}.",
        &DCRWALLET_PARITY_COMMIT[..12],
        VECTORS_SEED
    );
    let text = format!(
        "# wallet/txsizes and wallet/txrules: the Go oracle's answers.\n#\n{}{}{body}",
        comment(&about),
        integrity_header(&body)
    );
    std::fs::create_dir_all(data_dir()).expect("create tests/data");
    std::fs::write(data_dir().join(VECTORS_FILE), text).expect("write frozen vectors");
}
