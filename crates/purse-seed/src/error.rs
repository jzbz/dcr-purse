// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The leaf errors of `pgpwordlist` and `walletseed`, the sentinel
//! `hdkeychain.ErrInvalidSeedLen`, and helpers for the
//! [`purse_errors::Error`] chains this crate returns.
//!
//! The chains themselves are built with `purse-errors` exactly as dcrwallet
//! builds them with its `errors` package: same operations, kinds and
//! nesting, so their rendering under either separator, `as_kind` and
//! matching behave as Go's do.
//!
//! # What is wiped
//!
//! The only secret an error of this crate can carry is a word quoted from
//! the caller's input ([`WordError`]), which may be a seed word. It is held
//! in a zero-on-drop buffer inside the [`WordError`], which the chain holds
//! as a [`purse_errors::Foreign`] leaf; clones of the chain share that one
//! leaf, and the buffer is wiped when the last clone is dropped.
//!
//! - `Debug` of the chain ([`purse_errors::Error`] or
//!   [`purse_errors::AnyError`]) prints the leaf through [`WordError`]'s
//!   `Debug`, which prints `<redacted>` for the word.
//! - `Display`, `to_string()` and `render_to` write the word straight into
//!   the caller's sink, with no intermediate copy made by this crate or by
//!   `purse-errors`. The sink (a `to_string()` result, a log buffer, a
//!   network message) belongs to the caller and is not wiped.
//! - **`render(sep)` is not wiped any more.**
//!   [`purse_errors::Error::render`] and [`purse_errors::AnyError::render`]
//!   return an ordinary `String`: it is not wiped, and while it grew it may
//!   have been reallocated, leaving freed copies of a prefix behind. This
//!   crate's own `Error::render` used to return a `Zeroizing<String>`; the
//!   method keeps its name and its use (`let t = err.render(sep)`,
//!   `err.render(sep).as_str()`, `&*err.render(sep)`) still compiles, so a
//!   caller written against the old type loses the wiping with no compiler
//!   error. Call [`SeedErrorExt::render_zeroizing`] instead: it renders
//!   into a buffer sized exactly beforehand and wiped on drop, and is the
//!   form to use when the text is not sent on at once. `render_to` into a
//!   sink that wipes its memory is the other alternative.
//! - `matches` / `Error::matches`, and `is` / `Error::is` with an
//!   [`purse_errors::AnyError::Error`] target (Go's `errors.Is` with an
//!   `*Error` template), may render the candidate. This happens when every
//!   template level above matches on operation and kind and the template's
//!   underlying error at some depth is not an `Error`. The candidate's
//!   whole underlying error at that depth is then rendered with the
//!   process-wide separator into a temporary `String` that is not wiped:
//!   that includes any nested `Error` levels with their operation prefixes,
//!   and so the word (for a [`decode_user_input`] error, the nested
//!   [`decode_mnemonics`] error is rendered whole). The `String` may also
//!   leave freed copies behind from reallocation. A template that has no
//!   underlying error at that depth renders nothing. Nothing in dcrwallet
//!   matches seed errors that way.
//! - [`purse_errors::AnyError::find_foreign`] hands out the [`WordError`]
//!   itself, whose `Display` writes the word.
//! - The [`std::error::Error::source`] chain hands out each nested level and
//!   then the [`WordError`], and every link's `Display` includes the word. A
//!   reporter that walks `source()` and prints each link's `Display`
//!   (anyhow's or eyre's alternate or `Debug` format, tracing's error-chain
//!   formatting) therefore writes the word once per link: three times for a
//!   [`decode_user_input`] error, twice for a [`decode_mnemonics`] error.
//!   Callers that log seed errors should print `Display` or
//!   [`SeedErrorExt::render_zeroizing`] once, not the source chain.
//!
//! Not secret and not wiped: a position, an invalid hex byte, a random
//! source's message, the fixed texts. The input the caller passed in is the
//! caller's to wipe.
//!
//! Tests pin what safe Rust can observe of this: that `Display` streams the
//! pieces `render_to` writes, and that `render_zeroizing`'s writer refuses
//! an over-long write rather than reallocating the buffer. The absence of
//! freed heap copies in general (for example from inside a foreign leaf's
//! own `Display`) is checked by review only, since observing it would need
//! a custom global allocator, which needs `unsafe`.
//!
//! [`decode_user_input`]: crate::walletseed::decode_user_input
//! [`decode_mnemonics`]: crate::pgpwordlist::decode_mnemonics

use std::fmt;
use std::sync::LazyLock;

use purse_errors::{AnyError, Error};
use zeroize::Zeroizing;

/// The text of dcrd's `hdkeychain.ErrInvalidSeedLen` (`seed length must be
/// between 128 and 512 bits`).
pub const INVALID_SEED_LEN_TEXT: &str = "seed length must be between 128 and 512 bits";

/// dcrd's `hdkeychain.ErrInvalidSeedLen`, the sentinel error
/// [`crate::walletseed::generate_random_seed`] and
/// [`crate::walletseed::decode_user_input`] wrap for a seed of the wrong
/// length.
///
/// Go compares it by identity: dcrwallet's gRPC server maps
/// `errors.Is(err, hdkeychain.ErrInvalidSeedLen)` to `InvalidArgument`
/// (`internal/rpc/rpcserver/server.go:120`). It is one shared
/// [`AnyError`] text value, as `purse-errors` ports a Go sentinel: every
/// error of this crate wraps a clone of it, so
/// `err.is(&ERR_INVALID_SEED_LEN)` (or [`SeedErrorExt::is_invalid_seed_len`])
/// finds it anywhere in a chain, however the error was wrapped since. A new
/// value with the same text does not match, as in Go.
pub static ERR_INVALID_SEED_LEN: LazyLock<AnyError> =
    LazyLock::new(|| AnyError::new(INVALID_SEED_LEN_TEXT));

/// A word of a mnemonic that `pgpwordlist.DecodeMnemonics` rejects: the
/// leaf of its error, which quotes the word.
///
/// `Display` is Go's leaf text, the `errors.Errorf` message of
/// `pgpwordlist/pgpwordlist.go:47,51-52`. The word is held in a zero-on-drop
/// buffer and `Debug` prints `<redacted>` in its place; see "What is wiped"
/// in the [module documentation](self) for what rendering does with it.
#[non_exhaustive]
pub enum WordError {
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
}

impl WordError {
    /// The rejected word, after white-space trimming, in its original case.
    pub fn word(&self) -> &str {
        match self {
            WordError::UnknownWord { word } | WordError::WrongPosition { word, .. } => word,
        }
    }

    /// The position of a word of the wrong parity; `None` for an unknown
    /// word.
    pub fn position(&self) -> Option<usize> {
        match self {
            WordError::UnknownWord { .. } => None,
            WordError::WrongPosition { position, .. } => Some(*position),
        }
    }
}

/// Writes the leaf text straight into the formatter, word included.
impl fmt::Display for WordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WordError::UnknownWord { word } => {
                f.write_str("word ")?;
                f.write_str(word)?;
                f.write_str(" is not in the PGP word list")
            }
            WordError::WrongPosition { word, position } => {
                f.write_str("word ")?;
                f.write_str(word)?;
                write!(
                    f,
                    " is not valid at position {position}, check for missing words"
                )
            }
        }
    }
}

/// Prints the variant and position, never the word.
impl fmt::Debug for WordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WordError::UnknownWord { .. } => f.write_str("UnknownWord { word: <redacted> }"),
            WordError::WrongPosition { position, .. } => f
                .debug_struct("WrongPosition")
                .field("word", &format_args!("<redacted>"))
                .field("position", position)
                .finish(),
        }
    }
}

impl std::error::Error for WordError {}

/// An error of Go's `hex.DecodeString`, with Go's text as `Display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HexError {
    /// Go's `hex.InvalidByteError`: `encoding/hex: invalid byte: <%#U>`.
    InvalidByte(u8),
    /// Go's `hex.ErrLength`: `encoding/hex: odd length hex string`.
    OddLength,
}

impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HexError::InvalidByte(b) => {
                f.write_str("encoding/hex: invalid byte: ")?;
                write_go_sharp_u(f, *b)
            }
            HexError::OddLength => f.write_str("encoding/hex: odd length hex string"),
        }
    }
}

impl std::error::Error for HexError {}

/// Writes Go's `fmt.Sprintf("%#U", rune(b))` for a byte value: `U+` and
/// four upper-case hex digits, then ` '<char>'` when `strconv.IsPrint`
/// holds for the rune (Latin-1: U+0020..U+007E and U+00A1..U+00FF except
/// U+00AD).
fn write_go_sharp_u<W: fmt::Write + ?Sized>(out: &mut W, b: u8) -> fmt::Result {
    write!(out, "U+{:04X}", u32::from(b))?;
    let printable = matches!(b, 0x20..=0x7e) || (matches!(b, 0xa1..=0xff) && b != 0xad);
    if printable {
        out.write_str(" '")?;
        out.write_char(char::from(b))?;
        out.write_char('\'')?;
    }
    Ok(())
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for purse_errors::Error {}
    impl Sealed for purse_errors::AnyError {}
}

/// Helpers on the errors this crate returns, for [`purse_errors::Error`]
/// and [`purse_errors::AnyError`] alike (sealed: only those two implement
/// it).
///
/// [`render_zeroizing`](Self::render_zeroizing) is the replacement for this
/// crate's former `Error::render`, which returned a `Zeroizing<String>`:
/// `render(sep)` now resolves to [`purse_errors::Error::render`], whose
/// `String` is not wiped.
pub trait SeedErrorExt: sealed::Sealed {
    /// Go's `Error()` text rendered as if `errors.Separator` were `sep`
    /// ([`purse_errors::DAEMON_SEPARATOR`] or
    /// [`purse_errors::MULTILINE_SEPARATOR`]), in a buffer that is wiped
    /// when dropped: the text may quote a seed word.
    ///
    /// This is what this crate's `Error::render` returned before its errors
    /// became [`purse_errors::Error`]. `render(sep)` still compiles on
    /// them, and returns the same text, but now as an ordinary `String`
    /// that is not wiped ([`purse_errors::Error::render`]); a caller that
    /// relied on the old wiping must switch to this method, as no compiler
    /// error will say so.
    ///
    /// The text is the same as `render(sep)`'s. The buffer is sized exactly
    /// by a first pass that only counts bytes, then written by a second
    /// through a writer that refuses to grow it, so it is never reallocated
    /// (should a foreign leaf's `Display` write more the second time, the
    /// refused attempt is wiped and repeated in a larger buffer).
    ///
    /// ```
    /// use purse_seed::SeedErrorExt;
    /// use purse_seed::walletseed::decode_user_input;
    ///
    /// let err = decode_user_input("aardvark adroitness bogus").unwrap_err();
    /// // Not `err.render(..)`: that String is not wiped.
    /// let text = err.render_zeroizing(purse_errors::DAEMON_SEPARATOR);
    /// assert!(text.ends_with("word bogus is not in the PGP word list"));
    /// ```
    fn render_zeroizing(&self, sep: &str) -> Zeroizing<String>;

    /// Whether the chain wraps [`ERR_INVALID_SEED_LEN`] (Go's
    /// `errors.Is(err, hdkeychain.ErrInvalidSeedLen)`).
    fn is_invalid_seed_len(&self) -> bool;

    /// The innermost underlying error: for an `Error`, the underlying error
    /// of its innermost nested `Error` level (`None` if that level has
    /// none); any other value is its own leaf.
    ///
    /// For this crate's errors it is a [`WordError`], a [`HexError`] or a
    /// [`crate::walletseed::RandomSourceError`] as a
    /// [`purse_errors::Foreign`] leaf, [`ERR_INVALID_SEED_LEN`], or the text
    /// `checksum mismatch`. To reach a typed leaf directly, use
    /// `find_foreign::<WordError>()` and so on.
    fn leaf(&self) -> Option<&AnyError>;
}

/// A `fmt::Write` that only counts bytes, to size a buffer exactly.
struct CountingWriter(usize);

impl fmt::Write for CountingWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 = self.0.saturating_add(s.len());
        Ok(())
    }
}

/// A `fmt::Write` into a zero-on-drop string that refuses to grow it.
struct Bounded<'a>(&'a mut Zeroizing<String>);

impl fmt::Write for Bounded<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.0.capacity().saturating_sub(self.0.len());
        if s.len() > room {
            return Err(fmt::Error);
        }
        self.0.push_str(s);
        Ok(())
    }
}

/// [`SeedErrorExt::render_zeroizing`] over either type's `render_to`.
fn render_zeroizing_with(
    render_to: impl Fn(&mut dyn fmt::Write) -> fmt::Result,
) -> Zeroizing<String> {
    let mut counter = CountingWriter(0);
    // Counting cannot fail.
    let _ = render_to(&mut counter);
    let mut capacity = counter.0;
    loop {
        let mut out = Zeroizing::new(String::with_capacity(capacity));
        // The only error render_to returns is the sink's own, here a lack
        // of room.
        if render_to(&mut Bounded(&mut out)).is_ok() {
            return out;
        }
        capacity = capacity.saturating_mul(2).max(64);
    }
}

impl SeedErrorExt for Error {
    fn render_zeroizing(&self, sep: &str) -> Zeroizing<String> {
        render_zeroizing_with(|out| self.render_to(out, sep))
    }

    fn is_invalid_seed_len(&self) -> bool {
        self.is(&ERR_INVALID_SEED_LEN)
    }

    fn leaf(&self) -> Option<&AnyError> {
        let mut e = self;
        loop {
            match &e.err {
                Some(AnyError::Error(inner)) => e = inner,
                other => return other.as_ref(),
            }
        }
    }
}

impl SeedErrorExt for AnyError {
    fn render_zeroizing(&self, sep: &str) -> Zeroizing<String> {
        render_zeroizing_with(|out| self.render_to(out, sep))
    }

    fn is_invalid_seed_len(&self) -> bool {
        self.is(&ERR_INVALID_SEED_LEN)
    }

    fn leaf(&self) -> Option<&AnyError> {
        match self {
            AnyError::Error(e) => e.leaf(),
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use purse_errors::{DAEMON_SEPARATOR, Kind, MULTILINE_SEPARATOR, Op, e};

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

    fn word_chain(word: &str) -> Error {
        let inner = e!(
            Op::new("pgpwordlist.DecodeMnemonics"),
            Kind::Encoding,
            AnyError::foreign(WordError::WrongPosition {
                word: Zeroizing::new(word.to_owned()),
                position: 12,
            })
        );
        e!(Op::new("walletseed.DecodeUserInput"), Kind::Encoding, inner)
    }

    #[test]
    fn nested_rendering_dedups_kind_and_redacts_debug() {
        let outer = word_chain("Adroitness");
        let leaf = "word Adroitness is not valid at position 12, check for missing words";
        assert_eq!(
            outer.render(DAEMON_SEPARATOR),
            format!(
                "walletseed.DecodeUserInput: invalid encoding:: \
                 pgpwordlist.DecodeMnemonics: {leaf}"
            )
        );
        assert_eq!(
            outer.render(MULTILINE_SEPARATOR),
            format!(
                "walletseed.DecodeUserInput: invalid encoding:\n\t\
                 pgpwordlist.DecodeMnemonics: {leaf}"
            )
        );
        let leaf_value = outer.leaf().expect("leaf");
        assert_eq!(leaf_value.render(DAEMON_SEPARATOR), leaf);
        let w = outer.find_foreign::<WordError>().expect("word leaf");
        assert_eq!(w.word(), "Adroitness");
        assert_eq!(w.position(), Some(12));
        for debug in [
            format!("{outer:?}"),
            format!("{outer:#?}"),
            format!("{:?}", AnyError::from(outer.clone())),
            format!("{w:?}"),
        ] {
            assert!(!debug.contains("Adroitness"), "{debug}");
            assert!(debug.contains("<redacted>"), "{debug}");
        }
        let unknown = WordError::UnknownWord {
            word: Zeroizing::new("xyzzy".to_owned()),
        };
        assert_eq!(
            unknown.to_string(),
            "word xyzzy is not in the PGP word list"
        );
        assert_eq!(format!("{unknown:?}"), "UnknownWord { word: <redacted> }");
        assert_eq!(unknown.position(), None);
    }

    #[test]
    fn render_zeroizing_is_exact_and_equals_render() {
        let outer = word_chain("Adroitness");
        let any = AnyError::from(outer.clone());
        for sep in [DAEMON_SEPARATOR, MULTILINE_SEPARATOR] {
            let r = outer.render_zeroizing(sep);
            assert_eq!(r.capacity(), r.len());
            assert_eq!(r.as_str(), outer.render(sep));
            let r = any.render_zeroizing(sep);
            assert_eq!(r.capacity(), r.len());
            assert_eq!(r.as_str(), any.render(sep));
        }
    }

    /// A leaf whose text grows each time it is displayed.
    #[derive(Debug)]
    struct Growing(std::sync::atomic::AtomicUsize);

    impl fmt::Display for Growing {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let n = self.0.fetch_add(10, std::sync::atomic::Ordering::Relaxed);
            f.write_str(&"x".repeat(n))
        }
    }

    impl std::error::Error for Growing {}

    #[test]
    fn render_zeroizing_survives_a_leaf_that_grows() {
        let e = e!(
            Op::new("op"),
            AnyError::foreign(Growing(std::sync::atomic::AtomicUsize::new(5)))
        );
        // Counted at 5 bytes of leaf, written at 15 or more.
        let r = e.render_zeroizing(DAEMON_SEPARATOR);
        assert!(r.starts_with("op: xxxxxxxxxxxxxxx"), "{}", r.as_str());
        assert!(r.len() <= r.capacity());
        let g = e.find_foreign::<Growing>().expect("leaf");
        // Counted once (5), refused once rather than absorbed by growing the
        // buffer (15), written once more in a larger buffer (25): the leaf
        // was displayed three times, leaving the counter at 35.
        assert_eq!(g.0.load(std::sync::atomic::Ordering::Relaxed), 35);
        assert_eq!(r.as_str(), format!("op: {}", "x".repeat(25)));
    }

    #[test]
    fn bounded_refuses_instead_of_growing() {
        use std::fmt::Write as _;
        let mut s = Zeroizing::new(String::with_capacity(8));
        let ptr = s.as_ptr();
        let capacity = s.capacity();
        Bounded(&mut s).write_str("abcde").expect("fits");
        let over = "f".repeat(capacity.saturating_sub(s.len()).saturating_add(1));
        assert!(Bounded(&mut s).write_str(&over).is_err());
        assert_eq!((s.as_ptr(), s.capacity()), (ptr, capacity));
        assert_eq!(s.as_str(), "abcde");
        let room = capacity.saturating_sub(s.len());
        Bounded(&mut s)
            .write_str(&"z".repeat(room))
            .expect("fits exactly");
        assert!(Bounded(&mut s).write_str("!").is_err());
        assert_eq!(
            (s.as_ptr(), s.capacity(), s.len()),
            (ptr, capacity, capacity)
        );
    }

    /// A leaf that ignores every write error and reports success, whose
    /// text is short the first time it is displayed and long afterwards.
    #[derive(Debug)]
    struct SwallowingGrower(std::sync::atomic::AtomicUsize);

    impl fmt::Display for SwallowingGrower {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let first = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
            let pieces: &[&str] = if first {
                &["short"]
            } else {
                &["a much longer text, ", "in ", "pieces"]
            };
            for piece in pieces {
                let _ = f.write_str(piece);
            }
            Ok(())
        }
    }

    impl std::error::Error for SwallowingGrower {}

    #[test]
    fn render_zeroizing_is_whole_when_a_leaf_swallows_the_refusal() {
        for sep in [DAEMON_SEPARATOR, MULTILINE_SEPARATOR] {
            let e = e!(
                Op::new("outer"),
                e!(
                    Op::new("op"),
                    Kind::Encoding,
                    AnyError::foreign(SwallowingGrower(std::sync::atomic::AtomicUsize::new(0)))
                )
            );
            let r = e.render_zeroizing(sep);
            assert_eq!(r.as_str(), e.render(sep));
            assert_eq!(
                r.as_str(),
                format!("outer: invalid encoding{sep}op: a much longer text, in pieces")
            );
            let any = AnyError::from(e!(
                Op::new("op"),
                AnyError::foreign(SwallowingGrower(std::sync::atomic::AtomicUsize::new(0)))
            ));
            let r = any.render_zeroizing(sep);
            assert_eq!(r.as_str(), any.render(sep));
        }
    }

    #[test]
    fn hex_error_texts() {
        assert_eq!(
            HexError::InvalidByte(b'z').to_string(),
            "encoding/hex: invalid byte: U+007A 'z'"
        );
        assert_eq!(
            HexError::OddLength.to_string(),
            "encoding/hex: odd length hex string"
        );
    }

    #[test]
    fn sentinel_matches_by_identity_through_any_wrapping() {
        let e = e!(
            Op::new("walletseed.GenerateRandomSeed"),
            Kind::Invalid,
            &*ERR_INVALID_SEED_LEN
        );
        assert!(e.is_invalid_seed_len());
        // Wrapped again, as a caller's errors.E(op, err) would.
        let outer = AnyError::from(e!(Op::new("rpc.Something"), Kind::Seed, e.clone()));
        assert!(outer.is_invalid_seed_len());
        let fmt_wrapped = AnyError::wrap(format!("ctx: {outer}"), [outer.clone()]);
        assert!(fmt_wrapped.is_invalid_seed_len());
        let joined =
            purse_errors::join([Some(purse_errors::new("other")), Some(outer)]).expect("members");
        assert!(joined.is_invalid_seed_len());
        // The same text in a new value is a different error, as in Go.
        let lookalike = e!(
            Op::new("walletseed.GenerateRandomSeed"),
            Kind::Invalid,
            INVALID_SEED_LEN_TEXT
        );
        assert!(!lookalike.is_invalid_seed_len());
        assert_eq!(
            lookalike.render(DAEMON_SEPARATOR),
            e.render(DAEMON_SEPARATOR)
        );
        assert!(ERR_INVALID_SEED_LEN.is_invalid_seed_len());
        assert!(!AnyError::new(INVALID_SEED_LEN_TEXT).is_invalid_seed_len());
    }

    #[test]
    fn leaf_of_values_without_nesting() {
        let k = AnyError::Kind(Kind::Seed);
        assert!(k.leaf().is_some_and(|l| l.identical(&k)));
        let bare = e!(Op::new("op"), Kind::Io);
        assert!(bare.leaf().is_none());
    }
}
