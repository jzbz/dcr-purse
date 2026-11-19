// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2021 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The relay fee, dust, output and high-fee rules (dcrwallet
//! `wallet/txrules/rules.go`, spec 10 §4.2 and §4.3).

use dcroxide_txscript::is_unspendable;
use dcroxide_txscript::stdscript::{ScriptType, is_null_data_script_v0};
use dcroxide_wire::{MsgTx, TxOut};
use purse_dcrutil::{Amount, MAX_AMOUNT};
use purse_errors::{Error, Kind, e, errorf};

use crate::txsizes::varint_size;

/// The default minimum relay fee policy of a mempool, in atoms per 1000
/// bytes (Go `DefaultRelayFeePerKb`, `0.0001 * 1e8`).
pub const DEFAULT_RELAY_FEE_PER_KB: Amount = Amount(10_000);

/// The rate [`pays_high_fees`] compares against: 1000 times the default.
const HIGH_FEE_RATE: Amount = Amount(1000 * DEFAULT_RELAY_FEE_PER_KB.0);

/// Maps the stake-tagged P2PKH script types (submission, change, gen,
/// revocation, treasury gen) to [`ScriptType::PubKeyHashEcdsaSecp256k1`]
/// and the stake-tagged P2SH types to [`ScriptType::ScriptHash`], with
/// `true`; any other type comes back unchanged with `false` (Go
/// `StakeSubScriptType`).
pub fn stake_sub_script_type(script_type: ScriptType) -> (ScriptType, bool) {
    use ScriptType::*;
    match script_type {
        StakeSubmissionPubKeyHash
        | StakeChangePubKeyHash
        | StakeGenPubKeyHash
        | StakeRevocationPubKeyHash
        | TreasuryGenPubKeyHash => (PubKeyHashEcdsaSecp256k1, true),
        StakeSubmissionScriptHash
        | StakeChangeScriptHash
        | StakeGenScriptHash
        | StakeRevocationScriptHash
        | TreasuryGenScriptHash => (ScriptHash, true),
        other => (other, false),
    }
}

/// Whether an output of `amount` with a script of `script_size` bytes is
/// dust at `relay_fee_per_kb` (Go `IsDustAmount`).
///
/// The cost to the network is the output's serialized size plus 165, the
/// average size of an input redeeming compressed P2PKH; the output is dust
/// when `amount * 1000 / (3 * cost)` falls below the rate. The multiply
/// wraps and the division truncates, as Go's int64 arithmetic does.
///
/// Divergence: for the one script size whose cost wraps to 0 (-184, whose
/// varint takes 9 bytes), Go divides by zero and panics; this answers
/// `true`, the answer that drops the output. No dcrwallet call site passes
/// a negative size (QK-0741).
#[inline]
pub fn is_dust_amount(amount: Amount, script_size: i64, relay_fee_per_kb: Amount) -> bool {
    let total_size = (8i64 + 2)
        .wrapping_add(varint_size(script_size))
        .wrapping_add(script_size)
        .wrapping_add(165);
    let divisor = total_size.wrapping_mul(3);
    if divisor == 0 {
        return true;
    }
    // wrapping_div: i64::MIN / -1 is i64::MIN, as in Go.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "the divisor was checked nonzero just above"
    )]
    let quotient = amount.0.wrapping_mul(1000).wrapping_div(divisor);
    quotient < relay_fee_per_kb.0
}

/// `stdscript.IsNullDataScript(version, script)`: only version 0 scripts
/// are null data. dcroxide ports the version 0 test alone.
#[inline]
fn is_null_data_script(version: u16, script: &[u8]) -> bool {
    version == 0 && is_null_data_script_v0(script)
}

/// Whether `output` is dust at `relay_fee_per_kb` (Go `IsDustOutput`):
/// a null-data script never is; any other unspendable output (zero value,
/// a script over the size limit, one starting with `OP_RETURN`, or one
/// that does not parse as version 0) always is; otherwise
/// [`is_dust_amount`] decides.
#[inline]
pub fn is_dust_output(output: &TxOut, relay_fee_per_kb: Amount) -> bool {
    if is_null_data_script(output.version, &output.pk_script) {
        return false;
    }
    if is_unspendable(output.value, &output.pk_script) {
        return true;
    }
    // A script's length is below isize::MAX: the cast is exact.
    is_dust_amount(
        Amount(output.value),
        output.pk_script.len() as i64,
        relay_fee_per_kb,
    )
}

/// The consensus and policy checks dcrwallet applies to each output it is
/// asked to pay (Go `CheckOutput`), in Go's order:
///
/// | Condition | Kind | Text |
/// |---|---|---|
/// | value below 0 | [`Kind::Invalid`] | `transaction output amount is negative` |
/// | value above `MaxAmount` | [`Kind::Invalid`] | `transaction output amount exceeds maximum value` |
/// | [`is_dust_output`] | [`Kind::Policy`] | `transaction output is dust` |
pub fn check_output(output: &TxOut, relay_fee_per_kb: Amount) -> Result<(), Error> {
    if output.value < 0 {
        return Err(e!(Kind::Invalid, "transaction output amount is negative"));
    }
    if output.value > MAX_AMOUNT {
        return Err(e!(
            Kind::Invalid,
            "transaction output amount exceeds maximum value"
        ));
    }
    if is_dust_output(output, relay_fee_per_kb) {
        return Err(e!(Kind::Policy, "transaction output is dust"));
    }
    Ok(())
}

/// The fee for a transaction of `tx_serialize_size` bytes at
/// `relay_fee_per_kb` (Go `FeeForSerializeSize`):
/// `relay_fee_per_kb * size / 1000` with int64 wrapping multiply and
/// truncating divide; a zero result at a positive rate becomes the whole
/// per-kB rate; a negative result or one above `MaxAmount` becomes
/// `MaxAmount`.
#[inline]
pub fn fee_for_serialize_size(relay_fee_per_kb: Amount, tx_serialize_size: i64) -> Amount {
    // The divisor is 1000, so the division can neither fail nor wrap.
    let mut fee = relay_fee_per_kb
        .0
        .wrapping_mul(tx_serialize_size)
        .wrapping_div(1000);
    if fee == 0 && relay_fee_per_kb.0 > 0 {
        fee = relay_fee_per_kb.0;
    }
    if !(0..=MAX_AMOUNT).contains(&fee) {
        fee = MAX_AMOUNT;
    }
    Amount(fee)
}

/// The sum of the outputs' values (Go's unexported `sumOutputValues`),
/// wrapping as Go's does.
#[inline]
fn sum_output_values(outputs: &[TxOut]) -> i64 {
    outputs
        .iter()
        .fold(0i64, |sum, out| sum.wrapping_add(out.value))
}

/// Whether `tx` pays a fee above 1000 times the default relay fee rate
/// (10,000,000 atoms per kB) for its serialized size as it stands, given
/// that its inputs total `total_input` (Go `PaysHighFees`).
///
/// A fee of zero or less answers `false` ("impossible to determine"). The
/// size is `tx.serialize_size()` for the transaction's own serialization
/// type, signed or not: dcrwallet calls this on unsigned transactions too
/// (spec 10 §9 item 12).
pub fn pays_high_fees(total_input: Amount, tx: &MsgTx) -> bool {
    let fee = total_input.0.wrapping_sub(sum_output_values(&tx.tx_out));
    if fee <= 0 {
        return false;
    }
    // A serialized size is bounded by the transaction's allocation: the
    // cast is exact.
    let max_fee = fee_for_serialize_size(HIGH_FEE_RATE, tx.serialize_size() as i64);
    fee > max_fee.0
}

/// [`pays_high_fees`] with the input total summed from the inputs'
/// `value_in` (Go `TxPaysHighFees`). An input with a negative `value_in`
/// (the null value) is an error of kind [`Kind::Invalid`]:
/// `transaction input <i> does not specify the input value`.
pub fn tx_pays_high_fees(tx: &MsgTx) -> Result<bool, Error> {
    let mut input = 0i64;
    for (i, tx_in) in tx.tx_in.iter().enumerate() {
        if tx_in.value_in < 0 {
            let err = errorf!("transaction input {i} does not specify the input value");
            return Err(e!(Kind::Invalid, err));
        }
        input = input.wrapping_add(tx_in.value_in);
    }
    Ok(pays_high_fees(Amount(input), tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dust_threshold_for_p2pkh() {
        // ceil(rate * 603 / 1000): 6030 at the default rate, 603 at 1000,
        // 1538 at 2550 (spec 10 §4.3).
        for (rate, smallest) in [(10_000, 6030), (1000, 603), (2550, 1538)] {
            assert!(is_dust_amount(Amount(smallest - 1), 25, Amount(rate)));
            assert!(!is_dust_amount(Amount(smallest), 25, Amount(rate)));
        }
    }

    #[test]
    fn zero_cost_script_size_is_dust() {
        // 10 + 9 + (-184) + 165 == 0: Go divides by zero here.
        assert!(is_dust_amount(Amount(i64::MAX), -184, Amount(i64::MIN)));
        assert!(is_dust_amount(Amount(0), -184, Amount(0)));
    }

    #[test]
    fn fee_lift_and_clamp() {
        assert_eq!(fee_for_serialize_size(Amount(999), 1), Amount(999));
        assert_eq!(fee_for_serialize_size(Amount(0), 1000), Amount(0));
        assert_eq!(fee_for_serialize_size(Amount(-999), 1), Amount(0));
        assert_eq!(fee_for_serialize_size(Amount(-1000), 1), Amount(MAX_AMOUNT));
        assert_eq!(
            fee_for_serialize_size(Amount(-1000), 1000),
            Amount(MAX_AMOUNT)
        );
        assert_eq!(
            fee_for_serialize_size(Amount(i64::MAX), 2),
            Amount(MAX_AMOUNT)
        );
        assert_eq!(fee_for_serialize_size(Amount(10_000), 419), Amount(4190));
    }

    #[test]
    fn error_texts() {
        let out = |value| TxOut {
            value,
            version: 0,
            pk_script: vec![
                0x76, 0xa9, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x88,
                0xac,
            ],
        };
        let render = |r: Result<(), Error>| r.unwrap_err().render(":: ");
        assert_eq!(
            render(check_output(&out(-1), DEFAULT_RELAY_FEE_PER_KB)),
            "invalid operation: transaction output amount is negative"
        );
        assert_eq!(
            render(check_output(&out(MAX_AMOUNT + 1), DEFAULT_RELAY_FEE_PER_KB)),
            "invalid operation: transaction output amount exceeds maximum value"
        );
        assert_eq!(
            render(check_output(&out(6029), DEFAULT_RELAY_FEE_PER_KB)),
            "policy violation: transaction output is dust"
        );
        assert!(check_output(&out(6030), DEFAULT_RELAY_FEE_PER_KB).is_ok());
        let mut tx = MsgTx::default();
        tx.tx_in.push(dcroxide_wire::TxIn {
            value_in: 5,
            ..Default::default()
        });
        tx.tx_in.push(dcroxide_wire::TxIn {
            value_in: -1,
            ..Default::default()
        });
        assert_eq!(
            tx_pays_high_fees(&tx).unwrap_err().render(":: "),
            "invalid operation: transaction input 1 does not specify the input value"
        );
    }
}
