// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2013-2016 The btcsuite developers
// Copyright (c) 2015-2022 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrutil's `wif.go`: private keys in the Wallet Import Format.
//!
//! A Decred WIF is **not** Bitcoin's. It is plain base58 (no version
//! prefix of base58check) of
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 2 | the network's `PrivateKeyID` (mainnet `22 de`, `Pm…`) |
//! | 2 | 1 | the signature type: 0 ECDSA secp256k1, 1 Ed25519, 2 Schnorr secp256k1 |
//! | 3 | 32 | the private key, as given to `NewWIF` |
//! | 35 | 4 | the first four bytes of **one** BLAKE-256 over the bytes before it |
//!
//! with no compression flag. (dcrd's comment on `DecodeWIF` calls the
//! checksum a double SHA-256; the code is a single BLAKE-256, QUIRKS
//! QK-0191.)

use std::fmt;

use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use dcroxide_chainhash::hash_b;
use dcroxide_dcrec::secp256k1::ReducedPrivateKey;
use zeroize::Zeroizing;

use crate::base58;

/// A signature scheme (Go's `dcrec.SignatureType`, an `int`).
///
/// Any value can be passed to [`Wif::new`], as in Go, which refuses all but
/// the three named ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignatureType(pub i64);

impl SignatureType {
    /// `STEcdsaSecp256k1` (0): ECDSA over secp256k1.
    pub const ECDSA_SECP256K1: SignatureType = SignatureType(0);
    /// `STEd25519` (1): Ed25519.
    pub const ED25519: SignatureType = SignatureType(1);
    /// `STSchnorrSecp256k1` (2): EC-Schnorr-DCRv0 over secp256k1.
    pub const SCHNORR_SECP256K1: SignatureType = SignatureType(2);
}

/// The decimal value, as Go's `%v` prints a `dcrec.SignatureType` (it has
/// no `String` method).
impl fmt::Display for SignatureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Why [`Wif::new`] or [`Wif::decode`] failed. `Display` is Go's error
/// text, byte for byte. No variant carries any part of the WIF string or
/// the private key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WifError {
    /// `ErrMalformedPrivateKey`, `malformed private key`: the string is
    /// longer than 54 bytes, is not base58 (dcrd's base58 decodes an
    /// invalid character to nothing, so a mistyped WIF reports this, never a
    /// character error), or does not decode to exactly 39 bytes.
    MalformedPrivateKey,
    /// `ErrChecksumMismatch`, `checksum mismatch`.
    ChecksumMismatch,
    /// `ErrWrongWIFNetwork`, `WIF is not for the network identified by
    /// 0x<hex>`. It names the **expected** network, the one passed to
    /// [`Wif::decode`], not the one in the string (spec 24, Q-16).
    WrongNetwork([u8; 2]),
    /// `unsupported signature type '<n>'`, from [`Wif::new`].
    UnsupportedSignatureType(SignatureType),
    /// `bad private scalar size`: an Ed25519 key that is not 32 bytes
    /// (dcrd's `edwards.PrivKeyFromScalar`), from [`Wif::new`].
    BadPrivateScalarSize,
    /// `not on subgroup (>N)`: an Ed25519 key, read as a big-endian
    /// integer, above the group order N. N itself is accepted, as in dcrd.
    ScalarAboveOrder,
    /// `zero or negative scalar`: an Ed25519 key of zero.
    ZeroScalar,
}

impl fmt::Display for WifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WifError::MalformedPrivateKey => f.write_str("malformed private key"),
            WifError::ChecksumMismatch => f.write_str("checksum mismatch"),
            // Go's %#04x of a [2]byte: "0x" and the hex of both bytes; the
            // width (4) is below the six characters written, so no padding.
            WifError::WrongNetwork(net) => write!(
                f,
                "WIF is not for the network identified by 0x{:02x}{:02x}",
                net[0], net[1]
            ),
            WifError::UnsupportedSignatureType(t) => {
                write!(f, "unsupported signature type '{t}'")
            }
            WifError::BadPrivateScalarSize => f.write_str("bad private scalar size"),
            WifError::ScalarAboveOrder => f.write_str("not on subgroup (>N)"),
            WifError::ZeroScalar => f.write_str("zero or negative scalar"),
        }
    }
}

impl std::error::Error for WifError {}

/// The Ed25519 group order N, big-endian: 2^252 +
/// 27742317777372353535851937790883648493.
const ED25519_ORDER_BE: [u8; 32] = [
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x14, 0xde, 0xf9, 0xde, 0xa2, 0xf7, 0x9c, 0xd6, 0x58, 0x12, 0x63, 0x1a, 0x5c, 0xf5, 0xd3, 0xed,
];

/// The public key dcrd's `edwards.PrivKeyFromScalar(p)` gives
/// (`dcrec/edwards/v2@v2.0.4/privkey.go:122-152`): `p` must be 32 bytes;
/// read as a **big-endian** integer D (unlike RFC 8032's little-endian
/// scalars) it must satisfy 0 < D <= N; the key is D·G in the standard
/// 32-byte encoding. D = N gives the identity, `01 00 … 00`, as dcrd's
/// big-integer multiplication does.
///
/// The range checks and the multiplication (curve25519-dalek's
/// constant-time base-point table) do not branch on D's value; which error
/// is returned, if any, is the one fact about D that timing shows. dcrd's
/// own computation is a variable-time double-and-add over a big integer.
fn ed25519_pub_key(p: &[u8]) -> Result<[u8; 32], WifError> {
    let be: &[u8; 32] = p.try_into().map_err(|_| WifError::BadPrivateScalarSize)?;
    // N - D over all 32 bytes: a final borrow means D > N.
    let mut borrow = 0u16;
    let mut any = 0u8;
    for (n, d) in ED25519_ORDER_BE.iter().zip(be.iter()).rev() {
        let diff = u16::from(*n)
            .wrapping_sub(u16::from(*d))
            .wrapping_sub(borrow);
        borrow = (diff >> 8) & 1;
        any |= *d;
    }
    if borrow != 0 {
        return Err(WifError::ScalarAboveOrder);
    }
    if any == 0 {
        return Err(WifError::ZeroScalar);
    }
    let mut le = Zeroizing::new(*be);
    le.reverse();
    // D <= N, so the reduction is D itself, or zero for D = N.
    let scalar = Zeroizing::new(Scalar::from_bytes_mod_order(*le));
    Ok(EdwardsPoint::mul_base(&scalar).compress().to_bytes())
}

/// The compressed secp256k1 public key dcrd gives `privKey`:
/// `secp256k1.PrivKeyFromBytes(privKey).PubKey().SerializeCompressed()`.
/// `PrivKeyFromBytes` never fails: it reads at most the first 32 bytes,
/// left-padded with zeros when shorter, reduces them modulo the group
/// order and accepts zero, whose public key, the point at infinity,
/// serializes as `02` and 32 zero bytes (spec 24, Q-5).
fn secp256k1_pub_key(priv_key: &[u8]) -> Vec<u8> {
    ReducedPrivateKey::from_bytes(priv_key)
        .public_point()
        .serialize_compressed()
        .to_vec()
}

/// A private key with its public key, signature type and network (Go's
/// `WIF`).
///
/// # Secrets
///
/// The private key bytes are held in a buffer that is wiped when the value
/// is dropped. `Debug` shows the signature type, network and public key,
/// and only the private key's length; there is no `Display`: Go's
/// `String()`, the WIF string itself, is the explicit method
/// [`Wif::string`], which returns a [`Zeroizing`] string. dcrwallet once
/// formatted a map of these with `%v` into an RPC error, quoting every key
/// (QUIRKS, spec 08); that cannot happen by accident here.
///
/// What is wiped: the private key bytes, the 39-byte (or, from
/// [`Wif::new`], `len + 7`-byte) payload and the base58 digits in both
/// directions, the Ed25519 scalar and its encodings, and what
/// `dcroxide-dcrec` wipes inside `ReducedPrivateKey` (its documentation
/// lists that). What is not: the BLAKE-256 state that computes the
/// checksum over the private key (`dcroxide-crypto` does not wipe it),
/// copies inside k256 and curve25519-dalek, stack copies the compiler
/// leaves when values move, and anything the caller copies out of
/// [`Wif::priv_key`] or out of the string's [`Zeroizing`].
///
/// # Differences from Go
///
/// Go's `PrivKey` hands out the WIF's own slice, and dcrwallet's
/// `ImportPrivateKey` zeroes it after use, destroying the caller's WIF
/// (QUIRKS QK-0040); here [`Wif::priv_key`] lends a read-only slice and the
/// key is wiped when the `Wif` is dropped. Everything a method returns is
/// otherwise Go's, including for keys Go does not reject.
#[derive(Clone)]
pub struct Wif {
    scheme: SignatureType,
    priv_key: Zeroizing<Vec<u8>>,
    pub_key: Vec<u8>,
    net_id: [u8; 2],
}

impl Wif {
    /// Go's `NewWIF(privKey, net, scheme)` (`wif.go:66-84`).
    ///
    /// - ECDSA and Schnorr secp256k1 accept **any** `priv_key`: the public
    ///   key is that of the first 32 bytes reduced modulo the group order
    ///   (zero and short keys included, see [`Wif::pub_key`]), and the
    ///   bytes are kept as given, so [`Wif::string`] encodes all of them
    ///   (spec 24, Q-27: a key that is not 32 bytes gives a string that
    ///   [`Wif::decode`] refuses). dcrwallet passes 32-byte keys.
    /// - Ed25519 requires 32 bytes and a big-endian value in 1..=N
    ///   ([`WifError::BadPrivateScalarSize`], [`WifError::ScalarAboveOrder`],
    ///   [`WifError::ZeroScalar`]).
    /// - Any other scheme is [`WifError::UnsupportedSignatureType`].
    pub fn new(priv_key: &[u8], net: [u8; 2], scheme: SignatureType) -> Result<Wif, WifError> {
        let pub_key = match scheme {
            SignatureType::ECDSA_SECP256K1 | SignatureType::SCHNORR_SECP256K1 => {
                secp256k1_pub_key(priv_key)
            }
            SignatureType::ED25519 => ed25519_pub_key(priv_key)?.to_vec(),
            other => return Err(WifError::UnsupportedSignatureType(other)),
        };
        Ok(Wif {
            scheme,
            priv_key: Zeroizing::new(priv_key.to_vec()),
            pub_key,
            net_id: net,
        })
    }

    /// Go's `DecodeWIF(wif, net)` (`wif.go:110-162`), in Go's order of
    /// checks:
    ///
    /// 1. longer than 54 bytes: [`WifError::MalformedPrivateKey`];
    /// 2. not base58, or not 39 bytes decoded: [`WifError::MalformedPrivateKey`];
    /// 3. checksum: [`WifError::ChecksumMismatch`];
    /// 4. network bytes other than `net`: [`WifError::WrongNetwork`]`(net)`;
    /// 5. by the signature type byte: 0 and 2 give a secp256k1 key (any 32
    ///    bytes, reduced as in [`Wif::new`], so zero and values at or above
    ///    the order decode), 1 an Ed25519 key (which can fail as in
    ///    [`Wif::new`]), and **any other byte succeeds** with an empty
    ///    private key, an empty public key and signature type 0 (spec 24,
    ///    Q-10; QUIRKS QK-0515). Callers that care must check
    ///    [`Wif::priv_key`]'s length or [`Wif::dsa`].
    ///
    /// Go's strings may hold any bytes; every byte that is not ASCII is an
    /// invalid base58 character there, so input a caller had to convert to
    /// UTF-8 lossily fails with [`WifError::MalformedPrivateKey`] just as
    /// the original bytes would in Go (or with the same error for being
    /// too long, if the conversion lengthened it past 54 bytes).
    pub fn decode(wif: &str, net: [u8; 2]) -> Result<Wif, WifError> {
        if wif.len() > base58::MAX_WIF_STRING_LEN {
            return Err(WifError::MalformedPrivateKey);
        }
        let decoded = base58::decode_wif(wif.as_bytes()).ok_or(WifError::MalformedPrivateKey)?;
        let (payload, checksum) = decoded.split_at(base58::WIF_LEN.saturating_sub(4));
        if hash_b(payload).get(..4) != Some(checksum) {
            return Err(WifError::ChecksumMismatch);
        }
        let net_id = [decoded[0], decoded[1]];
        if net_id != net {
            return Err(WifError::WrongNetwork(net));
        }
        let key = &payload[3..];
        let (scheme, pub_key) = match decoded[2] {
            0 => (SignatureType::ECDSA_SECP256K1, secp256k1_pub_key(key)),
            1 => (SignatureType::ED25519, ed25519_pub_key(key)?.to_vec()),
            2 => (SignatureType::SCHNORR_SECP256K1, secp256k1_pub_key(key)),
            _ => {
                return Ok(Wif {
                    scheme: SignatureType::ECDSA_SECP256K1,
                    priv_key: Zeroizing::new(Vec::new()),
                    pub_key: Vec::new(),
                    net_id,
                });
            }
        };
        Ok(Wif {
            scheme,
            priv_key: Zeroizing::new(key.to_vec()),
            pub_key,
            net_id,
        })
    }

    /// Go's `String()` (`wif.go:167-181`): the WIF string, base58 of the
    /// network bytes, the signature type's low byte, the private key as
    /// stored (not padded) and the 4-byte checksum. Returned in a string
    /// that is wiped when dropped.
    pub fn string(&self) -> Zeroizing<String> {
        let mut payload: Zeroizing<Vec<u8>> =
            Zeroizing::new(Vec::with_capacity(self.priv_key.len().saturating_add(7)));
        payload.extend_from_slice(&self.net_id);
        // Go's byte(w.scheme): the low byte (the scheme is 0, 1 or 2).
        payload.push(self.scheme.0 as u8);
        payload.extend_from_slice(&self.priv_key);
        let checksum = hash_b(&payload);
        payload.extend_from_slice(&checksum[..4]);
        base58::encode(&payload)
    }

    /// Go's `PrivKey()`: the private key bytes as given to [`Wif::new`] or
    /// decoded (32 bytes from [`Wif::decode`], none for an unknown
    /// signature type byte).
    pub fn priv_key(&self) -> &[u8] {
        &self.priv_key
    }

    /// Go's `PubKey()`: the compressed public key, 33 bytes for secp256k1
    /// (`02` and 32 zero bytes when the reduced private key is zero, the
    /// point at infinity), 32 for Ed25519, none for an unknown signature
    /// type byte.
    pub fn pub_key(&self) -> &[u8] {
        &self.pub_key
    }

    /// Go's `DSA()`: the signature type.
    pub fn dsa(&self) -> SignatureType {
        self.scheme
    }

    /// The network's `PrivateKeyID`, the WIF's first two bytes (Go keeps it
    /// unexported; it is the `net` passed to [`Wif::new`] or
    /// [`Wif::decode`]).
    pub fn net_id(&self) -> [u8; 2] {
        self.net_id
    }
}

impl fmt::Debug for Wif {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Hex<'a>(&'a [u8]);
        impl fmt::Debug for Hex<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
            }
        }
        f.debug_struct("Wif")
            .field("dsa", &self.scheme.0)
            .field("net_id", &Hex(&self.net_id))
            .field("pub_key", &Hex(&self.pub_key))
            .field(
                "priv_key",
                &format_args!("<{} bytes redacted>", self.priv_key.len()),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_private_key() {
        let key = [0x5au8; 32];
        let w = Wif::new(&key, [0x22, 0xde], SignatureType::ECDSA_SECP256K1).expect("wif");
        let dbg = format!("{w:?}");
        assert!(!dbg.contains("5a5a5a5a"), "{dbg}");
        assert!(dbg.contains("<32 bytes redacted>"), "{dbg}");
        assert!(!dbg.contains(w.string().as_str()), "{dbg}");
    }

    #[test]
    fn errors_do_not_echo_input() {
        let s = "PmQdMn8xafwaQouk8ngs1CccRCB1ZmsqQxBaxNR4vhQi5a5QB5717";
        let e = Wif::decode(s, [0x22, 0xde]).expect_err("bad checksum");
        assert!(!e.to_string().contains(s) && !format!("{e:?}").contains(s));
    }

    #[test]
    fn string_is_sized_exactly() {
        let w = Wif::new(&[7u8; 32], [0x23, 0x0e], SignatureType::ED25519).expect("wif");
        let s = w.string();
        assert_eq!(s.capacity(), s.len());
    }
}
