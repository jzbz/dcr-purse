// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2014-2024 The btcsuite developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `wallet/internal/snacl`: scrypt-derived secret keys
//! with their stored parameters, and random crypto keys sealing data in NaCl
//! secretboxes.

use std::fmt;

use purse_errors::Kind;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::error::{Error, ErrorCause, Kdf, Op};
use crate::limits::Limits;
use crate::random::{OsRandom, RandomSource};
use crate::secret::SecretBytes;
use crate::secretbox;

/// The secretbox overhead, the Poly1305 tag (Go's `snacl.Overhead`).
pub const OVERHEAD: usize = secretbox::TAG_LEN;
/// The length of a key and of a salt (Go's `snacl.KeySize`).
pub const KEY_SIZE: usize = 32;
/// The length of a secretbox nonce (Go's `snacl.NonceSize`).
pub const NONCE_SIZE: usize = 24;
/// Go's `snacl.DefaultN`, 2^14. Only snacl's own tests use it: dcrwallet's
/// address manager passes `N = 2^18` (2 on simnet) and stored parameters
/// carry their own.
pub const DEFAULT_N: i64 = 16384;
/// Go's `snacl.DefaultR`.
pub const DEFAULT_R: i64 = 8;
/// Go's `snacl.DefaultP`.
pub const DEFAULT_P: i64 = 1;
/// The length of marshalled [`Parameters`]: salt, digest and three 8-byte
/// integers.
pub const MARSHALLED_LEN: usize = 88;

/// The length of the SHA-256 digest of a derived key.
const DIGEST_SIZE: usize = 32;

/// A random secret key that encrypts and decrypts data (Go's
/// `snacl.CryptoKey`): the crypto keys dcrwallet seals its extended keys,
/// imported keys and script hashes with, and the key half of a
/// [`SecretKey`].
///
/// Zeroed on drop; `Debug` is redacted and there is no `Display`. The key
/// lives on the heap and is built there, so moving a `CryptoKey` copies only
/// a pointer (see the crate's "Secrets" section).
#[derive(Default)]
pub struct CryptoKey(Box<Zeroizing<[u8; KEY_SIZE]>>);

impl CryptoKey {
    /// A key with these bytes. The caller's copy is not touched.
    pub fn from_bytes(bytes: &[u8; KEY_SIZE]) -> Self {
        // Built in place: the bytes never pass through a stack temporary.
        let mut key = CryptoKey::default();
        key.0.copy_from_slice(bytes);
        key
    }

    /// A key with the bytes of `bytes`, or `None` when it is not 32 bytes
    /// long (dcrwallet's `CopyBytes` copies whatever prefix it is given; a
    /// decrypted crypto key is always 32 bytes).
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let bytes: &[u8; KEY_SIZE] = bytes.try_into().ok()?;
        Some(CryptoKey::from_bytes(bytes))
    }

    /// The key's bytes (dcrwallet's `Bytes`).
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    /// A new key from the operating system's random source (Go's
    /// `snacl.GenerateCryptoKey`, `snacl.go:73-79`).
    ///
    /// # Errors
    ///
    /// As [`CryptoKey::generate_with`].
    pub fn generate() -> Result<Self, Error> {
        CryptoKey::generate_with(&mut OsRandom)
    }

    /// A new key of 32 bytes read from `rng`.
    ///
    /// # Errors
    ///
    /// Only a failing source: op [`Op::GenerateCryptoKey`], kind
    /// `Other`, [`ErrorCause::RandomSource`]. Go's source cannot fail.
    pub fn generate_with<R: RandomSource + ?Sized>(rng: &mut R) -> Result<Self, Error> {
        let mut key = CryptoKey::default();
        rng.fill(&mut key.0[..]).map_err(|e| {
            Error::new(
                Some(Op::GenerateCryptoKey),
                Kind::Other,
                ErrorCause::RandomSource(e.into_message()),
            )
        })?;
        Ok(key)
    }

    /// Seals `plaintext` under a fresh random nonce from the operating
    /// system's source (Go's `(*CryptoKey).Encrypt`, `snacl.go:35-43`).
    ///
    /// # Errors
    ///
    /// As [`CryptoKey::encrypt_with`].
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        self.encrypt_with(plaintext, &mut OsRandom)
    }

    /// Seals `plaintext` under a 24-byte nonce read from `rng`: the result
    /// is `nonce || tag || ciphertext`, `plaintext.len() + 40` bytes.
    ///
    /// # Errors
    ///
    /// Only a failing source: op [`Op::CryptoKeyEncrypt`], kind `Other`,
    /// [`ErrorCause::RandomSource`]. Go's encryption cannot fail.
    pub fn encrypt_with<R: RandomSource + ?Sized>(
        &self,
        plaintext: &[u8],
        rng: &mut R,
    ) -> Result<Vec<u8>, Error> {
        let mut nonce = [0u8; NONCE_SIZE];
        rng.fill(&mut nonce).map_err(|e| {
            Error::new(
                Some(Op::CryptoKeyEncrypt),
                Kind::Other,
                ErrorCause::RandomSource(e.into_message()),
            )
        })?;
        let mut sealed = Vec::with_capacity(plaintext.len().saturating_add(NONCE_SIZE + OVERHEAD));
        sealed.extend_from_slice(&nonce);
        secretbox::seal_into(&mut sealed, &self.0, &nonce, plaintext);
        Ok(sealed)
    }

    /// Opens what [`CryptoKey::encrypt`] sealed (Go's
    /// `(*CryptoKey).Decrypt`, `snacl.go:47-63`).
    ///
    /// # Errors
    ///
    /// Input shorter than the 24-byte nonce: op [`Op::CryptoKeyDecrypt`],
    /// kind `Invalid`, [`ErrorCause::MissingNonce`]
    /// (`cryptokey.Decrypt: invalid operation: missing nonce`). A box that
    /// does not authenticate, including one shorter than its tag: op
    /// [`Op::CryptoKeyDecrypt`], kind `Crypto`, no cause
    /// (`cryptokey.Decrypt: encryption/decryption error`).
    pub fn decrypt(&self, sealed: &[u8]) -> Result<SecretBytes, Error> {
        let Some((nonce, boxed)) = sealed.split_first_chunk::<NONCE_SIZE>() else {
            return Err(Error::new(
                Some(Op::CryptoKeyDecrypt),
                Kind::Invalid,
                ErrorCause::MissingNonce,
            ));
        };
        secretbox::open(&self.0, nonce, boxed)
            .ok_or_else(|| Error::bare(Some(Op::CryptoKeyDecrypt), Kind::Crypto))
    }

    /// Overwrites the key with zeros (Go's `(*CryptoKey).Zero`).
    pub fn zero(&mut self) {
        **self.0 = [0; KEY_SIZE];
    }
}

impl Clone for CryptoKey {
    /// A copy built in place on the heap, as [`CryptoKey::from_bytes`]
    /// builds one.
    fn clone(&self) -> Self {
        CryptoKey::from_bytes(self.as_bytes())
    }
}

impl fmt::Debug for CryptoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CryptoKey(<redacted>)")
    }
}

/// The stored, non-secret parameters of a [`SecretKey`] (Go's
/// `snacl.Parameters`): the scrypt salt and cost, and the SHA-256 digest of
/// the derived key, against which a passphrase is checked.
///
/// `n`, `r` and `p` are Go `int`s, 64 bits on every platform dcr-purse
/// supports: [`Parameters::unmarshal`] reads them as Go does, a `uint64`
/// converted to `int`, so values of 2^63 and above become negative and are
/// refused only when a derivation runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Parameters {
    /// The scrypt salt.
    pub salt: [u8; KEY_SIZE],
    /// SHA-256 of the derived key.
    pub digest: [u8; DIGEST_SIZE],
    /// scrypt's CPU/memory cost `N`.
    pub n: i64,
    /// scrypt's block size `r`.
    pub r: i64,
    /// scrypt's parallelism `p`.
    pub p: i64,
}

impl Parameters {
    /// The stored form, `salt || digest || N || R || P` with each integer
    /// as a little-endian `uint64` (Go's `(*SecretKey).Marshal`,
    /// `snacl.go:129-151`).
    pub fn marshal(&self) -> [u8; MARSHALLED_LEN] {
        let mut out = [0u8; MARSHALLED_LEN];
        let (salt, rest) = out.split_at_mut(KEY_SIZE);
        salt.copy_from_slice(&self.salt);
        let (digest, rest) = rest.split_at_mut(DIGEST_SIZE);
        digest.copy_from_slice(&self.digest);
        for (chunk, v) in rest
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip([self.n, self.r, self.p])
        {
            chunk.copy_from_slice(&v.cast_unsigned().to_le_bytes());
        }
        out
    }

    /// Reads the stored form (Go's `(*SecretKey).Unmarshal`,
    /// `snacl.go:154-181`). No value is range-checked; Go checks them only
    /// when scrypt runs.
    ///
    /// # Errors
    ///
    /// Any length but 88: op [`Op::SecretKeyUnmarshal`], kind `Encoding`,
    /// [`ErrorCause::BadMarshalledLen`]
    /// (`secretkey.Unmarshal: invalid encoding: bad marshalled data len <n>`).
    pub fn unmarshal(data: &[u8]) -> Result<Self, Error> {
        let bad_len = || {
            Error::new(
                Some(Op::SecretKeyUnmarshal),
                Kind::Encoding,
                ErrorCause::BadMarshalledLen(data.len()),
            )
        };
        let data: &[u8; MARSHALLED_LEN] = data.try_into().map_err(|_| bad_len())?;
        let (salt, rest) = data.split_first_chunk::<KEY_SIZE>().ok_or_else(bad_len)?;
        let (digest, rest) = rest
            .split_first_chunk::<DIGEST_SIZE>()
            .ok_or_else(bad_len)?;
        let (n, rest) = rest.split_first_chunk::<8>().ok_or_else(bad_len)?;
        let (r, rest) = rest.split_first_chunk::<8>().ok_or_else(bad_len)?;
        let (p, _) = rest.split_first_chunk::<8>().ok_or_else(bad_len)?;
        let int = |b: &[u8; 8]| u64::from_le_bytes(*b).cast_signed();
        Ok(Parameters {
            salt: *salt,
            digest: *digest,
            n: int(n),
            r: int(r),
            p: int(p),
        })
    }
}

/// x/crypto's `scrypt.Key(passphrase, salt, n, r, p, 32)` into `out`, with
/// Go's parameter checks in Go's order (`scrypt.go:195-215`), then this
/// crate's resource bounds.
pub(crate) fn scrypt_key(
    passphrase: &[u8],
    salt: &[u8; KEY_SIZE],
    n: i64,
    r: i64,
    p: i64,
    limits: &Limits,
    out: &mut [u8; KEY_SIZE],
) -> Result<(), ErrorCause> {
    // `N <= 1 || N&(N-1) != 0`.
    if n <= 1 || n & n.wrapping_sub(1) != 0 {
        return Err(ErrorCause::ScryptNotPowerOfTwo);
    }
    // `r <= 0 || p <= 0`.
    if r <= 0 || p <= 0 {
        return Err(ErrorCause::ScryptNotPositive);
    }
    // `uint64(r)*uint64(p) >= 1<<30 || r > maxInt/128/p || r > maxInt/256 ||
    // N > maxInt/128/r`, with Go's wrapping uint64 product and truncating
    // divisions; r and p are positive here, so no division fails.
    const MAX_INT_128: i64 = i64::MAX / 128;
    let too_large = r.cast_unsigned().wrapping_mul(p.cast_unsigned()) >= 1 << 30
        || MAX_INT_128.checked_div(p).is_none_or(|q| r > q)
        || r > i64::MAX / 256
        || MAX_INT_128.checked_div(r).is_none_or(|q| n > q);
    if too_large {
        return Err(ErrorCause::ScryptTooLarge);
    }
    // Go would now allocate 128*r*N + 128*r*p + 256*r bytes whatever they
    // come to, and run N*r*p block mixes. Here: 2 <= N <= 2^56 / r, and
    // 1 <= r, p with r*p < 2^30, so the products below fit u128.
    let (n_wide, r_wide, p_wide) = (n.unsigned_abs(), r.unsigned_abs(), p.unsigned_abs());
    let memory = u128::from(r_wide).saturating_mul(128).saturating_mul(
        u128::from(n_wide)
            .saturating_add(u128::from(p_wide))
            .saturating_add(2),
    );
    let memory = u64::try_from(memory).unwrap_or(u64::MAX);
    if memory > limits.max_memory_bytes {
        return Err(ErrorCause::MemoryLimit {
            kdf: Kdf::Scrypt,
            needed: memory,
            limit: limits.max_memory_bytes,
        });
    }
    let work = u128::from(n_wide)
        .saturating_mul(u128::from(r_wide))
        .saturating_mul(u128::from(p_wide));
    let work = u64::try_from(work).unwrap_or(u64::MAX);
    if work > limits.max_scrypt_work {
        return Err(ErrorCause::WorkLimit {
            kdf: Kdf::Scrypt,
            needed: work,
            limit: limits.max_scrypt_work,
        });
    }
    // N is a power of two, so its log is its trailing zero count (< 63);
    // r and p are below 2^30. The scrypt crate's own checks accept every
    // value Go accepts on a 64-bit target; on a narrower one its overflow
    // checks refuse what does not fit the address space.
    let unsupported = || ErrorCause::MemoryLimit {
        kdf: Kdf::Scrypt,
        needed: memory,
        limit: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
    };
    let log_n = u8::try_from(n.trailing_zeros()).map_err(|_| unsupported())?;
    let r = u32::try_from(r).map_err(|_| unsupported())?;
    let p = u32::try_from(p).map_err(|_| unsupported())?;
    let params = scrypt::Params::new(log_n, r, p).map_err(|_| unsupported())?;
    scrypt::scrypt(passphrase, salt, &params, out).map_err(|_| unsupported())
}

/// A key derived from a passphrase, with the parameters needed to derive it
/// again (Go's `snacl.SecretKey`): dcrwallet's public and private master
/// keys, which seal its two crypto keys.
///
/// The key is zeroed on drop; `Debug` shows the parameters only.
#[derive(Clone)]
pub struct SecretKey {
    key: CryptoKey,
    parameters: Parameters,
}

impl SecretKey {
    /// Derives a new secret key from `passphrase` with scrypt cost `n`, `r`,
    /// `p` and a random salt from the operating system's source, under
    /// [`Limits::DEFAULT`] (Go's `snacl.NewSecretKey`, `snacl.go:228-249`).
    ///
    /// # Errors
    ///
    /// As [`SecretKey::new_with`].
    pub fn new(passphrase: &[u8], n: i64, r: i64, p: i64) -> Result<Self, Error> {
        SecretKey::new_with(passphrase, n, r, p, &mut OsRandom, &Limits::DEFAULT)
    }

    /// Derives a new secret key from `passphrase` with scrypt cost `n`, `r`,
    /// `p` and a 32-byte salt read from `rng` (read first, as Go does, even
    /// when the cost is then refused), recording the derived key's SHA-256
    /// digest.
    ///
    /// # Errors
    ///
    /// All with op [`Op::NewSecretKey`] and kind `Other`: parameters x/crypto
    /// scrypt refuses ([`ErrorCause::ScryptNotPowerOfTwo`],
    /// [`ErrorCause::ScryptNotPositive`], [`ErrorCause::ScryptTooLarge`],
    /// with Go's texts); parameters over `limits`
    /// ([`ErrorCause::MemoryLimit`], [`ErrorCause::WorkLimit`]); a failing
    /// source ([`ErrorCause::RandomSource`]).
    pub fn new_with<R: RandomSource + ?Sized>(
        passphrase: &[u8],
        n: i64,
        r: i64,
        p: i64,
        rng: &mut R,
        limits: &Limits,
    ) -> Result<Self, Error> {
        let fail = |cause| Error::new(Some(Op::NewSecretKey), Kind::Other, cause);
        let mut salt = [0u8; KEY_SIZE];
        rng.fill(&mut salt)
            .map_err(|e| fail(ErrorCause::RandomSource(e.into_message())))?;
        let mut key = CryptoKey::default();
        scrypt_key(passphrase, &salt, n, r, p, limits, &mut key.0).map_err(fail)?;
        let digest: [u8; DIGEST_SIZE] = Sha256::digest(&key.0[..]).into();
        Ok(SecretKey {
            key,
            parameters: Parameters {
                salt,
                digest,
                n,
                r,
                p,
            },
        })
    }

    /// A secret key with these parameters and an all-zero key, to be
    /// derived with [`SecretKey::derive_key`].
    pub fn from_parameters(parameters: Parameters) -> Self {
        SecretKey {
            key: CryptoKey::default(),
            parameters,
        }
    }

    /// A secret key holding `key` with these parameters, as Go's struct
    /// literal `snacl.SecretKey{Key: key, Parameters: parameters}`. Nothing
    /// checks that the key matches the parameters' digest.
    pub fn from_parts(key: CryptoKey, parameters: Parameters) -> Self {
        SecretKey { key, parameters }
    }

    /// Reads stored parameters into a secret key with an all-zero key, as
    /// dcrwallet does when it loads a wallet (`var sk snacl.SecretKey;
    /// sk.Unmarshal(...)`).
    ///
    /// # Errors
    ///
    /// As [`Parameters::unmarshal`].
    pub fn unmarshal(data: &[u8]) -> Result<Self, Error> {
        Parameters::unmarshal(data).map(SecretKey::from_parameters)
    }

    /// The stored form of the parameters (Go's `(*SecretKey).Marshal`).
    pub fn marshal(&self) -> [u8; MARSHALLED_LEN] {
        self.parameters.marshal()
    }

    /// The parameters.
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }

    /// The key: derived, or all zeros before a derivation and after
    /// [`SecretKey::zero`] or a failed derivation.
    pub fn key(&self) -> &CryptoKey {
        &self.key
    }

    /// Derives the key from `passphrase` and checks it against the stored
    /// digest, under [`Limits::DEFAULT`] (Go's `(*SecretKey).DeriveKey`,
    /// `snacl.go:192-205`).
    ///
    /// # Errors
    ///
    /// As [`SecretKey::derive_key_with`].
    pub fn derive_key(&mut self, passphrase: &[u8]) -> Result<(), Error> {
        self.derive_key_with(passphrase, &Limits::DEFAULT)
    }

    /// Derives the key from `passphrase` with the stored scrypt parameters
    /// and checks its SHA-256 digest against the stored one in constant
    /// time.
    ///
    /// On failure the key is left all zeros. Go leaves it holding whatever
    /// it held before when scrypt refuses the parameters, and the key
    /// derived from the wrong passphrase on a digest mismatch
    /// (`snacl.go:110-113`, `:199-202`); no dcrwallet caller uses the key
    /// after either.
    ///
    /// # Errors
    ///
    /// A digest mismatch (the wrong passphrase): op
    /// [`Op::SecretKeyDeriveKey`], kind `Passphrase`, no cause
    /// (`secretkey.DeriveKey: invalid passphrase`). Refused parameters and
    /// parameters over `limits`: op [`Op::SecretKeyDeriveKey`], kind
    /// `Other`, with the cause as for [`SecretKey::new_with`].
    pub fn derive_key_with(&mut self, passphrase: &[u8], limits: &Limits) -> Result<(), Error> {
        let p = self.parameters;
        let derived = scrypt_key(passphrase, &p.salt, p.n, p.r, p.p, limits, &mut self.key.0);
        if let Err(cause) = derived {
            self.key.zero();
            return Err(Error::new(Some(Op::SecretKeyDeriveKey), Kind::Other, cause));
        }
        let digest: Zeroizing<[u8; DIGEST_SIZE]> =
            Zeroizing::new(Sha256::digest(&self.key.0[..]).into());
        if !bool::from(digest.ct_eq(&p.digest)) {
            self.key.zero();
            return Err(Error::bare(Some(Op::SecretKeyDeriveKey), Kind::Passphrase));
        }
        Ok(())
    }

    /// Seals `plaintext` with the key under a random nonce from the
    /// operating system's source (Go's `(*SecretKey).Encrypt`).
    ///
    /// # Errors
    ///
    /// As [`SecretKey::encrypt_with`].
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        self.encrypt_with(plaintext, &mut OsRandom)
    }

    /// Seals `plaintext` with the key under a nonce read from `rng`, as
    /// [`CryptoKey::encrypt_with`].
    ///
    /// # Errors
    ///
    /// Only a failing source: op [`Op::SecretKeyEncrypt`], kind `Other`,
    /// [`ErrorCause::RandomSource`].
    pub fn encrypt_with<R: RandomSource + ?Sized>(
        &self,
        plaintext: &[u8],
        rng: &mut R,
    ) -> Result<Vec<u8>, Error> {
        self.key
            .encrypt_with(plaintext, rng)
            .map_err(|e| Error::wrap(Op::SecretKeyEncrypt, e))
    }

    /// Opens a box sealed with the key (Go's `(*SecretKey).Decrypt`).
    ///
    /// # Errors
    ///
    /// [`CryptoKey::decrypt`]'s error nested under op
    /// [`Op::SecretKeyDecrypt`] with the same kind:
    /// `secretkey.Decrypt: invalid operation:: cryptokey.Decrypt: missing
    /// nonce` and `secretkey.Decrypt: encryption/decryption error::
    /// cryptokey.Decrypt` with the daemon's separator.
    pub fn decrypt(&self, sealed: &[u8]) -> Result<SecretBytes, Error> {
        self.key
            .decrypt(sealed)
            .map_err(|e| Error::wrap(Op::SecretKeyDecrypt, e))
    }

    /// Zeroes the key, keeping the parameters (Go's `(*SecretKey).Zero`).
    pub fn zero(&mut self) {
        self.key.zero();
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretKey")
            .field("key", &self.key)
            .field("parameters", &self.parameters)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted() {
        let mut sk = SecretKey::from_parameters(Parameters {
            salt: [7; 32],
            digest: [9; 32],
            n: 2,
            r: 1,
            p: 1,
        });
        sk.key = CryptoKey::from_bytes(&[0xab; 32]);
        let s = format!("{sk:?}");
        assert!(s.contains("CryptoKey(<redacted>)"), "{s}");
        assert!(!s.contains("171"), "{s}");
    }

    #[test]
    fn from_slice_wants_exactly_32_bytes() {
        assert!(CryptoKey::from_slice(&[1; 31]).is_none());
        assert!(CryptoKey::from_slice(&[1; 33]).is_none());
        assert_eq!(
            CryptoKey::from_slice(&[1; 32]).map(|k| *k.as_bytes()),
            Some([1; 32])
        );
    }

    #[test]
    fn marshal_round_trips_negative_values() {
        let p = Parameters {
            salt: [1; 32],
            digest: [2; 32],
            n: -1,
            r: i64::MIN,
            p: i64::MAX,
        };
        assert_eq!(Parameters::unmarshal(&p.marshal()), Ok(p));
    }
}
