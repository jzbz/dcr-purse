// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the oracle and frozen-vector tests: the crate's answers in the
//! JSON shapes of `tools/oracle/cmd/txrules`, the input corpora, and the
//! frozen file format.
//!
//! Two answers are mapped rather than computed, because the crate
//! deliberately does not panic where Go does (docs/PARITY.md, QK-0741 and
//! QK-0742): `dust.amount` at the zero-cost script size, where the
//! crate answers `true`, and `poolfee.ticket` with a zero denominator,
//! where it returns an error. In both places [`rust_answer`] first checks
//! the crate's own answer and then reports Go's panic text, so a frozen or
//! live comparison checks that the crate diverges exactly at Go's panics
//! and nowhere else.

#![allow(dead_code)]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

use std::path::PathBuf;

use dcroxide_chaincfg::Params;
use dcroxide_txscript::stdscript::ScriptType;
use dcroxide_wire::{MsgTx, TxIn, TxOut, TxSerializeType, var_int_serialize_size};
use num_bigint::{BigInt, Sign};
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, unhex};
use purse_txrules::*;
use sha2::{Digest, Sha256};

/// Go's text for an integer division by zero (a `runtime.Error`).
pub const GO_INTEGER_DIVIDE_BY_ZERO: &str = "runtime error: integer divide by zero";
/// Go's text for a `big.Int` division by zero (`math/big`'s `nat.div`).
pub const GO_BIG_DIVISION_BY_ZERO: &str = "division by zero";

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

/// Every `ScriptType`, indexed by its value (dcrd's `ST*` constants).
pub const SCRIPT_TYPES: [ScriptType; 21] = {
    use ScriptType::*;
    [
        NonStandard,
        PubKeyEcdsaSecp256k1,
        PubKeyEd25519,
        PubKeySchnorrSecp256k1,
        PubKeyHashEcdsaSecp256k1,
        PubKeyHashEd25519,
        PubKeyHashSchnorrSecp256k1,
        ScriptHash,
        MultiSig,
        NullData,
        StakeSubmissionPubKeyHash,
        StakeSubmissionScriptHash,
        StakeGenPubKeyHash,
        StakeGenScriptHash,
        StakeRevocationPubKeyHash,
        StakeRevocationScriptHash,
        StakeChangePubKeyHash,
        StakeChangeScriptHash,
        TreasuryAdd,
        TreasuryGenPubKeyHash,
        TreasuryGenScriptHash,
    ]
};

// ---------------------------------------------------------------------
// JSON shapes.
// ---------------------------------------------------------------------

pub fn bits_hex(f: f64) -> String {
    format!("{:016x}", f.to_bits())
}

pub fn hex_bits(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).expect("float bits"))
}

fn i64s(v: &Value) -> Vec<i64> {
    v.as_array()
        .expect("an array")
        .iter()
        .map(|x| x.as_i64().expect("an int64"))
        .collect()
}

fn int(v: &Value) -> i64 {
    v.as_i64().expect("an int64")
}

pub fn output_json(out: &TxOut) -> Value {
    json!({ "value": out.value, "version": out.version, "script": hex(&out.pk_script) })
}

pub fn output_from_json(v: &Value) -> TxOut {
    TxOut {
        value: int(&v["value"]),
        version: u16::try_from(v["version"].as_u64().expect("version")).expect("u16"),
        pk_script: unhex(v["script"].as_str().expect("script")),
    }
}

/// A transaction of the oracle's shape: `ser`, `ins` as `[value_in,
/// signature script length]`, `outs` as `[value, pkScript length]`.
pub fn tx_from_shape(v: &Value) -> MsgTx {
    let ser_type = match v["ser"].as_u64().expect("ser") {
        0 => TxSerializeType::Full,
        1 => TxSerializeType::NoWitness,
        2 => TxSerializeType::OnlyWitness,
        other => panic!("serialization type {other}"),
    };
    let pair = |x: &Value| {
        let p = i64s(x);
        (p[0], usize::try_from(p[1]).expect("length"))
    };
    MsgTx {
        ser_type,
        tx_in: v["ins"]
            .as_array()
            .expect("ins")
            .iter()
            .map(|x| {
                let (value_in, len) = pair(x);
                TxIn {
                    value_in,
                    signature_script: vec![0; len],
                    ..TxIn::default()
                }
            })
            .collect(),
        tx_out: v["outs"]
            .as_array()
            .expect("outs")
            .iter()
            .map(|x| {
                let (value, len) = pair(x);
                TxOut {
                    value,
                    version: 0,
                    pk_script: vec![0; len],
                }
            })
            .collect(),
        ..MsgTx::default()
    }
}

fn zero_outputs(lens: &[i64]) -> Vec<TxOut> {
    lens.iter()
        .map(|&l| TxOut {
            value: 0,
            version: 0,
            pk_script: vec![0; usize::try_from(l).expect("length")],
        })
        .collect()
}

fn error_json(err: &purse_errors::Error) -> Value {
    json!({ "error": err.render(":: "), "kind": err.kind.as_str(), "op": err.op.as_str() })
}

fn type_json(t: ScriptType) -> Value {
    json!({ "n": t as u8, "name": t.name() })
}

/// Whether Go's `IsDustAmount` divides by zero for `script_size`.
pub fn dust_divisor_is_zero(script_size: i64) -> bool {
    let varint = var_int_serialize_size(script_size as u64) as i64;
    175i64
        .wrapping_add(varint)
        .wrapping_add(script_size)
        .wrapping_mul(3)
        == 0
}

/// The crate's answer to one batch element of `cmd`, in the oracle's
/// shape. For `poolfee.ticket` the element is `[net, stakeDiff, relayFee,
/// height, poolFee bits, dcp0010, dcp0012]`.
pub fn rust_answer(cmd: &str, input: &Value) -> Value {
    match cmd {
        "size.estimate" => json!(estimate_serialize_size(
            &i64s(&input[0]),
            &zero_outputs(&i64s(&input[1])),
            int(&input[2])
        )),
        "size.scripts" => json!(estimate_serialize_size_from_script_sizes(
            &i64s(&input[0]),
            &i64s(&input[1]),
            int(&input[2])
        )),
        "size.input" => json!(estimate_input_size(int(input))),
        "size.output" => json!(estimate_output_size(int(input))),
        "fee" => json!(fee_for_serialize_size(Amount(int(&input[0])), int(&input[1])).0),
        "dust.amount" => {
            let (a, s, r) = (int(&input[0]), int(&input[1]), int(&input[2]));
            let dust = is_dust_amount(Amount(a), s, Amount(r));
            if dust_divisor_is_zero(s) {
                assert!(dust, "the zero-cost script size must be dust");
                json!({ "panic": GO_INTEGER_DIVIDE_BY_ZERO })
            } else {
                json!(dust)
            }
        }
        "dust.output" => json!(is_dust_output(
            &output_from_json(&input[0]),
            Amount(int(&input[1]))
        )),
        "check.output" => {
            match check_output(&output_from_json(&input[0]), Amount(int(&input[1]))) {
                Ok(()) => Value::Null,
                Err(e) => error_json(&e),
            }
        }
        "highfees" => json!(pays_high_fees(
            Amount(int(&input[0])),
            &tx_from_shape(&input[1])
        )),
        "highfees.tx" => match tx_pays_high_fees(&tx_from_shape(input)) {
            Ok(high) => json!({ "high": high }),
            Err(e) => error_json(&e),
        },
        "stake.subtype" => {
            let t = SCRIPT_TYPES[usize::try_from(int(input)).expect("type")];
            let (sub, ok) = stake_sub_script_type(t);
            json!([type_json(t), type_json(sub), ok])
        }
        "poolfee.valid" => json!(valid_pool_fee_rate(hex_bits(input.as_str().expect("bits")))),
        "poolfee.ticket" => {
            let p = params(input[0].as_str().expect("net"));
            match stake_pool_ticket_fee(
                Amount(int(&input[1])),
                Amount(int(&input[2])),
                i32::try_from(int(&input[3])).expect("int32 height"),
                hex_bits(input[4].as_str().expect("bits")),
                &p,
                input[5].as_bool().expect("dcp0010"),
                input[6].as_bool().expect("dcp0012"),
            ) {
                Ok(fee) => json!(fee.0),
                Err(e) => {
                    assert!(e.is_kind(purse_errors::Kind::Invalid), "{e}");
                    json!({ "panic": GO_BIG_DIVISION_BY_ZERO })
                }
            }
        }
        other => panic!("unknown command {other}"),
    }
}

// ---------------------------------------------------------------------
// Corpora.
// ---------------------------------------------------------------------

fn pick<T: Copy>(rng: &mut SplitMix64, items: &[T]) -> T {
    items[rng.below(items.len() as u64) as usize]
}

/// Script sizes: the varint boundaries and their neighbours, the sizes
/// dcrwallet uses, negatives (9-byte varint), the zero-cost size -184,
/// the extremes, and random values of every magnitude.
pub fn size_values(rng: &mut SplitMix64, random: usize) -> Vec<i64> {
    let mut v: Vec<i64> = vec![
        0,
        1,
        23,
        25,
        26,
        32,
        74,
        108,
        109,
        110,
        166,
        0xfb,
        0xfc,
        0xfd,
        0xfe,
        0xffff,
        0x1_0000,
        0xffff_ffff,
        0x1_0000_0000,
        i64::MAX,
        i64::MAX - 65,
        i64::MAX - 66,
        -1,
        -9,
        -183,
        -184,
        -185,
        -193,
        i64::MIN,
        i64::MIN + 1,
    ];
    for _ in 0..random {
        v.push(random_i64(rng));
    }
    v
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

/// Lengths a test may allocate a zero script of: small, around the 1- and
/// 3-byte varint boundaries, and occasionally around the 5-byte one.
fn script_len(rng: &mut SplitMix64) -> i64 {
    match rng.below(20) {
        0 => pick(rng, &[0xfc, 0xfd, 0xfe, 0xffff, 0x1_0000]),
        1..=3 => rng.below(600) as i64,
        _ => pick(rng, &[0, 1, 23, 25, 26, 32, 36, 40]),
    }
}

/// `size.estimate` cases: `[[input script sizes], [output script lengths],
/// change script size]`, with counts crossing the 253 boundary.
pub fn estimate_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let sizes = size_values(rng, 20);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let n_in = match i % 9 {
            0 => 0xfc + rng.below(3),
            1 => 0,
            _ => rng.below(6),
        } as usize;
        let n_out = match i % 11 {
            0 => 0xfb + rng.below(4),
            1 => 0,
            _ => rng.below(5),
        } as usize;
        let ins: Vec<i64> = (0..n_in)
            .map(|_| {
                if rng.below(5) == 0 {
                    pick(rng, &sizes)
                } else {
                    pick(rng, &[108, 110, 74, 109, 166])
                }
            })
            .collect();
        let outs: Vec<i64> = (0..n_out).map(|_| script_len(rng)).collect();
        let change = if rng.below(3) == 0 {
            pick(rng, &sizes)
        } else {
            pick(rng, &[0, 25, 26, 23, 36])
        };
        out.push(json!([ins, outs, change]));
    }
    out
}

/// `size.scripts` cases: as [`estimate_cases`] with any int64 as an output
/// size.
pub fn scripts_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let sizes = size_values(rng, 30);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let n_in = if i % 13 == 0 {
            0xfc + rng.below(3)
        } else {
            rng.below(6)
        } as usize;
        let n_out = if i % 17 == 0 {
            0xfb + rng.below(4)
        } else {
            rng.below(6)
        } as usize;
        let one = |rng: &mut SplitMix64| {
            if rng.below(3) == 0 {
                pick(rng, &sizes)
            } else {
                pick(rng, &[25, 23, 26, 32, 108, 110])
            }
        };
        let ins: Vec<i64> = (0..n_in).map(|_| one(rng)).collect();
        let outs: Vec<i64> = (0..n_out).map(|_| one(rng)).collect();
        let change = one(rng);
        out.push(json!([ins, outs, change]));
    }
    out
}

/// Relay fee rates: zero, the per-kB boundaries, the default and its
/// multiples, the maximum amount, negatives and the extremes.
pub fn rate_values(rng: &mut SplitMix64, random: usize) -> Vec<i64> {
    let mut v: Vec<i64> = vec![
        0,
        1,
        999,
        1000,
        1001,
        2550,
        10_000,
        100_000,
        10_000_000,
        MAX_AMOUNT,
        MAX_AMOUNT + 1,
        i64::MAX,
        i64::MAX / 1000,
        i64::MAX / 1000 + 1,
        -1,
        -999,
        -1000,
        -1001,
        -10_000,
        i64::MIN,
        i64::MIN + 1,
    ];
    for _ in 0..random {
        v.push(random_i64(rng));
    }
    v
}

/// `fee` cases `[rate, size]`: every pairing of the special rates and
/// sizes, then random pairs.
pub fn fee_cases(rng: &mut SplitMix64, random: usize) -> Vec<Value> {
    let rates = rate_values(rng, 0);
    let mut sizes = size_values(rng, 0);
    sizes.extend([
        181,
        217,
        253,
        419,
        999,
        1000,
        1001,
        100_000,
        922_337_203_685_478,
    ]);
    let mut out = Vec::new();
    for &r in &rates {
        for &s in &sizes {
            out.push(json!([r, s]));
        }
    }
    for _ in 0..random {
        let r = if rng.below(2) == 0 {
            pick(rng, &rates)
        } else {
            random_i64(rng)
        };
        let s = if rng.below(2) == 0 {
            pick(rng, &sizes)
        } else {
            random_i64(rng)
        };
        out.push(json!([r, s]));
    }
    out
}

/// The smallest non-dust amount for a script size and a positive rate in
/// the non-overflowing range, from the formula (used only to aim inputs at
/// the boundary; Go decides the answers).
fn dust_boundary(script_size: i64, rate: i64) -> i64 {
    let total = 175 + var_int_serialize_size(script_size as u64) as i64 + script_size;
    let cost = 3 * total;
    // amount * 1000 / cost >= rate  <=>  amount >= ceil(rate * cost / 1000)
    (rate * cost + 999) / 1000
}

/// `dust.amount` cases `[amount, script size, rate]`: amounts on either
/// side of the boundary for ordinary sizes and rates, the overflowing
/// amounts, every negative-size case including -184, then random triples.
pub fn dust_amount_cases(rng: &mut SplitMix64, random: usize) -> Vec<Value> {
    let rates = rate_values(rng, 0);
    let sizes = size_values(rng, 0);
    let mut out = Vec::new();
    for &s in &[0i64, 1, 23, 25, 26, 32, 0xfc, 0xfd, 0xffff, 0x1_0000] {
        for &r in &[1i64, 999, 1000, 2550, 10_000, 100_000, 1_000_000] {
            let b = dust_boundary(s, r);
            for d in -2..=2 {
                out.push(json!([b + d, s, r]));
            }
        }
    }
    let amounts = [
        0,
        1,
        -1,
        6029,
        6030,
        MAX_AMOUNT,
        i64::MAX / 1000,
        i64::MAX / 1000 + 1,
        i64::MAX,
        i64::MIN,
    ];
    for &s in &sizes {
        for &r in &[0, 10_000, -1, i64::MIN, i64::MAX] {
            out.push(json!([pick(rng, &amounts), s, r]));
        }
    }
    out.push(json!([0, -184, 0]));
    out.push(json!([i64::MAX, -184, i64::MIN]));
    for _ in 0..random {
        let a = if rng.below(3) == 0 {
            pick(rng, &amounts)
        } else {
            random_i64(rng)
        };
        let s = if rng.below(2) == 0 {
            pick(rng, &sizes)
        } else {
            random_i64(rng)
        };
        let r = if rng.below(2) == 0 {
            pick(rng, &rates)
        } else {
            random_i64(rng)
        };
        out.push(json!([a, s, r]));
    }
    out
}

/// Output scripts for the dust and output checks: standard shapes, the
/// null-data forms and their near misses, unparseable scripts, the size
/// limit, and random bytes.
pub fn scripts(rng: &mut SplitMix64, random: usize) -> Vec<Vec<u8>> {
    let mut p2pkh = vec![0x76, 0xa9, 0x14];
    p2pkh.extend(rng.bytes(20));
    p2pkh.extend([0x88, 0xac]);
    let mut p2sh = vec![0xa9, 0x14];
    p2sh.extend(rng.bytes(20));
    p2sh.push(0x87);
    let mut stake_p2pkh = vec![0xbd];
    stake_p2pkh.extend(&p2pkh);
    let push = |n: usize, rng: &mut SplitMix64| {
        let mut s = vec![0x6a];
        match n {
            0 => {}
            1..=75 => s.push(n as u8),
            76..=255 => s.extend([0x4c, n as u8]),
            _ => s.extend([0x4d, n as u8, (n >> 8) as u8]),
        }
        s.extend(rng.bytes(n));
        s
    };
    let mut v = vec![
        vec![],
        p2pkh.clone(),
        p2sh,
        stake_p2pkh,
        vec![0x6a],
        vec![0x6a, 0x00],
        vec![0x6a, 0x51],
        vec![0x6a, 0x4c],
        vec![0x6a, 0x4c, 0x01, 0x00],
        vec![0x6a, 0x01],
        vec![0x6a, 0x6a],
        vec![0x4c],
        vec![0x4d, 0x05],
        vec![0x01],
        vec![0x51],
        vec![0x00],
        vec![0xff],
        vec![0x6a; 3],
    ];
    for n in [1, 20, 75, 76, 80, 255, 256, 257, 300] {
        v.push(push(n, rng));
    }
    // dcrd's MaxScriptSize is 16384.
    v.push(vec![0x51; 16_384]);
    v.push(vec![0x51; 16_385]);
    v.push(vec![0x00; 0xfd]);
    for _ in 0..random {
        let len = pick(rng, &[0usize, 1, 2, 3, 20, 25, 40, 80]);
        v.push(rng.bytes(len));
    }
    v
}

/// `dust.output` and `check.output` cases `[output, rate]`.
pub fn output_cases(rng: &mut SplitMix64, random: usize) -> Vec<Value> {
    let scripts = scripts(rng, 40);
    let rates = rate_values(rng, 0);
    let values = [
        0,
        1,
        -1,
        6029,
        6030,
        6032,
        6033,
        603,
        MAX_AMOUNT,
        MAX_AMOUNT + 1,
        i64::MAX,
        i64::MIN,
    ];
    let mut out = Vec::new();
    for s in &scripts {
        // Every value and both versions for short scripts; the scripts at
        // the size limit are long, so a few values keep the frozen file
        // small.
        let (vals, versions): (&[i64], &[u16]) = if s.len() > 300 {
            (&[0, 1_000_000_000], &[0])
        } else {
            (&values, &[0, 1])
        };
        for &value in vals {
            for &version in versions {
                let o = TxOut {
                    value,
                    version,
                    pk_script: s.clone(),
                };
                out.push(json!([output_json(&o), 10_000]));
            }
        }
    }
    for _ in 0..random {
        let o = TxOut {
            value: if rng.below(2) == 0 {
                pick(rng, &values)
            } else {
                random_i64(rng)
            },
            version: pick(rng, &[0u16, 0, 0, 1, 0xffff]),
            pk_script: pick_script(rng, &scripts),
        };
        let r = if rng.below(2) == 0 {
            pick(rng, &rates)
        } else {
            random_i64(rng)
        };
        out.push(json!([output_json(&o), r]));
    }
    out
}

/// A random script of at most 300 bytes (the long ones are covered by the
/// exhaustive part of [`output_cases`]).
fn pick_script(rng: &mut SplitMix64, scripts: &[Vec<u8>]) -> Vec<u8> {
    loop {
        let s = &scripts[rng.below(scripts.len() as u64) as usize];
        if s.len() <= 300 {
            return s.clone();
        }
    }
}

/// A random transaction shape: up to 6 inputs and outputs (sometimes
/// enough to cross the 253 boundary), any serialization type.
fn random_shape(rng: &mut SplitMix64, negative_inputs: bool) -> Value {
    let n_in = if rng.below(40) == 0 {
        0xfd
    } else {
        rng.below(6)
    };
    let n_out = if rng.below(40) == 0 {
        0xfd
    } else {
        rng.below(6)
    };
    let ins: Vec<Value> = (0..n_in)
        .map(|_| {
            let v = if negative_inputs && rng.below(8) == 0 {
                let small = -(rng.below(1000) as i64) - 1;
                pick(rng, &[-1, i64::MIN, small])
            } else if rng.below(12) == 0 {
                // Zero, the boundary of TxPaysHighFees' null-value test
                // (`ValueIn < 0`).
                0
            } else if rng.below(10) == 0 {
                random_i64(rng).wrapping_abs()
            } else {
                rng.below(1_000_000_000) as i64
            };
            json!([v, pick(rng, &[0, 108, 110, 74, 0xfc, 0xfd])])
        })
        .collect();
    let outs: Vec<Value> = (0..n_out)
        .map(|_| {
            let v = if rng.below(10) == 0 {
                random_i64(rng)
            } else {
                rng.below(1_000_000_000) as i64
            };
            json!([v, pick(rng, &[0, 23, 25, 26, 0xfd])])
        })
        .collect();
    json!({ "ser": rng.below(3), "ins": ins, "outs": outs })
}

/// `highfees` cases `[totalInput, tx]`, with totals aimed at either side of
/// the threshold for each shape.
pub fn highfees_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let shape = random_shape(rng, false);
        let tx = tx_from_shape(&shape);
        let outputs = tx.tx_out.iter().fold(0i64, |s, o| s.wrapping_add(o.value));
        let max_fee = fee_for_serialize_size(Amount(10_000_000), tx.serialize_size() as i64).0;
        for total in [
            outputs.wrapping_add(max_fee),
            outputs.wrapping_add(max_fee).wrapping_add(1),
            outputs.wrapping_add(max_fee).wrapping_sub(1),
            outputs,
            outputs.wrapping_sub(1),
            random_i64(rng),
            i64::MIN,
            i64::MAX,
        ] {
            out.push(json!([total, shape]));
        }
    }
    out
}

/// `highfees.tx` cases: the boundary of the null-value test (an input of
/// value 0, accepted, and of -1, refused, each after a nonzero input), then
/// random shapes whose inputs sometimes carry the null value or 0.
pub fn highfees_tx_cases(rng: &mut SplitMix64, n: usize) -> Vec<Value> {
    let mut out = Vec::with_capacity(n + 4);
    for first in [0i64, 1_000_000] {
        for second in [0i64, -1] {
            out.push(json!({
                "ser": 0,
                "ins": [[first, 108], [second, 108], [2_000_000, 108]],
                "outs": [[2_500_000, 25]],
            }));
        }
    }
    out.extend((0..n).map(|_| random_shape(rng, true)));
    out
}

/// Percentages for the VSP fee: the upstream ones, values that lose a unit
/// in float64 (0.29, 1.15), the boundaries of `ValidPoolFeeRate`, signed
/// zeros, subnormals, NaN, the infinities, magnitudes beyond 2^63 / 100,
/// and random values.
pub fn pool_fee_values(rng: &mut SplitMix64, random: usize) -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        0.01,
        0.009_999_999_999_999_998,
        0.29,
        1.0,
        1.15,
        2.59,
        5.0,
        7.5,
        50.0,
        99.99,
        100.0,
        100.000_000_000_000_01,
        100.01,
        -0.01,
        -1.0,
        f64::MIN_POSITIVE,
        5e-324,
        f64::NAN,
        -f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        9.223_372_036_854_775e16,
        9.223_372_036_854_776e16,
        -9.223_372_036_854_776e16,
        -9.223_372_036_854_778e16,
        1e300,
        f64::MAX,
        f64::MIN,
    ];
    for _ in 0..random {
        let f = match rng.below(4) {
            0 => (rng.below(10_001) as f64) / 100.0,
            1 => (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 100.0,
            2 => f64::from_bits(rng.next_u64()),
            _ => (rng.below(1_000_000) as f64) / 7.0,
        };
        v.push(f);
    }
    v
}

/// Whether Go's `int64(math.Floor(f * 100.0))` depends on the
/// architecture: a NaN or a value outside `[-2^63, 2^63)`.
pub fn pool_fee_int_is_arch_dependent(f: f64) -> bool {
    let x = (f * 100.0).floor();
    !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x)
}

/// `poolfee.ticket` cases for one network, without the network:
/// `[stakeDiff, relayFee, height, poolFee bits, dcp0010, dcp0012]`.
pub fn ticket_cases(rng: &mut SplitMix64, net: &str, random: usize) -> Vec<Value> {
    let p = params(net);
    let svh = p.stake_validation_height;
    let interval = p.subsidy_reduction_interval;
    let fees = pool_fee_values(rng, 0);
    let mut heights: Vec<i64> = vec![
        i64::from(i32::MIN),
        -1,
        0,
        1,
        2,
        svh - 2,
        svh - 1,
        svh,
        svh + 1,
        25_000,
        50_000,
        interval - 1,
        interval,
        interval + 1,
        1_000_000,
        10_000_000,
        100_000_000,
        i64::from(i32::MAX),
    ];
    heights.retain(|h| i32::try_from(*h).is_ok());
    let diffs = [
        0i64,
        1,
        -1,
        2 * 100_000_000,
        10 * 100_000_000,
        15 * 100_000_000,
        200 * 100_000_000,
        MAX_AMOUNT,
        i64::MAX,
        i64::MIN,
    ];
    let relay = [0i64, 10_000, 1_000_000, 5_000_000, -1, i64::MAX, i64::MIN];
    let flags = [(false, false), (true, false), (false, true), (true, true)];
    let mut out = Vec::new();
    // Every height with ordinary prices, fees and each flag pair.
    for &h in &heights {
        for &(d10, d12) in &flags {
            out.push(json!([
                pick(rng, &diffs[2..7]),
                pick(rng, &relay[..4]),
                h,
                bits_hex(pick(rng, &[1.0, 2.59, 0.29, 5.0, 100.0])),
                d10,
                d12
            ]));
        }
    }
    // Every special percentage, price and fee term at one voting height.
    for &f in &fees {
        out.push(json!([
            15 * 100_000_000i64,
            5_000_000,
            svh + 100,
            bits_hex(f),
            true,
            true
        ]));
    }
    for &d in &diffs {
        for &z in &relay {
            out.push(json!([d, z, svh + 100, bits_hex(2.59), false, false]));
        }
    }
    // Zero denominators: no subsidy below stake validation and a zero
    // price, or a price cancelling the subsidy.
    out.push(json!([0, 0, 0, bits_hex(1.0), false, false]));
    out.push(json!([0, 1_000_000, svh - 3, bits_hex(50.0), true, true]));
    let subsidy = adjusted_subsidy(&p, svh + 100, false, false);
    out.push(json!([-subsidy, 0, svh + 100, bits_hex(1.0), false, false]));
    out.push(json!([
        -subsidy - 1,
        7,
        svh + 100,
        bits_hex(1.0),
        false,
        false
    ]));
    out.extend(shift_boundary_cases(rng, &p, svh + 100));
    if net == "mainnet" {
        // The three cases that first showed the negative divisor's
        // correction untested (Go answers 1 for each).
        for (d, z, bits) in [
            (
                -2305843009331151880i64,
                2305638378031244171i64,
                "3f8eb851eb851eb8",
            ),
            (
                -2305843009443834691,
                2305774799010528788,
                "3fa1eb851eb851ec",
            ),
            (
                -2305843009443834691,
                2305813776400989304,
                "3fb3333333333333",
            ),
        ] {
            out.push(json!([d, z, 306_144, bits, false, false]));
        }
    }
    for _ in 0..random {
        let d = match rng.below(4) {
            0 => pick(rng, &diffs),
            1 => random_i64(rng),
            _ => rng.below(500 * 100_000_000) as i64,
        };
        let z = match rng.below(4) {
            0 => pick(rng, &relay),
            1 => random_i64(rng),
            _ => rng.below(10_000_000) as i64,
        };
        let h = match rng.below(4) {
            0 => pick(rng, &heights),
            1 => i64::from(rng.next_u64() as u32 as i32),
            _ => rng.below(3_000_000) as i64,
        };
        let f = if rng.below(3) == 0 {
            pick(rng, &fees)
        } else {
            (rng.below(10_001) as f64) / 100.0
        };
        let (d10, d12) = pick(rng, &flags);
        out.push(json!([d, z, h, bits_hex(f), d10, d12]));
    }
    out
}

/// `a`'s inverse modulo `m` (both positive), if they are coprime.
fn mod_inverse(a: &BigInt, m: &BigInt) -> Option<BigInt> {
    // Extended Euclid on (a mod m, m).
    let (mut r0, mut r1) = (m.clone(), a % m);
    let (mut t0, mut t1) = (BigInt::from(0u8), BigInt::from(1u8));
    while r1.sign() != Sign::NoSign {
        let q = &r0 / &r1;
        (r0, r1) = (r1.clone(), &r0 - &q * &r1);
        (t0, t1) = (t1.clone(), &t0 - &q * &t1);
    }
    (r0 == BigInt::from(1u8)).then(|| ((t0 % m) + m) % m)
}

fn gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let (mut a, mut b) = (a.clone(), b.clone());
    while b.sign() != Sign::NoSign {
        (a, b) = (b.clone(), a % &b);
    }
    a
}

/// `poolfee.ticket` cases whose quotient `num·2^64 / den` lies within 2^-64
/// of an integer and is not one, so that Go's Euclidean `Div` and the
/// flooring `Rsh` decide the result (QK-0743): the crate's big-integer
/// path, with its correction of the truncated quotient, is then pinned by
/// Go. With `s` the adjusted vote subsidy at `height`, the original split:
///
/// - a negative divisor: percentage 1 (0.015), a price `v = -(D + s)` with
///   `D` near 2^62, and a fee term making `s·(v+z) = -(10000·D - r)` with
///   `0 < r·2^64 < 10000·D`, so the quotient is just below 2^64 and Go's
///   answer is 1;
/// - a positive divisor and a negative numerator beyond 2^127 (so off the
///   i128 fast path): a percentage near 2^41, `s + v = E` near 2^60, and a
///   fee term solving `p·s·(v+z) ≡ -r (mod 10000·E)` for such an `r`.
fn shift_boundary_cases(rng: &mut SplitMix64, p: &Params, height: i64) -> Vec<Value> {
    let s = adjusted_subsidy(p, height, false, false);
    let mut out = Vec::new();
    if s < 10_000 {
        return out;
    }
    let s128 = i128::from(s);
    let two64 = 1i128 << 64;
    // Negative divisor.
    assert_eq!((0.015f64 * 100.0).floor(), 1.0);
    let mut found = 0;
    let mut d = (1i128 << 62) + i128::from(rng.below(1 << 40));
    for _ in 0..50_000_000 {
        d += 1;
        let r = (10_000 * d) % s128;
        if r >= 1 && r.checked_mul(two64).is_some_and(|x| x < 10_000 * d) {
            let v = -(d + s128);
            let z = d + s128 - (10_000 * d - r) / s128;
            out.push(json!([
                i64::try_from(v).expect("price"),
                i64::try_from(z).expect("fee term"),
                height,
                bits_hex(0.015),
                false,
                false
            ]));
            found += 1;
            if found == 3 {
                break;
            }
            d += i128::from(rng.below(1 << 30));
        }
    }
    assert_eq!(found, 3, "negative-divisor boundary cases for s = {s}");
    // Positive divisor, negative numerator past i128.
    let mut found = 0;
    'search: for _ in 0..200 {
        // A percentage p with floor(f·100) == p for a float f.
        let mut pct = (1i64 << 41) + rng.below(1 << 40) as i64;
        let fee = loop {
            let f = pct as f64 / 100.0;
            if (f * 100.0).floor() == pct as f64 {
                break f;
            }
            pct += 1;
        };
        let e = (1i128 << 60) + i128::from(rng.below(1 << 59));
        let v = e - s128;
        let den = BigInt::from(e) * 10_000;
        let a = BigInt::from(pct) * s;
        let g = gcd(&a, &den);
        let m = &den / &g;
        let Some(inv) = mod_inverse(&(&a / &g), &m) else {
            continue;
        };
        let min_w = BigInt::from(i64::MIN) + v;
        let mut t = BigInt::from(1u8);
        // r = g·t with r·2^64 < den.
        while (&g * &t) << 64u32 < den {
            // w ≡ -t·inv (mod m), taken in [-m, 0).
            let w0 = ((-(&t * &inv)) % &m + &m) % &m;
            let w = w0 - &m;
            if w >= min_w {
                let num = &a * &w;
                let past_i128 = num.bits() > 127;
                let (Ok(w), true) = (i64::try_from(&w), past_i128) else {
                    t += 1u8;
                    continue;
                };
                let z = i128::from(w) - v;
                out.push(json!([
                    i64::try_from(v).expect("price"),
                    i64::try_from(z).expect("fee term"),
                    height,
                    bits_hex(fee),
                    false,
                    false
                ]));
                found += 1;
                if found == 3 {
                    break 'search;
                }
                continue 'search;
            }
            t += 1u8;
        }
    }
    assert_eq!(found, 3, "positive-divisor boundary cases for s = {s}");
    out
}

/// The vote subsidy after the fee's anticipated reductions, as the crate's
/// formula computes it (to aim a price at a zero denominator).
fn adjusted_subsidy(p: &Params, height: i64, d10: bool, d12: bool) -> i64 {
    use dcroxide_standalone::{SubsidyCache, SubsidySplitVariant};
    let variant = match (d10, d12) {
        (_, true) => SubsidySplitVariant::Dcp0012,
        (true, false) => SubsidySplitVariant::Dcp0010,
        _ => SubsidySplitVariant::Original,
    };
    let mut s = SubsidyCache::new(p).calc_stake_vote_subsidy_v3(height, variant);
    let adjs = (f64::from(p.ticket_pool_size) / p.subsidy_reduction_interval as f64).ceil() as i64;
    for _ in 0..adjs {
        s = s * 100 / 101;
    }
    s
}

// ---------------------------------------------------------------------
// Frozen vector files.
// ---------------------------------------------------------------------

/// The frozen vector file.
pub const VECTORS_FILE: &str = "txrules_vectors.txt";

/// The fixed PRNG seed its corpus is generated from.
pub const VECTORS_SEED: u64 = 0x7478_7275_6c65_7301;

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
