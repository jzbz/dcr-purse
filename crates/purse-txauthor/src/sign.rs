// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The btcsuite developers
// Copyright (c) 2016-2026 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The signing loop (Go `SecretsSource` and `AddAllInputScripts`,
//! `wallet/txauthor/author.go:183-234`; spec 10 §4.6) over dcroxide's port
//! of dcrd's `txscript/v4/sign`.

use std::cell::RefCell;

use dcroxide_chaincfg::Params;
use dcroxide_dcrec::secp256k1::ReducedPrivateKey;
use dcroxide_txscript::SIG_HASH_ALL;
use dcroxide_txscript::sign::{KeyDb, ScriptDb, SignError, SignatureType, sign_tx_output};
use dcroxide_txscript::stdaddr::{Address, AddressParamsV0};
use dcroxide_wire::MsgTx;
use purse_errors::AnyError;

use crate::author::AuthoredTx;

/// Private keys and redeem scripts for signing, looked up by the address
/// of the previous output script (Go `SecretsSource`: `sign.KeyDB`,
/// `sign.ScriptDB` and `ChainParams`).
///
/// The lookups return [`AnyError`], so an error a lookup returns reaches
/// the caller of [`add_all_input_scripts`] unchanged, kind included, as
/// Go's does.
pub trait SecretsSource {
    /// The network the addresses are encoded for.
    fn chain_params(&self) -> &Params;
    /// The private key for `addr`, its signature type, and whether its
    /// public key is serialized compressed (Go `GetKey`).
    ///
    /// A secp256k1 key (ECDSA or Schnorr) is read as Go's
    /// `PrivKeyFromBytes` reads it: any length, of which at most the first
    /// 32 bytes are taken, left-padded with zeros and reduced modulo the
    /// group order. One that reduces to zero is refused: for Schnorr with
    /// Go's error, `cannot sign tx input: private key is zero`; for ECDSA,
    /// which Go signs with, with `invalid privkey` (QK-0682). An Ed25519
    /// key is the 64-byte seed and public key, as in Go.
    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), AnyError>;
    /// The redeem script for the pay-to-script-hash address `addr` (Go
    /// `GetScript`).
    fn get_script(&mut self, addr: &Address) -> Result<Vec<u8>, AnyError>;
}

/// The version 0 address prefixes of a network, taken once per call as Go
/// takes `ChainParams()` once.
struct Prefixes([[u8; 2]; 5]);

impl Prefixes {
    fn of(p: &Params) -> Prefixes {
        Prefixes([
            p.addr_id_pub_key_v0(),
            p.addr_id_pub_key_hash_ecdsa_v0(),
            p.addr_id_pub_key_hash_ed25519_v0(),
            p.addr_id_pub_key_hash_schnorr_v0(),
            p.addr_id_script_hash_v0(),
        ])
    }
}

impl AddressParamsV0 for Prefixes {
    fn addr_id_pub_key_v0(&self) -> [u8; 2] {
        self.0[0]
    }
    fn addr_id_pub_key_hash_ecdsa_v0(&self) -> [u8; 2] {
        self.0[1]
    }
    fn addr_id_pub_key_hash_ed25519_v0(&self) -> [u8; 2] {
        self.0[2]
    }
    fn addr_id_pub_key_hash_schnorr_v0(&self) -> [u8; 2] {
        self.0[3]
    }
    fn addr_id_script_hash_v0(&self) -> [u8; 2] {
        self.0[4]
    }
}

/// The text of the error returned where Go's `sign` indexes the address of
/// a pay-to-pubkey script whose public key does not parse, and panics
/// (QK-0745): dcroxide's `sign_tx_output` returns it, before any lookup for
/// a previous script and after the redeem script's lookup for a redeem
/// script, where Go's panic falls (dcroxide QK-0034).
pub const INVALID_PUBKEY_SCRIPT: &str =
    "cannot sign a pay-to-pubkey script whose public key is not valid";

/// The text of the error returned where Go's signature-script merge
/// recurses without end and the process dies of a stack overflow: the new
/// signature script of a pay-to-script-hash input ends in a push of a
/// plain pay-to-script-hash script, and both it and the existing signature
/// script are nonempty and parse (QK-0746). dcroxide's `sign_tx_output`
/// returns it at the merge, after the same lookups as Go's (dcroxide
/// QK-0036).
pub const NESTED_SCRIPT_HASH_MERGE: &str = "cannot merge signature scripts of a \
     pay-to-script-hash input whose redeem script is itself pay-to-script-hash";

/// The text Go's `RawTxInSignature` returns for a Schnorr key that reduces
/// to zero (`cannot sign tx input: %w` around schnorr's
/// `ErrPrivateKeyIsZero`).
const SCHNORR_ZERO_KEY: &str = "cannot sign tx input: private key is zero";

/// The key `sign` uses for `key` as Go's `secp256k1.PrivKeyFromBytes` reads
/// it: at most the first 32 bytes, left-padded with zeros, reduced modulo
/// the group order. dcroxide's signer takes only a 32-byte key in [1, N),
/// so a nonzero reduced key is handed to it as its canonical 32 bytes,
/// which signs and serializes the public key exactly as Go does.
///
/// A key that reduces to zero is an error for Schnorr, with Go's text. For
/// ECDSA Go signs with it, for the point at infinity; that is not
/// reproduced (ADR-0012 Rule 7, QK-0682): the key is passed on unchanged and
/// dcroxide refuses it with `invalid privkey`.
fn go_secp_key(key: Vec<u8>, sig_type: SignatureType) -> Result<Vec<u8>, &'static str> {
    match ReducedPrivateKey::from_bytes(&key) {
        ReducedPrivateKey::Key(k) => Ok(k.serialize().to_vec()),
        ReducedPrivateKey::Zero => match sig_type {
            SignatureType::SchnorrSecp256k1 => Err(SCHNORR_ZERO_KEY),
            _ => Ok(key),
        },
    }
}

/// Bridges a [`SecretsSource`] to dcroxide's [`KeyDb`]/[`ScriptDb`]. A
/// lookup's own error travels through `sign_tx_output` inside
/// [`SignError::other`] and comes back unchanged ([`into_any_error`]), as
/// Go's `SignTxOutput` returns a lookup's error.
///
/// Go passes one value as both databases; dcroxide takes two `&mut`, so
/// both are `&Lookups` over one `RefCell`. dcroxide never calls one
/// lookup from inside another, so the borrows never overlap.
struct Lookups<'a, S: ?Sized>(RefCell<&'a mut S>);

impl<S: SecretsSource + ?Sized> KeyDb for &Lookups<'_, S> {
    fn get_key(&mut self, addr: &Address) -> Result<(Vec<u8>, SignatureType, bool), SignError> {
        let (key, sig_type, compressed) = self
            .0
            .borrow_mut()
            .get_key(addr)
            .map_err(SignError::other)?;
        let key = match sig_type {
            // Every use of the key that follows is Go's PrivKeyFromBytes
            // of it (RawTxInSignature and SignatureScript); where Go's
            // signing then fails, its error is returned here, which every
            // caller in sign treats as it treats that failure.
            SignatureType::EcdsaSecp256k1 | SignatureType::SchnorrSecp256k1 => {
                go_secp_key(key, sig_type).map_err(SignError::new)?
            }
            SignatureType::Ed25519 => key,
        };
        Ok((key, sig_type, compressed))
    }
}

impl<S: SecretsSource + ?Sized> ScriptDb for &Lookups<'_, S> {
    fn get_script(&mut self, addr: &Address) -> Result<Vec<u8>, SignError> {
        self.0
            .borrow_mut()
            .get_script(addr)
            .map_err(SignError::other)
    }
}

/// The error to return for dcroxide's `err`: a lookup's own error,
/// unchanged, or else a text error with the signer's text.
fn into_any_error(err: SignError) -> AnyError {
    let text = err.to_string();
    match err.into_inner() {
        // Only the lookups above wrap an error, and each wraps an AnyError;
        // anything else would keep its text.
        Some(inner) => inner
            .downcast::<AnyError>()
            .map_or_else(|_| purse_errors::new(text), |source_err| *source_err),
        None => purse_errors::new(text),
    }
}

/// Adds a signature script to every input of `tx` (Go
/// `AddAllInputScripts`), signing with `SigHashAll` the previous output
/// script `prev_pk_scripts[i]` of input `i`, keys and redeem scripts looked
/// up in `secrets`, with treasury rules enabled.
///
/// Each input's existing signature script is merged with the new one as
/// dcrd's `SignTxOutput` merges (spec 10 §4.6): outside multisig and
/// pay-to-script-hash the longer script is kept, the existing one on a
/// tie.
///
/// # Errors
///
/// - `tx.TxIn and prevPkScripts slices must have equal length` (a plain
///   error, as in Go) when the counts differ; nothing is signed.
/// - The first input's signing error, unchanged: a lookup's own error, or
///   the signer's text (`can't sign NULLDATA transactions`, `can't sign
///   unknown transactions`, ...). Inputs signed before it stay signed, as
///   in Go.
/// - [`INVALID_PUBKEY_SCRIPT`] where Go panics, for a pay-to-pubkey
///   previous or redeem script whose public key does not parse (ADR-0012
///   Rule 2(a); QK-0745).
/// - [`NESTED_SCRIPT_HASH_MERGE`] where Go's merge of the new signature
///   script into the existing one recurses until the process dies
///   (ADR-0012 Rule 2(a); QK-0746): the input spends a pay-to-script-hash
///   output (plain or stake-tagged) whose redeem script is itself
///   pay-to-script-hash (plain or stake-tagged), `secrets` knows both
///   scripts, the new signature script (the second script's opcodes then a
///   push of the first, each step dropped when it would exceed the script
///   limits) is nonempty, parses and ends in a push of a plain
///   pay-to-script-hash script, and the input's existing signature script
///   is nonempty and parses. It is returned after the second script lookup,
///   where Go goes on to recurse.
/// - `cannot sign tx input: private key is zero` for a Schnorr key that
///   reduces to zero, as in Go, and `invalid privkey` for an ECDSA key
///   that does, which Go signs with (ADR-0012 Rule 7; QK-0682); see
///   [`SecretsSource::get_key`].
pub fn add_all_input_scripts<S>(
    tx: &mut MsgTx,
    prev_pk_scripts: &[Vec<u8>],
    secrets: &mut S,
) -> Result<(), AnyError>
where
    S: SecretsSource + ?Sized,
{
    // Go reads ChainParams before the length check (spec 10 quirk 56).
    let params = Prefixes::of(secrets.chain_params());

    if tx.tx_in.len() != prev_pk_scripts.len() {
        return Err(purse_errors::new(
            "tx.TxIn and prevPkScripts slices must have equal length",
        ));
    }

    let lookups = Lookups(RefCell::new(secrets));
    for (i, pk_script) in prev_pk_scripts.iter().enumerate() {
        let (mut kdb, mut sdb) = (&lookups, &lookups);
        let script = sign_tx_output(
            &params,
            tx,
            i,
            pk_script,
            SIG_HASH_ALL,
            &mut kdb,
            &mut sdb,
            &tx.tx_in[i].signature_script,
            true,
        );
        match script {
            Ok(script) => tx.tx_in[i].signature_script = script,
            Err(err) => return Err(into_any_error(err)),
        }
    }
    Ok(())
}

impl AuthoredTx {
    /// Signs every input of the authored transaction with its previous
    /// scripts (Go `(*AuthoredTx).AddAllInputScripts`).
    ///
    /// # Errors
    ///
    /// As [`add_all_input_scripts`].
    pub fn add_all_input_scripts<S>(&mut self, secrets: &mut S) -> Result<(), AnyError>
    where
        S: SecretsSource + ?Sized,
    {
        add_all_input_scripts(&mut self.tx, &self.prev_scripts, secrets)
    }
}
