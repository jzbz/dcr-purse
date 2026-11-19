// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2017 Company 0 LLC
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The KEM: key generation, encapsulation and decapsulation
//! (`sntrup4591761.go`), and the key and ciphertext types.

use alloc::boxed::Box;
use core::fmt;

use sha2::{Digest, Sha512};
use subtle::{Choice, ConstantTimeEq};
use zeroize::{Zeroize, Zeroizing};

use crate::arith::{P, W, freeze3, freezeq};
use crate::{RandomSource, r3, rq, zx};

/// The size of an encoded public key (`PublicKeySize`).
pub const PUBLIC_KEY_SIZE: usize = 1218;
/// The size of an encoded private key (`PrivateKeySize`).
pub const PRIVATE_KEY_SIZE: usize = 1600;
/// The size of a ciphertext (`CiphertextSize`).
pub const CIPHERTEXT_SIZE: usize = 1047;
/// The size of a shared key (`SharedKeySize`).
pub const SHARED_KEY_SIZE: usize = 32;

/// Where the parts of a private key start: `zx.Encode(f)`,
/// `zx.Encode(1/g in R3)`, then the public key.
const SK_GINV: usize = zx::SMALL_BYTES;
const SK_PK: usize = 2 * zx::SMALL_BYTES;
/// Where a ciphertext's rounded body starts, after the 32-byte confirmation.
const CT_BODY: usize = 32;

// The layouts add up.
const _: () = assert!(SK_PK + PUBLIC_KEY_SIZE == PRIVATE_KEY_SIZE);
const _: () = assert!(CT_BODY + rq::ROUNDED_BYTES == CIPHERTEXT_SIZE);
const _: () = assert!(rq::ENCODED_BYTES == PUBLIC_KEY_SIZE);

/// A public key: `rq.Encode(h)`, h = g/(3f) in Rq.
///
/// Any 1218 bytes are accepted, as Go's `*[1218]byte` is: encapsulating to
/// a malformed key gives what Go gives.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; PUBLIC_KEY_SIZE]);

/// A ciphertext: the confirmation `SHA-512(zx.Encode(r))[0..32)` followed
/// by `rq.EncodeRounded(Round3(h·r))`.
///
/// Any 1047 bytes are accepted, as Go's `*[1047]byte` is.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Ciphertext([u8; CIPHERTEXT_SIZE]);

/// A private key: `zx.Encode(f) ‖ zx.Encode(1/g in R3) ‖ public key`.
///
/// Held on the heap and wiped on drop; `Debug` prints no key material, and
/// there is no `PartialEq` (compare with [`ConstantTimeEq`]).
#[derive(Clone)]
pub struct PrivateKey(Box<[u8; PRIVATE_KEY_SIZE]>);

/// A shared key: `SHA-512(zx.Encode(r))[32..64)`.
///
/// Held on the heap and wiped on drop; `Debug` prints no key material, and
/// there is no `PartialEq` (compare with [`ConstantTimeEq`]).
#[derive(Clone)]
pub struct SharedKey(Box<[u8; SHARED_KEY_SIZE]>);

macro_rules! public_bytes {
    ($t:ident, $n:ident, $what:literal) => {
        impl $t {
            #[doc = concat!("The ", $what, " with these bytes.")]
            pub const fn from_bytes(bytes: [u8; $n]) -> Self {
                $t(bytes)
            }

            /// The bytes.
            pub const fn as_bytes(&self) -> &[u8; $n] {
                &self.0
            }

            /// The bytes, by value.
            pub const fn to_bytes(&self) -> [u8; $n] {
                self.0
            }
        }

        impl From<[u8; $n]> for $t {
            fn from(bytes: [u8; $n]) -> Self {
                $t(bytes)
            }
        }

        impl TryFrom<&[u8]> for $t {
            type Error = core::array::TryFromSliceError;

            #[doc = concat!("The ", $what, " in `bytes`, which must be exactly [`", stringify!($n), "`] long.")]
            fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
                <[u8; $n]>::try_from(bytes).map($t)
            }
        }

        impl AsRef<[u8]> for $t {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($t), "("))?;
                for b in &self.0[..8] {
                    write!(f, "{b:02x}")?;
                }
                write!(f, "…, {} bytes)", $n)
            }
        }
    };
}

public_bytes!(PublicKey, PUBLIC_KEY_SIZE, "public key");
public_bytes!(Ciphertext, CIPHERTEXT_SIZE, "ciphertext");

macro_rules! secret_bytes {
    ($t:ident, $n:ident, $what:literal) => {
        impl $t {
            #[doc = concat!("The ", $what, " with a copy of these bytes; wiping `bytes` is the caller's.")]
            pub fn from_bytes(bytes: &[u8; $n]) -> Self {
                let mut k = Self::zeroed();
                k.raw_mut().copy_from_slice(bytes);
                k
            }

            /// The bytes.
            pub fn as_bytes(&self) -> &[u8; $n] {
                &self.0
            }

            fn raw_mut(&mut self) -> &mut [u8; $n] {
                &mut self.0
            }

            fn zeroed() -> Self {
                $t(Box::new([0u8; $n]))
            }
        }

        impl TryFrom<&[u8]> for $t {
            type Error = core::array::TryFromSliceError;

            #[doc = concat!("The ", $what, " in `bytes`, which must be exactly [`", stringify!($n), "`] long.")]
            fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
                let b: &[u8; $n] = bytes.try_into()?;
                Ok(Self::from_bytes(b))
            }
        }

        impl Drop for $t {
            fn drop(&mut self) {
                self.0.zeroize();
            }
        }

        impl zeroize::ZeroizeOnDrop for $t {}

        impl ConstantTimeEq for $t {
            fn ct_eq(&self, other: &Self) -> Choice {
                self.as_bytes()[..].ct_eq(&other.as_bytes()[..])
            }
        }

        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($t), "(<{} bytes redacted>)"), $n)
            }
        }
    };
}

secret_bytes!(PrivateKey, PRIVATE_KEY_SIZE, "private key");
secret_bytes!(SharedKey, SHARED_KEY_SIZE, "shared key");

impl PrivateKey {
    /// The public key stored in the last 1218 bytes. For a key that
    /// [`generate_key`] made, the public key it returned.
    pub fn public_key(&self) -> PublicKey {
        let mut pk = [0u8; PUBLIC_KEY_SIZE];
        pk.copy_from_slice(&self.as_bytes()[SK_PK..]);
        PublicKey(pk)
    }
}

/// The fixed-size window `bytes[at..at + N]`.
fn window<const N: usize>(bytes: &[u8], at: usize) -> &[u8; N] {
    bytes
        .get(at..at.wrapping_add(N))
        .and_then(|s| s.try_into().ok())
        .expect("window within a fixed-size key or ciphertext")
}

/// The mutable fixed-size window `bytes[at..at + N]`.
fn window_mut<const N: usize>(bytes: &mut [u8], at: usize) -> &mut [u8; N] {
    bytes
        .get_mut(at..at.wrapping_add(N))
        .and_then(|s| s.try_into().ok())
        .expect("window within a fixed-size key or ciphertext")
}

/// SHA-512 of the encoding of the small element `r`, into `out`.
fn hash_small(r: &[i8; P], out: &mut [u8; 64]) {
    let mut enc = Zeroizing::new([0u8; zx::SMALL_BYTES]);
    zx::encode(r, &mut enc);
    // The hasher state wipes itself (sha2's `zeroize`); the output array is
    // wiped here.
    let mut digest = sha2::digest::Output::<Sha512>::default();
    let mut hasher = Sha512::new();
    hasher.update(&enc[..]);
    hasher.finalize_into(&mut digest);
    out.copy_from_slice(&digest);
    digest.as_mut_slice().zeroize();
}

/// Generates a key pair with randomness from `rng` (`GenerateKey`,
/// `sntrup4591761.go:55-79`).
///
/// Draws, in order: 3044 bytes for a candidate g, repeated until g is
/// invertible in R3 (a fresh 3044 bytes per attempt); then 3044 bytes for
/// f. Each draw is one [`RandomSource::fill`] call.
///
/// # Errors
///
/// Returns the source's error, unchanged, if a draw fails.
pub fn generate_key<R: RandomSource + ?Sized>(
    rng: &mut R,
) -> Result<(PublicKey, PrivateKey), R::Error> {
    let mut g = Zeroizing::new([0i8; P]);
    let mut gr = Zeroizing::new([0i8; P]);
    // Whether a candidate was invertible is the one fact that leaves the
    // constant-time code: rejected candidates are discarded, as in Go.
    loop {
        zx::random_small(&mut g, rng)?;
        if r3::reciprocal(&mut gr, &g) == 0 {
            break;
        }
    }
    let mut f = Zeroizing::new([0i8; P]);
    zx::random_tsmall(&mut f, rng)?;
    Ok(derive_key(&f, &g, &gr))
}

/// The deterministic half of [`generate_key`] (`deriveKey`,
/// `sntrup4591761.go:37-52`): the key pair of f and g, given 1/g in R3.
pub(crate) fn derive_key(f: &[i8; P], g: &[i8; P], gr: &[i8; P]) -> (PublicKey, PrivateKey) {
    let mut f3r = Zeroizing::new([0i16; P]);
    // Every nonzero f is invertible in Rq; Go does not check the result.
    let _ = rq::reciprocal3(&mut f3r, f);
    let mut h = [0i16; P];
    rq::mult(&mut h, &f3r, g);
    let mut pk = PublicKey([0u8; PUBLIC_KEY_SIZE]);
    rq::encode(&h, &mut pk.0);

    let mut sk = PrivateKey::zeroed();
    zx::encode(f, window_mut(&mut sk.raw_mut()[..], 0));
    zx::encode(gr, window_mut(&mut sk.raw_mut()[..], SK_GINV));
    sk.raw_mut()[SK_PK..].copy_from_slice(&pk.0);
    (pk, sk)
}

/// Encapsulates a fresh shared key to `pk` with randomness from `rng`
/// (`Encapsulate`, `sntrup4591761.go:105-117`).
///
/// Draws 3044 bytes, in one [`RandomSource::fill`] call, for the t-small r.
///
/// # Errors
///
/// Returns the source's error, unchanged, if the draw fails.
pub fn encapsulate<R: RandomSource + ?Sized>(
    rng: &mut R,
    pk: &PublicKey,
) -> Result<(Ciphertext, SharedKey), R::Error> {
    let mut r = Zeroizing::new([0i8; P]);
    zx::random_tsmall(&mut r, rng)?;
    Ok(create_cipher(&r, pk))
}

/// The deterministic half of [`encapsulate`] (`createCipher`,
/// `sntrup4591761.go:82-103`).
pub(crate) fn create_cipher(r: &[i8; P], pk: &PublicKey) -> (Ciphertext, SharedKey) {
    let mut h = [0i16; P];
    rq::decode(&pk.0, &mut h);
    // c = Round3(h·r). Before rounding, c and h give r back: wiped.
    let mut c = Zeroizing::new([0i16; P]);
    rq::mult(&mut c, &h, r);
    rq::round3(&mut c);

    let mut s = Zeroizing::new([0u8; 64]);
    hash_small(r, &mut s);
    let mut k = SharedKey::zeroed();
    k.raw_mut().copy_from_slice(&s[32..]);

    let mut ct = Ciphertext([0u8; CIPHERTEXT_SIZE]);
    ct.0[..CT_BODY].copy_from_slice(&s[..32]);
    rq::encode_rounded(&c, window_mut(&mut ct.0[..], CT_BODY));
    (ct, k)
}

/// Decapsulates `ct` with `sk` (`Decapsulate`, `sntrup4591761.go:121-164`).
///
/// Returns the shared key and whether the ciphertext is valid for the key,
/// as Go returns the key and `ok` (1 or 0). The key is returned in both
/// cases, as Go returns it: on failure it is SHA-512 of the decrypted r,
/// which the sender did not choose, and must not be used. dcrd's mixing
/// code treats `ok != 1` as `sntrup4591761: decapsulate failure`
/// (`mixing@v0.7.5/keyagreement.go:82-88`).
///
/// The ciphertext is valid when the decrypted r has weight 286,
/// re-encrypts to the decoded c, and hashes to the confirmation; the three
/// checks are combined in constant time, as in Go. Each check is tested on
/// its own, against Go and as a property: a ciphertext built honestly from
/// an r of the wrong weight fails only the first, and an honest one with
/// a single coefficient moved by 3 only the second (the single-check
/// ciphertexts of the integration tests' `common` module). Any 1047 and
/// 1600 bytes are accepted, with Go's result. More than one encoding of a ciphertext
/// decapsulates as valid (Quirk QK-0740).
pub fn decapsulate(ct: &Ciphertext, sk: &PrivateKey) -> (SharedKey, Choice) {
    // t = c·f in Rq.
    let mut f = Zeroizing::new([0i8; P]);
    zx::decode(window(&sk.as_bytes()[..], 0), &mut f);
    let mut c = [0i16; P];
    rq::decode_rounded(window(&ct.0[..], CT_BODY), &mut c);
    let mut t = Zeroizing::new([0i16; P]);
    rq::mult(&mut t, &c, &f);

    // t3 = t·3 mod q, then mod 3. Go multiplies by 3 in int16.
    let mut t3 = Zeroizing::new([0i8; P]);
    for (t3i, &ti) in t3.iter_mut().zip(t.iter()) {
        *t3i = freeze3(i32::from(freezeq(i32::from(3i16.wrapping_mul(ti)))));
    }

    // r = t3·(1/g) in R3.
    let mut gr = Zeroizing::new([0i8; P]);
    zx::decode(window(&sk.as_bytes()[..], SK_GINV), &mut gr);
    let mut r = Zeroizing::new([0i8; P]);
    r3::mult(&mut r, &t3, &gr);

    // The weight of r must be 286. Go counts with ConstantTimeSelect.
    let mut weight: u16 = 0;
    for &ri in r.iter() {
        weight = weight.wrapping_add(u16::from(!ri.ct_eq(&0).unwrap_u8() & 1));
    }
    let mut ok = weight.ct_eq(&(W as u16));

    // r must re-encrypt to c: Round3(h·r) == c, coefficient by coefficient.
    let mut h = [0i16; P];
    rq::decode(window(&sk.as_bytes()[..], SK_PK), &mut h);
    let mut hr = Zeroizing::new([0i16; P]);
    rq::mult(&mut hr, &h, &r);
    rq::round3(&mut hr);
    let mut diff: u16 = 0;
    for (&a, &b) in hr.iter().zip(c.iter()) {
        // Go: ConstantTimeEq(int32(hr[i]-c[i]), 0), the difference in int16.
        diff |= a.wrapping_sub(b) as u16;
    }
    ok &= diff.ct_eq(&0);

    // The confirmation must be the first half of SHA-512(zx.Encode(r)).
    let mut s = Zeroizing::new([0u8; 64]);
    hash_small(&r, &mut s);
    ok &= s[..32].ct_eq(&ct.0[..CT_BODY]);

    let mut k = SharedKey::zeroed();
    k.raw_mut().copy_from_slice(&s[32..]);
    (k, ok)
}
