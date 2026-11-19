// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Property tests, no oracle: the authoring loop against its definition
//! (spec 10 §4.5), the randomization against a uniform transposition, and
//! the signing loop against dcroxide's script engine.
//!
//! `PURSE_TXAUTHOR_CASES` (default 1) multiplies the random instances;
//! `PURSE_TEST_SEED` varies or reproduces them.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and the definitions' own arithmetic on bounded values"
)]

mod common;

use common::*;
use dcroxide_crypto::rand::Prng;
use dcroxide_txscript::sign::SignatureType;
use dcroxide_txscript::stdaddr::{self, Address};
use dcroxide_txscript::{Engine, ScriptFlags};
use dcroxide_wire::{MsgTx, OutPoint, TxOut};
use purse_dcrutil::Amount;
use purse_errors::{AnyError, Kind};
use purse_testutil::SplitMix64;
use purse_testutil::serde_json::json;
use purse_txauthor::*;
use purse_txrules::txsizes::REDEEM_P2PKH_SIG_SCRIPT_SIZE;
use purse_txrules::{estimate_serialize_size, fee_for_serialize_size, is_dust_amount};

fn cases() -> usize {
    std::env::var("PURSE_TXAUTHOR_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
}

/// Sane-valued cases: nonnegative amounts and rates, real script sizes, a
/// size limit no transaction here reaches. The oracle differential covers
/// the rest.
fn sane_case(rng: &mut SplitMix64) -> purse_testutil::serde_json::Value {
    let n_out = rng.below(5) as usize;
    let outputs: Vec<_> = (0..n_out)
        .map(|_| {
            json!({
                "value": rng.below(200_000_000) as i64,
                "version": 0,
                "script": purse_testutil::hex(&vec![0; pick(rng, &[23usize, 25, 26, 36])]),
            })
        })
        .collect();
    let n_in = rng.below(8) as usize;
    let utxos: Vec<_> = (0..n_in)
        .map(|_| {
            json!([
                rng.below(150_000_000) as i64,
                pick(rng, &[REDEEM_P2PKH_SIG_SCRIPT_SIZE, 110, 74]),
                25
            ])
        })
        .collect();
    let size = pick(rng, &[25i64, 23, 26, 36]);
    json!({
        "outputs": outputs,
        "rate": pick(rng, &[0i64, 1, 1000, 2550, 10_000, 100_000]),
        "max": 1_000_000,
        "source": { "utxos": utxos, "mode": if rng.below(4) == 0 { "all" } else { "cumulative" } },
        "change": { "size": size, "script": purse_testutil::hex(&vec![7; size as usize]), "version": 0 },
    })
}

#[test]
fn authoring_follows_its_definition() {
    let mut rng = SplitMix64::from_env("purse-txauthor properties: authoring");
    let (mut with_change, mut without, mut short) = (0, 0, 0);
    for _ in 0..cases() * 20_000 {
        let case = sane_case(&mut rng);
        let outputs: Vec<TxOut> = case["outputs"]
            .as_array()
            .expect("outputs")
            .iter()
            .map(output_from_json)
            .collect();
        let rate = Amount(case["rate"].as_i64().expect("rate"));
        let change_size = case["change"]["size"].as_i64().expect("size");
        let mut source = ScriptedSource::new(&case["source"]);
        let mut change = ScriptedChange::new(&case["change"]);
        let result =
            new_unsigned_transaction(outputs.clone(), rate, &mut source, &mut change, 1_000_000);
        let target: i64 = outputs.iter().map(|o| o.value).sum();
        // The first target assumes one P2PKH input.
        let first_fee = fee_for_serialize_size(
            rate,
            estimate_serialize_size(&[REDEEM_P2PKH_SIG_SCRIPT_SIZE], &outputs, change_size),
        );
        assert_eq!(source.calls[0], target + first_fee.0);
        // A retry happens only when the inputs fell short of a fee larger
        // than the last target's, so with sane values targets only grow.
        assert!(source.calls.windows(2).all(|w| w[0] <= w[1]), "{case}");
        let atx = match result {
            Ok(atx) => atx,
            Err(e) => {
                assert!(e.is_kind(Kind::InsufficientBalance), "{e}");
                short += 1;
                continue;
            }
        };
        let n_in = atx.tx.tx_in.len();
        let script_sizes: Vec<i64> = case["source"]["utxos"]
            .as_array()
            .expect("utxos")
            .iter()
            .take(n_in)
            .map(|u| u[1].as_i64().expect("size"))
            .collect();
        let fee = fee_for_serialize_size(
            rate,
            estimate_serialize_size(&script_sizes, &outputs, change_size),
        )
        .0;
        assert_eq!(atx.tx.version, GENERATED_TX_VERSION);
        assert_eq!(atx.tx.tx_out[..outputs.len()], outputs[..]);
        assert_eq!(atx.prev_scripts.len(), n_in);
        let total = atx.total_input.0;
        assert!(total >= target + fee, "{case}");
        let remainder = total - target - fee;
        if atx.change_index >= 0 {
            with_change += 1;
            assert_eq!(atx.change_index as usize, outputs.len());
            assert_eq!(atx.tx.tx_out.len(), outputs.len() + 1);
            let c = &atx.tx.tx_out[outputs.len()];
            assert_eq!(c.value, remainder, "change is the remainder after the fee");
            assert!(!is_dust_amount(Amount(c.value), change_size, rate));
            assert_eq!(
                atx.estimated_signed_serialize_size,
                estimate_serialize_size(&script_sizes, &outputs, change_size)
            );
            assert_eq!(change.script_calls, 1);
        } else {
            without += 1;
            assert_eq!(atx.tx.tx_out, outputs);
            assert!(
                remainder == 0 || is_dust_amount(Amount(remainder), change_size, rate),
                "{case}"
            );
            assert_eq!(
                atx.estimated_signed_serialize_size,
                estimate_serialize_size(&script_sizes, &outputs, 0)
            );
            assert_eq!(change.script_calls, 0);
        }
        assert_eq!(change.size_calls.get(), 1);
    }
    eprintln!("authoring: {with_change} with change, {without} without, {short} short");
    assert!(with_change > 0 && without > 0 && short > 0);
}

#[test]
fn authoring_is_total() {
    // Any input, with the source's call limit standing in for Go's endless
    // loop (QK-0744): no panic, whatever the values.
    let mut rng = SplitMix64::from_env("purse-txauthor properties: total");
    for _ in 0..cases() * 20_000 {
        let case = random_author_case(&mut rng);
        let _ = author_answer(&case);
    }
}

#[test]
fn randomization_is_a_uniform_transposition() {
    let mut rng = SplitMix64::from_env("purse-txauthor properties: randomize");
    let mut seed = [0u8; 32];
    rng.fill(&mut seed);
    let mut prng = Prng::from_seed(seed);
    for n in [1usize, 2, 3, 5, 7, 16, 100] {
        let rounds = cases() * 20_000;
        let mut counts = vec![0usize; n];
        for round in 0..rounds {
            let mut outs: Vec<TxOut> = (0..n)
                .map(|i| TxOut {
                    value: i as i64,
                    ..TxOut::default()
                })
                .collect();
            let index = round % n;
            let r = randomize_output_position_with(&mut outs, index as i64, &mut prng)
                .expect("in range") as usize;
            assert!(r < n);
            counts[r] += 1;
            // A swap of r and index, nothing else.
            for (i, o) in outs.iter().enumerate() {
                let want = if i == r {
                    index
                } else if i == index {
                    r
                } else {
                    i
                };
                assert_eq!(o.value, want as i64);
            }
        }
        // Chi-square against uniform, with a bound far in the tail for up to
        // 99 degrees of freedom.
        let expect = rounds as f64 / n as f64;
        let chi2: f64 = counts
            .iter()
            .map(|&c| (c as f64 - expect).powi(2) / expect)
            .sum();
        assert!(
            chi2 < 3.0 * n as f64 + 30.0,
            "n {n}: chi2 {chi2}, counts {counts:?}"
        );
    }
    // The same seed gives the same positions.
    let run = |seed: [u8; 32]| {
        let mut p = Prng::from_seed(seed);
        (0..64)
            .map(|_| {
                let mut outs = vec![TxOut::default(); 9];
                randomize_output_position_with(&mut outs, 0, &mut p).expect("in range")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(seed), run(seed));
}

/// A secrets source over a list of keys, addressed by their P2PKH
/// addresses.
struct Keys {
    params: dcroxide_chaincfg::Params,
    keys: Vec<(Address, Key)>,
}

impl SecretsSource for Keys {
    fn chain_params(&self) -> &dcroxide_chaincfg::Params {
        &self.params
    }
    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError> {
        let (_, k) = self
            .keys
            .iter()
            .find(|(a, _)| a == addr)
            .ok_or_else(|| purse_errors::new("no key"))?;
        let t = match k.sig_type {
            0 => SignatureType::EcdsaSecp256k1,
            1 => SignatureType::Ed25519,
            _ => SignatureType::SchnorrSecp256k1,
        };
        Ok((k.priv_bytes.clone(), t, k.compressed))
    }
    fn get_script(&mut self, _: &Address) -> Result<Vec<u8>, AnyError> {
        Err(purse_errors::new("no scripts"))
    }
}

#[test]
fn signed_inputs_verify() {
    let mut rng = SplitMix64::from_env("purse-txauthor properties: signing");
    let params = dcroxide_chaincfg::simnet_params();
    let flags = ScriptFlags::DISCOURAGE_UPGRADABLE_NOPS
        .union(ScriptFlags::VERIFY_CLEAN_STACK)
        .union(ScriptFlags::VERIFY_CHECK_LOCK_TIME_VERIFY)
        .union(ScriptFlags::VERIFY_CHECK_SEQUENCE_VERIFY)
        .union(ScriptFlags::VERIFY_TREASURY);
    for _ in 0..cases() * 50 {
        let n = 1 + rng.below(4) as usize;
        let mut keys = Vec::new();
        let mut prev = Vec::new();
        let mut tx = MsgTx::default();
        for i in 0..n {
            let k = match rng.below(4) {
                0 => secp_key(&mut rng, false, 0),
                1 => ed_key(&mut rng),
                2 => secp_key(&mut rng, true, 2),
                _ => secp_key(&mut rng, true, 0),
            };
            let h = stdaddr::hash160(&k.pub_key);
            let a = match k.sig_type {
                0 => stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&h, &params),
                1 => stdaddr::new_address_pub_key_hash_ed25519_v0(&h, &params),
                _ => stdaddr::new_address_pub_key_hash_schnorr_secp256k1_v0(&h, &params),
            }
            .expect("address");
            prev.push(a.payment_script().1);
            keys.push((a, k));
            tx.tx_in.push(new_tx_in(
                OutPoint {
                    hash: utxo_hash(i),
                    index: i as u32,
                    tree: 0,
                },
                100_000_000,
                Vec::new(),
            ));
        }
        tx.tx_out.push(TxOut {
            value: 1,
            version: 0,
            pk_script: vec![0x6a],
        });
        let mut secrets = Keys {
            params: params.clone(),
            keys,
        };
        add_all_input_scripts(&mut tx, &prev, &mut secrets).expect("signed");
        for (i, script) in prev.iter().enumerate() {
            let mut engine = Engine::new(script, &tx, i, flags, 0).expect("engine");
            engine
                .execute()
                .unwrap_or_else(|e| panic!("input {i} of {n}: {e:?}"));
        }
        // Signing again keeps every script (equal length: the existing one
        // wins the tie), so the transaction is unchanged.
        let before = tx.clone();
        add_all_input_scripts(&mut tx, &prev, &mut secrets).expect("signed again");
        assert_eq!(tx, before);
    }
}

#[test]
fn lookup_errors_keep_their_kind() {
    let mut rng = SplitMix64::from_env("purse-txauthor properties: lookup errors");
    let params = dcroxide_chaincfg::mainnet_params();
    let k = secp_key(&mut rng, true, 0);
    let a = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
        &stdaddr::hash160(&k.pub_key),
        &params,
    )
    .expect("address");
    struct Locked(dcroxide_chaincfg::Params);
    impl SecretsSource for Locked {
        fn chain_params(&self) -> &dcroxide_chaincfg::Params {
            &self.0
        }
        fn get_key(&mut self, _: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError> {
            Err(purse_errors::e!(Kind::Locked, "wallet is locked").into())
        }
        fn get_script(&mut self, _: &Address) -> Result<Vec<u8>, AnyError> {
            Err(purse_errors::e!(Kind::NotExist, "no script").into())
        }
    }
    let mut tx = MsgTx::default();
    tx.tx_in.push(new_tx_in(OutPoint::default(), 1, Vec::new()));
    let err =
        add_all_input_scripts(&mut tx, &[a.payment_script().1], &mut Locked(params)).unwrap_err();
    assert!(err.is_kind(Kind::Locked), "{err}");
}
