// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2025 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Worst-case serialized sizes of scripts, inputs, outputs and whole
//! transactions (dcrwallet `wallet/txsizes`, spec 10 §4.1).
//!
//! The estimates count the full serialization (prefix and witness), so the
//! input count is counted twice: once in the prefix and once in the witness.
//! Given exact signature-script sizes the estimate equals the serialized
//! size of the signed transaction.
//!
//! Sizes are Go `int` (`i64`) and every sum wraps as Go's does; see the
//! crate documentation.

use dcroxide_wire::{TxOut, var_int_serialize_size};

/// The worst-case size of a signature script redeeming a compressed P2PK
/// output: `OP_DATA_73` and a 72-byte DER signature with its sighash byte.
pub const REDEEM_P2PK_SIG_SCRIPT_SIZE: i64 = 1 + 73;

/// The worst-case size of a signature script redeeming a compressed P2PKH
/// output: the signature push of [`REDEEM_P2PK_SIG_SCRIPT_SIZE`], then
/// `OP_DATA_33` and the 33-byte compressed public key.
pub const REDEEM_P2PKH_SIG_SCRIPT_SIZE: i64 = 1 + 73 + 1 + 33;

/// The worst-case size of a signature script redeeming a P2SH output whose
/// redeem script is `<compressed pubkey> OP_CHECKSIG`: the signature push,
/// `OP_DATA_35`, and the 35-byte redeem script.
pub const REDEEM_P2SH_SIG_SCRIPT_SIZE: i64 = 1 + 73 + 1 + 1 + 33 + 1;

/// The worst-case serialized size of an input redeeming a compressed P2PKH
/// output: outpoint (32 + 4 + 1), value 8, block height 4, block index 4,
/// the script length varint 1, the [`REDEEM_P2PKH_SIG_SCRIPT_SIZE`]-byte
/// script, and sequence 4. (Go's comment says a 107-byte script; the
/// constant is built from the 108-byte one, spec 10 §9 item 55.)
pub const REDEEM_P2PKH_INPUT_SIZE: i64 =
    32 + 4 + 1 + 8 + 4 + 4 + 1 + REDEEM_P2PKH_SIG_SCRIPT_SIZE + 4;

/// The size of a P2PKH output script:
/// `OP_DUP OP_HASH160 OP_DATA_20 <20 bytes> OP_EQUALVERIFY OP_CHECKSIG`.
pub const P2PKH_PK_SCRIPT_SIZE: i64 = 1 + 1 + 1 + 20 + 1 + 1;

/// The size of the stake-change P2PKH script (`OP_SSTXCHANGE` before a
/// P2PKH script) that pays change from a treasury add (`OP_TADD`).
pub const P2PKH_PK_TREASURY_SCRIPT_SIZE: i64 = 1 + 1 + 1 + 1 + 20 + 1 + 1;

/// The size of a P2SH output script: `OP_HASH160 OP_DATA_20 <20 bytes>
/// OP_EQUAL`.
pub const P2SH_PK_SCRIPT_SIZE: i64 = 1 + 1 + 20 + 1;

/// The size of a ticket commitment script: `OP_RETURN OP_DATA_30`, a
/// 20-byte hash, an 8-byte amount and the 2-byte fee limits.
pub const TICKET_COMMITMENT_SCRIPT_SIZE: i64 = 1 + 1 + 20 + 8 + 2;

/// The serialized size of an output with a P2PKH script: value 8, script
/// version 2, the length varint 1 and the 25-byte script.
pub const P2PKH_OUTPUT_SIZE: i64 = 8 + 2 + 1 + 25;

/// Go's `TSPENDInputSize`, which despite its name is a signature-script
/// size: the signature push, the public key push and `OP_TSPEND`.
pub const TSPEND_INPUT_SIZE: i64 = 1 + 73 + 1 + 33 + 1;

/// `wire.VarIntSerializeSize(uint64(n))` for a Go `int`.
#[inline]
pub(crate) fn varint_size(n: i64) -> i64 {
    // At most 9: the cast is exact.
    var_int_serialize_size(n as u64) as i64
}

/// `wire.VarIntSerializeSize(uint64(len))` for a slice length.
#[inline]
fn varint_size_of_len(len: usize) -> i64 {
    var_int_serialize_size(len as u64) as i64
}

/// The sum of the outputs' serialized sizes (Go's unexported
/// `sumOutputSerializeSizes`), wrapping as Go's `int` sum does.
#[inline]
fn sum_output_serialize_sizes(outputs: &[TxOut]) -> i64 {
    outputs.iter().fold(0i64, |sum, out| {
        // A serialized size is bounded by the output's allocation, far
        // below 2^63: the cast is exact.
        sum.wrapping_add(out.serialize_size() as i64)
    })
}

/// The worst-case serialized size of a signed transaction spending one
/// input per entry of `script_sizes` (each that input's signature-script
/// size) and paying `tx_outs`, plus one change output with a script of
/// `change_script_size` bytes when that is greater than 0 (Go
/// `EstimateSerializeSize`).
pub fn estimate_serialize_size(
    script_sizes: &[i64],
    tx_outs: &[TxOut],
    change_script_size: i64,
) -> i64 {
    let tx_ins_size = script_sizes.iter().fold(0i64, |sum, &size| {
        sum.wrapping_add(estimate_input_size(size))
    });
    let (change_size, output_count) = change(tx_outs.len(), change_script_size);
    // 12 bytes for the version, lock time and expiry.
    12i64
        .wrapping_add(varint_size_of_len(script_sizes.len()).wrapping_mul(2))
        .wrapping_add(varint_size_of_len(output_count))
        .wrapping_add(tx_ins_size)
        .wrapping_add(sum_output_serialize_sizes(tx_outs))
        .wrapping_add(change_size)
}

/// As [`estimate_serialize_size`], with the outputs given by their script
/// sizes (Go `EstimateSerializeSizeFromScriptSizes`).
pub fn estimate_serialize_size_from_script_sizes(
    input_sizes: &[i64],
    output_sizes: &[i64],
    change_script_size: i64,
) -> i64 {
    let tx_ins_size = input_sizes.iter().fold(0i64, |sum, &size| {
        sum.wrapping_add(estimate_input_size(size))
    });
    let tx_outs_size = output_sizes.iter().fold(0i64, |sum, &size| {
        sum.wrapping_add(estimate_output_size(size))
    });
    let (change_size, output_count) = change(output_sizes.len(), change_script_size);
    12i64
        .wrapping_add(varint_size_of_len(input_sizes.len()).wrapping_mul(2))
        .wrapping_add(varint_size_of_len(output_count))
        .wrapping_add(tx_ins_size)
        .wrapping_add(tx_outs_size)
        .wrapping_add(change_size)
}

/// The change output's size and the output count including it.
#[inline]
fn change(outputs: usize, change_script_size: i64) -> (i64, usize) {
    if change_script_size > 0 {
        // A slice length is below isize::MAX, so adding one cannot wrap.
        (
            estimate_output_size(change_script_size),
            outputs.wrapping_add(1),
        )
    } else {
        (0, outputs)
    }
}

/// The worst-case serialized size of an input whose signature script is
/// `script_size` bytes (Go `EstimateInputSize`): outpoint 32 + 4 + 1, value
/// 8, block height 4, block index 4, the script length varint, the script,
/// and sequence 4.
#[inline]
pub fn estimate_input_size(script_size: i64) -> i64 {
    (32i64 + 4 + 1 + 8 + 4 + 4)
        .wrapping_add(varint_size(script_size))
        .wrapping_add(script_size)
        .wrapping_add(4)
}

/// The worst-case serialized size of an output whose script is
/// `script_size` bytes (Go `EstimateOutputSize`): value 8, script version
/// 2, the script length varint, and the script.
#[inline]
pub fn estimate_output_size(script_size: i64) -> i64 {
    (8i64 + 2)
        .wrapping_add(varint_size(script_size))
        .wrapping_add(script_size)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, reason = "test code: small constants")]
mod tests {
    use super::*;

    #[test]
    fn constants_are_go_s() {
        assert_eq!(REDEEM_P2PK_SIG_SCRIPT_SIZE, 74);
        assert_eq!(REDEEM_P2PKH_SIG_SCRIPT_SIZE, 108);
        assert_eq!(REDEEM_P2SH_SIG_SCRIPT_SIZE, 110);
        assert_eq!(REDEEM_P2PKH_INPUT_SIZE, 166);
        assert_eq!(P2PKH_PK_SCRIPT_SIZE, 25);
        assert_eq!(P2PKH_PK_TREASURY_SCRIPT_SIZE, 26);
        assert_eq!(P2SH_PK_SCRIPT_SIZE, 23);
        assert_eq!(TICKET_COMMITMENT_SCRIPT_SIZE, 32);
        assert_eq!(P2PKH_OUTPUT_SIZE, 36);
        assert_eq!(TSPEND_INPUT_SIZE, 109);
    }

    #[test]
    fn negative_sizes_take_the_nine_byte_varint() {
        assert_eq!(estimate_input_size(-1), 57 + 9 - 1);
        assert_eq!(estimate_output_size(-1), 10 + 9 - 1);
        assert_eq!(
            estimate_output_size(i64::MIN),
            (19i64).wrapping_add(i64::MIN)
        );
        assert_eq!(
            estimate_input_size(i64::MAX),
            (66i64).wrapping_add(i64::MAX)
        );
    }

    #[test]
    fn non_positive_change_adds_no_output() {
        let with = estimate_serialize_size_from_script_sizes(&[108], &[25], 25);
        assert_eq!(
            estimate_serialize_size_from_script_sizes(&[108], &[25], 0),
            with - 36
        );
        assert_eq!(
            estimate_serialize_size_from_script_sizes(&[108], &[25], -5),
            with - 36
        );
    }
}
