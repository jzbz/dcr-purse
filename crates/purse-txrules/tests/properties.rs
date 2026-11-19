// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests needing no Go: what the size estimates, the fee, the dust
//! rule, the output check and the high-fee guard must satisfy by their
//! definitions (spec 10 §4.1 to §4.3), over random inputs, and that no
//! input makes any function panic.
//!
//! `PURSE_TXRULES_CASES` multiplies the number of random instances (default
//! 1); `PURSE_TEST_SEED` picks the seed.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: arithmetic on bounded test values"
)]

mod common;

use common::{random_i64, scripts};
use dcroxide_wire::{MsgTx, TxIn, TxOut, TxSerializeType};
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_testutil::SplitMix64;
use purse_txrules::*;

fn cases(base: usize) -> usize {
    let k: usize = std::env::var("PURSE_TXRULES_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    base * k.max(1)
}

fn small_script_size(rng: &mut SplitMix64) -> i64 {
    match rng.below(8) {
        0 => [0xfc, 0xfd, 0xfe, 0x1_0000][rng.below(4) as usize],
        _ => rng.below(300) as i64,
    }
}

/// With exact signature-script sizes, the estimate is the serialized size
/// of the transaction it describes: every input carries a script of its
/// size, every output is given, and a change output is added when its size
/// is positive.
#[test]
fn estimate_equals_the_serialized_size() {
    let mut rng = SplitMix64::from_env("purse-txrules estimate");
    for _ in 0..cases(400) {
        let n_in = if rng.below(30) == 0 {
            0xfd
        } else {
            rng.below(5) as usize
        };
        let n_out = if rng.below(30) == 0 {
            0xfc
        } else {
            rng.below(5) as usize
        };
        let script_sizes: Vec<i64> = (0..n_in).map(|_| small_script_size(&mut rng)).collect();
        let outs: Vec<TxOut> = (0..n_out)
            .map(|_| TxOut {
                value: rng.next_u64() as i64,
                version: 0,
                pk_script: vec![0; small_script_size(&mut rng) as usize],
            })
            .collect();
        let change = if rng.below(2) == 0 {
            0
        } else {
            small_script_size(&mut rng)
        };

        let mut tx = MsgTx {
            ser_type: TxSerializeType::Full,
            tx_in: script_sizes
                .iter()
                .map(|&s| TxIn {
                    signature_script: vec![0; s as usize],
                    ..TxIn::default()
                })
                .collect(),
            tx_out: outs.clone(),
            ..MsgTx::default()
        };
        if change > 0 {
            tx.tx_out.push(TxOut {
                pk_script: vec![0; change as usize],
                ..TxOut::default()
            });
        }
        let estimate = estimate_serialize_size(&script_sizes, &outs, change);
        assert_eq!(estimate, tx.serialize_size() as i64);

        let lens: Vec<i64> = outs.iter().map(|o| o.pk_script.len() as i64).collect();
        assert_eq!(
            estimate_serialize_size_from_script_sizes(&script_sizes, &lens, change),
            estimate
        );
    }
}

/// The two estimators agree for any int64 sizes, overflow included.
#[test]
fn estimators_agree_on_any_sizes() {
    let mut rng = SplitMix64::from_env("purse-txrules estimators");
    for _ in 0..cases(2000) {
        let ins: Vec<i64> = (0..rng.below(4)).map(|_| random_i64(&mut rng)).collect();
        let change = random_i64(&mut rng);
        let lens: Vec<i64> = (0..rng.below(4))
            .map(|_| rng.below(70_000) as i64)
            .collect();
        let outs: Vec<TxOut> = lens
            .iter()
            .map(|&l| TxOut {
                pk_script: vec![0; l as usize],
                ..TxOut::default()
            })
            .collect();
        assert_eq!(
            estimate_serialize_size(&ins, &outs, change),
            estimate_serialize_size_from_script_sizes(&ins, &lens, change)
        );
    }
}

/// For non-negative rates and sizes whose product does not overflow, the
/// fee is the truncated product, lifted to the rate when it is 0 and the
/// rate positive, and clamped to MaxAmount. (It is not monotone in the
/// size: the lift makes a small transaction pay the whole per-kB rate.)
#[test]
fn fee_by_definition() {
    let mut rng = SplitMix64::from_env("purse-txrules fee");
    for _ in 0..cases(20_000) {
        let rate = (rng.next_u64() >> rng.below(64)) as i64 & i64::MAX;
        let size = (rng.next_u64() >> rng.below(64)) as i64 & i64::MAX;
        let fee = fee_for_serialize_size(Amount(rate), size).0;
        assert!((0..=MAX_AMOUNT).contains(&fee), "{rate} {size} -> {fee}");
        if let Some(product) = rate.checked_mul(size) {
            let mut want = product / 1000;
            if want == 0 && rate > 0 {
                want = rate;
            }
            assert_eq!(fee, want.min(MAX_AMOUNT), "{rate} {size}");
        }
    }
}

/// For a fixed script size and a positive rate, dust is exactly the amounts
/// below `ceil(rate * 3 * cost / 1000)`, as long as nothing overflows; for
/// P2PKH that is `ceil(rate * 603 / 1000)`.
#[test]
fn dust_threshold_by_definition() {
    let mut rng = SplitMix64::from_env("purse-txrules dust");
    for _ in 0..cases(5000) {
        let size = small_script_size(&mut rng);
        let rate = 1 + rng.below(10_000_000) as i64;
        let varint = match size {
            0..0xfd => 1,
            0xfd..=0xffff => 3,
            _ => 5,
        };
        let cost = 3 * (175 + varint + size);
        let smallest = (rate * cost + 999) / 1000;
        assert!(is_dust_amount(Amount(smallest - 1), size, Amount(rate)));
        assert!(!is_dust_amount(Amount(smallest), size, Amount(rate)));
        if size == 25 {
            assert_eq!(smallest, (rate * 603 + 999) / 1000);
        }
        // Monotone in the amount.
        let a = rng.below(1 << 50) as i64;
        if is_dust_amount(Amount(a), size, Amount(rate)) {
            assert!(is_dust_amount(Amount(a / 2), size, Amount(rate)));
        }
    }
}

/// `check_output` accepts exactly the values in `[0, MaxAmount]` that are
/// not dust, and reports the first failing rule.
#[test]
fn check_output_follows_its_rules() {
    let mut rng = SplitMix64::from_env("purse-txrules check_output");
    let scripts = scripts(&mut rng, 50);
    for _ in 0..cases(5000) {
        let out = TxOut {
            value: random_i64(&mut rng),
            version: [0, 0, 1][rng.below(3) as usize],
            pk_script: scripts[rng.below(scripts.len() as u64) as usize].clone(),
        };
        let rate = Amount(random_i64(&mut rng));
        let got = check_output(&out, rate);
        let kind = got.as_ref().err().map(|e| e.kind);
        if out.value < 0 || out.value > MAX_AMOUNT {
            assert_eq!(kind, Some(purse_errors::Kind::Invalid));
        } else if is_dust_output(&out, rate) {
            assert_eq!(kind, Some(purse_errors::Kind::Policy));
        } else {
            assert!(got.is_ok());
        }
    }
}

/// The high-fee guard is `fee > fee_for_serialize_size(10,000,000, size)`
/// for a positive fee and false otherwise, and `tx_pays_high_fees` is it
/// over the summed input values unless an input is null.
#[test]
fn high_fees_by_definition() {
    let mut rng = SplitMix64::from_env("purse-txrules highfees");
    for _ in 0..cases(3000) {
        let tx = MsgTx {
            ser_type: [
                TxSerializeType::Full,
                TxSerializeType::NoWitness,
                TxSerializeType::OnlyWitness,
            ][rng.below(3) as usize],
            tx_in: (0..rng.below(4))
                .map(|_| TxIn {
                    value_in: if rng.below(10) == 0 {
                        -1
                    } else {
                        rng.below(1 << 40) as i64
                    },
                    signature_script: vec![0; rng.below(120) as usize],
                    ..TxIn::default()
                })
                .collect(),
            tx_out: (0..rng.below(4))
                .map(|_| TxOut {
                    value: rng.below(1 << 40) as i64,
                    version: 0,
                    pk_script: vec![0; 25],
                })
                .collect(),
            ..MsgTx::default()
        };
        let outputs: i64 = tx.tx_out.iter().map(|o| o.value).sum();
        let total = rng.below(1 << 42) as i64;
        let fee = total - outputs;
        let max = fee_for_serialize_size(Amount(10_000_000), tx.serialize_size() as i64).0;
        assert_eq!(pays_high_fees(Amount(total), &tx), fee > 0 && fee > max);
        let first_null = tx.tx_in.iter().position(|i| i.value_in < 0);
        match (tx_pays_high_fees(&tx), first_null) {
            (Err(e), Some(i)) => assert_eq!(
                e.render(":: "),
                format!(
                    "invalid operation: transaction input {i} does not specify the input value"
                )
            ),
            (Ok(high), None) => {
                let inputs: i64 = tx.tx_in.iter().map(|i| i.value_in).sum();
                assert_eq!(high, pays_high_fees(Amount(inputs), &tx));
            }
            (got, want) => panic!("{got:?} with first null input {want:?}"),
        }
    }
}

/// No input makes any function panic, the VSP fee included (it returns an
/// error where Go panics).
#[test]
fn total_on_any_input() {
    let mut rng = SplitMix64::from_env("purse-txrules total");
    let params = [
        dcroxide_chaincfg::mainnet_params(),
        dcroxide_chaincfg::testnet3_params(),
        dcroxide_chaincfg::simnet_params(),
        dcroxide_chaincfg::regnet_params(),
    ];
    let scripts = scripts(&mut rng, 50);
    for i in 0..cases(20_000) {
        let a = random_i64(&mut rng);
        let b = random_i64(&mut rng);
        let _ = estimate_input_size(a);
        let _ = estimate_output_size(a);
        let _ = estimate_serialize_size_from_script_sizes(&[a, b], &[b, a], a ^ b);
        let _ = fee_for_serialize_size(Amount(a), b);
        let _ = is_dust_amount(Amount(a), b, Amount(a.wrapping_mul(b)));
        let out = TxOut {
            value: b,
            version: rng.below(3) as u16,
            pk_script: scripts[rng.below(scripts.len() as u64) as usize].clone(),
        };
        let _ = is_dust_output(&out, Amount(a));
        let _ = check_output(&out, Amount(a));
        let f = f64::from_bits(rng.next_u64());
        let _ = valid_pool_fee_rate(f);
        let height = rng.next_u64() as u32 as i32;
        let _ = stake_pool_ticket_fee(
            Amount(a),
            Amount(b),
            height,
            f,
            &params[i % params.len()],
            rng.below(2) == 0,
            rng.below(2) == 0,
        );
    }
}
