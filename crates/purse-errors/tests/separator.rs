// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The process-wide separator against an explicit one. This file is its
//! own test process, so setting the separator cannot race other tests; its
//! one test does everything in sequence.

use purse_errors::{
    AnyError, DAEMON_SEPARATOR, Error, Kind, MULTILINE_SEPARATOR, Op, e, join, new, separator,
    set_separator,
};

/// A Joined holding an Error with a nested Error level, a Wrapped whose
/// text was fixed at build time, and a Foreign leaf printing an Error.
fn sample() -> (AnyError, AnyError) {
    let nested = e!(Op::new("a"), Kind::Io, e!(Op::new("b"), Kind::Invalid, "x"));
    let joined =
        join([Some(AnyError::from(nested.clone())), Some(new("plain"))]).expect("two errors");
    let wrapped = AnyError::wrap(format!("w: {nested}"), [AnyError::from(nested)]);
    (joined, wrapped)
}

#[derive(Debug)]
struct Printer(Error);

impl std::fmt::Display for Printer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "printer({})", self.0)
    }
}

impl std::error::Error for Printer {}

#[test]
fn explicit_and_process_wide_separators() {
    assert_eq!(separator(), MULTILINE_SEPARATOR);
    let (joined, wrapped_multi) = sample();

    set_separator(DAEMON_SEPARATOR);
    assert_eq!(separator(), DAEMON_SEPARATOR);

    // The explicit separator reaches Error levels inside a Joined.
    assert_eq!(
        joined.render(MULTILINE_SEPARATOR),
        "a: I/O error:\n\tb: invalid operation: x\nplain"
    );
    assert_eq!(
        joined.render(DAEMON_SEPARATOR),
        "a: I/O error:: b: invalid operation: x\nplain"
    );
    assert_eq!(joined.to_string(), joined.render(DAEMON_SEPARATOR));

    // A Wrapped text keeps the separator in force when it was built.
    assert_eq!(
        wrapped_multi.render(DAEMON_SEPARATOR),
        "w: a: I/O error:\n\tb: invalid operation: x"
    );

    // A Foreign leaf prints through its own Display, which uses the
    // process-wide separator whatever is passed to render.
    let inner = e!(Op::new("p"), Kind::Io, e!(Op::new("q"), Kind::Seed));
    let foreign = AnyError::foreign(Printer(inner));
    let outer = e!(Op::new("o"), foreign);
    assert_eq!(
        outer.render(MULTILINE_SEPARATOR),
        "o: printer(p: I/O error:: q: invalid seed)"
    );

    set_separator(MULTILINE_SEPARATOR);
    assert_eq!(joined.to_string(), joined.render(MULTILINE_SEPARATOR));
}
