// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `TestNewUnsignedTransaction` (`wallet/txauthor/
//! author_test.go`), ported verbatim with its helpers: a change source
//! returning 25 zero bytes with a declared size of 25, "P2PKH outputs"
//! whose scripts are 36 zero bytes (`txsizes.P2PKHOutputSize`, QK-0654; the
//! expected values depend on that length), and an input source that hands
//! out the unspent outputs one at a time, cumulatively, each with a
//! signature-script size of 108. Case 0's `InputSourceError` is Go's
//! `errors.Is(err, errors.InsufficientBalance)`.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: the vectors' own arithmetic on small amounts"
)]

use dcroxide_wire::{OutPoint, TxIn, TxOut};
use purse_dcrutil::Amount;
use purse_errors::{AnyError, Kind};
use purse_txauthor::{ChangeSource, InputDetail, InputSource, new_unsigned_transaction};
use purse_txrules::txsizes::{
    P2PKH_OUTPUT_SIZE, P2PKH_PK_SCRIPT_SIZE, REDEEM_P2PKH_SIG_SCRIPT_SIZE,
};
use purse_txrules::{estimate_serialize_size, fee_for_serialize_size};

struct AuthorTestChangeSource;

impl ChangeSource for AuthorTestChangeSource {
    fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError> {
        // Only length matters for these tests.
        Ok((vec![0; P2PKH_PK_SCRIPT_SIZE as usize], 0))
    }
    fn script_size(&self) -> i64 {
        P2PKH_PK_SCRIPT_SIZE
    }
}

fn p2pkh_outputs(amounts: &[i64]) -> Vec<TxOut> {
    amounts
        .iter()
        .map(|&a| TxOut {
            value: a,
            version: 0,
            pk_script: vec![0; P2PKH_OUTPUT_SIZE as usize],
        })
        .collect()
}

/// Returns outputs in order.
fn make_input_source(unspents: Vec<TxOut>) -> impl InputSource {
    let mut unspents = unspents.into_iter();
    let mut current_total = 0i64;
    let mut current_inputs: Vec<TxIn> = Vec::new();
    let mut redeem_script_sizes: Vec<i64> = Vec::new();
    move |target: Amount| -> Result<InputDetail, AnyError> {
        while current_total < target.0 {
            let Some(u) = unspents.next() else { break };
            current_inputs.push(TxIn {
                previous_out_point: OutPoint::default(),
                sequence: 0xffff_ffff,
                value_in: u.value,
                block_height: 0,
                block_index: 0xffff_ffff,
                signature_script: Vec::new(),
            });
            current_total += u.value;
            redeem_script_sizes.push(REDEEM_P2PKH_SIG_SCRIPT_SIZE);
        }
        Ok(InputDetail {
            amount: Amount(current_total),
            inputs: current_inputs.clone(),
            scripts: vec![Vec::new(); current_inputs.len()],
            redeem_script_sizes: redeem_script_sizes.clone(),
        })
    }
}

struct Case {
    unspent_outputs: Vec<TxOut>,
    outputs: Vec<TxOut>,
    relay_fee: i64,
    change_amount: i64,
    input_source_error: bool,
    input_count: usize,
}

fn fee(rate: i64, n_inputs: usize, outputs: &[TxOut]) -> i64 {
    fee_for_serialize_size(
        Amount(rate),
        estimate_serialize_size(
            &vec![REDEEM_P2PKH_SIG_SCRIPT_SIZE; n_inputs],
            outputs,
            P2PKH_PK_SCRIPT_SIZE,
        ),
    )
    .0
}

fn cases() -> Vec<Case> {
    let c = |unspent: &[i64], out: &[i64], relay: i64, change: i64, err: bool, n: usize| Case {
        unspent_outputs: p2pkh_outputs(unspent),
        outputs: p2pkh_outputs(out),
        relay_fee: relay,
        change_amount: change,
        input_source_error: err,
        input_count: n,
    };
    let zero = p2pkh_outputs(&[0]);
    vec![
        // 0
        c(&[100_000_000], &[100_000_000], 1_000, 0, true, 0),
        // 1
        c(
            &[100_000_000],
            &[1_000_000],
            1_000,
            100_000_000 - 1_000_000 - fee(1_000, 1, &p2pkh_outputs(&[1_000_000])),
            false,
            1,
        ),
        // 2
        c(
            &[100_000_000],
            &[1_000_000],
            10_000,
            100_000_000 - 1_000_000 - fee(10_000, 1, &p2pkh_outputs(&[1_000_000])),
            false,
            1,
        ),
        // 3
        c(
            &[100_000_000],
            &[1_000_000, 1_000_000, 1_000_000],
            10_000,
            100_000_000
                - 3_000_000
                - fee(
                    10_000,
                    1,
                    &p2pkh_outputs(&[1_000_000, 1_000_000, 1_000_000]),
                ),
            false,
            1,
        ),
        // 4
        c(
            &[100_000_000],
            &[1_000_000, 1_000_000, 1_000_000],
            2_550,
            100_000_000
                - 3_000_000
                - fee(2_550, 1, &p2pkh_outputs(&[1_000_000, 1_000_000, 1_000_000])),
            false,
            1,
        ),
        // Test dust thresholds (603 for a 1e3 relay fee).
        // 5
        c(
            &[100_000_000],
            &[100_000_000 - 602 - fee(1_000, 1, &zero)],
            1_000,
            0,
            false,
            1,
        ),
        // 6
        c(
            &[100_000_000],
            &[100_000_000 - 603 - fee(1_000, 1, &zero)],
            1_000,
            603,
            false,
            1,
        ),
        // Test dust thresholds (1537.65 for a 2.55e3 relay fee).
        // 7
        c(
            &[100_000_000],
            &[100_000_000 - 1537 - fee(2_550, 1, &zero)],
            2_550,
            0,
            false,
            1,
        ),
        // 8
        c(
            &[100_000_000],
            &[100_000_000 - 1538 - fee(2_550, 1, &zero)],
            2_550,
            1538,
            false,
            1,
        ),
        // Test two unspent outputs available but only one needed
        // (tested fee only includes one input rather than using a
        // serialize size for each).
        // 9
        c(
            &[100_000_000, 100_000_000],
            &[100_000_000 - 603 - fee(1_000, 1, &zero)],
            1_000,
            603,
            false,
            1,
        ),
        // Test that second output is not included to make the change
        // output not dust and be included in the transaction.
        //
        // It's debatable whether or not this is a good idea, but it's
        // how the function was written, so test it anyways.
        // 10
        c(
            &[100_000_000, 100_000_000],
            &[100_000_000 - 545 - fee(1_000, 1, &zero)],
            1_000,
            0,
            false,
            1,
        ),
        // Test two unspent outputs available where both are needed.
        // 11
        c(
            &[100_000_000, 100_000_000],
            &[100_000_000],
            1_000,
            100_000_000 - fee(1_000, 2, &p2pkh_outputs(&[100_000_000])),
            false,
            2,
        ),
        // Test that zero change outputs are not included
        // (ChangeAmount=0 means don't include any change output).
        // 12
        c(&[100_000_000], &[100_000_000], 0, 0, false, 1),
    ]
}

#[test]
fn test_new_unsigned_transaction() {
    let max_tx_size = dcroxide_chaincfg::mainnet_params().max_tx_size as i64;
    let mut change_source = AuthorTestChangeSource;
    let mut failures = Vec::new();
    for (i, test) in cases().into_iter().enumerate() {
        let mut input_source = make_input_source(test.unspent_outputs);
        let tx = match new_unsigned_transaction(
            test.outputs,
            Amount(test.relay_fee),
            &mut input_source,
            &mut change_source,
            max_tx_size,
        ) {
            Ok(tx) => tx,
            Err(err) => {
                let insufficient_balance = err.is_kind(Kind::InsufficientBalance);
                if insufficient_balance != test.input_source_error {
                    if !test.input_source_error {
                        failures.push(format!(
                            "Test {i}: InsufficientBalance={insufficient_balance} expected {}",
                            test.input_source_error
                        ));
                    }
                } else if !insufficient_balance {
                    failures.push(format!("Test {i}: Unexpected error: {err}"));
                }
                continue;
            }
        };
        if tx.change_index < 0 {
            if test.change_amount != 0 {
                failures.push(format!(
                    "Test {i}: No change output added but expected output with amount {}",
                    test.change_amount
                ));
                continue;
            }
        } else {
            let change_amount = tx.tx.tx_out[tx.change_index as usize].value;
            if test.change_amount == 0 {
                failures.push(format!(
                    "Test {i}: Included change output with value {change_amount} but expected no change"
                ));
                continue;
            }
            if change_amount != test.change_amount {
                failures.push(format!(
                    "Test {i}: Got change amount {change_amount}, Expected {}",
                    test.change_amount
                ));
                continue;
            }
        }
        if tx.tx.tx_in.len() != test.input_count {
            failures.push(format!(
                "Test {i}: Used {} outputs from input source, Expected {}",
                tx.tx.tx_in.len(),
                test.input_count
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Go's test passes case 0 whether or not an error comes back (a success
/// would leave `InputSourceError` unchecked); this pins that it is the
/// insufficient-balance error, with Go's text.
#[test]
fn case_0_is_insufficient_balance() {
    let mut source = make_input_source(p2pkh_outputs(&[100_000_000]));
    let err = new_unsigned_transaction(
        p2pkh_outputs(&[100_000_000]),
        Amount(1_000),
        &mut source,
        &mut AuthorTestChangeSource,
        393_216,
    )
    .unwrap_err();
    assert!(err.is_kind(Kind::InsufficientBalance));
    assert_eq!(
        err.render(":: "),
        "txauthor.NewUnsignedTransaction: insufficient balance"
    );
}
