// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Watching-only wallets: creation from an account's extended public key
//! (Go's `createWatchOnly`, spec 02 section 5.4) and the conversion of a
//! wallet to a watching-only one (Go's `ConvertToWatchingOnly` and
//! `deletePrivateKeys`, spec 02 section 5.9).
//!
//! What every other operation does on a watching-only wallet (the
//! refusals of `Lock`, `Unlock`, `NewAccount`, the private crypto key, the
//! public half only of an imported private key, `ImportPublicKey`'s
//! acceptance) is in the operation itself, behind
//! [`Manager::watching_only`].

use std::sync::atomic::Ordering;

use purse_errors::{AnyError, Kind, e};
use purse_snacl::{CryptoKey, Limits, RandomSource, SecretKey};
use purse_store::wallet::{
    CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, CryptoKeyPub, LastAccount, MasterKeyPriv,
    MasterKeyPub, WatchOnly,
};
use purse_store::{
    AccountNumber, AccountRow, AddrByHash, AddrRow, CoinTypeKeys, SealedKey, Store, WalletRecord,
    WriteTx,
};
use zeroize::Zeroize;

use super::hd::{check_branch_keys, hd_err, unusable_seed};
use super::rows::{bad_imported, store_err};
use super::{
    ACCOUNT_TYPE_BIP0044, AccountKeys, ChainParams, DEFAULT_ACCOUNT_NAME, DEFAULT_ACCOUNT_NUM,
    IMPORTED_ADDR_ACCOUNT, IMPORTED_ADDR_ACCOUNT_NAME, ImportedData, Manager, NO_INDEX, Result,
    StoreRead, crypto_msg, parse_key, put_account, render_snacl, to_stored,
};

/// An hdkeychain error of the account key: `ErrInvalidChild` makes the
/// key unusable (`invalid seed: unusable seed`), any other is returned
/// unwrapped, as Go's `createWatchOnly` does.
fn key_err(err: purse_hdkeychain::Error) -> AnyError {
    match err {
        purse_hdkeychain::Error::InvalidChild => unusable_seed(),
        other => hd_err(other),
    }
}

/// The error of the conversion's own write transaction.
enum ConvertErr {
    Store(purse_store::Error),
    Udb(AnyError),
}

impl From<purse_store::Error> for ConvertErr {
    fn from(e: purse_store::Error) -> Self {
        ConvertErr::Store(e)
    }
}

/// Removes the private half of a coin type's sealed pair, deleting the row
/// when no public half is left (the store's typed row stands for Go's two
/// keys, either of which may be absent).
fn drop_coin_type_priv<R>(tx: &mut WriteTx<'_>) -> purse_store::Result<()>
where
    R: WalletRecord<Value = CoinTypeKeys>,
{
    match tx.wallet_row::<R>()? {
        Some(CoinTypeKeys {
            enc_pub: Some(enc_pub),
            ..
        }) => tx.put_wallet_row::<R>(&CoinTypeKeys {
            enc_pub: Some(enc_pub),
            enc_priv: None,
        }),
        Some(_) => tx.delete_wallet_row::<R>().map(|_| ()),
        None => Ok(()),
    }
}

/// Go's `deletePrivateKeys` at the current database version
/// (`wallet/udb/addressdb.go:1483-1589`), then `putWatchingOnly(ns, true)`.
///
/// Deletes the private master key's parameters, the sealed private crypto
/// key and both coin types' sealed private keys, and empties the private
/// half of every imported key. Accounts are left as they are: Go collects
/// only rows of the version-1 account type, of which a current database
/// has none, so every account keeps its sealed private key, and the parked
/// SLIP0044 account 0 row keeps its own (QK-0761). Those keys are sealed by
/// the private crypto key this deletes, which no row holds afterwards.
fn delete_private_keys(tx: &mut WriteTx<'_>) -> Result<()> {
    let put = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
        tx.delete_wallet_row::<MasterKeyPriv>()?;
        tx.delete_wallet_row::<CryptoKeyPriv>()?;
        drop_coin_type_priv::<CoinTypeLegacy>(tx)?;
        drop_coin_type_priv::<CoinTypeSlip0044>(tx)
    };
    put(tx).map_err(store_err)?;

    // Every imported key, whatever the account its row names, as Go walks
    // the whole `addr` bucket for rows of the imported type.
    let rows = tx.rows_with_prefix::<AddrByHash>(&[]).map_err(store_err)?;
    for (h, row) in rows {
        let AddrRow::Imported { account, data } = row else {
            continue;
        };
        let ImportedData::Key {
            enc_pub,
            mut enc_priv,
        } = ImportedData::decode(&data).map_err(bad_imported)?
        else {
            continue;
        };
        if enc_priv.is_empty() {
            continue;
        }
        enc_priv.zeroize();
        tx.put::<AddrByHash>(
            &h,
            &AddrRow::Imported {
                account,
                data: ImportedData::Key {
                    enc_pub,
                    enc_priv: Vec::new(),
                }
                .encode(),
            },
        )
        .map_err(store_err)?;
    }
    tx.put_wallet_row::<WatchOnly>(&true).map_err(store_err)
}

impl Manager {
    /// Go's `createWatchOnly` (`wallet/udb/addressmanager.go:3090-3216`) as
    /// `InitializeWatchOnly` leaves it after its upgrades, written directly
    /// in its final form: a watching-only wallet whose account 0 is the
    /// extended key `hd_pub_key` (Go's string, taken as bytes: bytes that
    /// are not UTF-8 cannot be a base58 key and fail as Go's decoder fails
    /// them, with `ErrInvalidKeyLen`).
    ///
    /// As in Go (spec 02 section 5.4; QK-0037): both master keys are
    /// derived from the public passphrase, with their own salts; the
    /// private crypto key is generated and sealed, and account 0's private
    /// field holds the account key's string sealed by it; no coin type key
    /// and no parked SLIP0044 account 0 row are written; account 0's
    /// counters are the "no index" value the version 2 upgrade gives an
    /// account without addresses, the reserved imported account's 0. The
    /// key's network is checked by the parser; neither its depth nor that it
    /// is public is checked (QK-0762): a private key is stored as Go stores
    /// it, sealed by the public crypto key and, in account 0's private
    /// field, by the private crypto key, whose master key a later public
    /// [`Manager::change_passphrase`] leaves derived from the creation's
    /// public passphrase.
    ///
    /// Randomness is drawn in Go's order and sizes: the two master key
    /// salts and the two crypto keys (32 bytes each), then one 24-byte
    /// nonce per sealed value (the two crypto keys, account 0's two
    /// fields). Nothing is written unless everything succeeds, and the
    /// caller's transaction rolls back on an error.
    ///
    /// # Errors
    ///
    /// In Go's order: an existing manager, `Exist` `address manager already
    /// exists`; a key that does not parse, hdkeychain's error, unwrapped
    /// (`the provided serialized extended key length is invalid`, `the
    /// provided extended key is for the wrong network`, a bad checksum,
    /// ...); a branch child that is invalid, `invalid seed: unusable seed`;
    /// snacl's errors for a failing random source or scrypt cost over
    /// `limits`; the store's.
    pub fn create_watch_only<R: RandomSource + ?Sized>(
        tx: &mut WriteTx<'_>,
        chain: &ChainParams,
        hd_pub_key: &[u8],
        pub_pass: &[u8],
        rng: &mut R,
        limits: &Limits,
    ) -> Result<()> {
        if tx
            .wallet_row::<MasterKeyPub>()
            .map_err(store_err)?
            .is_some()
        {
            return Err(e!(Kind::Exist, "address manager already exists").into());
        }
        let acct_key = parse_key(hd_pub_key, chain).map_err(key_err)?;
        check_branch_keys(&acct_key).map_err(key_err)?;
        // Go's `acctKeyPub.String()`: the key as given, private or public.
        let acct_string = acct_key.string();
        drop(acct_key);

        let (n, r, p) = chain.scrypt_cost();
        let master_pub = SecretKey::new_with(pub_pass, n, r, p, rng, limits)?;
        let master_priv = SecretKey::new_with(pub_pass, n, r, p, rng, limits)?;
        let crypto_pub = CryptoKey::generate_with(rng)?;
        let crypto_priv = CryptoKey::generate_with(rng)?;

        let crypto_pub_enc = master_pub
            .encrypt_with(crypto_pub.as_bytes(), rng)
            .map_err(|err| crypto_msg(format!("encrypt crypto pubkey: {}", render_snacl(&err))))?;
        let crypto_priv_enc = master_priv
            .encrypt_with(crypto_priv.as_bytes(), rng)
            .map_err(|err| crypto_msg(format!("encrypt crypto privkey: {}", render_snacl(&err))))?;
        let acct_pub_enc = crypto_pub
            .encrypt_with(acct_string.as_bytes(), rng)
            .map_err(|err| {
                crypto_msg(format!("encrypt account 0 pubkey: {}", render_snacl(&err)))
            })?;
        let acct_priv_enc = crypto_priv
            .encrypt_with(acct_string.as_bytes(), rng)
            .map_err(|err| {
                crypto_msg(format!("encrypt account 0 privkey: {}", render_snacl(&err)))
            })?;

        let put = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
            tx.put_wallet_row::<LastAccount>(&AccountNumber(DEFAULT_ACCOUNT_NUM))?;
            tx.put_wallet_row::<MasterKeyPriv>(&to_stored(master_priv.parameters()))?;
            tx.put_wallet_row::<MasterKeyPub>(&to_stored(master_pub.parameters()))?;
            tx.put_wallet_row::<CryptoKeyPub>(&SealedKey(crypto_pub_enc))?;
            tx.put_wallet_row::<CryptoKeyPriv>(&SealedKey(crypto_priv_enc))?;
            tx.put_wallet_row::<WatchOnly>(&true)?;
            put_account(
                tx,
                IMPORTED_ADDR_ACCOUNT,
                &AccountRow {
                    account_type: ACCOUNT_TYPE_BIP0044,
                    last_used: [0, 0],
                    last_returned: [0, 0],
                    name: IMPORTED_ADDR_ACCOUNT_NAME.as_bytes().to_vec(),
                    keys: AccountKeys::default().encode(),
                },
            )?;
            put_account(
                tx,
                DEFAULT_ACCOUNT_NUM,
                &AccountRow {
                    account_type: ACCOUNT_TYPE_BIP0044,
                    last_used: [NO_INDEX, NO_INDEX],
                    last_returned: [NO_INDEX, NO_INDEX],
                    name: DEFAULT_ACCOUNT_NAME.as_bytes().to_vec(),
                    keys: AccountKeys {
                        enc_pub: acct_pub_enc,
                        enc_priv: acct_priv_enc,
                        kdf: None,
                    }
                    .encode(),
                },
            )
        };
        put(tx).map_err(store_err)
    }

    /// Go's `ConvertToWatchingOnly` (`wallet/udb/addressmanager.go:1167-1218`):
    /// removes the wallet's private key material from the store, marks it
    /// watching-only, and locks the manager for good. Nothing happens on a
    /// wallet that is already watching-only.
    ///
    /// Unlike every other method, this one runs its own write transaction
    /// on `store`, where Go takes the caller's bucket: the manager changes
    /// its memory (watching-only from then on, the private master key and
    /// sealed private keys dropped) only once that transaction has
    /// committed, so memory never says watching-only over a store that
    /// rolled the conversion back (QK-0733, not reproduced; PARITY.md,
    /// `purse-udb`). The manager is locked inside the transaction, so no
    /// private key is sealed or opened between the deletion and the
    /// commit; a conversion that rolls back leaves it locked, nothing else
    /// changed. Go's only caller is a test (spec 02 section 5.9), and its
    /// documentation tells the caller to convert a copy of the database.
    ///
    /// What the store keeps is Go's (QK-0761): every account's sealed
    /// private key and the parked SLIP0044 account 0 row stay, sealed by
    /// the deleted private crypto key; the private master key's parameters,
    /// the sealed private crypto key, the coin types' private keys and the
    /// imported keys' private halves are gone. Memory follows the store:
    /// where Go empties the cached accounts' sealed private keys, so that
    /// its answer to a private derivation depends on what was cached, the
    /// cache keeps them, and every account answers as Go's do after a
    /// reopen (QK-0764).
    ///
    /// # Errors
    ///
    /// The store's, and an imported address row that does not decode
    /// (kind `IO`, as for every reader of one). On an error the store is
    /// unchanged and the manager is as it was, except that it may be
    /// locked. A commit whose outcome the store cannot know
    /// (`purse_store::Error::commit_outcome_unknown`) is taken as made: the
    /// manager becomes watching-only, the restrictive side, and the caller
    /// must reopen the store as that error requires.
    pub fn convert_to_watching_only(&self, store: &Store) -> Result<()> {
        if self.is_watching_only() {
            return Ok(());
        }
        let converted = store.update(|tx| -> std::result::Result<_, ConvertErr> {
            let mut guard = self.write();
            if self.is_watching_only() {
                return Ok(None);
            }
            delete_private_keys(tx).map_err(ConvertErr::Udb)?;
            guard.lock();
            Ok(Some(guard))
        });
        match converted {
            Ok(Some(mut guard)) => {
                self.forget_private_keys(&mut guard);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(ConvertErr::Udb(err)) => Err(err),
            Err(ConvertErr::Store(err)) => {
                if err.commit_outcome_unknown() {
                    let mut guard = self.write();
                    guard.lock();
                    self.forget_private_keys(&mut guard);
                }
                Err(store_err(err))
            }
        }
    }

    /// The in-memory half of Go's `ConvertToWatchingOnly`: the sealed
    /// private crypto key and the private master key are dropped, and the
    /// manager is watching-only. Called with the write lock held, on a
    /// locked manager.
    ///
    /// Go also empties every cached account's sealed private key, while
    /// the store keeps it (QK-0761), so a cached account then answers a
    /// private derivation with `no private key for <name>/<branch>/<index>`
    /// and an uncached one, or any account after a reopen, with the locked
    /// account's text. The cache keeps the store's sealed keys here, so the
    /// answer is the store's whatever was cached (QK-0764).
    fn forget_private_keys(&self, inner: &mut super::Inner) {
        inner.crypto_key_priv_encrypted.zeroize();
        inner.crypto_key_priv_encrypted = Vec::new();
        inner.crypto_key_priv = None;
        inner.master_key_priv = None;
        self.watching_only.store(true, Ordering::SeqCst);
    }
}
