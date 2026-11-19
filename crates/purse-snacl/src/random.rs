// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The injectable random source: salts, nonces and generated keys.

use std::fmt;

/// A failure of a [`RandomSource`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomSourceError(String);

impl RandomSourceError {
    /// An error with the given message, which becomes the leaf text of the
    /// [`Error`](crate::Error) the failing operation returns.
    pub fn new(message: impl Into<String>) -> Self {
        RandomSourceError(message.into())
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_message(self) -> String {
        self.0
    }
}

impl fmt::Display for RandomSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RandomSourceError {}

impl From<getrandom::Error> for RandomSourceError {
    fn from(e: getrandom::Error) -> Self {
        RandomSourceError(e.to_string())
    }
}

/// A source of random bytes.
///
/// Implemented by [`OsRandom`] and by any
/// `FnMut(&mut [u8]) -> Result<(), RandomSourceError>` closure, so tests can
/// inject a deterministic source. Each operation of this crate that needs
/// randomness makes exactly one `fill` call, for exactly the bytes dcrwallet
/// draws there: 32 for a salt or a generated key, 24 for a nonce, 16 for an
/// Argon2id salt.
pub trait RandomSource {
    /// Fills all of `buf` with random bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot supply the bytes; `buf` may
    /// then be partly written.
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError>;
}

impl<F> RandomSource for F
where
    F: FnMut(&mut [u8]) -> Result<(), RandomSourceError>,
{
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        self(buf)
    }
}

/// The operating system's cryptographically secure random source, through
/// the `getrandom` crate.
///
/// dcrwallet draws from dcrd's `crypto/rand`, a ChaCha20 generator in user
/// space seeded from the operating system; the bytes are equally
/// unpredictable either way, and no output depends on which generator made
/// them.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), RandomSourceError> {
        getrandom::fill(buf).map_err(RandomSourceError::from)
    }
}
