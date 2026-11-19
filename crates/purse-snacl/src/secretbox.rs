// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! NaCl `crypto_secretbox` (XSalsa20-Poly1305), as x/crypto's
//! `nacl/secretbox` computes it, with the box laid out `tag || ciphertext`.
//!
//! The construction is assembled here from RustCrypto's `salsa20` (XSalsa20)
//! and `poly1305`, exactly as RustCrypto's own `crypto_secretbox` assembles
//! it: the first 32 bytes of the XSalsa20 keystream are the one-time Poly1305
//! key, the message is encrypted with the keystream from byte 32 on, and the
//! tag is Poly1305 over the ciphertext (x/crypto `secretbox.go:47-56`, `:76-120`). No
//! primitive is implemented here. `crypto_secretbox` itself is not used
//! because no release fits the workspace's dependency generation: 0.1.1 (and
//! the older `xsalsa20poly1305` 0.9.1) would add second versions of `aead`,
//! `cipher`, `cpufeatures`, `crypto-common`, `inout`, `poly1305`, `salsa20`
//! and `universal-hash` next to the ones `chacha20poly1305` 0.11 and scrypt
//! 0.12 bring, and the 0.2.0 pre-release pins `salsa20 = "=0.11.0-rc.1"`,
//! which cannot be resolved next to scrypt 0.12's `salsa20 ^0.11`.

use poly1305::Poly1305;
use poly1305::universal_hash::KeyInit;
use salsa20::XSalsa20;
use salsa20::cipher::{KeyIvInit, StreamCipher};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::secret::SecretBytes;

/// The Poly1305 tag's length, x/crypto's `secretbox.Overhead`.
pub(crate) const TAG_LEN: usize = 16;

/// A keystream positioned after the one-time Poly1305 key, and that key's
/// MAC.
fn setup(key: &[u8; 32], nonce: &[u8; 24]) -> (XSalsa20, Poly1305) {
    let mut cipher = XSalsa20::new(key.into(), nonce.into());
    let mut mac_key = Zeroizing::new([0u8; 32]);
    cipher.apply_keystream(&mut mac_key[..]);
    let mac = Poly1305::new((&*mac_key).into());
    (cipher, mac)
}

/// Appends the box of `plaintext` (`tag || ciphertext`) to `out`.
///
/// The plaintext is copied into `out` and encrypted there; `out` must have
/// room for `TAG_LEN + plaintext.len()` more bytes so that it does not
/// reallocate and leave a copy of the plaintext behind.
pub(crate) fn seal_into(out: &mut Vec<u8>, key: &[u8; 32], nonce: &[u8; 24], plaintext: &[u8]) {
    debug_assert!(
        out.capacity().saturating_sub(out.len()) >= TAG_LEN.saturating_add(plaintext.len())
    );
    let (mut cipher, mac) = setup(key, nonce);
    let start = out.len();
    out.extend_from_slice(&[0u8; TAG_LEN]);
    out.extend_from_slice(plaintext);
    let (_, boxed) = out.split_at_mut(start);
    let (tag, body) = boxed.split_at_mut(TAG_LEN);
    cipher.apply_keystream(body);
    tag.copy_from_slice(&mac.compute_unpadded(body));
}

/// Opens a box (`tag || ciphertext`), or `None` when it is shorter than a
/// tag or does not authenticate. The tag is checked, in constant time,
/// before anything is decrypted (x/crypto `secretbox.go:125-173`).
pub(crate) fn open(key: &[u8; 32], nonce: &[u8; 24], boxed: &[u8]) -> Option<SecretBytes> {
    let (tag, body) = boxed.split_at_checked(TAG_LEN)?;
    let (mut cipher, mac) = setup(key, nonce);
    let expected = mac.compute_unpadded(body);
    if !bool::from(expected.as_slice().ct_eq(tag)) {
        return None;
    }
    let mut plaintext = SecretBytes::from_slice(body);
    cipher.apply_keystream(plaintext.as_mut_bytes());
    Some(plaintext)
}
