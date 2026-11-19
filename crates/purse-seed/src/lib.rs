// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's seed encodings: a port of its `pgpwordlist` and `walletseed`
//! packages at the parity pin.
//!
//! - [`pgpwordlist`]: the 512-word PGP word list, byte to word by position
//!   parity, and [`pgpwordlist::decode_mnemonics`].
//! - [`walletseed`]: seed generation with an injectable random source, the
//!   seed mnemonic with its one-word double-SHA-256 checksum, and
//!   [`walletseed::decode_user_input`], which takes hex or a mnemonic.
//!
//! Seed material is returned in [`SecretBytes`], [`SecretString`] and
//! [`MnemonicWords`], which zero their buffers on drop and never print their
//! contents.
//!
//! # Errors
//!
//! The fallible functions return [`purse_errors::Error`] (re-exported as
//! [`Error`]), built as dcrwallet builds them with its `errors` package:
//! the same operations ([`pgpwordlist::OP_DECODE_MNEMONICS`],
//! [`walletseed::OP_GENERATE_RANDOM_SEED`],
//! [`walletseed::OP_DECODE_USER_INPUT`]), kinds and nesting. Its text
//! (`Display`, or `render` / `render_to` with an explicit separator) is
//! therefore dcrwallet's byte for byte under either separator, and
//! `kind`, `as_kind`, `is` and `matches` answer as Go's do. The leaves are
//! this crate's [`WordError`] (which quotes a word, possibly a seed word,
//! in wiped memory with a redacted `Debug`) and [`HexError`], the sentinel
//! [`ERR_INVALID_SEED_LEN`], [`walletseed::RandomSourceError`], and the
//! text `checksum mismatch`. [`SeedErrorExt`] adds a wiped rendering, the
//! seed-length test and the leaf. "What is wiped" in the [`error`] module
//! documentation states what is and is not wiped after rendering an error
//! that quotes a word.
//!
//! **`render(sep)` no longer wipes.** This crate's former `Error::render`
//! returned a `Zeroizing<String>`; on [`purse_errors::Error`] the same call
//! still compiles and returns the same text, but as an ordinary `String`
//! that is not wiped. Use [`SeedErrorExt::render_zeroizing`] (or
//! `render_to` into a sink that wipes its memory) for any rendering of an
//! error that may quote a seed word.
//!
//! Since the errors became [`purse_errors::Error`],
//! [`std::error::Error::source`] also returns the chain as Go's `Unwrap`
//! sees it (the nested level, then the leaf) where it used to return
//! `None`; a reporter that prints every link of that chain prints a quoted
//! word once per link (see "What is wiped").
//!
//! # Text input
//!
//! The mnemonic decoders take `&str`, whereas Go's take strings that may hold
//! arbitrary bytes (Go's case folding then sees each invalid UTF-8 byte as
//! U+FFFD). A caller holding raw terminal input that may not be UTF-8 must
//! decide how to convert it before calling them; only
//! [`walletseed::go_hex_decode`] takes bytes.

pub mod error;
pub mod pgpwordlist;
mod secret;
pub mod walletseed;
mod wordlist;

pub use error::{ERR_INVALID_SEED_LEN, HexError, INVALID_SEED_LEN_TEXT, SeedErrorExt, WordError};
/// The error every fallible function of this crate returns:
/// [`purse_errors::Error`], re-exported.
///
/// **Its `render(sep)` returns an ordinary `String` that is not wiped,**
/// unlike this crate's former `Error::render`, which returned a
/// `Zeroizing<String>`. Code written against the old method still compiles
/// unless it named that type, so the loss is silent. To render an error
/// that may quote a seed word, call [`SeedErrorExt::render_zeroizing`], or
/// write it with `render_to` into a sink that wipes its memory. Its
/// [`std::error::Error::source`] chain hands out the nested level and the
/// quoted word's [`WordError`], each of whose `Display` writes the word;
/// see "What is wiped" in the [`error`] module documentation.
#[doc(inline)]
pub use purse_errors::Error;
pub use secret::{MnemonicWords, SecretBytes, SecretString};
pub use wordlist::WORD_LIST;
