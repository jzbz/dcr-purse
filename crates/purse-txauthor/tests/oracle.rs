// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrwallet's `wallet/txauthor` through the Go
//! oracle `tools/oracle/cmd/txauthor`, and the generator of the frozen
//! vector file.
//!
//! Inputs come from `SplitMix64::from_env`; set `PURSE_TEST_SEED` to vary or
//! reproduce them, and `PURSE_TXAUTHOR_SCALE` (default 10) to size the
//! random parts of the corpora. Without Go the tests skip (fail under
//! `PURSE_REQUIRE_ORACLE`).
//!
//! Each authoring compares the whole answer: the serialized transaction,
//! the previous scripts, the total, the change index and the size
//! estimate, or the error with its kind and operation, together with every
//! target the loop asked the input source for and the calls made to the
//! change source. Each randomization compares the position, the bytes drawn
//! from the seeded stream and the output order after every call. Each
//! signing compares the transaction afterwards, every key and script
//! lookup in order, and the error.

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
        self.checked += 1;
        if got != want {
            self.failed += 1;
            if self.failed <= 6 {
                let mut shown = input.to_string();
                shown.truncate(1500);
                let mut g = got.to_string();
                g.truncate(1500);
                let mut w = want.to_string();
                w.truncate(1500);
                let _ = writeln!(self.report, "{what} {shown}\n  rust: {g}\n  go:   {w}");
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
    std::env::var("PURSE_TXAUTHOR_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
}

const BATCH: usize = 500;

/// Sends `cases` in batches and returns Go's answers in order.
fn call_batched(o: &mut Oracle, cmd: &str, cases: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(cases.len());
    for chunk in cases.chunks(BATCH) {
        let answer = o.call(cmd, json!({ "cases": chunk }));
        let answers = answer.as_array().expect("an array answer");
        assert_eq!(answers.len(), chunk.len(), "{cmd}: answer count");
        out.extend(answers.iter().cloned());
    }
    out
}

fn compare(o: &mut Oracle, label: &str, cmd: &str, cases: &[Value]) -> Vec<Value> {
    let go = call_batched(o, cmd, cases);
    let mut d = Diffs::default();
    for (case, want) in cases.iter().zip(&go) {
        d.check(cmd, case, &rust_answer(cmd, case), want);
    }
    d.finish(label);
    go
}

#[test]
fn constants_match_go() {
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let c = o.call("consts", json!({}));
    assert_eq!(
        c["max_script_element_size"],
        json!(dcroxide_txscript::MAX_SCRIPT_ELEMENT_SIZE)
    );
    for net in NETS {
        assert_eq!(
            c["max_tx_size"][net],
            json!(params(net).max_tx_size),
            "{net} MaxTxSize"
        );
    }
}

#[test]
fn authoring_matches_go() {
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txauthor authoring");
    let mut cases = edge_author_cases(&mut rng);
    let n_edge = cases.len();
    for _ in 0..scale() * 300 {
        cases.push(random_author_case(&mut rng));
    }
    let go = compare(&mut o, "author", "author", &cases);
    // The corpus reaches every outcome.
    let count = |f: &dyn Fn(&Value) -> bool| go.iter().filter(|a| f(a)).count();
    let with_change = count(&|a| a["change_index"].as_i64().is_some_and(|i| i >= 0));
    let without = count(&|a| a["change_index"] == json!(-1));
    let insufficient = count(&|a| a["error"]["kind"] == json!("insufficient balance"));
    let too_big = count(&|a| {
        a["error"]["error"] == json!("invalid operation: signed tx size exceeds allowed maximum")
    });
    let big_script = count(&|a| {
        a["error"]["error"]
            == json!("invalid operation: script size exceed maximum bytes pushable to the stack")
    });
    let source_errors = count(&|a| a["error"]["op"] == json!("txauthor.NewUnsignedTransaction"));
    let endless = count(&|a| {
        a["error"]["error"]
            == json!("txauthor.NewUnsignedTransaction: input source: call limit reached")
    });
    let panics = count(&|a| a.get("panic").is_some());
    eprintln!(
        "author: {n_edge} edge cases; {with_change} with change, {without} without, \
         {insufficient} insufficient balance, {too_big} over the size limit, {big_script} \
         change scripts too long, {source_errors} wrapped errors, {endless} cut by the \
         source's call limit, {panics} Go panics"
    );
    assert!(with_change > 0 && without > 0 && insufficient > 0 && too_big > 0);
    assert!(big_script > 0 && source_errors > 0 && endless > 0);
    // Go's only panic here is IsDustAmount's division by zero for a change
    // script size of -184 (QK-0741), which rust_answer maps.
    for (case, a) in cases.iter().zip(&go) {
        if let Some(p) = a.get("panic") {
            assert_eq!(p, &json!(GO_INTEGER_DIVIDE_BY_ZERO), "{case}");
            assert_eq!(case["change"]["size"], json!(-184), "{case}");
        }
    }
}

#[test]
fn randomization_matches_go() {
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-txauthor randomization");
    let cases = randomize_cases(&mut rng, scale() * 100);
    let go = compare(&mut o, "randomize", "randomize", &cases);
    let calls: Vec<&Value> = go
        .iter()
        .flat_map(|a| a["results"].as_array().expect("results"))
        .collect();
    let panics = calls.iter().filter(|r| r.get("panic").is_some()).count();
    eprintln!("randomize: {} calls, {panics} at Go panics", calls.len());
    assert!(panics > 0 && panics < calls.len());
}

#[test]
fn signing_matches_go() {
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let bin = purse_testutil::oracle_bin_or_skip("txauthor").expect("the oracle was built");
    let mut rng = SplitMix64::from_env("purse-txauthor signing");
    let mut cases = edge_sign_cases(&mut rng, Big::All);
    let n_edge = cases.len();
    cases.extend(sign_cases(&mut rng, scale() * 30));
    // Where the crate refuses what kills Go's process (QK-0746), each case
    // runs alone in a fresh process, which must die of exactly that.
    let (crashes, cases): (Vec<Value>, Vec<Value>) = cases
        .into_iter()
        .partition(|c| sign_answer(c).get("crash").is_some());
    for (case, fatal) in crashes.iter().zip(go_fatal_all(&bin, "sign", &crashes)) {
        assert_eq!(fatal.as_deref(), Some(GO_STACK_OVERFLOW), "{case}");
    }
    let go = compare(&mut o, "sign", "sign", &cases);
    let ok = go
        .iter()
        .filter(|a| a["error"].is_null() && a.get("panic").is_none())
        .count();
    let panics = go.iter().filter(|a| a.get("panic").is_some()).count();
    eprintln!(
        "sign: {} cases ({n_edge} edge), {ok} fully signed, {panics} at Go panics, {} at Go \
         stack overflows",
        go.len() + crashes.len(),
        crashes.len()
    );
    assert!(ok > 0 && panics > 0 && !crashes.is_empty());
}

/// The nested pay-to-script-hash shapes reach both outcomes of Go's merge:
/// it recurses until the process dies, and it returns. `signing_matches_go`
/// runs these cases against Go (each case that kills Go must be one the
/// crate refuses, and each the crate signs must be answered by Go with the
/// crate's transaction); this checks, without Go, that the corpus holds
/// enough of both, including the stake-tagged first redeem script whose
/// second script fills the builder.
#[test]
fn nested_p2sh_shapes_reach_both_outcomes() {
    let mut rng = SplitMix64::from_env("purse-txauthor nested");
    let cases = nested_p2sh_cases(&mut rng, Big::All);
    let refused: Vec<&Value> = cases
        .iter()
        .filter(|c| sign_answer(c).get("crash").is_some())
        .collect();
    let returned = cases.len() - refused.len();
    eprintln!(
        "nested: {} cases, {} refused, {returned} merged",
        cases.len(),
        refused.len()
    );
    assert!(refused.len() >= 20 && returned >= 40);
    // Among the refused: a stake-tagged first redeem script (OP_SSTX, 0xba)
    // whose second script fills the builder, so that the new signature
    // script is the second script alone.
    assert!(refused.iter().any(|c| {
        c["scripts"]
            .as_object()
            .expect("scripts")
            .values()
            .any(|s| s.as_str().expect("hex").starts_with("baa914"))
            && c.to_string().len() > 30_000
    }));
}

/// ECDSA keys that reduce to zero: Go signs with them, for the point at
/// infinity (and its P2PKH signature script carries 0x02 and 32 zero
/// bytes), and the crate refuses them with `invalid privkey` (ADR-0012
/// Rule 7, QK-0682). The differential leaves exactly these out.
#[test]
fn zero_ecdsa_keys_diverge() {
    let mut rng = SplitMix64::from_env("purse-txauthor zero keys");
    let cases = zero_ecdsa_key_cases(&mut rng);
    for case in &cases {
        let rust = sign_answer(case);
        assert_eq!(rust["error"]["error"], json!("invalid privkey"), "{case}");
    }
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let go = call_batched(&mut o, "sign", &cases);
    let infinity = format!("21{}{}", "02", "00".repeat(32));
    for (case, a) in cases.iter().zip(&go) {
        assert!(a["error"].is_null(), "{case}: {a}");
        assert!(
            a["tx"].as_str().expect("tx").contains(&infinity),
            "{case}: {a}"
        );
    }
}

// ---------------------------------------------------------------------
// Frozen vectors.
// ---------------------------------------------------------------------

fn freeze(o: &mut Oracle, body: &mut String, cmd: &str, cases: &[Value]) -> usize {
    let bin = purse_testutil::oracle_bin_or_skip("txauthor").expect("the oracle was built");
    let crashes: Vec<bool> = cases
        .iter()
        .map(|c| rust_answer(cmd, c).get("crash").is_some())
        .collect();
    let live: Vec<Value> = cases
        .iter()
        .zip(&crashes)
        .filter(|&(_, &crash)| !crash)
        .map(|(c, _)| c.clone())
        .collect();
    let dead: Vec<Value> = cases
        .iter()
        .zip(&crashes)
        .filter(|&(_, &crash)| crash)
        .map(|(c, _)| c.clone())
        .collect();
    let mut answers = call_batched(o, cmd, &live).into_iter();
    let mut fatals = go_fatal_all(&bin, cmd, &dead).into_iter();
    let go: Vec<Value> = crashes
        .iter()
        .map(|&crash| {
            if crash {
                let fatal = fatals
                    .next()
                    .expect("a case")
                    .expect("Go dies on this case");
                json!({ "crash": fatal })
            } else {
                answers.next().expect("an answer")
            }
        })
        .collect();
    for (case, out) in cases.iter().zip(&go) {
        assert_eq!(&rust_answer(cmd, case), out, "{cmd} {case}");
        body.push_str(&json!({ "cmd": cmd, "in": case, "out": out }).to_string());
        body.push('\n');
    }
    cases.len()
}

#[test]
#[ignore = "regenerates tests/data; run with PURSE_TXAUTHOR_REGEN=1"]
fn regenerate_frozen_vectors() {
    assert!(
        std::env::var_os("PURSE_TXAUTHOR_REGEN").is_some(),
        "set PURSE_TXAUTHOR_REGEN=1 to rewrite tests/data"
    );
    let mut o = oracle_or_skip("txauthor").expect("regeneration needs the Go oracle");
    let mut rng = SplitMix64(VECTORS_SEED);
    let mut body = String::new();
    let mut cases = edge_author_cases(&mut rng);
    // Random cases, of which only the first two with an input over 10 KB
    // (the 253-element varint boundary) are kept, to bound the file.
    let mut large = 0;
    while cases.len() < 304 {
        let case = random_author_case(&mut rng);
        if case.to_string().len() > 10_000 {
            if large == 2 {
                continue;
            }
            large += 1;
        }
        cases.push(case);
    }
    let authors = freeze(&mut o, &mut body, "author", &cases);
    let cases = randomize_cases(&mut rng, 60);
    let randomizes = freeze(&mut o, &mut body, "randomize", &cases);
    let mut cases = edge_sign_cases(&mut rng, Big::Some);
    cases.extend(sign_cases(&mut rng, 45));
    let signs = freeze(&mut o, &mut body, "sign", &cases);

    let about = format!(
        "Every \"out\" is the answer of the Go oracle tools/oracle/cmd/txauthor \
         (decred.org/dcrwallet/v5 wallet/txauthor at {}). An \"out\" holding \"panic\" is a \
         Go panic, and one of {{\"crash\": line}} the fatal-error line a fresh oracle process \
         died with on that input alone (QK-0746); the crate returns an error at both, as \
         docs/PARITY.md records, which tests/common/mod.rs `rust_answer` checks before \
         reporting Go's text. Corpus: \
         tests/common/mod.rs with SplitMix64({:#x}): edge_author_cases then \
         random_author_case up to 304 cases, skipping every one whose input exceeds 10 KB \
         after the first two, for author ({authors} entries), randomize_cases(60) \
         ({randomizes}), edge_sign_cases(Big::Some) then sign_cases(45) ({signs}). Written by `PURSE_TXAUTHOR_REGEN=1 cargo \
         test -p purse-txauthor --test oracle -- --ignored regenerate_frozen_vectors`, which \
         also checked the crate equal to every answer. Each line: {{\"cmd\": oracle command, \
         \"in\": one element of its batch, \"out\": Go's answer to it}}.",
        &DCRWALLET_PARITY_COMMIT[..12],
        VECTORS_SEED
    );
    let text = format!(
        "# wallet/txauthor: the Go oracle's answers.\n#\n{}{}{body}",
        comment(&about),
        integrity_header(&body)
    );
    std::fs::create_dir_all(data_dir()).expect("create tests/data");
    std::fs::write(data_dir().join(VECTORS_FILE), text).expect("write frozen vectors");
}
