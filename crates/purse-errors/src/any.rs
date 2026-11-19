// Copyright 2022 The Go Authors. All rights reserved.
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file. `join` re-implements the Go standard library's
// errors.Join, which is under the BSD-3-Clause license reproduced in this
// crate's NOTICE file.

//! Any error value that can appear in a chain (Go's `error` interface).

use std::any::Any;
use std::backtrace::Backtrace;
use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use crate::chain::{self, Node};
use crate::{Error, Kind, separator};

/// An error value as Go's `error` interface can hold one in dcrwallet: the
/// dynamic types that matter to the errors package, each a variant.
///
/// Cloning is cheap and keeps the value's identity, as copying a Go
/// interface value does: [`AnyError::identical`] is Go's `==` on error
/// values, which [`AnyError::is`] uses for sentinel errors.
///
/// [`Display`](fmt::Display) renders Go's `Error()` text, using the
/// process-wide [`separator`] for nested [`Error`] levels.
///
/// Chains of any depth are walked, rendered and dropped without recursion.
/// ([`Debug`](fmt::Debug) output is derived and does recurse.)
#[derive(Clone, Debug)]
pub enum AnyError {
    /// A wallet error (Go's `*errors.Error`).
    Error(Arc<Error>),
    /// A bare [`Kind`] used as an error value.
    Kind(Kind),
    /// A text error ([`new`](crate::new), a string argument of
    /// [`e!`](crate::e), or [`errorf!`](crate::errorf); Go's `errors.New`
    /// and `fmt.Errorf` without `%w`).
    Text(Text),
    /// A text that wraps other errors (Go's `fmt.Errorf` with `%w`).
    Wrapped(Wrapped),
    /// Several errors joined (Go's `errors.Join`).
    Joined(Joined),
    /// Any other Rust error, treated as an opaque leaf; see [`Foreign`].
    Foreign(Foreign),
}

/// A text error (Go's `*errors.errorString`). Each [`AnyError::new`] makes
/// a value with a new identity.
#[derive(Clone, Debug)]
pub struct Text(Arc<str>);

impl Text {
    /// The error text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A text that wraps other errors (Go's `*fmt.wrapError` or
/// `*fmt.wrapErrors`).
#[derive(Clone, Debug)]
pub struct Wrapped(Arc<WrappedInner>);

#[derive(Debug)]
struct WrappedInner {
    text: String,
    errs: Vec<AnyError>,
}

impl Drop for WrappedInner {
    fn drop(&mut self) {
        dismantle(None, std::mem::take(&mut self.errs));
    }
}

impl Wrapped {
    /// The error text, fixed when the value was made.
    pub fn text(&self) -> &str {
        &self.0.text
    }

    /// The wrapped errors, in order.
    pub fn wrapped(&self) -> &[AnyError] {
        &self.0.errs
    }
}

/// Several errors joined (Go's `*errors.joinError`); never empty.
#[derive(Clone, Debug)]
pub struct Joined(Arc<JoinedInner>);

#[derive(Debug)]
struct JoinedInner {
    errs: Vec<AnyError>,
}

impl Drop for JoinedInner {
    fn drop(&mut self) {
        dismantle(None, std::mem::take(&mut self.errs));
    }
}

impl Joined {
    /// The joined errors, in order.
    pub fn errors(&self) -> &[AnyError] {
        &self.0.errs
    }
}

/// Any other Rust error, held as an opaque leaf of a chain.
///
/// Matching never looks inside it: its [`source`](StdError::source) chain
/// is not traversed by [`AnyError::is`], [`AnyError::as_kind`] or
/// [`AnyError::as_error`], although Go's `errors.Is` and `errors.As` follow
/// a foreign type's `Unwrap`. That is a deliberate divergence, and it holds
/// only because of these rules for ports:
///
/// - Never put a wallet [`AnyError`], [`Error`] or [`Kind`], or any error
///   that code matches against, behind a foreign type's `source()`. Express
///   wrapping with [`e!`](crate::e) (an `Error` around it) or
///   [`AnyError::wrap`] (Go's `fmt.Errorf` with `%w`), which keep the
///   wrapped value matchable.
/// - Port a Go pointer sentinel (`context.Canceled`, `io.EOF`, a
///   package-level `errX = errors.New(...)`) as one shared [`AnyError`],
///   for example a `static ERR_X: LazyLock<AnyError>`, and pass clones of
///   it, so that identity matching with [`AnyError::is`] works as Go's
///   pointer comparison does. A fresh foreign value of the same type and
///   text never matches it.
///
/// [`AnyError::find_foreign`] finds a foreign leaf by its concrete type.
#[derive(Clone, Debug)]
pub struct Foreign(Arc<dyn StdError + Send + Sync + 'static>);

impl Foreign {
    /// The error as a trait object.
    pub fn as_dyn(&self) -> &(dyn StdError + Send + Sync + 'static) {
        &*self.0
    }

    /// The error, if it is a `T`.
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl AnyError {
    /// A text error with a new identity (Go's `errors.New`).
    pub fn new(text: impl Into<String>) -> AnyError {
        AnyError::Text(Text(Arc::from(text.into())))
    }

    /// A text that wraps `errs` (Go's `fmt.Errorf` with one `%w` verb per
    /// wrapped error). `text` is the complete message, already formatted:
    /// for Go's `fmt.Errorf("open %s: %w", name, err)` pass
    /// `format!("open {name}: {err}")` and `[err]`, since `%w` prints the
    /// wrapped error's text. The text is fixed now, as `fmt.Errorf`'s is,
    /// including any separator inside it.
    pub fn wrap(text: impl Into<String>, errs: impl IntoIterator<Item = AnyError>) -> AnyError {
        AnyError::Wrapped(Wrapped(Arc::new(WrappedInner {
            text: text.into(),
            errs: errs.into_iter().collect(),
        })))
    }

    /// Joins errors (Go's `errors.Join`): `None` entries are dropped, and
    /// the result is `None` when nothing is left. A single error is still
    /// wrapped in a new [`Joined`] value.
    pub fn join<I>(errs: I) -> Option<AnyError>
    where
        I: IntoIterator,
        I::Item: Into<Option<AnyError>>,
    {
        let errs: Vec<AnyError> = errs.into_iter().filter_map(Into::into).collect();
        if errs.is_empty() {
            None
        } else {
            Some(AnyError::Joined(Joined(Arc::new(JoinedInner { errs }))))
        }
    }

    /// Holds any Rust error as an [`AnyError`].
    ///
    /// These types become their own variant instead of a [`Foreign`] leaf,
    /// so the value keeps the dynamic type Go would see: [`AnyError`],
    /// [`Error`] and [`Kind`], and each of them in a `Box` or an `Arc`. An
    /// `Arc<Error>` or `Arc<AnyError>` is taken over as it is, keeping its
    /// identity. Anything nested deeper (`Box<Arc<Error>>`, say) stays a
    /// [`Foreign`] leaf.
    pub fn foreign<E: StdError + Send + Sync + 'static>(err: E) -> AnyError {
        let b: Box<dyn Any> = Box::new(err);
        let b = match b.downcast::<AnyError>() {
            Ok(a) => return *a,
            Err(b) => b,
        };
        let b = match b.downcast::<Box<AnyError>>() {
            Ok(a) => return **a,
            Err(b) => b,
        };
        let b = match b.downcast::<Arc<AnyError>>() {
            // A clone of an AnyError keeps its identity.
            Ok(a) => return Arc::unwrap_or_clone(*a),
            Err(b) => b,
        };
        let b = match b.downcast::<Error>() {
            Ok(e) => return AnyError::Error(Arc::new(*e)),
            Err(b) => b,
        };
        let b = match b.downcast::<Box<Error>>() {
            Ok(e) => return AnyError::Error(Arc::from(*e)),
            Err(b) => b,
        };
        let b = match b.downcast::<Arc<Error>>() {
            Ok(e) => return AnyError::Error(*e),
            Err(b) => b,
        };
        let b = match b.downcast::<Kind>() {
            Ok(k) => return AnyError::Kind(*k),
            Err(b) => b,
        };
        let b = match b.downcast::<Box<Kind>>() {
            Ok(k) => return AnyError::Kind(**k),
            Err(b) => b,
        };
        let b = match b.downcast::<Arc<Kind>>() {
            Ok(k) => return AnyError::Kind(**k),
            Err(b) => b,
        };
        match b.downcast::<E>() {
            Ok(e) => AnyError::Foreign(Foreign(Arc::new(*e))),
            Err(_) => unreachable!("the box holds an E"),
        }
    }

    /// Go's `==` on two error values: the same allocation for every variant
    /// but [`AnyError::Kind`], which compares by value.
    pub fn identical(&self, other: &AnyError) -> bool {
        chain::identical(chain::node(self), chain::node(other))
    }

    /// Whether this error equals or wraps `target` (Go's `errors.Is`).
    ///
    /// The chain is walked from this value inward, following
    /// [`Error::wrapped`] and every member of wrapped and joined values
    /// depth-first, in order. A node matches when it is
    /// [identical](Self::identical) to `target`, or when it is an [`Error`]
    /// and `target` is an [`Error`] template it matches (see
    /// [`matches`](Self::matches)) or a [`Kind`] other than [`Kind::Other`]
    /// equal to its own.
    ///
    /// Divergence (QUIRKS QK-0628): Go panics, ending the whole call, when
    /// `target` is an [`Error`] template whose (possibly nested) template
    /// level has a non-[`Error`] underlying error and the [`Error`] level
    /// it is compared with at that depth has none. Here only that
    /// [`Error`] node of the chain fails to match; the walk continues with
    /// the rest of the chain, so `is` may return true where Go panics.
    pub fn is(&self, target: &AnyError) -> bool {
        chain::is(chain::node(self), chain::node(target))
    }

    /// Whether the chain carries `kind` (Go's `errors.Is(err, kind)`).
    ///
    /// For [`Kind::Other`] this is true only when a bare `Other` value is
    /// in the chain, never for an [`Error`] whose kind is unset.
    pub fn is_kind(&self, kind: Kind) -> bool {
        chain::is(chain::node(self), Node::Kind(kind))
    }

    /// Whether some [`Error`] in the chain matches `template` (Go's
    /// `errors.Is(err, template)` with an `*Error` target).
    ///
    /// Every non-empty field of the template must equal the same field of
    /// one level, and a template's nested [`Error`] levels must match that
    /// level's nested levels in turn; a non-[`Error`] leaf of the template
    /// matches an underlying error with the same rendered text (with the
    /// process-wide [`separator`]).
    ///
    /// Divergence (QUIRKS QK-0628), as for [`is`](Self::is): where Go
    /// panics because the template has a leaf at a depth where the
    /// candidate level has no underlying error, that candidate does not
    /// match and the walk goes on to the rest of the chain.
    pub fn matches(&self, template: &Error) -> bool {
        chain::is(chain::node(self), Node::Error(template))
    }

    /// The outermost kind in the chain (Go's `errors.As(err, &kind)`): the
    /// first [`Error`] whose kind is not [`Kind::Other`], or the first bare
    /// [`Kind`] value, which may be `Other`.
    pub fn as_kind(&self) -> Option<Kind> {
        chain::as_kind(chain::node(self))
    }

    /// The first [`Error`] in the chain (Go's `errors.As(err, &e)` with
    /// `e *Error`). A bare [`Kind`] other than [`Kind::Other`] reached
    /// first yields a new `Error` carrying only that kind.
    pub fn as_error(&self) -> Option<Cow<'_, Error>> {
        chain::as_error(chain::node(self))
    }

    /// The first [`Foreign`] leaf of type `T` in the chain (Go's
    /// `errors.As` with a concrete foreign target type).
    pub fn find_foreign<T: StdError + 'static>(&self) -> Option<&T> {
        chain::find_foreign(chain::node(self))
    }

    /// The errors this value wraps directly (Go's `Unwrap`): for an
    /// [`Error`], [`Error::wrapped`]; for wrapped and joined values, their
    /// members; nothing for the leaf variants.
    pub fn wrapped(&self) -> Vec<AnyError> {
        match self {
            AnyError::Error(e) => e.wrapped().into_iter().collect(),
            AnyError::Wrapped(w) => w.wrapped().to_vec(),
            AnyError::Joined(j) => j.errors().to_vec(),
            AnyError::Kind(_) | AnyError::Text(_) | AnyError::Foreign(_) => Vec::new(),
        }
    }

    /// The stack traces of the chain (Go's `errors.Stacks`): empty unless
    /// this value is an [`Error`], then as [`Error::stacks`].
    pub fn stacks(&self) -> Vec<&Backtrace> {
        match self {
            AnyError::Error(e) => e.stacks(),
            _ => Vec::new(),
        }
    }

    /// Go's `Error()` text, rendered as if `errors.Separator` were `sep`.
    ///
    /// `sep` applies to the nested [`Error`] levels this renderer walks
    /// itself: those of this value and of every [`Error`] reached through
    /// [`Joined`] members. The text of a [`Text`] or [`Wrapped`] value was
    /// fixed when it was built (as `fmt.Errorf`'s is), and a [`Foreign`]
    /// leaf renders through its own [`Display`](fmt::Display), so an
    /// [`Error`] printed inside one uses the process-wide [`separator`].
    pub fn render(&self, sep: &str) -> String {
        chain::render(chain::node(self), sep)
    }

    /// The concrete error to hand out from [`StdError::source`], so that
    /// downcasting along the source chain finds an [`Error`], a [`Kind`] or
    /// a foreign type rather than an `AnyError` wrapper.
    pub(crate) fn as_source(&self) -> &(dyn StdError + 'static) {
        match self {
            AnyError::Error(e) => &**e,
            AnyError::Kind(k) => k,
            AnyError::Foreign(f) => f.as_dyn(),
            AnyError::Text(_) | AnyError::Wrapped(_) | AnyError::Joined(_) => self,
        }
    }
}

/// Drops a chain without recursion: each child whose last reference is
/// being dropped is moved onto an explicit stack before its parent goes.
pub(crate) fn dismantle(first: Option<AnyError>, mut rest: Vec<AnyError>) {
    let mut next = first;
    while let Some(a) = next.take().or_else(|| rest.pop()) {
        match a {
            AnyError::Error(arc) => {
                if let Some(mut e) = Arc::into_inner(arc) {
                    next = e.err.take();
                }
            }
            AnyError::Wrapped(Wrapped(arc)) => {
                if let Some(mut w) = Arc::into_inner(arc) {
                    rest.append(&mut w.errs);
                }
            }
            AnyError::Joined(Joined(arc)) => {
                if let Some(mut j) = Arc::into_inner(arc) {
                    rest.append(&mut j.errs);
                }
            }
            AnyError::Kind(_) | AnyError::Text(_) | AnyError::Foreign(_) => {}
        }
    }
}

impl fmt::Display for AnyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(separator()))
    }
}

impl StdError for AnyError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            // The value is the inner error itself, so its source is the
            // inner error's source, not the inner error (which would repeat
            // this value's message).
            AnyError::Error(e) => e.source(),
            AnyError::Wrapped(w) => w.wrapped().first().map(AnyError::as_source),
            AnyError::Foreign(f) => f.as_dyn().source(),
            AnyError::Kind(_) | AnyError::Text(_) | AnyError::Joined(_) => None,
        }
    }
}

impl From<Error> for AnyError {
    fn from(e: Error) -> AnyError {
        AnyError::Error(Arc::new(e))
    }
}

impl From<Arc<Error>> for AnyError {
    fn from(e: Arc<Error>) -> AnyError {
        AnyError::Error(e)
    }
}

impl From<Kind> for AnyError {
    fn from(k: Kind) -> AnyError {
        AnyError::Kind(k)
    }
}

pub(crate) fn text_ptr_eq(a: &Text, b: &Text) -> bool {
    Arc::ptr_eq(&a.0, &b.0)
}

pub(crate) fn wrapped_ptr_eq(a: &Wrapped, b: &Wrapped) -> bool {
    Arc::ptr_eq(&a.0, &b.0)
}

pub(crate) fn joined_ptr_eq(a: &Joined, b: &Joined) -> bool {
    Arc::ptr_eq(&a.0, &b.0)
}

pub(crate) fn foreign_ptr_eq(a: &Foreign, b: &Foreign) -> bool {
    Arc::ptr_eq(&a.0, &b.0)
}

/// A text error with a new identity (Go's `errors.New`); the same as
/// [`AnyError::new`].
pub fn new(text: impl Into<String>) -> AnyError {
    AnyError::new(text)
}

/// Joins errors (Go's `errors.Join`); the same as [`AnyError::join`].
pub fn join<I>(errs: I) -> Option<AnyError>
where
    I: IntoIterator,
    I::Item: Into<Option<AnyError>>,
{
    AnyError::join(errs)
}

/// Returns a text error built with Rust [`format!`] syntax (Go's
/// `errors.Errorf` without `%w`): `errorf!("bad length {}", n)`.
///
/// For Go's `%w`, which keeps the wrapped error reachable by
/// [`AnyError::is`] and [`AnyError::as_kind`], use [`AnyError::wrap`].
#[macro_export]
macro_rules! errorf {
    ($($arg:tt)*) => {
        $crate::AnyError::new(::std::format!($($arg)*))
    };
}
