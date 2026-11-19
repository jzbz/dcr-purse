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
//! contents. Errors render as dcrwallet's `errors.Error` does, byte for byte
//! ([`Error`]); [`ErrorCause`]'s `Display` is the bare leaf text, such as
//! the `encoding/hex` message of [`walletseed::go_hex_decode`].
//!
//! # Text input
//!
//! The mnemonic decoders take `&str`, whereas Go's take strings that may hold
//! arbitrary bytes (Go's case folding then sees each invalid UTF-8 byte as
//! U+FFFD). A caller holding raw terminal input that may not be UTF-8 must
//! decide how to convert it before calling them; only
//! [`walletseed::go_hex_decode`] takes bytes.

mod error;
pub mod pgpwordlist;
mod secret;
pub mod walletseed;
mod wordlist;

pub use error::{DAEMON_SEPARATOR, Error, ErrorCause, ErrorKind, LIBRARY_SEPARATOR, Op};
pub use secret::{MnemonicWords, SecretBytes, SecretString};
pub use wordlist::WORD_LIST;
