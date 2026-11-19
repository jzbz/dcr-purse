// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `wallet/txrules/poolfees_test.go` at the parity pin, ported:
//! `TestStakePoolTicketFee`, its 7 mainnet vectors unchanged. The amounts
//! are written in atoms; Go writes them as float constants times 1e8
//! (`0.01500463 * 1e8`), which are exact.

use purse_dcrutil::Amount;
use purse_txrules::{StakePoolFees, stake_pool_ticket_fee};

struct Case {
    stake_diff: i64,
    fee: i64,
    height: i32,
    pool_fee: f64,
    expected: i64,
    dcp0010: bool,
    dcp0012: bool,
}

const COIN: i64 = 100_000_000;

#[rustfmt::skip]
const TESTS: [Case; 7] = [
    Case { stake_diff: 10 * COIN, fee: 1_000_000, height: 25000, pool_fee: 1.00, expected: 1_500_463, dcp0010: false, dcp0012: false },
    Case { stake_diff: 20 * COIN, fee: 1_000_000, height: 25000, pool_fee: 1.00, expected: 1_621_221, dcp0010: false, dcp0012: false },
    Case { stake_diff: 5 * COIN, fee: 5_000_000, height: 50000, pool_fee: 2.59, expected: 3_310_616, dcp0010: false, dcp0012: false },
    Case { stake_diff: 15 * COIN, fee: 5_000_000, height: 50000, pool_fee: 2.59, expected: 3_956_376, dcp0010: false, dcp0012: false },
    Case { stake_diff: 15 * COIN, fee: 5_000_000, height: 50000, pool_fee: 2.59, expected: 9_023_823, dcp0010: true, dcp0012: false },
    Case { stake_diff: 15 * COIN, fee: 5_000_000, height: 50000, pool_fee: 2.59, expected: 9_784_185, dcp0010: false, dcp0012: true },
    Case { stake_diff: 15 * COIN, fee: 5_000_000, height: 50000, pool_fee: 2.59, expected: 9_784_185, dcp0010: true, dcp0012: true },
];

#[test]
fn test_stake_pool_ticket_fee() {
    let params = dcroxide_chaincfg::mainnet_params();
    let mut cached = StakePoolFees::new(&params);
    for (i, t) in TESTS.iter().enumerate() {
        let got = stake_pool_ticket_fee(
            Amount(t.stake_diff),
            Amount(t.fee),
            t.height,
            t.pool_fee,
            &params,
            t.dcp0010,
            t.dcp0012,
        )
        .expect("fee");
        assert_eq!(
            got,
            Amount(t.expected),
            "Test {i}: Got {got}: Want {}",
            t.expected
        );
        let again = cached
            .ticket_fee(
                Amount(t.stake_diff),
                Amount(t.fee),
                t.height,
                t.pool_fee,
                t.dcp0010,
                t.dcp0012,
            )
            .expect("fee");
        assert_eq!(again, got, "Test {i}: cached calculator");
    }
}
