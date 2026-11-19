// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Ports of `errors/errors_test.go`, and unit tests of the Rust-only
//! surface. Rendering and matching are pinned against Go by
//! `tests/oracle.rs`.

use super::*;

/// `depth` from `errors_test.go`: the number of levels, counting a
/// non-`Error` leaf as one.
fn depth(err: Option<&AnyError>) -> usize {
    match err {
        None => 0,
        Some(AnyError::Error(e)) => depth(e.err.as_ref()).saturating_add(1),
        Some(_) => 1,
    }
}

fn depth_of(e: &Error) -> usize {
    depth(e.err.as_ref()).saturating_add(1)
}

/// `eq` from `errors_test.go`: field equality, the underlying errors
/// compared with Go's `==`.
fn eq(e0: &Error, e1: &Error) -> bool {
    if e0.op != e1.op || e0.kind != e1.kind {
        return false;
    }
    match (&e0.err, &e1.err) {
        (None, None) => true,
        (Some(a), Some(b)) => a.identical(b),
        _ => false,
    }
}

#[test]
fn collapse() {
    let e0 = e!(Op::new("abc"));
    let e0 = e!(e0, Kind::Passphrase);
    assert_eq!(depth_of(&e0), 1, "e0 was not collapsed");

    let e1 = e!(Op::new("abc"), Kind::Passphrase);
    assert!(eq(&e0, &e1), "e0 was not collapsed to e1");
}

#[test]
fn is() {
    let base = new("base error");
    let e = AnyError::from(e!(base.clone(), Op::new("operation"), Kind::Permission));
    assert!(e.is(&base), "no match on base errors");
    assert!(
        !e.is(&e!("different error").into()),
        "match on different error strings"
    );
    assert!(
        e.is(&e!(Op::new("operation")).into()),
        "no match on operation"
    );
    assert!(
        !e.is(&e!(Op::new("different operation")).into()),
        "match on different operation"
    );
    assert!(e.is(&e!(Kind::Permission).into()), "no match on kind");
    assert!(!e.is(&e!(Kind::Invalid).into()), "match on different kind");
}

#[test]
fn double_wrapped_error_with_kind() {
    let err = e!(Kind::Invalid, "abc");
    // Wrap the error again.
    let err = e!(err, "def");
    // Now try to match against the kind.
    assert!(
        err.is_kind(Kind::Invalid),
        "is_kind returned false for {err:?}, wrapped: {:?}",
        err.wrapped()
    );
}

#[test]
fn kind_codes_round_trip() {
    for (i, k) in Kind::ALL.iter().enumerate() {
        assert_eq!(usize::from(k.code()), i);
        assert_eq!(Kind::from_code(i64::from(k.code())), Some(*k));
        assert_eq!(Kind::describe_code(i64::from(k.code())), k.as_str());
        assert_eq!(k.to_string(), k.as_str());
    }
    for code in [-1, 21, 22, i64::MIN, i64::MAX] {
        assert_eq!(Kind::from_code(code), None);
        assert_eq!(Kind::describe_code(code), UNKNOWN_KIND_TEXT);
    }
    assert_eq!(Kind::default(), Kind::Other);
}

#[test]
fn op_forms() {
    const OP: Op = Op::new("wallet.Foo");
    assert_eq!(OP.as_str(), "wallet.Foo");
    assert_eq!(
        opf!("wallet.Bar({})", 7),
        Op::from("wallet.Bar(7)".to_owned())
    );
    assert!(Op::NONE.is_empty());
    assert!(Op::default().is_empty());
    assert_eq!(Op::from("x"), Op::from(String::from("x")));
}

#[test]
fn errorf_is_a_plain_text_error() {
    let err = errorf!("bad length {}", 3);
    assert!(matches!(&err, AnyError::Text(t) if t.as_str() == "bad length 3"));
    assert!(err.wrapped().is_empty());
}

#[test]
fn new_values_have_distinct_identities() {
    let a = new("x");
    let b = new("x");
    assert!(!a.identical(&b));
    assert!(a.identical(&a.clone()));
    assert!(!a.is(&b));
    assert!(AnyError::Kind(Kind::Io).identical(&AnyError::Kind(Kind::Io)));
}

#[test]
fn option_args_none_is_ignored() {
    let none: Option<AnyError> = None;
    let e = e!(Op::new("a"), none);
    assert!(e.err.is_none());
    assert_eq!(e.render(MULTILINE_SEPARATOR), "a");
    let some = Some(new("leaf"));
    let e = e!(Op::new("a"), some);
    assert_eq!(e.render(MULTILINE_SEPARATOR), "a: leaf");
}

#[test]
fn any_error_args_dispatch_by_variant() {
    // A bare Kind passed as an error value sets the kind, as in Go's type
    // switch, where the Kind case comes before the error case.
    let e = e!(Op::new("a"), AnyError::Kind(Kind::Io));
    assert_eq!(e.kind, Kind::Io);
    assert!(e.err.is_none());
    // An Error inside an AnyError is collapsed like a direct Error.
    let inner = AnyError::from(e!(Kind::Invalid, "x"));
    let e = e!(Op::new("a"), &inner);
    assert!(matches!(&e.err, Some(AnyError::Text(t)) if t.as_str() == "x"));
    assert_eq!(e.kind, Kind::Invalid);
}

#[derive(Debug, PartialEq)]
struct Custom(u8);

impl std::fmt::Display for Custom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "custom {}", self.0)
    }
}

impl std::error::Error for Custom {}

#[test]
fn foreign_errors() {
    let leaf = AnyError::foreign(Custom(9));
    assert_eq!(leaf.to_string(), "custom 9");
    let wrapped = AnyError::wrap(format!("ctx: {leaf}"), [leaf.clone()]);
    let e = AnyError::from(e!(Op::new("a"), Kind::Io, wrapped));
    assert_eq!(e.render(DAEMON_SEPARATOR), "a: I/O error: ctx: custom 9");
    assert_eq!(e.find_foreign::<Custom>(), Some(&Custom(9)));
    assert!(e.find_foreign::<std::fmt::Error>().is_none());
    assert!(e.is(&leaf));
    assert!(!e.is(&AnyError::foreign(Custom(9))));
}

#[test]
fn foreign_routes_model_types_to_their_variants() {
    let err = e!(Op::new("a"), Kind::Io);
    assert!(matches!(AnyError::foreign(err.clone()), AnyError::Error(_)));
    assert!(matches!(
        AnyError::foreign(Kind::Seed),
        AnyError::Kind(Kind::Seed)
    ));
    let t = new("t");
    assert!(AnyError::foreign(t.clone()).identical(&t));
}

#[test]
fn join_drops_none() {
    assert!(join([None, None]).is_none());
    assert!(join(Vec::<AnyError>::new()).is_none());
    let a = new("a");
    let j = join([None, Some(a.clone())]).expect("one error");
    assert!(matches!(&j, AnyError::Joined(x) if x.errors().len() == 1));
    assert!(!j.identical(&a));
    assert!(j.is(&a));
}

#[test]
fn std_error_sources() {
    use std::error::Error as _;
    let leaf = e!(Op::new("b"), Kind::Io);
    let e = e!(Op::new("a"), Kind::Invalid, leaf);
    let src = e.source().expect("nested error");
    assert_eq!(src.to_string(), "b: I/O error");
    // A leaf Error's source is its kind, as Go's Unwrap returns it.
    let next = src.source().expect("kind");
    assert_eq!(next.to_string(), "I/O error");
    assert!(next.source().is_none());
}

#[test]
fn stacks_are_recorded_and_inherited() {
    let inner = with_stack!(Op::new("a"), Kind::Io);
    assert!(inner.stack().is_some());
    assert_eq!(inner.stacks().len(), 1);
    // Collapsing keeps the inner stack.
    let collapsed = e!(inner.clone(), Op::new("a"));
    assert_eq!(depth_of(&collapsed), 1);
    assert!(collapsed.stack().is_some());
    // Nesting keeps one stack per level.
    let outer = with_stack!(Op::new("b"), inner);
    assert_eq!(outer.stacks().len(), 2);
    assert_eq!(AnyError::from(outer).stacks().len(), 2);
    assert!(new("x").stacks().is_empty());
}

/// QK-0628: Go panics in `match` here; the level does not match instead.
#[test]
fn template_leaf_against_level_without_leaf_does_not_match() {
    let e = AnyError::from(e!(Op::new("x"), Kind::Invalid));
    let template = e!("text");
    assert!(!e.matches(&template));
    assert!(!e.is(&AnyError::from(template.clone())));
    // The walk continues past the level that panics in Go.
    let deeper = AnyError::from(Error::from_parts(
        Op::new("x"),
        Kind::Invalid,
        Some(AnyError::from(e!("text"))),
    ));
    let outer = AnyError::join([e, deeper]).expect("two errors");
    assert!(outer.matches(&template));
}

#[test]
fn foreign_recognises_boxed_and_shared_model_types() {
    use std::sync::Arc;

    // Arc<Error> keeps its identity.
    let arc = Arc::new(e!(Op::new("a"), Kind::Io));
    let any = AnyError::from(Arc::clone(&arc));
    let f = AnyError::foreign(Arc::clone(&arc));
    assert!(f.identical(&any));
    assert!(matches!(&f, AnyError::Error(x) if Arc::ptr_eq(x, &arc)));

    // Box<Error> and Error keep op and kind.
    for f in [
        AnyError::foreign(Box::new(e!(Op::new("b"), Kind::Seed))),
        AnyError::foreign(e!(Op::new("b"), Kind::Seed)),
    ] {
        let AnyError::Error(x) = &f else {
            panic!("not an Error: {f:?}");
        };
        assert_eq!((x.op.as_str(), x.kind), ("b", Kind::Seed));
        assert_eq!(f.as_kind(), Some(Kind::Seed));
    }

    // AnyError, Box<AnyError> and Arc<AnyError> keep identity.
    let t = new("t");
    assert!(AnyError::foreign(t.clone()).identical(&t));
    assert!(AnyError::foreign(Box::new(t.clone())).identical(&t));
    assert!(AnyError::foreign(Arc::new(t.clone())).identical(&t));
    let shared = Arc::new(t.clone());
    let _keep = Arc::clone(&shared);
    assert!(AnyError::foreign(shared).identical(&t));

    // Kind, Box<Kind> and Arc<Kind>.
    for f in [
        AnyError::foreign(Kind::Locked),
        AnyError::foreign(Box::new(Kind::Locked)),
        AnyError::foreign(Arc::new(Kind::Locked)),
    ] {
        assert!(matches!(f, AnyError::Kind(Kind::Locked)));
    }

    // Deeper nesting stays a foreign leaf.
    let deep = AnyError::foreign(Box::new(Arc::new(e!(Op::new("c"), Kind::Io))));
    assert!(matches!(deep, AnyError::Foreign(_)));
    assert_eq!(deep.as_kind(), None);
}

#[test]
fn source_chain_reaches_concrete_types() {
    use std::error::Error as _;

    let io = std::io::Error::other("disk gone");
    let e = e!(
        Op::new("a"),
        Kind::Io,
        e!(Op::new("b"), AnyError::foreign(io))
    );
    let mut found_inner = false;
    let mut found_io = false;
    let mut src = e.source();
    while let Some(s) = src {
        if let Some(inner) = s.downcast_ref::<Error>() {
            assert_eq!(inner.op.as_str(), "b");
            found_inner = true;
        }
        if let Some(io) = s.downcast_ref::<std::io::Error>() {
            assert_eq!(io.to_string(), "disk gone");
            found_io = true;
        }
        src = s.source();
    }
    assert!(found_inner && found_io);

    // A root AnyError::Error does not hand out its own inner error (which
    // would repeat its message) as its source.
    let root = AnyError::from(e!(Op::new("x"), Kind::Io, "leaf"));
    let first = root.source().expect("leaf");
    assert_eq!(first.to_string(), "leaf");

    // A Kind source downcasts to Kind.
    let leaf = e!(Op::new("x"), Kind::Seed);
    assert_eq!(
        leaf.source().and_then(|s| s.downcast_ref::<Kind>()),
        Some(&Kind::Seed)
    );
}

#[test]
fn borrowed_op_and_arc_args() {
    use std::sync::Arc;

    let op = opf!("wallet.Foo({})", 1);
    let e = e!(&op, Kind::Io);
    assert_eq!(e.op, op);
    let inner = Arc::new(e!(Op::new("b"), Kind::Io, "x"));
    let outer = e!(Op::new("a"), &inner);
    assert!(matches!(&outer.err, Some(AnyError::Error(x)) if Arc::ptr_eq(x, &inner)));
}

#[test]
fn identical_on_error_values() {
    use std::sync::Arc;

    let arc = Arc::new(e!(Op::new("a"), Kind::Io));
    let a = AnyError::Error(Arc::clone(&arc));
    let b = AnyError::Error(arc);
    assert!(a.identical(&b));
    let c = AnyError::from(e!(Op::new("a"), Kind::Io));
    let d = AnyError::from(e!(Op::new("a"), Kind::Io));
    assert!(!c.identical(&d));
    assert!(!a.identical(&c));
    // Equal-but-distinct Error values still match as templates.
    assert!(c.is(&d));
}

/// A foreign leaf that writes some text and then fails on its own.
#[derive(Debug)]
struct Failing;

impl std::fmt::Display for Failing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("half")?;
        Err(std::fmt::Error)
    }
}

impl std::error::Error for Failing {}

/// A sink that takes `room` bytes and then fails, recording each piece.
struct Sink {
    room: usize,
    pieces: Vec<String>,
}

impl std::fmt::Write for Sink {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if s.len() > self.room {
            return Err(std::fmt::Error);
        }
        self.room = self.room.saturating_sub(s.len());
        self.pieces.push(s.to_owned());
        Ok(())
    }
}

/// A chain with every variant: nested Error levels, an empty level, a
/// joined value, a wrapped value, a bare kind and foreign leaves.
fn every_variant() -> AnyError {
    let nested = e!(
        Op::new("a"),
        Kind::Io,
        e!(Op::new("b"), Kind::Io, AnyError::foreign(Custom(3)))
    );
    let empty = Error::from_parts(Op::NONE, Kind::Other, None);
    let joined = join([
        Some(AnyError::from(nested)),
        Some(AnyError::from(empty)),
        Some(AnyError::wrap("w", [new("x")])),
        Some(AnyError::Kind(Kind::Seed)),
        Some(new("")),
    ])
    .expect("members");
    AnyError::from(e!(Op::new("top"), Kind::Invalid, joined))
}

#[test]
fn render_to_writes_what_render_returns() {
    let err = every_variant();
    for sep in [DAEMON_SEPARATOR, MULTILINE_SEPARATOR, "", "<>"] {
        let mut s = String::new();
        err.render_to(&mut s, sep)
            .expect("a String takes everything");
        assert_eq!(s, err.render(sep));
        if let AnyError::Error(e) = &err {
            let mut s = String::new();
            e.render_to(&mut s, sep).expect("a String takes everything");
            assert_eq!(s, e.render(sep));
        }
    }
    // Display streams with the process-wide separator, which this test does
    // not change.
    assert_eq!(err.to_string(), err.render(separator()));
}

#[test]
fn render_to_streams_without_an_intermediate_buffer() {
    // A foreign leaf's text arrives as the pieces its Display writes, not
    // as one pre-rendered string.
    let err = e!(Op::new("op"), Kind::Encoding, AnyError::foreign(Custom(7)));
    let mut sink = Sink {
        room: usize::MAX,
        pieces: Vec::new(),
    };
    err.render_to(&mut sink, DAEMON_SEPARATOR).expect("room");
    assert_eq!(
        sink.pieces,
        ["op", ": ", "invalid encoding", ": ", "custom ", "7"]
    );
}

#[test]
fn display_streams_without_an_intermediate_buffer() {
    // Display of both types hands the formatter the same pieces render_to
    // writes, not one pre-rendered string.
    use std::fmt::Write as _;
    let err = e!(Op::new("op"), Kind::Encoding, AnyError::foreign(Custom(7)));
    let any = AnyError::from(err.clone());
    let mut want = Sink {
        room: usize::MAX,
        pieces: Vec::new(),
    };
    err.render_to(&mut want, separator()).expect("room");
    assert!(want.pieces.len() > 1);
    for d in [&err as &dyn std::fmt::Display, &any] {
        let mut sink = Sink {
            room: usize::MAX,
            pieces: Vec::new(),
        };
        write!(sink, "{d}").expect("room");
        assert_eq!(sink.pieces, want.pieces);
    }
}

/// A foreign leaf that ignores every write error and reports success,
/// writing pieces of decreasing length so that a sink short of room would
/// still take a later one.
#[derive(Debug)]
struct Swallowing;

impl std::fmt::Display for Swallowing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for piece in ["a long piece", "mid", "s"] {
            let _ = f.write_str(piece);
        }
        Ok(())
    }
}

impl std::error::Error for Swallowing {}

#[test]
fn a_leaf_that_swallows_a_sink_failure_cannot_hide_it() {
    let err = join([
        Some(AnyError::from(e!(
            Op::new("a"),
            AnyError::foreign(Swallowing)
        ))),
        Some(new("after")),
    ])
    .expect("members");
    let full = err.render(DAEMON_SEPARATOR);
    assert_eq!(full, "a: a long piecemids\nafter");
    let AnyError::Joined(j) = &err else {
        panic!("not joined");
    };
    let AnyError::Error(first) = &j.errors()[0] else {
        panic!("not an Error");
    };
    // No separator is written (one level, joined by newlines), so Display's
    // process-wide separator renders the same text.
    let renders: [&dyn Fn(&mut Sink) -> std::fmt::Result; 2] =
        [&|s| err.render_to(s, DAEMON_SEPARATOR), &|s| {
            use std::fmt::Write as _;
            write!(s, "{err}")
        }];
    for room in 0..full.len() {
        for render in renders {
            let mut sink = Sink {
                room,
                pieces: Vec::new(),
            };
            assert!(render(&mut sink).is_err(), "room {room}");
            let written = sink.pieces.concat();
            assert!(full.starts_with(&written), "room {room}: {written:?}");
        }
        // The same through an Error's own render_to, cut inside its text.
        let own = first.render(DAEMON_SEPARATOR);
        if room < own.len() {
            let mut sink = Sink {
                room,
                pieces: Vec::new(),
            };
            assert!(first.render_to(&mut sink, DAEMON_SEPARATOR).is_err());
            assert!(own.starts_with(&sink.pieces.concat()), "room {room}");
        }
    }
}

#[test]
fn render_to_stops_at_a_failing_sink() {
    let err = every_variant();
    let full = err.render(DAEMON_SEPARATOR);
    for room in 0..full.len() {
        let mut sink = Sink {
            room,
            pieces: Vec::new(),
        };
        assert!(err.render_to(&mut sink, DAEMON_SEPARATOR).is_err());
        let written = sink.pieces.concat();
        assert!(full.starts_with(&written), "room {room}");
    }
}

#[test]
fn a_failing_foreign_display_does_not_cut_the_text() {
    // As before render_to existed: the text the leaf wrote is kept and the
    // rest follows; neither render, render_to nor Display reports an error.
    let err = join([
        Some(AnyError::from(e!(Op::new("a"), AnyError::foreign(Failing)))),
        Some(new("after")),
    ])
    .expect("members");
    assert_eq!(err.render(DAEMON_SEPARATOR), "a: half\nafter");
    let mut s = String::new();
    assert!(err.render_to(&mut s, DAEMON_SEPARATOR).is_ok());
    assert_eq!(s, "a: half\nafter");
    assert_eq!(err.to_string(), "a: half\nafter");
    let empty = AnyError::from(Error::from_parts(
        Op::NONE,
        Kind::Other,
        Some(AnyError::foreign(Failing)),
    ));
    assert_eq!(empty.to_string(), "half");
}
