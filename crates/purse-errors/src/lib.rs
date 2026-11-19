// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Error creation and matching for all wallet systems: a port of
//! dcrwallet's `errors` package.
//!
//! Its rendered text reaches JSON-RPC and gRPC clients verbatim, so the
//! rendering is byte-exact with Go's: the 21 [`Kind`] strings, the
//! [`Op`]/[`Kind`]/underlying-error model of [`Error`], the promote and
//! collapse rules of Go's `E` (here [`e!`] and [`Builder`]), and the
//! `": "` / [`separator`] joints of `Error()`.
//!
//! | Go | Rust |
//! |---|---|
//! | `errors.Op`, `errors.Opf` | [`Op`], [`opf!`] |
//! | `errors.Kind`, `Other` .. `Deployment` | [`Kind`] |
//! | `*errors.Error`, `&errors.Error{...}` | [`Error`], [`Error::from_parts`] |
//! | `errors.E`, `errors.WithStack` | [`e!`], [`with_stack!`], [`Builder`] |
//! | an `error` value | [`AnyError`] |
//! | `errors.New`, `errors.Errorf` | [`new`], [`errorf!`]; with `%w`, [`AnyError::wrap`] |
//! | `errors.Join` | [`join`] |
//! | `errors.Is(err, target)` | [`AnyError::is`], [`AnyError::is_kind`], [`AnyError::matches`] |
//! | `errors.As(err, &kind)`, `errors.As(err, &e)` | [`AnyError::as_kind`], [`AnyError::as_error`] |
//! | `(*Error).Unwrap` | [`Error::wrapped`] |
//! | `errors.Stacks` | [`Error::stacks`], [`AnyError::stacks`] |
//! | `errors.Separator` | [`separator`], [`set_separator`] |
//!
//! Where Go's `E` would panic or silently drop an argument of a type it
//! does not know, the Rust signatures do not accept one: [`e!`] needs at
//! least one argument, and only the types [`Arg`] converts from are
//! arguments. A typed-nil `*Error` (which panics in Go) cannot be built.
//!
//! Deliberate divergence (QUIRKS QK-0628): Go's unexported `match` panics
//! when a template's non-`Error` leaf meets a candidate level without one,
//! as in `Is(E(Op("x"), Invalid), E("text"))`; here that level does not
//! match and the walk goes on (see [`AnyError::is`]).
//!
//! A second deliberate divergence: a [`Foreign`] leaf is opaque to
//! matching, where Go's `errors.Is` and `errors.As` follow a foreign type's
//! `Unwrap`. The rules on [`Foreign`] keep that invisible.
//!
//! # Sentinel errors
//!
//! Go compares sentinel errors (`io.EOF`, `context.Canceled`, a
//! package-level `var errX = errors.New(...)`) by pointer. Port each as one
//! shared [`AnyError`] and hand out clones of it; a new value with the same
//! text is a different error, as in Go.
//!
//! ```
//! use std::sync::LazyLock;
//!
//! use purse_errors::{AnyError, Kind, Op, e};
//!
//! /// Go: `var errNoInputValue = errors.New("no input value")`.
//! static ERR_NO_INPUT_VALUE: LazyLock<AnyError> =
//!     LazyLock::new(|| AnyError::new("no input value"));
//!
//! fn read_input() -> Result<String, AnyError> {
//!     Err(e!(Op::new("tool.readInput"), Kind::Io, &*ERR_NO_INPUT_VALUE).into())
//! }
//!
//! let err = read_input().unwrap_err();
//! assert!(err.is(&ERR_NO_INPUT_VALUE));
//! assert!(!err.is(&AnyError::new("no input value")));
//! assert_eq!(err.render(":: "), "tool.readInput: I/O error: no input value");
//! ```
//!
//! # Secrets
//!
//! The text of [`new`], of string arguments to [`e!`], of [`errorf!`] and
//! of [`AnyError::wrap`] is kept in ordinary, unzeroized memory, and its
//! [`Debug`](std::fmt::Debug) output is verbatim. Never put secret material
//! (seeds, keys, passphrases, or anything derived from them) in it. A cause
//! that must quote secret material belongs in an error type local to the
//! crate that owns the secret, passed in through [`AnyError::foreign`],
//! which holds the secret in `Zeroizing` and whose `Debug` redacts it
//! (`purse-seed`'s `ErrorCause` is the model).

#![forbid(unsafe_code)]

mod any;
mod chain;
mod error;
mod kind;
mod op;

use std::sync::{PoisonError, RwLock};

pub use any::{AnyError, Foreign, Joined, Text, Wrapped, join, new};
pub use error::{Arg, Builder, Error};
pub use kind::{Kind, UNKNOWN_KIND_TEXT};
pub use op::Op;

/// Go's default `errors.Separator`, used by library code, tests and the
/// `cmd/` tools: nested errors render on separate, indented lines.
pub const MULTILINE_SEPARATOR: &str = ":\n\t";

/// The separator the dcrwallet daemon installs at start-up
/// (`dcrwallet.go:36-39`), so that every error sent to RPC clients and
/// written to the log is on one line.
pub const DAEMON_SEPARATOR: &str = ":: ";

static SEPARATOR: RwLock<&'static str> = RwLock::new(MULTILINE_SEPARATOR);

/// The process-wide separator inserted between nested [`Error`] levels by
/// [`Display`](std::fmt::Display) (Go's `errors.Separator`); initially
/// [`MULTILINE_SEPARATOR`].
pub fn separator() -> &'static str {
    *SEPARATOR.read().unwrap_or_else(PoisonError::into_inner)
}

/// Replaces the process-wide separator (Go's assignment to
/// `errors.Separator`). The daemon sets [`DAEMON_SEPARATOR`] once at
/// start-up; as in Go, it affects every later rendering in the process,
/// including the text comparison inside [`AnyError::matches`].
pub fn set_separator(sep: &'static str) {
    *SEPARATOR.write().unwrap_or_else(PoisonError::into_inner) = sep;
}

#[cfg(test)]
mod tests;
