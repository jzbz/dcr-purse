// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The encoding contract every key and value type follows, the decode
//! error, and the byte reader the decoders share.

use std::fmt;

/// A key or value type of the store, with its exact byte encoding.
///
/// Encodings are canonical: for every value `x`, `T::decode(&x.encode())`
/// returns `x`, and for every byte string `b` that decodes,
/// `T::decode(b)?.encode()` returns `b`. A decoder rejects trailing bytes,
/// short input, out-of-range enumerations and non-canonical padding with a
/// [`DecodeError`], never by panicking, so a damaged or hostile row is an
/// error.
///
/// Key types additionally implement [`Ord`] so that the order of two keys
/// equals the byte order of their encodings, which is the order the engine
/// iterates in (the property tests check this for every key type).
pub trait Codec: Sized {
    /// The encoded form: a fixed-size array for fixed-size types, a vector
    /// otherwise.
    type Bytes: AsRef<[u8]>;

    /// The encoding.
    fn encode(&self) -> Self::Bytes;

    /// Decodes `bytes`, which must be exactly one encoding.
    fn decode(bytes: &[u8]) -> Result<Self, DecodeError>;
}

/// A row or key that is not a valid encoding of its type.
///
/// Rendered as dcrwallet renders its own short-row errors, for example
/// `outpoint len 35` (`wallet/udb/txdb.go:209`), or `<type>: <reason>` for
/// a row of the right length with an invalid field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    what: &'static str,
    detail: DecodeDetail,
}

/// What was wrong with an encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeDetail {
    /// The length is not one the type has.
    Len(usize),
    /// A field holds a value the type does not define.
    Field(&'static str),
}

impl DecodeError {
    /// A length error for the type named `what`.
    pub fn len(what: &'static str, len: usize) -> DecodeError {
        DecodeError {
            what,
            detail: DecodeDetail::Len(len),
        }
    }

    /// An invalid field of the type named `what`.
    pub fn field(what: &'static str, reason: &'static str) -> DecodeError {
        DecodeError {
            what,
            detail: DecodeDetail::Field(reason),
        }
    }

    /// The name of the type that failed to decode.
    pub fn what(&self) -> &'static str {
        self.what
    }

    /// What was wrong.
    pub fn detail(&self) -> DecodeDetail {
        self.detail
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail {
            DecodeDetail::Len(n) => write!(f, "{} len {n}", self.what),
            DecodeDetail::Field(reason) => write!(f, "{}: {reason}", self.what),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Reads fields from an encoding front to back. Every read is bounds
/// checked; a short read records a length error for the whole input.
pub(crate) struct Reader<'a> {
    what: &'static str,
    all: &'a [u8],
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(what: &'static str, bytes: &'a [u8]) -> Self {
        Reader {
            what,
            all: bytes,
            rest: bytes,
        }
    }

    fn short(&self) -> DecodeError {
        DecodeError::len(self.what, self.all.len())
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, tail) = self
            .rest
            .split_first_chunk::<N>()
            .ok_or_else(|| self.short())?;
        self.rest = tail;
        Ok(*head)
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let (head, tail) = self.rest.split_at_checked(n).ok_or_else(|| self.short())?;
        self.rest = tail;
        Ok(head)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16_be(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32_be(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32_le(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn i64_be(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    pub(crate) fn u64_be(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// The bytes not read yet; the reader is then exhausted.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.rest)
    }

    /// Fails unless every byte was read.
    pub(crate) fn finish(self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(self.short())
        }
    }

    pub(crate) fn field(&self, reason: &'static str) -> DecodeError {
        DecodeError::field(self.what, reason)
    }
}

/// Builds a fixed-size encoding front to back.
pub(crate) struct ArrayWriter<const N: usize> {
    buf: [u8; N],
    at: usize,
}

impl<const N: usize> ArrayWriter<N> {
    pub(crate) fn new() -> Self {
        ArrayWriter { buf: [0; N], at: 0 }
    }

    /// Appends `b`. The encoders write exactly `N` bytes by construction;
    /// a write past the end would be a bug in this crate, caught by the
    /// round-trip tests, and is dropped rather than panicking.
    pub(crate) fn put(mut self, b: &[u8]) -> Self {
        let end = self.at.saturating_add(b.len());
        if let Some(dst) = self.buf.get_mut(self.at..end) {
            dst.copy_from_slice(b);
            self.at = end;
        } else {
            debug_assert!(false, "ArrayWriter overflow");
        }
        self
    }

    pub(crate) fn finish(self) -> [u8; N] {
        debug_assert_eq!(self.at, N, "ArrayWriter underfilled");
        self.buf
    }
}
