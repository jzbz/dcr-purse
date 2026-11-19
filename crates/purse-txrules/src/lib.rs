// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2025 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's transaction rules, ported from the parity pin: the worst-case
//! size arithmetic of `wallet/txsizes` and the fee, dust, high-fee and VSP
//! fee rules of `wallet/txrules` (spec 10 §2.1, §2.2, §4.1 to §4.4).
//!
//! | Go | Rust |
//! |---|---|
//! | `txsizes.RedeemP2PKHSigScriptSize` and the other size constants | [`txsizes`] constants |
//! | `txsizes.EstimateSerializeSize` | [`estimate_serialize_size`] |
//! | `txsizes.EstimateSerializeSizeFromScriptSizes` | [`estimate_serialize_size_from_script_sizes`] |
//! | `txsizes.EstimateInputSize`, `EstimateOutputSize` | [`estimate_input_size`], [`estimate_output_size`] |
//! | `txrules.DefaultRelayFeePerKb` | [`DEFAULT_RELAY_FEE_PER_KB`] |
//! | `txrules.StakeSubScriptType` | [`stake_sub_script_type`] |
//! | `txrules.IsDustAmount`, `IsDustOutput`, `CheckOutput` | [`is_dust_amount`], [`is_dust_output`], [`check_output`] |
//! | `txrules.FeeForSerializeSize` | [`fee_for_serialize_size`] |
//! | `txrules.PaysHighFees`, `TxPaysHighFees` | [`pays_high_fees`], [`tx_pays_high_fees`] |
//! | `txrules.ValidPoolFeeRate` | [`valid_pool_fee_rate`] |
//! | `txrules.StakePoolTicketFee` | [`stake_pool_ticket_fee`], [`StakePoolFees::ticket_fee`] |
//!
//! # Integer semantics
//!
//! Sizes and script lengths are Go `int`, which is 64 bits on every target
//! dcrwallet ships for, so they are `i64` here, and every sum and product
//! wraps as Go's does. A negative script size reaches the varint width as
//! `uint64(size)`, that is 9 bytes, exactly as in Go. Amounts are
//! [`Amount`](purse_dcrutil::Amount) (int64 atoms) with Go's wrapping
//! multiply and truncating divide. Nothing here is checked arithmetic: the
//! answers are Go's for every input, including the ones that overflow.
//!
//! # Divergences
//!
//! Each is tested and recorded in `docs/PARITY.md` with its QUIRKS entry:
//!
//! - [`is_dust_amount`] answers `true` (dust) where Go divides by zero, the
//!   one script size, -184, whose estimated cost to the network is 0 bytes.
//!   No dcrwallet call site passes a negative size (QK-0741).
//! - [`stake_pool_ticket_fee`] returns an error where Go's big-integer
//!   division panics, when the ticket price plus the adjusted vote subsidy
//!   is 0 (ADR-0012 Rule 2(c); the price comes from the network backend;
//!   QK-0742). It also computes the subsidy from the parameters it is given
//!   on every call, where Go keeps one process-wide subsidy cache built from
//!   the parameters of its first call (QK-0269).
//! - Where `int64(math.Floor(poolFee * 100))` depends on the CPU (a NaN or
//!   out-of-range percentage), the VSP fee is amd64's on every target
//!   (ADR-0012 Rule 9(a); QK-0743).
//!
//! # What is not here
//!
//! dcroxide's `TxSerializeType` has the three serialization types; Go's
//! `MsgTx.SerializeSize` answers 0 for any other `SerType` value, which only
//! Go code can set, so [`pays_high_fees`] cannot be given one (ADR-0012
//! Rule 4). Likewise [`stake_sub_script_type`] takes dcroxide's
//! `ScriptType` enum, which holds the 21 values `stdscript` defines and no
//! other.

mod gofloat;
pub mod poolfees;
pub mod rules;
pub mod txsizes;

pub use poolfees::{StakePoolFees, stake_pool_ticket_fee, valid_pool_fee_rate};
pub use rules::{
    DEFAULT_RELAY_FEE_PER_KB, check_output, fee_for_serialize_size, is_dust_amount, is_dust_output,
    pays_high_fees, stake_sub_script_type, tx_pays_high_fees,
};
pub use txsizes::{
    estimate_input_size, estimate_output_size, estimate_serialize_size,
    estimate_serialize_size_from_script_sizes,
};
