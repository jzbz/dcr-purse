// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Containers for seed material: zeroed on drop and never formatted.

use std::fmt;

use zeroize::Zeroizing;

use crate::wordlist::WORD_LIST;

/// Secret bytes (a seed, or the bytes a mnemonic decodes to).
///
/// The buffer is zeroed when dropped. `Debug` prints only the length, and
/// there is no `Display`, `PartialEq` or serialization: compare and export
/// through [`SecretBytes::as_bytes`] deliberately.
#[derive(Clone, Default)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Copies `bytes` into a new secret buffer.
    pub fn from_slice(bytes: &[u8]) -> Self {
        SecretBytes(Zeroizing::new(bytes.to_vec()))
    }

    /// `len` zero bytes.
    pub(crate) fn zeroed(len: usize) -> Self {
        SecretBytes(Zeroizing::new(vec![0; len]))
    }

    /// An empty buffer with room for `capacity` bytes, so that pushing up to
    /// that many never reallocates (a reallocation would leave an unzeroed
    /// copy behind).
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        SecretBytes(Zeroizing::new(Vec::with_capacity(capacity)))
    }

    /// Appends a byte. Callers reserve the capacity up front.
    pub(crate) fn push(&mut self, b: u8) {
        debug_assert!(self.0.len() < self.0.capacity(), "would reallocate");
        self.0.push(b);
    }

    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, mutably.
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

/// A secret string (a seed mnemonic).
///
/// Zeroed when dropped; `Debug` prints only the length and there is no
/// `Display`.
#[derive(Clone, Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Wraps an already-built string.
    pub(crate) fn new(s: Zeroizing<String>) -> Self {
        SecretString(s)
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The buffer's capacity (tests check it was sized exactly).
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.0.capacity()
    }
}

impl AsRef<str> for SecretString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString(<{} bytes redacted>)", self.0.len())
    }
}

/// A seed mnemonic as a list of words (Go's `[]string` from
/// `EncodeMnemonicSlice`).
///
/// Holds word-list indices in a zero-on-drop buffer and hands out the
/// static words; `Debug` prints only the word count.
#[derive(Clone, Default)]
pub struct MnemonicWords(Zeroizing<Vec<u16>>);

impl MnemonicWords {
    /// An empty list with room for `capacity` words.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        MnemonicWords(Zeroizing::new(Vec::with_capacity(capacity)))
    }

    /// Appends the word at word-list index `index` (below 512). Callers
    /// reserve the capacity up front.
    pub(crate) fn push_index(&mut self, index: u16) {
        debug_assert!(usize::from(index) < WORD_LIST.len());
        debug_assert!(self.0.len() < self.0.capacity(), "would reallocate");
        self.0.push(index);
    }

    /// The number of words.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no words.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The word at `i`, if any.
    pub fn get(&self, i: usize) -> Option<&'static str> {
        self.0.get(i).map(|&idx| word_at(idx))
    }

    /// The words in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.0.iter().map(|&idx| word_at(idx))
    }

    /// The words joined by single spaces (Go's `strings.Join(words, " ")`),
    /// in a buffer sized exactly so that it never reallocates.
    pub fn join(&self) -> SecretString {
        let mut out = Zeroizing::new(String::with_capacity(self.joined_len()));
        for (i, w) in self.iter().enumerate() {
            if i != 0 {
                out.push(' ');
            }
            out.push_str(w);
        }
        SecretString::new(out)
    }

    /// The length of [`MnemonicWords::join`]'s result: the words and one
    /// space between each pair.
    fn joined_len(&self) -> usize {
        self.iter()
            .map(str::len)
            .fold(self.len().saturating_sub(1), usize::saturating_add)
    }
}

/// The word at a word-list index known to be in range.
fn word_at(idx: u16) -> &'static str {
    WORD_LIST.get(usize::from(idx)).copied().unwrap_or_default()
}

impl fmt::Debug for MnemonicWords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MnemonicWords(<{} words redacted>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_contents() {
        let b = SecretBytes::from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(format!("{b:?}"), "SecretBytes(<4 bytes redacted>)");
        let s = SecretString::new(Zeroizing::new("aardvark adroitness".to_owned()));
        assert_eq!(format!("{s:?}"), "SecretString(<19 bytes redacted>)");
        let mut w = MnemonicWords::with_capacity(2);
        w.push_index(0);
        w.push_index(1);
        assert_eq!(format!("{w:?}"), "MnemonicWords(<2 words redacted>)");
        assert_eq!(w.join().as_str(), "aardvark adroitness");
    }
}
