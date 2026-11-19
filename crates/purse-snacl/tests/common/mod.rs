// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the differential, frozen, fixture and database tests: the Rust
//! side's answers to the oracle's commands (`tools/oracle/cmd/snacl`, whose
//! header documents each command and the outcome shape), the input corpus,
//! the vector files and the wallet database walk.

#![allow(dead_code, reason = "each test binary uses a subset")]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: indices and counters on small, bounded values"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use purse_boltread::{Db, OpenOptions};
use purse_errors::{Kind, MULTILINE_SEPARATOR};
use purse_snacl::{
    AccountKey, Argon2idParams, CryptoKey, Error, Limits, Parameters, RandomSource,
    RandomSourceError, SecretKey,
};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{SplitMix64, hex, unhex};
use sha2::{Digest, Sha256};

/// The general vector file, relative to `tests/data`.
pub const VECTORS: &str = "snacl_vectors.txt";
/// The upstream-fixture vector file.
pub const FIXTURE_VECTORS: &str = "fixture_vectors.txt";
/// The generated database and its expectations.
pub const GENERATED_DB: &str = "generated-simnet.db.gz";
pub const GENERATED_EXPECT: &str = "generated_db.txt";

/// The PRNG seed the frozen corpus is generated from.
pub const FROZEN_RNG_SEED: u64 = 0x736e_6163_6c00_0001;

/// The entries the frozen files hold (checked against their headers too).
pub const VECTORS_ENTRIES: usize = 1178;
pub const FIXTURE_VECTORS_ENTRIES: usize = 49;

/// This crate's `tests/data`.
pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// A fresh, empty scratch directory under cargo's target tmpdir.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-snacl")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// Outcomes

/// One request and Go's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cmd: String,
    pub args: Value,
    pub outcome: Value,
    /// Set on entries pinning a deliberate divergence from Go.
    pub divergence: Option<String>,
    /// The fixture database an entry was built from.
    pub fixture: Option<String>,
}

impl Entry {
    pub fn to_line(&self) -> String {
        let mut v = json!({"cmd": self.cmd, "args": self.args, "outcome": self.outcome});
        if let Some(d) = &self.divergence {
            v["divergence"] = json!(d);
        }
        if let Some(f) = &self.fixture {
            v["fixture"] = json!(f);
        }
        serde_json::to_string(&v).expect("serialize entry")
    }

    pub fn from_line(line: &str) -> Entry {
        let v: Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("bad line {line}: {e}"));
        Entry {
            cmd: v["cmd"].as_str().expect("cmd").to_owned(),
            args: v["args"].clone(),
            outcome: v["outcome"].clone(),
            divergence: v
                .get("divergence")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fixture: v.get("fixture").and_then(Value::as_str).map(str::to_owned),
        }
    }
}

/// The `err` object of an outcome for `e`.
pub fn err_object(e: &Error) -> Value {
    json!({
        "text": e.to_string(),
        "text_lib": e.render(MULTILINE_SEPARATOR),
        "kind": e.kind().code(),
        "op": e.op().map_or("", |op| op.as_str()),
    })
}

fn err_outcome(e: &Error, extra: &[(&str, Value)]) -> Value {
    let mut o = err_object(e);
    for (k, v) in extra {
        o[*k] = v.clone();
    }
    json!({ "err": o })
}

fn ok(v: Value) -> Value {
    json!({ "ok": v })
}

/// A random source serving fixed bytes, then failing as the oracle's
/// injected reader does; it counts what it served.
pub struct Replay {
    data: Vec<u8>,
    pub served: usize,
}

impl Replay {
    pub fn new(data: Vec<u8>) -> Self {
        Replay { data, served: 0 }
    }
}

impl RandomSource for Replay {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        let rest = self.data.get(self.served..).unwrap_or_default();
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.served += n;
        if n < buf.len() {
            return Err(RandomSourceError::new("injected reader: exhausted"));
        }
        Ok(())
    }
}

fn b(args: &Value, key: &str) -> Vec<u8> {
    unhex(
        args[key]
            .as_str()
            .unwrap_or_else(|| panic!("no {key} in {args}")),
    )
}

fn arr32(v: &[u8]) -> [u8; 32] {
    v.try_into().expect("32 bytes")
}

fn i(args: &Value, key: &str) -> i64 {
    args[key]
        .as_i64()
        .unwrap_or_else(|| panic!("no integer {key} in {args}"))
}

fn u(args: &Value, key: &str) -> u64 {
    args[key]
        .as_u64()
        .unwrap_or_else(|| panic!("no unsigned {key} in {args}"))
}

/// The bytes Go drew, read back from its output: the first `n` bytes of the
/// hex member `key` of the `ok` object, or zeros when Go failed.
fn drawn(go: &Value, key: &str, n: usize) -> Vec<u8> {
    go["ok"][key]
        .as_str()
        .map_or_else(|| vec![0; n], |s| unhex(s)[..n].to_vec())
}

/// The Rust side's outcome for one oracle request. `go` is Go's outcome, read
/// only for the random bytes Go drew (they are part of its output, see the
/// oracle's header) and replayed through a [`Replay`] source, which must be
/// consumed exactly. With `check_key_after`, a failed derive-key reports the
/// key Go leaves behind (scrypt of the wrong passphrase), recomputed here, so
/// the comparison pins Go's behaviour; Rust's own key must then be zero.
pub fn rust_answer(cmd: &str, args: &Value, go: &Value, check_key_after: bool) -> Value {
    let limits = Limits::DEFAULT;
    match cmd {
        "new-secret-key" => {
            let mut rng = Replay::new(drawn(go, "params", 32));
            let r = SecretKey::new_with(
                &b(args, "pass"),
                i(args, "n"),
                i(args, "r"),
                i(args, "p"),
                &mut rng,
                &limits,
            );
            assert_eq!(rng.served, 32, "new-secret-key draws one 32-byte salt");
            match r {
                Ok(sk) => ok(json!({
                    "params": hex(&sk.marshal()),
                    "key": hex(sk.key().as_bytes()),
                })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "marshal" => {
            let p = Parameters {
                salt: arr32(&b(args, "salt")),
                digest: arr32(&b(args, "digest")),
                n: i(args, "n"),
                r: i(args, "r"),
                p: i(args, "p"),
            };
            ok(json!({ "params": hex(&p.marshal()) }))
        }
        "unmarshal" => match Parameters::unmarshal(&b(args, "data")) {
            Ok(p) => ok(json!({
                "salt": hex(&p.salt), "digest": hex(&p.digest), "n": p.n, "r": p.r, "p": p.p,
            })),
            Err(e) => err_outcome(&e, &[]),
        },
        "derive-key" => {
            let mut sk = match SecretKey::unmarshal(&b(args, "params")) {
                Ok(sk) => sk,
                Err(e) => return err_outcome(&e, &[]),
            };
            let pass = b(args, "pass");
            match sk.derive_key(&pass) {
                Ok(()) => {
                    let k = hex(sk.key().as_bytes());
                    ok(json!({ "key": k, "key_after": k }))
                }
                Err(e) => {
                    assert_eq!(sk.key().as_bytes(), &[0; 32], "a failed derivation zeroes");
                    if !check_key_after {
                        return err_outcome(&e, &[("key_after", go["err"]["key_after"].clone())]);
                    }
                    let after = if e.kind() == Kind::Passphrase {
                        let p = sk.parameters();
                        let mut salt = Replay::new(p.salt.to_vec());
                        let wrong = SecretKey::new_with(&pass, p.n, p.r, p.p, &mut salt, &limits)
                            .expect("parameters derived once already");
                        hex(wrong.key().as_bytes())
                    } else {
                        hex(&[0; 32])
                    };
                    err_outcome(&e, &[("key_after", json!(after))])
                }
            }
        }
        "generate-crypto-key" => {
            let mut rng = Replay::new(drawn(go, "key", 32));
            let r = CryptoKey::generate_with(&mut rng);
            assert_eq!(rng.served, 32);
            match r {
                Ok(k) => ok(json!({ "key": hex(k.as_bytes()) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "crypto-encrypt" | "secret-encrypt" => {
            let key = CryptoKey::from_bytes(&arr32(&b(args, "key")));
            let mut rng = Replay::new(drawn(go, "sealed", 24));
            let pt = b(args, "plaintext");
            let r = if cmd == "crypto-encrypt" {
                key.encrypt_with(&pt, &mut rng)
            } else {
                SecretKey::from_parts(key, zero_parameters()).encrypt_with(&pt, &mut rng)
            };
            assert_eq!(rng.served, 24, "encryption draws one 24-byte nonce");
            match r {
                Ok(sealed) => ok(json!({ "sealed": hex(&sealed) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "crypto-decrypt" | "secret-decrypt" => {
            let key = CryptoKey::from_bytes(&arr32(&b(args, "key")));
            let sealed = b(args, "sealed");
            let r = if cmd == "crypto-decrypt" {
                key.decrypt(&sealed)
            } else {
                SecretKey::from_parts(key, zero_parameters()).decrypt(&sealed)
            };
            match r {
                Ok(pt) => ok(json!({ "plaintext": hex(pt.as_bytes()) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "argon2-new-params" => {
            let num_cpu = go
                .get("ok")
                .or_else(|| go.get("err"))
                .and_then(|o| o["num_cpu"].as_u64())
                .expect("num_cpu");
            let num_cpu = usize::try_from(num_cpu).expect("fits");
            let mut rng = Replay::new(b(args, "stream"));
            match Argon2idParams::new_with(&mut rng, num_cpu) {
                Ok(p) => ok(json!({
                    "params": hex(&p.marshal_binary()),
                    "served": rng.served,
                    "num_cpu": num_cpu,
                })),
                Err(e) => err_outcome(
                    &e,
                    &[("served", json!(rng.served)), ("num_cpu", json!(num_cpu))],
                ),
            }
        }
        "argon2-marshal" => {
            let p = Argon2idParams {
                salt: b(args, "salt").try_into().expect("16 bytes"),
                time: u32::try_from(u(args, "time")).expect("u32"),
                memory: u32::try_from(u(args, "memory")).expect("u32"),
                threads: u8::try_from(u(args, "threads")).expect("u8"),
            };
            ok(json!({ "data": hex(&p.marshal_binary()) }))
        }
        "argon2-unmarshal" => match Argon2idParams::unmarshal_binary(&b(args, "data")) {
            Ok(p) => ok(json!({
                "salt": hex(&p.salt), "time": p.time, "memory": p.memory, "threads": p.threads,
            })),
            Err(e) => err_outcome(&e, &[]),
        },
        "argon2-derive" => {
            let p = match Argon2idParams::unmarshal_binary(&b(args, "params")) {
                Ok(p) => p,
                Err(e) => return err_outcome(&e, &[]),
            };
            match p.derive_key(&b(args, "pass")) {
                Ok(k) => ok(json!({ "key": hex(k.as_bytes()) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "seal" => {
            let key = AccountKey::from_bytes(&arr32(&b(args, "key")));
            let mut rng = Replay::new(drawn(go, "sealed", 24));
            let r = key.seal_with(&b(args, "plaintext"), &mut rng);
            assert_eq!(rng.served, 24, "seal draws one 24-byte nonce");
            match r {
                Ok(s) => ok(json!({ "sealed": hex(&s) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "unseal" => {
            let key = AccountKey::from_bytes(&arr32(&b(args, "key")));
            match key.unseal(&b(args, "sealed")) {
                Ok(pt) => ok(json!({ "plaintext": hex(pt.as_bytes()) })),
                Err(e) => err_outcome(&e, &[]),
            }
        }
        "account-seal" | "account-unseal" => {
            let p = match Argon2idParams::unmarshal_binary(&b(args, "params")) {
                Ok(p) => p,
                Err(e) => return err_outcome(&e, &[]),
            };
            let key = match p.derive_key(&b(args, "pass")) {
                Ok(k) => k,
                Err(e) => return err_outcome(&e, &[]),
            };
            let k = json!(hex(key.as_bytes()));
            if cmd == "account-seal" {
                let mut rng = Replay::new(drawn(go, "sealed", 24));
                let r = key.seal_with(&b(args, "plaintext"), &mut rng);
                assert_eq!(rng.served, 24);
                match r {
                    Ok(s) => ok(json!({ "key": k, "sealed": hex(&s) })),
                    Err(e) => err_outcome(&e, &[("key", k)]),
                }
            } else {
                match key.unseal(&b(args, "sealed")) {
                    Ok(pt) => ok(json!({ "key": k, "plaintext": hex(pt.as_bytes()) })),
                    Err(e) => err_outcome(&e, &[("key", k)]),
                }
            }
        }
        _ => panic!("no Rust answer for {cmd}"),
    }
}

fn zero_parameters() -> Parameters {
    Parameters {
        salt: [0; 32],
        digest: [0; 32],
        n: 0,
        r: 0,
        p: 0,
    }
}

/// Checks the Rust outcome against an entry's Go outcome. Where Go panics,
/// Rust must return an error without op or kind whose text is Go's panic
/// message. An entry marked as a divergence must show exactly the recorded
/// one.
pub fn check(entry: &Entry, rust: &Value) {
    let ctx = || format!("{} {}", entry.cmd, entry.args);
    match entry.divergence.as_deref() {
        None => {}
        Some("argon2-memory-below-minimum") => {
            assert!(
                entry.outcome.get("ok").is_some(),
                "Go derives a key: {}",
                ctx()
            );
            let text = rust["err"]["text"].as_str().unwrap_or_default();
            assert!(
                text.starts_with("argon2: a memory cost of ") && rust["err"]["kind"] == json!(0),
                "Rust refuses: {} -> {rust}",
                ctx()
            );
            return;
        }
        Some(other) => panic!("unknown divergence {other}"),
    }
    if let Some(msg) = entry.outcome.get("panic") {
        let want = json!({"err": {"text": msg, "text_lib": msg, "kind": 0, "op": ""}});
        assert_eq!(
            rust,
            &want,
            "Go panics here, Rust must return its message: {}",
            ctx()
        );
        return;
    }
    assert_eq!(rust, &entry.outcome, "{}", ctx());
}

/// Runs an entry through the Rust side and checks it.
pub fn check_entry(entry: &Entry) {
    let rust = rust_answer(&entry.cmd, &entry.args, &entry.outcome, true);
    check(entry, &rust);
}

// ---------------------------------------------------------------------------
// Corpus

/// Builds the differential corpus, asking Go (`ask`) for each outcome; later
/// requests use earlier answers (derive with Go's parameters, decrypt Go's
/// boxes).
pub fn build_corpus(
    rng: &mut SplitMix64,
    ask: &mut dyn FnMut(&str, &Value) -> Value,
) -> Vec<Entry> {
    let mut c = Corpus {
        entries: Vec::new(),
        ask,
    };
    let pass = |rng: &mut SplitMix64| {
        let len = rng.below(41) as usize;
        rng.bytes(len)
    };

    // new-secret-key: small valid costs, then every refusal branch.
    let mut keys: Vec<(Vec<u8>, Vec<u8>)> = Vec::new(); // (params, pass)
    for _ in 0..40 {
        let p = pass(rng);
        let args = json!({
            "pass": hex(&p),
            "n": 1i64 << (1 + rng.below(8)),
            "r": 1 + rng.below(4) as i64,
            "p": 1 + rng.below(3) as i64,
        });
        let o = c.q("new-secret-key", args);
        keys.push((unhex(o["ok"]["params"].as_str().expect("params")), p));
    }
    // The last four fail both of Go's first two checks, so they pin the
    // order (N first, then r and p).
    let refused: [(i64, i64, i64); 26] = [
        (0, 1, 1),
        (1, 1, 1),
        (3, 1, 1),
        (6, 1, 1),
        (-2, 1, 1),
        (-1, 1, 1),
        (i64::MIN, 1, 1),
        (i64::MAX, 1, 1),
        (1 << 62 | 1, 1, 1),
        (2, 0, 1),
        (2, 1, 0),
        (2, -1, 1),
        (2, 1, -1),
        (2, i64::MIN, i64::MIN),
        (2, 1 << 15, 1 << 15),
        (2, 1 << 30, 1),
        (2, 1, 1 << 30),
        (2, 1 << 32, 1 << 32),
        (2, 1 << 55, 1),
        (2, i64::MAX / 256 + 1, 1),
        (1 << 60, 1, 1),
        (1 << 50, 128, 1),
        (0, 0, 1),
        (3, -1, 1),
        (-2, 1, 0),
        (1, 0, 0),
    ];
    for (n, r, p) in refused {
        c.q(
            "new-secret-key",
            json!({"pass": hex(&pass(rng)), "n": n, "r": r, "p": p}),
        );
    }
    // The cheapest cost (N = 16, r = 1, p = 1): every byte of its digest is
    // flipped below.
    let p = pass(rng);
    let o = c.q(
        "new-secret-key",
        json!({"pass": hex(&p), "n": 16, "r": 1, "p": 1}),
    );
    let cheap = (unhex(o["ok"]["params"].as_str().expect("params")), p);
    // snacl's DefaultN/R/P (16 MiB), once.
    let p = pass(rng);
    let o = c.q(
        "new-secret-key",
        json!({"pass": hex(&p), "n": 16384, "r": 8, "p": 1}),
    );
    keys.push((unhex(o["ok"]["params"].as_str().expect("params")), p));

    // marshal and unmarshal.
    for k in 0..24 {
        let field = |rng: &mut SplitMix64| match k % 3 {
            0 => rng.next_u64().cast_signed(),
            1 => rng.below(1 << 20) as i64,
            _ => -(rng.below(1 << 20) as i64),
        };
        let args = json!({
            "salt": hex(&rng.bytes(32)), "digest": hex(&rng.bytes(32)),
            "n": field(rng), "r": field(rng), "p": field(rng),
        });
        c.q("marshal", args);
    }
    for _ in 0..24 {
        c.q("unmarshal", json!({ "data": hex(&rng.bytes(88)) }));
    }
    for len in [0usize, 1, 32, 64, 87, 89, 176] {
        c.q("unmarshal", json!({ "data": hex(&rng.bytes(len)) }));
    }

    // derive-key: Go's parameters with the right and a wrong passphrase.
    for (params, p) in &keys {
        c.q("derive-key", json!({"params": hex(params), "pass": hex(p)}));
        let mut wrong = p.clone();
        if wrong.is_empty() {
            wrong.push(0);
        } else {
            let at = rng.below(wrong.len() as u64) as usize;
            wrong[at] ^= 1 << rng.below(8);
        }
        c.q(
            "derive-key",
            json!({"params": hex(params), "pass": hex(&wrong)}),
        );
        // The right passphrase against a stored digest (bytes 32..64 of the
        // parameters) with one bit flipped: Go compares all 32 bytes.
        for at in [32usize, 47, 48, 63] {
            let mut tampered = params.clone();
            tampered[at] ^= 1 << rng.below(8);
            c.q(
                "derive-key",
                json!({"params": hex(&tampered), "pass": hex(p)}),
            );
        }
    }
    // Every digest byte of the cheapest key flipped, so no comparison of
    // fewer than all 32 bytes passes.
    let (params, p) = &cheap;
    c.q("derive-key", json!({"params": hex(params), "pass": hex(p)}));
    for at in 32..64usize {
        let mut tampered = params.clone();
        tampered[at] ^= 1 << rng.below(8);
        c.q(
            "derive-key",
            json!({"params": hex(&tampered), "pass": hex(p)}),
        );
    }
    // Crafted parameters: stored costs Go refuses (negative after the
    // uint64-to-int conversion included), a digest that never matches, and
    // a bad length.
    for (n, r, p) in [
        (u64::MAX, 1u64, 1u64),
        (1 << 63, 1, 1),
        (4, 1 << 63, 1),
        (4, 1, u64::MAX),
        (3, 1, 1),
        (1 << 40, 1 << 20, 1),
        (4, 1, 1),
        (8, 2, 2),
        // Both of Go's first two checks fail: N is reported.
        (0, 0, 1),
        (u64::MAX, 0, 1),
    ] {
        let mut data = rng.bytes(64);
        for v in [n, r, p] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        c.q(
            "derive-key",
            json!({"params": hex(&data), "pass": hex(&pass(rng))}),
        );
    }
    c.q(
        "derive-key",
        json!({"params": hex(&rng.bytes(87)), "pass": ""}),
    );

    // Crypto keys.
    for _ in 0..6 {
        c.q("generate-crypto-key", json!({}));
    }
    let mut boxes: Vec<(Vec<u8>, Vec<u8>)> = Vec::new(); // (key, sealed)
    for k in 0..40 {
        let key = rng.bytes(32);
        let len = match k % 4 {
            0 => 32,
            1 => rng.below(8) as usize,
            _ => rng.below(200) as usize,
        };
        let cmd = if k % 2 == 0 {
            "crypto-encrypt"
        } else {
            "secret-encrypt"
        };
        let o = c.q(
            cmd,
            json!({"key": hex(&key), "plaintext": hex(&rng.bytes(len))}),
        );
        boxes.push((key, unhex(o["ok"]["sealed"].as_str().expect("sealed"))));
    }
    for (k, (key, sealed)) in boxes.iter().enumerate() {
        let cmd = if k % 2 == 0 {
            "crypto-decrypt"
        } else {
            "secret-decrypt"
        };
        c.q(cmd, json!({"key": hex(key), "sealed": hex(sealed)}));
        c.q(
            cmd,
            json!({"key": hex(&rng.bytes(32)), "sealed": hex(sealed)}),
        );
        let mut bad = sealed.clone();
        let at = rng.below(bad.len() as u64) as usize;
        bad[at] ^= 1 << rng.below(8);
        c.q(cmd, json!({"key": hex(key), "sealed": hex(&bad)}));
        let cut = rng.below(sealed.len() as u64 + 1) as usize;
        c.q(cmd, json!({"key": hex(key), "sealed": hex(&sealed[..cut])}));
    }
    let (key, sealed) = &boxes[0];
    for len in 0..=48usize.min(sealed.len()) {
        for cmd in ["crypto-decrypt", "secret-decrypt"] {
            c.q(cmd, json!({"key": hex(key), "sealed": hex(&sealed[..len])}));
        }
    }
    for len in [0usize, 23, 24, 39, 40, 41, 100] {
        c.q(
            "crypto-decrypt",
            json!({"key": hex(&rng.bytes(32)), "sealed": hex(&rng.bytes(len))}),
        );
    }

    // Argon2id parameters.
    for len in [0usize, 1, 8, 15, 16, 17, 32] {
        c.q(
            "argon2-new-params",
            json!({ "stream": hex(&rng.bytes(len)) }),
        );
    }
    for _ in 0..16 {
        let args = json!({
            "salt": hex(&rng.bytes(16)),
            "time": rng.next_u64() as u32,
            "memory": rng.next_u64() as u32,
            "threads": rng.next_u64() as u8,
        });
        c.q("argon2-marshal", args);
    }
    for _ in 0..16 {
        c.q("argon2-unmarshal", json!({ "data": hex(&rng.bytes(25)) }));
    }
    for len in [0usize, 1, 16, 24, 26, 50] {
        c.q("argon2-unmarshal", json!({ "data": hex(&rng.bytes(len)) }));
    }
    let small_params = |rng: &mut SplitMix64| {
        let threads = 1 + rng.below(4) as u8;
        let min = 8 * u32::from(threads);
        Argon2idParams {
            salt: rng.bytes(16).try_into().expect("16"),
            time: 1 + rng.below(3) as u32,
            memory: min + rng.below(u64::from(min) * 3) as u32,
            threads,
        }
    };
    // Every params blob a derivation or seal request carries is Go's own
    // encoding (argon2-marshal), not this crate's: a broken marshal_binary
    // then fails the argon2-marshal comparison instead of sending Go a cost
    // it was never meant to run.
    for _ in 0..40 {
        let p = c.argon2_params(&small_params(rng));
        c.q(
            "argon2-derive",
            json!({"params": hex(&p), "pass": hex(&pass(rng))}),
        );
    }
    // The least memory Go and the argon2 crate both accept, 8 KiB per
    // thread, and one KiB more.
    for threads in 1..=4u8 {
        for extra in [0u32, 1] {
            let p = c.argon2_params(&Argon2idParams {
                salt: rng.bytes(16).try_into().expect("16"),
                time: 1,
                memory: 8 * u32::from(threads) + extra,
                threads,
            });
            c.q(
                "argon2-derive",
                json!({"params": hex(&p), "pass": hex(&pass(rng))}),
            );
        }
    }
    // Where Go panics: time 0, threads 0, both (time is checked first).
    for (time, memory, threads) in [(0u32, 64u32, 1u8), (1, 64, 0), (0, 64, 0), (0, 0, 0)] {
        let p = c.argon2_params(&Argon2idParams {
            salt: [7; 16],
            time,
            memory,
            threads,
        });
        c.q("argon2-derive", json!({"params": hex(&p), "pass": "00"}));
    }
    c.q(
        "argon2-derive",
        json!({"params": hex(&rng.bytes(24)), "pass": ""}),
    );
    // Where Go derives a key from a memory cost below 8 KiB per thread and
    // this crate cannot.
    for (memory, threads) in [(0u32, 1u8), (1, 1), (7, 1), (8, 2), (15, 2), (31, 4)] {
        let p = c.argon2_params(&Argon2idParams {
            salt: [9; 16],
            time: 1,
            memory,
            threads,
        });
        let args = json!({"params": hex(&p), "pass": "70617373"});
        c.q_divergent("argon2-derive", args, "argon2-memory-below-minimum");
    }

    // The per-account XChaCha20-Poly1305 blobs.
    let mut sealed_blobs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for k in 0..30 {
        let key = rng.bytes(32);
        let len = if k % 3 == 0 {
            111
        } else {
            rng.below(200) as usize
        };
        let o = c.q(
            "seal",
            json!({"key": hex(&key), "plaintext": hex(&rng.bytes(len))}),
        );
        sealed_blobs.push((key, unhex(o["ok"]["sealed"].as_str().expect("sealed"))));
    }
    for (key, sealed) in &sealed_blobs {
        c.q("unseal", json!({"key": hex(key), "sealed": hex(sealed)}));
        c.q(
            "unseal",
            json!({"key": hex(&rng.bytes(32)), "sealed": hex(sealed)}),
        );
        let mut bad = sealed.clone();
        let at = rng.below(bad.len() as u64) as usize;
        bad[at] ^= 1 << rng.below(8);
        c.q("unseal", json!({"key": hex(key), "sealed": hex(&bad)}));
    }
    let (key, sealed) = &sealed_blobs[0];
    for len in 0..=42usize {
        c.q(
            "unseal",
            json!({"key": hex(key), "sealed": hex(&sealed[..len])}),
        );
    }

    // Argon2id + seal as udb composes them, with small costs...
    let mut accounts: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = Vec::new(); // (params, pass, sealed)
    let mut seal_params: Vec<Argon2idParams> = (0..16).map(|_| small_params(rng)).collect();
    // ... at the least memory accepted, and one KiB more ...
    for threads in 1..=4u8 {
        for extra in [0u32, 1] {
            seal_params.push(Argon2idParams {
                salt: rng.bytes(16).try_into().expect("16"),
                time: 1,
                memory: 8 * u32::from(threads) + extra,
                threads,
            });
        }
    }
    for sp in &seal_params {
        let params = c.argon2_params(sp);
        let p = pass(rng);
        let pt = rng.bytes(111);
        let o = c.q(
            "account-seal",
            json!({"params": hex(&params), "pass": hex(&p), "plaintext": hex(&pt)}),
        );
        accounts.push((
            params,
            p,
            unhex(o["ok"]["sealed"].as_str().expect("sealed")),
        ));
    }
    // ... and dcrwallet's own: time 1, 256 MiB, one and eight threads.
    for threads in [1u8, 8] {
        let params = c.argon2_params(&Argon2idParams {
            salt: rng.bytes(16).try_into().expect("16"),
            time: Argon2idParams::DEFAULT_TIME,
            memory: Argon2idParams::DEFAULT_MEMORY,
            threads,
        });
        let p = pass(rng);
        let o = c.q(
            "account-seal",
            json!({"params": hex(&params), "pass": hex(&p), "plaintext": hex(&rng.bytes(111))}),
        );
        accounts.push((
            params,
            p,
            unhex(o["ok"]["sealed"].as_str().expect("sealed")),
        ));
    }
    let n_accounts = accounts.len();
    for (k, (params, p, sealed)) in accounts.iter().enumerate() {
        c.q(
            "account-unseal",
            json!({"params": hex(params), "pass": hex(p), "sealed": hex(sealed)}),
        );
        if k >= n_accounts - 2 {
            continue; // one 256 MiB derivation each is enough
        }
        let mut wrong = p.clone();
        wrong.push(b'!');
        c.q(
            "account-unseal",
            json!({"params": hex(params), "pass": hex(&wrong), "sealed": hex(sealed)}),
        );
        let cut = rng.below(40) as usize;
        c.q(
            "account-unseal",
            json!({"params": hex(params), "pass": hex(p), "sealed": hex(&sealed[..cut])}),
        );
    }
    let p = c.argon2_params(&Argon2idParams {
        salt: [1; 16],
        time: 0,
        memory: 64,
        threads: 1,
    });
    c.q(
        "account-seal",
        json!({"params": hex(&p), "pass": "", "plaintext": "00"}),
    );
    c.q(
        "account-unseal",
        json!({"params": hex(&rng.bytes(3)), "pass": "", "sealed": ""}),
    );

    c.entries
}

struct Corpus<'a> {
    entries: Vec<Entry>,
    ask: &'a mut dyn FnMut(&str, &Value) -> Value,
}

impl Corpus<'_> {
    fn q(&mut self, cmd: &str, args: Value) -> Value {
        let outcome = (self.ask)(cmd, &args);
        self.entries.push(Entry {
            cmd: cmd.to_owned(),
            args,
            outcome: outcome.clone(),
            divergence: None,
            fixture: None,
        });
        outcome
    }

    /// Go's encoding of `p` (an argon2-marshal request, recorded like any
    /// other).
    fn argon2_params(&mut self, p: &Argon2idParams) -> Vec<u8> {
        let o = self.q(
            "argon2-marshal",
            json!({
                "salt": hex(&p.salt),
                "time": p.time,
                "memory": p.memory,
                "threads": p.threads,
            }),
        );
        unhex(o["ok"]["data"].as_str().expect("argon2-marshal data"))
    }

    fn q_divergent(&mut self, cmd: &str, args: Value, divergence: &str) {
        self.q(cmd, args);
        if let Some(e) = self.entries.last_mut() {
            e.divergence = Some(divergence.to_owned());
        }
    }
}

// ---------------------------------------------------------------------------
// Vector files

/// The SHA-256 of a vector file's body: every line that is not a comment
/// (nor empty), each followed by a newline.
pub fn body_sha256(text: &str) -> String {
    let mut h = Sha256::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        h.update(line.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// Reads a vector file, checking its `# entries:`, `# count <cmd>:` and
/// `# body-sha256:` headers against its body and `entries` against the
/// pinned count.
pub fn read_vectors(name: &str, entries: usize) -> Vec<Entry> {
    let text = std::fs::read_to_string(data_dir().join(name))
        .unwrap_or_else(|e| panic!("read tests/data/{name}: {e}"));
    let parsed = verify_vectors(name, &text);
    assert_eq!(parsed.len(), entries, "{name}: pinned entry count");
    parsed
}

/// Checks a vector file's integrity headers and returns its entries.
pub fn verify_vectors(name: &str, text: &str) -> Vec<Entry> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{name}: no {key:?} header"))
            .to_owned()
    };
    let entries: Vec<Entry> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(Entry::from_line)
        .collect();
    let recorded: usize = field("# entries:").parse().expect("entry count");
    assert_eq!(entries.len(), recorded, "{name}: entry count");
    assert_eq!(
        body_sha256(text),
        field("# body-sha256:"),
        "{name}: body hash"
    );
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &entries {
        *counts.entry(e.cmd.clone()).or_default() += 1;
    }
    let mut recorded_counts: BTreeMap<String, usize> = BTreeMap::new();
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("# count ") {
            let (cmd, n) = rest.split_once(':').expect("count line");
            recorded_counts.insert(cmd.trim().to_owned(), n.trim().parse().expect("count"));
        }
    }
    assert_eq!(counts, recorded_counts, "{name}: per-command counts");
    entries
}

/// Writes a vector file: `header` lines (each prefixed with `# `), the
/// counts, the body hash, then one entry per line.
pub fn write_vectors(name: &str, header: &[String], entries: &[Entry]) {
    let body: String = entries.iter().map(|e| e.to_line() + "\n").collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for e in entries {
        *counts.entry(&e.cmd).or_default() += 1;
    }
    let mut text = String::new();
    for h in header {
        text.push_str(&format!("# {h}\n").replace("# \n", "#\n"));
    }
    text.push_str(&format!("# entries: {}\n", entries.len()));
    for (cmd, n) in counts {
        text.push_str(&format!("# count {cmd}: {n}\n"));
    }
    text.push_str(&format!("# body-sha256: {}\n", body_sha256(&body)));
    text.push_str(&body);
    let path = data_dir().join(name);
    std::fs::write(&path, &text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!(
        "wrote tests/data/{name} ({} entries, {} bytes)",
        entries.len(),
        text.len()
    );
}

/// Whether regeneration was asked for (`PURSE_SNACL_REGEN=1`).
pub fn regen_requested() -> bool {
    std::env::var_os("PURSE_SNACL_REGEN").is_some()
}

// ---------------------------------------------------------------------------
// Wallet databases

/// Decompresses a gzip file.
pub fn gunzip(path: &Path) -> Vec<u8> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(f)
        .read_to_end(&mut out)
        .unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
    out
}

/// An upstream fixture database (purse-boltread's copy), decompressed.
pub fn upstream_fixture(name: &str) -> Vec<u8> {
    gunzip(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../purse-boltread/tests/data/upstream")
            .join(name),
    )
}

/// Opens database bytes read-only.
pub fn open_db(bytes: Vec<u8>) -> Db {
    Db::from_source(bytes, &OpenOptions::default()).expect("open database")
}

/// The value at `key` in the bucket at `path`.
pub fn get(db: &Db, path: &[&[u8]], key: &[u8]) -> Option<Vec<u8>> {
    let bucket = db.bucket_path(path).expect("bucket lookup")?;
    bucket.get(key).expect("get").map(|v| v.to_vec())
}

/// Every key/value pair (not nested buckets) of the bucket at `path`.
pub fn pairs(db: &Db, path: &[&[u8]]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let Some(bucket) = db.bucket_path(path).expect("bucket lookup") else {
        return Vec::new();
    };
    bucket
        .iter()
        .map(|item| item.expect("item"))
        .filter_map(|item| item.value().map(|v| (item.key().to_vec(), v.to_vec())))
        .collect()
}

/// The names of the nested buckets of the bucket at `path`.
pub fn sub_buckets(db: &Db, path: &[&[u8]]) -> Vec<Vec<u8>> {
    let Some(bucket) = db.bucket_path(path).expect("bucket lookup") else {
        return Vec::new();
    };
    bucket
        .iter()
        .map(|item| item.expect("item"))
        .filter(|item| item.is_bucket())
        .map(|item| item.key().to_vec())
        .collect()
}

pub const WADDRMGR: &[u8] = b"waddrmgr";

/// Two length-prefixed fields (`u32 LE len, bytes`, twice), as account rows
/// and imported address rows begin.
pub fn two_fields(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let len = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().expect("4")) as usize;
    let a = len(0);
    let first = raw[4..4 + a].to_vec();
    let bl = len(4 + a);
    let second = raw[8 + a..8 + a + bl].to_vec();
    (first, second)
}

/// An account row's encrypted xpub and xpriv (`type, u32 rdlen, raw`; the
/// raw data of every row type starts with the two fields).
pub fn account_row(row: &[u8]) -> (u8, Vec<u8>, Vec<u8>) {
    let (pubk, privk) = two_fields(&row[5..]);
    (row[0], pubk, privk)
}

/// An address row: (type, raw data).
pub fn address_row(row: &[u8]) -> (u8, Vec<u8>) {
    let rdlen = u32::from_le_bytes(row[14..18].try_into().expect("4")) as usize;
    (row[0], row[18..18 + rdlen].to_vec())
}

/// Runs `f` over `items` on all cores, keeping the order.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker panicked"))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Upstream fixtures

/// An upstream fixture database the fixture tests open, with whether they
/// also try the wrong passphrases (each try is a full scrypt at the
/// database's cost, N = 2^18 for these).
pub const FIXTURES: [(&str, bool); 2] = [("v24.db.gz", true), ("v1.db.gz", false)];

/// The passphrases every upstream fixture generator names
/// (`wallet/udb/testdata/v*.go`: `pubPass = []byte("public")`, `privPass =
/// []byte("private")`).
pub const FIXTURE_PUB_PASS: &[u8] = b"public";
pub const FIXTURE_PRIV_PASS: &[u8] = b"private";

/// The sealed values of a fixture's address manager, by what seals them.
pub struct FixtureBlobs {
    pub mpub: Vec<u8>,
    pub mpriv: Vec<u8>,
    pub cpub: Vec<u8>,
    pub cpriv: Vec<u8>,
    /// (where, blob) sealed with the public crypto key.
    pub public: Vec<(String, Vec<u8>)>,
    /// (where, blob) sealed with the private crypto key.
    pub private: Vec<(String, Vec<u8>)>,
}

/// Reads `waddrmgr/main`'s master key parameters, crypto keys and coin
/// type keys, the `slip0044acct0` row and every `acct` row's xpub and xpriv.
pub fn fixture_blobs(db: &Db) -> FixtureBlobs {
    let main: &[&[u8]] = &[WADDRMGR, b"main"];
    let req = |k: &[u8]| {
        get(db, main, k).unwrap_or_else(|| panic!("no main/{}", String::from_utf8_lossy(k)))
    };
    let mut public = Vec::new();
    let mut private = Vec::new();
    for k in [
        &b"ctpub"[..],
        b"ctpub-slip0044",
        b"ctpriv",
        b"ctpriv-slip0044",
    ] {
        if let Some(v) = get(db, main, k) {
            let list = if k.starts_with(b"ctpub") {
                &mut public
            } else {
                &mut private
            };
            list.push((format!("main/{}", String::from_utf8_lossy(k)), v));
        }
    }
    let mut rows: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(v) = get(db, main, b"slip0044acct0") {
        rows.push(("main/slip0044acct0".to_owned(), v));
    }
    for (k, v) in pairs(db, &[WADDRMGR, b"acct"]) {
        rows.push((format!("acct/{}", hex(&k)), v));
    }
    for (place, row) in rows {
        let (_, pubk, privk) = account_row(&row);
        if !pubk.is_empty() {
            public.push((format!("{place}/pub"), pubk));
        }
        if !privk.is_empty() {
            private.push((format!("{place}/priv"), privk));
        }
    }
    FixtureBlobs {
        mpub: req(b"mpub"),
        mpriv: req(b"mpriv"),
        cpub: req(b"cpub"),
        cpriv: req(b"cpriv"),
        public,
        private,
    }
}

/// Removes `key_after` from an error outcome (the fixture flow does not
/// recompute Go's leftover key: that would be another full scrypt).
pub fn strip_key_after(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(o) = v.get_mut("err").and_then(Value::as_object_mut) {
        o.remove("key_after");
    }
    v
}

/// The fixture flow: the requests the fixture tests make of a fixture,
/// each with the Rust side's outcome (`outcome`; `key_after` stripped). Later
/// requests use earlier Rust answers (the derived master keys, the decrypted
/// crypto keys), so a wrong Rust answer also changes the requests.
pub fn fixture_flow(file: &str, wrong: bool) -> Vec<Entry> {
    let db = open_db(upstream_fixture(file));
    let blobs = fixture_blobs(&db);
    let mut out: Vec<Entry> = Vec::new();
    let mut run = |cmd: &str, args: Value| -> Value {
        let outcome = strip_key_after(&rust_answer(cmd, &args, &Value::Null, false));
        out.push(Entry {
            cmd: cmd.to_owned(),
            args,
            outcome: outcome.clone(),
            divergence: None,
            fixture: Some(file.to_owned()),
        });
        outcome
    };
    for p in [&blobs.mpub, &blobs.mpriv] {
        run("unmarshal", json!({ "data": hex(p) }));
    }
    let key = |o: &Value, k: &str| {
        o["ok"][k]
            .as_str()
            .unwrap_or_else(|| panic!("{file}: {o}"))
            .to_owned()
    };
    let kp = key(
        &run(
            "derive-key",
            json!({"params": hex(&blobs.mpub), "pass": hex(FIXTURE_PUB_PASS)}),
        ),
        "key",
    );
    let kv = key(
        &run(
            "derive-key",
            json!({"params": hex(&blobs.mpriv), "pass": hex(FIXTURE_PRIV_PASS)}),
        ),
        "key",
    );
    if wrong {
        run(
            "derive-key",
            json!({"params": hex(&blobs.mpub), "pass": hex(FIXTURE_PRIV_PASS)}),
        );
        run(
            "derive-key",
            json!({"params": hex(&blobs.mpriv), "pass": hex(FIXTURE_PUB_PASS)}),
        );
        run(
            "derive-key",
            json!({"params": hex(&blobs.mpriv), "pass": hex(b"wrong")}),
        );
    }
    let ck_pub = key(
        &run(
            "secret-decrypt",
            json!({"key": kp, "sealed": hex(&blobs.cpub)}),
        ),
        "plaintext",
    );
    let ck_priv = key(
        &run(
            "secret-decrypt",
            json!({"key": kv, "sealed": hex(&blobs.cpriv)}),
        ),
        "plaintext",
    );
    run(
        "secret-decrypt",
        json!({"key": kp, "sealed": hex(&blobs.cpriv)}),
    );
    run(
        "secret-decrypt",
        json!({"key": kv, "sealed": hex(&blobs.cpub)}),
    );
    for (_, blob) in &blobs.public {
        run(
            "crypto-decrypt",
            json!({"key": ck_pub, "sealed": hex(blob)}),
        );
    }
    for (_, blob) in &blobs.private {
        run(
            "crypto-decrypt",
            json!({"key": ck_priv, "sealed": hex(blob)}),
        );
    }
    if let Some((_, blob)) = blobs.private.first() {
        run(
            "crypto-decrypt",
            json!({"key": ck_pub, "sealed": hex(blob)}),
        );
    }
    out
}

/// Compares a fixture flow with Go's outcomes for the same requests
/// (`key_after` stripped from both).
pub fn check_flow_against(flow: &[Entry], go: &[Entry], what: &str) {
    assert_eq!(flow.len(), go.len(), "{what}: request count");
    for (r, g) in flow.iter().zip(go) {
        assert_eq!((&r.cmd, &r.args), (&g.cmd, &g.args), "{what}: request");
        assert_eq!(
            r.outcome,
            strip_key_after(&g.outcome),
            "{what}: {} {}",
            r.cmd,
            r.args
        );
    }
}

// ---------------------------------------------------------------------------
// Generated databases

/// The make-db request the generated-database tests use (the database path
/// is added by the caller).
pub fn make_db_request(net: &str) -> Value {
    json!({
        "net": net,
        "seed": hex(&(0u8..32).collect::<Vec<u8>>()),
        "pub_pass": hex(b"public"),
        "priv_pass": hex(b"private"),
        "wrong_pass": hex(b"wrong"),
        "acct_pass": hex(b"account passphrase"),
        "voting_seed": hex(&(0x40u8..0x60).collect::<Vec<u8>>()),
        "voting_pass": hex(b"voting passphrase"),
        "import_key": hex(&[0x11; 32]),
        "script": hex(&[0x51, 0x52, 0x53, 0xae]),
    })
}

/// What a walk of a generated database found.
#[derive(Debug, Default)]
pub struct DbWalk {
    pub accounts: BTreeMap<String, String>,
    pub unique_threads: Vec<u8>,
    pub imported_keys: usize,
    pub scripts: usize,
}

/// Opens every sealed value of a database written by the oracle's make-db
/// with purse-snacl, checks each plaintext against the Go result (`expect`)
/// and each blob against a Rust reproduction with the stored nonce, and
/// checks wrong passphrases give the errors Go's API gave.
pub fn check_generated_db(bytes: Vec<u8>, request: &Value, expect: &Value) -> DbWalk {
    let db = open_db(bytes);
    let pass = |k: &str| b(request, k);
    let main: &[&[u8]] = &[WADDRMGR, b"main"];
    let get_main = |k: &[u8]| {
        get(&db, main, k).unwrap_or_else(|| panic!("no main/{}", String::from_utf8_lossy(k)))
    };

    // Master keys: parameters reproduced byte for byte from the passphrase
    // and the stored salt, then derived from the stored form.
    let mut masters = Vec::new();
    for (name, pp) in [(&b"mpub"[..], "pub_pass"), (b"mpriv", "priv_pass")] {
        let stored = get_main(name);
        let mut sk = SecretKey::unmarshal(&stored).expect("unmarshal");
        assert_eq!(sk.marshal().to_vec(), stored);
        let p = *sk.parameters();
        let again = SecretKey::new_with(
            &pass(pp),
            p.n,
            p.r,
            p.p,
            &mut Replay::new(p.salt.to_vec()),
            &Limits::DEFAULT,
        )
        .expect("new_with");
        assert_eq!(
            again.marshal().to_vec(),
            stored,
            "{} reproduced",
            String::from_utf8_lossy(name)
        );
        sk.derive_key(&pass(pp)).expect("the right passphrase");
        masters.push(sk);
    }
    // The wrong private passphrase: the error Manager.Unlock returned.
    let mut wrong = SecretKey::unmarshal(&get_main(b"mpriv")).expect("unmarshal");
    let e = wrong
        .derive_key(&pass("wrong_pass"))
        .expect_err("wrong passphrase");
    assert_eq!(err_object(&e), expect["unlock_wrong_priv"]);
    // The wrong public passphrase: udb.Open replaces the error with a bare
    // errors.E(errors.Passphrase); the kind is the one Go reports.
    let mut wrong = SecretKey::unmarshal(&get_main(b"mpub")).expect("unmarshal");
    let e = wrong
        .derive_key(&pass("wrong_pass"))
        .expect_err("wrong passphrase");
    assert_eq!(json!(e.kind().code()), expect["open_wrong_pub"]["kind"]);
    assert_eq!(e.to_string(), "secretkey.DeriveKey: invalid passphrase");
    assert_eq!(
        expect["open_wrong_pub"]["text"],
        json!("invalid passphrase")
    );

    // Crypto keys, reproduced with the stored nonce.
    let mut crypto = Vec::new();
    for (sk, name) in masters.iter().zip([&b"cpub"[..], b"cpriv"]) {
        let stored = get_main(name);
        let ck = sk.decrypt(&stored).expect("decrypt crypto key");
        let resealed = sk
            .encrypt_with(ck.as_bytes(), &mut Replay::new(stored[..24].to_vec()))
            .expect("reseal");
        assert_eq!(resealed, stored);
        crypto.push(CryptoKey::from_slice(ck.as_bytes()).expect("32-byte crypto key"));
    }
    let open = |ck: &CryptoKey, blob: &[u8]| -> Vec<u8> {
        let pt = ck.decrypt(blob).expect("decrypt");
        let resealed = ck
            .encrypt_with(pt.as_bytes(), &mut Replay::new(blob[..24].to_vec()))
            .expect("reseal");
        assert_eq!(resealed, blob);
        pt.as_bytes().to_vec()
    };
    for (name, ck) in [
        (&b"ctpub-slip0044"[..], &crypto[0]),
        (b"ctpriv-slip0044", &crypto[1]),
        (b"ctpub", &crypto[0]),
        (b"ctpriv", &crypto[1]),
    ] {
        if let Some(blob) = get(&db, main, name) {
            let s = String::from_utf8(open(ck, &blob)).expect("ASCII extended key");
            let prefix = if name.starts_with(b"ctpub") {
                "spub"
            } else {
                "sprv"
            };
            assert!(s.starts_with(prefix), "{s}");
        }
    }

    // Accounts: the xpub with the public crypto key; the xpriv with the
    // private one, or with the account's own Argon2id key.
    let unique: Vec<u64> = expect["unique_accounts"]
        .as_array()
        .expect("unique_accounts")
        .iter()
        .map(|v| v.as_u64().expect("account"))
        .collect();
    let mut walk = DbWalk::default();
    for (k, row) in pairs(&db, &[WADDRMGR, b"acct"]) {
        let account = u32::from_le_bytes(k.as_slice().try_into().expect("4-byte key"));
        let (_, pubk, privk) = account_row(&row);
        if pubk.is_empty() {
            continue; // the imported-address account
        }
        let xpub = String::from_utf8(open(&crypto[0], &pubk)).expect("ASCII xpub");
        assert!(xpub.starts_with("spub"), "{xpub}");
        let kdf = get(&db, &[WADDRMGR, b"acctvars", &k], b"kdf-params");
        let xpriv = match kdf {
            None => {
                assert!(
                    !unique.contains(&u64::from(account)),
                    "account {account} has no kdf-params"
                );
                open(&crypto[1], &privk)
            }
            Some(stored) => {
                let params = Argon2idParams::unmarshal_binary(&stored).expect("kdf-params");
                assert_eq!(params.marshal_binary().to_vec(), stored);
                walk.unique_threads.push(params.threads);
                let (pp, wrong_member) = if u64::from(account) == unique[0] {
                    ("acct_pass", "unlock_account_wrong")
                } else {
                    assert_eq!(u64::from(account), unique[1]);
                    ("voting_pass", "unlock_voting_wrong")
                };
                let key = params.derive_key(&pass(pp)).expect("derive");
                let pt = key.unseal(&privk).expect("the right account passphrase");
                let resealed = key
                    .seal_with(pt.as_bytes(), &mut Replay::new(privk[..24].to_vec()))
                    .expect("reseal");
                assert_eq!(resealed, privk);
                let wrong_key = params.derive_key(&pass("wrong_pass")).expect("derive");
                let e = wrong_key
                    .unseal(&privk)
                    .expect_err("wrong account passphrase");
                assert_eq!(err_object(&e), expect[wrong_member], "account {account}");
                pt.as_bytes().to_vec()
            }
        };
        let xpriv = String::from_utf8(xpriv).expect("ASCII xpriv");
        walk.accounts.insert(account.to_string(), xpriv);
    }
    let want: BTreeMap<String, String> =
        serde_json::from_value(expect["accounts"].clone()).expect("accounts");
    assert_eq!(walk.accounts, want);
    assert_eq!(walk.unique_threads.len(), 2);

    // Imported keys and scripts.
    for (_, row) in pairs(&db, &[WADDRMGR, b"addr"]) {
        let (ty, raw) = address_row(&row);
        match ty {
            1 => {
                let (pubk, privk) = two_fields(&raw);
                assert_eq!(
                    json!(hex(&open(&crypto[0], &pubk))),
                    expect["import_pubkey"]
                );
                assert_eq!(
                    json!(hex(&open(&crypto[1], &privk))),
                    expect["import_privkey"]
                );
                walk.imported_keys += 1;
            }
            2 => {
                let (hash, _script) = two_fields(&raw);
                assert_eq!(
                    json!(hex(&open(&crypto[0], &hash))),
                    expect["script_hash160"]
                );
                walk.scripts += 1;
            }
            _ => {}
        }
    }
    assert_eq!((walk.imported_keys, walk.scripts), (1, 1));
    assert_eq!(expect["unlock_account_ok"], json!(true));
    assert_eq!(expect["unlock_voting_ok"], json!(true));
    assert_eq!(expect["db_version"], json!(26));
    walk
}

/// The expectations file of the frozen generated database: its make-db
/// request (without the path), Go's result and the decompressed database's
/// SHA-256.
pub fn read_generated_expect() -> Value {
    let text =
        std::fs::read_to_string(data_dir().join(GENERATED_EXPECT)).expect("read expectations");
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("no {key:?} header"))
            .to_owned()
    };
    let body: Vec<&str> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(body.len().to_string(), field("# entries:"));
    assert_eq!(body_sha256(&text), field("# body-sha256:"), "body hash");
    serde_json::from_str(body[0]).expect("expectations line")
}
