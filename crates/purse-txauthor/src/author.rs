// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2026 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The authoring loop (Go `NewUnsignedTransaction`, `wallet/txauthor/
//! author.go:90-166`; spec 10 §4.5) and the types it works with.

use dcroxide_txscript::MAX_SCRIPT_ELEMENT_SIZE;
use dcroxide_wire::{MsgTx, TxIn, TxOut, TxSerializeType};
use purse_dcrutil::Amount;
use purse_errors::{AnyError, Error, Kind, Op, e};
use purse_txrules::txsizes::REDEEM_P2PKH_SIG_SCRIPT_SIZE;
use purse_txrules::{estimate_serialize_size, fee_for_serialize_size, is_dust_amount};

/// The version of every authored transaction (Go's unexported
/// `generatedTxVersion`, `author.go:28`).
///
/// A constant of its own rather than wire's current version, as in Go: a
/// new transaction version may need changes here before it can be
/// authored.
pub const GENERATED_TX_VERSION: u16 = 1;

/// The operation of every error the loop wraps.
const OP: Op = Op::new("txauthor.NewUnsignedTransaction");

/// What an input source offers for a target amount (Go `InputDetail`): the
/// total spendable amount, the inputs, the previous output scripts they
/// spend (same order), and the worst-case signature-script size of each
/// input.
///
/// Despite its name, `redeem_script_sizes` holds signature-script sizes, as
/// Go's `RedeemScriptSizes` does (spec 10 quirk 23). Sizes are Go `int`,
/// `i64` here, like every size in `purse-txrules`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDetail {
    /// The total value of `inputs`, as the source reports it. The loop
    /// trusts it: nothing compares it with the inputs' `value_in`.
    pub amount: Amount,
    /// The inputs, in the order they go into the transaction.
    pub inputs: Vec<TxIn>,
    /// The previous output script of each input.
    pub scripts: Vec<Vec<u8>>,
    /// The worst-case signature-script size of each input.
    pub redeem_script_sizes: Vec<i64>,
}

/// Provides inputs referencing spendable outputs for a target amount (Go
/// `InputSource`).
///
/// The loop calls it with increasing targets (in practice; a source that
/// answers with fewer inputs can make the next target smaller) and expects
/// cumulative answers. A source that cannot reach the target says so by
/// answering a smaller `amount`, which the loop turns into an
/// `InsufficientBalance` error, or by returning an error of its own, which
/// the loop wraps with its operation. Unlike Go's, the loop never asks for
/// the same target twice in a row (QK-0744, QK-0753); a source whose
/// answers make the targets cycle with a longer period keeps it asking, as
/// in Go.
///
/// Any `FnMut(Amount) -> Result<InputDetail, AnyError>` is a source. Go's
/// source returns a pointer that may be nil alongside a nil error, which
/// panics in the loop; a Rust source cannot answer that (ADR-0012 Rule 4).
pub trait InputSource {
    /// The inputs for `target`, cumulative over the calls of one authoring.
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError>;
}

impl<F> InputSource for F
where
    F: FnMut(Amount) -> Result<InputDetail, AnyError>,
{
    fn fetch(&mut self, target: Amount) -> Result<InputDetail, AnyError> {
        self(target)
    }
}

/// Provides the change output's script (Go `ChangeSource`).
///
/// [`script_size`](ChangeSource::script_size) is consulted once, before any
/// input is fetched, and alone drives the size estimate and the dust test;
/// [`script`](ChangeSource::script) is called at most once per authoring,
/// and only when a change output is added. The script's real length is
/// never compared with the declared size (spec 10 quirk 16).
pub trait ChangeSource {
    /// The change script and its script version.
    fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError>;
    /// The size the change script will have, for fee estimation.
    fn script_size(&self) -> i64;
}

/// A newly authored transaction (Go `AuthoredTx`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredTx {
    /// The unsigned transaction.
    pub tx: MsgTx,
    /// The previous output script of each input, from the input source.
    pub prev_scripts: Vec<Vec<u8>>,
    /// The input source's reported total.
    pub total_input: Amount,
    /// The change output's index, negative (-1) when there is none. Go
    /// `int`; kept signed because callers set and pass it as Go's does.
    pub change_index: i64,
    /// The worst-case size of the transaction once signed.
    pub estimated_signed_serialize_size: i64,
}

/// Go's `sumOutputValues`: the wrapping sum of the output values.
fn sum_output_values(outputs: &[TxOut]) -> i64 {
    outputs
        .iter()
        .fold(0i64, |sum, out| sum.wrapping_add(out.value))
}

/// Authors an unsigned transaction paying `outputs` at `relay_fee_per_kb`
/// (Go `NewUnsignedTransaction`; spec 10 §4.5).
///
/// Inputs come from repeated calls to `fetch_inputs` with the output total
/// plus the fee for the worst-case signed size, raised until the inputs
/// pay for themselves. Whatever is left beyond that fee goes to a change
/// output from `fetch_change` appended after `outputs`, unless it is zero
/// or dust at `relay_fee_per_kb`, in which case it is left to the fee and
/// the size estimate is recomputed without the change output.
///
/// Every Amount sum and difference wraps as Go's `int64` arithmetic does.
///
/// # Errors
///
/// In the order Go checks them:
///
/// - an error from `fetch_inputs`, wrapped with the operation
///   `txauthor.NewUnsignedTransaction`;
/// - `txauthor.NewUnsignedTransaction: insufficient balance` when the
///   source's amount falls short of the target, and also when a retry
///   would ask the source again for the target it was just asked for,
///   where Go asks for it again (QK-0744, QK-0753; see below);
/// - `invalid operation: signed tx size exceeds allowed maximum` (no
///   operation, as in Go) when the worst-case signed size, which counts the
///   change output even if none is then added, exceeds `max_tx_size`;
/// - an error from `fetch_change`, wrapped with the operation;
/// - `invalid operation: script size exceed maximum bytes pushable to the
///   stack` (no operation) when the change script is longer than 2048
///   bytes.
///
/// # Divergence: the endless loop
///
/// Go retries with the fee its source's answer requires and never checks
/// that the retry changes anything. When a wrapped sum stalls the loop (an
/// output total within the fee of `i64::MAX`, so that the target wraps
/// negative, QK-0744, whether the source answers with no inputs or with
/// inputs covering the output total whose own fee wraps the retry; or a
/// source amount that exceeds a wrapped output total by more than
/// `i64::MAX`, so that the remaining amount wraps negative, QK-0753), the
/// source answers the same and Go asks it for the same target forever,
/// holding the wallet's outpoint lock. This function returns the
/// insufficient-balance error instead at the point where the retry would
/// repeat the target just asked for (ADR-0012 Rule 2(a); project owner,
/// 2026-11-19), without making that call. Without a wrap each retry raises
/// the target, so a repeat needs one. At a repeat Go ends only if the
/// source fails or answers the repeated target differently; the wallet's
/// sources answer it the same way. Every answer Go gives is otherwise
/// kept, the calls before the repeat included.
///
/// Only that stall, of period 1, is refused. A source whose answer stops
/// changing, as a cumulative source's does once it has added what it will
/// (the wallet's `udb` source and its all-outputs wrapper among them), can
/// stall in no other way: equal answers give equal retry targets. A source
/// whose answers make the targets cycle with a period of 2 or more keeps
/// Go's endless loop here too; only a source whose answer never settles,
/// such as one that answers each target afresh, can do that.
pub fn new_unsigned_transaction<I, C>(
    outputs: Vec<TxOut>,
    relay_fee_per_kb: Amount,
    fetch_inputs: &mut I,
    fetch_change: &mut C,
    max_tx_size: i64,
) -> Result<AuthoredTx, Error>
where
    I: InputSource + ?Sized,
    C: ChangeSource + ?Sized,
{
    let target_amount = sum_output_values(&outputs);
    let change_script_size = fetch_change.script_size();
    let mut max_signed_size = estimate_serialize_size(
        &[REDEEM_P2PKH_SIG_SCRIPT_SIZE],
        &outputs,
        change_script_size,
    );
    let target_fee = fee_for_serialize_size(relay_fee_per_kb, max_signed_size).0;
    let mut target = target_amount.wrapping_add(target_fee);

    loop {
        let detail = fetch_inputs
            .fetch(Amount(target))
            .map_err(|err| e!(OP, err))?;

        if detail.amount.0 < target {
            return Err(e!(OP, Kind::InsufficientBalance));
        }

        max_signed_size =
            estimate_serialize_size(&detail.redeem_script_sizes, &outputs, change_script_size);
        let max_required_fee = fee_for_serialize_size(relay_fee_per_kb, max_signed_size).0;
        let remaining_amount = detail.amount.0.wrapping_sub(target_amount);
        if remaining_amount < max_required_fee {
            let retry = target_amount.wrapping_add(max_required_fee);
            if retry == target {
                // Go asks for this target again, and forever while the
                // source answers as it did (QK-0744, QK-0753).
                return Err(e!(OP, Kind::InsufficientBalance));
            }
            target = retry;
            continue;
        }

        if max_signed_size > max_tx_size {
            return Err(e!(Kind::Invalid, "signed tx size exceeds allowed maximum"));
        }

        let InputDetail {
            amount,
            inputs,
            scripts,
            redeem_script_sizes,
        } = detail;
        let mut tx = MsgTx {
            ser_type: TxSerializeType::Full,
            version: GENERATED_TX_VERSION,
            tx_in: inputs,
            tx_out: outputs,
            lock_time: 0,
            expiry: 0,
        };
        let mut change_index = -1i64;
        let change_amount = remaining_amount.wrapping_sub(max_required_fee);
        if change_amount != 0
            && !is_dust_amount(Amount(change_amount), change_script_size, relay_fee_per_kb)
        {
            let (change_script, change_script_version) =
                fetch_change.script().map_err(|err| e!(OP, err))?;
            if change_script.len() > MAX_SCRIPT_ELEMENT_SIZE {
                return Err(e!(
                    Kind::Invalid,
                    "script size exceed maximum bytes pushable to the stack"
                ));
            }
            // A slice length is far below 2^63: the cast is exact.
            change_index = tx.tx_out.len() as i64;
            tx.tx_out.push(TxOut {
                value: change_amount,
                version: change_script_version,
                pk_script: change_script,
            });
        } else {
            max_signed_size = estimate_serialize_size(&redeem_script_sizes, &tx.tx_out, 0);
        }
        return Ok(AuthoredTx {
            tx,
            prev_scripts: scripts,
            total_input: amount,
            change_index,
            estimated_signed_serialize_size: max_signed_size,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Change(i64);

    impl ChangeSource for Change {
        fn script(&mut self) -> Result<(Vec<u8>, u16), AnyError> {
            Ok((vec![0; 25], 0))
        }
        fn script_size(&self) -> i64 {
            self.0
        }
    }

    fn one_input(amount: i64) -> InputDetail {
        InputDetail {
            amount: Amount(amount),
            inputs: vec![TxIn::default()],
            scripts: vec![Vec::new()],
            redeem_script_sizes: vec![REDEEM_P2PKH_SIG_SCRIPT_SIZE],
        }
    }

    #[test]
    fn size_limit_error_has_no_op() {
        let mut source = |_: Amount| Ok(one_input(100_000_000));
        let err = new_unsigned_transaction(
            vec![TxOut {
                value: 1_000,
                version: 0,
                pk_script: vec![0; 25],
            }],
            Amount(1_000),
            &mut source,
            &mut Change(25),
            10,
        )
        .unwrap_err();
        assert_eq!(
            err.render(":: "),
            "invalid operation: signed tx size exceeds allowed maximum"
        );
        assert!(err.op.is_empty());
    }

    #[test]
    fn source_error_is_wrapped() {
        let mut source = |_: Amount| -> Result<InputDetail, AnyError> {
            Err(e!(Kind::Locked, "wallet is locked").into())
        };
        let err = new_unsigned_transaction(
            Vec::new(),
            Amount(1_000),
            &mut source,
            &mut Change(25),
            100_000,
        )
        .unwrap_err();
        assert_eq!(
            err.render(":: "),
            "txauthor.NewUnsignedTransaction: wallet locked: wallet is locked"
        );
        assert!(err.is_kind(Kind::Locked));
    }

    /// The most calls the tests' sources answer: a loop that does not end
    /// fails on the source's error instead of hanging.
    const TEST_CALL_LIMIT: usize = 64;

    /// Authors one output of `total` at 10,000 atoms/kB, answering every
    /// target with `answer(target)` up to [`TEST_CALL_LIMIT`] calls, and
    /// returns the error and the targets asked for.
    fn author_from(total: i64, mut answer: impl FnMut(i64) -> InputDetail) -> (Error, Vec<i64>) {
        let mut calls = Vec::new();
        let mut source = |target: Amount| -> Result<InputDetail, AnyError> {
            calls.push(target.0);
            if calls.len() > TEST_CALL_LIMIT {
                return Err(purse_errors::new("test source: call limit reached"));
            }
            Ok(answer(target.0))
        };
        let err = new_unsigned_transaction(
            vec![TxOut {
                value: total,
                version: 0,
                pk_script: vec![0; 25],
            }],
            Amount(10_000),
            &mut source,
            &mut Change(25),
            100_000,
        )
        .unwrap_err();
        (err, calls)
    }

    /// As [`author_from`], from a source that answers with nothing.
    fn from_empty_source(total: i64) -> (Error, Vec<i64>) {
        author_from(total, |_| InputDetail::default())
    }

    /// One input of `amount` whose worst-case signature script is 1,000
    /// bytes: its fee at 10,000 atoms/kB, 11,470 atoms with the output and
    /// change of [`author_from`], is far above a P2PKH input's.
    fn big_input(amount: i64) -> InputDetail {
        InputDetail {
            redeem_script_sizes: vec![1_000],
            ..one_input(amount)
        }
    }

    #[test]
    fn endless_loop_is_insufficient_balance() {
        // QK-0744: the target wraps. Go asks for the second target forever
        // (the oracle's answers in tests/data); the crate asks twice.
        // QK-0753: the remaining amount wraps, the target does not.
        for (total, want) in [
            (i64::MAX, [i64::MIN + 2_529, i64::MIN + 869]),
            (i64::MIN, [i64::MIN + 2_530, i64::MIN + 870]),
        ] {
            let (err, calls) = from_empty_source(total);
            assert_eq!(
                err.render(":: "),
                "txauthor.NewUnsignedTransaction: insufficient balance"
            );
            assert!(err.is_kind(Kind::InsufficientBalance));
            assert_eq!(calls, want, "{total}");
        }
    }

    #[test]
    fn stall_with_inputs_covering_the_outputs_is_insufficient_balance() {
        // QK-0744 with a source that answers: the first target does not
        // wrap and the input covers the output total, so the remaining
        // amount (here 3,000 atoms, and 11,469) is not negative; but it is
        // below the input's own fee of 11,470, and the retry for that fee
        // wraps (to i64::MIN in the second case). The same input answers
        // the wrapped target, so Go asks for it forever (the oracle's
        // answers in tests/data); the crate asks twice. A check on a
        // negative remaining amount alone misses this.
        for total in [i64::MAX - 3_000, i64::MAX - 11_469] {
            let (err, calls) = author_from(total, |_| big_input(i64::MAX));
            assert_eq!(
                err.render(":: "),
                "txauthor.NewUnsignedTransaction: insufficient balance"
            );
            assert!(err.is_kind(Kind::InsufficientBalance));
            assert_eq!(
                calls,
                [total.wrapping_add(2_530), total.wrapping_add(11_470)],
                "{total}"
            );
            assert!(calls[0] > 0 && calls[1] < 0, "{calls:?}");
        }
        // One atom more of output room and the input pays its fee exactly:
        // no retry, and a transaction without change, as in Go.
        let mut source = |_: Amount| Ok(big_input(i64::MAX));
        let atx = new_unsigned_transaction(
            vec![TxOut {
                value: i64::MAX - 11_470,
                version: 0,
                pk_script: vec![0; 25],
            }],
            Amount(10_000),
            &mut source,
            &mut Change(25),
            100_000,
        )
        .expect("the input pays its fee");
        assert_eq!(atx.change_index, -1);
        assert_eq!(atx.total_input, Amount(i64::MAX));
    }

    #[test]
    fn longer_cycles_keep_go_s_loop() {
        // Only a repeat of the target just asked for is refused. A source
        // answering each target afresh, nothing for the first target and a
        // worthless input for any other, makes the targets alternate
        // between the output total plus 2,530 and plus 870 atoms, both
        // wrapped: Go asks forever, and so does the crate, until the test
        // source's limit. No cumulative source can answer so.
        let total = i64::MAX - 100;
        let first = total.wrapping_add(2_530);
        let (err, calls) = author_from(total, |target| {
            if target == first {
                InputDetail::default()
            } else {
                InputDetail {
                    redeem_script_sizes: vec![REDEEM_P2PKH_SIG_SCRIPT_SIZE],
                    ..one_input(0)
                }
            }
        });
        assert_eq!(
            err.render(":: "),
            "txauthor.NewUnsignedTransaction: test source: call limit reached"
        );
        assert_eq!(calls.len(), TEST_CALL_LIMIT + 1);
        let second = total.wrapping_add(870);
        assert!(
            calls
                .iter()
                .enumerate()
                .all(|(i, &t)| t == if i % 2 == 0 { first } else { second }),
            "{calls:?}"
        );
    }

    #[test]
    fn wrapped_first_target_is_retried_as_in_go() {
        // The first target wraps, the second (the fee for no inputs) does
        // not: Go asks for i64::MAX and finds the balance short, as here.
        let (err, calls) = from_empty_source(i64::MAX - 870);
        assert!(err.is_kind(Kind::InsufficientBalance));
        assert_eq!(calls, [i64::MIN + 1_659, i64::MAX]);
    }

    #[test]
    fn dust_sweep_has_no_outputs() {
        // A sweep (no outputs) whose remainder after the fee is dust: the
        // transaction has no outputs and no change (QK-0260).
        let fee = fee_for_serialize_size(
            Amount(10_000),
            estimate_serialize_size(&[REDEEM_P2PKH_SIG_SCRIPT_SIZE], &[], 25),
        );
        let mut source = move |_: Amount| Ok(one_input(fee.0 + 100));
        let atx = new_unsigned_transaction(
            Vec::new(),
            Amount(10_000),
            &mut source,
            &mut Change(25),
            100_000,
        )
        .unwrap();
        assert!(atx.tx.tx_out.is_empty());
        assert_eq!(atx.change_index, -1);
    }
}
