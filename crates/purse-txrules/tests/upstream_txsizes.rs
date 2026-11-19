// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016-2025 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `wallet/txsizes/size_test.go` at the parity pin, ported:
//! `TestEstimateSerializeSize`, its 17 vectors unchanged.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: the vectors' own constant sums"
)]

use dcroxide_wire::TxOut;
use purse_txrules::estimate_serialize_size;
use purse_txrules::txsizes::{
    P2PKH_OUTPUT_SIZE, P2PKH_PK_SCRIPT_SIZE, REDEEM_P2PKH_INPUT_SIZE, REDEEM_P2PKH_SIG_SCRIPT_SIZE,
};

const P2PKH_SCRIPT_SIZE: i64 = P2PKH_PK_SCRIPT_SIZE;
const P2SH_SCRIPT_SIZE: i64 = 23;

fn make_ints(value: i64, n: usize) -> Vec<i64> {
    vec![value; n]
}

#[test]
fn test_estimate_serialize_size() {
    let r = REDEEM_P2PKH_SIG_SCRIPT_SIZE;
    #[rustfmt::skip]
    let tests: Vec<(Vec<i64>, Vec<i64>, i64, i64)> = vec![
        (vec![r], vec![], 0, 181),
        (vec![r], vec![P2PKH_SCRIPT_SIZE], 0, 217),
        (vec![r], vec![], P2PKH_SCRIPT_SIZE, 217),
        (vec![r], vec![P2PKH_SCRIPT_SIZE], P2PKH_SCRIPT_SIZE, 253),
        (vec![r], vec![P2SH_SCRIPT_SIZE], 0, 215),
        (vec![r], vec![P2SH_SCRIPT_SIZE], P2PKH_SCRIPT_SIZE, 251),

        (vec![r, r], vec![], 0, 347),
        (vec![r, r], vec![P2PKH_SCRIPT_SIZE], 0, 383),
        (vec![r, r], vec![], P2PKH_SCRIPT_SIZE, 383),
        (vec![r, r], vec![P2PKH_SCRIPT_SIZE], P2PKH_SCRIPT_SIZE, 419),
        (vec![r, r], vec![P2SH_SCRIPT_SIZE], 0, 381),
        (vec![r, r], vec![P2SH_SCRIPT_SIZE], P2PKH_SCRIPT_SIZE, 417),

        // 0xfd is discriminant for 16-bit compact ints, compact int
        // total size increases from 1 byte to 3.
        (vec![r], make_ints(P2PKH_SCRIPT_SIZE, 0xfc), 0, 9253),
        (vec![r], make_ints(P2PKH_SCRIPT_SIZE, 0xfd), 0, 9253 + P2PKH_OUTPUT_SIZE + 2),
        (vec![r], make_ints(P2PKH_SCRIPT_SIZE, 0xfc), P2PKH_SCRIPT_SIZE, 9253 + P2PKH_OUTPUT_SIZE + 2),
        (make_ints(r, 0xfc), vec![], 0, 41847),
        // 4 not 2, varint encoded twice.
        (make_ints(r, 0xfd), vec![], 0, 41847 + REDEEM_P2PKH_INPUT_SIZE + 4),
    ];
    for (i, (inputs, output_lengths, change, expected)) in tests.iter().enumerate() {
        let outputs: Vec<TxOut> = output_lengths
            .iter()
            .map(|&l| TxOut {
                pk_script: vec![0; usize::try_from(l).expect("length")],
                ..TxOut::default()
            })
            .collect();
        let actual = estimate_serialize_size(inputs, &outputs, *change);
        assert_eq!(
            actual, *expected,
            "Test {i}: Got {actual}: Expected {expected}"
        );
    }
}
