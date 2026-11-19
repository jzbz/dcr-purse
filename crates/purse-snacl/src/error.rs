// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2014-2024 The btcsuite developers
// Copyright (c) 2015-2024 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The errors this crate returns, shaped and rendered as the dcrwallet
//! `*errors.Error` values `snacl`, `kdf` and `udb`'s `seal`/`unseal` build.
//!
//! None of these errors carries secret material: every leaf text is fixed or
//! quotes only a length, a parameter or a random source's message.

use std::fmt::{self, Write as _};

use purse_errors::{AnyError, DAEMON_SEPARATOR, Kind};

/// The operation an error was raised in (dcrwallet's `errors.Op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Op {
    /// `cryptokey.Encrypt` (`snacl.go:36`; Go never returns an error from it).
    CryptoKeyEncrypt,
    /// `cryptokey.Decrypt` (`snacl.go:48`).
    CryptoKeyDecrypt,
    /// `snacl.GenerateCryptoKey` (`snacl.go:75`; Go never returns an error
    /// from it).
    GenerateCryptoKey,
    /// `secretkey.Unmarshal` (`snacl.go:155`).
    SecretKeyUnmarshal,
    /// `secretkey.DeriveKey` (`snacl.go:193`).
    SecretKeyDeriveKey,
    /// `secretkey.Encrypt` (`snacl.go:209`).
    SecretKeyEncrypt,
    /// `secretkey.Decrypt` (`snacl.go:219`).
    SecretKeyDecrypt,
    /// `snacl.NewSecretKey` (`snacl.go:229`).
    NewSecretKey,
}

impl Op {
    /// The operation string as dcrwallet spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Op::CryptoKeyEncrypt => "cryptokey.Encrypt",
            Op::CryptoKeyDecrypt => "cryptokey.Decrypt",
            Op::GenerateCryptoKey => "snacl.GenerateCryptoKey",
            Op::SecretKeyUnmarshal => "secretkey.Unmarshal",
            Op::SecretKeyDeriveKey => "secretkey.DeriveKey",
            Op::SecretKeyEncrypt => "secretkey.Encrypt",
            Op::SecretKeyDecrypt => "secretkey.Decrypt",
            Op::NewSecretKey => "snacl.NewSecretKey",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The key derivation function a resource error is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kdf {
    /// scrypt (the master keys).
    Scrypt,
    /// Argon2id (the per-account passphrases).
    Argon2,
}

impl Kdf {
    /// `"scrypt"` or `"argon2"`, the prefix Go's own messages for each use.
    pub fn as_str(self) -> &'static str {
        match self {
            Kdf::Scrypt => "scrypt",
            Kdf::Argon2 => "argon2",
        }
    }
}

impl fmt::Display for Kdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The innermost condition of an error: the leaf text dcrwallet's chain ends
/// in.
///
/// The variants marked "Go: ..." are where this crate returns an error and
/// Go does not: Go panics, or allocates without bound, or cannot fail there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCause {
    /// `missing nonce`: a crypto-key box shorter than its 24-byte nonce
    /// (`snacl.go:49-51`).
    MissingNonce,
    /// `bad marshalled data len <n>`: scrypt parameters that are not 88
    /// bytes long (`snacl.go:164-166`).
    BadMarshalledLen(usize),
    /// x/crypto scrypt's `scrypt: N must be > 1 and a power of 2`
    /// (`scrypt.go:196-198`).
    ScryptNotPowerOfTwo,
    /// x/crypto scrypt's `scrypt: parameters must be > 0`
    /// (`scrypt.go:199-201`).
    ScryptNotPositive,
    /// x/crypto scrypt's `scrypt: parameters are too large`
    /// (`scrypt.go:202-204`).
    ScryptTooLarge,
    /// kdf's `invalid marshaled Argon2id parameters`: Argon2id parameters
    /// that are not 25 bytes long (`kdf.go:56-58`).
    InvalidArgon2idParams,
    /// `argon2: number of rounds too small`: a time cost of 0. Go: x/crypto
    /// argon2 panics with this message (`argon2.go:106-108`).
    Argon2RoundsTooSmall,
    /// `argon2: parallelism degree too low`: a thread count of 0. Go: x/crypto
    /// argon2 panics with this message (`argon2.go:109-111`).
    Argon2ParallelismTooLow,
    /// A memory cost below 8 KiB per thread. Go: x/crypto argon2 hashes the
    /// stored value into the initial block and works on `8 * threads` blocks
    /// instead (`argon2.go:114-117`); the Argon2 implementation this crate
    /// uses refuses such parameters, so their key cannot be computed.
    /// dcrwallet never writes them (it stores 262144 KiB and at most 255
    /// threads).
    Argon2MemoryBelowMinimum {
        /// The stored memory cost, in KiB.
        memory: u32,
        /// The stored thread count.
        threads: u8,
    },
    /// Parameters needing more memory than [`Limits`](crate::Limits)
    /// allows. Go: no limit; it allocates what the parameters ask for and
    /// fails (or the process dies) only when the allocation does.
    MemoryLimit {
        /// The function.
        kdf: Kdf,
        /// The bytes the parameters need.
        needed: u64,
        /// The limit in force.
        limit: u64,
    },
    /// Parameters needing more work than [`Limits`](crate::Limits) allows
    /// (scrypt: `N * r * p`; Argon2: time cost times memory blocks). Go: no
    /// limit.
    WorkLimit {
        /// The function.
        kdf: Kdf,
        /// The work the parameters need.
        needed: u64,
        /// The limit in force.
        limit: u64,
    },
    /// The working memory of a derivation could not be allocated. Go: the
    /// allocation failure kills the process.
    OutOfMemory {
        /// The function.
        kdf: Kdf,
        /// The bytes that could not be allocated.
        bytes: u64,
    },
    /// The Argon2 implementation refused parameters that passed every check
    /// above (not expected to happen); the text is its message.
    Argon2Rejected(String),
    /// `ciphertext too short (len <n>) to encode nonce and MAC tag`: a
    /// per-account blob shorter than 40 bytes
    /// (`wallet/udb/addressmanager.go:192-196`).
    CiphertextTooShort(usize),
    /// `chacha20poly1305: plaintext too large`: a per-account plaintext of
    /// `2^38 - 64` bytes or more. Go: x/crypto panics with this message
    /// above `2^38 - 64` bytes, and seals exactly `2^38 - 64`.
    PlaintextTooLarge,
    /// The random source failed; the text is the source's own message. Go:
    /// dcrd's random source cannot fail after start-up (only kdf's reader
    /// argument can, and its error is returned as it is).
    RandomSource(String),
}

impl fmt::Display for ErrorCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCause::MissingNonce => f.write_str("missing nonce"),
            ErrorCause::BadMarshalledLen(n) => write!(f, "bad marshalled data len {n}"),
            ErrorCause::ScryptNotPowerOfTwo => {
                f.write_str("scrypt: N must be > 1 and a power of 2")
            }
            ErrorCause::ScryptNotPositive => f.write_str("scrypt: parameters must be > 0"),
            ErrorCause::ScryptTooLarge => f.write_str("scrypt: parameters are too large"),
            ErrorCause::InvalidArgon2idParams => {
                f.write_str("invalid marshaled Argon2id parameters")
            }
            ErrorCause::Argon2RoundsTooSmall => f.write_str("argon2: number of rounds too small"),
            ErrorCause::Argon2ParallelismTooLow => {
                f.write_str("argon2: parallelism degree too low")
            }
            ErrorCause::Argon2MemoryBelowMinimum { memory, threads } => write!(
                f,
                "argon2: a memory cost of {memory} KiB is below 8 KiB per thread \
                 ({threads} threads), which is not supported"
            ),
            ErrorCause::MemoryLimit { kdf, needed, limit } => write!(
                f,
                "{kdf}: parameters need {needed} bytes of memory, above the limit of {limit}"
            ),
            ErrorCause::WorkLimit { kdf, needed, limit } => write!(
                f,
                "{kdf}: parameters need {needed} units of work, above the limit of {limit}"
            ),
            ErrorCause::OutOfMemory { kdf, bytes } => {
                write!(f, "{kdf}: could not allocate {bytes} bytes")
            }
            ErrorCause::Argon2Rejected(msg) => write!(f, "argon2: {msg}"),
            ErrorCause::CiphertextTooShort(n) => write!(
                f,
                "ciphertext too short (len {n}) to encode nonce and MAC tag"
            ),
            ErrorCause::PlaintextTooLarge => f.write_str("chacha20poly1305: plaintext too large"),
            ErrorCause::RandomSource(msg) => f.write_str(msg),
        }
    }
}

/// What follows an error's op and kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Detail {
    /// Nothing (Go's `Err == nil`).
    None,
    /// A leaf condition.
    Cause(ErrorCause),
    /// A nested error (Go's `*errors.Error` in the `Err` field).
    Nested(Box<Error>),
}

/// An error from this crate: dcrwallet's `*errors.Error` with the fields the
/// ported code sets.
///
/// `Display` reproduces Go's `(*Error).Error()` byte for byte with the
/// daemon's separator ([`DAEMON_SEPARATOR`]); [`Error::render`] takes
/// another. [`Error::to_purse_error`] (or `From`) converts it, without loss,
/// into the wallet-wide [`purse_errors::Error`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Error {
    op: Option<Op>,
    kind: Kind,
    detail: Detail,
}

impl Error {
    /// `errors.E(op, kind, cause)`.
    pub(crate) fn new(op: Option<Op>, kind: Kind, cause: ErrorCause) -> Self {
        Error {
            op,
            kind,
            detail: Detail::Cause(cause),
        }
    }

    /// `errors.E(op, kind)`: no underlying error.
    pub(crate) fn bare(op: Option<Op>, kind: Kind) -> Self {
        Error {
            op,
            kind,
            detail: Detail::None,
        }
    }

    /// `errors.E(op, inner)` for an `*errors.Error` `inner`, with Go's
    /// promotion of the nested kind and its collapse of a nested level that
    /// adds nothing (`errors/errors.go:187-245`).
    pub(crate) fn wrap(op: Op, inner: Error) -> Self {
        // E takes the nested kind (the outer level sets none), so Go's
        // collapse test `(prev.Op == "" || e.Op == prev.Op) && (prev.Kind
        // == 0 || e.Kind == prev.Kind)` reduces to its op half.
        let kind = inner.kind;
        if inner.op.is_none() || inner.op == Some(op) {
            return Error {
                op: Some(op),
                kind,
                detail: inner.detail,
            };
        }
        Error {
            op: Some(op),
            kind,
            detail: Detail::Nested(Box::new(inner)),
        }
    }

    /// The operation of the outermost level, if it has one.
    pub fn op(&self) -> Option<Op> {
        self.op
    }

    /// The kind of the outermost level (Go's `Kind` field, which `errors.E`
    /// promotes from a nested error when the outer level sets none).
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The leaf condition of this level, if it ends in one.
    pub fn cause(&self) -> Option<&ErrorCause> {
        match &self.detail {
            Detail::Cause(c) => Some(c),
            _ => None,
        }
    }

    /// The nested error of this level, if any.
    pub fn nested(&self) -> Option<&Error> {
        match &self.detail {
            Detail::Nested(inner) => Some(inner),
            _ => None,
        }
    }

    /// The innermost leaf condition, following nested errors.
    pub fn root_cause(&self) -> Option<&ErrorCause> {
        let mut e = self;
        loop {
            match &e.detail {
                Detail::None => return None,
                Detail::Cause(c) => return Some(c),
                Detail::Nested(inner) => e = inner,
            }
        }
    }

    /// Whether any level of the chain has kind `kind` (Go's
    /// `errors.Is(err, kind)`).
    pub fn is_kind(&self, kind: Kind) -> bool {
        let mut e = self;
        loop {
            if e.kind == kind {
                return true;
            }
            match &e.detail {
                Detail::Nested(inner) => e = inner,
                _ => return false,
            }
        }
    }

    /// Renders the error as Go's `(*Error).Error()` does with
    /// `errors.Separator` set to `separator`
    /// ([`purse_errors::DAEMON_SEPARATOR`] or
    /// [`purse_errors::MULTILINE_SEPARATOR`]).
    pub fn render(&self, separator: &str) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail.
        let _ = self.write_rendered(&mut out, separator);
        out
    }

    /// `(*Error).Error()` (`errors/errors.go:247-290`): each level writes
    /// its op and kind unless equal to the last ones written, nested levels
    /// are joined by the separator and a leaf follows `": "`; an error that
    /// writes nothing renders as `Other`'s text.
    fn write_rendered<W: fmt::Write>(&self, out: &mut W, separator: &str) -> fmt::Result {
        let mut counter = Counter { inner: out, n: 0 };
        let mut last_op: Option<Op> = None;
        let mut last_kind = Kind::Other;
        let mut e = self;
        loop {
            let mut pad = false;
            if let Some(op) = e.op
                && last_op != Some(op)
            {
                counter.write_str(op.as_str())?;
                pad = true;
                last_op = Some(op);
            }
            if e.kind != Kind::Other && e.kind != last_kind {
                if pad {
                    counter.write_str(": ")?;
                }
                counter.write_str(e.kind.as_str())?;
                pad = true;
                last_kind = e.kind;
            }
            match &e.detail {
                Detail::None => break,
                Detail::Nested(inner) => {
                    if pad {
                        counter.write_str(separator)?;
                    }
                    e = inner;
                }
                Detail::Cause(c) => {
                    if pad {
                        counter.write_str(": ")?;
                    }
                    write!(counter, "{c}")?;
                    break;
                }
            }
        }
        if counter.n == 0 {
            counter.write_str(Kind::Other.as_str())?;
        }
        Ok(())
    }

    /// The same error in the wallet-wide model: each level becomes a
    /// [`purse_errors::Error`] built field by field (Go's
    /// `&errors.Error{Op, Kind, Err}`), a leaf becomes a text error and a
    /// level without an op gets the empty op. Rendering, `Is` and `As`
    /// answers are unchanged.
    pub fn to_purse_error(&self) -> purse_errors::Error {
        let op = self.op.map_or_else(purse_errors::Op::default, |op| {
            purse_errors::Op::new(op.as_str())
        });
        let err = match &self.detail {
            Detail::None => None,
            Detail::Cause(c) => Some(AnyError::new(c.to_string())),
            Detail::Nested(inner) => Some(AnyError::from(inner.to_purse_error())),
        };
        purse_errors::Error::from_parts(op, self.kind, err)
    }
}

/// Counts what passes through, to know whether anything was written.
struct Counter<'a, W: fmt::Write> {
    inner: &'a mut W,
    n: usize,
}

impl<W: fmt::Write> fmt::Write for Counter<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.n = self.n.saturating_add(s.len());
        self.inner.write_str(s)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_rendered(f, DAEMON_SEPARATOR)
    }
}

/// `source()` is `None`: `Display` already includes a nested error's text,
/// as Go's rendering does. The nested error is reachable through
/// [`Error::nested`].
impl std::error::Error for Error {}

impl From<Error> for purse_errors::Error {
    fn from(e: Error) -> Self {
        e.to_purse_error()
    }
}

impl From<Error> for AnyError {
    fn from(e: Error) -> Self {
        AnyError::from(e.to_purse_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use purse_errors::MULTILINE_SEPARATOR;

    #[test]
    fn wrap_keeps_a_level_with_its_own_op() {
        let inner = Error::bare(Some(Op::CryptoKeyDecrypt), Kind::Crypto);
        let e = Error::wrap(Op::SecretKeyDecrypt, inner);
        assert_eq!(
            e.to_string(),
            "secretkey.Decrypt: encryption/decryption error:: cryptokey.Decrypt"
        );
        assert_eq!(
            e.render(MULTILINE_SEPARATOR),
            "secretkey.Decrypt: encryption/decryption error:\n\tcryptokey.Decrypt"
        );
        assert_eq!(e.kind(), Kind::Crypto);
        assert!(e.nested().is_some());
    }

    #[test]
    fn wrap_collapses_a_level_without_op() {
        let inner = Error::new(None, Kind::Other, ErrorCause::RandomSource("boom".into()));
        let e = Error::wrap(Op::SecretKeyEncrypt, inner);
        assert_eq!(e.to_string(), "secretkey.Encrypt: boom");
        assert!(e.nested().is_none());
    }

    #[test]
    fn empty_error_renders_as_other() {
        let e = Error::bare(None, Kind::Other);
        assert_eq!(e.to_string(), "unclassified error");
        assert_eq!(e.to_purse_error().render(DAEMON_SEPARATOR), e.to_string());
    }

    #[test]
    fn purse_errors_rendering_agrees() {
        let cases = [
            Error::new(
                Some(Op::CryptoKeyDecrypt),
                Kind::Invalid,
                ErrorCause::MissingNonce,
            ),
            Error::wrap(
                Op::SecretKeyDecrypt,
                Error::new(
                    Some(Op::CryptoKeyDecrypt),
                    Kind::Invalid,
                    ErrorCause::MissingNonce,
                ),
            ),
            Error::bare(None, Kind::Passphrase),
            Error::new(None, Kind::Crypto, ErrorCause::CiphertextTooShort(3)),
            Error::new(None, Kind::Other, ErrorCause::InvalidArgon2idParams),
        ];
        for e in &cases {
            for sep in [DAEMON_SEPARATOR, MULTILINE_SEPARATOR] {
                assert_eq!(e.to_purse_error().render(sep), e.render(sep));
            }
            assert!(e.to_purse_error().is_kind(e.kind()) || e.kind() == Kind::Other);
        }
    }
}
