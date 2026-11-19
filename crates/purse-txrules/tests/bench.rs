// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Times the crate's main entry points against dcrwallet's through the Go
//! oracle's `bench` command, with the same inputs on both sides, and prints
//! a table of nanoseconds per call and the Go/Rust ratio (above 1 means the
//! crate is faster). Ignored by default; run it in an optimised build:
//!
//! ```text
//! cargo test --release -p purse-txrules --test bench -- --ignored --nocapture
//! ```
//!
//! `PURSE_TXRULES_BENCH_MS` sets the time budget per entry point and side
//! (default 300). The Go side is `testing.Benchmark`'s ns/op, the best of
//! the rounds that fit the budget; the Rust side is the best of as many
//! rounds of a timed loop. Nothing is asserted: the numbers are for the
//! bench ledger (ADR-0014).

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: timing arithmetic"
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use dcroxide_wire::{MsgTx, TxIn, TxOut};
use purse_dcrutil::Amount;
use purse_testutil::oracle_or_skip;
use purse_testutil::serde_json::json;
use purse_txrules::txsizes::{P2PKH_PK_SCRIPT_SIZE, REDEEM_P2PKH_SIG_SCRIPT_SIZE};
use purse_txrules::*;

fn budget() -> Duration {
    Duration::from_millis(
        std::env::var("PURSE_TXRULES_BENCH_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300),
    )
}

/// The best ns per call of `f` over rounds of a timed loop that together
/// take `budget`.
fn time_it(budget: Duration, mut f: impl FnMut()) -> f64 {
    // Calibrate a round to about 10 ms.
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

/// The P2PKH script `tools/oracle/cmd/txrules` benchmarks with.
fn p2pkh_script() -> Vec<u8> {
    let mut s = vec![0x76, 0xa9, 0x14];
    s.extend((3u8..23).collect::<Vec<u8>>());
    s.extend([0x88, 0xac]);
    s
}

#[test]
#[ignore = "timing table; run in a release build with --ignored --nocapture"]
fn bench_against_go() {
    let Some(mut o) = oracle_or_skip("txrules") else {
        return;
    };
    let budget = budget();
    let go = o.call("bench", json!({ "millis": budget.as_millis() as u64 }));
    let go = go.as_array().expect("bench rows");

    let p2pkh = p2pkh_script();
    let outs = vec![
        TxOut {
            value: 100_000_000,
            version: 0,
            pk_script: p2pkh.clone(),
        },
        TxOut {
            value: 250_000_000,
            version: 0,
            pk_script: p2pkh,
        },
    ];
    let script_sizes = [REDEEM_P2PKH_SIG_SCRIPT_SIZE, REDEEM_P2PKH_SIG_SCRIPT_SIZE];
    let tx = MsgTx {
        tx_in: (0..2)
            .map(|_| TxIn {
                value_in: 200_000_000,
                signature_script: vec![0; REDEEM_P2PKH_SIG_SCRIPT_SIZE as usize],
                ..TxIn::default()
            })
            .collect(),
        tx_out: outs.clone(),
        ..MsgTx::default()
    };
    let mainnet = dcroxide_chaincfg::mainnet_params();
    let mut fees = StakePoolFees::new(&mainnet);
    let _ = fees.ticket_fee(
        Amount(15e8 as i64),
        Amount(5e6 as i64),
        1_000_000,
        2.59,
        true,
        true,
    );

    let rust: Vec<(&str, f64)> = vec![
        (
            "EstimateSerializeSize(2 P2PKH in, 2 P2PKH out, change)",
            time_it(budget, || {
                black_box(estimate_serialize_size(
                    black_box(&script_sizes),
                    black_box(&outs),
                    black_box(P2PKH_PK_SCRIPT_SIZE),
                ));
            }),
        ),
        (
            "EstimateSerializeSizeFromScriptSizes(2 in, 2 out, change)",
            time_it(budget, || {
                black_box(estimate_serialize_size_from_script_sizes(
                    black_box(&script_sizes),
                    black_box(&[25, 25]),
                    black_box(P2PKH_PK_SCRIPT_SIZE),
                ));
            }),
        ),
        (
            "FeeForSerializeSize",
            time_it(budget, || {
                black_box(fee_for_serialize_size(
                    black_box(Amount(10_000)),
                    black_box(419),
                ));
            }),
        ),
        (
            "IsDustOutput(P2PKH)",
            time_it(budget, || {
                black_box(is_dust_output(
                    black_box(&outs[0]),
                    black_box(Amount(10_000)),
                ));
            }),
        ),
        (
            "CheckOutput(P2PKH)",
            time_it(budget, || {
                black_box(check_output(black_box(&outs[0]), black_box(Amount(10_000)))).ok();
            }),
        ),
        (
            "PaysHighFees(2 in, 2 out)",
            time_it(budget, || {
                black_box(pays_high_fees(
                    black_box(Amount(400_000_000)),
                    black_box(&tx),
                ));
            }),
        ),
        (
            "TxPaysHighFees(2 in, 2 out)",
            time_it(budget, || {
                black_box(tx_pays_high_fees(black_box(&tx))).ok();
            }),
        ),
        (
            "StakePoolTicketFee(mainnet, height 1000000)",
            time_it(budget, || {
                black_box(fees.ticket_fee(
                    black_box(Amount(1_500_000_000)),
                    black_box(Amount(5_000_000)),
                    black_box(1_000_000),
                    black_box(2.59),
                    true,
                    true,
                ))
                .ok();
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
    println!(
        "(StakePoolTicketFee: Rust with a kept StakePoolFees; the free function builds a \
         subsidy cache per call)"
    );
    let per_call = time_it(budget, || {
        black_box(stake_pool_ticket_fee(
            black_box(Amount(1_500_000_000)),
            black_box(Amount(5_000_000)),
            black_box(1_000_000),
            black_box(2.59),
            &mainnet,
            true,
            true,
        ))
        .ok();
    });
    println!("stake_pool_ticket_fee, fresh cache per call: {per_call:.2} ns");
}
