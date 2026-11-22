// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Passphrase changes: `ChangePassphrase` of `udb.Manager`
//! (`wallet/udb/addressmanager.go:1038-1155`; spec 02 section 5.8), for the
//! private and the public passphrase.
//!
//! The crypto keys never change: a change derives a new master key from
//! the new passphrase, with a new salt and this version's scrypt cost for
//! the network, seals the same crypto key under it, and rewrites the two
//! rows of that half (`crypto_priv` and `master_priv`, or `crypto_pub` and
//! `master_pub`). No other row is touched, so accounts, coin types and
//! imported keys, all sealed by the crypto keys, stay as they are.

use purse_errors::{AnyError, Kind, e};
use purse_snacl::SecretKey;
use purse_store::wallet::{CryptoKeyPriv, CryptoKeyPub, MasterKeyPriv, MasterKeyPub};
use purse_store::{SealedKey, Store, WriteTx};
use zeroize::{Zeroize, Zeroizing};

use super::rows::store_err;
use super::{Inner, Manager, Result, crypto_msg, keyed_hash, render_snacl, to_stored};

/// The error of the change's own write transaction.
enum ChangeErr {
    Store(purse_store::Error),
    Udb(AnyError),
}

impl From<purse_store::Error> for ChangeErr {
    fn from(e: purse_store::Error) -> Self {
        ChangeErr::Store(e)
    }
}

/// What a change installs in memory once its transaction has committed.
pub(super) enum Staged {
    /// A private passphrase change.
    Private {
        /// The new private master key: derived while the wallet is
        /// unlocked, zeroed while it is locked, as Go leaves it.
        master: SecretKey,
        /// The private crypto key sealed under `master`.
        sealed: Vec<u8>,
        /// The key of the new private passphrase hasher (Go's new keyed
        /// BLAKE2b-256).
        hasher_key: Zeroizing<[u8; 32]>,
        /// The new passphrase's hash under it, while the wallet is
        /// unlocked: `None` exactly when the change was staged on a locked
        /// manager.
        hash: Option<Zeroizing<[u8; 32]>>,
    },
    /// A public passphrase change.
    Public {
        /// The new public master key, derived (Go keeps it derived).
        master: SecretKey,
        /// The public crypto key sealed under `master`.
        sealed: Vec<u8>,
    },
}

impl Manager {
    /// Go's `ChangePassphrase` (`wallet/udb/addressmanager.go:1038-1155`):
    /// changes the private passphrase (`private`) or the public one from
    /// `old` to `new`.
    ///
    /// `old` is checked by deriving the current master key's parameters
    /// from it into a temporary key (wiped on return). A new master key is
    /// derived from `new` with a new 32-byte salt and the network's scrypt
    /// cost (Go's `scryptOptionsForNet`), whatever the old parameters were,
    /// and the crypto key of that half is sealed under it. For the private
    /// passphrase the sealed private crypto key is opened with the
    /// temporary key, so a locked wallet can be changed, and the lock state
    /// stays as it was: an unlocked wallet stays unlocked and recognises
    /// only `new` from then on (`Unlock`, `UnlockedWithPassphrase`); a
    /// locked one keeps no derived key. Draws come from the manager's
    /// source in Go's order and sizes: the salt, then for the private
    /// passphrase the 32-byte key of a new passphrase hasher, then the
    /// seal's 24-byte nonce. Nothing checks `new`: an empty private
    /// passphrase is accepted, as in Go, though creation refuses one
    /// (QK-0768), and so is `old` again.
    ///
    /// Like [`Manager::convert_to_watching_only`], and unlike every other
    /// method, this one runs its own write transaction on `store`, where Go
    /// takes the caller's bucket: the manager installs the new master key,
    /// sealed crypto key and passphrase hash only once that transaction
    /// has committed, so memory never accepts a passphrase the store does
    /// not hold (QK-0733, not reproduced; PARITY.md, `purse-udb`). Go's
    /// production callers (`Wallet.ChangePrivatePassphrase` and
    /// `ChangePublicPassphrase`) run the call alone in a `walletdb.Update`
    /// of their own, which this is. The manager's lock is held from the
    /// check of `old` until the new state is installed, as Go holds its
    /// mutex for the whole call, except after a commit whose outcome is
    /// unknown (below). As for any write transaction of the store, the
    /// caller must not hold one of its own on the same thread: the call
    /// would wait for it.
    ///
    /// On a wallet made by [`Manager::create_watch_only`] a public change
    /// leaves the private master key and the private crypto key as they
    /// were, as in Go (spec 02 quirk 34, QK-0037): the manager never
    /// derives that master key, but it still derives from the public
    /// passphrase given at creation, so that passphrase keeps opening
    /// account 0's private field, which holds account 0's key string (the
    /// extended private key itself when the wallet was created from one,
    /// QK-0762, pending the owner). A public change does not re-protect
    /// that copy, and no call can.
    ///
    /// # Errors
    ///
    /// In Go's order: a private change on a watching-only wallet, the bare
    /// `watching only wallet`; a wrong `old`, `secretkey.DeriveKey: invalid
    /// passphrase` (kind `Passphrase`), and the same op with kind `Other`
    /// for stored scrypt parameters over the manager's limits; snacl's
    /// errors for a failing random source; a sealed private crypto key that
    /// does not open, `Crypto` `decrypt crypto privkey: ...`; a failing seal,
    /// `Crypto` `encrypt crypto privkey: ...` or `encrypt crypto pubkey:
    /// ...`; the store's. On an error nothing is written and the manager is
    /// as it was. A commit whose outcome the store cannot know
    /// (`purse_store::Error::commit_outcome_unknown`) is taken as made, and
    /// the caller must reopen the store as that error requires. On that
    /// path the store has released the manager's lock with the
    /// transaction, so the lock is taken again and the new state fitted to
    /// what the manager has become meanwhile: a private change is not
    /// installed on a manager that has become watching-only, and one whose
    /// manager was locked or unlocked by another thread in the gap leaves
    /// the manager locked, holding the new parameters with no derived key
    /// or hash, so the new passphrase unlocks it; the change is otherwise
    /// installed as staged.
    pub fn change_passphrase(
        &self,
        store: &Store,
        old: &[u8],
        new: &[u8],
        private: bool,
    ) -> Result<()> {
        let mut staged = None;
        let committed = store.update(|tx| -> std::result::Result<_, ChangeErr> {
            let mut guard = self.write();
            let s = self
                .stage_change(&mut guard, old, new, private)
                .map_err(ChangeErr::Udb)?;
            write_change(tx, &s).map_err(ChangeErr::Udb)?;
            staged = Some(s);
            Ok(guard)
        });
        match committed {
            Ok(mut guard) => {
                if let Some(s) = staged.take() {
                    install(&mut guard, s);
                }
                Ok(())
            }
            Err(ChangeErr::Udb(err)) => Err(err),
            Err(ChangeErr::Store(err)) => {
                if err.commit_outcome_unknown()
                    && let Some(s) = staged.take()
                {
                    // The lock was released with the transaction.
                    let mut guard = self.write();
                    self.install_unconfirmed(&mut guard, s);
                }
                Err(store_err(err))
            }
        }
    }

    /// The install of a change whose commit's outcome is unknown, taken as
    /// made, with the manager's lock taken again after the store released
    /// it with the transaction (`inner` is that lock's state). Another
    /// thread may have locked, unlocked or converted the manager in the
    /// gap, and the staged state is fitted to what the manager is now, so
    /// that a locked manager never holds a derived private master key or a
    /// passphrase hash and an unlocked one never holds a zeroed key:
    ///
    /// - a private change on a manager that has become watching-only
    ///   installs nothing (the staged secrets are wiped on drop);
    /// - a private change staged under a lock state the manager no longer
    ///   has locks the manager (as [`Manager::lock`] does, every key an
    ///   account passphrase opened included) and installs the change's
    ///   locked form: the new parameters with a zeroed key and no hash.
    ///   This is the restrictive side, which
    ///   [`Manager::convert_to_watching_only`] takes on the same path; the
    ///   new passphrase unlocks the manager afterwards. Re-deriving the key
    ///   to keep an unlock made in the gap would cost a second scrypt
    ///   derivation on a path that already requires a reopen;
    /// - any other change installs as staged. A public change does not
    ///   depend on the lock state.
    pub(super) fn install_unconfirmed(&self, inner: &mut Inner, mut staged: Staged) {
        if let Staged::Private { master, hash, .. } = &mut staged {
            if self.is_watching_only() {
                return;
            }
            let staged_locked = hash.is_none();
            if staged_locked != inner.locked {
                inner.lock();
                master.zero();
                *hash = None;
            }
        }
        install(inner, staged);
    }

    /// Steps 1 to 4 of Go's `ChangePassphrase` short of its writes: checks
    /// `old`, derives the new master key and seals the crypto key under it.
    /// Changes nothing but the random source.
    pub(super) fn stage_change(
        &self,
        inner: &mut Inner,
        old: &[u8],
        new: &[u8],
        private: bool,
    ) -> Result<Staged> {
        if private && self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly).into());
        }
        let current = if private {
            // Every wallet that is not watching-only has the key.
            *inner
                .master_key_priv
                .as_ref()
                .ok_or_else(|| AnyError::from(e!(Kind::WatchingOnly)))?
                .parameters()
        } else {
            *inner.master_key_pub.parameters()
        };
        // Go's temporary `secretKey`: dropped (wiped) on return.
        let mut check = SecretKey::from_parameters(current);
        check.derive_key_with(old, &self.limits)?;

        let (n, r, p) = self.chain.scrypt_cost();
        let mut master = SecretKey::new_with(new, n, r, p, &mut *inner.rng, &self.limits)?;
        if !private {
            let sealed = master
                .encrypt_with(inner.crypto_key_pub.as_bytes(), &mut *inner.rng)
                .map_err(|err| {
                    crypto_msg(format!("encrypt crypto pubkey: {}", render_snacl(&err)))
                })?;
            return Ok(Staged::Public { master, sealed });
        }
        let mut hasher_key = Zeroizing::new([0u8; 32]);
        inner.rng.fill(&mut hasher_key[..]).map_err(|err| {
            AnyError::from(e!(
                Kind::Other,
                format!("passphrase hasher key: {}", err.message())
            ))
        })?;
        let plain = check
            .decrypt(&inner.crypto_key_priv_encrypted)
            .map_err(|err| crypto_msg(format!("decrypt crypto privkey: {}", render_snacl(&err))))?;
        let sealed = master
            .encrypt_with(plain.as_bytes(), &mut *inner.rng)
            .map_err(|err| crypto_msg(format!("encrypt crypto privkey: {}", render_snacl(&err))))?;
        drop(plain);
        let hash = if inner.locked {
            master.zero();
            None
        } else {
            Some(keyed_hash(&hasher_key, &[new]))
        };
        Ok(Staged::Private {
            master,
            sealed,
            hasher_key,
            hash,
        })
    }
}

/// Go's two writes, in its order: the sealed crypto key, then the master
/// key parameters (`putCryptoKeys`, `putMasterKeyParams` with the other
/// half nil, spec 02 section 9 item 3).
fn write_change(tx: &mut WriteTx<'_>, staged: &Staged) -> Result<()> {
    let put = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
        match staged {
            Staged::Private { master, sealed, .. } => {
                tx.put_wallet_row::<CryptoKeyPriv>(&SealedKey(sealed.clone()))?;
                tx.put_wallet_row::<MasterKeyPriv>(&to_stored(master.parameters()))
            }
            Staged::Public { master, sealed } => {
                tx.put_wallet_row::<CryptoKeyPub>(&SealedKey(sealed.clone()))?;
                tx.put_wallet_row::<MasterKeyPub>(&to_stored(master.parameters()))
            }
        }
    };
    put(tx).map_err(store_err)
}

/// Step 4.5 or 5.3 of Go's `ChangePassphrase`, after the commit: the new
/// master key replaces the old (dropped, which wipes it), and for the
/// private passphrase the sealed private crypto key, the passphrase hasher
/// and the hash are replaced too (the hash is none while locked). Go copies
/// the new sealed key into the old one's storage, which has the same
/// length (both seal a 32-byte key), so the two agree. The account
/// passphrases' hasher is not this one and is kept (see
/// `Inner::account_hasher_key`), so accounts their own passphrase unlocked
/// stay unlocked, as in Go, where each has a hasher of its own.
fn install(inner: &mut Inner, staged: Staged) {
    match staged {
        Staged::Private {
            master,
            sealed,
            hasher_key,
            hash,
        } => {
            inner.crypto_key_priv_encrypted.zeroize();
            inner.crypto_key_priv_encrypted = sealed;
            inner.master_key_priv = Some(master);
            inner.hasher_key = hasher_key;
            inner.priv_pass_hash = hash;
        }
        Staged::Public { master, .. } => inner.master_key_pub = master,
    }
}
