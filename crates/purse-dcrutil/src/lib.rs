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
//! # Timing
//!
//! What of a WIF's handling runs in constant time, and what does not. The
//! full account, with the evidence and its limits, is the note on the
//! base58 codec in this crate's `src/base58.rs` (its "Timing" section),
//! after ADR-0012's addendum of 2026-11-21 and its note of 2026-11-22.
//!
//! Constant time, on x86_64 with the pinned compiler:
//!
//! - the base58 lookups of [`Wif::decode`] and [`Wif::string`], from a
//!   character to its digit and from a digit to its character, which use
//!   no comparison, no table and no read of the alphabet (decred/base58,
//!   which Go's dcrutil uses, looks both up in tables indexed by the
//!   secret; the outputs are the same). That is a property of the
//!   compiler's output, not of the source: a disassembly check of the
//!   test binary finds no conditional branch and no memory access that
//!   depends on the character or the digit, and no division instruction
//!   in the big-number arithmetic, at opt-levels 1, 2, 3, "s" and "z" (in
//!   CI on every push), and dudect's timing test finds no difference at
//!   opt-levels 3 and "s" (in CI when a path that can change its result
//!   changes, and nightly). No other architecture or compiler was
//!   examined, and a consumer that builds this crate another way gets
//!   code these checks did not see;
//! - an Ed25519 key's range checks and public key ([`Wif::new`], and
//!   [`Wif::decode`] of signature type 1), but for which error, if any, is
//!   returned;
//! - a secp256k1 key's public key, `dcroxide-dcrec`'s multiplication by
//!   the generator, constant time on the reduced key as that crate
//!   documents it (whether the key reduces to zero is not hidden).
//!
//! Not constant time:
//!
//! - the string's and the key's lengths, which are public;
//! - whether a string holds a character outside the alphabet, an early
//!   exit whose result, [`WifError::MalformedPrivateKey`], says the same;
//! - the count of leading `1` characters, which encode leading zero bytes
//!   (none in a WIF of the four networks, whose network bytes are not
//!   zero);
//! - the base58 big-number arithmetic, whose work follows the magnitude
//!   of each prefix of the value: set by the network and signature type
//!   bytes, which are public, and by the private key only through a carry
//!   near a boundary;
//! - the rest of a decode or an encode, not examined for timing: the
//!   BLAKE-256 checksum over the key (`dcroxide-crypto`), the comparison
//!   of a decoded checksum with it (a slice comparison), and the network
//!   and signature type checks, which read public bytes.
//!
//! # Panics in Go
//!
//! No input makes this crate panic. Go's `Amount.Format` panics for units
//! so extreme that `FormatFloat` cannot allocate its buffer; the port
//! returns the text instead ([`Amount::format`]).

pub mod amount;
mod base58;
#[cfg(test)]
mod ct_check;
mod gomath;
pub mod gosort;
pub mod txsort;
pub mod wif;

pub use amount::{
    ATOMS_PER_CENT, ATOMS_PER_COIN, Amount, AmountUnit, InvalidCoinAmount, MAX_AMOUNT, new_amount,
};
pub use wif::{SignatureType, Wif, WifError};
