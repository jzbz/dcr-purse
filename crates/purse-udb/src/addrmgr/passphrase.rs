// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Per-account passphrases and imported voting accounts:
//! `SetAccountPassphrase`, `UnlockAccount`, `LockAccount` and
//! `ImportVotingAccount` of `udb.Manager` (`wallet/udb/addressmanager.go`;
//! spec 02 sections 5.14 and 5.15).
//!
//! An account with its own passphrase keeps its extended private key
//! sealed with XChaCha20-Poly1305 under a key Argon2id derives from that
//! passphrase, with the Argon2id parameters stored beside it in the
//! account's key material ([`AccountKeys::kdf`]); the wallet passphrase
//! neither opens nor locks it, but every wallet lock locks it too, as in
//! Go. An unlocked account's key is held in the manager's
//! `account_unlocks`, tied to the sealed value it belongs to, so that
//! memory follows the store when a transaction that resealed the key rolls
//! back (QK-0733).

use purse_errors::{AnyError, Kind, e};
use purse_snacl::{Argon2idParams, CryptoKey};
use purse_store::wallet::LastImportedAccount;
use purse_store::{AccountKey, AccountNumber, AccountRow, Accounts, WriteTx};
use subtle::ConstantTimeEq;

use super::addresses::exists;
use super::rows::{AccountKeys, StoreRead, bad_keys, store_err};
use super::{
    ACCOUNT_TYPE_IMPORTED_VOTING, AccountUnlock, ExtendedKey, IMPORTED_ADDR_ACCOUNT, Manager,
    NO_INDEX, Result, crypto_msg, fetch_account_row, io_hd, parse_key, put_account, render_snacl,
    validate_account_name,
};
use dcroxide_txscript::stdaddr::hash160;

/// `errors.E(errors.Crypto, "account is not encrypted with a unique
/// passphrase")`.
fn not_unique() -> AnyError {
    e!(
        Kind::Crypto,
        "account is not encrypted with a unique passphrase"
    )
    .into()
}

/// Rewrites the key material of `account`'s row: the private key sealed as
/// `enc_priv`, with `kdf` the parameters of its own passphrase (none: sealed
/// by the private crypto key). Go writes the parameters (`kdf-params`) and
/// the row (`acct`) separately; here they are one value, so they change
/// together (spec 02 section 9 item 27).
fn reseal_row(
    tx: &mut WriteTx<'_>,
    account: u32,
    enc_priv: Vec<u8>,
    kdf: Option<Argon2idParams>,
) -> Result<()> {
    let mut row = fetch_account_row(tx, account)?;
    let mut keys = AccountKeys::decode(&row.keys).map_err(|e| bad_keys(account, e))?;
    keys.enc_priv = enc_priv;
    keys.kdf = kdf;
    row.keys = keys.encode();
    tx.put::<Accounts>(&AccountKey(account), &row)
        .map_err(store_err)
}

impl Manager {
    /// Go's `SetAccountPassphrase` (`wallet/udb/addressmanager.go:1690-1789`)
    /// and `removeAccountPassphrase` (`:1791-1843`): seals the account's
    /// extended private key under a key Argon2id derives from `passphrase`,
    /// with new parameters (a 16-byte salt drawn from the manager's source,
    /// time 1, 256 MiB, the processor count of [`Manager::with_num_cpu`]),
    /// or, for an empty passphrase, seals it again under the private crypto
    /// key and drops the parameters.
    ///
    /// The account must be unlocked: by the wallet when the wallet
    /// passphrase protects it, by [`Manager::unlock_account`] when it has
    /// its own. It stays unlocked afterwards, as in Go. A watching-only
    /// wallet accepts only a non-empty passphrase for an account above the
    /// reserved imported account (an imported voting account it unlocked).
    ///
    /// Go changes the cached account before the caller's transaction
    /// commits. Here the cached entry is dropped and the account is cached
    /// again only from a snapshot holding the new sealed key; the kept
    /// private key is tied to that sealed key, so after a rollback the
    /// account reads as the store has it, unlocked or not as it was
    /// (QK-0733, not reproduced). Go also draws 32 bytes for a new
    /// passphrase hasher here; this draws the salt and the nonce only (see
    /// the crate documentation, "Randomness").
    ///
    /// # Errors
    ///
    /// In Go's order: a watching-only wallet, for an account below the
    /// reserved imported account `watching only wallet: cannot set
    /// passphrase for watching wallet`, for an empty passphrase `invalid
    /// passphrase: watching-only imported accounts must have a passprase`
    /// (sic); loading the account (`item does not exist: no account <n>`,
    /// and for the reserved imported account `Crypto` `decrypt account
    /// 2147483647 pubkey: ...`); no private key, `wallet locked: wallet
    /// must be unlocked to set a unique account passphrase` (an imported
    /// xpub account too, which no unlock helps, spec 02 quirk 16) or
    /// `wallet locked: account must be unlocked to set a unique account
    /// passphrase`; for an empty passphrase on a locked wallet, `wallet
    /// locked: wallet must be unlocked to remove account's unique
    /// passphrase`; snacl's errors for a failing random source or Argon2id
    /// parameters over the manager's limits; the store's.
    pub fn set_account_passphrase(
        &self,
        tx: &mut WriteTx<'_>,
        account: u32,
        passphrase: &[u8],
    ) -> Result<()> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if self.is_watching_only() {
            if account < IMPORTED_ADDR_ACCOUNT {
                return Err(e!(
                    Kind::WatchingOnly,
                    "cannot set passphrase for watching wallet"
                )
                .into());
            }
            if passphrase.is_empty() {
                return Err(e!(
                    Kind::Passphrase,
                    "watching-only imported accounts must have a passprase"
                )
                .into());
            }
        }
        let info = inner
            .load_account_info(tx, &self.chain, account)?
            .into_owned();
        let Some(xpriv) = info.key_priv else {
            let who = if info.unique_key.is_some() {
                "account"
            } else {
                "wallet"
            };
            return Err(e!(
                Kind::Locked,
                format!("{who} must be unlocked to set a unique account passphrase")
            )
            .into());
        };

        if passphrase.is_empty() {
            // removeAccountPassphrase. Its watching-only check cannot be
            // reached: an empty passphrase on a watching-only wallet was
            // refused above.
            if self.is_watching_only() {
                return Err(e!(
                    Kind::WatchingOnly,
                    "cannot remove passphrase for watching wallet"
                )
                .into());
            }
            if inner.locked {
                return Err(e!(
                    Kind::Locked,
                    "wallet must be unlocked to remove account's unique passphrase"
                )
                .into());
            }
            let sealed = inner.seal_priv(
                xpriv.string().as_bytes(),
                &format!("encrypt account {account} privkey"),
            )?;
            reseal_row(tx, account, sealed.clone(), None)?;
            inner.accounts.remove(&account);
            inner.rekeyed.insert(account, sealed);
            return Ok(());
        }

        let pass_hash = inner.account_pass_hash(account, passphrase);
        let kdf = Argon2idParams::new_with(&mut *inner.rng, self.num_cpu)?;
        let key = kdf.derive_key_with(passphrase, &self.limits)?;
        let sealed = key.seal_with(xpriv.string().as_bytes(), &mut *inner.rng)?;
        drop(key);
        reseal_row(tx, account, sealed.clone(), Some(kdf))?;
        inner.accounts.remove(&account);
        inner.rekeyed.insert(account, sealed.clone());
        inner
            .account_unlocks
            .entry(account)
            .or_default()
            .push(AccountUnlock {
                sealed,
                key_priv: xpriv,
                pass_hash,
            });
        Ok(())
    }

    /// Go's `UnlockAccount` (`wallet/udb/addressmanager.go:1590-1649`):
    /// opens the private key of an account with its own passphrase. The
    /// wallet need not be unlocked; a watching-only wallet unlocks only
    /// accounts at or above the reserved imported account (its imported
    /// voting accounts).
    ///
    /// On an account already unlocked, a matching passphrase changes
    /// nothing and a different one fails without locking it, as in Go.
    /// Otherwise the key is derived with Argon2id from the stored
    /// parameters and the sealed key opened; the opened key stays in memory
    /// until [`Manager::lock_account`] or any wallet lock, tied to the
    /// sealed value it was opened from. Go also logs `Account <n> has been
    /// unlocked`; this crate logs nothing.
    ///
    /// # Errors
    ///
    /// In Go's order: a watching-only wallet and an account below the
    /// reserved one, `watching only wallet: cannot unlock watching wallet`;
    /// loading the account; an account without its own passphrase, `Crypto`
    /// `account is not encrypted with a unique passphrase`; a different
    /// passphrase on an unlocked account, the bare `invalid passphrase`;
    /// Argon2id parameters the derivation refuses (only a corrupt or
    /// foreign row holds them; Go panics, QK-0630); a sealed key shorter
    /// than 40 bytes, `encryption/decryption error: ciphertext too short
    /// (len <n>) to encode nonce and MAC tag`; a wrong passphrase (or any
    /// sealed key that does not authenticate), the bare `invalid
    /// passphrase`; a key that does not parse, `IO` with hdkeychain's
    /// error.
    pub fn unlock_account<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
        passphrase: &[u8],
    ) -> Result<()> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if self.is_watching_only() && account < IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::WatchingOnly, "cannot unlock watching wallet").into());
        }
        let info = inner.load_account_info(tx, &self.chain, account)?;
        let Some(kdf) = info.unique_key else {
            return Err(not_unique());
        };
        let sealed = info.key_encrypted.clone();
        let unlocked = info.key_priv.is_some();
        drop(info);
        let pass_hash = inner.account_pass_hash(account, passphrase);
        if unlocked {
            let matches = inner
                .account_unlocks
                .get(&account)
                .and_then(|v| v.iter().find(|u| u.sealed == sealed))
                .is_some_and(|u| bool::from(u.pass_hash.ct_eq(&pass_hash[..])));
            if !matches {
                return Err(e!(Kind::Passphrase).into());
            }
            return Ok(());
        }
        let key = kdf.derive_key_with(passphrase, &self.limits)?;
        let plain = key.unseal(&sealed)?;
        drop(key);
        let key_priv = parse_key(plain.as_bytes(), &self.chain).map_err(io_hd)?;
        drop(plain);
        inner
            .account_unlocks
            .entry(account)
            .or_default()
            .push(AccountUnlock {
                sealed,
                key_priv,
                pass_hash,
            });
        Ok(())
    }

    /// Go's `LockAccount` (`wallet/udb/addressmanager.go:1653-1686`): drops
    /// (and wipes) the private key of an account with its own passphrase.
    /// Go also logs `Account <n> has been locked`; this crate logs nothing.
    ///
    /// # Errors
    ///
    /// In Go's order: a watching-only wallet and an account below the
    /// reserved one, `watching only wallet: cannot lock watching wallet`;
    /// loading the account; an account without its own passphrase, `Crypto`
    /// `account is not encrypted with a unique passphrase`; a locked
    /// account, `wallet locked: account is already locked`.
    pub fn lock_account<S: StoreRead + ?Sized>(&self, tx: &mut S, account: u32) -> Result<()> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if self.is_watching_only() && account < IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::WatchingOnly, "cannot lock watching wallet").into());
        }
        let info = inner.load_account_info(tx, &self.chain, account)?;
        if info.unique_key.is_none() {
            return Err(not_unique());
        }
        if info.key_priv.is_none() {
            return Err(e!(Kind::Locked, "account is already locked").into());
        }
        drop(info);
        // Every entry of the account holds the same key: all go.
        inner.account_unlocks.remove(&account);
        Ok(())
    }

    /// Go's `ImportVotingAccount` (`wallet/udb/addressmanager.go:2181-2237`)
    /// and `importAccount` (`:2241-2318`): records an extended private key
    /// as the next imported account (after the last imported xpub or voting
    /// account; the first is 0x80000000), of type imported voting, its
    /// private key sealed by its own passphrase (new Argon2id parameters, as
    /// [`Manager::set_account_passphrase`] makes them). Returns the account
    /// number. The account is left locked and is not cached, as in Go.
    ///
    /// As in Go, the wallet may be locked or watching-only, and the key's
    /// network is not checked (a key of another network is stored, and the
    /// account then fails to load; QK-0041). Go first seals the private key
    /// under the private crypto key in whatever state it is in (zeroed
    /// while locked) and overwrites that field in the same transaction;
    /// that seal, which no committed row ever holds, is made here too (under
    /// the zero key when no private crypto key is held), so its draw and
    /// its error are Go's; on a manager converted to watching-only in this
    /// process, where Go dereferences the nil key and panics, this
    /// succeeds, as Go does after a reopen (QK-0765). Where Go's account
    /// number wraps from 0xFFFFFFFF to 0 and the import overwrites account
    /// 0, this refuses with `ImportXpubAccount`'s text (QK-0041).
    ///
    /// Randomness, in Go's order: the nonces of the public key's seal and
    /// of the private key's first seal, the 16-byte salt, the nonce of the
    /// final seal.
    ///
    /// # Errors
    ///
    /// In Go's order: an empty passphrase, `invalid passphrase: passphrase
    /// must be specified`; [`validate_account_name`]'s; a public key,
    /// `invalid operation: extended key must be an xpriv`; a name in use,
    /// `item already exists: account named <quoted name> already exists`;
    /// a key whose external branch or its first child does not derive,
    /// `invalid operation: undable to derive external branch` or `undable
    /// to derive index` (sic); that first address already recorded, `item
    /// already exists: address belonging to this key already exists in the
    /// database`; [`Manager::last_imported_account`]'s; no account number
    /// left, `invalid operation: exhausted possible imported accounts`
    /// (dcr-purse's check, QK-0041); a failing random source, `Crypto`
    /// `encrypt account pubkey: ...` or `encrypt account privkey: ...`, or
    /// snacl's error; Argon2id parameters over the manager's limits; the
    /// store's.
    pub fn import_voting_account(
        &self,
        tx: &mut WriteTx<'_>,
        acct_key_priv: &ExtendedKey,
        passphrase: &[u8],
        name: &[u8],
    ) -> Result<u32> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if passphrase.is_empty() {
            return Err(e!(Kind::Passphrase, "passphrase must be specified").into());
        }
        validate_account_name(name)?;
        if !acct_key_priv.is_private() {
            return Err(e!(Kind::Invalid, "extended key must be an xpriv").into());
        }
        if super::accounts::name_in_use(tx, name)? {
            return Err(super::accounts::name_exists(name));
        }
        let acct_key_pub = acct_key_priv.neuter();
        let branch = acct_key_pub
            .child(super::EXTERNAL_BRANCH)
            .map_err(|_| AnyError::from(e!(Kind::Invalid, "undable to derive external branch")))?;
        let first = branch
            .child(0)
            .map_err(|_| AnyError::from(e!(Kind::Invalid, "undable to derive index")))?;
        drop(branch);
        if exists(tx, &hash160(first.serialized_pub_key()))? {
            return Err(e!(
                Kind::Exist,
                "address belonging to this key already exists in the database"
            )
            .into());
        }
        let account = self
            .last_imported_account(tx)?
            .checked_add(1)
            .ok_or_else(|| {
                AnyError::from(e!(Kind::Invalid, "exhausted possible imported accounts"))
            })?;
        let enc_pub =
            inner.seal_public(acct_key_pub.string().as_bytes(), "encrypt account pubkey")?;
        // Go's first seal of the private key, overwritten below: under the
        // private crypto key, or the zero key Go holds while locked.
        {
            let zero = CryptoKey::from_bytes(&[0; 32]);
            let super::Inner {
                crypto_key_priv,
                rng,
                ..
            } = &mut *inner;
            let key = crypto_key_priv.as_ref().unwrap_or(&zero);
            let mut first_seal = key
                .encrypt_with(acct_key_priv.string().as_bytes(), &mut **rng)
                .map_err(|err| {
                    crypto_msg(format!("encrypt account privkey: {}", render_snacl(&err)))
                })?;
            zeroize::Zeroize::zeroize(&mut first_seal);
        }
        let kdf = Argon2idParams::new_with(&mut *inner.rng, self.num_cpu)?;
        let key = kdf.derive_key_with(passphrase, &self.limits)?;
        let enc_priv = key.seal_with(acct_key_priv.string().as_bytes(), &mut *inner.rng)?;
        drop(key);

        tx.put_wallet_row::<LastImportedAccount>(&AccountNumber(account))
            .map_err(store_err)?;
        put_account(
            tx,
            account,
            &AccountRow {
                account_type: ACCOUNT_TYPE_IMPORTED_VOTING,
                last_used: [NO_INDEX, NO_INDEX],
                last_returned: [NO_INDEX, NO_INDEX],
                name: name.to_vec(),
                keys: AccountKeys {
                    enc_pub,
                    enc_priv,
                    kdf: Some(kdf),
                }
                .encode(),
            },
        )
        .map_err(store_err)?;
        Ok(account)
    }
}
