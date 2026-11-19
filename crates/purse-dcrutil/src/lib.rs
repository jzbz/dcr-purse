// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2024 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The three parts of dcrd's `dcrutil` module that dcrwallet uses, ported
//! from v4.0.3, the version dcrwallet links at the parity pin:
//!
//! - [`Amount`] (`amount.go`, `const.go`): atoms, [`new_amount`] from a
//!   float in coins with Go's exact rounding, [`Amount::to_coin`] and the
//!   text of [`Amount::format`] and `Display`, which dcrwallet's RPC
//!   results and messages carry. Where Go's own answer depends on the CPU
//!   (the fused multiply-add of arm64, riscv64 and amd64 from GOAMD64 v3,
//!   and the out-of-range and NaN conversions, which differ between amd64,
//!   arm64, riscv64, 386 and arm), this crate gives the answer of amd64 at
//!   the default GOAMD64 on every target: see the [`amount`] module.
//! - [`Wif`] (`wif.go`): private keys in Decred's Wallet Import Format,
//!   with every error of `NewWIF` and `DecodeWIF` and their texts.
//! - [`txsort`]: the canonical input and output order, with Go's
//!   (unstable but deterministic) `sort.Sort` ported ([`gosort`]) so that
//!   inputs that compare equal end up where Go puts them. dcrwallet's own
//!   packages do not import it; the mixing client it links
//!   (`dcrd/mixing@v0.7.5/mixclient`) sorts every coinjoin with it.
//!
//! The rest of dcrutil (`appdata.go`, `block.go`, `tx.go`, `hash160.go`,
//! `bitflags.go`) is not here.
//!
//! # Left out on purpose
//!
//! - `AmountSorter`: [`Amount`] derives `Ord`, and amounts that compare
//!   equal are identical, so any sort gives Go's result; no dcrwallet code
//!   uses it outside tests.
//! - The `ErrMalformedPrivateKey` and `ErrChecksumMismatch` values and the
//!   `ErrWrongWIFNetwork` type are the [`WifError`] variants.
//!
//! # Secrets
//!
//! A [`Wif`] holds a private key: it is wiped on drop, `Debug` redacts it,
//! there is no `Display`, the string form comes only from [`Wif::string`]
//! as a [`zeroize::Zeroizing`] string, and no error quotes the input.
//!
//! # Panics in Go
//!
//! No input makes this crate panic. Go's `Amount.Format` panics for units
//! so extreme that `FormatFloat` cannot allocate its buffer; the port
//! returns the text instead ([`Amount::format`]).

pub mod amount;
mod base58;
mod gomath;
pub mod gosort;
pub mod txsort;
pub mod wif;

pub use amount::{
    ATOMS_PER_CENT, ATOMS_PER_COIN, Amount, AmountUnit, InvalidCoinAmount, MAX_AMOUNT, new_amount,
};
pub use wif::{SignatureType, Wif, WifError};
