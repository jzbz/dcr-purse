// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Containers for secrets: zeroed on drop and never formatted.

use std::fmt;

use zeroize::Zeroizing;

/// Secret bytes: a decrypted plaintext.
///
/// The buffer is zeroed when dropped. `Debug` prints only the length, and
/// there is no `Display`, `PartialEq` or serialization: compare and export
/// through [`SecretBytes::as_bytes`] deliberately.
#[derive(Clone, Default)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Copies `bytes` into a new secret buffer of exactly their length.
    pub fn from_slice(bytes: &[u8]) -> Self {
        SecretBytes(Zeroizing::new(bytes.to_vec()))
    }

    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, mutably (decryption works in place).
    pub(crate) fn as_mut_bytes(&mut self) -> &mut [u8] {
        &mut self.0
    }

    /// The number of bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes(<{} bytes redacted>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_contents() {
        let b = SecretBytes::from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(format!("{b:?}"), "SecretBytes(<4 bytes redacted>)");
        assert_eq!(b.as_bytes(), &[0xde, 0xad, 0xbe, 0xef]);
    }
}
