// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The wallet error (Go's `*errors.Error`) and its construction (Go's `E`).

use std::backtrace::Backtrace;
use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use crate::chain::{self, Node};
use crate::{AnyError, Kind, Op, separator};

/// Describes an error condition raised within the wallet (Go's
/// `errors.Error`): an operation, a kind, and an underlying error, any of
/// which may be unset.
///
/// Build one with [`e!`](crate::e) or [`Builder`] (Go's `E`), which apply
/// Go's promote and collapse rules, or with [`Error::from_parts`] (a Go
/// struct literal), which does not.
///
/// [`Display`](fmt::Display) renders Go's `Error()` text with the
/// process-wide [`separator`]; [`Error::render`] takes the separator
/// explicitly.
#[derive(Clone, Default)]
pub struct Error {
    /// The operation; empty when unset.
    pub op: Op,
    /// The class of error; [`Kind::Other`] when unset.
    pub kind: Kind,
    /// The underlying error.
    pub err: Option<AnyError>,
    stack: Option<Arc<Backtrace>>,
}

impl Error {
    /// An error with exactly these fields and no stack trace (a Go struct
    /// literal `&errors.Error{Op: op, Kind: kind, Err: err}`). Unlike
    /// [`e!`](crate::e), nothing is promoted or collapsed.
    pub fn from_parts(op: Op, kind: Kind, err: Option<AnyError>) -> Error {
        Error {
            op,
            kind,
            err,
            stack: None,
        }
    }

    /// The stack trace recorded by [`Builder::build_with_stack`] or
    /// inherited through a collapse, if any. It is never rendered and never
    /// compared.
    pub fn stack(&self) -> Option<&Backtrace> {
        self.stack.as_deref()
    }

    /// The stack traces of this level and of each nested [`Error`] reached
    /// by following [`Error::err`] while it is an `Error`, outermost first,
    /// skipping levels without one (Go's `errors.Stacks`).
    pub fn stacks(&self) -> Vec<&Backtrace> {
        let mut out = Vec::new();
        let mut e = self;
        loop {
            if let Some(s) = e.stack() {
                out.push(s);
            }
            match &e.err {
                Some(AnyError::Error(inner)) => e = inner,
                _ => return out,
            }
        }
    }

    /// The error this one wraps (Go's `Unwrap`): [`Error::err`] if set,
    /// else the kind as a bare [`AnyError::Kind`] unless it is
    /// [`Kind::Other`], else `None`.
    pub fn wrapped(&self) -> Option<AnyError> {
        match &self.err {
            Some(e) => Some(e.clone()),
            None if self.kind != Kind::Other => Some(AnyError::Kind(self.kind)),
            None => None,
        }
    }

    /// As [`AnyError::is`], starting at this error.
    pub fn is(&self, target: &AnyError) -> bool {
        chain::is(Node::Error(self), chain::node(target))
    }

    /// As [`AnyError::is_kind`], starting at this error.
    pub fn is_kind(&self, kind: Kind) -> bool {
        chain::is(Node::Error(self), Node::Kind(kind))
    }

    /// As [`AnyError::matches`], starting at this error.
    pub fn matches(&self, template: &Error) -> bool {
        chain::is(Node::Error(self), Node::Error(template))
    }

    /// As [`AnyError::as_kind`], starting at this error.
    pub fn as_kind(&self) -> Option<Kind> {
        chain::as_kind(Node::Error(self))
    }

    /// As [`AnyError::as_error`], starting at this error: always this error
    /// itself.
    pub fn as_error(&self) -> Option<Cow<'_, Error>> {
        chain::as_error(Node::Error(self))
    }

    /// As [`AnyError::find_foreign`], starting at this error.
    pub fn find_foreign<T: StdError + 'static>(&self) -> Option<&T> {
        chain::find_foreign(Node::Error(self))
    }

    /// Go's `Error()` text, rendered as if `errors.Separator` were `sep`.
    ///
    /// Each level writes its operation and kind, separated by `": "`; an
    /// operation or kind equal to the last one written is skipped. `sep`
    /// goes before a nested [`Error`] level and `": "` before any other
    /// underlying error, each only when the level wrote something. An empty
    /// result becomes `"unclassified error"`.
    ///
    /// `sep` applies to the nested [`Error`] levels this renderer walks
    /// itself: the levels of this chain and those of every [`Error`] reached
    /// through [`Joined`](crate::Joined) members. The text of a
    /// [`Text`](crate::Text) or [`Wrapped`](crate::Wrapped) value was fixed
    /// when it was built (as `fmt.Errorf`'s is), and a
    /// [`Foreign`](crate::Foreign) leaf renders through its own
    /// [`Display`](fmt::Display), so an [`Error`] printed inside one uses the
    /// process-wide [`separator`].
    pub fn render(&self, sep: &str) -> String {
        chain::render(Node::Error(self), sep)
    }
}

impl Drop for Error {
    /// Drops the chain below this level without recursion, however deep it
    /// is.
    fn drop(&mut self) {
        if let Some(err) = self.err.take() {
            crate::any::dismantle(Some(err), Vec::new());
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(separator()))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("op", &self.op)
            .field("kind", &self.kind)
            .field("err", &self.err)
            .field("stack", &self.stack.is_some())
            .finish()
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.err {
            Some(e) => Some(e.as_source()),
            None if self.kind != Kind::Other => Some(&self.kind),
            None => None,
        }
    }
}

/// One argument of [`e!`](crate::e) or [`Builder::arg`], classified as Go's
/// `E` classifies its arguments by dynamic type.
///
/// Conversions exist from [`Op`] and `&Op`, [`Kind`], strings, [`Error`],
/// `Arc<Error>` and `&Arc<Error>`, [`AnyError`] and `&AnyError` (by
/// variant: an `Error` or a bare `Kind` counts as such, as in Go's type
/// switch) and `Option` of any of these (`None` is ignored, as Go ignores a
/// nil error). The borrowed forms clone, which is cheap and keeps identity:
/// an `Op` clone shares its text, and an `Arc` or `AnyError` clone is the
/// same value to [`AnyError::identical`].
///
/// There is deliberately no conversion from `&Error`: it would have to put
/// a copy in a new allocation, silently giving the argument a new identity
/// where Go would pass the same pointer. Pass the `Error` by value, or keep
/// it in an `Arc` (or an [`AnyError`]) and pass a reference to that.
///
/// Go also ignores arguments of other types, integers for example; those
/// have no conversion here, so a port drops them at the call site.
#[derive(Debug)]
pub struct Arg(ArgRepr);

#[derive(Debug)]
enum ArgRepr {
    Op(Op),
    Kind(Kind),
    Text(String),
    Error(Arc<Error>),
    Other(AnyError),
    Nil,
}

impl From<Op> for Arg {
    fn from(op: Op) -> Arg {
        Arg(ArgRepr::Op(op))
    }
}

impl From<&Op> for Arg {
    fn from(op: &Op) -> Arg {
        Arg(ArgRepr::Op(op.clone()))
    }
}

impl From<Kind> for Arg {
    fn from(kind: Kind) -> Arg {
        Arg(ArgRepr::Kind(kind))
    }
}

impl From<&str> for Arg {
    fn from(text: &str) -> Arg {
        Arg(ArgRepr::Text(text.to_owned()))
    }
}

impl From<&String> for Arg {
    fn from(text: &String) -> Arg {
        Arg(ArgRepr::Text(text.clone()))
    }
}

impl From<String> for Arg {
    fn from(text: String) -> Arg {
        Arg(ArgRepr::Text(text))
    }
}

impl From<Error> for Arg {
    fn from(e: Error) -> Arg {
        Arg(ArgRepr::Error(Arc::new(e)))
    }
}

impl From<Arc<Error>> for Arg {
    fn from(e: Arc<Error>) -> Arg {
        Arg(ArgRepr::Error(e))
    }
}

impl From<&Arc<Error>> for Arg {
    fn from(e: &Arc<Error>) -> Arg {
        Arg(ArgRepr::Error(Arc::clone(e)))
    }
}

impl From<AnyError> for Arg {
    fn from(a: AnyError) -> Arg {
        Arg(match a {
            AnyError::Error(e) => ArgRepr::Error(e),
            AnyError::Kind(k) => ArgRepr::Kind(k),
            other => ArgRepr::Other(other),
        })
    }
}

impl From<&AnyError> for Arg {
    fn from(a: &AnyError) -> Arg {
        Arg::from(a.clone())
    }
}

impl<T: Into<Arg>> From<Option<T>> for Arg {
    fn from(a: Option<T>) -> Arg {
        a.map_or(Arg(ArgRepr::Nil), Into::into)
    }
}

/// Builds an [`Error`] from arguments as Go's `E` does; [`e!`](crate::e) is
/// shorthand for it.
///
/// Arguments are applied in order. An [`Op`] or [`Kind`] sets that field;
/// a string, an [`Error`] or any other error sets the underlying error.
/// A later argument of the same category replaces an earlier one. An
/// [`Error`] argument also copies its kind at once if the kind is still
/// [`Kind::Other`], so that kind survives even when a later argument
/// replaces the underlying error (QUIRKS QK-0626).
///
/// [`build`](Builder::build) then, when the last argument that set the
/// underlying error was an [`Error`] (call it `prev`), promotes `prev`'s
/// operation and kind into any field still unset, and collapses the
/// nesting, replacing `prev` by its own underlying error and inheriting its
/// stack trace, when `prev` has no operation and no kind that differ from
/// the new error's.
///
/// Go panics when `E` gets no arguments; [`Builder::new`] requires the
/// first one.
#[derive(Debug)]
pub struct Builder {
    e: Error,
    err_is_prev: bool,
}

impl Builder {
    /// A builder with its first argument applied.
    pub fn new(first: impl Into<Arg>) -> Builder {
        Builder {
            e: Error::default(),
            err_is_prev: false,
        }
        .arg(first)
    }

    /// Applies one more argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<Arg>) -> Builder {
        match arg.into().0 {
            ArgRepr::Op(op) => self.e.op = op,
            ArgRepr::Kind(kind) => self.e.kind = kind,
            ArgRepr::Text(text) => {
                self.e.err = Some(AnyError::new(text));
                self.err_is_prev = false;
            }
            ArgRepr::Error(prev) => {
                if self.e.kind == Kind::Other {
                    self.e.kind = prev.kind;
                }
                self.e.err = Some(AnyError::Error(prev));
                self.err_is_prev = true;
            }
            ArgRepr::Other(err) => {
                self.e.err = Some(err);
                self.err_is_prev = false;
            }
            ArgRepr::Nil => {}
        }
        self
    }

    /// Finishes the error (Go's `E`).
    pub fn build(self) -> Error {
        let mut e = self.e;
        if self.err_is_prev
            && let Some(AnyError::Error(prev)) = e.err.clone()
        {
            if e.op.is_empty() {
                e.op = prev.op.clone();
            }
            if e.kind == Kind::Other {
                e.kind = prev.kind;
            }
            if (prev.op.is_empty() || e.op == prev.op)
                && (prev.kind == Kind::Other || e.kind == prev.kind)
            {
                e.err = prev.err.clone();
                if e.stack.is_none() {
                    e.stack = prev.stack.clone();
                }
            }
        }
        e
    }

    /// Finishes the error and records the current stack trace in it,
    /// replacing any inherited through a collapse (Go's `WithStack`).
    pub fn build_with_stack(self) -> Error {
        let mut e = self.build();
        e.stack = Some(Arc::new(Backtrace::force_capture()));
        e
    }
}

/// Creates an [`Error`] from one or more arguments (Go's `errors.E`); see
/// [`Builder`] for the rules and [`Arg`] for the accepted types.
///
/// ```
/// use purse_errors::{e, Kind, Op};
///
/// const OP: Op = Op::new("wallet.Unlock");
/// let inner = e!(Op::new("udb.Decrypt"), Kind::Passphrase);
/// let err = e!(OP, inner);
/// assert_eq!(err.render(":: "), "wallet.Unlock: invalid passphrase:: udb.Decrypt");
/// ```
#[macro_export]
macro_rules! e {
    ($first:expr $(, $rest:expr)* $(,)?) => {
        $crate::Builder::new($first)$(.arg($rest))*.build()
    };
}

/// As [`e!`](crate::e), recording the current stack trace in the error
/// (Go's `errors.WithStack`).
#[macro_export]
macro_rules! with_stack {
    ($first:expr $(, $rest:expr)* $(,)?) => {
        $crate::Builder::new($first)$(.arg($rest))*.build_with_stack()
    };
}
