// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2018-2019 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The errors `pgpwordlist` and `walletseed` return, rendered as dcrwallet's
//! `errors.Error` renders them.
//!
//! dcrwallet's `errors` package is ported separately (`purse-errors`); until
//! the two are bridged this module carries the parts of it these packages
//! need: the operation, the kind, and `(*Error).Error()`'s rendering of a
//! chain of nested errors.

use std::fmt;

use zeroize::Zeroizing;

/// The separator the dcrwallet daemon puts between nested errors.
///
/// dcrwallet's `main` package sets `errors.Separator = ":: "` in `init`
/// (`dcrwallet.go:36-39`), so this is the form JSON-RPC and gRPC clients
/// see. [`Error`]'s `Display` uses it.
pub const DAEMON_SEPARATOR: &str = ":: ";

/// The default of dcrwallet's `errors.Separator` (`errors/errors.go:27`),
/// seen by Go tests, tools and library users that do not override it.
pub const LIBRARY_SEPARATOR: &str = ":\n\t";

/// The operation an error was raised in (dcrwallet's `errors.Op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Op {
    /// `pgpwordlist.DecodeMnemonics`.
    DecodeMnemonics,
    /// `walletseed.GenerateRandomSeed`.
    GenerateRandomSeed,
    /// `walletseed.DecodeUserInput`.
    DecodeUserInput,
}

impl Op {
    /// The operation string as dcrwallet spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Op::DecodeMnemonics => "pgpwordlist.DecodeMnemonics",
            Op::GenerateRandomSeed => "walletseed.GenerateRandomSeed",
            Op::DecodeUserInput => "walletseed.DecodeUserInput",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The class of an error (the subset of dcrwallet's `errors.Kind` these
/// packages use).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// `errors.Other` (value 0): unclassified; never appears in error text.
    Other,
    /// `errors.Invalid` (value 2): invalid operation.
    Invalid,
    /// `errors.Encoding` (value 7): invalid encoding.
    Encoding,
}

impl ErrorKind {
    /// The Go constant's name (`"Other"`, `"Invalid"`, `"Encoding"`).
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Other => "Other",
            ErrorKind::Invalid => "Invalid",
            ErrorKind::Encoding => "Encoding",
        }
    }

    /// dcrwallet's `Kind.String()`, the text the kind contributes to an
    /// error message.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Other => "unclassified error",
            ErrorKind::Invalid => "invalid operation",
            ErrorKind::Encoding => "invalid encoding",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What went wrong: the innermost error of dcrwallet's chain, or a nested
/// [`Error`].
///
/// `Display` writes exactly the leaf text dcrwallet would put after the
/// last `": "` (for [`ErrorCause::Nested`], the nested error rendered as
/// [`Error`]'s `Display` renders it).
///
/// # What is zeroed
///
/// A word quoted from the caller's input ([`ErrorCause::UnknownWord`],
/// [`ErrorCause::WrongPosition`]) may be a word of a seed, so it is held in a
/// zero-on-drop buffer and `Debug` does not print it. Rendering writes the
/// word: [`Error::render`] returns a zero-on-drop string, and `Display`
/// streams straight into the caller's formatter without intermediate
/// copies. Whatever that formatter writes into (a `to_string()` result, a
/// log buffer) belongs to the caller and is not zeroed by this crate. The
/// other fields (a hex byte value, a position, a random source's message)
/// are not secret and are not zeroed.
#[non_exhaustive]
pub enum ErrorCause {
    /// `word <word> is not in the PGP word list`.
    UnknownWord {
        /// The word as given, after white-space trimming, in its original
        /// case.
        word: Zeroizing<String>,
    },
    /// `word <word> is not valid at position <position>, check for missing
    /// words`.
    WrongPosition {
        /// The word as given, after white-space trimming, in its original
        /// case.
        word: Zeroizing<String>,
        /// The zero-based count of words accepted before this one (empty
        /// words are not counted).
        position: usize,
    },
    /// Go's `hex.InvalidByteError`: `encoding/hex: invalid byte: <%#U>`.
    InvalidHexByte(u8),
    /// Go's `hex.ErrLength`: `encoding/hex: odd length hex string`.
    OddLengthHex,
    /// `checksum mismatch`.
    ChecksumMismatch,
    /// dcrd's `hdkeychain.ErrInvalidSeedLen`: `seed length must be between
    /// 128 and 512 bits`.
    InvalidSeedLen,
    /// The random source failed; the text is the source's own message.
    RandomSource(String),
    /// A nested error (Go's `*errors.Error` in the `Err` field).
    Nested(Box<Error>),
}

impl ErrorCause {
    /// Writes the leaf text; a nested error is rendered with the daemon
    /// separator.
    fn write_leaf<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            ErrorCause::UnknownWord { word } => {
                out.write_str("word ")?;
                out.write_str(word)?;
                out.write_str(" is not in the PGP word list")
            }
            ErrorCause::WrongPosition { word, position } => {
                out.write_str("word ")?;
                out.write_str(word)?;
                write!(
                    out,
                    " is not valid at position {position}, check for missing words"
                )
            }
            ErrorCause::InvalidHexByte(b) => {
                out.write_str("encoding/hex: invalid byte: ")?;
                write_go_sharp_u(out, *b)
            }
            ErrorCause::OddLengthHex => out.write_str("encoding/hex: odd length hex string"),
            ErrorCause::ChecksumMismatch => out.write_str("checksum mismatch"),
            ErrorCause::InvalidSeedLen => {
                out.write_str("seed length must be between 128 and 512 bits")
            }
            ErrorCause::RandomSource(msg) => out.write_str(msg),
            ErrorCause::Nested(inner) => inner.write_rendered(out, DAEMON_SEPARATOR),
        }
    }
}

impl fmt::Display for ErrorCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_leaf(f)
    }
}

impl fmt::Debug for ErrorCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCause::UnknownWord { .. } => f.write_str("UnknownWord { word: <redacted> }"),
            ErrorCause::WrongPosition { position, .. } => f
                .debug_struct("WrongPosition")
                .field("word", &format_args!("<redacted>"))
                .field("position", position)
                .finish(),
            ErrorCause::InvalidHexByte(b) => f.debug_tuple("InvalidHexByte").field(b).finish(),
            ErrorCause::OddLengthHex => f.write_str("OddLengthHex"),
            ErrorCause::ChecksumMismatch => f.write_str("ChecksumMismatch"),
            ErrorCause::InvalidSeedLen => f.write_str("InvalidSeedLen"),
            ErrorCause::RandomSource(msg) => f.debug_tuple("RandomSource").field(msg).finish(),
            ErrorCause::Nested(inner) => f.debug_tuple("Nested").field(inner).finish(),
        }
    }
}

/// Writes Go's `fmt.Sprintf("%#U", rune(b))` for a byte value: `U+` and
/// four upper-case hex digits, then ` '<char>'` when `strconv.IsPrint`
/// holds for the rune (Latin-1: U+0020..U+007E and U+00A1..U+00FF except
/// U+00AD).
fn write_go_sharp_u<W: fmt::Write>(out: &mut W, b: u8) -> fmt::Result {
    write!(out, "U+{:04X}", u32::from(b))?;
    let printable = matches!(b, 0x20..=0x7e) || (matches!(b, 0xa1..=0xff) && b != 0xad);
    if printable {
        out.write_str(" '")?;
        out.write_char(char::from(b))?;
        out.write_char('\'')?;
    }
    Ok(())
}

/// A `fmt::Write` that only counts bytes, to size a buffer exactly.
struct CountingWriter(usize);

impl fmt::Write for CountingWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 = self.0.saturating_add(s.len());
        Ok(())
    }
}

/// An error from this crate: dcrwallet's `*errors.Error` with the fields
/// these packages set.
///
/// `Display` reproduces `(*Error).Error()` byte for byte with the daemon's
/// separator ([`DAEMON_SEPARATOR`]); [`Error::render`] takes another.
pub struct Error {
    op: Op,
    kind: ErrorKind,
    cause: ErrorCause,
}

impl Error {
    /// Builds an error. Go's `errors.E(op, kind, cause)`, without the
    /// collapsing of a nested error that adds nothing (no call site here
    /// produces one).
    pub(crate) fn new(op: Op, kind: ErrorKind, cause: ErrorCause) -> Self {
        Error { op, kind, cause }
    }

    /// The operation the error was raised in.
    pub fn op(&self) -> Op {
        self.op
    }

    /// The error's kind (the outermost one, as `errors.Is(err, kind)` sees
    /// it first).
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The cause: a leaf condition or a nested error.
    pub fn cause(&self) -> &ErrorCause {
        &self.cause
    }

    /// The innermost cause, following nested errors.
    pub fn root_cause(&self) -> &ErrorCause {
        let mut e = self;
        while let ErrorCause::Nested(inner) = &e.cause {
            e = inner;
        }
        &e.cause
    }

    /// Whether the chain wraps `hdkeychain.ErrInvalidSeedLen` (Go's
    /// `errors.Is(err, hdkeychain.ErrInvalidSeedLen)`, which dcrwallet's
    /// gRPC server maps to `InvalidArgument`).
    pub fn is_invalid_seed_len(&self) -> bool {
        matches!(self.root_cause(), ErrorCause::InvalidSeedLen)
    }

    /// Renders the error as `(*Error).Error()` does with `errors.Separator`
    /// set to `separator` ([`DAEMON_SEPARATOR`] or [`LIBRARY_SEPARATOR`]).
    ///
    /// The string is sized exactly before it is written, so it never
    /// reallocates, and is zeroed when dropped (the text may quote a seed
    /// word).
    pub fn render(&self, separator: &str) -> Zeroizing<String> {
        let mut counter = CountingWriter(0);
        // Writing to a counter or a String cannot fail.
        let _ = self.write_rendered(&mut counter, separator);
        let mut out = Zeroizing::new(String::with_capacity(counter.0));
        let _ = self.write_rendered(&mut *out, separator);
        debug_assert_eq!(out.len(), counter.0);
        out
    }

    /// `(*Error).Error()` (`errors/errors.go:247-290`): each level writes
    /// its op and kind unless equal to the last ones written, nested levels
    /// are joined by the separator and the leaf text follows `": "`.
    fn write_rendered<W: fmt::Write>(&self, out: &mut W, separator: &str) -> fmt::Result {
        let mut last_op: Option<Op> = None;
        let mut last_kind = ErrorKind::Other;
        let mut e = self;
        loop {
            let mut pad = false;
            if last_op != Some(e.op) {
                out.write_str(e.op.as_str())?;
                pad = true;
                last_op = Some(e.op);
            }
            if e.kind != ErrorKind::Other && e.kind != last_kind {
                if pad {
                    out.write_str(": ")?;
                }
                out.write_str(e.kind.as_str())?;
                pad = true;
                last_kind = e.kind;
            }
            if let ErrorCause::Nested(inner) = &e.cause {
                if pad {
                    out.write_str(separator)?;
                }
                e = inner;
                continue;
            }
            if pad {
                out.write_str(": ")?;
            }
            return e.cause.write_leaf(out);
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_rendered(f, DAEMON_SEPARATOR)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("op", &self.op)
            .field("kind", &self.kind)
            .field("cause", &self.cause)
            .finish()
    }
}

/// `source()` is `None`: `Display` already includes a nested error's text,
/// as Go's rendering does, so reporting it again as a source would print it
/// twice. The nested error is reachable through [`Error::cause`].
impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharp_u_matches_go_for_latin1() {
        let render = |b: u8| {
            let mut s = String::new();
            write_go_sharp_u(&mut s, b).unwrap();
            s
        };
        assert_eq!(render(b'g'), "U+0067 'g'");
        assert_eq!(render(b'\''), "U+0027 '''");
        assert_eq!(render(0x09), "U+0009");
        assert_eq!(render(0x7f), "U+007F");
        assert_eq!(render(0xa0), "U+00A0");
        assert_eq!(render(0xad), "U+00AD");
        assert_eq!(render(0xc3), "U+00C3 'Ã'");
        assert_eq!(render(0xff), "U+00FF 'ÿ'");
    }

    #[test]
    fn nested_rendering_dedups_kind() {
        let inner = Error::new(
            Op::DecodeMnemonics,
            ErrorKind::Encoding,
            ErrorCause::UnknownWord {
                word: Zeroizing::new("xyzzy".to_owned()),
            },
        );
        let outer = Error::new(
            Op::DecodeUserInput,
            ErrorKind::Encoding,
            ErrorCause::Nested(Box::new(inner)),
        );
        assert_eq!(
            outer.to_string(),
            "walletseed.DecodeUserInput: invalid encoding:: \
             pgpwordlist.DecodeMnemonics: word xyzzy is not in the PGP word list"
        );
        assert_eq!(
            outer.render(LIBRARY_SEPARATOR).as_str(),
            "walletseed.DecodeUserInput: invalid encoding:\n\t\
             pgpwordlist.DecodeMnemonics: word xyzzy is not in the PGP word list"
        );
        assert!(!format!("{outer:?}").contains("xyzzy"));
    }

    #[test]
    fn render_is_sized_exactly_and_cause_displays_leaf() {
        let inner = Error::new(
            Op::DecodeMnemonics,
            ErrorKind::Encoding,
            ErrorCause::WrongPosition {
                word: Zeroizing::new("Adroitness".to_owned()),
                position: 12,
            },
        );
        assert_eq!(
            inner.cause().to_string(),
            "word Adroitness is not valid at position 12, check for missing words"
        );
        let outer = Error::new(
            Op::DecodeUserInput,
            ErrorKind::Encoding,
            ErrorCause::Nested(Box::new(inner)),
        );
        for sep in [DAEMON_SEPARATOR, LIBRARY_SEPARATOR] {
            let r = outer.render(sep);
            assert_eq!(r.capacity(), r.len());
        }
        assert_eq!(
            outer.cause().to_string(),
            "pgpwordlist.DecodeMnemonics: invalid encoding: \
             word Adroitness is not valid at position 12, check for missing words"
        );
        assert_eq!(
            ErrorCause::InvalidHexByte(b'z').to_string(),
            "encoding/hex: invalid byte: U+007A 'z'"
        );
        assert!(std::error::Error::source(&outer).is_none());
    }

    #[test]
    fn other_kind_is_not_rendered() {
        let e = Error::new(
            Op::GenerateRandomSeed,
            ErrorKind::Other,
            ErrorCause::RandomSource("boom".to_owned()),
        );
        assert_eq!(e.to_string(), "walletseed.GenerateRandomSeed: boom");
    }
}
