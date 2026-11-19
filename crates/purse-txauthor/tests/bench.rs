// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Times the crate's main entry points against dcrwallet's through the Go
//! oracle's `bench` command, with the same inputs on both sides, and prints
//! a table of nanoseconds per call and the Go/Rust ratio (above 1 means the
//! crate is faster). Ignored by default; run it in an optimised build:
//!
//! ```text
//! cargo test --release -p purse-txauthor --test bench -- --ignored --nocapture
//! ```
//!
//! `PURSE_TXAUTHOR_BENCH_MS` sets the time budget per entry point and side
//! (default 300). The Go side is `testing.Benchmark`'s ns/op, the best of
//! the rounds that fit the budget; the Rust side is the best of as many
//! rounds of a timed loop. Both sides build a fresh input source per
//! authoring and copy the unsigned transaction per signing, inside the
//! timed call; the Rust authoring also clones the two outputs it is given,
//! since it takes them by value where Go shares the caller's slice. Nothing is asserted: the numbers are for the bench ledger
//! (ADR-0014).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: timing arithmetic"
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use dcroxide_chainhash::Hash;
use dcroxide_dcrec::secp256k1;
use dcroxide_txscript::sign::SignatureType;
use dcroxide_txscript::stdaddr::{self, Address};
use dcroxide_wire::{MsgTx, OutPoint, TxIn, TxOut};
use purse_dcrutil::Amount;
use purse_errors::AnyError;
use purse_testutil::oracle_or_skip;
use purse_testutil::serde_json::json;
use purse_txauthor::*;
use purse_txrules::txsizes::REDEEM_P2PKH_SIG_SCRIPT_SIZE;

fn budget() -> Duration {
    Duration::from_millis(
        std::env::var("PURSE_TXAUTHOR_BENCH_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300),
    )
}

/// The best ns per call of `f` over rounds of a timed loop that together
/// take `budget`.
fn time_it(budget: Duration, mut f: impl FnMut()) -> f64 {
    let mut iters: u64 = 1;
    loop {
        let start = Instant::now();
        for _ in 0..iters {
            f();
        }
        if start.elapsed() >= Duration::from_millis(10) {
            break;
        }
        iters *= 2;
    }
    let mut best = f64::INFINITY;
    let start = Instant::now();
    while start.elapsed() < budget || best.is_infinite() {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        let ns = t.elapsed().as_nanos() as f64 / iters as f64;
        best = best.min(ns);
    }
    best
}

/// The oracle's `p2pkhScript(fill)`.
fn p2pkh_script(fill: u8) -> Vec<u8> {
    let mut s = vec![0x76, 0xa9, 0x14];
    s.extend([fill; 20]);
    s.extend([0x88, 0xac]);
    s
}

fn hash_of(i: usize) -> Hash {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&(i as u64).to_le_bytes());
    Hash(h)
}

fn tx_in(i: usize, value: i64) -> TxIn {
    TxIn {
        previous_out_point: OutPoint {
            hash: hash_of(i),
            index: i as u32,
            tree: 0,
        },
        sequence: 0xffff_ffff,
        value_in: value,
        block_height: 0,
        block_index: 0xffff_ffff,
        signature_script: Vec::new(),
    }
}

struct Change(Vec<u8>);

impl ChangeSource for Change {
    fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError> {
        Ok((self.0.clone(), 0))
    }
    fn script_size(&self) -> i64 {
        self.0.len() as i64
    }
}

/// Five 1 DCR P2PKH UTXOs offered cumulatively, as the oracle's source.
#[derive(Default)]
struct Source {
    total: i64,
    detail: InputDetail,
}

impl InputSource for Source {
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError> {
        while self.total < target.0 && self.detail.inputs.len() < 5 {
            let next = self.detail.inputs.len();
            self.detail.inputs.push(tx_in(next, 100_000_000));
            self.detail.scripts.push(p2pkh_script(next as u8));
            self.detail
                .redeem_script_sizes
                .push(REDEEM_P2PKH_SIG_SCRIPT_SIZE);
            self.total += 100_000_000;
        }
        let mut d = self.detail.clone();
        d.amount = Amount(self.total);
        Ok(d)
    }
}

struct Keys(dcroxide_chaincfg::Params, Vec<(Address, Vec<u8>)>);

impl SecretsSource for Keys {
    fn chain_params(&self) -> &dcroxide_chaincfg::Params {
        &self.0
    }
    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError> {
        let (_, k) = self
            .1
            .iter()
            .find(|(a, _)| a == addr)
            .ok_or_else(|| purse_errors::new("no key"))?;
        Ok((k.clone(), SignatureType::EcdsaSecp256k1, true))
    }
    fn get_script(&mut self, _: &Address) -> Result<Vec<u8>, AnyError> {
        Err(purse_errors::new("no scripts"))
    }
}

#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn go_vs_rust() {
    let Some(mut o) = oracle_or_skip("txauthor") else {
        return;
    };
    let budget = budget();
    let go = o.call("bench", json!({ "millis": budget.as_millis() as u64 }));
    let go = go.as_array().expect("bench rows").clone();

    let mainnet = dcroxide_chaincfg::mainnet_params();
    let outs = vec![
        TxOut {
            value: 100_000_000,
            version: 0,
            pk_script: p2pkh_script(0x11),
        },
        TxOut {
            value: 250_000_000,
            version: 0,
            pk_script: p2pkh_script(0x22),
        },
    ];
    let max = mainnet.max_tx_size as i64;
    let mut change = Change(p2pkh_script(0x33));

    let mut keys = Vec::new();
    let mut prev = Vec::new();
    let mut unsigned = MsgTx::default();
    for i in 0..2u8 {
        let k = [i + 1; 32];
        let pk = secp256k1::PrivateKey::from_bytes(&k)
            .expect("key")
            .public_key()
            .serialize_compressed();
        let a =
            stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(&stdaddr::hash160(&pk), &mainnet)
                .expect("address");
        prev.push(a.payment_script().1);
        keys.push((a, k.to_vec()));
        unsigned.tx_in.push(tx_in(i as usize, 200_000_000));
    }
    unsigned.tx_out.push(TxOut {
        value: 300_000_000,
        version: 0,
        pk_script: p2pkh_script(0x44),
    });
    let mut secrets = Keys(mainnet.clone(), keys);
    let mut prng = dcroxide_crypto::rand::Prng::from_seed([9; 32]);
    // Go builds a three-pointer slice per call; the Rust side permutes one
    // array in place, so neither times copying outputs.
    let mut three = [outs[0].clone(), outs[1].clone(), outs[0].clone()];

    let rust: Vec<(&str, f64)> = vec![
        (
            "NewUnsignedTransaction(2 P2PKH out, 5 UTXOs, change)",
            time_it(budget, || {
                let mut source = Source::default();
                black_box(new_unsigned_transaction(
                    black_box(outs.clone()),
                    Amount(10_000),
                    &mut source,
                    &mut change,
                    max,
                ))
                .ok();
            }),
        ),
        (
            "RandomizeOutputPosition(3 outputs)",
            time_it(budget, || {
                black_box(randomize_output_position(black_box(&mut three), 2)).ok();
            }),
        ),
        (
            "AddAllInputScripts(2 P2PKH inputs)",
            time_it(budget, || {
                let mut tx = unsigned.clone();
                black_box(add_all_input_scripts(&mut tx, &prev, &mut secrets)).ok();
            }),
        ),
    ];

    assert_eq!(go.len(), rust.len(), "entry points");
    println!(
        "\n{:<58} {:>10} {:>10} {:>8}",
        "entry point", "Go ns", "Rust ns", "Go/Rust"
    );
    for (row, (name, rust_ns)) in go.iter().zip(&rust) {
        assert_eq!(row["name"].as_str(), Some(*name), "entry point order");
        let go_ns = row["ns"].as_f64().expect("ns");
        println!(
            "{name:<58} {go_ns:>10.2} {rust_ns:>10.2} {:>8.2}",
            go_ns / rust_ns
        );
    }
    let injected = time_it(budget, || {
        black_box(randomize_output_position_with(
            black_box(&mut three),
            2,
            &mut prng,
        ))
        .ok();
    });
    println!("randomize_output_position_with an owned Prng: {injected:.2} ns");

    // The signing loop's own share: dcroxide's sign_tx_output called
    // directly on the same inputs, and the secp256k1 steps it is made of.
    let direct = time_it(budget, || {
        let mut tx = unsigned.clone();
        for (i, script) in prev.iter().enumerate() {
            let (a, k) = &secrets.1[i];
            let a = a.clone();
            let k = k.clone();
            let mut kdb = move |addr: &Address| {
                assert_eq!(addr, &a);
                Ok((k.clone(), SignatureType::EcdsaSecp256k1, true))
            };
            let mut sdb = |_: &Address| Err(dcroxide_txscript::sign::SignError(String::new()));
            let s = dcroxide_txscript::sign::sign_tx_output(
                &mainnet,
                &tx,
                i,
                script,
                dcroxide_txscript::SIG_HASH_ALL,
                &mut kdb,
                &mut sdb,
                &[],
                true,
            )
            .expect("signed");
            tx.tx_in[i].signature_script = s;
        }
        black_box(tx);
    });
    let key = secp256k1::PrivateKey::from_bytes(&[1; 32]).expect("key");
    let pubkey = time_it(budget, || {
        black_box(black_box(&key).public_key().serialize_compressed());
    });
    let ecdsa = time_it(budget, || {
        black_box(secp256k1::ecdsa::sign(black_box(&key), &[7; 32]));
    });
    println!(
        "dcroxide sign_tx_output, 2 inputs, called directly: {direct:.2} ns; per input, \
         secp256k1 public_key: {pubkey:.2} ns, ecdsa::sign: {ecdsa:.2} ns"
    );
}
