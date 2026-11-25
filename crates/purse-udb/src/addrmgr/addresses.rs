// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Addresses: the managed addresses of `wallet/udb/address.go`, and the
//! address methods of `udb.Manager` (`wallet/udb/addressmanager.go`):
//! lookups, private keys and redeem scripts, imports into the reserved
//! imported account, recording of derived addresses, and the used and
//! returned child indexes (spec 02 sections 5.16 to 5.19).
//!
//! An address row is keyed by the address's HASH160 in `addr_by_hash`,
//! where dcrwallet keys it by SHA-256 of the HASH160; every insert also
//! writes `addr_by_acct_sha` (account LE ‖ SHA-256 of the HASH160), which is
//! dcrwallet's per-account `addracctidx/<account>/` set and gives
//! `ForEachAccountAddress` its order. As in dcrwallet, an insert that
//! replaces a row of another account leaves that account's entry in place
//! (QK-0757).

use std::borrow::Cow;

use dcroxide_txscript::stdaddr::{self, Address, hash160};
use purse_errors::{AnyError, Kind, e};
use purse_snacl::CryptoKey;
use purse_store::{AddrAcctShaKey, AddrByAcctSha, AddrByHash, AddrRow, Hash160, WriteTx};
use sha2::{Digest, Sha256};

use super::hd::hd_err;
use super::rows::{ImportedData, StoreRead, bad_imported, store_err};
use super::{
    AccountInfo, ChainParams, ExtendedKey, IMPORTED_ADDR_ACCOUNT, INTERNAL_BRANCH, Inner,
    MAX_ADDRESSES_PER_ACCOUNT, Manager, ReducedPrivateKey, Result, Wif, crypto_msg,
    fetch_account_row, render_snacl,
};

/// A managed address: Go's `ManagedAddress` with its two concrete types,
/// `managedAddress` (`ManagedPubKeyAddress`) and `scriptAddress`
/// (`ManagedScriptAddress`) (`wallet/udb/address.go`).
///
/// Go's managed addresses alias the caller's memory (spec 02 section 10
/// item 37); these own their bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedAddress {
    /// A public key's P2PKH (ECDSA secp256k1) address: a derived address,
    /// or an imported key.
    PubKey {
        /// The account.
        account: u32,
        /// The P2PKH address of the key's HASH160.
        address: Address,
        /// The public key: the 33-byte compressed key of a derived address,
        /// the stored bytes of an imported one.
        pub_key: Vec<u8>,
        /// The child index (0 for an imported key).
        index: u32,
        /// Whether the key was imported.
        imported: bool,
        /// Whether the address is on the internal branch.
        internal: bool,
    },
    /// An imported script's P2SH address.
    Script {
        /// The account (always the reserved imported account).
        account: u32,
        /// The P2SH address of the script's HASH160.
        address: Address,
        /// The redeem script.
        script: Vec<u8>,
    },
}

impl ManagedAddress {
    /// Go's `Account`.
    pub fn account(&self) -> u32 {
        match self {
            ManagedAddress::PubKey { account, .. } | ManagedAddress::Script { account, .. } => {
                *account
            }
        }
    }

    /// Go's `Address`.
    pub fn address(&self) -> &Address {
        match self {
            ManagedAddress::PubKey { address, .. } | ManagedAddress::Script { address, .. } => {
                address
            }
        }
    }

    /// Go's `AddrHash`: the HASH160 of the key or script.
    pub fn addr_hash(&self) -> [u8; 20] {
        // Both kinds are built from a HASH160, so the address has one.
        self.address().hash160().copied().unwrap_or_default()
    }

    /// Go's `Imported`: true for imported keys and every script.
    pub fn imported(&self) -> bool {
        match self {
            ManagedAddress::PubKey { imported, .. } => *imported,
            ManagedAddress::Script { .. } => true,
        }
    }

    /// Go's `Internal`.
    pub fn internal(&self) -> bool {
        match self {
            ManagedAddress::PubKey { internal, .. } => *internal,
            ManagedAddress::Script { .. } => false,
        }
    }

    /// Go's `Multisig`: false for every address udb creates.
    pub fn multisig(&self) -> bool {
        false
    }

    /// Go's `ManagedPubKeyAddress.PubKey`, for a public key address.
    pub fn pub_key(&self) -> Option<&[u8]> {
        match self {
            ManagedAddress::PubKey { pub_key, .. } => Some(pub_key),
            ManagedAddress::Script { .. } => None,
        }
    }

    /// Go's `ManagedPubKeyAddress.Index`, for a public key address.
    pub fn index(&self) -> Option<u32> {
        match self {
            ManagedAddress::PubKey { index, .. } => Some(*index),
            ManagedAddress::Script { .. } => None,
        }
    }

    /// Go's `ManagedScriptAddress.RedeemScript`: script version 0 and the
    /// script, for a script address.
    pub fn redeem_script(&self) -> Option<(u16, &[u8])> {
        match self {
            ManagedAddress::Script { script, .. } => Some((0, script)),
            ManagedAddress::PubKey { .. } => None,
        }
    }
}

/// Go's `normalizeAddress`: a pay-to-pubkey address becomes its
/// pay-to-pubkey-hash address; every other address is returned unchanged.
pub(crate) fn normalize_address(addr: &Address) -> Cow<'_, Address> {
    match addr.address_pub_key_hash() {
        Some(a) => Cow::Owned(a),
        None => Cow::Borrowed(addr),
    }
}

/// Go's `addressID`: the address's HASH160.
///
/// # Errors
///
/// An address without one: `Invalid` `address id cannot be created from
/// type <Go type> (requires Hash160 method)`. Every version 0 address has
/// one once normalized, so no caller of this crate reaches it.
fn address_id(addr: &Address) -> Result<[u8; 20]> {
    addr.hash160().copied().ok_or_else(|| {
        e!(
            Kind::Invalid,
            format!(
                "address id cannot be created from type {} (requires Hash160 method)",
                addr.go_type_name()
            )
        )
        .into()
    })
}

/// The P2PKH (ECDSA secp256k1) address of a HASH160.
fn p2pkh(chain: &ChainParams, hash: [u8; 20]) -> Address {
    Address::PubKeyHashEcdsaSecp256k1V0 {
        net_id: chain.ids.pub_key_hash_ecdsa,
        hash,
    }
}

/// Go's `newManagedAddressWithoutPrivKey`: the P2PKH address of
/// `HASH160(pub_key)`.
fn managed_pub_key(chain: &ChainParams, account: u32, pub_key: Vec<u8>) -> ManagedAddress {
    ManagedAddress::PubKey {
        account,
        address: p2pkh(chain, hash160(&pub_key)),
        pub_key,
        index: 0,
        imported: false,
        internal: false,
    }
}

/// Go's `newScriptAddress`.
///
/// # Errors
///
/// A hash that is not 20 bytes: stdaddr's `ErrInvalidHashLen`, unwrapped.
fn managed_script(
    chain: &ChainParams,
    account: u32,
    hash: &[u8],
    script: Vec<u8>,
) -> Result<ManagedAddress> {
    let address =
        stdaddr::new_address_script_hash_v0_from_hash(hash, chain).map_err(AnyError::foreign)?;
    Ok(ManagedAddress::Script {
        account,
        address,
        script,
    })
}

/// Go's `fetchAddress`: the row of the address whose HASH160 is `id`.
///
/// # Errors
///
/// No row: `NotExist` `no address with id <id hex>`; the store's.
fn fetch_address<S: StoreRead + ?Sized>(tx: &mut S, id: &[u8]) -> Result<AddrRow> {
    let row = match <[u8; 20]>::try_from(id) {
        Ok(h) => tx.get::<AddrByHash>(&Hash160(h)).map_err(store_err)?,
        // Every row is keyed by a 20-byte HASH160, so a key of another
        // length names none, as in Go, where its SHA-256 matches no row.
        Err(_) => None,
    };
    row.ok_or_else(|| e!(Kind::NotExist, format!("no address with id {}", hex(id))).into())
}

/// The redeem script of an address row (the tail of Go's
/// `redeemScriptForHash160`, `wallet/udb/addressmanager.go:2559-2574`).
///
/// # Errors
///
/// A derived address or an imported key: `invalid operation: redeem script
/// lookup requires P2SH address`; an imported value that does not decode:
/// `IO`.
fn script_of_row(row: AddrRow) -> Result<Vec<u8>> {
    let lookup =
        || -> AnyError { e!(Kind::Invalid, "redeem script lookup requires P2SH address").into() };
    match row {
        AddrRow::Imported { data, .. } => {
            match ImportedData::decode(&data).map_err(bad_imported)? {
                ImportedData::Script { script, .. } => Ok(script),
                ImportedData::Key { .. } => Err(lookup()),
            }
        }
        AddrRow::Derived { .. } => Err(lookup()),
    }
}

/// Go's `redeemScriptForHash160` (`wallet/udb/addressmanager.go:2559-2574`)
/// over the `addr_by_hash` row of `hash160`, already read (the transaction
/// store's `AddMultisigOut` and `UnspentMultisigCreditsForAddress` read it
/// through their own reader): the redeem script of an imported script.
///
/// # Errors
///
/// No row: `item does not exist: no address with id <HASH160 hex>`; as
/// `script_of_row` otherwise.
pub(crate) fn redeem_script_for_hash160(row: Option<AddrRow>, hash160: &[u8]) -> Result<Vec<u8>> {
    let row = row.ok_or_else(|| -> AnyError {
        e!(
            Kind::NotExist,
            format!("no address with id {}", hex(hash160))
        )
        .into()
    })?;
    script_of_row(row)
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len().saturating_mul(2));
    for byte in b {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Go's `putAddress` (`wallet/udb/addressdb.go:1337-1350`): writes the row
/// and adds it to its account's set. An entry another account's set holds
/// for the same HASH160 is left in place, as Go leaves it (QK-0757).
fn put_address(tx: &mut WriteTx<'_>, h: [u8; 20], row: &AddrRow) -> Result<()> {
    let h = Hash160(h);
    tx.put::<AddrByHash>(&h, row).map_err(store_err)?;
    tx.put::<AddrByAcctSha>(&AddrAcctShaKey::for_address(row.account(), &h), &h)
        .map_err(store_err)
}

/// Whether an address row exists for `h` (Go's `existsAddress`).
pub(super) fn exists<S: StoreRead + ?Sized>(tx: &mut S, h: &[u8; 20]) -> Result<bool> {
    Ok(tx
        .get::<AddrByHash>(&Hash160(*h))
        .map_err(store_err)?
        .is_some())
}

/// Go's `deriveKey` (`wallet/udb/addressmanager.go:438-468`).
fn derive_key<S: StoreRead + ?Sized>(
    tx: &mut S,
    info: &AccountInfo,
    account: u32,
    branch: u32,
    index: u32,
    private: bool,
) -> Result<ExtendedKey> {
    let key = if private {
        match &info.key_priv {
            Some(k) => k,
            None => {
                if info.unique_key.is_some() {
                    return Err(e!(Kind::Locked, "account with unique passphrase is locked").into());
                }
                let name = fetch_account_row(tx, account)?.name;
                let name = String::from_utf8_lossy(&name);
                if !info.key_encrypted.is_empty() {
                    // Go's errors.E(errors.Locked, "private key %s/%d/%d is
                    // locked", name, branch, index) is no format call: the
                    // name, the last string argument, is the message
                    // (QK-0030).
                    return Err(e!(Kind::Locked, name.into_owned()).into());
                }
                return Err(AnyError::new(format!(
                    "no private key for {name}/{branch}/{index}"
                )));
            }
        }
    } else {
        &info.key_pub
    };
    key.child(branch)
        .and_then(|b| b.child(index))
        .map_err(hd_err)
}

impl Inner {
    /// Go's `deriveKeyFromPath`.
    fn derive_key_from_path<S: StoreRead + ?Sized>(
        &mut self,
        tx: &mut S,
        chain: &ChainParams,
        account: u32,
        branch: u32,
        index: u32,
        private: bool,
    ) -> Result<ExtendedKey> {
        if private && account == IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::Invalid, "account does not record private keys").into());
        }
        let info = self.load_account_info(tx, chain, account)?;
        derive_key(tx, &info, account, branch, index, private)
    }

    /// Go's `rowInterfaceToManaged` and the three conversions it selects.
    fn row_to_managed<S: StoreRead + ?Sized>(
        &mut self,
        tx: &mut S,
        chain: &ChainParams,
        row: &AddrRow,
    ) -> Result<ManagedAddress> {
        match row {
            AddrRow::Derived {
                account,
                branch,
                index,
            } => self.chain_row_to_managed(tx, chain, *account, u32::from(*branch), *index),
            AddrRow::Imported { account, data } => {
                match ImportedData::decode(data).map_err(bad_imported)? {
                    ImportedData::Key { enc_pub, .. } => {
                        let plain = self.crypto_key_pub.decrypt(&enc_pub).map_err(|err| {
                            crypto_msg(format!("decrypt imported pubkey: {}", render_snacl(&err)))
                        })?;
                        let mut m = managed_pub_key(chain, *account, plain.as_bytes().to_vec());
                        if let ManagedAddress::PubKey { imported, .. } = &mut m {
                            *imported = true;
                        }
                        Ok(m)
                    }
                    ImportedData::Script { enc_hash, script } => {
                        let hash = self.crypto_key_pub.decrypt(&enc_hash).map_err(|err| {
                            crypto_msg(format!(
                                "decrypt imported P2SH address: {}",
                                render_snacl(&err)
                            ))
                        })?;
                        managed_script(chain, *account, hash.as_bytes(), script)
                    }
                }
            }
        }
    }

    /// Go's `chainAddressRowToManaged`: derived privately while the wallet
    /// (or the account with its own passphrase) is unlocked, publicly
    /// otherwise, as Go does although only the public key is kept (spec 02
    /// section 10 item 18): the two differ in which errors can occur.
    fn chain_row_to_managed<S: StoreRead + ?Sized>(
        &mut self,
        tx: &mut S,
        chain: &ChainParams,
        account: u32,
        branch: u32,
        index: u32,
    ) -> Result<ManagedAddress> {
        let mut private = !self.locked;
        if account > IMPORTED_ADDR_ACCOUNT {
            private = false;
        } else {
            let (set, unlocked) = Manager::has_passphrase(self, tx, chain, account);
            if set {
                private = unlocked;
            }
        }
        let key = self.derive_key_from_path(tx, chain, account, branch, index, private)?;
        Ok(ManagedAddress::PubKey {
            account,
            address: p2pkh(chain, hash160(key.serialized_pub_key())),
            pub_key: key.serialized_pub_key().to_vec(),
            index,
            imported: false,
            internal: branch == INTERNAL_BRANCH,
        })
    }

    /// Seals `plain` with the public crypto key.
    fn seal_pub(&mut self, plain: &[u8], what: &str) -> Result<Vec<u8>> {
        let Inner {
            crypto_key_pub,
            rng,
            ..
        } = self;
        crypto_key_pub
            .encrypt_with(plain, &mut **rng)
            .map_err(|err| crypto_msg(format!("{what}: {}", render_snacl(&err))))
    }

    /// Seals `plain` with the private crypto key, which is present only
    /// while unlocked.
    pub(super) fn seal_priv(&mut self, plain: &[u8], what: &str) -> Result<Vec<u8>> {
        let Inner {
            crypto_key_priv,
            rng,
            ..
        } = self;
        let key = crypto_key_priv
            .as_ref()
            .ok_or_else(|| AnyError::from(e!(Kind::Locked)))?;
        key.encrypt_with(plain, &mut **rng)
            .map_err(|err| crypto_msg(format!("{what}: {}", render_snacl(&err))))
    }

    /// Seals `plain` with the public crypto key (for the account methods).
    pub(super) fn seal_public(&mut self, plain: &[u8], what: &str) -> Result<Vec<u8>> {
        self.seal_pub(plain, what)
    }
}

impl Manager {
    /// Go's `Address` (`wallet/udb/addressmanager.go:1007-1013`): the
    /// managed address of a recorded address. A pay-to-pubkey address is
    /// looked up as its pay-to-pubkey-hash address.
    ///
    /// A derived address's key is derived again (privately while unlocked,
    /// as Go does), loading its account.
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address <normalized address>`; for
    /// a derived row, loading the account (an unknown account: `item does
    /// not exist: no account <n>`), the derivation's errors, and for the
    /// reserved imported account while unlocked `invalid operation: account
    /// does not record private keys`; for an imported key or script, a
    /// sealed value that does not open (`Crypto` `decrypt imported pubkey:
    /// ...`, `decrypt imported P2SH address: ...`); the store's.
    pub fn address<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        addr: &Address,
    ) -> Result<ManagedAddress> {
        let addr = normalize_address(addr);
        let mut inner = self.write();
        let id = address_id(&addr)?;
        let row = match fetch_address(tx, &id) {
            Ok(r) => r,
            Err(err) if err.is_kind(Kind::NotExist) => {
                return Err(e!(Kind::NotExist, format!("no address {}", addr.encode())).into());
            }
            Err(err) => return Err(err),
        };
        inner.row_to_managed(tx, &self.chain, &row)
    }

    /// Go's `AddrAccount` (`wallet/udb/addressmanager.go:1016-1030`): the
    /// account an address is recorded under.
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address <address>`, the address as
    /// passed in (a pay-to-pubkey address in its pubkey form, where
    /// [`Manager::address`] prints the normalized form; QK-0758); the
    /// store's.
    pub fn addr_account<S: StoreRead + ?Sized>(&self, tx: &mut S, addr: &Address) -> Result<u32> {
        let id = address_id(&normalize_address(addr))?;
        match fetch_address(tx, &id) {
            Ok(r) => Ok(r.account()),
            Err(err) if err.is_kind(Kind::NotExist) => {
                Err(e!(Kind::NotExist, format!("no address {}", addr.encode())).into())
            }
            Err(err) => Err(err),
        }
    }

    /// Go's `ExistsHash160`: whether an address with this HASH160 is
    /// recorded. Bytes that are not 20 long name no address.
    ///
    /// # Errors
    ///
    /// The store's (Go's bucket read cannot fail).
    pub fn exists_hash160<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        hash160: &[u8],
    ) -> Result<bool> {
        match <[u8; 20]>::try_from(hash160) {
            Ok(h) => exists(tx, &h),
            Err(_) => Ok(false),
        }
    }

    /// Go's `ImportPrivateKey` (`wallet/udb/addressmanager.go:1243-1292`):
    /// records the WIF's public key, and its private key unless the wallet
    /// is watching-only, under the reserved imported account.
    ///
    /// The signature suite is not examined: whatever the WIF's public key
    /// is, its HASH160 names the address, which is always a P2PKH ECDSA
    /// address (spec 02 section 5.18). Go zeroes the caller's WIF after
    /// sealing it (QK-0040); the WIF is borrowed here and left as it is.
    ///
    /// # Errors
    ///
    /// A locked wallet that is not watching-only: the bare `wallet locked`;
    /// a recorded address: `item already exists: address for private key
    /// already exists`; the store's.
    pub fn import_private_key(&self, tx: &mut WriteTx<'_>, wif: &Wif) -> Result<ManagedAddress> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if !self.is_watching_only() && inner.locked {
            return Err(e!(Kind::Locked).into());
        }
        let pub_key = wif.pub_key();
        let h = hash160(pub_key);
        if exists(tx, &h)? {
            return Err(e!(Kind::Exist, "address for private key already exists").into());
        }
        let enc_pub = inner.seal_pub(pub_key, "encrypt imported pubkey")?;
        let enc_priv = if self.is_watching_only() {
            Vec::new()
        } else {
            inner.seal_priv(wif.priv_key(), "encrypt imported privkey")?
        };
        put_address(
            tx,
            h,
            &AddrRow::Imported {
                account: IMPORTED_ADDR_ACCOUNT,
                data: ImportedData::Key { enc_pub, enc_priv }.encode(),
            },
        )?;
        let mut m = managed_pub_key(&self.chain, IMPORTED_ADDR_ACCOUNT, pub_key.to_vec());
        if let ManagedAddress::PubKey { imported, .. } = &mut m {
            *imported = true;
        }
        Ok(m)
    }

    /// Go's `ImportPublicKey` (`wallet/udb/addressmanager.go:1297-1348`):
    /// records a compressed secp256k1 public key (the point is not
    /// checked) under the reserved imported account. Only a watching-only
    /// wallet accepts one.
    ///
    /// # Errors
    ///
    /// In Go's order: not watching-only, `invalid operation: public keys may
    /// only be imported by watching-only wallets`; not 33 bytes, `invalid
    /// encoding: invalid length for compressed pubkey`; a first byte other
    /// than 2 or 3, `invalid encoding: invalid format byte for compressed
    /// pubkey`; a recorded address, `item already exists: address for
    /// public key already exists`; the store's.
    pub fn import_public_key(
        &self,
        tx: &mut WriteTx<'_>,
        pub_key: &[u8],
    ) -> Result<ManagedAddress> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if !self.is_watching_only() {
            return Err(e!(
                Kind::Invalid,
                "public keys may only be imported by watching-only wallets"
            )
            .into());
        }
        if pub_key.len() != 33 {
            return Err(e!(Kind::Encoding, "invalid length for compressed pubkey").into());
        }
        if pub_key[0] != 2 && pub_key[0] != 3 {
            return Err(e!(Kind::Encoding, "invalid format byte for compressed pubkey").into());
        }
        let h = hash160(pub_key);
        if exists(tx, &h)? {
            return Err(e!(Kind::Exist, "address for public key already exists").into());
        }
        let enc_pub = inner.seal_pub(pub_key, "encrypt imported pubkey")?;
        put_address(
            tx,
            h,
            &AddrRow::Imported {
                account: IMPORTED_ADDR_ACCOUNT,
                data: ImportedData::Key {
                    enc_pub,
                    enc_priv: Vec::new(),
                }
                .encode(),
            },
        )?;
        let mut m = managed_pub_key(&self.chain, IMPORTED_ADDR_ACCOUNT, pub_key.to_vec());
        if let ManagedAddress::PubKey { imported, .. } = &mut m {
            *imported = true;
        }
        Ok(m)
    }

    /// Go's `ImportScript` (`wallet/udb/addressmanager.go:1355-1382`):
    /// records a script (any bytes; it is not checked) as a P2SH address
    /// under the reserved imported account. The wallet may be locked.
    ///
    /// # Errors
    ///
    /// A recorded address with the script's HASH160: `item already exists:
    /// script already exists`; the store's.
    pub fn import_script(&self, tx: &mut WriteTx<'_>, script: &[u8]) -> Result<ManagedAddress> {
        let mut guard = self.write();
        let inner = &mut *guard;
        let h = hash160(script);
        if exists(tx, &h)? {
            return Err(e!(Kind::Exist, "script already exists").into());
        }
        let enc_hash = inner.seal_pub(&h, "encrypt script hash")?;
        put_address(
            tx,
            h,
            &AddrRow::Imported {
                account: IMPORTED_ADDR_ACCOUNT,
                data: ImportedData::Script {
                    enc_hash,
                    script: script.to_vec(),
                }
                .encode(),
            },
        )?;
        managed_script(&self.chain, IMPORTED_ADDR_ACCOUNT, &h, script.to_vec())
    }

    /// Go's `PrivateKey` (`wallet/udb/addressmanager.go:2470-2523`): the
    /// private key of a derived address or an imported key, wiped when
    /// dropped (Go returns a `done` function that zeroes it).
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address with id <HASH160 hex>`; a
    /// derived row: loading the account and deriving privately (a locked
    /// BIP0044 account: `wallet locked: <account name>`, QK-0030; an
    /// imported xpub account: `no private key for <name>/<branch>/<index>`;
    /// the reserved imported account: `invalid operation: account does not
    /// record private keys`); an imported key: no lock check, as in Go
    /// (QK-0040): while locked the zeroed private crypto key fails to open
    /// the key, `encryption/decryption error: decrypt imported privkey:
    /// ...`, and a key imported without its private half fails the same
    /// way with `missing nonce`; a script: `invalid operation: no private key
    /// for P2SH address`; the store's.
    pub fn private_key<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        addr: &Address,
    ) -> Result<ReducedPrivateKey> {
        let mut guard = self.write();
        let inner = &mut *guard;
        let id = address_id(&normalize_address(addr))?;
        match fetch_address(tx, &id)? {
            AddrRow::Derived {
                account,
                branch,
                index,
            } => {
                let xpriv = inner.derive_key_from_path(
                    tx,
                    &self.chain,
                    account,
                    u32::from(branch),
                    index,
                    true,
                )?;
                let bytes = xpriv.serialized_priv_key().map_err(hd_err)?;
                Ok(ReducedPrivateKey::from_bytes(bytes))
            }
            AddrRow::Imported { data, .. } => {
                match ImportedData::decode(&data).map_err(bad_imported)? {
                    ImportedData::Key { enc_priv, .. } => {
                        // Go decrypts with whatever key it holds: the zeroed
                        // key while locked.
                        let zero = CryptoKey::from_bytes(&[0; 32]);
                        let key = inner.crypto_key_priv.as_ref().unwrap_or(&zero);
                        let plain = key.decrypt(&enc_priv).map_err(|err| {
                            crypto_msg(format!("decrypt imported privkey: {}", render_snacl(&err)))
                        })?;
                        Ok(ReducedPrivateKey::from_bytes(plain.as_bytes()))
                    }
                    ImportedData::Script { .. } => {
                        Err(e!(Kind::Invalid, "no private key for P2SH address").into())
                    }
                }
            }
        }
    }

    /// Go's `HavePrivateKey` (`wallet/udb/addressmanager.go:2527-2547`):
    /// whether the private key of an address is available when the wallet
    /// or account is unlocked: true for a derived address of a BIP0044
    /// account (on a watching-only wallet too, QK-0037), false for imported
    /// xpub and voting accounts, whether an imported key has its private
    /// half, false for a script.
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address with id <HASH160 hex>`; the
    /// store's.
    pub fn have_private_key<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        addr: &Address,
    ) -> Result<bool> {
        let Ok(id) = address_id(&normalize_address(addr)) else {
            return Ok(false);
        };
        Ok(match fetch_address(tx, &id)? {
            AddrRow::Derived { account, .. } => account < IMPORTED_ADDR_ACCOUNT,
            AddrRow::Imported { data, .. } => {
                match ImportedData::decode(&data).map_err(bad_imported)? {
                    ImportedData::Key { enc_priv, .. } => !enc_priv.is_empty(),
                    ImportedData::Script { .. } => false,
                }
            }
        })
    }

    /// Go's `RedeemScript` (`wallet/udb/addressmanager.go:2551-2574`): the
    /// script of an imported script address. The wallet may be locked.
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address with id <HASH160 hex>`; a
    /// derived address or imported key: `invalid operation: redeem script
    /// lookup requires P2SH address`; the store's.
    pub fn redeem_script<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        addr: &Address,
    ) -> Result<Vec<u8>> {
        let id = address_id(&normalize_address(addr))?;
        script_of_row(fetch_address(tx, &id)?)
    }

    /// Go's `SyncAccountToAddrIndex` (`wallet/udb/addressmanager.go:1985-2065`):
    /// records the derived addresses of an account branch from
    /// `sync_to_index` down to the first child already recorded (or child
    /// 0). Changes no counter.
    ///
    /// A child whose derivation is invalid is skipped; if child 0 is one,
    /// the index wraps and the next derivation fails, as in Go (QK-0044).
    ///
    /// # Errors
    ///
    /// In Go's order: the reserved imported account, `invalid operation:
    /// cannot sync imported account branch index`; loading the account; a
    /// branch other than 0 or 1, `invalid operation: account branch <n>`;
    /// an index above [`MAX_ADDRESSES_PER_ACCOUNT`], `invalid operation:
    /// child index <n> exceeds max`; hdkeychain's, unwrapped; the store's.
    pub fn sync_account_to_addr_index(
        &self,
        tx: &mut WriteTx<'_>,
        account: u32,
        sync_to_index: u32,
        branch: u32,
    ) -> Result<()> {
        if account == IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::Invalid, "cannot sync imported account branch index").into());
        }
        let xpub_branch = {
            let mut inner = self.write();
            let info = inner.load_account_info(tx, &self.chain, account)?;
            match branch {
                0 | 1 => info.key_pub.child(branch).map_err(hd_err)?,
                _ => return Err(e!(Kind::Invalid, format!("account branch {branch}")).into()),
            }
        };
        if sync_to_index > MAX_ADDRESSES_PER_ACCOUNT {
            return Err(e!(
                Kind::Invalid,
                format!("child index {sync_to_index} exceeds max")
            )
            .into());
        }
        // The branch is 0 or 1 here.
        let branch_byte = u8::from(branch == INTERNAL_BRANCH);
        let mut child = sync_to_index;
        loop {
            let key = match xpub_branch.child(child) {
                Ok(k) => k,
                Err(purse_hdkeychain::Error::InvalidChild) => {
                    child = child.wrapping_sub(1);
                    continue;
                }
                Err(err) => return Err(hd_err(err)),
            };
            let h = hash160(key.serialized_pub_key());
            if exists(tx, &h)? {
                break;
            }
            put_address(
                tx,
                h,
                &AddrRow::Derived {
                    account,
                    branch: branch_byte,
                    index: child,
                },
            )?;
            if child == 0 {
                break;
            }
            child = child.wrapping_sub(1);
        }
        Ok(())
    }

    /// Go's `RecordDerivedAddress` (`wallet/udb/addressmanager.go:2330-2338`):
    /// records a derived address for `HASH160(pub_key)`, replacing any row
    /// for that HASH160, an imported key's or script's included (QK-0044).
    /// Nothing else is checked: neither the account, nor that the key is
    /// the one at that path.
    ///
    /// # Errors
    ///
    /// A branch other than 0 or 1: `invalid operation: account branch <n>`,
    /// where Go records any branch (PARITY.md, `purse-udb`: the store keeps
    /// a branch in one byte, and no caller at the pin passes another
    /// branch, nor calls this method at all); the store's.
    pub fn record_derived_address(
        &self,
        tx: &mut WriteTx<'_>,
        account: u32,
        branch: u32,
        child: u32,
        pub_key: &[u8],
    ) -> Result<()> {
        if branch > INTERNAL_BRANCH {
            return Err(e!(Kind::Invalid, format!("account branch {branch}")).into());
        }
        put_address(
            tx,
            hash160(pub_key),
            &AddrRow::Derived {
                account,
                branch: u8::from(branch == INTERNAL_BRANCH),
                index: child,
            },
        )
    }

    /// Go's `MarkUsed` (`wallet/udb/addressmanager.go:1872-1893`): marks a
    /// derived address's child used ([`Manager::mark_used_child_index`]);
    /// does nothing for an imported key or script.
    ///
    /// # Errors
    ///
    /// No row: `item does not exist: no address with id <HASH160 hex>`;
    /// [`Manager::mark_used_child_index`]'s.
    pub fn mark_used(&self, tx: &mut WriteTx<'_>, addr: &Address) -> Result<()> {
        let id = address_id(&normalize_address(addr))?;
        match fetch_address(tx, &id)? {
            AddrRow::Derived {
                account,
                branch,
                index,
            } => self.mark_used_child_index(tx, account, u32::from(branch), index),
            AddrRow::Imported { .. } => Ok(()),
        }
    }

    /// Go's `MarkUsedChildIndex` (`wallet/udb/addressmanager.go:1896-1943`):
    /// raises the branch's last used child to `child`, and its last
    /// returned child with it when that is lower. The comparisons are
    /// Go's, in wrapping 32-bit arithmetic, so the "none" value
    /// `0xFFFFFFFF` is below every child.
    ///
    /// # Errors
    ///
    /// A branch other than 0 or 1: `invalid operation: account branch <n>`;
    /// an account with no row: `I/O error: no account <n>`, where Go panics
    /// (QK-0039; ADR-0012 Rule 2(a)); the store's.
    pub fn mark_used_child_index(
        &self,
        tx: &mut WriteTx<'_>,
        account: u32,
        branch: u32,
        child: u32,
    ) -> Result<()> {
        let b = match branch {
            0 => 0,
            1 => 1,
            _ => return Err(e!(Kind::Invalid, format!("account branch {branch}")).into()),
        };
        let mut row = counters_row(tx, account)?;
        let (last_used, last_ret) = (row.last_used[b], row.last_returned[b]);
        if child.wrapping_add(1) <= last_used.wrapping_add(1) {
            return Ok(());
        }
        row.last_used[b] = child;
        if last_ret.wrapping_add(1) < child.wrapping_add(1) {
            row.last_returned[b] = child;
        }
        tx.put::<purse_store::Accounts>(&purse_store::AccountKey(account), &row)
            .map_err(store_err)
    }

    /// Go's `MarkReturnedChildIndex` (`wallet/udb/addressmanager.go:1949-1971`):
    /// raises the branch's last returned child to `child`, or sets it when
    /// it is "none". Any branch other than 1 is the external branch, and
    /// the value is never lowered, whatever Go's comment says (QK-0039).
    ///
    /// # Errors
    ///
    /// An account with no row: `I/O error: no account <n>`, where Go panics
    /// (QK-0039; ADR-0012 Rule 2(a)); the store's.
    pub fn mark_returned_child_index(
        &self,
        tx: &mut WriteTx<'_>,
        account: u32,
        branch: u32,
        child: u32,
    ) -> Result<()> {
        let b = usize::from(branch == INTERNAL_BRANCH);
        let mut row = counters_row(tx, account)?;
        let last_ret = row.last_returned[b];
        if child > last_ret || last_ret == u32::MAX {
            row.last_returned[b] = child;
            tx.put::<purse_store::Accounts>(&purse_store::AccountKey(account), &row)
                .map_err(store_err)?;
        }
        Ok(())
    }

    /// Go's `ForEachAccountAddress` (`wallet/udb/addressmanager.go:2428-2440`):
    /// calls `f` with each address in the account's set, in the order of
    /// SHA-256 of the HASH160 (dcrwallet's bucket order), stopping at the
    /// first error. The set includes an address whose row now belongs to
    /// another account, yielded as that account's (QK-0757).
    ///
    /// The manager's lock is held while each address is built and released
    /// before `f` runs, so `f` may call the manager (in Go it would
    /// deadlock).
    ///
    /// # Errors
    ///
    /// Building an address ([`Manager::address`]'s errors for its row); `f`'s;
    /// the store's.
    pub fn for_each_account_address<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
        mut f: impl FnMut(ManagedAddress) -> Result<()>,
    ) -> Result<()> {
        let set = tx
            .rows_with_prefix::<AddrByAcctSha>(&AddrAcctShaKey::account_prefix(account))
            .map_err(store_err)?;
        for (key, h) in set {
            let row = tx
                .get::<AddrByHash>(&h)
                .map_err(store_err)?
                .ok_or_else(|| {
                    // Go: errors.E(errors.IO, err) around fetchAddressByHash's
                    // miss; every entry this crate writes has its row.
                    AnyError::from(e!(
                        Kind::Io,
                        AnyError::from(e!(
                            Kind::NotExist,
                            format!("no address with hash {}", hex(&key.sha))
                        ))
                    ))
                })?;
            let m = self.write().row_to_managed(tx, &self.chain, &row)?;
            f(m)?;
        }
        Ok(())
    }

    /// Go's `ForEachActiveAccountAddress`: the same as
    /// [`Manager::for_each_account_address`] (Go's has a TODO to return only
    /// active addresses).
    ///
    /// # Errors
    ///
    /// As [`Manager::for_each_account_address`].
    pub fn for_each_active_account_address<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
        f: impl FnMut(ManagedAddress) -> Result<()>,
    ) -> Result<()> {
        self.for_each_account_address(tx, account, f)
    }

    /// Go's `ForEachActiveAddress` (`wallet/udb/addressmanager.go:2452-2465`):
    /// calls `f` with the address of every recorded row, in the order of
    /// SHA-256 of the HASH160 (dcrwallet's `addr` bucket order), stopping
    /// at the first error. Every account a derived row names is loaded.
    ///
    /// The rows are read and sorted first (the store keys them by HASH160);
    /// the manager's lock is released before each call of `f`.
    ///
    /// # Errors
    ///
    /// As [`Manager::for_each_account_address`].
    pub fn for_each_active_address<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        mut f: impl FnMut(Address) -> Result<()>,
    ) -> Result<()> {
        let mut rows: Vec<([u8; 32], AddrRow)> = tx
            .rows_with_prefix::<AddrByHash>(&[])
            .map_err(store_err)?
            .into_iter()
            .map(|(h, row)| (Sha256::digest(h.0).into(), row))
            .collect();
        rows.sort_unstable_by_key(|a| a.0);
        for (_, row) in rows {
            let m = self.write().row_to_managed(tx, &self.chain, &row)?;
            f(m.address().clone())?;
        }
        Ok(())
    }
}

/// The SLIP0044 coin type upgrade's address step, which Go does not have
/// (QK-0760; ADR-0012 Rule 7): every derived row of `account` is removed,
/// with every `addr_by_acct_sha` entry that points at it, and the child of
/// `xpub` (the account's new extended public key) at the same branch and
/// index is recorded in its place, unless a row for that child's HASH160
/// already exists (`SyncAccountToAddrIndex` also leaves an existing row
/// alone). A row at a child `xpub` cannot derive, hardened or invalid, is
/// removed and not replaced: only `RecordDerivedAddress`, which has no
/// caller at the pin, writes one.
///
/// Go leaves the rows in place, keyed by the legacy children's HASH160s,
/// so after the upgrade the wallet claims the legacy addresses and signs
/// for them with the key of the SLIP0044 child at the same path. Here the
/// recorded paths stay recorded, for the keys the account now holds.
///
/// It reads the account's set and, when that holds a derived row, the
/// whole index once; an upgrade before any address is recorded (`dcrwallet
/// --create`) reads only the empty set.
///
/// # Errors
///
/// The store's; hdkeychain's for a derivation failure other than a
/// hardened or invalid child (none is known to occur).
pub(super) fn rederive_account_rows(
    tx: &mut WriteTx<'_>,
    account: u32,
    xpub: &ExtendedKey,
) -> Result<()> {
    let set = tx
        .rows_with_prefix::<AddrByAcctSha>(&AddrAcctShaKey::account_prefix(account))
        .map_err(store_err)?;
    let mut paths: Vec<(u8, u32)> = Vec::new();
    let mut gone = std::collections::BTreeSet::new();
    for (_, h) in set {
        if let Some(AddrRow::Derived {
            account: a,
            branch,
            index,
        }) = tx.get::<AddrByHash>(&h).map_err(store_err)?
        {
            // An entry QK-0757 left can point at another account's row.
            if a == account && gone.insert(h.0) {
                paths.push((branch, index));
            }
        }
    }
    if gone.is_empty() {
        return Ok(());
    }
    for h in &gone {
        tx.delete::<AddrByHash>(&Hash160(*h)).map_err(store_err)?;
    }
    // Entries in other accounts' sets can point at these rows (QK-0757).
    let stale: Vec<AddrAcctShaKey> = tx
        .rows_with_prefix::<AddrByAcctSha>(&[])
        .map_err(store_err)?
        .into_iter()
        .filter(|(_, h)| gone.contains(&h.0))
        .map(|(k, _)| k)
        .collect();
    for k in &stale {
        tx.delete::<AddrByAcctSha>(k).map_err(store_err)?;
    }
    let branches = [xpub.child(0), xpub.child(1)];
    paths.sort_unstable();
    for (branch, index) in paths {
        let Some(Ok(b)) = branches.get(usize::from(branch)) else {
            continue;
        };
        let child = match b.child(index) {
            Ok(k) => k,
            Err(
                purse_hdkeychain::Error::InvalidChild
                | purse_hdkeychain::Error::DeriveHardFromPublic,
            ) => continue,
            Err(err) => return Err(hd_err(err)),
        };
        let h = hash160(child.serialized_pub_key());
        if exists(tx, &h)? {
            continue;
        }
        put_address(
            tx,
            h,
            &AddrRow::Derived {
                account,
                branch,
                index,
            },
        )?;
    }
    Ok(())
}

/// The account row whose counters the marking methods change.
fn counters_row(tx: &mut WriteTx<'_>, account: u32) -> Result<purse_store::AccountRow> {
    tx.get::<purse_store::Accounts>(&purse_store::AccountKey(account))
        .map_err(store_err)?
        .ok_or_else(|| e!(Kind::Io, format!("no account {account}")).into())
}
