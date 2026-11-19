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
//! ## Timing
//!
//! The curve arithmetic is `dcroxide-dcrec`'s (`dcroxide_dcrec::secp256k1`).
//! What runs in constant time is the **arithmetic on a private key's
//! value**:
//!
//! - the range check of a master key, of a parsed private key and of every
//!   derivation's intermediate key Il, from private and public parents
//!   alike (`check_scalar`);
//! - the private child's sum `Il + parent mod n` (`add_private_tweak`);
//! - a private key's public key (`ReducedPrivateKey::public_point`, a
//!   constant-time multiplication by the generator).
//!
//! What timing does not hide:
//!
//! - **The stored length of a private key.** [`ExtendedKey::child`] strips
//!   a derived private key's leading zero bytes, a count that depends on
//!   the key's value, as Go does; and the stored length then sets the
//!   layout of a hardened child's HMAC preimage, the length of the slice
//!   that `add_private_tweak` and `ReducedPrivateKey::from_bytes` read
//!   (dcrec treats a slice's length as public), and the padding
//!   [`ExtendedKey::string`] and [`ExtendedKey::priv_key_bytes`] add. So
//!   timing can reveal how many leading zero bytes a stripped private key
//!   has, as in Go. Keys made by [`ExtendedKey::new_master`],
//!   [`ExtendedKey::new_key_from_string`] and
//!   [`ExtendedKey::child_bip32_std`] are always stored as 32 bytes.
//!   (Whether a private child's sum is zero, which no seed reaches in
//!   practice, is not hidden either.)
//! - **Il of a non-hardened public child.** A normal child of a *public*
//!   key is `Il·G + parent`, computed with dcrec's variable-time
//!   `add_public_tweak`, deliberately: dcrd computes this step in variable
//!   time too (`ScalarBaseMultNonConst` and `AddNonConst`,
//!   `extendedkey.go:317-341`). Il is HMAC-SHA512 over the parent's chain
//!   code, public key and index, so timing can in principle leak Il, a
//!   function of those three. For an extended public key someone was
//!   given, they are already that person's, who can compute Il anyway.
//!   But [`ExtendedKey::neuter`] of a private key keeps the private key's
//!   chain code, which this crate otherwise treats as secret (it is
//!   wiped), so deriving public children from a neutered key runs
//!   variable-time arithmetic on values derived from that chain code: the
//!   same values anyone holding the extended public key can compute, but
//!   exposed to a timing observer who does not hold it. A caller who needs
//!   constant-time derivation of non-hardened children derives them from
//!   the private key and neuters the child, `key.child(i)?.neuter()`
//!   rather than `key.neuter().child(i)?` (the same extended public key):
//!   that path runs only the constant-time arithmetic above (the stored
//!   length, as just said, aside).
//! - Anything outside that arithmetic: this crate makes no other
//!   constant-time claim. For example the base58 codec of an extended key
//!   string branches on its digits, as Go's does.
//!
//! ## Wiping
//!
//! What this crate wipes itself:
//!
//! - A key's private key bytes and chain code live in one heap allocation
//!   that every constructor fills in place and that is wiped when the key
//!   is dropped or [zeroed](ExtendedKey::zero), so moving a key leaves no
//!   copy of them behind. (The public key, version bytes, depth, child
//!   number and parent fingerprint are stored inline; they are public.)
//! - Every buffer this crate's own code puts a seed-derived secret in: the
//!   HMAC-SHA512 output (Il and Ir) and padded key, the HMAC preimage of a
//!   hardened child, the 82-byte serialization, and the base58 digits in
//!   both directions (the crate decodes and encodes extended keys itself,
//!   over fixed wiped buffers, rather than with `dcroxide-base58`, whose
//!   working vectors are freed unwiped).
//! - The HMAC and SHA-512 states, wiped on drop by the `zeroize` features
//!   of `hmac`, `sha2` and `digest`.
//!
//! What dcrec wipes, as its documentation at the pinned revision states:
//!
//! - In the range check (`check_scalar`, which is also the first step of
//!   `add_private_tweak`): its copy of the 32-byte encoding and the scalar
//!   it holds, both `Zeroizing`.
//! - In `add_private_tweak`: Il's scalar, the parent's padded encoding and
//!   reduced scalar, and the sum, all `Zeroizing`.
//! - In `ReducedPrivateKey::from_bytes`: the padded encoding, the reduced
//!   scalar and the encoding the key is built from, all `Zeroizing`.
//! - The keys it returns (`ReducedPrivateKey`, holding a `PrivateKey`, a
//!   k256 `SecretKey`), which wipe themselves on drop; the 32-byte
//!   encoding `serialize` returns (`Zeroizing`), from which this crate
//!   copies the child key into its heap allocation; and its copy of the
//!   scalar it multiplies by the generator for a public key.
//!
//! What is not wiped, by anyone:
//!
//! - In the range check, the `CtOption<Scalar>` that k256's
//!   `Scalar::from_repr` returns, which holds the parsed value and which
//!   nothing can wipe, and the copy of the encoding `from_repr` takes by
//!   value.
//! - k256's internal copies inside reduction, addition, the multiplication
//!   by the generator (its intermediate points and windows), building a
//!   `SecretKey`, and the intermediate encoding behind `serialize`.
//! - Stack copies the compiler leaves when values move, in this crate, in
//!   dcrec and in k256.
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
//! So a private key's bytes can remain in freed stack memory.
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
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use dcroxide_dcrec::secp256k1::{self, PublicKey, ReducedPrivateKey};
use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
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

    /// This crate's error for a dcroxide-dcrec parse error: the kind by
    /// variant, and the description dcrec's `Display`, which is dcrd's
    /// error text.
    fn from_dcrec(e: secp256k1::Error) -> PubKeyError {
        let kind = match e {
            secp256k1::Error::PubKeyInvalidLen(_) => PubKeyErrorKind::InvalidLen,
            secp256k1::Error::PubKeyInvalidFormat(_) => PubKeyErrorKind::InvalidFormat,
            secp256k1::Error::PubKeyXTooBig => PubKeyErrorKind::XTooBig,
            secp256k1::Error::PubKeyNotOnCurve { .. } => PubKeyErrorKind::NotOnCurve,
            // Unreachable here: dcrec returns these two only for a 65-byte
            // (uncompressed or hybrid) key, and the only bytes this crate
            // parses are a stored public key, 33 bytes, or a zeroed key's
            // empty one. They are mapped onto the nearest existing kinds,
            // so that the public kind keeps the four variants it has always
            // had, and the description still carries dcrd's exact text.
            // The unit test `parse_errors_have_only_the_four_kinds` covers
            // every length and every 33-byte format byte.
            secp256k1::Error::PubKeyYTooBig => PubKeyErrorKind::XTooBig,
            secp256k1::Error::PubKeyMismatchedOddness(_) => PubKeyErrorKind::InvalidFormat,
        };
        PubKeyError {
            kind,
            description: e.to_string(),
        }
    }
}

impl fmt::Display for PubKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl core::error::Error for PubKeyError {}

/// Parses a public key as `secp256k1.ParsePubKey` (v4.4.1) does, with its
/// error texts: dcroxide-dcrec's [`PublicKey::parse`].
fn parse_pub_key(b: &[u8]) -> Result<PublicKey, Error> {
    PublicKey::parse(b).map_err(|e| Error::PubKey(PubKeyError::from_dcrec(e)))
}

/// Go's `pubKeyBytes` for a private key,
/// `secp256k1.PrivKeyFromBytes(key).PubKey().SerializeCompressed()`: the
/// stored key at whatever length, reduced modulo n, and its public point
/// multiplied in constant time. A zero key gives the point at infinity,
/// which serializes as `02` followed by 32 zero bytes.
fn pub_key_of(stored: &[u8]) -> [u8; 33] {
    ReducedPrivateKey::from_bytes(stored)
        .public_point()
        .serialize_compressed()
}

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

        // Go's `overflow || IsZero` on the master key.
        secp256k1::check_scalar(&secret.key).map_err(|_| Error::UnusableSeed)?;
        let pub_key = pub_key_of(secret.stored());
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
            // Go's `overflow || IsZero` on the private key.
            secp256k1::check_scalar(&secret.key).map_err(|_| Error::UnusableSeed)?;
            let pub_key = pub_key_of(secret.stored());
            Kind::Private { pub_key }
        } else {
            // Go keeps the key data as given once it parses.
            parse_pub_key(key_data)?;
            let mut pub_key = [0u8; 33];
            pub_key.copy_from_slice(key_data);
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
    ///
    /// Timing: from a private key the arithmetic runs in constant time,
    /// though stored lengths (a stripped key's leading zero bytes, the
    /// parent's and the child's) are not hidden. From a public key, a normal child's `Il·G + parent`
    /// runs in variable time, as dcrd's does, so timing can in principle
    /// leak Il, which is HMAC-SHA512 over this key's chain code, public key
    /// and index: values the holder of this extended public key can
    /// compute, but for a [neutered](Self::neuter) private key derived from
    /// a chain code this crate otherwise treats as secret. To derive a
    /// non-hardened public child in constant time, derive it from the
    /// private key and neuter it. See the [crate documentation](crate#timing).
    pub fn child(&self, i: u32) -> Result<Self, Error> {
        self.derive(i, false)
    }

    /// As [`child`](Self::child), except that a derived private key keeps
    /// its leading zeros and is always stored as 32 bytes (Go's
    /// `ChildBIP32Std`). The preimage of this step is still laid out from
    /// this key's stored form, so it is strict BIP32 only from a parent
    /// that was not itself stored stripped.
    ///
    /// Timing is as for [`child`](Self::child): constant-time arithmetic
    /// from a private key (whose own stored length is not hidden), and a
    /// variable-time `Il·G + parent` for a normal child of a public key,
    /// neutered ones included. See the [crate documentation](crate#timing).
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
        // Go's `overflow || IsZero` on Il comes first, for private and
        // public parents alike, before the parent key is looked at.
        secp256k1::check_scalar(il).map_err(|_| Error::InvalidChild)?;

        let kind = match self.kind {
            Kind::Private { .. } => {
                // childKey = parse256(Il) + parentKey, in constant time.  Go
                // reduces the parent's stored bytes modulo n (SetByteSlice),
                // at whatever length they are stored; so does dcrec.
                let child = secp256k1::add_private_tweak(il, self.stored_priv_key())
                    .map_err(|_| Error::InvalidChild)?;
                let sum = child.serialize();

                // The Decred variant strips leading zero bytes; a zero sum
                // (not rejected, as in Go) strips to an empty key.
                let strip: u8 = if strict_bip32 {
                    0
                } else {
                    sum.iter()
                        .take_while(|&&b| b == 0)
                        .fold(0u8, |n, _| n.saturating_add(1))
                };
                let len = 32u8.saturating_sub(strip);
                secret.key[..usize::from(len)].copy_from_slice(&sum[usize::from(strip)..]);
                secret.len = len;
                drop(sum);
                // Go's pubKeyBytes of the stored key: the same value, whose
                // reduction dcrec has already made, so its public point is
                // taken from the child key directly (the point at infinity
                // for a zero sum).
                Kind::Private {
                    pub_key: child.public_point().serialize_compressed(),
                }
            }
            Kind::Public { .. } | Kind::Zeroed => {
                // childKey = serP(point(parse256(Il)) + parentKey).  The
                // parent is parsed after Il's check, as in Go; a zeroed key's
                // key bytes are empty and fail with the length error.  Go's
                // check that Il·G has no zero coordinate cannot fire for Il
                // in [1, n-1] (dcrec's `add_public_tweak` documents why).
                // Variable time in Il, as dcrd's ScalarBaseMultNonConst and
                // AddNonConst are (:317-341). Il is HMAC-SHA512 over this
                // key's chain code, public key and index: known to whoever
                // holds this extended public key, but for a neutered private
                // key derived from a chain code that is otherwise kept
                // secret (see the crate docs' "Timing").
                let parent = parse_pub_key(self.pub_key_bytes())?;
                // A sum at infinity is not rejected, as in Go: it serializes
                // as 02 followed by 32 zero bytes.
                let child =
                    secp256k1::add_public_tweak(il, &parent).map_err(|_| Error::InvalidChild)?;
                Kind::Public {
                    pub_key: child.serialize_compressed(),
                }
            }
        };

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
    ///
    /// The result keeps this key's chain code. Its normal children are
    /// derived with variable-time arithmetic on Il, HMAC-SHA512 over that
    /// chain code, the public key and the index, as dcrd derives them, so
    /// timing can in principle leak those Il values (which anyone holding
    /// the extended public key can compute). `key.child(i)?.neuter()` gives
    /// the same extended public key as `key.neuter().child(i)?` with
    /// constant-time arithmetic; see the [crate documentation](crate#timing).
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
    //!
    //! Every Il these tests hand `derive_from_ilr`, for public parents (on
    //! the variable-time `add_public_tweak` path) as for private ones, is
    //! test data built here, not a secret.

    #![allow(
        clippy::arithmetic_side_effects,
        reason = "test code: byte-wise negation modulo n and small index arithmetic"
    )]

    use super::*;

    /// The group order n, big-endian.
    const N: [u8; 32] = secp256k1::GROUP_ORDER_BYTES;

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

    /// `n - d` for a key's private scalar `d` in [1, n-1], big-endian, by
    /// schoolbook subtraction.
    fn neg_of(k: &ExtendedKey) -> [u8; 32] {
        let d = k.priv_key_bytes().expect("private");
        assert!(*d != [0; 32] && *d < N, "d in [1, n-1]");
        let mut out = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let diff = i16::from(N[i]) - i16::from(d[i]) - borrow;
            out[i] = diff.rem_euclid(256) as u8;
            borrow = i16::from(diff < 0);
        }
        assert_eq!(borrow, 0);
        out
    }

    #[test]
    fn invalid_il_is_rejected_at_the_call_site() {
        // extendedkey.go:277-280: overflow or zero Il gives ErrInvalidChild,
        // for private and public parents alike, before anything else. These
        // Il values are test data, not secrets.
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
    fn il_is_checked_before_the_parent_public_key_is_parsed() {
        // extendedkey.go:277-280 before :330-333: a parent whose public key
        // does not parse still reports ErrInvalidChild for an invalid Il,
        // and the parse error only for a valid one. These Il values are test
        // data, not secrets.
        let mut zeroed = master();
        zeroed.zero();
        let mut off_curve = master().neuter();
        let mut x0 = [0u8; 33];
        x0[0] = 0x02;
        off_curve.kind = Kind::Public { pub_key: x0 };
        let mut bad_format = master().neuter();
        let mut f04 = [0x11u8; 33];
        f04[0] = 0x04;
        bad_format.kind = Kind::Public { pub_key: f04 };
        let mut one = [0u8; 32];
        one[31] = 1;
        for (p, text) in [
            (&zeroed, "malformed public key: invalid length: 0"),
            (
                &off_curve,
                "invalid public key: x coordinate \
                 0000000000000000000000000000000000000000000000000000000000000000 is not on \
                 the secp256k1 curve",
            ),
            (&bad_format, "invalid public key: unsupported format: 4"),
        ] {
            for il in [[0u8; 32], N, [0xff; 32]] {
                let r = p.derive_from_ilr(0, false, &ilr(&il));
                assert_eq!(r.err(), Some(Error::InvalidChild));
            }
            let err = p
                .derive_from_ilr(0, false, &ilr(&one))
                .expect_err("parent does not parse");
            assert_eq!(err.to_string(), text);
            assert!(matches!(err, Error::PubKey(_)));
        }
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
        let il = neg_of(&m);
        let i = HARDENED_KEY_START.wrapping_add(7);
        let k = m.derive_from_ilr(i, false, &ilr(&il)).expect("no error");
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
        let k = m.derive_from_ilr(i, true, &ilr(&il)).expect("no error");
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
        // that key fails in ParsePubKey: x = 0 is not on the curve. This Il
        // (the negated test master key) is test data, not a secret.
        let m = master();
        let il = neg_of(&m);
        let p = m.neuter();
        let k = p.derive_from_ilr(3, false, &ilr(&il)).expect("no error");
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
        // Go's PrivKeyFromBytes(k).PubKey().SerializeCompressed() of a key
        // that is zero modulo n: empty (a zero sum stripped), 32 zero bytes
        // (the same sum under ChildBIP32Std) and n itself.
        assert_eq!(pub_key_of(&[]), want);
        assert_eq!(pub_key_of(&[0; 32]), want);
        assert_eq!(pub_key_of(&N), want);
        assert_eq!(
            secp256k1::PublicPoint::Infinity.serialize_compressed(),
            want
        );
    }

    #[test]
    fn parse_errors_have_only_the_four_kinds() {
        // The parse helper's errors, for every length but 33 and for every
        // 33-byte format byte, come from the four dcrec variants that have
        // a kind of their own; only a 65-byte key reaches the other two,
        // and this crate never parses one (a stored public key is 33 bytes,
        // a zeroed key's is empty). Each error's description is dcrec's
        // text, which is dcrd's.
        fn kind_of(b: &[u8]) -> Option<PubKeyErrorKind> {
            match parse_pub_key(b) {
                Ok(_) => None,
                Err(Error::PubKey(e)) => {
                    let dcrec = PublicKey::parse(b).expect_err("the same input fails");
                    assert_eq!(e.description(), dcrec.to_string());
                    assert_eq!(e.to_string(), dcrec.to_string());
                    if b.len() != secp256k1::PUB_KEY_BYTES_LEN_UNCOMPRESSED {
                        assert!(
                            !matches!(
                                dcrec,
                                secp256k1::Error::PubKeyYTooBig
                                    | secp256k1::Error::PubKeyMismatchedOddness(_)
                            ),
                            "{} bytes reached a 65-byte kind: {dcrec}",
                            b.len()
                        );
                    }
                    Some(e.kind())
                }
                Err(other) => panic!("not a public key error: {other:?}"),
            }
        }

        // Every length but 33 (and 65, below) is the length error.
        for len in (0..=200usize).filter(|&l| l != 33 && l != 65) {
            for fill in [0x00, 0x02, 0x03, 0x04, 0xff] {
                let b = vec![fill; len];
                assert_eq!(kind_of(&b), Some(PubKeyErrorKind::InvalidLen), "{len}");
                assert_eq!(
                    parse_pub_key(&b).err().map(|e| e.to_string()),
                    Some(alloc::format!(
                        "malformed public key: invalid length: {len}"
                    ))
                );
            }
        }

        // Every 33-byte format byte, against x values on and off the curve
        // and at and above the field prime.
        let g = match ReducedPrivateKey::from_bytes(&[1]).public_point() {
            secp256k1::PublicPoint::Key(k) => k.serialize_uncompressed(),
            secp256k1::PublicPoint::Infinity => panic!("1·G is not infinity"),
        };
        let gx: [u8; 32] = g[1..33].try_into().expect("32 bytes");
        let gy: [u8; 32] = g[33..].try_into().expect("32 bytes");
        let p = secp256k1::FIELD_PRIME_BYTES;
        let mut p_minus_1 = p;
        p_minus_1[31] -= 1;
        let xs = [[0u8; 32], gx, p_minus_1, p, [0xff; 32]];
        for format in 0..=u8::MAX {
            for x in xs {
                let mut b = [0u8; 33];
                b[0] = format;
                b[1..].copy_from_slice(&x);
                let kind = kind_of(&b);
                if format != 0x02 && format != 0x03 {
                    assert_eq!(kind, Some(PubKeyErrorKind::InvalidFormat));
                } else if x >= p {
                    assert_eq!(kind, Some(PubKeyErrorKind::XTooBig));
                } else if x == gx {
                    assert_eq!(kind, None);
                } else {
                    assert!(
                        matches!(kind, None | Some(PubKeyErrorKind::NotOnCurve)),
                        "{kind:?}"
                    );
                }
            }
            // x = 0 is on no point.
            let mut b = [0u8; 33];
            b[0] = format;
            if format == 0x02 || format == 0x03 {
                assert_eq!(kind_of(&b), Some(PubKeyErrorKind::NotOnCurve));
            }
        }

        // 65 bytes, which this crate never parses, is where dcrec's other
        // two kinds come from; the private conversion maps them onto
        // existing kinds and keeps dcrd's text.
        let mut y_too_big = [0u8; 65];
        y_too_big[0] = secp256k1::PUB_KEY_FORMAT_UNCOMPRESSED;
        y_too_big[1..33].copy_from_slice(&gx);
        y_too_big[33..].copy_from_slice(&[0xff; 32]);
        let err = parse_pub_key(&y_too_big).expect_err("y >= p");
        assert_eq!(err.to_string(), "invalid public key: y >= field prime");
        assert_eq!(kind_of(&y_too_big), Some(PubKeyErrorKind::XTooBig));
        // G's y is even, so the odd hybrid format byte mismatches it.
        assert_eq!(gy[31] & 1, 0);
        let mut oddness = [0u8; 65];
        oddness[0] = secp256k1::PUB_KEY_FORMAT_HYBRID_ODD;
        oddness[1..33].copy_from_slice(&gx);
        oddness[33..].copy_from_slice(&gy);
        let err = parse_pub_key(&oddness).expect_err("mismatched oddness");
        assert_eq!(
            err.to_string(),
            "invalid public key: y oddness does not match specified value of true"
        );
        assert_eq!(kind_of(&oddness), Some(PubKeyErrorKind::InvalidFormat));
        for format in 0..=u8::MAX {
            for fill in [0x00, 0xff] {
                let mut b = [fill; 65];
                b[0] = format;
                let _ = kind_of(&b);
            }
        }
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
        let pub_key = pub_key_of(&full);
        // The stored length does not change the public key: 31 bytes are
        // left-padded, as Go's PrivKeyFromBytes pads them.
        assert_eq!(pub_key_of(&[0x11; 31]), pub_key);
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
