// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the oracle and frozen-vector tests: the Rust side's answers in
//! the oracle's JSON shapes (`tools/oracle/cmd/dcrutil`), Go's answers as
//! modelled per architecture, the input corpora, and the frozen file
//! format.

#![allow(dead_code)]
#![allow(
    clippy::excessive_precision,
    reason = "upstream test amounts are kept as Go writes them"
)]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

use std::cell::Cell;
use std::path::PathBuf;

use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut, TxSerializeType};
use purse_dcrutil::{Amount, AmountUnit, SignatureType, Wif, new_amount};
use purse_dcrutil::{gosort, txsort};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{Oracle, SplitMix64, hex, unhex};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------
// Architectures.
// ---------------------------------------------------------------------

/// How the Go oracle's build computes `round(x * y)` and converts floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    /// amd64 at GOAMD64 v1 or v2: the product and the sum rounded
    /// separately; `CVTTSD2SQ` (NaN and out of range give `i64::MIN`); 0/0
    /// is `0xfff8000000000000`. What purse-dcrutil does on every target.
    /// Not amd64 at v3 or v4: Go 1.27.4 fuses `x * y + 0.5` there
    /// (`VFMADD231SD` in its code for dcrutil v4.0.3), so such a build is
    /// refused rather than compared with this model.
    Amd64,
    /// arm64: `x * y ± 0.5` fused (`FMADDD`/`FNMSUBD`, verified in Go's
    /// assembly for dcrutil v4.0.3), the sign taken from the rounded
    /// product; `FCVTZS` (saturating, NaN gives 0); 0/0 is
    /// `0x7ff8000000000000`.
    Arm64,
}

impl Arch {
    /// The oracle's architecture. Panics for any other: no model of Go's
    /// answers exists for it, and a comparison would be blind.
    pub fn of(o: &mut Oracle) -> Arch {
        let a = o.call("arch", Value::Null);
        let goarch = a["goarch"].as_str().expect("goarch");
        let goamd64 = a["goamd64"].as_str().unwrap_or_default();
        match (goarch, goamd64) {
            ("amd64", "v1" | "v2") => Arch::Amd64,
            ("arm64", _) => Arch::Arm64,
            _ => panic!(
                "oracle built for {goarch} (GOAMD64={goamd64:?}): Go's NewAmount and MulF64 \
                 there are not modelled (amd64 at v3 and v4 fuses the non-negative \
                 multiply-add; riscv64, 386 and arm convert differently, see \
                 src/amount.rs); the differential tests know amd64 v1/v2 and arm64"
            ),
        }
    }
}

/// Go's `round(x * y)` (`amount.go:61-66` after inlining) on `arch`.
pub fn go_round_product(arch: Arch, x: f64, y: f64) -> i64 {
    let p = x * y;
    match arch {
        Arch::Amd64 => {
            let s = if p < 0.0 { p - 0.5 } else { p + 0.5 };
            if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&s) {
                s as i64
            } else {
                i64::MIN
            }
        }
        Arch::Arm64 => {
            let s = if p < 0.0 {
                x.mul_add(y, -0.5)
            } else {
                x.mul_add(y, 0.5)
            };
            // Rust's `as` is FCVTZS: saturating, NaN to 0.
            s as i64
        }
    }
}

/// Whether two answers of `round` differ while neither is an out-of-range
/// conversion (amd64's `i64::MIN`, arm64's saturation): a difference made
/// by the fused rounding alone.
pub fn in_range_difference(amd64: i64, arm64: i64) -> bool {
    amd64 != arm64 && amd64 != i64::MIN && arm64 != i64::MIN && arm64 != i64::MAX
}

/// Go's `NewAmount` on `arch`, in the oracle's shape.
pub fn go_new_amount(arch: Arch, f: f64) -> Value {
    if f.is_nan() || f.is_infinite() {
        json!({ "error": "invalid coin amount" })
    } else {
        json!({ "atoms": go_round_product(arch, f, 1e8) })
    }
}

/// Go's `MulF64` on `arch`.
pub fn go_mul_f64(arch: Arch, atoms: i64, f: f64) -> i64 {
    go_round_product(arch, atoms as f64, f)
}

/// The bits of a quotient as Go on `arch` produces them: only the NaN of
/// 0/0 differs between the two.
pub fn go_quotient_bits(arch: Arch, amd64_bits: &str) -> String {
    if arch == Arch::Arm64 && amd64_bits == "fff8000000000000" {
        "7ff8000000000000".to_owned()
    } else {
        amd64_bits.to_owned()
    }
}

/// The Go answer of an `amount.coin` or `amount.format` element on `arch`,
/// from the answer amd64 gives (the Rust crate's).
pub fn go_quotient_answer(arch: Arch, amd64: &Value, key: &str) -> Value {
    let mut v = amd64.clone();
    let bits = amd64[key].as_str().expect("bits");
    v[key] = json!(go_quotient_bits(arch, bits));
    v
}

// ---------------------------------------------------------------------
// The Rust side in the oracle's shapes.
// ---------------------------------------------------------------------

pub fn bits_hex(f: f64) -> String {
    format!("{:016x}", f.to_bits())
}

pub fn hex_bits(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).expect("bits"))
}

pub fn rust_new_amount(f: f64) -> Value {
    match new_amount(f) {
        Ok(a) => json!({ "atoms": a.0 }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

pub fn rust_coin(atoms: i64) -> Value {
    let a = Amount(atoms);
    json!({ "to_coin": bits_hex(a.to_coin()), "string": a.to_string() })
}

pub fn rust_format(atoms: i64, unit: i64) -> Value {
    let a = Amount(atoms);
    let u = AmountUnit(unit);
    json!({ "to_unit": bits_hex(a.to_unit(u)), "format": a.format(u) })
}

pub fn rust_unit(unit: i64) -> Value {
    json!(AmountUnit(unit).to_string())
}

pub fn rust_mul_f64(atoms: i64, f: f64) -> Value {
    json!(Amount(atoms).mul_f64(f).0)
}

fn wif_value(r: Result<Wif, purse_dcrutil::WifError>) -> Value {
    match r {
        Ok(w) => json!({
            "string": w.string().as_str(),
            "priv": hex(w.priv_key()),
            "pub": hex(w.pub_key()),
            "dsa": w.dsa().0,
            "net": hex(&w.net_id()),
        }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn net_of(v: &Value) -> [u8; 2] {
    unhex(v["net"].as_str().expect("net"))
        .try_into()
        .expect("2-byte net")
}

/// `wif.new` for one case `{"priv", "net", "scheme"}`.
pub fn rust_wif_new(case: &Value) -> Value {
    let key = unhex(case["priv"].as_str().expect("priv"));
    let scheme = SignatureType(case["scheme"].as_i64().expect("scheme"));
    wif_value(Wif::new(&key, net_of(case), scheme))
}

/// `wif.decode` for one case `{"wif", "net"}`.
pub fn rust_wif_decode(case: &Value) -> Value {
    wif_value(Wif::decode(
        case["wif"].as_str().expect("wif"),
        net_of(case),
    ))
}

/// `txsort.hex` for one serialized transaction.
pub fn rust_txsort_hex(tx_hex: &str) -> Value {
    let bytes = unhex(tx_hex);
    let (mut tx, used) = MsgTx::from_bytes(&bytes).expect("corpus transactions deserialize");
    assert_eq!(used, bytes.len(), "trailing bytes in a corpus transaction");
    let is_sorted = txsort::is_sorted(&tx);
    let hash = tx.tx_hash().to_string();
    let sorted = txsort::sort(&tx);
    let unmodified = hex(&tx.serialize()) == tx_hex;
    txsort::in_place_sort(&mut tx);
    json!({
        "is_sorted": is_sorted,
        "sorted": hex(&sorted.serialize()),
        "in_place": hex(&tx.serialize()),
        "unmodified": unmodified,
        "hash": hash,
        "sorted_hash": sorted.tx_hash().to_string(),
    })
}

/// A transaction's inputs and outputs in the oracle's field form.
pub fn tx_fields(tx: &MsgTx) -> Value {
    let inputs: Vec<Value> = tx
        .tx_in
        .iter()
        .map(|i| {
            json!({
                "hash": hex(&i.previous_out_point.hash.0),
                "index": i.previous_out_point.index,
                "tree": i.previous_out_point.tree,
                "sequence": i.sequence,
                "value_in": i.value_in,
                "block_height": i.block_height,
                "block_index": i.block_index,
                "script": hex(&i.signature_script),
            })
        })
        .collect();
    let outputs: Vec<Value> = tx
        .tx_out
        .iter()
        .map(|o| json!({ "value": o.value, "version": o.version, "script": hex(&o.pk_script) }))
        .collect();
    json!({ "inputs": inputs, "outputs": outputs })
}

/// The transaction a field form describes.
pub fn tx_from_fields(v: &Value) -> MsgTx {
    let mut tx = MsgTx::default();
    for i in v["inputs"].as_array().expect("inputs") {
        tx.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: dcroxide_chainhash::Hash(
                    unhex(i["hash"].as_str().expect("hash"))
                        .try_into()
                        .expect("32 bytes"),
                ),
                index: u32::try_from(i["index"].as_u64().expect("index")).expect("u32"),
                tree: i8::try_from(i["tree"].as_i64().expect("tree")).expect("i8"),
            },
            sequence: u32::try_from(i["sequence"].as_u64().expect("seq")).expect("u32"),
            value_in: i["value_in"].as_i64().expect("value_in"),
            block_height: u32::try_from(i["block_height"].as_u64().expect("h")).expect("u32"),
            block_index: u32::try_from(i["block_index"].as_u64().expect("i")).expect("u32"),
            signature_script: unhex(i["script"].as_str().expect("script")),
        });
    }
    for o in v["outputs"].as_array().expect("outputs") {
        tx.tx_out.push(TxOut {
            value: o["value"].as_i64().expect("value"),
            version: u16::try_from(o["version"].as_u64().expect("version")).expect("u16"),
            pk_script: unhex(o["script"].as_str().expect("script")),
        });
    }
    tx
}

/// `txsort.fields` for one transaction in field form.
pub fn rust_txsort_fields(v: &Value) -> Value {
    let mut tx = tx_from_fields(v);
    let is_sorted = txsort::is_sorted(&tx);
    let sorted = txsort::sort(&tx);
    txsort::in_place_sort(&mut tx);
    let in_place = tx_fields(&tx);
    json!({
        "is_sorted": is_sorted,
        "inputs": in_place["inputs"],
        "outputs": in_place["outputs"],
        "sort_agrees": tx_fields(&sorted) == in_place,
    })
}

// ---------------------------------------------------------------------
// Corpora.
// ---------------------------------------------------------------------

fn pick<T: Copy>(rng: &mut SplitMix64, items: &[T]) -> T {
    items[rng.below(items.len() as u64) as usize]
}

/// `f` moved by `k` units in the last place along the bit pattern (across
/// zero this walks into the other sign's patterns, which is fine for a
/// corpus).
fn ulps(f: f64, k: i64) -> f64 {
    f64::from_bits(f.to_bits().wrapping_add_signed(k))
}

/// A random float of a random binary magnitude between 2^lo and 2^hi.
fn magnitude(rng: &mut SplitMix64, lo: i32, hi: i32) -> f64 {
    let e = lo + rng.below((hi - lo + 1) as u64) as i32;
    let m = 1.0 + (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
    m * 2f64.powi(e)
}

/// Inputs for `NewAmount`: every special value and power of ten with its
/// neighbours, x.5 atom ties and exact atoms at many magnitudes with their
/// neighbours, the money supply and the `int64` limit with theirs, values
/// on which a fused multiply-add rounds differently, decimal strings, and
/// random bit patterns. `scale` multiplies the random parts.
pub fn new_amount_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        f64::NAN,
        -f64::NAN,
        f64::from_bits(0x7ff0_0000_0000_0001),
        f64::from_bits(0xfff4_0000_dead_beef),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::MAX,
        f64::MIN,
        1.0,
        -1.0,
        0.5e-8,
        -0.5e-8,
        1e-8,
        21e6,
        -21e6,
        21e6 + 1e-8,
        -21e6 - 1e-8,
        54.999999999999943157,
        55.000000000000056843,
        0.01234567,
    ];
    let neighbourhood = |v: &mut Vec<f64>, f: f64, k: i64| {
        for d in -k..=k {
            v.push(ulps(f, d));
            v.push(-ulps(f, d));
        }
    };
    for e in -330..=308 {
        let p: f64 = format!("1e{e}").parse().expect("power of ten");
        neighbourhood(&mut v, p, 2);
    }
    // The int64 limit in coins: f * 1e8 near 2^63.
    neighbourhood(&mut v, 9_223_372_036_854_775_808.0 / 1e8, 64);
    neighbourhood(&mut v, 92_233_720_368.547_758, 16);
    neighbourhood(&mut v, 21e6, 8);
    for bits in 0..64 {
        let atoms = (1u64 << bits) as f64;
        neighbourhood(&mut v, atoms / 1e8, 2);
        neighbourhood(&mut v, (atoms + 0.5) / 1e8, 3);
        neighbourhood(&mut v, (atoms - 0.5) / 1e8, 8);
    }
    for _ in 0..200 * scale {
        // Ties and exact atoms at random magnitudes.
        let k = (rng.next_u64() >> rng.below(64)) as f64;
        neighbourhood(&mut v, (k + 0.5) / 1e8, 2);
        neighbourhood(&mut v, k / 1e8, 1);
        // Decimal amounts with up to ten places, as RPC clients send them.
        let whole = rng.next_u64() >> (11 + rng.below(53));
        let places = rng.below(11) as usize;
        let frac = rng.next_u64() % 10u64.pow(places as u32).max(1);
        let s = format!("{whole}.{frac:0places$}");
        let f: f64 = s.parse().expect("decimal");
        v.push(f);
        v.push(-f);
        // Random magnitudes and random bit patterns.
        v.push(magnitude(rng, -40, 70));
        v.push(-magnitude(rng, -40, 70));
        v.push(f64::from_bits(rng.next_u64()));
    }
    // Values on which Go's arm64 build, rounding x*1e8 + 0.5 once, gives
    // another atom count than amd64 in range: where amd64's separately
    // rounded sum is a tie, which happens only where the sum crosses a
    // power of two (atom counts just below 2^m, and half an atom). Out of
    // range the two differ for every positive value; the random parts
    // above have plenty of those.
    let mut found = 0;
    let mut tries = 0;
    while found < 10 * scale && tries < 1_000_000 * scale {
        tries += 1;
        let m = rng.below(64) as i32;
        let base = (2f64.powi(m) - 0.5) / 1e8;
        let f = ulps(base, rng.below(129) as i64 - 64);
        let f = if rng.below(2) == 0 { -f } else { f };
        if in_range_difference(
            go_round_product(Arch::Amd64, f, 1e8),
            go_round_product(Arch::Arm64, f, 1e8),
        ) {
            v.push(f);
            found += 1;
        }
    }
    v
}

/// Amounts for `ToCoin`, `String` and `Format`.
pub fn atoms_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<i64> {
    let mut v = vec![
        0,
        1,
        -1,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
        i64::MAX - 1,
        2_100_000_000_000_000,
        -2_100_000_000_000_000,
        2_100_000_000_000_001,
        -2_100_000_000_000_001,
        44_433_322_211_100,
        100_000_000,
        100_000,
        10_000_000,
        20_000_000,
        20_400_000,
        1 << 53,
        (1 << 53) + 1,
        -(1 << 53) - 1,
    ];
    for bits in 0..63 {
        let p = 1i64 << bits;
        for d in [-1, 0, 1] {
            v.push(p + d);
            v.push(-(p + d));
        }
    }
    let mut p = 1i64;
    while let Some(next) = p.checked_mul(10) {
        for d in [-1, 0, 1] {
            v.push(p + d);
            v.push(-p - d);
        }
        p = next;
    }
    for _ in 0..300 * scale {
        let a = (rng.next_u64() >> rng.below(64)) as i64;
        v.push(a);
        v.push(a.wrapping_neg());
        // Whole and round coin amounts.
        v.push((rng.below(21_000_000) as i64) * 100_000_000);
        v.push((rng.below(21_000_000_000) as i64) * 100_000);
    }
    v
}

/// Units for `Format` and `AmountUnit.String`: the named ones, every unit
/// from -340 to 320, and far units on both sides, avoiding those for which
/// Go's `FormatFloat` panics or tries to allocate gigabytes (below -2^20,
/// and `i64::MAX - 2 ..= i64::MAX`). The wrapping units `i64::MAX - 7 ..=
/// i64::MAX - 3`, for which Go's buffer size wraps to a small one, are
/// included.
pub fn units_corpus() -> Vec<i64> {
    let mut v: Vec<i64> = (-340..=320).collect();
    v.extend([
        1000,
        1 << 20,
        1 << 40,
        i64::MAX - 8,
        i64::MAX - 7,
        i64::MAX - 6,
        i64::MAX - 5,
        i64::MAX - 4,
        i64::MAX - 3,
        -1000,
        -12345,
        -(1 << 20),
    ]);
    v
}

/// Every unit `AmountUnit.String` can be asked about, panicking ones
/// included (`String` itself never panics).
pub fn unit_names_corpus(rng: &mut SplitMix64) -> Vec<i64> {
    let mut v = units_corpus();
    v.extend([i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1, i64::MAX - 2]);
    for _ in 0..200 {
        v.push(rng.next_u64() as i64);
    }
    v
}

/// `(atoms, unit)` pairs for `Format`: every unit with a handful of
/// amounts, every amount with a handful of units.
pub fn format_corpus(rng: &mut SplitMix64, atoms: &[i64], units: &[i64]) -> Vec<(i64, i64)> {
    let named = [6, 3, 0, -3, -6, -8, -1, -9, -12];
    let mut v = Vec::new();
    for &u in units {
        for _ in 0..4 {
            v.push((pick(rng, atoms), u));
        }
        v.push((0, u));
        v.push((i64::MAX, u));
        v.push((i64::MIN, u));
    }
    for &a in atoms {
        for _ in 0..3 {
            v.push((a, pick(rng, &named)));
        }
        v.push((a, pick(rng, units)));
    }
    v
}

/// `(atoms, multiplier)` pairs for `MulF64`, with multipliers on which the
/// fused multiply-add differs.
pub fn mul_corpus(rng: &mut SplitMix64, atoms: &[i64], scale: usize) -> Vec<(i64, f64)> {
    let specials = [
        0.0,
        -0.0,
        0.5,
        -0.5,
        1.0,
        -1.0,
        2.0,
        1.02,
        0.01,
        0.66,
        0.666,
        2.0 / 3.0,
        1e-8,
        1e8,
        1e300,
        -1e300,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
    ];
    let mut v = Vec::new();
    for &f in &specials {
        for _ in 0..20 {
            v.push((pick(rng, atoms), f));
        }
    }
    for _ in 0..1000 * scale {
        let f = match rng.below(4) {
            0 => magnitude(rng, -40, 10),
            1 => -magnitude(rng, -40, 10),
            2 => f64::from_bits(rng.next_u64()),
            _ => (rng.below(10_001) as f64) / 10_000.0,
        };
        v.push((pick(rng, atoms), f));
    }
    // Multipliers on which Go's arm64 build gives another answer: out of
    // range (any positive product beyond int64), and in range where the
    // product lands just below a power of two minus half an atom.
    let mut found = 0;
    let mut tries = 0;
    while found < 10 * scale && tries < 1_000_000 * scale {
        tries += 1;
        let a = (rng.next_u64() >> (rng.below(40) + 24)) as i64;
        let a = if tries % 2 == 0 { -a } else { a };
        let f = magnitude(rng, -20, 4);
        if go_mul_f64(Arch::Amd64, a, f) != go_mul_f64(Arch::Arm64, a, f) {
            v.push((a, f));
            found += 1;
        }
    }
    let mut found = 0;
    let mut tries = 0;
    while found < 10 * scale && tries < 1_000_000 * scale {
        tries += 1;
        let a = 1 + (rng.next_u64() >> (rng.below(62) + 2)) as i64;
        let m = rng.below(63) as i32;
        let f = ulps((2f64.powi(m) - 0.5) / a as f64, rng.below(129) as i64 - 64);
        let a = if rng.below(2) == 0 { -a } else { a };
        if in_range_difference(go_mul_f64(Arch::Amd64, a, f), go_mul_f64(Arch::Arm64, a, f)) {
            v.push((a, f));
            found += 1;
        }
    }
    v
}

/// The four networks' `PrivateKeyID` (the oracle's `consts` checks them
/// against chaincfg).
pub const NETS: [(&str, [u8; 2]); 4] = [
    ("mainnet", [0x22, 0xde]),
    ("testnet3", [0x23, 0x0e]),
    ("simnet", [0x23, 0x07]),
    ("regnet", [0x22, 0xfe]),
];

/// The secp256k1 group order, big-endian.
pub const SECP_N: &str = "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141";
/// The Ed25519 group order, big-endian.
pub const ED_N: &str = "1000000000000000000000000000000014def9dea2f79cd65812631a5cf5d3ed";

/// `x + d` for a 32-byte big-endian number (wrapping).
fn add_be(x: &str, d: i64) -> String {
    let mut b = unhex(x);
    let mut carry = d;
    for byte in b.iter_mut().rev() {
        let s = i64::from(*byte) + carry;
        *byte = s.rem_euclid(256) as u8;
        carry = s.div_euclid(256);
    }
    hex(&b)
}

/// Private keys at the edges of both groups.
pub fn edge_keys() -> Vec<String> {
    let mut v = vec![
        "00".repeat(32),
        format!("{}01", "00".repeat(31)),
        "ff".repeat(32),
        format!("7f{}", "ff".repeat(31)),
        format!("80{}", "00".repeat(31)),
        "0c28fca386c7a227600b2fe50b7cae11ec86d3bf1fbe471be89827e19d72aa1d".to_owned(),
        "dda35a1488fb97b6eb3fe6e9ef2a25814e396fb5dc295fe994b96789b21a0398".to_owned(),
        "0ca35a1488fb97b6eb3fe6e9ef2a25814e396fb5dc295fe994b96789b21a0398".to_owned(),
    ];
    for n in [SECP_N, ED_N] {
        for d in -2..=2 {
            v.push(add_be(n, d));
        }
    }
    v
}

/// Cases for `wif.new`: edge and random keys of every length from 0 to 40
/// bytes, under the three schemes and unsupported ones, on the four
/// networks and random ones.
pub fn wif_new_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<Value> {
    let schemes = [
        0i64,
        1,
        2,
        0,
        1,
        2,
        -1,
        3,
        255,
        256,
        99_999,
        i64::MAX,
        i64::MIN,
    ];
    let mut v = Vec::new();
    let net = |rng: &mut SplitMix64| {
        if rng.below(5) == 0 {
            rng.bytes(2)
        } else {
            NETS[rng.below(4) as usize].1.to_vec()
        }
    };
    for key in edge_keys() {
        for scheme in [0, 1, 2] {
            let n = net(rng);
            v.push(json!({ "priv": key, "net": hex(&n), "scheme": scheme }));
        }
    }
    for len in 0..=40usize {
        for &scheme in &schemes {
            let n = net(rng);
            v.push(json!({ "priv": hex(&rng.bytes(len)), "net": hex(&n), "scheme": scheme }));
        }
    }
    for _ in 0..200 * scale {
        let mut key = rng.bytes(32);
        if rng.below(4) == 0 {
            // Ed25519 keys below the order, with leading zero bytes.
            key[0] &= 0x0f;
            for b in key.iter_mut().take(rng.below(4) as usize) {
                *b = 0;
            }
        }
        let n = net(rng);
        v.push(json!({ "priv": hex(&key), "net": hex(&n), "scheme": pick(rng, &schemes) }));
    }
    v
}

/// A 39-byte WIF payload with a correct checksum, base58-encoded.
pub fn wif_string(net: [u8; 2], scheme: u8, key: &[u8]) -> String {
    let mut p = Vec::new();
    p.extend_from_slice(&net);
    p.push(scheme);
    p.extend_from_slice(key);
    let cksum = dcroxide_chainhash::hash_b(&p);
    p.extend_from_slice(&cksum[..4]);
    dcroxide_base58::encode(&p)
}

/// Strings for `wif.decode`: valid WIFs of every scheme byte (unknown ones
/// included) on every network, decoded with the right and wrong networks;
/// edge keys; payloads of the wrong length; mutations; non-base58 and
/// non-ASCII characters; strings around the 54-byte limit.
pub fn wif_decode_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<Value> {
    let mut v = Vec::new();
    let mut valid = Vec::new();
    for key in edge_keys() {
        for scheme in [0u8, 1, 2, 3] {
            let net = NETS[rng.below(4) as usize].1;
            valid.push((wif_string(net, scheme, &unhex(&key)), net));
        }
    }
    for scheme in 0..=255u8 {
        let net = NETS[rng.below(4) as usize].1;
        valid.push((wif_string(net, scheme, &rng.bytes(32)), net));
    }
    for _ in 0..100 * scale {
        let net = if rng.below(5) == 0 {
            [rng.below(256) as u8, rng.below(256) as u8]
        } else {
            NETS[rng.below(4) as usize].1
        };
        let mut key = rng.bytes(32);
        if rng.below(3) == 0 {
            key[0] &= 0x0f;
        }
        valid.push((wif_string(net, rng.below(3) as u8, &key), net));
    }
    for (s, net) in &valid {
        v.push(json!({ "wif": s, "net": hex(net) }));
        let other = NETS[rng.below(4) as usize].1;
        v.push(json!({ "wif": s, "net": hex(&other) }));
    }
    // Wrong decoded lengths, with valid checksums over their own bytes.
    for len in 0..=60usize {
        let mut p = rng.bytes(len.saturating_sub(4));
        let cksum = dcroxide_chainhash::hash_b(&p);
        p.extend_from_slice(&cksum[..4.min(len)]);
        p.truncate(len);
        if len >= 2 {
            p[0] = 0x22;
            p[1] = 0xde;
        }
        v.push(json!({ "wif": dcroxide_base58::encode(&p), "net": "22de" }));
    }
    // Leading zero bytes, which base58 writes as leading '1's.
    for zeros in 0..6usize {
        let mut key = rng.bytes(32);
        for b in key.iter_mut().take(zeros) {
            *b = 0;
        }
        let s = wif_string([0, 0], 0, &key);
        v.push(json!({ "wif": s, "net": "0000" }));
        v.push(json!({ "wif": format!("1{s}"), "net": "0000" }));
    }
    // Mutations.
    let alphabet = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let odd = [
        "0", "O", "I", "l", " ", "+", "/", "\u{e9}", "\u{20ac}", "\u{0}",
    ];
    for round in 0..500 * scale {
        let (s, net) = &valid[round % valid.len()];
        let mut m: Vec<char> = s.chars().collect();
        match rng.below(6) {
            0 => {
                let i = rng.below(m.len() as u64) as usize;
                m[i] = char::from(alphabet[rng.below(58) as usize]);
            }
            1 => {
                m.remove(rng.below(m.len() as u64) as usize);
            }
            2 => {
                let i = rng.below(m.len() as u64 + 1) as usize;
                m.insert(i, char::from(alphabet[rng.below(58) as usize]));
            }
            3 => {
                let i = rng.below(m.len() as u64) as usize;
                let c = pick(rng, &odd);
                m.splice(i..=i, c.chars());
            }
            4 => {
                m.truncate(rng.below(m.len() as u64) as usize);
            }
            _ => {
                for _ in 0..rng.below(8) {
                    m.push(char::from(alphabet[rng.below(58) as usize]));
                }
            }
        }
        let s: String = m.into_iter().collect();
        v.push(json!({ "wif": s, "net": hex(net) }));
    }
    for n in [0usize, 1, 50, 51, 52, 53, 54, 55, 56, 80] {
        v.push(json!({ "wif": "z".repeat(n), "net": "22de" }));
        v.push(json!({ "wif": "1".repeat(n), "net": "22de" }));
    }
    v.push(json!({ "wif": "", "net": "22de" }));
    wif_length_limit_block(rng, &mut v);
    v
}

/// Valid WIFs of exactly 54 characters, `DecodeWIF`'s limit, and their
/// neighbours, appended to `v` whatever the corpus scale (so the frozen
/// corpus, at scale 0, pins the limit) and after every other draw.
///
/// The four networks' WIFs are 53 characters. A 39-byte payload needs 54
/// base58 digits once it reaches 58^53, which lies inside the payloads
/// starting `58 d4`: from `58 d5` up every payload is 54 digits, up to
/// `58 d3` every one is 53. So each network here from `58 d5` on gives 54
/// characters, decoded with its own network (accepted) and with one of the
/// four (wrong network), then with its last digit changed (checksum
/// mismatch, still 54 characters) and with a `1` in front (55 characters:
/// too long). A WIF can never be longer than 54 characters: 55 digits
/// without a leading `1` are at least 58^54 > 2^312, and each leading `1`
/// adds a zero byte, so a limit above 54 accepts nothing more.
fn wif_length_limit_block(rng: &mut SplitMix64, v: &mut Vec<Value>) {
    let alphabet = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let nets: [([u8; 2], usize); 6] = [
        ([0x58, 0xd3], 53),
        ([0x58, 0xd5], 54),
        ([0x59, 0x00], 54),
        ([0x5b, 0x00], 54),
        ([0x80, 0x00], 54),
        ([0xff, 0xff], 54),
    ];
    for (net, len) in nets {
        for scheme in [0u8, 1, 2] {
            let mut key = rng.bytes(32);
            if scheme == 1 {
                // Below the Ed25519 order, so the key is accepted.
                key[0] = (key[0] & 0x0f) | 0x01;
            }
            let s = wif_string(net, scheme, &key);
            assert_eq!(s.len(), len, "{s}: base58 length for network {net:02x?}");
            v.push(json!({ "wif": s, "net": hex(&net) }));
            let other = NETS[rng.below(4) as usize].1;
            v.push(json!({ "wif": s, "net": hex(&other) }));
            if len == 54 {
                let mut m = s.clone().into_bytes();
                let last = m.len() - 1;
                let at = alphabet.iter().position(|&c| c == m[last]).expect("base58");
                m[last] = alphabet[(at + 1) % alphabet.len()];
                let changed = String::from_utf8(m).expect("ascii");
                v.push(json!({ "wif": changed, "net": hex(&net) }));
                v.push(json!({ "wif": format!("1{s}"), "net": hex(&net) }));
            }
        }
    }
}

/// A random transaction with up to `max_in` inputs and `max_out` outputs,
/// drawn from few hashes, trees, indices, values and scripts so that ties
/// are common.
pub fn random_tx(rng: &mut SplitMix64, max_in: u64, max_out: u64) -> MsgTx {
    let mut tx = MsgTx {
        ser_type: TxSerializeType::Full,
        version: pick(rng, &[1u16, 1, 3, 0xffff]),
        lock_time: rng.next_u64() as u32,
        expiry: rng.next_u64() as u32,
        ..MsgTx::default()
    };
    let hashes: Vec<[u8; 32]> = (0..1 + rng.below(4))
        .map(|_| rng.bytes(32).try_into().expect("32"))
        .collect();
    for _ in 0..rng.below(max_in + 1) {
        // Few hashes, trees and indices, so ties are common.
        let mut hash = pick(rng, &hashes);
        if rng.below(4) == 0 {
            // Differs from a common hash only at one end.
            let i = if rng.below(2) == 0 { 0 } else { 31 };
            hash[i] = hash[i].wrapping_add(1);
        }
        tx.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: dcroxide_chainhash::Hash(hash),
                index: if rng.below(3) == 0 {
                    rng.next_u64() as u32
                } else {
                    rng.below(3) as u32
                },
                tree: pick(rng, &[0i8, 0, 1, 1, 2, 127]),
            },
            sequence: rng.next_u64() as u32,
            value_in: rng.next_u64() as i64,
            block_height: rng.next_u64() as u32,
            block_index: rng.next_u64() as u32,
            signature_script: {
                let n = rng.below(8) as usize;
                rng.bytes(n)
            },
        });
    }
    let scripts: Vec<Vec<u8>> = (0..1 + rng.below(3))
        .map(|_| {
            let n = rng.below(6) as usize;
            rng.bytes(n)
        })
        .collect();
    for _ in 0..rng.below(max_out + 1) {
        let mut script = pick_vec(rng, &scripts);
        if rng.below(4) == 0 {
            script.push(rng.below(256) as u8);
        }
        tx.tx_out.push(TxOut {
            value: {
                let r = rng.next_u64() as i64;
                pick(rng, &[0i64, 1, -1, 5, i64::MAX, i64::MIN, r])
            },
            version: pick(rng, &[0u16, 0, 1, 0xffff]),
            pk_script: script,
        });
    }
    tx
}

fn pick_vec(rng: &mut SplitMix64, items: &[Vec<u8>]) -> Vec<u8> {
    items[rng.below(items.len() as u64) as usize].clone()
}

/// The upstream testdata transactions (`txsort/testdata`), as hex.
pub fn upstream_txs() -> Vec<(String, String)> {
    let dir = data_dir().join("txsort");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("tests/data/txsort")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".hex"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let text = std::fs::read_to_string(dir.join(&n)).expect("read testdata");
            (n, text.trim().to_owned())
        })
        .collect()
}

/// Serialized transactions for `txsort.hex`: the upstream testdata and
/// `count` random transactions full of ties, with up to `max_io` inputs and
/// outputs.
pub fn txsort_hex_corpus(rng: &mut SplitMix64, count: usize, max_io: u64) -> Vec<String> {
    let mut v: Vec<String> = upstream_txs().into_iter().map(|(_, h)| h).collect();
    for _ in 0..count {
        v.push(hex(&random_tx(rng, max_io, max_io).serialize()));
    }
    v
}

/// The adversary of Go's own `TestAdversary` (`sort/sort_test.go`),
/// McIlroy's ("A killer adversary for quicksort", 1999) with its pivot guess
/// kept as a position, run against the crate's port of `sort.Sort`: an
/// input of `n` keys that drives the sort into its heapsort fallback (the
/// crate's unit tests check that it gets there). Its first guess is
/// position n/4 rather than Go's 0, so that the first pivot choice does not
/// look increasing and the partial insertion sort does not defuse it. If the port is Go's
/// algorithm, Go follows the same path on it, and the differential test
/// compares where both leave every element.
pub fn adversary_keys(n: usize) -> Vec<u32> {
    let gas = n.saturating_sub(1) as u32;
    let mut s: Vec<(Cell<u32>, u32)> = (0..n).map(|i| (Cell::new(gas), i as u32)).collect();
    // The comparison needs positions, as Go's Less(i, j) has them: an
    // element's address within the slice gives its position.
    let base = s.as_ptr() as usize;
    let size = std::mem::size_of::<(Cell<u32>, u32)>();
    let pos = |e: &(Cell<u32>, u32)| (e as *const (Cell<u32>, u32) as usize - base) / size;
    let state = Cell::new((0u32, n / 4));
    gosort::sort_by(&mut s, |x, y| {
        let (mut nsolid, mut candidate) = state.get();
        if x.0.get() == gas && y.0.get() == gas {
            let freeze = if pos(x) == candidate { x } else { y };
            freeze.0.set(nsolid);
            nsolid += 1;
        }
        if x.0.get() == gas {
            candidate = pos(x);
        } else if y.0.get() == gas {
            candidate = pos(y);
        }
        state.set((nsolid, candidate));
        x.0.get() < y.0.get()
    });
    let mut keys = vec![0u32; n];
    for (v, orig) in &s {
        keys[*orig as usize] = v.get();
    }
    keys
}

/// A transaction whose inputs all spend the same hash and tree, with
/// output indices `keys` and the input's position as its sequence number,
/// so the order `sort.Sort` leaves equal keys in is visible.
pub fn tagged_tx(keys: &[u32]) -> MsgTx {
    let mut tx = MsgTx::default();
    for (pos, &k) in keys.iter().enumerate() {
        tx.tx_in.push(TxIn {
            previous_out_point: OutPoint {
                hash: dcroxide_chainhash::Hash([7; 32]),
                index: k,
                tree: 0,
            },
            sequence: pos as u32,
            ..TxIn::default()
        });
    }
    tx
}

/// Key sequences that exercise every path of `sort.Sort`: sorted,
/// reversed, organ pipes, sawtooth, few distinct values, all equal, nearly
/// sorted with a few swaps (the partial insertion sort), random, and
/// McIlroy's adversary (the heapsort fallback), at lengths around the
/// thresholds 12, 50 and beyond.
pub fn sort_key_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<Vec<u32>> {
    let mut v = Vec::new();
    let lens = [
        0usize, 1, 2, 7, 8, 12, 13, 20, 49, 50, 51, 64, 100, 257, 1000,
    ];
    for &n in &lens {
        let n32 = n as u32;
        v.push((0..n32).collect());
        v.push((0..n32).rev().collect());
        v.push((0..n32).map(|i| i.min(n32 - i)).collect());
        v.push((0..n32).map(|i| i % 7).collect());
        v.push((0..n32).map(|_| rng.below(3) as u32).collect());
        v.push(vec![5; n]);
        let mut nearly: Vec<u32> = (0..n32).collect();
        for _ in 0..1 + rng.below(6) {
            if n > 1 {
                let i = rng.below(n as u64 - 1) as usize;
                nearly.swap(i, i + 1);
            }
        }
        v.push(nearly);
        let mut nearly_rev: Vec<u32> = (0..n32).rev().collect();
        if n > 3 {
            nearly_rev.swap(1, n - 2);
        }
        v.push(nearly_rev);
        v.push(adversary_keys(n));
    }
    for _ in 0..30 * scale {
        let n = rng.below(300) as usize;
        let distinct = 1 + rng.below(n as u64 + 1);
        v.push((0..n).map(|_| rng.below(distinct) as u32).collect());
    }
    shifting_threshold_keys(&mut v);
    v
}

/// `partialInsertionSort`'s `shortestShifting` (50) from both sides,
/// appended whatever the scale and without drawing from the PRNG: at 49
/// keys Go does not shift and goes on to partition; at 50 it shifts the
/// out-of-order pair into place. Only ties make the two paths' results
/// differ, so the keys are `i / 2` (every key twice) with one adjacent
/// pair of unequal keys swapped, every such pair at 49 and at 50 keys.
/// Whether a given swap reaches `partialInsertionSort` depends on the
/// pivot sample, so all of them are kept: with the threshold at 49 or at
/// 51 some of them come out in a different order.
fn shifting_threshold_keys(v: &mut Vec<Vec<u32>>) {
    for n in [49u32, 50] {
        let keys: Vec<u32> = (0..n).map(|i| i / 2).collect();
        for p in 0..keys.len() - 1 {
            if keys[p] != keys[p + 1] {
                let mut k = keys.clone();
                k.swap(p, p + 1);
                v.push(k);
            }
        }
    }
}

/// Transactions in field form for `txsort.fields`: random ones with
/// negative trees and ties, and tagged ones for every sort key sequence.
pub fn txsort_fields_corpus(rng: &mut SplitMix64, scale: usize) -> Vec<Value> {
    let mut v = random_fields_txs(rng, 50 * scale, 30);
    for keys in sort_key_corpus(rng, scale) {
        v.push(tx_fields(&tagged_tx(&keys)));
    }
    v
}

/// `count` random transactions in field form, with negative trees among
/// their inputs.
pub fn random_fields_txs(rng: &mut SplitMix64, count: usize, max_io: u64) -> Vec<Value> {
    (0..count)
        .map(|_| {
            let mut tx = random_tx(rng, max_io, max_io);
            for i in tx.tx_in.iter_mut() {
                if rng.below(4) == 0 {
                    i.previous_out_point.tree = pick(rng, &[-1i8, -2, i8::MIN, i8::MAX]);
                }
            }
            tx_fields(&tx)
        })
        .collect()
}

/// The compact frozen form of a `txsort.fields` answer for a
/// [`tagged_tx`]: whether it was sorted, and the input positions (the
/// sequence numbers) in the order the sort left them. Every other field of
/// a tagged transaction is fixed, so this is the whole answer.
pub fn tagged_answer(fields_answer: &Value) -> Value {
    let order: Vec<Value> = fields_answer["inputs"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(|i| i["sequence"].clone())
        .collect();
    json!({ "is_sorted": fields_answer["is_sorted"], "order": order })
}

// ---------------------------------------------------------------------
// Frozen vector files.
// ---------------------------------------------------------------------

/// The frozen vector files.
pub const AMOUNT_FILE: &str = "amount_vectors.txt";
pub const WIF_FILE: &str = "wif_vectors.txt";
pub const TXSORT_FILE: &str = "txsort_vectors.txt";

/// The fixed PRNG seed each file's corpus is generated from.
pub const AMOUNT_SEED: u64 = 0x6463_7275_7469_6c01;
pub const WIF_SEED: u64 = 0x6463_7275_7469_6c02;
pub const TXSORT_SEED: u64 = 0x6463_7275_7469_6c03;

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

/// The answer the Rust crate gives to one frozen entry.
pub fn rust_answer(cmd: &str, input: &Value) -> Value {
    match cmd {
        "amount.new" => rust_new_amount(hex_bits(input.as_str().expect("bits"))),
        "amount.mulf64" => rust_mul_f64(
            input[0].as_i64().expect("atoms"),
            hex_bits(input[1].as_str().expect("bits")),
        ),
        "amount.coin" => rust_coin(input.as_i64().expect("atoms")),
        "amount.format" => rust_format(
            input[0].as_i64().expect("atoms"),
            input[1].as_i64().expect("unit"),
        ),
        "amount.unit" => rust_unit(input.as_i64().expect("unit")),
        "wif.new" => rust_wif_new(input),
        "wif.decode" => rust_wif_decode(input),
        "txsort.hex" => rust_txsort_hex(input.as_str().expect("tx hex")),
        "txsort.fields" => rust_txsort_fields(input),
        "txsort.tagged" => {
            let keys: Vec<u32> = input
                .as_array()
                .expect("keys")
                .iter()
                .map(|k| u32::try_from(k.as_u64().expect("key")).expect("u32"))
                .collect();
            tagged_answer(&rust_txsort_fields(&tx_fields(&tagged_tx(&keys))))
        }
        other => panic!("unknown frozen command {other}"),
    }
}

/// Splits `items` into oracle batches of at most `n`.
pub fn batches<T>(items: &[T], n: usize) -> impl Iterator<Item = &[T]> {
    items.chunks(n.max(1))
}
