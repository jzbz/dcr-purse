// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the integration tests: the injected random source, the Rust
//! side of every oracle command, the request corpus, the frozen-vector file
//! format, a model of the ciphertext tail's decoding, and a model of Go's
//! `createCipher` that builds the ciphertexts failing exactly one of
//! decapsulation's three checks.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    dead_code,
    reason = "test code: arithmetic and indexing on small test values; each test binary uses part"
)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, unhex};
use sntrup4591761::{
    CIPHERTEXT_SIZE, Ciphertext, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE, PrivateKey, PublicKey,
    decapsulate, encapsulate, generate_key,
};

/// Bytes one random polynomial draws.
pub const POLY_BYTES: usize = 3044;

/// The frozen vectors, relative to the crate.
pub const VECTORS: &str = "tests/data/sntrup_vectors.txt";
/// The seed of the frozen corpus.
pub const FROZEN_SEED: u64 = 0x736e_7472_7570_0001;

/// The stream a test injects, with Go's `bytes.Reader` behaviour: it
/// serves bytes until it runs out, then fails having served what it had.
pub struct Stream {
    bytes: Vec<u8>,
    pos: usize,
}

/// What a [`Stream`] reports when it runs out: the text Go's `io.ReadFull`
/// returns for the same reader.
#[derive(Debug, PartialEq, Eq)]
pub struct Exhausted(pub &'static str);

impl Stream {
    pub fn new(bytes: Vec<u8>) -> Self {
        Stream { bytes, pos: 0 }
    }

    pub fn consumed(&self) -> usize {
        self.pos
    }

    /// A `RandomSource` closure over the stream.
    ///
    /// Go reads each polynomial as 761 four-byte `io.ReadFull` calls; when
    /// the stream ends inside one, that call returns "unexpected EOF", and
    /// when it ends exactly between two, "EOF". The crate draws the 3044
    /// bytes in one call, so the closure works out which of Go's reads the
    /// stream would have ended in.
    pub fn source(&mut self) -> impl FnMut(&mut [u8]) -> Result<(), Exhausted> + '_ {
        move |buf: &mut [u8]| {
            let left = self.bytes.len() - self.pos;
            if left >= buf.len() {
                buf.copy_from_slice(&self.bytes[self.pos..self.pos + buf.len()]);
                self.pos += buf.len();
                Ok(())
            } else {
                buf[..left].copy_from_slice(&self.bytes[self.pos..]);
                self.pos = self.bytes.len();
                Err(Exhausted(if left.is_multiple_of(4) {
                    "EOF"
                } else {
                    "unexpected EOF"
                }))
            }
        }
    }
}

fn field<'a>(args: &'a Value, name: &str) -> &'a str {
    args[name]
        .as_str()
        .unwrap_or_else(|| panic!("request has no string {name:?}: {args}"))
}

fn fixed<const N: usize>(args: &Value, name: &str) -> [u8; N] {
    unhex(field(args, name))
        .try_into()
        .unwrap_or_else(|_| panic!("{name} is not {N} bytes"))
}

/// The outcome shape both sides are compared in: `{"ok": result}` or
/// `{"err": {"text", "kind"}}`, `result` in the oracle's own shapes.
pub fn ok(result: Value) -> Value {
    json!({ "ok": result })
}

fn rand_err(e: Exhausted, consumed: usize) -> Value {
    json!({ "err": { "text": format!("{}\t{consumed}", e.0), "kind": "rand" } })
}

/// The Rust crate's outcome for a request, in the oracle's shapes.
pub fn rust_outcome(cmd: &str, args: &Value) -> Value {
    match cmd {
        "keygen" => {
            let mut s = Stream::new(unhex(field(args, "rand")));
            let r = generate_key(&mut s.source());
            match r {
                Ok((pk, sk)) => {
                    let consumed = s.consumed();
                    ok(json!({
                        "pk": hex(pk.as_bytes()),
                        "sk": hex(sk.as_bytes()),
                        "consumed": consumed,
                    }))
                }
                Err(e) => rand_err(e, s.consumed()),
            }
        }
        "encapsulate" => {
            let pk = PublicKey::from_bytes(fixed::<PUBLIC_KEY_SIZE>(args, "pk"));
            let mut s = Stream::new(unhex(field(args, "rand")));
            let r = encapsulate(&mut s.source(), &pk);
            match r {
                Ok((ct, key)) => {
                    let consumed = s.consumed();
                    ok(json!({
                        "ct": hex(ct.as_bytes()),
                        "key": hex(key.as_bytes()),
                        "consumed": consumed,
                    }))
                }
                Err(e) => rand_err(e, s.consumed()),
            }
        }
        "decapsulate" => {
            let ct = Ciphertext::from_bytes(fixed::<CIPHERTEXT_SIZE>(args, "ct"));
            let sk = PrivateKey::from_bytes(&fixed::<PRIVATE_KEY_SIZE>(args, "sk"));
            let (key, good) = decapsulate(&ct, &sk);
            ok(json!({ "key": hex(key.as_bytes()), "ok": u8::from(bool::from(good)) }))
        }
        other => panic!("no Rust side for {other:?}"),
    }
}

/// The oracle's outcome for a request.
pub fn go_outcome(oracle: &mut purse_testutil::Oracle, cmd: &str, args: &Value) -> Value {
    match oracle.try_call(cmd, args.clone()) {
        Ok(v) => ok(v),
        Err(e) => json!({ "err": { "text": e.text, "kind": e.kind } }),
    }
}

/// One request of the corpus.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub cmd: String,
    pub args: Value,
    /// What the request exercises, for the counts in failure messages.
    pub label: &'static str,
    /// For a decapsulation built to fail exactly one of Go's three checks:
    /// the key (hex) Go must return, the SHA-512 half of the r the
    /// ciphertext was built from. Equal keys show that decryption gave
    /// that r back, so the other two checks passed and the one under test
    /// alone decided `ok`. Not sent to either side.
    pub expect_key: Option<String>,
}

fn req(cmd: &str, label: &'static str, args: Value) -> Request {
    Request {
        cmd: cmd.to_owned(),
        args,
        label,
        expect_key: None,
    }
}

/// How many instances of each kind [`corpus`] builds.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// Key pairs generated from random streams; every other request is
    /// built around them.
    pub keys: usize,
    /// Encapsulations to each generated key.
    pub encaps_per_key: usize,
    /// Fully random public keys, private keys and ciphertexts.
    pub random_inputs: usize,
}

/// The frozen corpus's plan.
pub const FROZEN_PLAN: Plan = Plan {
    keys: 8,
    encaps_per_key: 2,
    random_inputs: 8,
};

/// A seeded stream: `len` bytes of `SplitMix64(seed)`, after an optional
/// literal prefix. Requests carry this compact form; [`resolve`] expands it.
fn seeded(rng: &mut SplitMix64, prefix: &[u8], len: usize) -> (Value, Vec<u8>) {
    let seed = rng.next_u64();
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(&SplitMix64(seed).bytes(len));
    let args = json!({ "rand": hex(prefix), "rand_seed": seed, "rand_len": len });
    (args, bytes)
}

/// Expands a request's compact arguments into what both sides take:
/// `rand_seed`/`rand_len` become the bytes of `SplitMix64(rand_seed)`
/// appended to `rand`, and `sk_ref`/`pk_ref` (the index of an earlier
/// keygen request) become that request's private or public key, taken
/// from `outcomes`, the outcomes of the requests before it.
pub fn resolve(args: &Value, outcomes: &[Value]) -> Value {
    let mut out = args.clone();
    let map = out.as_object_mut().expect("arguments are an object");
    if let Some(seed) = map.remove("rand_seed") {
        let seed = seed.as_u64().expect("rand_seed");
        let len = map
            .remove("rand_len")
            .and_then(|l| l.as_u64())
            .expect("rand_len") as usize;
        let mut bytes = unhex(map.get("rand").and_then(Value::as_str).unwrap_or(""));
        bytes.extend_from_slice(&SplitMix64(seed).bytes(len));
        map.insert("rand".to_owned(), json!(hex(&bytes)));
    }
    for (key, part) in [("sk_ref", "sk"), ("pk_ref", "pk")] {
        if let Some(i) = map.remove(key) {
            let i = i.as_u64().expect(key) as usize;
            let v = outcomes[i]["ok"][part].clone();
            assert!(v.is_string(), "{key} {i} names no key pair");
            map.insert(part.to_owned(), v);
        }
    }
    out
}

/// Builds the request corpus from `rng`, in the compact form [`resolve`]
/// expands. Requests that depend on an earlier answer (an encapsulation to
/// a generated key, a decapsulation of its ciphertext) take it from the
/// Rust crate, which every request then checks against Go, so a corpus is
/// the same whichever side built it. The ciphertexts from an r of the
/// wrong weight are the test model's ([`model_create_cipher`]), since the
/// crate's encapsulation only makes weight 286.
pub fn corpus(rng: &mut SplitMix64, plan: Plan) -> Vec<Request> {
    let mut out = Vec::new();

    // Short streams: every way a draw can fail, at both of Go's error texts.
    for len in [0usize, 1, 3, 4, 5, 3040, 3043, POLY_BYTES - 1] {
        out.push(req("keygen", "keygen-short", seeded(rng, &[], len).0));
    }

    // Rejected g candidates: a random g is invertible in R3 almost always,
    // so these streams start with candidates that are not (g = 0, from
    // words that map to the coefficient 0) before random ones, and one is
    // cut right after the rejected candidates.
    for rejected in [1usize, 2] {
        let mut prefix = Vec::new();
        for _ in 0..rejected * 761 {
            // (0x2000_0000 & 0x3fff_ffff)·3 >> 30 = 1: the coefficient 0,
            // with random low bits that leave it there.
            let w = 0x2000_0000u32 | (rng.next_u64() as u32 & 0x00ff_ffff);
            prefix.extend_from_slice(&w.to_le_bytes());
        }
        out.push(req("keygen", "keygen-short", seeded(rng, &prefix, 0).0));
        out.push(req(
            "keygen",
            "keygen-retry",
            seeded(rng, &prefix, 2 * POLY_BYTES).0,
        ));
    }

    // (keygen request index, public key, private key)
    let mut keys: Vec<(usize, PublicKey, PrivateKey)> = Vec::new();
    for i in 0..plan.keys {
        // Two polynomials and a few bytes to spare; a stream whose first g
        // is rejected (essentially never) is replaced by the next.
        let (args, stream, pk, sk, used) = loop {
            let (args, stream) = seeded(rng, &[], 2 * POLY_BYTES + 9);
            let mut s = Stream::new(stream.clone());
            let r = generate_key(&mut s.source());
            if let Ok((pk, sk)) = r {
                let used = s.consumed();
                break (args, stream, pk, sk, used);
            }
        };
        let _ = stream;
        out.push(req("keygen", "keygen", args.clone()));
        keys.push((out.len() - 1, pk, sk));
        // Every fourth stream is also cut inside or right after f's draw.
        if i % 4 == 3 {
            for cut in [used - 1, used - 4, used - POLY_BYTES, used - POLY_BYTES - 2] {
                let mut short = args.clone();
                short["rand_len"] = json!(cut);
                out.push(req("keygen", "keygen-short", short));
            }
        }
    }

    for k in 0..keys.len() {
        let (kidx, pk, sk) = (&keys[k].0, &keys[k].1, &keys[k].2);
        for e in 0..plan.encaps_per_key {
            let (mut args, stream) = seeded(rng, &[], POLY_BYTES + (e % 2) * 7);
            let mut s = Stream::new(stream);
            let (ct, key) = encapsulate(&mut s.source(), pk).expect("enough bytes");
            args["pk_ref"] = json!(kidx);
            out.push(req("encapsulate", "encapsulate", args));
            let ct = *ct.as_bytes();
            out.push(decap_ref("decapsulate", &ct, *kidx));
            for t in tamperings(rng, &ct) {
                out.push(decap_ref("decapsulate-tampered", &t, *kidx));
            }
            // One body coefficient moved by ±3: r, and so the weight and
            // the confirmation, are unchanged; only the re-encryption
            // comparison at that coefficient fails.
            for j in 0..2 {
                let i = MOVED_COEFFICIENTS[(2 * e + j + 2 * k) % MOVED_COEFFICIENTS.len()];
                let moved = move_coefficient_either_way(&ct, i, rng.below(2) == 0);
                let mut r = decap_ref("decapsulate-moved-coefficient", &moved, *kidx);
                r.expect_key = Some(hex(key.as_bytes()));
                out.push(r);
            }
            for a in tail_aliases(&ct) {
                out.push(decap_ref("decapsulate-alias", &a, *kidx));
            }
            // The same ciphertext under another key pair.
            let other = keys[(k + 1) % keys.len()].0;
            out.push(decap_ref("decapsulate-wrong-key", &ct, other));
            // A malformed private key.
            out.push(decap("decapsulate-bad-sk", &ct, &bad_sk(rng, sk, e)));
        }
        // Ciphertexts built honestly (Go's createCipher, as the test model
        // computes it) from an r whose weight is not 286: decryption gives
        // r back and it re-encrypts and hashes correctly, so only the
        // weight check rejects them.
        for j in 0..2 {
            let w = WRONG_WEIGHTS[(2 * k + j) % WRONG_WEIGHTS.len()];
            let r = small_of_weight(rng, w);
            let (ct, key) = model_create_cipher(&r, pk);
            let mut q = decap_ref("decapsulate-wrong-weight", &ct, *kidx);
            q.expect_key = Some(hex(&key));
            out.push(q);
        }
        // A short encapsulation stream.
        let cut = [0usize, 2, 4, POLY_BYTES - 1][k % 4];
        let (mut args, _) = seeded(rng, &[], cut);
        args["pk_ref"] = json!(kidx);
        out.push(req("encapsulate", "encapsulate-short", args));
    }

    for i in 0..plan.random_inputs {
        let pk = special_bytes::<PUBLIC_KEY_SIZE>(rng, i);
        let (mut args, _) = seeded(rng, &[], POLY_BYTES);
        args["pk"] = json!(hex(&pk));
        out.push(req("encapsulate", "encapsulate-bad-pk", args));
        let ct = special_bytes::<CIPHERTEXT_SIZE>(rng, i);
        let sk = special_bytes::<PRIVATE_KEY_SIZE>(rng, i + 1);
        out.push(req(
            "decapsulate",
            "decapsulate-random",
            json!({ "ct": hex(&ct), "sk": hex(&sk) }),
        ));
        // A random ciphertext under a real key.
        out.push(decap_ref(
            "decapsulate-random-ct",
            &ct,
            keys[i % keys.len()].0,
        ));
    }
    out
}

fn decap(label: &'static str, ct: &[u8; CIPHERTEXT_SIZE], sk: &PrivateKey) -> Request {
    req(
        "decapsulate",
        label,
        json!({ "ct": hex(ct), "sk": hex(sk.as_bytes()) }),
    )
}

fn decap_ref(label: &'static str, ct: &[u8; CIPHERTEXT_SIZE], keygen: usize) -> Request {
    req(
        "decapsulate",
        label,
        json!({ "ct": hex(ct), "sk_ref": keygen }),
    )
}

/// Byte strings that stress the decoders: random, all zero, all 0xff, and
/// random with every byte's top bits set.
fn special_bytes<const N: usize>(rng: &mut SplitMix64, i: usize) -> [u8; N] {
    let mut b = [0u8; N];
    match i % 4 {
        0 => rng.fill(&mut b),
        1 => {}
        2 => b = [0xff; N],
        _ => {
            rng.fill(&mut b);
            for x in &mut b {
                *x |= 0xc0;
            }
        }
    }
    b
}

/// Tampered copies of a ciphertext: one bit flipped in the confirmation,
/// one in the body, one in the last three bytes, and the body's last byte
/// set to 0xff.
pub fn tamperings(rng: &mut SplitMix64, ct: &[u8; CIPHERTEXT_SIZE]) -> Vec<[u8; CIPHERTEXT_SIZE]> {
    let mut out = Vec::new();
    for range in [0..32usize, 32..1044, 1044..1047] {
        let mut t = *ct;
        let i = range.start + rng.below((range.end - range.start) as u64) as usize;
        t[i] ^= 1 << rng.below(8);
        out.push(t);
    }
    let mut t = *ct;
    t[1046] = 0xff;
    if t != *ct {
        out.push(t);
    }
    out
}

/// A malformed private key: the two-bit value 3 written into f, into 1/g,
/// or random bytes over the stored public key.
fn bad_sk(rng: &mut SplitMix64, sk: &PrivateKey, which: usize) -> PrivateKey {
    let mut b = *sk.as_bytes();
    match which % 3 {
        0 => b[rng.below(191) as usize] |= 0x03,
        1 => b[191 + rng.below(191) as usize] |= 0x0c,
        _ => {
            let at = 382 + rng.below(1218) as usize;
            rng.fill(&mut b[at..]);
        }
    }
    PrivateKey::from_bytes(&b)
}

/// Go's decoding of the last three bytes of a ciphertext body
/// (`rq/encoding.go:213-230`) to the two digits f0, f1 before the final
/// `·3 + 2296` and reduction, written out again here as a model.
fn tail_digits(b: [u8; 3]) -> (u32, u32) {
    let mut c0 = u32::from(b[0]);
    let mut c1 = u32::from(b[1]);
    let c2 = u32::from(b[2]);
    let f1 = 89_478_485u32
        .wrapping_mul(c2)
        .wrapping_add(349_525u32.wrapping_mul(c1))
        .wrapping_add(1365u32.wrapping_mul(c0.wrapping_add(1)))
        >> 21;
    c1 = c1.wrapping_add(c2 << 8);
    c1 = c1.wrapping_sub((f1.wrapping_mul(3)) << 1);
    c0 = c0.wrapping_add(c1 << 8);
    (c0, f1)
}

/// Go's coefficient for a decoded digit: `modq.Freeze(int32(d·3 + 2296))`.
fn coefficient(d: u32) -> i16 {
    let a = d.wrapping_mul(3).wrapping_add(4591 - 2295) as i32;
    let a = a.wrapping_sub(4591i32.wrapping_mul(228i32.wrapping_mul(a) >> 20));
    let a = a.wrapping_sub(
        4591i32.wrapping_mul(58_470i32.wrapping_mul(a).wrapping_add(134_217_728) >> 28),
    );
    a as i16
}

/// For every 24-bit tail value that decodes to a valid pair of rounded
/// coefficients, the encoder's canonical value for that pair, keyed by the
/// canonical value: the aliases of each canonical tail.
fn alias_table() -> &'static HashMap<u32, Vec<u32>> {
    static TABLE: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let valid = |x: i16| (-2295..=2295).contains(&x) && (x + 2295) % 3 == 0;
        let mut table: HashMap<u32, Vec<u32>> = HashMap::new();
        for v in 0u32..1 << 24 {
            let b = v.to_le_bytes();
            let (d0, d1) = tail_digits([b[0], b[1], b[2]]);
            let (a, c) = (coefficient(d0), coefficient(d1));
            if valid(a) && valid(c) {
                // EncodeRounded: f0 + 3·512·f1, f = (x + 2295)/3.
                let canon = ((a + 2295) / 3) as u32 + 1536 * ((c + 2295) / 3) as u32;
                if canon != v {
                    table.entry(canon).or_default().push(v);
                }
            }
        }
        table
    })
}

/// Every other encoding of `ct`'s last three bytes that Go decodes to the
/// same two coefficients (Quirk QK-0740).
pub fn tail_aliases(ct: &[u8; CIPHERTEXT_SIZE]) -> Vec<[u8; CIPHERTEXT_SIZE]> {
    let tail = u32::from_le_bytes([ct[1044], ct[1045], ct[1046], 0]);
    alias_table()
        .get(&tail)
        .map(|vs| {
            vs.iter()
                .map(|v| {
                    let mut a = *ct;
                    a[1044..].copy_from_slice(&v.to_le_bytes()[..3]);
                    a
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Statistics of the alias table: (canonical tails with an alias, aliases
/// in all, tails with exactly one, tails with exactly two).
pub fn alias_stats() -> (usize, usize, usize, usize) {
    let t = alias_table();
    let total = t.values().map(Vec::len).sum();
    let one = t.values().filter(|v| v.len() == 1).count();
    let two = t.values().filter(|v| v.len() == 2).count();
    (t.len(), total, one, two)
}

// ---- Ciphertexts that fail exactly one check ----
//
// Go's Decapsulate ANDs three checks (sntrup4591761.go:121-164): the
// decrypted r has weight 286, Round3(h·r) equals the decoded c at all 761
// coefficients, and SHA-512(zx.Encode(r)) starts with the confirmation.
// Random tampering almost always changes the decrypted r and so fails all
// three at once, which leaves a port that dropped one of them
// indistinguishable from Go. The inputs below fail one check alone.

/// Body coefficients the corpus moves by ±3: each position within a
/// four-byte group (0, 1, 2), the two of the three-byte tail (759, 760),
/// and some in between.
pub const MOVED_COEFFICIENTS: [usize; 8] = [0, 760, 1, 759, 2, 380, 758, 381];

/// Weights other than 286 the corpus builds ciphertexts from. Decryption
/// is guaranteed to give r back up to weight 289 (the coefficients of
/// 3·c·f are g·r + 3·e·f, at most 2·w + 6·286 in magnitude, against
/// q/2 = 2295.5); the heavy 400 decrypts with overwhelming probability,
/// which each request confirms through its expected key.
pub const WRONG_WEIGHTS: [usize; 5] = [0, 285, 287, 400, 1];

const P: usize = 761;
const Q: i64 = 4591;

/// A small element of weight `w`: `w` distinct random positions, each ±1.
pub fn small_of_weight(rng: &mut SplitMix64, w: usize) -> [i8; P] {
    let mut idx: Vec<usize> = (0..P).collect();
    for i in 0..w {
        let j = i + rng.below((P - i) as u64) as usize;
        idx.swap(i, j);
    }
    let mut r = [0i8; P];
    for &i in &idx[..w] {
        r[i] = if rng.below(2) == 0 { 1 } else { -1 };
    }
    r
}

/// The t-small element `zx.RandomTSmall` makes of 3044 bytes, written out
/// again as a model: 761 little-endian words, the low bits forced so that
/// the first 286 give ±1 and the rest 0, an ascending sort as unsigned
/// integers (the sort the crate implements: Go 1.26 or later on a 64-bit
/// target), then (w & 3) − 1.
pub fn model_random_tsmall(bytes: &[u8]) -> [i8; P] {
    assert_eq!(bytes.len(), POLY_BYTES);
    let mut w: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    for x in &mut w[..286] {
        *x &= !1;
    }
    for x in &mut w[286..] {
        *x = (*x & !2) | 1;
    }
    w.sort_unstable();
    std::array::from_fn(|i| (w[i] & 3) as i8 - 1)
}

/// Go's `rq.Decode` of a canonical public key, the encoding `rq.Encode`
/// writes for coefficients in −2295..=2295: each eight-byte group is the
/// little-endian integer Σ (h[5g+k] + 2295)·6144^k over k < 5
/// (`rq/encoding.go:14-60`, where 3·2^11 = 6144 and the shifts are its
/// powers), and the last two bytes hold h[760] + 2295. Panics on a key
/// that is not canonical; every key `generate_key` makes is.
pub fn model_decode_pk(pk: &PublicKey) -> [i16; P] {
    let b = pk.as_bytes();
    let mut h = [0i16; P];
    for g in 0..152 {
        let mut v = u64::from_le_bytes(b[8 * g..8 * g + 8].try_into().expect("eight bytes"));
        for k in 0..5 {
            let a = v % 6144;
            v /= 6144;
            assert!(a < 4591, "not a canonical public key");
            h[5 * g + k] = a as i16 - 2295;
        }
        assert_eq!(v, 0, "not a canonical public key");
    }
    let a = u16::from_le_bytes([b[1216], b[1217]]);
    assert!(a < 4591, "not a canonical public key");
    h[760] = a as i16 - 2295;
    h
}

/// h·r in Rq = Z_q[x]/(x^761 − x − 1), coefficients in −2295..=2295:
/// the integer product, reduced by x^761 = x + 1 from the top, then mod q.
pub fn model_mult(h: &[i16; P], r: &[i8; P]) -> [i16; P] {
    let mut fg = vec![0i64; 2 * P - 1];
    for (i, &ri) in r.iter().enumerate() {
        for (j, &hj) in h.iter().enumerate() {
            fg[i + j] += i64::from(hj) * i64::from(ri);
        }
    }
    for i in (P..2 * P - 1).rev() {
        fg[i - P] += fg[i];
        fg[i - P + 1] += fg[i];
    }
    std::array::from_fn(|i| {
        let m = fg[i].rem_euclid(Q);
        (if m > 2295 { m - Q } else { m }) as i16
    })
}

/// Go's createCipher (`sntrup4591761.go:82-103`) on a canonical public
/// key, through the model: c = Round3(h·r) (`rq.Round3`), the ciphertext
/// SHA-512(zx.Encode(r))[0..32) ‖ rq.EncodeRounded(c), and the key
/// SHA-512(zx.Encode(r))[32..64). `r` may have any weight, which Go's
/// Encapsulate never produces.
pub fn model_create_cipher(r: &[i8; P], pk: &PublicKey) -> ([u8; CIPHERTEXT_SIZE], [u8; 32]) {
    use sha2::{Digest, Sha512};
    let mut c = model_mult(&model_decode_pk(pk), r);
    for x in &mut c {
        *x = (((21846 * (i32::from(*x) + 2295) + 32768) >> 16) * 3 - 2295) as i16;
    }
    // zx.Encode: four coefficients per byte, c + 1 in two bits.
    let mut enc = [0u8; 191];
    for (i, byte) in enc.iter_mut().enumerate() {
        for k in 0..4 {
            if let Some(&x) = r.get(4 * i + k) {
                *byte |= ((x + 1) as u8) << (2 * k);
            }
        }
    }
    let s = Sha512::digest(enc);
    let mut ct = [0u8; CIPHERTEXT_SIZE];
    ct[..32].copy_from_slice(&s[..32]);
    // rq.EncodeRounded: digits d = (x + 2295)/3 in 0..=1530, three to a
    // little-endian four-byte group in base 1536, the last two in three
    // bytes.
    let d = |x: i16| ((21846 * (i32::from(x) + 2295)) >> 16) as u32;
    for g in 0..253 {
        let v = d(c[3 * g]) + 1536 * d(c[3 * g + 1]) + 1536 * 1536 * d(c[3 * g + 2]);
        ct[32 + 4 * g..36 + 4 * g].copy_from_slice(&v.to_le_bytes());
    }
    let v = d(c[759]) + 1536 * d(c[760]);
    ct[1044..].copy_from_slice(&v.to_le_bytes()[..3]);
    let mut key = [0u8; 32];
    key.copy_from_slice(&s[32..]);
    (ct, key)
}

/// `ct` with body coefficient `i` moved by 3·`delta` (`delta` ±1) and
/// re-encoded as `rq.EncodeRounded` writes it, or `None` if the moved
/// coefficient would leave −2295..=2295. Panics if `ct`'s body is not
/// canonical; every honest ciphertext's is.
pub fn move_coefficient(
    ct: &[u8; CIPHERTEXT_SIZE],
    i: usize,
    delta: i32,
) -> Option<[u8; CIPHERTEXT_SIZE]> {
    assert!(i < P && delta.abs() == 1);
    let (at, len, pos) = if i < 759 {
        (32 + 4 * (i / 3), 4, i % 3)
    } else {
        (1044, 3, i - 759)
    };
    let mut v = 0u32;
    for k in 0..len {
        v |= u32::from(ct[at + k]) << (8 * k);
    }
    let mut digits = [v % 1536, (v / 1536) % 1536, v / (1536 * 1536)];
    assert!(
        digits.iter().all(|&x| x <= 1530),
        "not a canonical ciphertext body"
    );
    let moved = i64::from(digits[pos]) + i64::from(delta);
    if !(0..=1530).contains(&moved) {
        return None;
    }
    digits[pos] = moved as u32;
    let v = digits[0] + 1536 * digits[1] + 1536 * 1536 * digits[2];
    let mut out = *ct;
    for k in 0..len {
        out[at + k] = (v >> (8 * k)) as u8;
    }
    Some(out)
}

/// [`move_coefficient`] up (by +3) if `up` and that stays in range, else
/// the other way; one of the two always does.
pub fn move_coefficient_either_way(
    ct: &[u8; CIPHERTEXT_SIZE],
    i: usize,
    up: bool,
) -> [u8; CIPHERTEXT_SIZE] {
    let first = if up { 1 } else { -1 };
    move_coefficient(ct, i, first)
        .or_else(|| move_coefficient(ct, i, -first))
        .expect("a coefficient can move one way")
}

// ---- The frozen-vector file ----

/// One line of the frozen file: a request and Go's outcome.
pub struct Entry {
    pub cmd: String,
    pub args: Value,
    pub outcome: Value,
}

pub fn data_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(VECTORS)
}

/// SHA-256 of `data` in hex (the body hash), through the `sha2` the crate
/// depends on.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
}

impl Entry {
    pub fn to_line(&self) -> String {
        // serde_json's map is sorted (no preserve_order), so the line is
        // canonical.
        json!({ "cmd": self.cmd, "args": self.args, "outcome": self.outcome }).to_string()
    }

    pub fn from_line(line: &str) -> Self {
        let v: Value = purse_testutil::serde_json::from_str(line).expect("vector line is JSON");
        Entry {
            cmd: v["cmd"].as_str().expect("cmd").to_owned(),
            args: v["args"].clone(),
            outcome: v["outcome"].clone(),
        }
    }
}

/// Reads and verifies the frozen file: the body hash, the entry count and
/// the per-command counts in its header.
pub fn read_vectors() -> (String, Vec<Entry>) {
    let text = std::fs::read_to_string(data_path())
        .unwrap_or_else(|e| panic!("read {VECTORS}: {e} (it is checked in)"));
    let entries = verify_vectors(&text);
    (text, entries)
}

pub fn verify_vectors(text: &str) -> Vec<Entry> {
    let header = |key: &str| {
        text.lines()
            .take_while(|l| l.starts_with('#'))
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{VECTORS}: no {key:?} header"))
            .to_owned()
    };
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        header("# body-sha256:"),
        sha256_hex(body.join("\n").as_bytes()),
        "{VECTORS}: body hash"
    );
    let entries: Vec<Entry> = body.iter().map(|l| Entry::from_line(l)).collect();
    assert_eq!(
        header("# entries:").parse::<usize>().ok(),
        Some(entries.len()),
        "{VECTORS}: entry count"
    );
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &entries {
        *counts.entry(e.cmd.clone()).or_default() += 1;
    }
    for (cmd, n) in &counts {
        assert_eq!(
            header(&format!("# count {cmd}:")).parse::<usize>().ok(),
            Some(*n),
            "{VECTORS}: count of {cmd}"
        );
    }
    entries
}

/// Writes the frozen file.
pub fn write_vectors(header: &[String], entries: &[Entry]) {
    let body: Vec<String> = entries.iter().map(Entry::to_line).collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for e in entries {
        *counts.entry(&e.cmd).or_default() += 1;
    }
    let mut text = String::new();
    for h in header {
        if h.is_empty() {
            text.push_str("#\n");
        } else {
            text.push_str(&format!("# {h}\n"));
        }
    }
    text.push_str(&format!("# entries: {}\n", entries.len()));
    for (cmd, n) in counts {
        text.push_str(&format!("# count {cmd}: {n}\n"));
    }
    text.push_str(&format!(
        "# body-sha256: {}\n",
        sha256_hex(body.join("\n").as_bytes())
    ));
    for l in &body {
        text.push_str(l);
        text.push('\n');
    }
    std::fs::write(data_path(), text).expect("write the frozen vectors");
}

/// The irreducible factors of x^761 − x − 1 over GF(3), of degrees 19, 60
/// and 682, coefficient 0 first (`+` is 1, `-` is −1); the same as
/// `src/tests.rs`'s, which checks they multiply to the modulus. A multiple
/// of any of them is a g key generation must reject.
pub const R3_FACTORS: [&str; 3] = [
    "--0---+-+-0-++++-0++",
    "+-++00+-0-0-0+++++00-0++00+0+00--++-0++00++-00++++-00+-0--+0+",
    "++00-+++0+-0-+000+++--+0+0++0-+0-0-+00-+0+------0++++---0++0+000+-0--+0-+--0-0-+-0+00+-++000++000-000+++---00-0+--+-++0--+-00+-+-+-0++-+000-+00++0000-0-+0++-0++00+--+00+0+-0+++-0+0+--+--+-+0-+++0-++0+00-0++0000+--0--+-000-00+---+--0+0-++--000--0-+--++-0-00--00+0-0--0000+-0++0--+0+----+0+000+-0--0--+0+++0++000+0+-00-0+-+0+---++-++-+--+0000+0-0+0-00-00++-0++-0-0-+---+--00+-+0+-00+0--00-++--+0+0--00++++0-++---0++++0000++0+++--0++00---+00-0-+00+0--00-+0-+0++-++-++00-+-000--000+00-0+-++-0++++----00-++000-0-00---++-+0-0++++0+0++-++--0+-0++00++0--0---+---+++0+++0--+++++00++-----000++-++0++-+0+0-++++0+0++-00-000-++0000-0++-0-0--+0++0+-0-++--0--++++-++++0-----+---++++----00+0-+0+-0-+",
];

/// The 3044 bytes `zx.RandomSmall` reads as the small element x^shift·factor
/// (`factor` given as in [`R3_FACTORS`], `shift` + its degree below 761),
/// with random bits where they do not change the coefficient: a word w
/// gives ((w & 0x3fffffff)·3 >> 30) − 1, so 0x0000_0000, 0x2000_0000 and
/// 0x3000_0000 give −1, 0 and 1, and the top two bits and the low 24 bits
/// are free.
pub fn small_words(rng: &mut SplitMix64, factor: &str, shift: usize) -> Vec<u8> {
    assert!(
        shift + factor.len() <= 761,
        "x^{shift}·factor must not wrap"
    );
    let mut coeffs = vec![0i8; 761];
    for (i, ch) in factor.chars().enumerate() {
        coeffs[shift + i] = match ch {
            '+' => 1,
            '-' => -1,
            _ => 0,
        };
    }
    let mut out = Vec::with_capacity(POLY_BYTES);
    for c in coeffs {
        let base: u32 = match c {
            -1 => 0x0000_0000,
            0 => 0x2000_0000,
            _ => 0x3000_0000,
        };
        let free = rng.next_u64() as u32 & 0xc0ff_ffff;
        out.extend_from_slice(&(base | free).to_le_bytes());
    }
    out
}

/// Key generation streams whose first one or two g candidates are
/// multiples of a factor of the modulus (not invertible in R3), each
/// followed by `rest`: (label, stream, number of rejected candidates).
pub fn rejection_streams(rng: &mut SplitMix64, rest: &[u8]) -> Vec<(String, Vec<u8>, usize)> {
    let mut out = Vec::new();
    for (fi, factor) in R3_FACTORS.iter().enumerate() {
        let room = 761 - factor.len();
        for shift in [0, room / 2, room] {
            let mut s = small_words(rng, factor, shift);
            s.extend_from_slice(rest);
            out.push((format!("factor {fi}, x^{shift}"), s, 1));
        }
    }
    let mut s = small_words(rng, R3_FACTORS[0], 3);
    s.extend(small_words(rng, R3_FACTORS[1], 100));
    s.extend_from_slice(rest);
    out.push(("factors 0 and 1".to_owned(), s, 2));
    out
}

/// Whether optimised code is running, or `PURSE_SNTRUP_HEAVY` asks for the
/// larger sizes anyway.
pub fn heavy() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("PURSE_SNTRUP_HEAVY").is_some()
}

/// A count from the environment, or `default`.
pub fn env_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
