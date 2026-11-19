// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The operation an error was raised in (Go's `errors.Op`).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Describes the operation, method, or RPC in which an error condition was
/// raised (Go's `errors.Op`).
///
/// The empty `Op` is Go's zero value: it means "no operation" and is never
/// rendered. Declare the usual constant form as
/// `const OP: Op = Op::new("wallet.Foo");`; build a formatted one (Go's
/// `Opf`) with [`opf!`](crate::opf).
///
/// Cloning is always cheap (a pointer copy, or a reference-count increment
/// for a formatted `Op`), so functions take and store `Op` by value freely.
/// Equality, ordering and hashing are those of the text.
#[derive(Clone)]
pub struct Op(Repr);

#[derive(Clone)]
enum Repr {
    Static(&'static str),
    Shared(Arc<str>),
}

impl Op {
    /// The empty (unset) operation.
    pub const NONE: Op = Op(Repr::Static(""));

    /// An operation named by a static string.
    pub const fn new(name: &'static str) -> Op {
        Op(Repr::Static(name))
    }

    /// An operation named by a string built at run time.
    pub fn from_string(name: String) -> Op {
        Op(Repr::Shared(Arc::from(name)))
    }

    /// The operation's name.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Static(s) => s,
            Repr::Shared(s) => s,
        }
    }

    /// Whether this is the empty (unset) operation.
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

impl Default for Op {
    fn default() -> Op {
        Op::NONE
    }
}

impl PartialEq for Op {
    fn eq(&self, other: &Op) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Op {}

impl PartialOrd for Op {
    fn partial_cmp(&self, other: &Op) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Op {
    fn cmp(&self, other: &Op) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Op {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Op").field(&self.as_str()).finish()
    }
}

impl From<&'static str> for Op {
    fn from(name: &'static str) -> Op {
        Op::new(name)
    }
}

impl From<String> for Op {
    fn from(name: String) -> Op {
        Op::from_string(name)
    }
}

impl AsRef<str> for Op {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returns a formatted [`Op`] (Go's `errors.Opf`), taking Rust
/// [`format!`] syntax: `opf!("wallet.Foo({})", account)`.
#[macro_export]
macro_rules! opf {
    ($($arg:tt)*) => {
        $crate::Op::from_string(::std::format!($($arg)*))
    };
}
