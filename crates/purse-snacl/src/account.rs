// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2024 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The per-account passphrase blobs of dcrwallet's `wallet/udb`: an account
//! extended private key sealed with XChaCha20-Poly1305 under an
//! Argon2id-derived key (`argon2idKey`, `seal` and `unseal`,
//! `wallet/udb/addressmanager.go:156-207`).

use std::fmt;

use chacha20poly1305::{AeadInOut, KeyInit, Tag, XChaCha20Poly1305, XNonce};
use purse_errors::Kind;
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCause};
use crate::random::{OsRandom, RandomSource};
use crate::secret::SecretBytes;

/// The XChaCha20 nonce's length (`xchacha20NonceSize`).
pub const XCHACHA20_NONCE_SIZE: usize = 24;
/// The Poly1305 tag's length (`poly1305TagSize`).
pub const POLY1305_TAG_SIZE: usize = 16;
/// The bytes a blob adds to its plaintext (`xchacha20poly1305Overhead`).
pub const XCHACHA20POLY1305_OVERHEAD: usize = XCHACHA20_NONCE_SIZE + POLY1305_TAG_SIZE;

/// A 32-byte key derived from an account passphrase with Argon2id
/// (dcrwallet's `keyType`), made by
/// [`Argon2idParams::derive_key`](crate::Argon2idParams::derive_key).
///
/// Zeroed on drop; `Debug` is redacted and there is no `Display`. The key
/// lives on the heap and is built there, so moving an `AccountKey` copies
/// only a pointer (see the crate's "Secrets" section).
#[derive(Default)]
pub struct AccountKey(Box<Zeroizing<[u8; 32]>>);

impl AccountKey {
    /// A key with these bytes. The caller's copy is not touched.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        // Built in place: the bytes never pass through a stack temporary.
        let mut key = AccountKey::default();
        key.as_mut_bytes().copy_from_slice(bytes);
        key
    }

    /// The key's bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn as_mut_bytes(&mut self) -> &mut [u8; 32] {
        &mut self.0
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&**self.0).into())
    }

    /// Seals `plaintext` under a fresh random nonce from the operating
    /// system's source (dcrwallet's `seal`).
    ///
    /// # Errors
    ///
    /// As [`AccountKey::seal_with`].
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        self.seal_with(plaintext, &mut OsRandom)
    }

    /// Seals `plaintext` under a 24-byte nonce read from `rng`, with no
    /// associated data: the result is `nonce || ciphertext || tag`,
    /// `plaintext.len() + 40` bytes (`aead.Seal(nonce, nonce, plaintext,
    /// nil)`).
    ///
    /// # Errors
    ///
    /// No op, kind `Other`: a failing source ([`ErrorCause::RandomSource`];
    /// Go's cannot fail) or a plaintext of `2^38 - 64` bytes (256 GiB less
    /// 64 bytes) or more ([`ErrorCause::PlaintextTooLarge`]). Go panics
    /// with that message above `2^38 - 64` bytes and seals a plaintext of
    /// exactly that length, which this crate refuses, as the XChaCha20
    /// implementation does: the one length at which the two differ.
    pub fn seal_with<R: RandomSource + ?Sized>(
        &self,
        plaintext: &[u8],
        rng: &mut R,
    ) -> Result<Vec<u8>, Error> {
        let mut nonce = [0u8; XCHACHA20_NONCE_SIZE];
        rng.fill(&mut nonce).map_err(|e| {
            Error::new(
                None,
                Kind::Other,
                ErrorCause::RandomSource(e.into_message()),
            )
        })?;
        // Refused before anything is allocated or copied; the cipher's own
        // check, below, is then unreachable.
        if plaintext_too_large(plaintext.len()) {
            return Err(Error::new(None, Kind::Other, ErrorCause::PlaintextTooLarge));
        }
        // Sized up front: the plaintext is copied in and encrypted in place,
        // and a reallocation would leave a copy of it behind. Until the
        // encryption has run the buffer holds the plaintext, so every exit
        // before that wipes it.
        let mut sealed = Zeroizing::new(Vec::with_capacity(
            plaintext.len().saturating_add(XCHACHA20POLY1305_OVERHEAD),
        ));
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(plaintext);
        let (_, body) = sealed.split_at_mut(XCHACHA20_NONCE_SIZE);
        let tag = self
            .cipher()
            .encrypt_inout_detached(&XNonce::from(nonce), &[], body.into())
            .map_err(|_| Error::new(None, Kind::Other, ErrorCause::PlaintextTooLarge))?;
        sealed.extend_from_slice(&tag);
        // Now only ciphertext: moved out, leaving an empty vector to drop.
        Ok(core::mem::take(&mut *sealed))
    }

    /// Opens a blob [`AccountKey::seal`] made (dcrwallet's `unseal`). The
    /// tag is checked before anything is decrypted.
    ///
    /// # Errors
    ///
    /// A blob shorter than 40 bytes: no op, kind `Crypto`,
    /// [`ErrorCause::CiphertextTooShort`] (`encryption/decryption error:
    /// ciphertext too short (len <n>) to encode nonce and MAC tag`). A blob
    /// that does not authenticate: no op, kind `Passphrase`, no cause
    /// (`invalid passphrase`); dcrwallet reports every authentication
    /// failure as a wrong passphrase.
    pub fn unseal(&self, sealed: &[u8]) -> Result<SecretBytes, Error> {
        let too_short = || {
            Error::new(
                None,
                Kind::Crypto,
                ErrorCause::CiphertextTooShort(sealed.len()),
            )
        };
        if sealed.len() < XCHACHA20POLY1305_OVERHEAD {
            return Err(too_short());
        }
        let (nonce, rest) = sealed
            .split_first_chunk::<XCHACHA20_NONCE_SIZE>()
            .ok_or_else(too_short)?;
        let (body, tag) = rest
            .split_last_chunk::<POLY1305_TAG_SIZE>()
            .ok_or_else(too_short)?;
        let mut plaintext = SecretBytes::from_slice(body);
        self.cipher()
            .decrypt_inout_detached(
                &XNonce::from(*nonce),
                &[],
                plaintext.as_mut_bytes().into(),
                &Tag::from(*tag),
            )
            .map_err(|_| Error::bare(None, Kind::Passphrase))?;
        Ok(plaintext)
    }
}

impl Clone for AccountKey {
    /// A copy built in place on the heap, as [`AccountKey::from_bytes`]
    /// builds one.
    fn clone(&self) -> Self {
        AccountKey::from_bytes(self.as_bytes())
    }
}

/// Whether XChaCha20-Poly1305 refuses a plaintext of `len` bytes: its block
/// counter is 32 bits, so a message of `u32::MAX` 64-byte blocks or more
/// (`2^38 - 64` bytes) cannot be encrypted. Never true on a 32-bit target.
fn plaintext_too_large(len: usize) -> bool {
    const BLOCK: usize = 64;
    let max_blocks = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
    len / BLOCK >= max_blocks
}

impl fmt::Debug for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plaintext_bound_is_checked_before_sealing() {
        // The real bound cannot be reached in a test (256 GiB); the
        // predicate is checked at it instead.
        // 2^38 - 64.
        if let Ok(bound) = usize::try_from(274_877_906_880_u64) {
            assert!(!plaintext_too_large(bound.saturating_sub(1)));
            assert!(plaintext_too_large(bound));
            assert!(plaintext_too_large(bound.saturating_add(63)));
            assert!(plaintext_too_large(usize::MAX));
        } else {
            assert!(!plaintext_too_large(usize::MAX));
        }
        assert!(!plaintext_too_large(0));
    }

    #[test]
    fn clones_are_equal_and_independent() {
        let k = AccountKey::from_bytes(&[9; 32]);
        let mut c = k.clone();
        assert_eq!(c.as_bytes(), &[9; 32]);
        c.as_mut_bytes()[0] = 1;
        assert_eq!(k.as_bytes(), &[9; 32]);
    }

    #[test]
    fn debug_is_redacted() {
        assert_eq!(
            format!("{:?}", AccountKey::from_bytes(&[5; 32])),
            "AccountKey(<redacted>)"
        );
    }
}
