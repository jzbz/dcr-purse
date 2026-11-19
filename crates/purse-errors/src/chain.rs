// Copyright (c) 2018-2019 The Decred developers
// Copyright 2018 The Go Authors. All rights reserved.
// Copyright 2022 The Go Authors. All rights reserved.
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file. The traversal re-implements the Go standard library's
// errors.Is, errors.As and errors.Join, which are under the BSD-3-Clause
// license reproduced in this crate's NOTICE file.

//! Chain traversal and rendering: Go's `errors.Is` and `errors.As` driven
//! by the `Is`, `As` and `Unwrap` methods of `*Error` and `Kind`, the
//! unexported `match`, and the `Error()` methods of `*Error` and of
//! `errors.Join`'s result.
//!
//! Every walk here is a loop over an explicit stack, never a recursion per
//! level, so chains of any depth (Go's goroutine stacks grow to a gigabyte)
//! cannot overflow the thread's stack.

use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt::{self, Write as _};

use crate::any::{foreign_ptr_eq, joined_ptr_eq, text_ptr_eq, wrapped_ptr_eq};
use crate::{AnyError, Error, Foreign, Joined, Kind, Text, Wrapped};

/// One node of a chain, borrowed. A bare kind is held by value, since
/// `Error::wrapped` makes one out of a field.
#[derive(Clone, Copy)]
pub(crate) enum Node<'a> {
    Error(&'a Error),
    Kind(Kind),
    Text(&'a Text),
    Wrapped(&'a Wrapped),
    Joined(&'a Joined),
    Foreign(&'a Foreign),
}

pub(crate) fn node(a: &AnyError) -> Node<'_> {
    match a {
        AnyError::Error(e) => Node::Error(e),
        AnyError::Kind(k) => Node::Kind(*k),
        AnyError::Text(t) => Node::Text(t),
        AnyError::Wrapped(w) => Node::Wrapped(w),
        AnyError::Joined(j) => Node::Joined(j),
        AnyError::Foreign(f) => Node::Foreign(f),
    }
}

/// Go's `Unwrap() error` on an `*Error` node: `Err`, else the kind unless
/// it is `Other`, else nothing.
fn unwrap_error(e: &Error) -> Option<Node<'_>> {
    match &e.err {
        Some(inner) => Some(node(inner)),
        None if e.kind != Kind::Other => Some(Node::Kind(e.kind)),
        None => None,
    }
}

/// Go's `==` on two interface values holding these nodes. Every dynamic
/// type in the model is comparable (pointers, and `Kind`).
pub(crate) fn identical(a: Node<'_>, b: Node<'_>) -> bool {
    match (a, b) {
        (Node::Error(x), Node::Error(y)) => std::ptr::eq(x, y),
        (Node::Kind(x), Node::Kind(y)) => x == y,
        (Node::Text(x), Node::Text(y)) => text_ptr_eq(x, y),
        (Node::Wrapped(x), Node::Wrapped(y)) => wrapped_ptr_eq(x, y),
        (Node::Joined(x), Node::Joined(y)) => joined_ptr_eq(x, y),
        (Node::Foreign(x), Node::Foreign(y)) => foreign_ptr_eq(x, y),
        _ => false,
    }
}

/// The nodes of a chain in the order the standard library's `errors.Is`
/// and `errors.As` visit them (`errors/wrap.go`): a node, then what its
/// `Unwrap` returns; for `Unwrap() []error`, each member's whole subtree in
/// turn.
struct Walk<'a> {
    stack: Vec<Node<'a>>,
}

fn walk(root: Node<'_>) -> Walk<'_> {
    Walk { stack: vec![root] }
}

impl<'a> Iterator for Walk<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        let n = self.stack.pop()?;
        match n {
            Node::Error(e) => self.stack.extend(unwrap_error(e)),
            Node::Wrapped(w) => self.stack.extend(w.wrapped().iter().rev().map(node)),
            Node::Joined(j) => self.stack.extend(j.errors().iter().rev().map(node)),
            Node::Kind(_) | Node::Text(_) | Node::Foreign(_) => {}
        }
        Some(n)
    }
}

/// `errors.Is`: a node matches when it is identical to the target, or when
/// its `Is` method (only `*Error` has one) reports true.
pub(crate) fn is(root: Node<'_>, target: Node<'_>) -> bool {
    walk(root).any(|n| identical(n, target) || matches!(n, Node::Error(e) if error_is(e, target)))
}

/// `(*Error).Is` (`errors/errors.go:330-338`).
fn error_is(e: &Error, target: Node<'_>) -> bool {
    match target {
        Node::Error(t) => match_error(t, e),
        Node::Kind(k) => e.kind != Kind::Other && e.kind == k,
        _ => false,
    }
}

/// The unexported `match(template, e)` (`errors/errors.go:357-386`), with
/// its recursion over nested template levels turned into a loop.
fn match_error(mut t: &Error, mut e: &Error) -> bool {
    loop {
        if !t.op.is_empty() && t.op != e.op {
            return false;
        }
        if t.kind != Kind::Other && t.kind != e.kind {
            return false;
        }
        let Some(terr) = &t.err else {
            return true;
        };
        if let Some(eerr) = &e.err
            && terr.identical(eerr)
        {
            return true;
        }
        match terr {
            AnyError::Error(ti) => match &e.err {
                Some(AnyError::Error(ei)) => {
                    t = ti;
                    e = ei;
                }
                _ => return false,
            },
            // Go calls Error() on a nil interface here and panics (QK-0628).
            _ => {
                return match &e.err {
                    Some(eerr) => terr.to_string() == eerr.to_string(),
                    None => false,
                };
            }
        }
    }
}

/// `errors.As` with a `*Kind` target.
pub(crate) fn as_kind(root: Node<'_>) -> Option<Kind> {
    walk(root).find_map(|n| match n {
        // A Kind is assignable to the target by type, Other included
        // (Kind.As is never consulted).
        Node::Kind(k) => Some(k),
        Node::Error(e) if e.kind != Kind::Other => Some(e.kind),
        _ => None,
    })
}

/// `errors.As` with a `**Error` target.
pub(crate) fn as_error(root: Node<'_>) -> Option<Cow<'_, Error>> {
    walk(root).find_map(|n| match n {
        Node::Error(e) => Some(Cow::Borrowed(e)),
        // Kind.As synthesises &Error{Kind: k}, except for Other.
        Node::Kind(k) if k != Kind::Other => {
            Some(Cow::Owned(Error::from_parts(crate::Op::NONE, k, None)))
        }
        _ => None,
    })
}

/// `errors.As` with a concrete foreign target type.
pub(crate) fn find_foreign<T: StdError + 'static>(root: Node<'_>) -> Option<&T> {
    walk(root).find_map(|n| match n {
        Node::Foreign(f) => f.downcast_ref::<T>(),
        _ => None,
    })
}

/// Work left for [`render`]: a node to render, the newline between two
/// joined members, or the end of an `*Error`'s text, which became
/// `"unclassified error"` if it is still empty.
enum Piece<'a> {
    Node(Node<'a>),
    Newline,
    EndError(usize),
}

/// Go's `Error()` text of `root` as if `errors.Separator` were `sep`.
pub(crate) fn render(root: Node<'_>, sep: &str) -> String {
    let mut out = String::new();
    // A String never fails to take text.
    let _ = render_to(root, sep, &mut out);
    out
}

/// A writer that counts the bytes it has passed on, and remembers whether
/// the writer underneath failed (as opposed to a foreign leaf's `Display`
/// returning an error on its own).
///
/// Once the writer underneath has failed, every later write is refused
/// without reaching it, so a foreign leaf that ignores the failure and
/// writes on cannot put text after the cut: the writer underneath always
/// holds a prefix of the full text.
struct Counted<'w, W: fmt::Write + ?Sized> {
    inner: &'w mut W,
    len: usize,
    sink_failed: bool,
}

impl<W: fmt::Write + ?Sized> fmt::Write for Counted<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.sink_failed {
            return Err(fmt::Error);
        }
        if let Err(e) = self.inner.write_str(s) {
            self.sink_failed = true;
            return Err(e);
        }
        self.len = self.len.saturating_add(s.len());
        Ok(())
    }
}

/// [`render`] into any [`fmt::Write`], with no intermediate buffer: each
/// piece of text, a foreign leaf's [`Display`](fmt::Display) output
/// included, goes straight to `out`.
///
/// Only an error from `out` itself is returned, and it ends the rendering
/// at once, whatever the foreign leaf being written at the time reports (a
/// leaf's `Display` may ignore a failed write and return `Ok`). An error a
/// foreign leaf's `Display` returns on its own is ignored and the rest of
/// the text is written, as the `String` rendering has always done (writing
/// into a `String` keeps whatever the leaf wrote before failing).
pub(crate) fn render_to<W: fmt::Write + ?Sized>(
    root: Node<'_>,
    sep: &str,
    out: &mut W,
) -> fmt::Result {
    let mut out = Counted {
        inner: out,
        len: 0,
        sink_failed: false,
    };
    let mut stack = vec![Piece::Node(root)];
    while let Some(piece) = stack.pop() {
        match piece {
            Piece::Newline => out.write_char('\n')?,
            Piece::EndError(start) => {
                if out.len == start {
                    out.write_str(Kind::Other.as_str())?;
                }
            }
            Piece::Node(Node::Error(e)) => render_error(e, sep, &mut out, &mut stack)?,
            Piece::Node(Node::Kind(k)) => out.write_str(k.as_str())?,
            Piece::Node(Node::Text(t)) => out.write_str(t.as_str())?,
            Piece::Node(Node::Wrapped(w)) => out.write_str(w.text())?,
            Piece::Node(Node::Joined(j)) => {
                // joinError.Error: the members' texts joined by newlines.
                for (i, m) in j.errors().iter().enumerate().rev() {
                    stack.push(Piece::Node(node(m)));
                    if i != 0 {
                        stack.push(Piece::Newline);
                    }
                }
            }
            Piece::Node(Node::Foreign(f)) => {
                // What the leaf returns is not consulted: an error of its
                // own is ignored, and a failure of `out` it swallowed still
                // ends the rendering.
                let _ = write!(out, "{}", f.as_dyn());
                if out.sink_failed {
                    return Err(fmt::Error);
                }
            }
        }
    }
    Ok(())
}

/// `(*Error).Error` (`errors/errors.go:247-290`): writes the levels of an
/// `*Error` chain and leaves its non-`*Error` leaf, if any, on `stack`.
fn render_error<'a, W: fmt::Write + ?Sized>(
    mut e: &'a Error,
    sep: &str,
    out: &mut Counted<'_, W>,
    stack: &mut Vec<Piece<'a>>,
) -> fmt::Result {
    let start = out.len;
    let mut last_op: &str = "";
    let mut last_kind = Kind::Other;
    loop {
        let mut pad = false;
        if !e.op.is_empty() && e.op.as_str() != last_op {
            out.write_str(e.op.as_str())?;
            pad = true;
            last_op = e.op.as_str();
        }
        if e.kind != Kind::Other && e.kind != last_kind {
            if pad {
                out.write_str(": ")?;
            }
            out.write_str(e.kind.as_str())?;
            pad = true;
            last_kind = e.kind;
        }
        match &e.err {
            None => break,
            Some(AnyError::Error(inner)) => {
                if pad {
                    out.write_str(sep)?;
                }
                e = inner;
            }
            Some(leaf) => {
                if pad {
                    out.write_str(": ")?;
                }
                stack.push(Piece::EndError(start));
                stack.push(Piece::Node(node(leaf)));
                return Ok(());
            }
        }
    }
    if out.len == start {
        out.write_str(Kind::Other.as_str())?;
    }
    Ok(())
}
