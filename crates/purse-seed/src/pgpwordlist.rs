// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `pgpwordlist` package: the PGP word list for
//! human-readable hashes and seeds, and conversion between bytes and words.

use std::sync::LazyLock;

use zeroize::Zeroizing;

use crate::error::{Error, ErrorCause, ErrorKind, Op};
use crate::secret::SecretBytes;
use crate::wordlist::WORD_LIST;

/// The length of the longest word in the list.
const MAX_WORD_LEN: usize = 11;

/// The word-list index of byte `b` at position `index`: `2*b`, plus one at
/// an odd position.
pub(crate) fn word_index(b: u8, index: usize) -> u16 {
    u16::from(b).wrapping_mul(2) | u16::from(index & 1 != 0)
}

/// Returns the PGP word list encoding of `b` when found at position `index`
/// (Go's `ByteToMnemonic`, `pgpwordlist/pgpwordlist.go:26-32`).
///
/// Even positions take the word at list index `2*b`, odd positions the one
/// at `2*b + 1`. Go's `index` is a signed `int`, where a negative odd index
/// also selects the odd word; positions here are unsigned, so that case
/// does not arise.
pub fn byte_to_mnemonic(b: u8, index: usize) -> &'static str {
    WORD_LIST
        .get(usize::from(word_index(b, index)))
        .copied()
        .unwrap_or_default()
}

/// Returns the bytes encoded by `words` (Go's `DecodeMnemonics`,
/// `pgpwordlist/pgpwordlist.go:36-60`).
///
/// Each word is trimmed of white space as Go's `strings.TrimSpace` does
/// (Unicode `White_Space`); words that are then empty are skipped and do not
/// count as a position. Words match case-insensitively under Go's simple
/// Unicode lower-casing, so `İ` (U+0130) matches `i` and the Kelvin sign
/// (U+212A) matches `k`. A word of the wrong parity for its position (the
/// count of words accepted so far) is rejected. There is no checksum and no
/// length limit at this layer; no words, or only empty ones, decode to an
/// empty result.
///
/// # Errors
///
/// Kind [`ErrorKind::Encoding`], op [`Op::DecodeMnemonics`], with cause
/// [`ErrorCause::UnknownWord`] or [`ErrorCause::WrongPosition`]; the
/// message quotes the trimmed word as given.
pub fn decode_mnemonics<S: AsRef<str>>(words: &[S]) -> Result<SecretBytes, Error> {
    decode_words(words.iter().map(AsRef::as_ref), words.len())
}

/// [`decode_mnemonics`] over an iterator of at most `capacity` words.
pub(crate) fn decode_words<'a>(
    words: impl Iterator<Item = &'a str>,
    capacity: usize,
) -> Result<SecretBytes, Error> {
    let mut decoded = SecretBytes::with_capacity(capacity);
    let mut idx: usize = 0;
    for w in words {
        let w = go_trim_space(w);
        if w.is_empty() {
            continue;
        }
        let Some(b) = lookup(w) else {
            return Err(Error::new(
                Op::DecodeMnemonics,
                ErrorKind::Encoding,
                ErrorCause::UnknownWord {
                    word: Zeroizing::new(w.to_owned()),
                },
            ));
        };
        if usize::from(b & 1) != idx & 1 {
            return Err(Error::new(
                Op::DecodeMnemonics,
                ErrorKind::Encoding,
                ErrorCause::WrongPosition {
                    word: Zeroizing::new(w.to_owned()),
                    position: idx,
                },
            ));
        }
        decoded.push(u8::try_from(b.wrapping_shr(1)).unwrap_or_default());
        idx = idx.saturating_add(1);
    }
    Ok(decoded)
}

/// The lower-cased words with their list indices, sorted for binary search
/// (Go's `wordIndexes` map, `pgpwordlist/wordlist.go:25-31`).
static WORD_INDEXES: LazyLock<Vec<(String, u16)>> = LazyLock::new(|| {
    let mut v: Vec<(String, u16)> = (0u16..)
        .zip(WORD_LIST.iter())
        .map(|(i, w)| (w.to_ascii_lowercase(), i))
        .collect();
    v.sort_unstable();
    v
});

/// Looks up `strings.ToLower(w)` in the word index.
///
/// Every word is ASCII lower-case letters once lower-cased and at most
/// [`MAX_WORD_LEN`] long, so a candidate whose lower-cased form has any
/// other character, or is longer, matches nothing. The lower-cased copy
/// lives in a zero-on-drop buffer.
fn lookup(w: &str) -> Option<u16> {
    let mut buf = Zeroizing::new([0u8; MAX_WORD_LEN]);
    let mut n: usize = 0;
    for c in w.chars() {
        let lower = go_simple_lower(c);
        if !lower.is_ascii_lowercase() {
            return None;
        }
        *buf.get_mut(n)? = u8::try_from(lower).ok()?;
        n = n.saturating_add(1);
    }
    let key = buf.get(..n)?;
    WORD_INDEXES
        .binary_search_by(|(word, _)| word.as_bytes().cmp(key))
        .ok()
        .and_then(|i| WORD_INDEXES.get(i))
        .map(|&(_, idx)| idx)
}

/// Go's `unicode.ToLower`: the simple (one-to-one) lower-case mapping.
///
/// Rust's `char::to_lowercase` is the full mapping, which differs only for
/// U+0130 (`İ`, full mapping `i` + U+0307, simple mapping `i`).
pub(crate) fn go_simple_lower(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if c == '\u{130}' {
        return 'i';
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// Go's `unicode.IsSpace`: the Latin-1 white space characters and the
/// Unicode `White_Space` property above Latin-1.
pub(crate) fn is_go_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0B}'
            | '\u{0C}'
            | '\r'
            | ' '
            | '\u{85}'
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Go's `strings.TrimSpace`.
pub(crate) fn go_trim_space(s: &str) -> &str {
    s.trim_matches(is_go_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_space_agrees_with_white_space_property() {
        // Go's IsSpace is the White_Space property, which Rust's
        // char::is_whitespace also implements.
        for c in (0u32..=0x10FFFF).filter_map(char::from_u32) {
            assert_eq!(is_go_space(c), c.is_whitespace(), "{:#x}", u32::from(c));
        }
    }

    #[test]
    fn simple_lowercase_specials() {
        assert_eq!(go_simple_lower('\u{130}'), 'i');
        assert_eq!(go_simple_lower('\u{212A}'), 'k');
        assert_eq!(go_simple_lower('\u{212B}'), '\u{E5}');
        assert_eq!(go_simple_lower('Σ'), 'σ');
        assert!(lookup("istanbul").is_some());
        assert_eq!(lookup("İstanbul"), lookup("istanbul"));
        assert!(lookup("kiwi").is_some());
        assert_eq!(lookup("\u{212A}iwi"), lookup("kiwi"));
        // Full lower-casing would give "i\u{307}stanbul".
        assert_eq!(lookup("i\u{307}stanbul"), None);
        assert_eq!(lookup("aardvarks"), None);
        assert_eq!(lookup("responsiveness"), None);
    }

    #[test]
    fn index_covers_every_word() {
        assert_eq!(WORD_INDEXES.len(), 512);
        for (i, w) in (0u16..).zip(WORD_LIST.iter()) {
            assert_eq!(lookup(w), Some(i));
            assert_eq!(lookup(&w.to_ascii_uppercase()), Some(i));
        }
    }

    /// The transcription check of spec 22 section 4.1: SHA-256 of the words
    /// joined by newlines, as Go's `alternatingWords` constant.
    #[test]
    fn word_list_digest() {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(WORD_LIST.join("\n"));
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "55c9067321ccd946013bfda1ba480ffa09d3b97e119eddc72ba9598a28ef70d3"
        );
    }
}
