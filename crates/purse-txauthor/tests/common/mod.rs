// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the oracle and frozen-vector tests: the scripted input,
//! change and secrets sources of `tools/oracle/cmd/txauthor`, built the same
//! way here; the crate's answers in that oracle's JSON shapes; the input
//! corpora; and the frozen file format.
//!
//! Where the crate deliberately returns an error and Go panics
//! (docs/PARITY.md; QK-0263, QK-0492, QK-0745) or dies (QK-0746),
//! [`rust_answer`] first checks the crate's own error and then reports Go's
//! panic or fatal-error text, so a frozen or live comparison checks that
//! the crate diverges exactly at Go's panics and crashes and nowhere else.
//! Where Go's authoring loop never ends (QK-0744, QK-0753) and the crate
//! returns an error, it checks that error and then plays on the scripted
//! source the repeated calls Go makes until the source's call limit or
//! `fail_at` ends them (`go_endless_loop`), so the comparison also checks
//! every target Go asks for. The one other divergence, an ECDSA key that
//! reduces to zero (QK-0682), is kept out of the corpora and checked on
//! its own (`zero_ecdsa_key_cases`).

#![allow(dead_code)]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use dcroxide_chaincfg::Params;
use dcroxide_chainhash::Hash;
use dcroxide_crypto::rand::Prng;
use dcroxide_dcrec::{edwards, secp256k1};
use dcroxide_txscript::sign::SignatureType;
use dcroxide_txscript::stdaddr::{self, Address};
use dcroxide_txscript::stdscript;
use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut};
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_errors::{AnyError, Builder, Kind, Op};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, unhex};
use purse_txauthor::*;
use purse_txrules::txsizes::*;
use purse_txrules::{estimate_serialize_size, fee_for_serialize_size};
use sha2::{Digest, Sha256};

/// The separator dcrwallet's daemon renders nested errors with, which the
/// oracle sets too.
pub const SEP: &str = ":: ";

/// Go's panic text for `rand.Int32N(0)`.
pub const GO_INT32N_PANIC: &str = "rand: invalid argument to Int32N";

/// The networks the oracle serves, by the name it takes.
pub const NETS: [&str; 4] = ["mainnet", "testnet3", "simnet", "regnet"];

pub fn params(net: &str) -> Params {
    match net {
        "mainnet" => dcroxide_chaincfg::mainnet_params(),
        "testnet3" => dcroxide_chaincfg::testnet3_params(),
        "simnet" => dcroxide_chaincfg::simnet_params(),
        "regnet" => dcroxide_chaincfg::regnet_params(),
        other => panic!("unknown network {other}"),
    }
}

/// Go's runtime text for an index out of range (`runtime/error.go`
/// `boundsError`) as the oracle answers it: cut after "out of range", as
/// tools/oracle/cmd/oprunner cuts it (ADR-0002), because the indices the
/// runtime prints can differ between architectures.
pub const GO_INDEX_PANIC: &str = "runtime error: index out of range";

// ---------------------------------------------------------------------
// JSON helpers.
// ---------------------------------------------------------------------

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or_else(|| panic!("an int64: {v}"))
}

pub fn output_json(out: &TxOut) -> Value {
    json!({ "value": out.value, "version": out.version, "script": hex(&out.pk_script) })
}

pub fn output_from_json(v: &Value) -> TxOut {
    TxOut {
        value: int(&v["value"]),
        version: u16::try_from(int(&v["version"])).expect("u16"),
        pk_script: unhex(v["script"].as_str().expect("script")),
    }
}

/// An error rendered as the oracle renders one.
pub fn error_json(err: &AnyError) -> Value {
    let (kind, op) = match err.as_error() {
        Some(e) => (e.kind.as_str().to_owned(), e.op.as_str().to_owned()),
        None => (String::new(), String::new()),
    };
    json!({ "error": err.render(SEP), "kind": kind, "op": op })
}

/// The error an `errSpec` describes, built as the oracle builds it.
pub fn build_error(spec: &Value) -> AnyError {
    let kind = int(&spec["kind"]);
    if spec["bare"].as_bool().unwrap_or(false) {
        return AnyError::Kind(Kind::from_code(kind).expect("kind"));
    }
    let op = spec["op"].as_str().unwrap_or("");
    let text = spec["text"].as_str().unwrap_or("");
    let mut builder: Option<Builder> = None;
    let mut push = |arg: purse_errors::Arg| {
        builder = Some(match builder.take() {
            None => Builder::new(arg),
            Some(b) => b.arg(arg),
        });
    };
    if !op.is_empty() {
        push(Op::from_string(op.to_owned()).into());
    }
    if kind >= 0 {
        push(Kind::from_code(kind).expect("kind").into());
    }
    match builder {
        None => purse_errors::new(text),
        Some(b) => {
            let b = if text.is_empty() { b } else { b.arg(text) };
            b.build().into()
        }
    }
}

// ---------------------------------------------------------------------
// author: the scripted sources.
// ---------------------------------------------------------------------

pub fn utxo_hash(i: usize) -> Hash {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&(i as u64).to_le_bytes());
    Hash(h)
}

/// Go `wire.NewTxIn`.
pub fn new_tx_in(prev: OutPoint, value: i64, sig: Vec<u8>) -> TxIn {
    TxIn {
        previous_out_point: prev,
        sequence: 0xffff_ffff,
        value_in: value,
        block_height: 0,
        block_index: 0xffff_ffff,
        signature_script: sig,
    }
}

/// The oracle's scripted input source (`sourceSpec` in
/// `tools/oracle/cmd/txauthor/main.go`).
pub struct ScriptedSource {
    utxos: Vec<[i64; 3]>,
    all: bool,
    delta: i64,
    fail_at: usize,
    error: Option<Value>,
    limit: usize,
    pub calls: Vec<i64>,
    /// Whether the last call answered rather than failed.
    answered: bool,
    total: i64,
    detail: InputDetail,
    next: usize,
}

impl ScriptedSource {
    pub fn new(spec: &Value) -> ScriptedSource {
        let utxos = spec["utxos"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|u| [int(&u[0]), int(&u[1]), int(&u[2])])
                    .collect()
            })
            .unwrap_or_default();
        let limit = spec["limit"].as_u64().unwrap_or(0) as usize;
        ScriptedSource {
            utxos,
            all: spec["mode"].as_str() == Some("all"),
            delta: spec["delta"].as_i64().unwrap_or(0),
            fail_at: spec["fail_at"].as_u64().unwrap_or(0) as usize,
            error: spec.get("error").filter(|e| !e.is_null()).cloned(),
            limit: if limit == 0 { 64 } else { limit },
            calls: Vec::new(),
            answered: false,
            total: 0,
            detail: InputDetail::default(),
            next: 0,
        }
    }

    /// Whether the last call answered, with an amount that met its target.
    pub fn met_last_target(&self) -> bool {
        self.answered
            && self
                .calls
                .last()
                .is_some_and(|&t| self.total.wrapping_add(self.delta) >= t)
    }

    fn add(&mut self) {
        let i = self.next;
        let [value, sig_size, pk_len] = self.utxos[i];
        let op = OutPoint {
            hash: utxo_hash(i),
            index: i as u32,
            tree: (i % 2) as i8,
        };
        self.detail.inputs.push(new_tx_in(op, value, Vec::new()));
        self.detail
            .scripts
            .push((0..pk_len as usize).map(|j| (i + j) as u8).collect());
        self.detail.redeem_script_sizes.push(sig_size);
        self.total = self.total.wrapping_add(value);
        self.next += 1;
    }
}

impl InputSource for ScriptedSource {
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError> {
        self.calls.push(target.0);
        self.answered = false;
        let n = self.calls.len();
        if n == self.fail_at
            && let Some(e) = &self.error
        {
            return Err(build_error(e));
        }
        if n > self.limit {
            return Err(purse_errors::new("input source: call limit reached"));
        }
        if self.all {
            while self.next < self.utxos.len() {
                self.add();
            }
        } else {
            while self.total < target.0 && self.next < self.utxos.len() {
                self.add();
            }
        }
        let mut detail = self.detail.clone();
        detail.amount = Amount(self.total.wrapping_add(self.delta));
        self.answered = true;
        Ok(detail)
    }
}

/// The oracle's scripted change source (`changeSpec`).
pub struct ScriptedChange {
    size: i64,
    script: Vec<u8>,
    version: u16,
    error: Option<Value>,
    pub script_calls: usize,
    pub size_calls: Cell<usize>,
}

impl ScriptedChange {
    pub fn new(spec: &Value) -> ScriptedChange {
        ScriptedChange {
            size: int(&spec["size"]),
            script: unhex(spec["script"].as_str().unwrap_or("")),
            version: u16::try_from(spec["version"].as_u64().unwrap_or(0)).expect("u16"),
            error: spec.get("error").filter(|e| !e.is_null()).cloned(),
            script_calls: 0,
            size_calls: Cell::new(0),
        }
    }
}

impl ChangeSource for ScriptedChange {
    fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError> {
        self.script_calls += 1;
        match &self.error {
            Some(e) => Err(build_error(e)),
            None => Ok((self.script.clone(), self.version)),
        }
    }

    fn script_size(&self) -> i64 {
        self.size_calls.set(self.size_calls.get() + 1);
        self.size
    }
}

pub fn author_answer(case: &Value) -> Value {
    let outputs: Vec<TxOut> = case["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .map(output_from_json)
        .collect();
    let mut source = ScriptedSource::new(&case["source"]);
    let mut change = ScriptedChange::new(&case["change"]);
    let result = new_unsigned_transaction(
        outputs,
        Amount(int(&case["rate"])),
        &mut source,
        &mut change,
        int(&case["max"]),
    );
    if case["change"]["size"] == json!(-184)
        && let Ok(atx) = &result
        && let Some(answer) = go_dust_panic(case, atx, &source)
    {
        return answer_with_calls(answer, &source, &change);
    }
    if let Err(err) = &result
        && change.script_calls == 0
        && let Some(answer) = go_endless_loop(err, &mut source)
    {
        return answer_with_calls(answer, &source, &change);
    }
    let answer = match result {
        Ok(atx) => json!({
            "tx": hex(&atx.tx.serialize()),
            "prev": atx.prev_scripts.iter().map(|s| hex(s)).collect::<Vec<_>>(),
            "total": atx.total_input.0,
            "change_index": atx.change_index,
            "size": atx.estimated_signed_serialize_size,
        }),
        Err(e) => json!({ "error": error_json(&AnyError::from(e)) }),
    };
    answer_with_calls(answer, &source, &change)
}

fn answer_with_calls(mut answer: Value, source: &ScriptedSource, change: &ScriptedChange) -> Value {
    answer["calls"] = json!(source.calls);
    answer["script_calls"] = json!(change.script_calls);
    answer["size_calls"] = json!(change.size_calls.get());
    answer
}

/// Whether the crate refuses `case` as Go's endless loop (QK-0744,
/// QK-0753), where [`author_answer`] plays on Go's repeated calls.
pub fn refused_as_endless(case: &Value) -> bool {
    let outputs: Vec<TxOut> = case["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .map(output_from_json)
        .collect();
    let mut source = ScriptedSource::new(&case["source"]);
    let mut change = ScriptedChange::new(&case["change"]);
    let result = new_unsigned_transaction(
        outputs,
        Amount(int(&case["rate"])),
        &mut source,
        &mut change,
        int(&case["max"]),
    );
    result.is_err_and(|e| e.is_kind(Kind::InsufficientBalance))
        && change.script_calls == 0
        && source.met_last_target()
}

/// The most calls [`go_endless_loop`] makes before deciding the scripted
/// source never refuses; the corpora's sources refuse by the 64th call.
const MAX_REPEATED_CALLS: usize = 1 << 16;

/// Go's answer where its loop never ends (QK-0744, QK-0753): the crate
/// returned its insufficient-balance error although the source answered
/// its last call with an amount that met the target and the change source
/// was not asked (its errors are wrapped alike), which happens only where a
/// retry would ask for that target again. This checks that error, then
/// asks the scripted source for the same target until it fails, checking
/// that every answer equals the last one (so Go, given it, would retry
/// with the same target again), and reports that failure wrapped as Go
/// wraps it: the scripted source's call limit or its error at `fail_at`.
/// The calls recorded are then Go's: the crate's, followed by the repeats.
fn go_endless_loop(err: &purse_errors::Error, source: &mut ScriptedSource) -> Option<Value> {
    if !err.is_kind(Kind::InsufficientBalance) || !source.met_last_target() {
        return None;
    }
    let &target = source.calls.last()?;
    let last = InputDetail {
        amount: Amount(source.total.wrapping_add(source.delta)),
        ..source.detail.clone()
    };
    assert_eq!(
        err.render(SEP),
        "txauthor.NewUnsignedTransaction: insufficient balance"
    );
    for _ in 0..MAX_REPEATED_CALLS {
        match source.fetch(Amount(target)) {
            Ok(detail) => {
                assert_eq!(detail, last, "the source answers a repeated target alike");
            }
            Err(e) => {
                let wrapped = purse_errors::e!(Op::new("txauthor.NewUnsignedTransaction"), e);
                return Some(json!({ "error": error_json(&AnyError::from(wrapped)) }));
            }
        }
    }
    panic!("the scripted source answered {MAX_REPEATED_CALLS} repeated calls");
}

/// Go's text for an integer division by zero (a `runtime.Error`).
pub const GO_INTEGER_DIVIDE_BY_ZERO: &str = "runtime error: integer divide by zero";

/// Go's answer where its `IsDustAmount` divides by zero: a change source
/// declaring a script size of -184 (QK-0741) and a nonzero change amount,
/// which is the one way the loop reaches the dust test. The crate treats
/// that change as dust and authors the transaction without it; this checks
/// that and reports Go's panic.
fn go_dust_panic(case: &Value, atx: &AuthoredTx, source: &ScriptedSource) -> Option<Value> {
    let outputs: Vec<TxOut> = case["outputs"]
        .as_array()
        .expect("outputs")
        .iter()
        .map(output_from_json)
        .collect();
    let target = outputs.iter().fold(0i64, |s, o| s.wrapping_add(o.value));
    let size = estimate_serialize_size(&source.detail.redeem_script_sizes, &outputs, -184);
    let fee = fee_for_serialize_size(Amount(int(&case["rate"])), size).0;
    let change = atx.total_input.0.wrapping_sub(target).wrapping_sub(fee);
    if change == 0 {
        return None;
    }
    assert_eq!(atx.change_index, -1, "a size -184 change must be dust");
    Some(json!({ "panic": GO_INTEGER_DIVIDE_BY_ZERO }))
}

// ---------------------------------------------------------------------
// randomize.
// ---------------------------------------------------------------------

/// A [`Prng`] that counts the bytes drawn.
pub struct Counted(pub Prng, pub u64);

impl RandomSource for Counted {
    fn uint64(&mut self) -> u64 {
        self.1 += 8;
        self.0.uint64()
    }
}

pub fn randomize_answer(case: &Value) -> Value {
    let key: [u8; 32] = unhex(case["key"].as_str().expect("key"))
        .try_into()
        .expect("32-byte key");
    let n = case["n"].as_u64().expect("n") as usize;
    let mut rng = Counted(Prng::from_seed(key), 0);
    let mut outs: Vec<TxOut> = (0..n)
        .map(|i| TxOut {
            value: i as i64,
            ..TxOut::default()
        })
        .collect();
    let mut results = Vec::new();
    for call in case["calls"].as_array().expect("calls") {
        let index = int(&call[0]);
        let change = call[1].as_bool().expect("change");
        let len = outs.len();
        let got = if change {
            let mut atx = AuthoredTx {
                tx: MsgTx {
                    tx_out: std::mem::take(&mut outs),
                    ..MsgTx::default()
                },
                prev_scripts: Vec::new(),
                total_input: Amount(0),
                change_index: index,
                estimated_signed_serialize_size: 0,
            };
            let r = atx.randomize_change_position_with(&mut rng);
            outs = atx.tx.tx_out;
            match r {
                Ok(()) => Ok(atx.change_index),
                Err(e) => {
                    assert_eq!(atx.change_index, index, "index changed on error");
                    Err(e)
                }
            }
        } else {
            randomize_output_position_with(&mut outs, index, &mut rng)
        };
        let mut r = match got {
            Ok(i) => json!({ "index": i }),
            Err(e) => {
                assert!(e.is_kind(Kind::Invalid), "{e}");
                if len == 0 {
                    json!({ "panic": GO_INT32N_PANIC })
                } else {
                    json!({ "panic": GO_INDEX_PANIC })
                }
            }
        };
        r["drawn"] = json!(rng.1);
        r["order"] = json!(outs.iter().map(|o| o.value).collect::<Vec<_>>());
        results.push(r);
    }
    json!({ "results": results })
}

// ---------------------------------------------------------------------
// sign: the scripted secrets source.
// ---------------------------------------------------------------------

pub struct ScriptedSecrets<'a> {
    params: Params,
    case: &'a Value,
    pub log: Vec<Value>,
}

impl<'a> ScriptedSecrets<'a> {
    pub fn new(case: &'a Value) -> ScriptedSecrets<'a> {
        ScriptedSecrets {
            params: params(case["net"].as_str().expect("net")),
            case,
            log: Vec::new(),
        }
    }
}

fn sig_type(n: i64) -> SignatureType {
    match n {
        0 => SignatureType::EcdsaSecp256k1,
        1 => SignatureType::Ed25519,
        2 => SignatureType::SchnorrSecp256k1,
        other => panic!("signature type {other}"),
    }
}

impl SecretsSource for ScriptedSecrets<'_> {
    fn chain_params(&self) -> &Params {
        &self.params
    }

    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError> {
        let addr = addr.encode();
        self.log.push(json!(["key", addr]));
        if let Some(e) = self.case["key_errors"].get(&addr) {
            return Err(build_error(e));
        }
        match self.case["keys"].get(&addr) {
            Some(k) => Ok((
                unhex(k[0].as_str().expect("key")),
                sig_type(int(&k[1])),
                k[2].as_bool().expect("compressed"),
            )),
            None => Err(purse_errors::e!(Kind::NotExist, format!("no key for {addr}")).into()),
        }
    }

    fn get_script(&mut self, addr: &Address) -> Result<Vec<u8>, AnyError> {
        let addr = addr.encode();
        self.log.push(json!(["script", addr]));
        if let Some(e) = self.case["script_errors"].get(&addr) {
            return Err(build_error(e));
        }
        match self.case["scripts"].get(&addr) {
            Some(s) => Ok(unhex(s.as_str().expect("script"))),
            None => Err(purse_errors::e!(Kind::NotExist, format!("no script for {addr}")).into()),
        }
    }
}

/// Go's text for the panic `sign` hits on a pay-to-pubkey script without an
/// address (`addresses[0]` of an empty slice), cut as [`GO_INDEX_PANIC`] is.
pub const GO_SIGN_PUBKEY_PANIC: &str = GO_INDEX_PANIC;

/// The first line Go writes when the signature-script merge recurses
/// without end (QK-0746); the process then exits.
pub const GO_STACK_OVERFLOW: &str = "fatal error: stack overflow";

/// Runs `cmd` on `case` alone in a fresh oracle process at `bin` and returns
/// the `fatal error:` line it died with, or `None` if it answered.
pub fn go_fatal(bin: &std::path::Path, cmd: &str, case: &Value) -> Option<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the oracle");
    let mut line = json!({ "cmd": cmd, "args": { "cases": [case] } }).to_string();
    line.push('\n');
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(line.as_bytes()).expect("write the request");
    }
    let out = child.wait_with_output().expect("wait for the oracle");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let fatal = stderr
        .lines()
        .find(|l| l.starts_with("fatal error:"))
        .map(str::to_owned);
    if fatal.is_some() {
        assert!(out.stdout.is_empty(), "the oracle answered and died");
    }
    fatal
}

/// [`go_fatal`] for each of `cases`, in order, four processes at a time:
/// each dies only when its goroutine stack reaches the oracle's limit,
/// which takes seconds for the scripts near the 16 KB limit.
pub fn go_fatal_all(bin: &std::path::Path, cmd: &str, cases: &[Value]) -> Vec<Option<String>> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const WORKERS: usize = 4;
    let next = AtomicUsize::new(0);
    let out = Mutex::new(vec![None; cases.len()]);
    std::thread::scope(|s| {
        for _ in 0..WORKERS {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    let fatal = go_fatal(bin, cmd, case);
                    out.lock().expect("no worker panicked")[i] = fatal;
                }
            });
        }
    });
    out.into_inner().expect("no worker panicked")
}

pub fn sign_answer(case: &Value) -> Value {
    let (mut tx, _) = MsgTx::from_bytes(&unhex(case["tx"].as_str().expect("tx"))).expect("tx");
    let prev: Vec<Vec<u8>> = case["prev"]
        .as_array()
        .expect("prev")
        .iter()
        .map(|s| unhex(s.as_str().expect("prev script")))
        .collect();
    let mut secrets = ScriptedSecrets::new(case);
    let result = add_all_input_scripts(&mut tx, &prev, &mut secrets);
    let mut answer = match result {
        Ok(()) => json!({ "error": null }),
        Err(e) if e.to_string() == NESTED_SCRIPT_HASH_MERGE => {
            // Go's process dies; there is no answer to compare beyond that.
            assert!(e.as_error().is_none());
            return json!({ "crash": GO_STACK_OVERFLOW });
        }
        Err(e) if e.to_string() == INVALID_PUBKEY_SCRIPT => {
            assert!(e.as_error().is_none());
            json!({ "panic": GO_SIGN_PUBKEY_PANIC })
        }
        Err(e) => json!({ "error": error_json(&e) }),
    };
    answer["tx"] = json!(hex(&tx.serialize()));
    answer["log"] = Value::Array(secrets.log);
    answer
}

/// The crate's answer to one batch element of `cmd`, in the oracle's
/// shape.
pub fn rust_answer(cmd: &str, input: &Value) -> Value {
    match cmd {
        "author" => author_answer(input),
        "randomize" => randomize_answer(input),
        "sign" => sign_answer(input),
        other => panic!("unknown command {other}"),
    }
}

// ---------------------------------------------------------------------
// Corpora.
// ---------------------------------------------------------------------

pub fn pick<T: Copy>(rng: &mut SplitMix64, items: &[T]) -> T {
    items[rng.below(items.len() as u64) as usize]
}

/// A random int64 of a random magnitude and sign.
pub fn random_i64(rng: &mut SplitMix64) -> i64 {
    let bits = rng.below(64);
    let x = (rng.next_u64() >> bits) as i64;
    if rng.below(4) == 0 {
        x.wrapping_neg()
    } else {
        x
    }
}

fn amount(rng: &mut SplitMix64) -> i64 {
    match rng.below(16) {
        0 => 0,
        1 => pick(rng, &[1, 602, 603, 1537, 1538, 6029, 6030, 6031]),
        2 => pick(rng, &[MAX_AMOUNT, MAX_AMOUNT + 1, i64::MAX, i64::MIN, -1]),
        3 => random_i64(rng),
        4..=8 => 100_000 + rng.below(10_000_000) as i64,
        _ => (1 + rng.below(50) as i64) * 10_000_000,
    }
}

fn rate(rng: &mut SplitMix64) -> i64 {
    match rng.below(14) {
        0 => 0,
        1 => pick(rng, &[1, 999, 1001, 2550]),
        2 => pick(rng, &[-1, -10_000, i64::MAX / 1000, MAX_AMOUNT]),
        3 => random_i64(rng),
        4..=6 => 1000,
        7..=9 => 10_000,
        _ => pick(rng, &[100_000, 1_000_000, 10_000_000, 3_000]),
    }
}

fn script_bytes(rng: &mut SplitMix64, len: usize) -> Vec<u8> {
    rng.bytes(len)
}

fn output(rng: &mut SplitMix64) -> Value {
    let len = match rng.below(12) {
        0 => rng.below(300) as usize,
        1 => pick(rng, &[0, 23, 26, 0xfd]),
        2..=5 => 36,
        _ => 25,
    };
    json!({
        "value": amount(rng),
        "version": if rng.below(10) == 0 { rng.below(3) } else { 0 },
        "script": hex(&script_bytes(rng, len)),
    })
}

fn sig_size(rng: &mut SplitMix64) -> i64 {
    match rng.below(12) {
        0 => random_i64(rng),
        1 => pick(rng, &[0, 74, 110, 166, 0xfd, -1]),
        _ => REDEEM_P2PKH_SIG_SCRIPT_SIZE,
    }
}

fn error_spec(rng: &mut SplitMix64) -> Value {
    let kind = if rng.below(3) == 0 {
        -1
    } else {
        rng.below(21) as i64
    };
    let op = pick(rng, &["", "", "wallet.SelectInputs", "udb.UnspentOutputs"]);
    let text = pick(rng, &["", "source failed", "no addresses"]);
    json!({ "op": op, "kind": kind, "text": text, "bare": kind >= 0 && rng.below(4) == 0 })
}

fn source(rng: &mut SplitMix64) -> Value {
    let n = match rng.below(20) {
        0 => 0,
        1 => 250 + rng.below(10) as usize,
        _ => 1 + rng.below(6) as usize,
    };
    let utxos: Vec<Value> = (0..n)
        .map(|_| json!([amount(rng), sig_size(rng), pick(rng, &[0, 25, 23, 26])]))
        .collect();
    let mut s = json!({ "utxos": utxos });
    if rng.below(5) == 0 {
        s["mode"] = json!("all");
    }
    if rng.below(8) == 0 {
        s["delta"] = json!(match rng.below(3) {
            0 => random_i64(rng),
            1 => -(rng.below(10_000) as i64),
            _ => rng.below(10_000) as i64,
        });
    }
    if rng.below(8) == 0 {
        s["fail_at"] = json!(1 + rng.below(3));
        s["error"] = error_spec(rng);
    }
    if rng.below(10) == 0 {
        s["limit"] = json!(1 + rng.below(4));
    }
    s
}

fn change(rng: &mut SplitMix64) -> Value {
    let size = match rng.below(14) {
        0 => random_i64(rng),
        1 => pick(rng, &[0, -1, -184, 26, 23, 36, 0xfd, i64::MAX]),
        _ => P2PKH_PK_SCRIPT_SIZE,
    };
    let len = match rng.below(14) {
        0 => pick(rng, &[0, 2048, 2049, 3000]),
        1 => rng.below(100) as usize,
        _ => 25,
    };
    let mut c = json!({
        "size": size,
        "script": hex(&script_bytes(rng, len)),
        "version": if rng.below(10) == 0 { rng.below(5) } else { 0 },
    });
    if rng.below(12) == 0 {
        c["error"] = error_spec(rng);
    }
    c
}

fn max_size(rng: &mut SplitMix64) -> i64 {
    match rng.below(10) {
        0 => pick(rng, &[0, -1, 100, 200, 250, 300, 400, 1000]),
        1 => random_i64(rng),
        2 => 1_000_000,
        _ => 393_216,
    }
}

/// A case of random shape: outputs, rate, sources and size limit drawn
/// from distributions that favour the boundaries.
pub fn random_author_case(rng: &mut SplitMix64) -> Value {
    let n_out = match rng.below(20) {
        0 => 0,
        1 => 250 + rng.below(10) as usize,
        _ => 1 + rng.below(4) as usize,
    };
    let outputs: Vec<Value> = (0..n_out).map(|_| output(rng)).collect();
    json!({
        "outputs": outputs,
        "rate": rate(rng),
        "max": max_size(rng),
        "source": source(rng),
        "change": change(rng),
    })
}

fn p2pkh_like(len: usize) -> Value {
    json!(hex(&vec![0u8; len]))
}

/// The fee for one P2PKH input paying outputs with script lengths `lens`
/// plus a change script of `change`, at `rate`.
fn fee_for(rate: i64, n_in: usize, lens: &[usize], change: i64) -> i64 {
    let outs: Vec<TxOut> = lens
        .iter()
        .map(|&l| TxOut {
            value: 0,
            version: 0,
            pk_script: vec![0; l],
        })
        .collect();
    let sizes = vec![REDEEM_P2PKH_SIG_SCRIPT_SIZE; n_in];
    fee_for_serialize_size(Amount(rate), estimate_serialize_size(&sizes, &outs, change)).0
}

/// The smallest change amount that is not dust for a script of `size` at
/// `rate` (both small and positive).
fn dust_threshold(size: i64, rate: i64) -> i64 {
    let total = 8 + 2 + if size < 0xfd { 1 } else { 3 } + size + 165;
    // amount * 1000 / (3 * total) >= rate
    (rate * 3 * total + 999) / 1000
}

/// Cases built around the loop's edges: change exactly at, just below and
/// just above the dust threshold, exact payment with no change,
/// insufficient balance by one atom, a second input needed or not, the
/// size limit at the estimate and one below, high fee rates, change
/// scripts at and over the 2048-byte limit, wrapped targets, and Go's
/// endless loop (QK-0744, QK-0753), from an empty source and from one
/// whose input covers the output total, which the source's call limit cuts
/// in Go and where the crate returns an error.
pub fn edge_author_cases(rng: &mut SplitMix64) -> Vec<Value> {
    let mut cases = Vec::new();
    let change25 = json!({ "size": 25, "script": hex(&[0u8; 25]), "version": 0 });
    for &rate in &[0i64, 1, 1000, 2550, 10_000, 100_000, 10_000_000] {
        for &(change_size, lens) in &[(25i64, &[36usize][..]), (25, &[25, 23][..]), (26, &[][..])] {
            let change = json!({
                "size": change_size,
                "script": hex(&vec![0u8; change_size as usize]),
                "version": 0,
            });
            let fee = fee_for(rate, 1, lens, change_size);
            let dust = dust_threshold(change_size, rate);
            let utxo = 1_000_000_000i64;
            for delta in [-2i64, -1, 0, 1, 2] {
                // Change of dust + delta: below the threshold it is dropped.
                let pay = utxo - fee - (dust + delta);
                let n = lens.len().max(1);
                let mut outputs: Vec<Value> = lens
                    .iter()
                    .map(|&l| json!({ "value": 0, "version": 0, "script": p2pkh_like(l) }))
                    .collect();
                if outputs.is_empty() {
                    // A sweep: the change is the only output.
                } else {
                    outputs[0]["value"] = json!(pay - (n as i64 - 1));
                    for o in outputs.iter_mut().skip(1) {
                        o["value"] = json!(1);
                    }
                }
                let utxo_value = if lens.is_empty() {
                    fee + dust + delta
                } else {
                    utxo
                };
                cases.push(json!({
                    "outputs": outputs,
                    "rate": rate,
                    "max": 393_216,
                    "source": { "utxos": [[utxo_value, 108, 25], [utxo, 108, 25]] },
                    "change": change,
                }));
            }
            // Exact payment, no change; and one atom short.
            for short in [0i64, 1] {
                let outputs: Vec<Value> = lens
                    .iter()
                    .enumerate()
                    .map(|(i, &l)| {
                        json!({
                            "value": if i == 0 { 50_000_000 } else { 0 },
                            "version": 0,
                            "script": p2pkh_like(l),
                        })
                    })
                    .collect();
                cases.push(json!({
                    "outputs": outputs,
                    "rate": rate,
                    "max": 393_216,
                    "source": { "utxos": [[50_000_000 + fee - short, 108, 25]] },
                    "change": change,
                }));
            }
        }
    }
    // A second input pays for itself or not.
    for extra in [-1i64, 0, 1, 100, 1_000] {
        let fee2 = fee_for(10_000, 2, &[25], 25);
        cases.push(json!({
            "outputs": [{ "value": 100_000_000, "version": 0, "script": p2pkh_like(25) }],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[60_000_000, 108, 25], [40_000_000 + fee2 + extra, 108, 25], [5, 108, 25]] },
            "change": change25,
        }));
    }
    // The size limit at the estimate, one below, and the dust case where
    // the limit counts a change output that is not added.
    let est = estimate_serialize_size(
        &[REDEEM_P2PKH_SIG_SCRIPT_SIZE],
        &[TxOut {
            value: 0,
            version: 0,
            pk_script: vec![0; 25],
        }],
        25,
    );
    for max in [est - 1, est, est + 1, 0, -1] {
        for pay in [
            10_000_000i64,
            100_000_000 - fee_for(10_000, 1, &[25], 25) - 100,
        ] {
            cases.push(json!({
                "outputs": [{ "value": pay, "version": 0, "script": p2pkh_like(25) }],
                "rate": 10_000,
                "max": max,
                "source": { "utxos": [[100_000_000, 108, 25]] },
                "change": change25,
            }));
        }
    }
    // High fee rates and the MaxAmount clamp.
    for &r in &[1_000_000_000i64, MAX_AMOUNT, i64::MAX / 100, i64::MAX] {
        cases.push(json!({
            "outputs": [{ "value": 1, "version": 0, "script": p2pkh_like(25) }],
            "rate": r,
            "max": 393_216,
            "source": { "utxos": [[MAX_AMOUNT, 108, 25], [MAX_AMOUNT, 108, 25]] },
            "change": change25,
        }));
    }
    // Change scripts at and over the stack element limit, with and without
    // a matching declared size.
    for len in [2047usize, 2048, 2049] {
        for size in [25i64, len as i64] {
            cases.push(json!({
                "outputs": [{ "value": 1_000_000, "version": 0, "script": p2pkh_like(25) }],
                "rate": 10_000,
                "max": 393_216,
                "source": { "utxos": [[100_000_000, 108, 25]] },
                "change": { "size": size, "script": hex(&vec![0x51u8; len]), "version": 0 },
            }));
        }
    }
    // Output totals that wrap, and the endless loop: a total within the
    // fee of i64::MAX keeps the target negative and the source empty
    // (QK-0744), and so does a total of i64::MIN, where the remaining
    // amount wraps instead (QK-0753).
    let f0 = fee_for(10_000, 0, &[25], 25);
    for total in [
        i64::MAX,
        i64::MAX - f0 + 1,
        i64::MAX - f0,
        i64::MAX - 2 * f0,
        i64::MIN,
        i64::MAX / 2 + 1,
    ] {
        for limit in [3u64, 7] {
            cases.push(json!({
                "outputs": [
                    { "value": total, "version": 0, "script": p2pkh_like(25) },
                ],
                "rate": 10_000,
                "max": 393_216,
                "source": { "utxos": [[100_000_000, 108, 25]], "limit": limit },
                "change": change25,
            }));
        }
        // Two outputs whose sum wraps.
        cases.push(json!({
            "outputs": [
                { "value": total, "version": 0, "script": p2pkh_like(25) },
                { "value": total, "version": 0, "script": p2pkh_like(25) },
            ],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[100_000_000, 108, 25], [i64::MAX, 108, 25]], "limit": 5 },
            "change": change25,
        }));
    }
    // The endless loop with a source that answers (QK-0744): the first
    // target does not wrap, the one input covers the output total, so the
    // remaining amount is not negative (3,000 atoms and 11,469), but it is
    // below the fee for the input's 1,000-byte signature script (11,470),
    // and the retry for that fee wraps; the source answers the wrapped
    // target with the same input. With one atom more of room the input
    // pays its fee and Go ends. No rng draw, so the random cases after
    // these are unchanged.
    for total in [i64::MAX - 3_000, i64::MAX - 11_469, i64::MAX - 11_470] {
        cases.push(json!({
            "outputs": [{ "value": total, "version": 0, "script": p2pkh_like(25) }],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[i64::MAX, 1_000, 25]], "limit": 5 },
            "change": change25,
        }));
    }
    // A change size within 19 of i64::MAX wraps the change output's
    // estimated size and so the estimate the size limit is checked against:
    // the limit of 0 passes, and without change Go reports the estimate
    // without the change output, 15 bytes, above the limit.
    cases.push(json!({
        "outputs": [],
        "rate": 0,
        "max": 0,
        "source": { "utxos": [] },
        "change": { "size": i64::MAX - 10, "script": hex(&[0x76; 2]), "version": 0 },
    }));
    // Source and change errors of every shape, at each call.
    for at in 1..=3u64 {
        let e = error_spec(rng);
        cases.push(json!({
            "outputs": [{ "value": 150_000_000, "version": 0, "script": p2pkh_like(25) }],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[100_000_000, 108, 25], [100_000_000, 108, 25]], "fail_at": at, "error": e },
            "change": change25,
        }));
    }
    for _ in 0..6 {
        let e = error_spec(rng);
        cases.push(json!({
            "outputs": [{ "value": 1_000_000, "version": 0, "script": p2pkh_like(25) }],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[100_000_000, 108, 25]] },
            "change": { "size": 25, "script": hex(&[0u8; 25]), "version": 0, "error": e },
        }));
    }
    // A source that reports more than it holds (delta), and less.
    for delta in [1i64, -1, 1_000_000, i64::MIN, i64::MAX] {
        cases.push(json!({
            "outputs": [{ "value": 1_000_000, "version": 0, "script": p2pkh_like(25) }],
            "rate": 10_000,
            "max": 393_216,
            "source": { "utxos": [[1_000_000, 108, 25]], "delta": delta },
            "change": change25,
        }));
    }
    cases
}

pub fn randomize_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let len = match i % 7 {
            0 => 0,
            1 => 1,
            2 => pick(rng, &[2usize, 3, 4, 8, 16, 255, 256]),
            3 => 200 + rng.below(100) as usize,
            _ => 2 + rng.below(9) as usize,
        };
        let calls: Vec<Value> = (0..1 + rng.below(4))
            .map(|_| {
                let index = match rng.below(10) {
                    0 => -1,
                    1 => len as i64,
                    2 => pick(rng, &[i64::MIN, i64::MAX, -2, len as i64 + 1]),
                    _ if len > 0 => rng.below(len as u64) as i64,
                    _ => 0,
                };
                json!([index, rng.below(2) == 0])
            })
            .collect();
        out.push(json!({ "key": hex(&rng.bytes(32)), "n": len, "calls": calls }));
    }
    out
}

// ---------------------------------------------------------------------
// Corpora: signing.
// ---------------------------------------------------------------------

/// A key of one of the three signature suites, with what the secrets
/// source returns for it.
#[derive(Clone)]
pub struct Key {
    /// The private key bytes the source returns (32 for secp256k1, the
    /// 64-byte seed and public key for Ed25519).
    pub priv_bytes: Vec<u8>,
    pub sig_type: i64,
    /// The serialized public key (compressed or not for ECDSA).
    pub pub_key: Vec<u8>,
    pub compressed: bool,
}

pub fn secp_key(rng: &mut SplitMix64, compressed: bool, sig_type: i64) -> Key {
    loop {
        let bytes: [u8; 32] = rng.bytes(32).try_into().expect("32");
        if let Some(k) = secp256k1::PrivateKey::from_bytes(&bytes) {
            let pk = k.public_key();
            let pub_key = if compressed {
                pk.serialize_compressed().to_vec()
            } else {
                pk.serialize_uncompressed().to_vec()
            };
            return Key {
                priv_bytes: bytes.to_vec(),
                sig_type,
                pub_key,
                compressed,
            };
        }
    }
}

pub fn ed_key(rng: &mut SplitMix64) -> Key {
    let seed: [u8; 32] = rng.bytes(32).try_into().expect("32");
    let pub_key = edwards::SecretKey::from_seed(seed).public_key().serialize();
    let mut priv_bytes = seed.to_vec();
    priv_bytes.extend_from_slice(&pub_key);
    Key {
        priv_bytes,
        sig_type: 1,
        pub_key: pub_key.to_vec(),
        compressed: true,
    }
}

fn key_json(k: &Key) -> Value {
    json!([hex(&k.priv_bytes), k.sig_type, k.compressed])
}

fn pkh_addr(k: &Key, p: &Params) -> Address {
    let h = stdaddr::hash160(&k.pub_key);
    match k.sig_type {
        0 => stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&h, p).expect("addr"),
        1 => stdaddr::new_address_pub_key_hash_ed25519_v0(&h, p).expect("addr"),
        _ => stdaddr::new_address_pub_key_hash_schnorr_secp256k1_v0(&h, p).expect("addr"),
    }
}

fn pk_addr(k: &Key, p: &Params) -> Address {
    match k.sig_type {
        0 => stdaddr::new_address_pub_key_ecdsa_secp256k1_v0_raw(&k.pub_key, p).expect("addr"),
        1 => stdaddr::new_address_pub_key_ed25519_v0_raw(&k.pub_key, p).expect("addr"),
        _ => stdaddr::new_address_pub_key_schnorr_secp256k1_v0_raw(&k.pub_key, p).expect("addr"),
    }
}

/// A 33-byte compressed-form key that does not parse as a point.
fn invalid_secp_pub_key(rng: &mut SplitMix64) -> Vec<u8> {
    loop {
        let mut k = vec![0x02];
        k.extend_from_slice(&rng.bytes(32));
        if secp256k1::PublicKey::parse(&k).is_err() {
            return k;
        }
    }
}

/// A pay-to-pubkey script of a random suite whose key does not parse:
/// compressed or uncompressed ECDSA (`OP_CHECKSIG`), Schnorr (`OP_2
/// OP_CHECKSIGALT`) or Ed25519 (`OP_1 OP_CHECKSIGALT`).
fn invalid_pub_key_script(rng: &mut SplitMix64) -> Vec<u8> {
    match rng.below(4) {
        0 => {
            let mut s = vec![0x21];
            s.extend_from_slice(&invalid_secp_pub_key(rng));
            s.push(0xac);
            s
        }
        1 => loop {
            let mut k = vec![0x04];
            k.extend_from_slice(&rng.bytes(64));
            if secp256k1::PublicKey::parse(&k).is_err() {
                let mut s = vec![0x41];
                s.extend_from_slice(&k);
                s.push(0xac);
                return s;
            }
        },
        2 => {
            let mut s = vec![0x21];
            s.extend_from_slice(&invalid_secp_pub_key(rng));
            s.extend_from_slice(&[0x52, 0xbe]);
            s
        }
        _ => loop {
            let k = rng.bytes(32);
            if edwards::parse_pub_key(&k).is_err() {
                let mut s = vec![0x20];
                s.extend_from_slice(&k);
                s.extend_from_slice(&[0x51, 0xbe]);
                return s;
            }
        },
    }
}

/// One input's previous script and what the sources hold for it.
struct Prev {
    script: Vec<u8>,
    keys: Vec<(String, Value)>,
    scripts: Vec<(String, Value)>,
    key_errors: Vec<(String, Value)>,
    script_errors: Vec<(String, Value)>,
}

impl Prev {
    fn new(script: Vec<u8>) -> Prev {
        Prev {
            script,
            keys: Vec::new(),
            scripts: Vec::new(),
            key_errors: Vec::new(),
            script_errors: Vec::new(),
        }
    }
}

fn multisig(keys: &[Key], threshold: i64) -> Vec<u8> {
    let pks: Vec<&[u8]> = keys.iter().map(|k| k.pub_key.as_slice()).collect();
    stdscript::multi_sig_script_v0(threshold, &pks).expect("multisig script")
}

fn p2sh(redeem: &[u8], p: &Params) -> (Address, Vec<u8>) {
    let a = stdaddr::new_address_script_hash_v0(redeem, p).expect("p2sh");
    let script = a.payment_script().1;
    (a, script)
}

/// A previous script of kind `kind` (see the match) and its secrets.
fn prev_script(rng: &mut SplitMix64, kind: u64, p: &Params) -> Prev {
    let ecdsa = |rng: &mut SplitMix64| secp_key(rng, true, 0);
    match kind {
        // P2PKH, ECDSA, compressed or not.
        0 | 1 => {
            let k = secp_key(rng, kind == 0, 0);
            let a = pkh_addr(&k, p);
            let mut v = Prev::new(a.payment_script().1);
            v.keys.push((a.encode(), key_json(&k)));
            v
        }
        // P2PK of each suite.
        2 => {
            if rng.below(4) == 0 {
                // Uncompressed: no address constructor takes that form,
                // so the script is written out; the address a signer
                // extracts encodes the compressed key.
                let k = secp_key(rng, false, 0);
                let compressed = secp256k1::PublicKey::parse(&k.pub_key)
                    .expect("key")
                    .serialize_compressed();
                let a = stdaddr::new_address_pub_key_ecdsa_secp256k1_v0_raw(&compressed, p)
                    .expect("addr");
                let mut script = vec![0x41];
                script.extend_from_slice(&k.pub_key);
                script.push(0xac);
                let mut v = Prev::new(script);
                v.keys.push((a.encode(), key_json(&k)));
                return v;
            }
            let k = match rng.below(3) {
                0 => ed_key(rng),
                1 => secp_key(rng, true, 2),
                _ => ecdsa(rng),
            };
            let a = pk_addr(&k, p);
            let mut v = Prev::new(a.payment_script().1);
            v.keys.push((a.encode(), key_json(&k)));
            v
        }
        // P2PKH of the alternative suites.
        3 => {
            let k = if rng.below(2) == 0 {
                ed_key(rng)
            } else {
                secp_key(rng, true, 2)
            };
            let a = pkh_addr(&k, p);
            let mut v = Prev::new(a.payment_script().1);
            v.keys.push((a.encode(), key_json(&k)));
            v
        }
        // P2SH of an m-of-3 multisig, some keys held.
        4 => {
            let keys: Vec<Key> = (0..3).map(|_| ecdsa(rng)).collect();
            let m = 1 + rng.below(3) as i64;
            let redeem = multisig(&keys, m);
            let (a, script) = p2sh(&redeem, p);
            let mut v = Prev::new(script);
            v.scripts.push((a.encode(), json!(hex(&redeem))));
            for k in &keys {
                if rng.below(3) != 0 {
                    v.keys.push((pk_addr(k, p).encode(), key_json(k)));
                }
            }
            v
        }
        // P2SH of a P2PKH script.
        5 => {
            let k = ecdsa(rng);
            let inner = pkh_addr(&k, p);
            let redeem = inner.payment_script().1;
            let (a, script) = p2sh(&redeem, p);
            let mut v = Prev::new(script);
            v.scripts.push((a.encode(), json!(hex(&redeem))));
            v.keys.push((inner.encode(), key_json(&k)));
            v
        }
        // Bare 1-of-2 or 2-of-2 multisig.
        6 => {
            let keys: Vec<Key> = (0..2).map(|_| ecdsa(rng)).collect();
            let m = 1 + rng.below(2) as i64;
            let mut v = Prev::new(multisig(&keys, m));
            for k in &keys {
                if rng.below(4) != 0 {
                    v.keys.push((pk_addr(k, p).encode(), key_json(k)));
                }
            }
            v
        }
        // Stake-tagged P2PKH and treasury generation.
        7 => {
            let k = ecdsa(rng);
            let a = pkh_addr(&k, p);
            let script = match rng.below(5) {
                0 => a.voting_rights_script(),
                1 => a.stake_change_script(),
                2 => a.pay_vote_commitment_script(),
                3 => a.pay_revoke_commitment_script(),
                _ => a.pay_from_treasury_script(),
            }
            .expect("stake script")
            .1;
            let mut v = Prev::new(script);
            v.keys.push((a.encode(), key_json(&k)));
            v
        }
        // Stake-tagged P2SH of a P2PKH script.
        8 => {
            let k = ecdsa(rng);
            let inner = pkh_addr(&k, p);
            let redeem = inner.payment_script().1;
            let a = stdaddr::new_address_script_hash_v0(&redeem, p).expect("p2sh");
            let script = match rng.below(5) {
                0 => a.voting_rights_script(),
                1 => a.stake_change_script(),
                2 => a.pay_vote_commitment_script(),
                3 => a.pay_revoke_commitment_script(),
                _ => a.pay_from_treasury_script(),
            }
            .expect("stake script")
            .1;
            let mut v = Prev::new(script);
            v.scripts.push((a.encode(), json!(hex(&redeem))));
            v.keys.push((inner.encode(), key_json(&k)));
            v
        }
        // Null data and nonstandard scripts.
        9 => Prev::new(if rng.below(2) == 0 {
            vec![0x6a, 0x04, 1, 2, 3, 4]
        } else {
            let len = 1 + rng.below(30) as usize;
            rng.bytes(len)
        }),
        // P2PKH whose key is missing or whose lookup fails.
        10 => {
            let k = ecdsa(rng);
            let a = pkh_addr(&k, p);
            let mut v = Prev::new(a.payment_script().1);
            if rng.below(2) == 0 {
                v.key_errors.push((a.encode(), error_spec(rng)));
            }
            v
        }
        // P2SH whose script is missing or whose lookup fails.
        11 => {
            let k = ecdsa(rng);
            let redeem = pkh_addr(&k, p).payment_script().1;
            let (a, script) = p2sh(&redeem, p);
            let mut v = Prev::new(script);
            if rng.below(2) == 0 {
                v.script_errors.push((a.encode(), error_spec(rng)));
            }
            v
        }
        // P2PK whose key is not a point (Go panics in sign), in each of
        // the four shapes stdscript classifies without parsing the key.
        12 => Prev::new(invalid_pub_key_script(rng)),
        // P2SH whose redeem script is such a P2PK.
        13 => {
            let redeem = invalid_pub_key_script(rng);
            let (a, script) = p2sh(&redeem, p);
            let mut v = Prev::new(script);
            v.scripts.push((a.encode(), json!(hex(&redeem))));
            v
        }
        // P2SH whose redeem script is itself P2SH, of anything: the
        // second script is looked up but never signed. An invalid P2PK
        // keeps the new signature script's pushes aligned; random bytes
        // often do not, or do not parse (QK-0746; edge_sign_cases has the
        // deliberate shapes).
        _ => {
            let inner = if rng.below(2) == 0 {
                let mut inner = vec![0x21];
                inner.extend_from_slice(&invalid_secp_pub_key(rng));
                inner.push(0xac);
                inner
            } else {
                let len = rng.below(40) as usize;
                rng.bytes(len)
            };
            let (inner_addr, redeem) = p2sh(&inner, p);
            let (a, script) = p2sh(&redeem, p);
            let mut v = Prev::new(script);
            v.scripts.push((a.encode(), json!(hex(&redeem))));
            if rng.below(2) == 0 {
                v.scripts.push((inner_addr.encode(), json!(hex(&inner))));
            }
            v
        }
    }
}

pub const PREV_KINDS: u64 = 15;

/// A signing case: a transaction of one to four inputs, each spending a
/// previous script of a random kind, with some inputs carrying a
/// signature script already (merged), and occasionally one previous
/// script too many or too few.
pub fn sign_case(rng: &mut SplitMix64, kinds: &[u64]) -> Value {
    let net = pick(rng, &NETS);
    let p = params(net);
    let mut tx = MsgTx::default();
    let mut prev = Vec::new();
    let mut keys = BTreeMap::new();
    let mut scripts = BTreeMap::new();
    let mut key_errors = BTreeMap::new();
    let mut script_errors = BTreeMap::new();
    for (i, &kind) in kinds.iter().enumerate() {
        let v = prev_script(rng, kind, &p);
        let sig = match if kind == 14 {
            rng.below(2)
        } else {
            rng.below(8)
        } {
            0 => {
                let len = 1 + rng.below(200) as usize;
                rng.bytes(len)
            }
            1 => vec![0x00],
            _ => Vec::new(),
        };
        tx.tx_in.push(new_tx_in(
            OutPoint {
                hash: Hash(rng.bytes(32).try_into().expect("32")),
                index: rng.below(4) as u32,
                tree: (i % 2) as i8,
            },
            1 + rng.below(1 << 40) as i64,
            sig,
        ));
        prev.push(json!(hex(&v.script)));
        keys.extend(v.keys);
        scripts.extend(v.scripts);
        key_errors.extend(v.key_errors);
        script_errors.extend(v.script_errors);
    }
    tx.tx_out.push(TxOut {
        value: 1 + rng.below(1 << 40) as i64,
        version: 0,
        pk_script: rng.bytes(25),
    });
    if rng.below(3) == 0 {
        tx.version = 3;
        tx.expiry = rng.below(1000) as u32;
    }
    if rng.below(25) == 0 {
        if rng.below(2) == 0 || prev.is_empty() {
            prev.push(json!("76a914"));
        } else {
            prev.pop();
        }
    }
    json!({
        "net": net,
        "tx": hex(&tx.serialize()),
        "prev": prev,
        "keys": keys,
        "scripts": scripts,
        "key_errors": key_errors,
        "script_errors": script_errors,
    })
}

pub fn sign_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let mut out = Vec::with_capacity(n + PREV_KINDS as usize);
    // Every kind alone first, then random mixes.
    for kind in 0..PREV_KINDS {
        out.push(sign_case(rng, &[kind]));
    }
    for _ in 0..n {
        let k = 1 + rng.below(4) as usize;
        let kinds: Vec<u64> = (0..k).map(|_| rng.below(PREV_KINDS)).collect();
        out.push(sign_case(rng, &kinds));
    }
    out
}

/// A mainnet signing case of one input spending `prev_pk` with the existing
/// signature script `sig`, and the sources' keys and scripts by address.
fn one_input_case(
    rng: &mut SplitMix64,
    prev_pk: &[u8],
    sig: &[u8],
    keys: &[(String, Value)],
    scripts: &[(String, Value)],
) -> Value {
    let mut tx = MsgTx::default();
    tx.tx_in.push(new_tx_in(
        OutPoint {
            hash: Hash(rng.bytes(32).try_into().expect("32")),
            index: 0,
            tree: 0,
        },
        1 + rng.below(1 << 40) as i64,
        sig.to_vec(),
    ));
    tx.tx_out.push(TxOut {
        value: 1 + rng.below(1 << 40) as i64,
        version: 0,
        pk_script: rng.bytes(25),
    });
    let map = |v: &[(String, Value)]| -> BTreeMap<String, Value> { v.iter().cloned().collect() };
    json!({
        "net": "mainnet",
        "tx": hex(&tx.serialize()),
        "prev": [hex(prev_pk)],
        "keys": map(keys),
        "scripts": map(scripts),
        "key_errors": {},
        "script_errors": {},
    })
}

/// `n` bytes of `OP_1`, then `tail`.
fn ones_then(n: usize, tail: &[u8]) -> Vec<u8> {
    let mut v = vec![0x51; n];
    v.extend_from_slice(tail);
    v
}

/// A push of a pay-to-script-hash script for a random hash.
fn p2sh_push(rng: &mut SplitMix64) -> Vec<u8> {
    let mut v = vec![0x17, 0xa9, 0x14];
    v.extend_from_slice(&rng.bytes(20));
    v.push(0x87);
    v
}

/// How many of the nested cases near the 16384-byte script limit to make.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Big {
    /// None.
    No,
    /// One case of each signature-script shape, for the frozen file.
    Some,
    /// Every one.
    All,
}

/// Second redeem scripts for the nested pay-to-script-hash cases (QK-0746),
/// each with the shape of signature script it gives: Go's new signature
/// script is the second script's opcodes, then a push of the first redeem
/// script, each step dropped when it would exceed the script limits
/// (16384 bytes), and the merge recurses forever only when that script is
/// nonempty, parses and ends in a push of a plain pay-to-script-hash
/// script. `big` adds the scripts near the 16384-byte limit; each comes
/// with whether only a stake-tagged first redeem script is used with it
/// (`Some(true)`), only a plain one (`Some(false)`), or both (`None`).
fn nested_second_scripts(rng: &mut SplitMix64, big: Big) -> Vec<(Vec<u8>, Option<bool>)> {
    let mut v: Vec<(Vec<u8>, Option<bool>)> = [
        // The first redeem script is the only push.
        Vec::new(),
        vec![0x51],
        // A one-byte push swallows the push opcode of the first redeem
        // script: the script parses but ends in OP_EQUAL, no data.
        vec![0x01],
        vec![0x02, 0x00],
        // A 23-byte push takes the push opcode and 22 bytes.
        vec![0x17],
        // OP_PUSHDATA1 reads the push opcode (23) as its length and so
        // pushes exactly the first redeem script.
        vec![0x4c],
        // OP_PUSHDATA2 runs past the end: the script does not parse.
        vec![0x4d],
        // Already ending in a pay-to-script-hash push.
        p2sh_push(rng),
    ]
    .into_iter()
    .map(|x| (x, None))
    .collect();
    let some = big == Big::Some;
    if big != Big::No {
        let both = |frozen: Option<bool>| if some { frozen } else { None };
        v.extend([
            // Too long for AddOps: the new script is empty, and Go keeps
            // the existing one.
            (vec![0x51; 16_385], both(Some(false))),
            // AddOps fits, the push of the first redeem script does not:
            // the new script is the second script alone, here ending in
            // OP_1 (no recursion) or in a pay-to-script-hash push (Go
            // recurses, with a plain or a stake-tagged first redeem
            // script).
            (vec![0x51; 16_370], both(Some(false))),
            (ones_then(16_346, &p2sh_push(rng)), both(Some(false))),
            (ones_then(16_360, &p2sh_push(rng)), both(Some(true))),
            (
                ones_then(16_363, &{
                    let mut push = vec![0x14];
                    push.extend_from_slice(&rng.bytes(20));
                    push
                }),
                both(Some(false)),
            ),
        ]);
        if !some {
            v.push((vec![0x51; 16_400], None));
        }
    }
    v
}

/// The nested pay-to-script-hash cases (QK-0746): a previous output paying
/// to the hash of a first redeem script R1 that pays to the hash of a
/// second script X, with every X of [`nested_second_scripts`], R1 plain or
/// stake-tagged (`OP_SSTX`), the previous output plain, and for the short X
/// stake-tagged too, with existing signature scripts that are mergeable
/// (`[0x00]`, `[0x51]`) or not (empty, `[0x01]`); the long X only with
/// `[0x51]`.
pub fn nested_p2sh_cases(rng: &mut SplitMix64, big: Big) -> Vec<Value> {
    let p = params("mainnet");
    let mut out = Vec::new();
    for (x, only_stake_r1) in nested_second_scripts(rng, big) {
        let x_addr = stdaddr::new_address_script_hash_v0(&x, &p).expect("p2sh");
        let plain_r1 = x_addr.payment_script().1;
        let stake_r1 = x_addr.voting_rights_script().expect("stake script").1;
        let small = x.len() < 100;
        for (r1, stake_r1_used, stake_prev) in [
            (&plain_r1, false, false),
            (&stake_r1, true, false),
            (&plain_r1, false, true),
        ] {
            if (stake_prev && !small) || only_stake_r1.is_some_and(|s| s != stake_r1_used) {
                continue;
            }
            let r1_addr = stdaddr::new_address_script_hash_v0(r1, &p).expect("p2sh");
            let prev_pk = if stake_prev {
                r1_addr.voting_rights_script().expect("stake script").1
            } else {
                r1_addr.payment_script().1
            };
            let scripts = [
                (r1_addr.encode(), json!(hex(r1))),
                (x_addr.encode(), json!(hex(&x))),
            ];
            let sigs: &[&[u8]] = if small {
                &[&[0x00], &[0x51], &[], &[0x01]]
            } else {
                &[&[0x51]]
            };
            for sig in sigs {
                out.push(one_input_case(rng, &prev_pk, sig, &[], &scripts));
            }
        }
    }
    out
}

/// The secp256k1 group order N, big-endian.
const GROUP_ORDER: [u8; 32] = secp256k1::GROUP_ORDER_BYTES;

/// `GROUP_ORDER + k` for small k, big-endian, 32 bytes.
fn order_plus(k: u8) -> Vec<u8> {
    let mut v = GROUP_ORDER.to_vec();
    let (last, carry) = v[31].overflowing_add(k);
    v[31] = last;
    assert!(!carry, "no carry for small k");
    v
}

/// Private keys Go's `PrivKeyFromBytes` reads but which are not a 32-byte
/// scalar in [1, N): shorter and longer than 32 bytes, N + 1, all ones,
/// and the ones that reduce to zero (zero, N, empty).
pub fn odd_secp_keys(rng: &mut SplitMix64) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let nonzero = vec![
        vec![0x01; 14],
        vec![0x01; 31],
        vec![0x01; 33],
        rng.bytes(40),
        order_plus(1),
        order_plus(5),
        vec![0xff; 32],
        // A short key, as hdkeychain's stored form can be (QK-0684).
        vec![0x07; 3],
    ];
    let zero = vec![vec![0; 32], GROUP_ORDER.to_vec(), Vec::new(), vec![0; 5]];
    (nonzero, zero)
}

/// Signing cases with the keys of [`odd_secp_keys`]: a P2PKH (ECDSA
/// compressed and uncompressed, Schnorr), a P2PK (ECDSA, Schnorr) and a
/// 1-of-2 bare multisig previous script (whose first key is the odd one,
/// the second a valid ECDSA key). With `zero` the keys that reduce to zero
/// are used, ECDSA excluded: Go signs with those (QK-0682), see
/// [`zero_ecdsa_key_cases`].
pub fn odd_key_cases(rng: &mut SplitMix64, zero: bool) -> Vec<Value> {
    let p = params("mainnet");
    let (nonzero, zeros) = odd_secp_keys(rng);
    let keys = if zero { zeros } else { nonzero };
    let mut out = Vec::new();
    for key in keys {
        let mut suites = vec![(2i64, true)];
        if !zero {
            suites.extend([(0, true), (0, false)]);
        }
        for (sig_type, compressed) in suites {
            let holder = secp_key(rng, true, sig_type);
            let odd = json!([hex(&key), sig_type, compressed]);
            let a = pkh_addr(&holder, &p);
            out.push(one_input_case(
                rng,
                &a.payment_script().1,
                &[],
                &[(a.encode(), odd.clone())],
                &[],
            ));
            let a = pk_addr(&holder, &p);
            out.push(one_input_case(
                rng,
                &a.payment_script().1,
                &[],
                &[(a.encode(), odd.clone())],
                &[],
            ));
            let other = secp_key(rng, true, 0);
            let ms = multisig(&[secp_key(rng, true, 0), other.clone()], 1);
            let first = stdscript::extract_addrs_v0(&ms, &p).1[0].encode();
            out.push(one_input_case(
                rng,
                &ms,
                &[],
                &[
                    (first, odd),
                    (pk_addr(&other, &p).encode(), key_json(&other)),
                ],
                &[],
            ));
        }
    }
    // Ed25519 keys are the 64-byte seed and public key in Go as here.
    let holder = ed_key(rng);
    let a = pkh_addr(&holder, &p);
    for len in [63, 65] {
        let mut k = holder.priv_bytes.clone();
        k.resize(len, 0);
        out.push(one_input_case(
            rng,
            &a.payment_script().1,
            &[],
            &[(a.encode(), json!([hex(&k), 1, true]))],
            &[],
        ));
    }
    out
}

/// P2PKH and multisig cases signed with ECDSA keys that reduce to zero,
/// which Go signs with and the crate refuses (`invalid privkey`; ADR-0012
/// Rule 7, QK-0682). They are kept out of the differential and checked
/// against Go by `zero_ecdsa_keys_diverge`.
pub fn zero_ecdsa_key_cases(rng: &mut SplitMix64) -> Vec<Value> {
    let p = params("mainnet");
    let (_, zeros) = odd_secp_keys(rng);
    let mut out = Vec::new();
    for key in zeros {
        let holder = secp_key(rng, true, 0);
        let a = pkh_addr(&holder, &p);
        out.push(one_input_case(
            rng,
            &a.payment_script().1,
            &[],
            &[(a.encode(), json!([hex(&key), 0, true]))],
            &[],
        ));
    }
    out
}

/// The deterministic signing cases: the nested pay-to-script-hash shapes
/// and the odd keys. `big` selects the nested shapes near the script size
/// limit, about 33 KB of hex each.
pub fn edge_sign_cases(rng: &mut SplitMix64, big: Big) -> Vec<Value> {
    let mut out = nested_p2sh_cases(rng, big);
    out.extend(odd_key_cases(rng, false));
    out.extend(odd_key_cases(rng, true));
    out
}

// ---------------------------------------------------------------------
// Frozen vector files.
// ---------------------------------------------------------------------

/// The frozen vector file.
pub const VECTORS_FILE: &str = "txauthor_vectors.txt";

/// The fixed PRNG seed its corpus is generated from.
pub const VECTORS_SEED: u64 = 0x7478_6175_7468_6f72;

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// The SHA-256 of a vector file's body: every line that is not a comment
/// (nor empty), each followed by a newline, in order.
pub fn body_sha256(text: &str) -> String {
    let mut h = Sha256::new();
    for line in body_lines(text) {
        h.update(line.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

fn body_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

/// The `# entries:` and `# body-sha256:` header lines for a body.
pub fn integrity_header(body: &str) -> String {
    format!(
        "# entries: {}\n# body-sha256: {}\n",
        body_lines(body).count(),
        body_sha256(body)
    )
}

/// Checks a vector file's integrity headers against its body and returns
/// its entries, each `{"cmd": name, "in": input, "out": Go's answer}`.
pub fn read_vectors(name: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(data_dir().join(name))
        .unwrap_or_else(|e| panic!("read tests/data/{name}: {e}"));
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{name}: no {key:?} header"))
            .to_owned()
    };
    let entries: usize = field("# entries:").parse().expect("entry count");
    assert_eq!(body_lines(&text).count(), entries, "{name}: entry count");
    assert_eq!(
        body_sha256(&text),
        field("# body-sha256:"),
        "{name}: body hash"
    );
    body_lines(&text)
        .map(|l| purse_testutil::serde_json::from_str(l).expect("vector line"))
        .collect()
}

/// Wraps `text` into `#` comment lines of at most 78 columns.
pub fn comment(text: &str) -> String {
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
