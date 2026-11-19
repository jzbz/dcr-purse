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
//! (`purse-seed`'s `WordError` is the model).
//!
//! A [`Foreign`] leaf is shared, not copied, by clones of the chain, and
//! [`Debug`](std::fmt::Debug) of a chain prints it through the leaf's own
//! `Debug`. Its text leaves the leaf only when the chain is rendered:
//!
//! - [`Display`](std::fmt::Display) and [`Error::render_to`] /
//!   [`AnyError::render_to`] write each piece straight into the caller's
//!   sink, with no intermediate buffer; whatever that sink is (a
//!   `to_string()` result, a log line) belongs to the caller. If the sink
//!   fails, nothing more is written to it, so it holds a prefix of the
//!   text, and `render_to` reports the failure even when a leaf's
//!   `Display` ignored it.
//! - [`Error::render`] and [`AnyError::render`] return an ordinary
//!   `String`, which is not wiped and may have been reallocated while it
//!   grew, leaving freed copies of a prefix behind. To render into wiped
//!   memory, size a buffer with one `render_to` into a byte counter and
//!   write it with a second through a writer that refuses to grow it,
//!   starting again in a larger buffer if `render_to` reports that refusal
//!   (a leaf's text may be longer the second time).
//! - [`AnyError::matches`] / [`Error::matches`], and [`AnyError::is`] /
//!   [`Error::is`] with an [`AnyError::Error`] target (Go's `errors.Is`
//!   with an `*Error` template), may render the candidate. This happens
//!   when every template level above matches on operation and kind and
//!   the template's underlying error at some depth is not an [`Error`]:
//!   the candidate's whole underlying error at that depth, including any
//!   nested [`Error`] levels with their operation prefixes and the leaf
//!   below them, is then rendered with the process-wide [`separator`] into
//!   a temporary `String` that is not wiped and may leave freed copies
//!   behind from reallocation (the template's side is rendered the same
//!   way). A template with no underlying error at that depth renders
//!   nothing.
//! - [`std::error::Error::source`] hands out each nested [`Error`] level
//!   and then the foreign leaf itself (as Go's `Unwrap` does), and every
//!   one of those links' `Display` includes the leaf's text. A reporter
//!   that walks the source chain and prints each link (anyhow's or eyre's
//!   chain formats, tracing's error-chain formatting) therefore writes the
//!   leaf's text once per link. To log a chain whose leaf may quote secret
//!   material, print its `Display` once instead.
//!
//! The streaming is pinned by tests where safe Rust can observe it: the
//! pieces a sink receives from `render_to` and from `Display`, and that a
//! failed sink receives nothing more. The absence of freed heap copies in
//! general, for example from inside a leaf's own `Display`, is checked by
//! review only, since observing it would need a custom global allocator,
//! and this crate forbids `unsafe`.

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
