// Copyright (c) 2014-2016 The btcsuite developers
// Copyright (c) 2015-2021 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Decred hierarchical deterministic extended keys: a port of dcrd's
//! `hdkeychain` v3.1.3, the version dcrwallet links.
//!
//! # The Decred variant
//!
//! Derivation follows [BIP32] with one legacy difference that every Decred
//! wallet depends on. A private key produced by [`ExtendedKey::child`] is
//! stored with its leading zero bytes **stripped** (Go converts the scalar
//! to a byte slice and drops the zeros), and hardened derivation copies
//! whatever length is stored into the HMAC preimage, left-aligned. So a
//! child whose private key begins with `00` derives different hardened
//! children than plain BIP32 would: the preimage is
//! `00 ‖ key31 ‖ 00 ‖ ser32(i)` instead of `00 ‖ 00 ‖ key31 ‖ ser32(i)`.
//!
//! Which form a key carries depends on how it was made, and this type keeps
//! that provenance exactly as Go does, as the stored length:
//!
//! | Constructor | Stored private key length |
//! | --- | --- |
//! | [`ExtendedKey::new_master`] | always 32 |
//! | [`ExtendedKey::new_key_from_string`] | always 32 |
//! | [`ExtendedKey::child`] | 32 minus the leading zero bytes of the result |
//! | [`ExtendedKey::child_bip32_std`] | always 32 |
//!
//! [`ExtendedKey::serialized_priv_key`] returns the stored bytes, so it can
//! be shorter than 32; [`ExtendedKey::priv_key_bytes`] returns them padded
//! to 32. [`ExtendedKey::string`] always pads to 32, which means that
//! serializing a stripped key and parsing it back changes its hardened
//! children. `child_bip32_std` only controls how the *child* is stored:
//! the preimage is still laid out from the parent's stored form.
//!
//! # Secrets
//!
//! [`ExtendedKey`] implements neither `Display` nor a revealing `Debug`:
//! Go's `String()`, which for a private key is the private extended key
//! string, is the explicit method [`ExtendedKey::string`], and it returns a
//! [`Zeroizing`] string.
//!
//! What is wiped:
//!
//! - A key's private key bytes and chain code live in one heap allocation
//!   that every constructor fills in place and that is wiped when the key
//!   is dropped or [zeroed](ExtendedKey::zero), so moving a key leaves no
//!   copy of them behind. (The public key, version bytes, depth, child
//!   number and parent fingerprint are stored inline; they are public.)
//! - Every buffer this crate's own code puts a seed-derived secret in: the
//!   HMAC-SHA512 output and padded key, the scalar byte encodings, the
//!   82-byte serialization, and the base58 digits in both directions (the
//!   crate decodes and encodes extended keys itself, over fixed wiped
//!   buffers, rather than with `dcroxide-base58`, whose working vectors
//!   are freed unwiped), and the k256 scalars it holds.
//! - The HMAC and SHA-512 states, wiped on drop by the `zeroize` features
//!   of `hmac`, `sha2` and `digest`.
//!
//! What is not, and why:
//!
//! - Temporaries inside k256: the by-value copies of scalar encodings
//!   that `Scalar::from_repr` takes, and the scalar multiplication's
//!   intermediate points and windows. k256 exposes no way to wipe them.
//! - HMAC-SHA512 working state beyond what those `zeroize` features cover
//!   (for example values spilled to the stack inside the compression
//!   function).
//! - The BLAKE-256 state in `dcroxide-crypto` that computes the checksum
//!   over a serialized private key; it has no wiping.
//! - Anything the caller copies out: the slice of
//!   [`ExtendedKey::serialized_priv_key`], strings from
//!   [`ExtendedKey::string`] once moved out of their [`Zeroizing`], and
//!   [clones](ExtendedKey#impl-Clone-for-ExtendedKey), which are
//!   independent keys.
//!
//! # Differences from the Go package
//!
//! Go's `ExtendedKey` is a pointer whose byte slices are shared: `Neuter`
//! returns a key that aliases the private key's memoized public key, chain
//! code and fingerprint, so `Zero` on one clears parts of the other, and
//! `Neuter` on a public key returns the same pointer. Here every key owns
//! its bytes, so zeroing a key affects only that key. Likewise Go's
//! `SerializedPrivKey` hands out the key's own storage (dcrwallet's one
//! caller zeroes it afterwards, zeroing the extended key too), while
//! [`ExtendedKey::serialized_priv_key`] lends a read-only slice. Go
//! memoizes a private key's public key on first use; here it is computed
//! when the key is made, which is not observable. Every value any method
//! returns is otherwise the same as Go's, including for the degenerate
//! cases Go does not reject (a zero private child sum, a public child sum
//! at infinity), which no seed can reach in practice.
//!
//! # Features
//!
//! - `chaincfg`: implements [`NetworkParams`] for `dcroxide_chaincfg::Params`
//!   and converts it to [`HdVersions`].
//!
//! [BIP32]: https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki

#![no_std]

extern crate alloc;

mod base58;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
use k256::elliptic_curve::Group;
use k256::elliptic_curve::PrimeField;
use k256::elliptic_curve::group::GroupEncoding;
use k256::elliptic_curve::ops::Reduce;
use k256::elliptic_curve::point::DecompressPoint;
use k256::elliptic_curve::subtle::Choice;
use k256::{AffinePoint, FieldBytes, ProjectivePoint, Scalar};
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// The recommended length in bytes for a seed to a master node.
pub const RECOMMENDED_SEED_LEN: u8 = 32;

/// The index at which a hardened key starts. Each extended key has 2^31
/// normal child keys and 2^31 hardened child keys: normal children are
/// `[0, 2^31 - 1]` and hardened children `[2^31, 2^32 - 1]`.
pub const HARDENED_KEY_START: u32 = 0x8000_0000;

/// The minimum number of bytes allowed for a seed to a master node.
pub const MIN_SEED_BYTES: u8 = 16;

/// The maximum number of bytes allowed for a seed to a master node.
pub const MAX_SEED_BYTES: u8 = 64;

/// Length of a serialized extended key: 4 bytes version, 1 byte depth,
/// 4 bytes fingerprint, 4 bytes child number, 32 bytes chain code and
/// 33 bytes key data.
const SERIALIZED_KEY_LEN: usize = 4 + 1 + 4 + 4 + 32 + 33;

/// The serialized key plus its 4-byte checksum.
const DECODED_DATA_LEN: usize = SERIALIZED_KEY_LEN + 4;

/// The longest string [`ExtendedKey::new_key_from_string`] attempts to
/// decode: `ceil(82 * log_58(256))` estimated as Go does,
/// `(82 * 137 / 100) + 1 = 113`.
const MAX_KEY_LEN: usize = (DECODED_DATA_LEN * 137 / 100) + 1;

/// The HMAC key of master node derivation.
const MASTER_KEY: &[u8] = b"Bitcoin seed";

/// What `String()` returns for a key whose stored key bytes are empty.
const ZEROED_KEY_STRING: &str = "zeroed extended key";

/// The secp256k1 field prime, big-endian.
const FIELD_PRIME: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xff, 0xff, 0xfc, 0x2f,
];

/// The errors of this package, with Go's error texts as their `Display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// `ErrDeriveHardFromPublic`: a hardened child was requested from a
    /// public extended key.
    DeriveHardFromPublic,
    /// `ErrNotPrivExtKey`: a private key was requested from a public
    /// extended key.
    NotPrivExtKey,
    /// `ErrInvalidChild`: the child at this index falls outside the valid
    /// range for secp256k1 private keys. The caller should skip this index
    /// and use the next one.
    InvalidChild,
    /// `ErrUnusableSeed`: the seed derives to an invalid master key, and
    /// another seed must be chosen. Also returned for a private extended
    /// key string whose key is zero or not below the group order.
    UnusableSeed,
    /// `ErrInvalidSeedLen`: the seed or seed length is outside
    /// [`MIN_SEED_BYTES`]..=[`MAX_SEED_BYTES`].
    InvalidSeedLen,
    /// `ErrBadChecksum`: the checksum of a serialized extended key does not
    /// match.
    BadChecksum,
    /// `ErrInvalidKeyLen`: the serialized extended key is not the expected
    /// length (an invalid base58 character also lands here, since the
    /// decoder then returns nothing).
    InvalidKeyLen,
    /// `ErrWrongNetwork`: the serialized extended key's version is neither
    /// of the network's.
    WrongNetwork,
    /// A public key failed to parse (the secp256k1 package's error).
    PubKey(PubKeyError),
    /// Not a Go error: [`ExtendedKey::priv_key_bytes`] of a private key
    /// whose stored key is empty, which only a zero private child sum
    /// produces (Go's `PrivKeyFromBytes` would give the zero scalar).
    EmptyPrivKey,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::DeriveHardFromPublic => "cannot derive a hardened key from a public key",
            Error::NotPrivExtKey => "unable to create private keys from a public extended key",
            Error::InvalidChild => "the extended key at this index is invalid",
            Error::UnusableSeed => "unusable seed",
            Error::InvalidSeedLen => "seed length must be between 128 and 512 bits",
            Error::BadChecksum => "bad extended key checksum",
            Error::InvalidKeyLen => "the provided serialized extended key length is invalid",
            Error::WrongNetwork => "the provided serialized extended key is for the wrong network",
            Error::PubKey(e) => return fmt::Display::fmt(e, f),
            Error::EmptyPrivKey => "the extended private key is empty",
        })
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Error::PubKey(e) => Some(e),
            _ => None,
        }
    }
}

/// The kind of a [`PubKeyError`], named as the secp256k1 package's
/// `ErrorKind` values are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PubKeyErrorKind {
    /// `ErrPubKeyInvalidLen`: the serialized public key has the wrong length.
    InvalidLen,
    /// `ErrPubKeyInvalidFormat`: the format byte is not one the length allows.
    InvalidFormat,
    /// `ErrPubKeyXTooBig`: the x coordinate is not below the field prime.
    XTooBig,
    /// `ErrPubKeyNotOnCurve`: no curve point has this x coordinate.
    NotOnCurve,
}

impl fmt::Display for PubKeyErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PubKeyErrorKind::InvalidLen => "ErrPubKeyInvalidLen",
            PubKeyErrorKind::InvalidFormat => "ErrPubKeyInvalidFormat",
            PubKeyErrorKind::XTooBig => "ErrPubKeyXTooBig",
            PubKeyErrorKind::NotOnCurve => "ErrPubKeyNotOnCurve",
        })
    }
}

/// A public key parse error, as `secp256k1.ParsePubKey` (v4.4.1) reports
/// it: a kind and a description, the description being the error's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PubKeyError {
    kind: PubKeyErrorKind,
    description: String,
}

impl PubKeyError {
    /// The error's kind.
    pub fn kind(&self) -> PubKeyErrorKind {
        self.kind
    }

    /// The error's text (Go's `Description`).
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl fmt::Display for PubKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl core::error::Error for PubKeyError {}

/// The errors of [`generate_seed`]: Go's `ErrInvalidSeedLen`, or the error
/// of the randomness source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeedError<E> {
    /// The length is outside [`MIN_SEED_BYTES`]..=[`MAX_SEED_BYTES`]
    /// (`ErrInvalidSeedLen`; same text as [`Error::InvalidSeedLen`]).
    InvalidSeedLen,
    /// The randomness source failed.
    Rand(E),
}

impl<E: fmt::Display> fmt::Display for SeedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeedError::InvalidSeedLen => fmt::Display::fmt(&Error::InvalidSeedLen, f),
            SeedError::Rand(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for SeedError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            SeedError::InvalidSeedLen => None,
            SeedError::Rand(e) => Some(e),
        }
    }
}

/// The hierarchical deterministic extended key version bytes of a network
/// (Go's `NetworkParams` interface).
pub trait NetworkParams {
    /// The extended private key version bytes.
    fn hd_priv_key_version(&self) -> [u8; 4];

    /// The extended public key version bytes.
    fn hd_pub_key_version(&self) -> [u8; 4];
}

/// A plain pair of extended key version bytes, with the four Decred
/// networks' values (dcrd `chaincfg` v3.3.0) as constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HdVersions {
    /// The extended private key version bytes.
    pub private: [u8; 4],
    /// The extended public key version bytes.
    pub public: [u8; 4],
}

impl HdVersions {
    /// Mainnet: `dprv` / `dpub`.
    pub const MAINNET: HdVersions = HdVersions {
        private: [0x02, 0xfd, 0xa4, 0xe8],
        public: [0x02, 0xfd, 0xa9, 0x26],
    };

    /// Testnet version 3: `tprv` / `tpub`.
    pub const TESTNET3: HdVersions = HdVersions {
        private: [0x04, 0x35, 0x83, 0x97],
        public: [0x04, 0x35, 0x87, 0xd1],
    };

    /// Simnet: `sprv` / `spub`.
    pub const SIMNET: HdVersions = HdVersions {
        private: [0x04, 0x20, 0xb9, 0x03],
        public: [0x04, 0x20, 0xbd, 0x3d],
    };

    /// Regnet: `rprv` / `rpub`.
    pub const REGNET: HdVersions = HdVersions {
        private: [0xea, 0xb4, 0x04, 0x48],
        public: [0xea, 0xb4, 0xf9, 0x87],
    };
}

impl NetworkParams for HdVersions {
    fn hd_priv_key_version(&self) -> [u8; 4] {
        self.private
    }

    fn hd_pub_key_version(&self) -> [u8; 4] {
        self.public
    }
}

#[cfg(feature = "chaincfg")]
impl NetworkParams for dcroxide_chaincfg::Params {
    fn hd_priv_key_version(&self) -> [u8; 4] {
        self.hd_private_key_id
    }

    fn hd_pub_key_version(&self) -> [u8; 4] {
        self.hd_public_key_id
    }
}

#[cfg(feature = "chaincfg")]
impl From<&dcroxide_chaincfg::Params> for HdVersions {
    fn from(params: &dcroxide_chaincfg::Params) -> Self {
        HdVersions {
            private: params.hd_private_key_id,
            public: params.hd_public_key_id,
        }
    }
}

/// The secret part of an extended key, kept on the heap so that moving the
/// key moves only a pointer, and wiped on drop.
struct Secret {
    /// The stored private key bytes, `key[..len]`, left-aligned; the rest is
    /// zero. Unused (zero) for public and zeroed keys.
    key: [u8; 32],
    /// 32, or fewer for a key `child` stored stripped.
    len: u8,
    /// The chain code.
    chain_code: [u8; 32],
}

impl Secret {
    /// A zeroed secret, allocated on the heap, to be filled in place.
    fn boxed() -> Box<Secret> {
        Box::new(Secret {
            key: [0; 32],
            len: 0,
            chain_code: [0; 32],
        })
    }

    /// The stored private key bytes.
    fn stored(&self) -> &[u8] {
        &self.key[..usize::from(self.len)]
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.key.zeroize();
        self.len.zeroize();
        self.chain_code.zeroize();
    }
}

/// What kind of key an extended key is, with its (public) compressed
/// public key.
#[derive(Clone, Copy)]
enum Kind {
    /// A private key; the private key bytes are in [`Secret`]. `pub_key`
    /// is the compressed public key (Go memoizes it; here it is computed
    /// when the key is made).
    Private { pub_key: [u8; 33] },
    /// A public key.
    Public { pub_key: [u8; 33] },
    /// A key cleared by [`ExtendedKey::zero`]: Go's public key with an
    /// empty key slice.
    Zeroed,
}

/// A hierarchical deterministic extended key (Go's `*ExtendedKey`).
///
/// See the [crate documentation](crate) for how the stored length of a
/// private key decides its hardened children.
pub struct ExtendedKey {
    priv_ver: [u8; 4],
    pub_ver: [u8; 4],
    kind: Kind,
    secret: Box<Secret>,
    parent_fp: [u8; 4],
    child_num: u32,
    depth: u16,
}

impl Clone for ExtendedKey {
    /// A copy that is an independent key: it has its own secret
    /// allocation, so [`zero`](ExtendedKey::zero) on the original does not
    /// clear it (nor the reverse), and it is wiped when it is itself
    /// dropped or zeroed.
    fn clone(&self) -> Self {
        let mut secret = Secret::boxed();
        secret.key.copy_from_slice(&self.secret.key);
        secret.len = self.secret.len;
        secret.chain_code.copy_from_slice(&self.secret.chain_code);
        ExtendedKey {
            priv_ver: self.priv_ver,
            pub_ver: self.pub_ver,
            kind: self.kind,
            secret,
            parent_fp: self.parent_fp,
            child_num: self.child_num,
            depth: self.depth,
        }
    }
}

impl ZeroizeOnDrop for ExtendedKey {}

impl fmt::Debug for ExtendedKey {
    /// Shows the key's kind and position; never the private key or the
    /// chain code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtendedKey")
            .field("is_private", &self.is_private())
            .field("zeroed", &matches!(self.kind, Kind::Zeroed))
            .field("depth", &self.depth)
            .field("child_num", &self.child_num)
            .field("parent_fingerprint", &self.parent_fingerprint())
            .finish_non_exhaustive()
    }
}

/// Wipes a scalar encoding.
fn wipe(bytes: &mut FieldBytes) {
    bytes.as_mut_slice().zeroize();
}

/// HMAC-SHA512 of `data` under `key`. Both keys this package uses (the
/// 12-byte master key and 32-byte chain codes) are shorter than SHA-512's
/// 128-byte block, so HMAC uses them zero-padded to the block size; the
/// padded key is passed directly, which makes construction infallible.
fn hmac_sha512(key: &[u8], data: &[u8]) -> Zeroizing<[u8; 64]> {
    let mut padded = Key::<Hmac<Sha512>>::default();
    padded[..key.len()].copy_from_slice(key);
    let mut mac = Hmac::<Sha512>::new(&padded);
    padded.as_mut_slice().zeroize();
    mac.update(data);
    let mut out = Zeroizing::new([0u8; 64]);
    // The finalized output wipes itself on drop (digest's `zeroize`).
    out.copy_from_slice(mac.finalize().as_bytes());
    out
}

/// `RIPEMD160(BLAKE256(v))`.
fn hash160(v: &[u8]) -> [u8; 20] {
    dcroxide_crypto::ripemd160::sum160(&dcroxide_crypto::blake256::sum256(v))
}

/// The first four bytes of `BLAKE256(BLAKE256(v))`.
fn double_blake256_cksum(v: &[u8]) -> [u8; 4] {
    let second = dcroxide_crypto::blake256::sum256d(v);
    [second[0], second[1], second[2], second[3]]
}

/// The scalar for 32 big-endian bytes, or `None` when they are zero or not
/// below the group order (Go's `SetByteSlice` overflow or `IsZero`).
fn nonzero_scalar(bytes: &[u8; 32]) -> Option<Scalar> {
    let mut repr = FieldBytes::default();
    repr.copy_from_slice(bytes);
    let scalar = Option::<Scalar>::from(Scalar::from_repr(repr));
    wipe(&mut repr);
    let mut scalar = scalar?;
    if bool::from(scalar.is_zero()) {
        scalar.zeroize();
        None
    } else {
        Some(scalar)
    }
}

/// `k·G`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "group operation on the curve: scalar multiplication cannot overflow"
)]
fn base_mul(k: &Scalar) -> ProjectivePoint {
    ProjectivePoint::GENERATOR * k
}

/// `a + b mod n`.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "field operation: addition modulo the group order cannot overflow"
)]
fn scalar_add(a: &Scalar, b: &Scalar) -> Scalar {
    a + b
}

/// `P + Q` on the curve.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "group operation on the curve: point addition cannot overflow"
)]
fn point_add(p: &ProjectivePoint, q: &ProjectivePoint) -> ProjectivePoint {
    p + q
}

/// The compressed serialization of a point as Go's
/// `PublicKey.SerializeCompressed` gives it. The point at infinity, which
/// Go's affine conversion turns into (0, 0), serializes as `02` followed by
/// 32 zero bytes.
fn serialize_compressed(point: &ProjectivePoint) -> [u8; 33] {
    let mut out = [0u8; 33];
    if bool::from(point.is_identity()) {
        out[0] = 0x02;
    } else {
        out.copy_from_slice(&point.to_affine().to_bytes());
    }
    out
}

/// The compressed public key of a private scalar (Go's
/// `PrivKeyFromBytes(k).PubKey().SerializeCompressed()`).
fn pub_key_of(scalar: &Scalar) -> [u8; 33] {
    serialize_compressed(&base_mul(scalar))
}

/// Parses a 33-byte compressed public key as `secp256k1.ParsePubKey`
/// (v4.4.1) does, with its error texts.
fn parse_compressed_pub_key(b: &[u8; 33]) -> Result<AffinePoint, PubKeyError> {
    let format = b[0];
    if format != 0x02 && format != 0x03 {
        return Err(PubKeyError {
            kind: PubKeyErrorKind::InvalidFormat,
            description: format!("invalid public key: unsupported format: {format:x}"),
        });
    }
    let mut x = [0u8; 32];
    x.copy_from_slice(&b[1..33]);
    // Big-endian byte order makes the lexicographic comparison numeric.
    if x >= FIELD_PRIME {
        return Err(PubKeyError {
            kind: PubKeyErrorKind::XTooBig,
            description: String::from("invalid public key: x >= field prime"),
        });
    }
    let x_bytes: FieldBytes = x.into();
    Option::from(AffinePoint::decompress(
        &x_bytes,
        Choice::from(u8::from(format == 0x03)),
    ))
    .ok_or_else(|| {
        let mut hex = String::with_capacity(64);
        for byte in x {
            hex.push_str(&format!("{byte:02x}"));
        }
        PubKeyError {
            kind: PubKeyErrorKind::NotOnCurve,
            description: format!(
                "invalid public key: x coordinate {hex} is not on the secp256k1 curve"
            ),
        }
    })
}

impl ExtendedKey {
    /// Creates a master node from a seed (Go's `NewMaster`).
    ///
    /// The seed must be [`MIN_SEED_BYTES`] to [`MAX_SEED_BYTES`] long,
    /// else [`Error::InvalidSeedLen`]. In the vanishingly rare case
    /// (< 1 in 2^127) that the seed derives to an invalid private key the
    /// error is [`Error::UnusableSeed`] and another seed must be used. The
    /// master's private key is stored as all 32 bytes, leading zeros kept.
    pub fn new_master<N: NetworkParams + ?Sized>(seed: &[u8], net: &N) -> Result<Self, Error> {
        if seed.len() < usize::from(MIN_SEED_BYTES) || seed.len() > usize::from(MAX_SEED_BYTES) {
            return Err(Error::InvalidSeedLen);
        }

        // I = HMAC-SHA512(Key = "Bitcoin seed", Data = S); Il is the master
        // secret key and Ir the master chain code.
        let lr = hmac_sha512(MASTER_KEY, seed);
        let mut secret = Secret::boxed();
        secret.key.copy_from_slice(&lr[..32]);
        secret.len = 32;
        secret.chain_code.copy_from_slice(&lr[32..]);

        let Some(mut scalar) = nonzero_scalar(&secret.key) else {
            return Err(Error::UnusableSeed);
        };
        let pub_key = pub_key_of(&scalar);
        scalar.zeroize();
        Ok(ExtendedKey {
            priv_ver: net.hd_priv_key_version(),
            pub_ver: net.hd_pub_key_version(),
            kind: Kind::Private { pub_key },
            secret,
            parent_fp: [0; 4],
            child_num: 0,
            depth: 0,
        })
    }

    /// Parses a base58 extended key string, which must carry one of the
    /// network's two versions (Go's `NewKeyFromString`).
    ///
    /// Whether the result is private is decided by the key data (a leading
    /// `00` byte), not by which version matched, and the result carries
    /// both of `net`'s versions. A private key is stored as all 32 bytes.
    /// Errors, in the order checked: [`Error::InvalidKeyLen`] for a string
    /// longer than 113 bytes, an invalid base58 character or a decoded
    /// length other than 82; [`Error::BadChecksum`];
    /// [`Error::WrongNetwork`]; [`Error::UnusableSeed`] for private key
    /// data that is zero or not below the group order; and
    /// [`Error::PubKey`] for public key data that does not parse.
    pub fn new_key_from_string<N: NetworkParams + ?Sized>(
        key: &str,
        net: &N,
    ) -> Result<Self, Error> {
        if key.len() > MAX_KEY_LEN {
            return Err(Error::InvalidKeyLen);
        }

        // decred/base58 Decode, which returns nothing for an invalid
        // character, then the 82-byte length check.
        let decoded = base58::decode_extended_key(key.as_bytes()).ok_or(Error::InvalidKeyLen)?;

        // version (4) || depth (1) || parent fingerprint (4) ||
        // child num (4) || chain code (32) || key data (33) || checksum (4)
        let payload = &decoded[..SERIALIZED_KEY_LEN];
        let checksum = &decoded[SERIALIZED_KEY_LEN..];
        if checksum != double_blake256_cksum(payload) {
            return Err(Error::BadChecksum);
        }

        let priv_ver = net.hd_priv_key_version();
        let pub_ver = net.hd_pub_key_version();
        let version = &payload[..4];
        if version != priv_ver && version != pub_ver {
            return Err(Error::WrongNetwork);
        }

        let depth = u16::from(payload[4]);
        let mut parent_fp = [0u8; 4];
        parent_fp.copy_from_slice(&payload[5..9]);
        let mut child_num = [0u8; 4];
        child_num.copy_from_slice(&payload[9..13]);
        let child_num = u32::from_be_bytes(child_num);
        let mut secret = Secret::boxed();
        secret.chain_code.copy_from_slice(&payload[13..45]);
        let key_data = &payload[45..78];

        let kind = if key_data[0] == 0x00 {
            secret.key.copy_from_slice(&key_data[1..]);
            secret.len = 32;
            let Some(mut scalar) = nonzero_scalar(&secret.key) else {
                return Err(Error::UnusableSeed);
            };
            let pub_key = pub_key_of(&scalar);
            scalar.zeroize();
            Kind::Private { pub_key }
        } else {
            let mut pub_key = [0u8; 33];
            pub_key.copy_from_slice(key_data);
            parse_compressed_pub_key(&pub_key).map_err(Error::PubKey)?;
            Kind::Public { pub_key }
        };

        Ok(ExtendedKey {
            priv_ver,
            pub_ver,
            kind,
            secret,
            parent_fp,
            child_num,
            depth,
        })
    }

    /// The child number of the extended key.
    pub fn child_num(&self) -> u32 {
        self.child_num
    }

    /// The depth of the extended key. It is 16 bits wide in memory (and
    /// wraps at 65536, as Go's `uint16` does) while the string form keeps
    /// it modulo 256.
    pub fn depth(&self) -> u16 {
        self.depth
    }

    /// Whether this is a private extended key. A zeroed key is not.
    pub fn is_private(&self) -> bool {
        matches!(self.kind, Kind::Private { .. })
    }

    /// The fingerprint of the parent key, its four bytes read big-endian.
    pub fn parent_fingerprint(&self) -> u32 {
        u32::from_be_bytes(self.parent_fp)
    }

    /// The child at index `i`, derived with the Decred variant (Go's
    /// `Child`): a derived private key is stored with its leading zero
    /// bytes stripped, which changes its own hardened children. This is
    /// the derivation every Decred wallet uses.
    ///
    /// A private key derives a private child, a public key a public child.
    /// An index of [`HARDENED_KEY_START`] or more derives a hardened child,
    /// which needs a private key ([`Error::DeriveHardFromPublic`]
    /// otherwise). In the vanishingly rare case (< 1 in 2^127) that the
    /// index derives no valid key the error is [`Error::InvalidChild`],
    /// and the caller should skip to the next index.
    pub fn child(&self, i: u32) -> Result<Self, Error> {
        self.derive(i, false)
    }

    /// As [`child`](Self::child), except that a derived private key keeps
    /// its leading zeros and is always stored as 32 bytes (Go's
    /// `ChildBIP32Std`). The preimage of this step is still laid out from
    /// this key's stored form, so it is strict BIP32 only from a parent
    /// that was not itself stored stripped.
    pub fn child_bip32_std(&self, i: u32) -> Result<Self, Error> {
        self.derive(i, true)
    }

    /// Go's `child(i, strictBIP32)` (`extendedkey.go:218-349`): the HMAC
    /// step, then [`derive_from_ilr`](Self::derive_from_ilr).
    fn derive(&self, i: u32, strict_bip32: bool) -> Result<Self, Error> {
        let is_child_hardened = i >= HARDENED_KEY_START;
        if !self.is_private() && is_child_hardened {
            return Err(Error::DeriveHardFromPublic);
        }

        // Hardened: 0x00 || stored private key || ser32(i), where a stored
        // key shorter than 32 bytes is left-aligned after the 0x00 and the
        // remaining bytes up to the index stay zero. Normal:
        // serP(parent public key) || ser32(i).
        let mut data = Zeroizing::new([0u8; 37]);
        if is_child_hardened {
            let stored = self.stored_priv_key();
            data[1..stored.len().saturating_add(1)].copy_from_slice(stored);
        } else {
            let pub_key = self.pub_key_bytes();
            data[..pub_key.len()].copy_from_slice(pub_key);
        }
        data[33..].copy_from_slice(&i.to_be_bytes());

        // I = HMAC-SHA512(Key = chain code, Data = data).
        let ilr = hmac_sha512(&self.secret.chain_code, &data[..]);
        self.derive_from_ilr(i, strict_bip32, &ilr)
    }

    /// The rest of Go's `child` from `I = Il || Ir` on
    /// (`extendedkey.go:265-348`): Il is the intermediate key, Ir the child
    /// chain code.
    fn derive_from_ilr(&self, i: u32, strict_bip32: bool, ilr: &[u8; 64]) -> Result<Self, Error> {
        let mut secret = Secret::boxed();
        secret.chain_code.copy_from_slice(&ilr[32..]);
        let il: &[u8; 32] = ilr[..32].try_into().map_err(|_| Error::InvalidChild)?;
        let Some(mut il_scalar) = nonzero_scalar(il) else {
            return Err(Error::InvalidChild);
        };

        let kind = match self.kind {
            Kind::Private { .. } => {
                // childKey = parse256(Il) + parentKey.  Go reduces the parent
                // bytes modulo n (SetByteSlice); stored keys are below n.
                let stored = self.stored_priv_key();
                let mut parent_repr = FieldBytes::default();
                parent_repr[32usize.saturating_sub(stored.len())..].copy_from_slice(stored);
                let mut parent = <Scalar as Reduce<FieldBytes>>::reduce(&parent_repr);
                wipe(&mut parent_repr);
                let mut sum = scalar_add(&il_scalar, &parent);
                parent.zeroize();
                let mut sum_repr = sum.to_bytes();

                // The Decred variant strips leading zero bytes; a zero sum
                // (not rejected, as in Go) strips to an empty key.
                let strip: u8 = if strict_bip32 {
                    0
                } else {
                    sum_repr
                        .iter()
                        .take_while(|&&b| b == 0)
                        .fold(0u8, |n, _| n.saturating_add(1))
                };
                let len = 32u8.saturating_sub(strip);
                secret.key[..usize::from(len)].copy_from_slice(&sum_repr[usize::from(strip)..]);
                secret.len = len;
                wipe(&mut sum_repr);
                let pub_key = pub_key_of(&sum);
                sum.zeroize();
                Kind::Private { pub_key }
            }
            Kind::Public { .. } | Kind::Zeroed => {
                // childKey = serP(point(parse256(Il)) + parentKey).  Go
                // rejects an intermediate point with a zero coordinate after
                // affine conversion, which only the point at infinity has.
                let im_pub_key = base_mul(&il_scalar);
                il_scalar.zeroize();
                if bool::from(im_pub_key.is_identity()) {
                    return Err(Error::InvalidChild);
                }
                let parent = match &self.kind {
                    Kind::Public { pub_key } => parse_compressed_pub_key(pub_key),
                    // A zeroed key's key slice is empty.
                    _ => Err(PubKeyError {
                        kind: PubKeyErrorKind::InvalidLen,
                        description: String::from("malformed public key: invalid length: 0"),
                    }),
                }
                .map_err(Error::PubKey)?;
                // A sum at infinity is not rejected, as in Go.
                let child = point_add(&im_pub_key, &ProjectivePoint::from(parent));
                Kind::Public {
                    pub_key: serialize_compressed(&child),
                }
            }
        };
        il_scalar.zeroize();

        // The child's parent fingerprint is the first 4 bytes of
        // RIPEMD160(BLAKE256(parent public key)).
        let fp = hash160(self.pub_key_bytes());
        Ok(ExtendedKey {
            priv_ver: self.priv_ver,
            pub_ver: self.pub_ver,
            kind,
            secret,
            parent_fp: [fp[0], fp[1], fp[2], fp[3]],
            child_num: i,
            depth: self.depth.wrapping_add(1),
        })
    }

    /// The extended public key of this key (Go's `Neuter`); a public or
    /// zeroed key is returned as a copy of itself.
    pub fn neuter(&self) -> Self {
        match self.kind {
            Kind::Private { pub_key } => {
                let mut secret = Secret::boxed();
                secret.chain_code.copy_from_slice(&self.secret.chain_code);
                ExtendedKey {
                    priv_ver: self.priv_ver,
                    pub_ver: self.pub_ver,
                    kind: Kind::Public { pub_key },
                    secret,
                    parent_fp: self.parent_fp,
                    child_num: self.child_num,
                    depth: self.depth,
                }
            }
            Kind::Public { .. } | Kind::Zeroed => self.clone(),
        }
    }

    /// The 33-byte compressed public key (Go's `SerializedPubKey`); empty
    /// for a zeroed key.
    pub fn serialized_pub_key(&self) -> &[u8] {
        self.pub_key_bytes()
    }

    /// The stored private key bytes (Go's `SerializedPrivKey`), or
    /// [`Error::NotPrivExtKey`] for a public or zeroed key.
    ///
    /// For a key derived with [`child`](Self::child) these are the key's
    /// big-endian bytes **without leading zeros**, so they can be shorter
    /// than 32. To get the 32-byte private key, as Go code gets it with
    /// `secp256k1.PrivKeyFromBytes(k.SerializedPrivKey())`, use
    /// [`priv_key_bytes`](Self::priv_key_bytes).
    pub fn serialized_priv_key(&self) -> Result<&[u8], Error> {
        match self.kind {
            Kind::Private { .. } => Ok(self.stored_priv_key()),
            Kind::Public { .. } | Kind::Zeroed => Err(Error::NotPrivExtKey),
        }
    }

    /// The private key as 32 big-endian bytes: the stored bytes of
    /// [`serialized_priv_key`](Self::serialized_priv_key) left-padded with
    /// zeros, which is what Go's `secp256k1.PrivKeyFromBytes` makes of a
    /// short slice (and what [`string`](Self::string) writes). Use this
    /// where Go code does `PrivKeyFromBytes(k.SerializedPrivKey())`.
    ///
    /// [`Error::NotPrivExtKey`] for a public or zeroed key, and
    /// [`Error::EmptyPrivKey`] for the empty key only a zero private child
    /// sum produces.
    pub fn priv_key_bytes(&self) -> Result<Zeroizing<[u8; 32]>, Error> {
        let stored = self.serialized_priv_key()?;
        if stored.is_empty() {
            return Err(Error::EmptyPrivKey);
        }
        let mut out = Zeroizing::new([0u8; 32]);
        out[32usize.saturating_sub(stored.len())..].copy_from_slice(stored);
        Ok(out)
    }

    /// The extended key as a base58 string (Go's `String()`).
    ///
    /// For a private key this is the private extended key string (`dprv…`
    /// on mainnet), a secret, which is why this is an explicit method
    /// returning a [`Zeroizing`] string rather than a `Display`
    /// implementation. The private key is always written zero-padded to
    /// 32 bytes and the depth modulo 256. A key whose stored key is empty
    /// (a [zeroed](Self::zero) key) gives `zeroed extended key`.
    pub fn string(&self) -> Zeroizing<String> {
        let (version, key_data): ([u8; 4], &[u8]) = match &self.kind {
            Kind::Zeroed => return Zeroizing::new(String::from(ZEROED_KEY_STRING)),
            Kind::Private { .. } if self.secret.len == 0 => {
                return Zeroizing::new(String::from(ZEROED_KEY_STRING));
            }
            Kind::Private { .. } => (self.priv_ver, self.stored_priv_key()),
            Kind::Public { pub_key } => (self.pub_ver, &pub_key[..]),
        };

        let mut serialized = Zeroizing::new([0u8; DECODED_DATA_LEN]);
        serialized[..4].copy_from_slice(&version);
        serialized[4] = self.depth.to_le_bytes()[0];
        serialized[5..9].copy_from_slice(&self.parent_fp);
        serialized[9..13].copy_from_slice(&self.child_num.to_be_bytes());
        serialized[13..45].copy_from_slice(&self.secret.chain_code);
        if self.is_private() {
            // 0x00 || key left-padded with zeros to 32 bytes.
            let start = 46usize.saturating_add(32usize.saturating_sub(key_data.len()));
            serialized[start..SERIALIZED_KEY_LEN].copy_from_slice(key_data);
        } else {
            serialized[45..SERIALIZED_KEY_LEN].copy_from_slice(key_data);
        }
        let checksum = double_blake256_cksum(&serialized[..SERIALIZED_KEY_LEN]);
        serialized[SERIALIZED_KEY_LEN..].copy_from_slice(&checksum);
        base58::encode_extended_key(&serialized)
    }

    /// Clears the key's private key, chain code, public key and
    /// fingerprint, and resets its depth and child number (Go's `Zero`).
    ///
    /// Afterwards the key is a public key with no key bytes:
    /// [`is_private`](Self::is_private) is false,
    /// [`string`](Self::string) is `zeroed extended key`,
    /// [`serialized_pub_key`](Self::serialized_pub_key) is empty, a hardened
    /// child fails with [`Error::DeriveHardFromPublic`] and a normal child
    /// with the public key length error. The version bytes are kept. Keys
    /// already derived from this one, its neutered key and its
    /// [clones](ExtendedKey#impl-Clone-for-ExtendedKey) are independent
    /// keys and are not affected (in Go, a neutered key shares buffers with
    /// its private key).
    pub fn zero(&mut self) {
        self.secret.key.zeroize();
        self.secret.len = 0;
        self.secret.chain_code.zeroize();
        self.kind = Kind::Zeroed;
        self.parent_fp.zeroize();
        self.depth = 0;
        self.child_num = 0;
    }

    /// The stored private key bytes; empty for a public or zeroed key.
    fn stored_priv_key(&self) -> &[u8] {
        match self.kind {
            Kind::Private { .. } => self.secret.stored(),
            Kind::Public { .. } | Kind::Zeroed => &[],
        }
    }

    /// Go's `pubKeyBytes`: the compressed public key, empty for a zeroed
    /// key.
    fn pub_key_bytes(&self) -> &[u8] {
        match &self.kind {
            Kind::Private { pub_key } | Kind::Public { pub_key } => &pub_key[..],
            Kind::Zeroed => &[],
        }
    }
}

/// A random seed for [`ExtendedKey::new_master`] (Go's `GenerateSeed`),
/// with the randomness supplied by `fill`, which must fill the buffer it is
/// given from a cryptographically secure source.
///
/// `length` is in bytes and must be [`MIN_SEED_BYTES`] to
/// [`MAX_SEED_BYTES`]; [`RECOMMENDED_SEED_LEN`] is recommended. Go reads
/// the standard library's `crypto/rand`; the caller chooses here.
pub fn generate_seed<E>(
    length: u8,
    fill: impl FnOnce(&mut [u8]) -> Result<(), E>,
) -> Result<Zeroizing<Vec<u8>>, SeedError<E>> {
    if !(MIN_SEED_BYTES..=MAX_SEED_BYTES).contains(&length) {
        return Err(SeedError::InvalidSeedLen);
    }
    let mut buf = Zeroizing::new(vec![0u8; usize::from(length)]);
    fill(&mut buf).map_err(SeedError::Rand)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    //! Cases no seed reaches in practice, built directly: the call-site
    //! checks after the HMAC step (via `derive_from_ilr`), the degenerate
    //! sums Go does not reject, and depth wrapping. Expectations marked
    //! "by reading" are pinned from extendedkey.go v3.1.3 (and secp256k1
    //! v4.4.1) alone, because no seed can drive the Go oracle there.

    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: scalar negation and small index arithmetic"
    )]

    use super::*;
    use alloc::string::ToString;

    const N: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36,
        0x41, 0x41,
    ];

    fn master() -> ExtendedKey {
        ExtendedKey::new_master(&[0x42; 32], &HdVersions::MAINNET).expect("master")
    }

    /// `I` with the given Il and a fixed Ir.
    fn ilr(il: &[u8; 32]) -> [u8; 64] {
        let mut out = [0x5c; 64];
        out[..32].copy_from_slice(il);
        out
    }

    /// The payload bytes of a key's string.
    fn payload(k: &ExtendedKey) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(
            &base58::decode_extended_key(k.string().as_bytes()).expect("82 bytes")[..],
        );
        out
    }

    /// The private scalar of a key.
    fn scalar_of(k: &ExtendedKey) -> Scalar {
        nonzero_scalar(&k.priv_key_bytes().expect("private")).expect("valid")
    }

    #[test]
    fn invalid_il_is_rejected_at_the_call_site() {
        // extendedkey.go:277-280: overflow or zero Il gives ErrInvalidChild,
        // for private and public parents alike, before anything else.
        let m = master();
        let parents = [m.clone(), m.neuter()];
        for il in [[0u8; 32], N, [0xff; 32]] {
            for p in &parents {
                for strict in [false, true] {
                    for i in [0, 1, HARDENED_KEY_START] {
                        if !p.is_private() && i >= HARDENED_KEY_START {
                            continue;
                        }
                        let r = p.derive_from_ilr(i, strict, &ilr(&il));
                        assert_eq!(r.err(), Some(Error::InvalidChild));
                    }
                }
            }
        }
        // And a valid Il next to them derives.
        let mut one = [0u8; 32];
        one[31] = 1;
        assert!(parents[0].derive_from_ilr(0, false, &ilr(&one)).is_ok());
        assert!(parents[1].derive_from_ilr(0, false, &ilr(&one)).is_ok());
    }

    #[test]
    fn zero_private_sum_is_not_rejected() {
        // By reading, extendedkey.go:298-312: Il = n - parent passes the Il
        // check, ilModN.Add gives zero, Bytes() is 32 zero bytes, and with
        // strictBIP32 false the loop strips them all to an empty slice; no
        // error. String() (:442-444) then prints "zeroed extended key"
        // although IsPrivate() is true, and pubKeyBytes (:166-167) of the
        // zero scalar is the point at infinity, (0, 0) after ToAffine, so
        // 02 followed by 32 zero bytes.
        let m = master();
        let mut il = (-scalar_of(&m)).to_bytes();
        let i = HARDENED_KEY_START.wrapping_add(7);
        let k = m
            .derive_from_ilr(i, false, &ilr(il.as_mut()))
            .expect("no error");
        let mut infinity = [0u8; 33];
        infinity[0] = 0x02;
        assert!(k.is_private());
        assert_eq!(k.serialized_priv_key(), Ok(&[][..]));
        assert_eq!(k.priv_key_bytes().err(), Some(Error::EmptyPrivKey));
        assert_eq!(*k.string(), "zeroed extended key");
        assert_eq!(k.serialized_pub_key(), &infinity[..]);
        assert_eq!((k.depth(), k.child_num()), (1, i));
        // Its neutered key serializes normally with that encoding.
        assert!(k.neuter().string().starts_with("dpub"));
        // Its hardened child feeds no key bytes and derives.
        assert!(k.child(HARDENED_KEY_START).is_ok());

        // With strictBIP32 the 32 zero bytes are kept and serialized
        // (String() only special-cases an empty key).
        let k = m
            .derive_from_ilr(i, true, &ilr(il.as_mut()))
            .expect("no error");
        assert_eq!(k.serialized_priv_key(), Ok(&[0u8; 32][..]));
        assert_eq!(*k.priv_key_bytes().expect("32 bytes"), [0u8; 32]);
        assert!(k.string().starts_with("dprv"));
        assert_eq!(k.serialized_pub_key(), &infinity[..]);
    }

    #[test]
    fn public_sum_at_infinity_is_not_rejected() {
        // By reading, extendedkey.go:317-341: Il = n - d for a public
        // parent d·G gives Il·G = -(d·G), which is not infinity, so the
        // check at :320 passes; AddNonConst then yields infinity, which
        // ToAffine maps to (0, 0), and SerializeCompressed gives 02 and 32
        // zero bytes; there is no check on the sum. Deriving further from
        // that key fails in ParsePubKey: x = 0 is not on the curve.
        let m = master();
        let mut il = (-scalar_of(&m)).to_bytes();
        let p = m.neuter();
        let k = p
            .derive_from_ilr(3, false, &ilr(il.as_mut()))
            .expect("no error");
        let mut infinity = [0u8; 33];
        infinity[0] = 0x02;
        assert!(!k.is_private());
        assert_eq!(k.serialized_pub_key(), &infinity[..]);
        assert!(k.string().starts_with("dpub"));
        let err = k.child(0).expect_err("x = 0 is not on the curve");
        assert_eq!(
            err.to_string(),
            "invalid public key: x coordinate \
             0000000000000000000000000000000000000000000000000000000000000000 is not on \
             the secp256k1 curve"
        );
        assert!(matches!(err, Error::PubKey(e) if e.kind() == PubKeyErrorKind::NotOnCurve));
    }

    #[test]
    fn infinity_serializes_as_go_does() {
        let mut want = [0u8; 33];
        want[0] = 0x02;
        assert_eq!(serialize_compressed(&ProjectivePoint::IDENTITY), want);
        assert_eq!(pub_key_of(&Scalar::ZERO), want);
    }

    #[test]
    fn depth_wraps_in_memory_and_serializes_mod_256() {
        // extendedkey.go:125, :347-348 (depth+1 on a uint16), :447
        // (depth % 256 on the wire).
        for (start, want_depth, want_byte) in
            [(u16::MAX, 0u16, 0u8), (255, 256, 0), (254, 255, 255)]
        {
            let mut m = master();
            m.depth = start;
            for parent in [m.clone(), m.neuter()] {
                let c = parent.child(1).expect("child");
                assert_eq!(c.depth(), want_depth, "from {start}");
                assert_eq!(payload(&c)[4], want_byte, "from {start}");
                assert_eq!(c.is_private(), parent.is_private());
            }
        }
    }

    #[test]
    fn stored_length_is_left_aligned_in_the_hardened_preimage() {
        // Two keys with the same scalar, stored as 31 and as 32 bytes, give
        // different hardened children but the same public key and string.
        let m = master();
        let mut a = m.clone();
        let mut b = m.clone();
        let mut full = [0u8; 32];
        full[1..].copy_from_slice(&[0x11; 31]);
        let pub_key = pub_key_of(&nonzero_scalar(&full).expect("valid"));
        a.secret.key = [0; 32];
        a.secret.key[..31].copy_from_slice(&[0x11; 31]);
        a.secret.len = 31;
        a.kind = Kind::Private { pub_key };
        b.secret.key = full;
        b.secret.len = 32;
        b.kind = Kind::Private { pub_key };
        let (ca, cb) = (a.child(HARDENED_KEY_START), b.child(HARDENED_KEY_START));
        assert_ne!(*ca.expect("child").string(), *cb.expect("child").string());
        assert_eq!(
            *a.child(0).expect("child").string(),
            *b.child(0).expect("child").string()
        );
        assert_eq!(*a.string(), *b.string());
        assert_eq!(
            *a.priv_key_bytes().expect("a"),
            *b.priv_key_bytes().expect("b")
        );
    }

    #[test]
    fn clone_and_neuter_survive_zero() {
        let mut m = master();
        let c = m.clone();
        let n = m.neuter();
        let before = (c.string(), n.string());
        m.zero();
        assert_eq!((c.string(), n.string()), before);
        assert_eq!(m.secret.key, [0; 32]);
        assert_eq!(m.secret.chain_code, [0; 32]);
    }
}
