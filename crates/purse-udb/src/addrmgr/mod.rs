// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The address manager: a port of dcrwallet's `udb.Manager`
//! (`wallet/udb/addressmanager.go`, `addressdb.go`) over `purse-store`.
//!
//! The crate documentation has the table of Go entry points, the store
//! rows, and the locking and secret-handling model. This slice holds
//! creation from a seed ([`Manager::create`]), opening
//! ([`Manager::open`]), the wallet lock ([`Manager::unlock`],
//! [`Manager::lock`]), the coin type ([`Manager::coin_type_priv_key`],
//! [`Manager::upgrade_to_slip0044_coin_type`]), accounts (BIP0044
//! accounts, imported xpub accounts, renaming and lookup), addresses:
//! derivation and recording, the used and returned indexes, lookups,
//! private keys, and imported keys and scripts, sealing with the crypto
//! keys ([`Manager::encrypt_public`] and its siblings), watching-only
//! wallets ([`Manager::create_watch_only`],
//! [`Manager::convert_to_watching_only`]), per-account passphrases
//! ([`Manager::set_account_passphrase`], [`Manager::unlock_account`],
//! [`Manager::lock_account`]), imported voting accounts
//! ([`Manager::import_voting_account`]) and passphrase changes
//! ([`Manager::change_passphrase`]).

mod accounts;
mod addresses;
mod change;
mod cointype;
mod hd;
mod passphrase;
mod rows;
mod watchonly;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{PoisonError, RwLock, RwLockWriteGuard};

use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
use purse_errors::{AnyError, Kind, e};
use purse_snacl::{Argon2idParams, CryptoKey, Limits, Parameters, RandomSource, SecretKey};
use purse_store::wallet::{
    CoinTypeLegacy, CoinTypeSlip0044, CryptoKeyPriv, CryptoKeyPub, LastAccount,
    LastImportedAccount, MasterKeyPriv, MasterKeyPub, Slip0044Account0, WatchOnly,
};
use purse_store::{
    AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts, AddrAcctShaKey,
    AddrByAcctSha, CoinTypeKeys, MasterKeyParams, SealedKey, Slip0044Acct0Row, WriteTx,
};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

pub use addresses::ManagedAddress;
pub use dcroxide_dcrec::secp256k1::ReducedPrivateKey;
pub use dcroxide_txscript::stdaddr::Address;
pub use hd::{
    AddrIds, ChainParams, HdKeys, coin_types, derive_account_key, derive_coin_type_key,
    hd_keys_from_seed,
};
pub use purse_dcrutil::wif::Wif;
pub use purse_hdkeychain::ExtendedKey;
pub use rows::{AccountKeys, AccountKeysError, ImportedData, ImportedDataError, StoreRead};
use rows::{bad_keys, store_err};

/// Go's `MaxAccountNum`: the highest BIP0044 account number.
pub const MAX_ACCOUNT_NUM: u32 = 0x7FFF_FFFE;
/// Go's `MaxAddressesPerAccount`.
pub const MAX_ADDRESSES_PER_ACCOUNT: u32 = 0x7FFF_FFFF;
/// Go's `ImportedAddrAccount`: the reserved account of imported keys and
/// scripts. Accounts above it are imported xpub and voting accounts.
pub const IMPORTED_ADDR_ACCOUNT: u32 = 0x7FFF_FFFF;
/// Go's `ImportedAddrAccountName`.
pub const IMPORTED_ADDR_ACCOUNT_NAME: &str = "imported";
/// Go's `DefaultAccountNum`.
pub const DEFAULT_ACCOUNT_NUM: u32 = 0;
/// Go's `defaultAccountName`: account 0's name at creation (not reserved).
pub const DEFAULT_ACCOUNT_NAME: &str = "default";
/// Go's `maxCoinType`.
pub const MAX_COIN_TYPE: u32 = 0x7FFF_FFFF;
/// Go's `ExternalBranch`.
pub const EXTERNAL_BRANCH: u32 = 0;
/// Go's `InternalBranch`.
pub const INTERNAL_BRANCH: u32 = 1;
/// The "no index yet" value of the last-used and last-returned counters.
pub const NO_INDEX: u32 = u32::MAX;

/// Go's `actBIP0044`: the account type of BIP0044 and imported xpub
/// accounts (and of the reserved imported account's row).
pub const ACCOUNT_TYPE_BIP0044: u8 = 1;
/// Go's `importedVoting`: the account type of imported voting accounts.
pub const ACCOUNT_TYPE_IMPORTED_VOTING: u8 = 2;

/// The error type of every operation: dcrwallet's `error`, an
/// `*errors.Error` or a dependency's error passed through unwrapped,
/// rendered byte for byte as Go renders it (ADR-0011).
pub type Result<T> = std::result::Result<T, AnyError>;

/// A random source the manager keeps for later draws.
pub type BoxedRandom = Box<dyn RandomSource + Send + Sync>;

/// Go's `ValidateAccountName` (`wallet/udb/addressmanager.go:2069-2077`).
///
/// # Errors
///
/// The empty name: `Invalid` `accounts may not be named the empty string`;
/// `imported`: `Invalid` `reserved account name`. Any other bytes are
/// accepted.
pub fn validate_account_name(name: &[u8]) -> Result<()> {
    if name.is_empty() {
        return Err(e!(Kind::Invalid, "accounts may not be named the empty string").into());
    }
    if name == IMPORTED_ADDR_ACCOUNT_NAME.as_bytes() {
        return Err(e!(Kind::Invalid, "reserved account name").into());
    }
    Ok(())
}

/// Go's `IsImportedVoting`.
pub fn is_imported_voting(account_type: u8) -> bool {
    account_type == ACCOUNT_TYPE_IMPORTED_VOTING
}

/// Go's `AccountProperties` (`wallet/udb/addressmanager.go:213-224`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountProperties {
    /// The account number.
    pub account_number: u32,
    /// The account name, as stored.
    pub account_name: Vec<u8>,
    /// The account type byte (0 for the reserved imported account, as Go
    /// reports it).
    pub account_type: u8,
    /// Last used child of the external branch.
    pub last_used_external_index: u32,
    /// Last used child of the internal branch.
    pub last_used_internal_index: u32,
    /// Last returned child of the external branch.
    pub last_returned_external_index: u32,
    /// Last returned child of the internal branch.
    pub last_returned_internal_index: u32,
    /// Imported keys and scripts (the reserved imported account only).
    pub imported_key_count: u32,
    /// Whether the account has its own passphrase.
    pub account_encrypted: bool,
    /// Whether that passphrase has unlocked it.
    pub account_unlocked: bool,
}

/// Go's `accountInfo` (`wallet/udb/addressmanager.go:136-154`): a cached
/// account.
///
/// Only what the store holds is kept, as of a committed snapshot: the name,
/// which `RenameAccount` changes, is read from the account's row where Go
/// reads its cached copy, and an account whose sealed private key this
/// manager rewrites (`SetAccountPassphrase`) is dropped from the cache and
/// cached again only from a snapshot that holds the new key
/// ([`Inner::rekeyed`]), so memory cannot disagree with the store after a
/// caller's transaction rolls back (QK-0733).
#[derive(Clone)]
struct AccountInfo {
    account_type: u8,
    key_encrypted: Vec<u8>,
    key_pub: ExtendedKey,
    /// The account's extended private key, wiped when dropped. In the
    /// cache, present only for an account the wallet passphrase protects,
    /// while the wallet is unlocked; an account with its own passphrase is
    /// cached without it, and [`Inner::load_account_info`] returns a copy
    /// carrying the key [`Inner::account_unlocks`] holds for it.
    key_priv: Option<ExtendedKey>,
    unique_key: Option<Argon2idParams>,
}

/// The private key of an account with its own passphrase, opened by
/// `UnlockAccount` or kept by `SetAccountPassphrase` (Go's `acctKeyPriv`
/// and `uniquePassHash` of a uniquely encrypted account).
///
/// It is tied to the sealed key it was opened from, or sealed into: it
/// unlocks the account only while the account's row holds that value.
/// Sealed values carry a random nonce, so a value written by a transaction
/// that rolled back is never the row's, and the account then reads as the
/// store has it (QK-0733).
struct AccountUnlock {
    /// The sealed private key of the row this key unlocks.
    sealed: Vec<u8>,
    /// The account's extended private key; wiped when dropped.
    key_priv: ExtendedKey,
    /// The keyed hash of the account passphrase ([`Inner::account_pass_hash`]).
    pass_hash: Zeroizing<[u8; 32]>,
}

/// The state Go's `Manager.mtx` guards.
struct Inner {
    locked: bool,
    accounts: BTreeMap<u32, AccountInfo>,
    /// The public master key, derived at open and kept until drop; a
    /// public passphrase change replaces it (only its parameters are read
    /// again, by the next change).
    master_key_pub: SecretKey,
    /// The private master key's parameters, its key derived only while
    /// unlocked; `None` for a watching-only wallet.
    master_key_priv: Option<SecretKey>,
    crypto_key_pub: CryptoKey,
    /// The sealed private crypto key (empty when the store has none).
    crypto_key_priv_encrypted: Vec<u8>,
    /// The private crypto key, present only while unlocked.
    crypto_key_priv: Option<CryptoKey>,
    /// The key of the private passphrase hasher (Go's
    /// `privPassphraseHasher`): drawn at open, replaced by a private
    /// passphrase change, as Go replaces the hasher.
    hasher_key: Zeroizing<[u8; 32]>,
    /// The key of the account passphrases' hasher
    /// ([`Inner::account_pass_hash`]): the key drawn at open, kept until
    /// drop, so that a private passphrase change leaves the hashes of
    /// unlocked accounts valid (Go gives each account a hasher of its own).
    account_hasher_key: Zeroizing<[u8; 32]>,
    /// The hash of the private passphrase that unlocked the wallet.
    priv_pass_hash: Option<Zeroizing<[u8; 32]>>,
    /// The source of the nonces of values the manager seals after open.
    rng: BoxedRandom,
    /// Set by a coin type upgrade in this process: account 0's keys may
    /// differ between snapshots taken before and after its commit, so
    /// account 0 is cached only from a snapshot that shows the upgraded
    /// state (see [`Inner::load_account_info`]).
    coin_type_upgraded: bool,
    /// The private keys of accounts with their own passphrase that
    /// `UnlockAccount` opened or `SetAccountPassphrase` kept, by account,
    /// each tied to the sealed value it belongs to. All of an account's
    /// entries hold the same extended private key (an account's key never
    /// changes; only its sealing does); there is more than one only after
    /// a passphrase was set in a transaction that may have rolled back.
    /// Emptied by every wallet lock, as Go's `lock` drops every account's
    /// key, and per account by `LockAccount`.
    account_unlocks: BTreeMap<u32, Vec<AccountUnlock>>,
    /// Accounts whose sealed private key this manager rewrote
    /// (`SetAccountPassphrase`), with the value written. Such an account is
    /// cached again only from a snapshot whose row holds that value, so a
    /// snapshot taken before the rewrite committed, or a rewrite that
    /// rolled back, never puts stale key material in the cache; until then
    /// every use reads the row.
    rekeyed: BTreeMap<u32, Vec<u8>>,
}

impl Inner {
    /// Go's `(*Manager).lock` (`wallet/udb/addressmanager.go:357-377`):
    /// drops every account's private key, accounts with their own
    /// passphrase included, the private crypto key and the private master
    /// key's derived key. Dropping wipes them.
    fn lock(&mut self) {
        for info in self.accounts.values_mut() {
            info.key_priv = None;
        }
        self.account_unlocks.clear();
        self.crypto_key_priv = None;
        if let Some(k) = self.master_key_priv.as_mut() {
            k.zero();
        }
        self.locked = true;
        self.priv_pass_hash = None;
    }

    /// The keyed hash of the private passphrase (Go's
    /// `privPassphraseHasher`).
    fn pass_hash(&self, passphrase: &[u8]) -> Zeroizing<[u8; 32]> {
        keyed_hash(&self.hasher_key, &[passphrase])
    }

    /// The keyed hash of an account passphrase (Go's per-account
    /// `uniquePassHasher`, a BLAKE2b-256 keyed with 32 random bytes drawn
    /// at each load of the account and at each `SetAccountPassphrase`).
    ///
    /// The hash only recognises the passphrase of an account that is
    /// already unlocked, in memory, so any keyed PRF with a per-process
    /// key serves (spec 02 section 12 hazard 3): this is the manager's
    /// hasher under a prefix naming the account, which draws nothing.
    fn account_pass_hash(&self, account: u32, passphrase: &[u8]) -> Zeroizing<[u8; 32]> {
        keyed_hash(
            &self.account_hasher_key,
            &[b"account", &account.to_le_bytes(), passphrase],
        )
    }

    /// The private key [`Inner::account_unlocks`] holds for `info`, an
    /// account with its own passphrase whose row holds `info`'s sealed key.
    fn unlocked_entry(&self, account: u32, info: &AccountInfo) -> Option<&AccountUnlock> {
        info.unique_key?;
        self.account_unlocks
            .get(&account)?
            .iter()
            .find(|u| u.sealed == info.key_encrypted)
    }

    /// Whether an account read from a snapshot may enter the cache: not
    /// while a rewrite of its sealed key by this manager is not visible in
    /// the snapshot ([`Inner::rekeyed`]), nor account 0 from a snapshot
    /// taken before a coin type upgrade of this process committed.
    fn cacheable<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
        info: &AccountInfo,
    ) -> Result<bool> {
        if !tx.is_snapshot() {
            return Ok(false);
        }
        if self
            .rekeyed
            .get(&account)
            .is_some_and(|sealed| *sealed != info.key_encrypted)
        {
            return Ok(false);
        }
        if account == DEFAULT_ACCOUNT_NUM && self.coin_type_upgraded {
            let legacy = tx.wallet_row::<CoinTypeLegacy>().map_err(store_err)?;
            if legacy.is_some_and(|k| k.enc_pub.is_some()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether `hash` is the hash of the passphrase that unlocked the
    /// wallet, compared in constant time.
    fn hash_matches(&self, hash: &[u8; 32]) -> bool {
        self.priv_pass_hash
            .as_ref()
            .is_some_and(|h| bool::from(h.ct_eq(&hash[..])))
    }

    /// Go's `loadAccountInfo` (`wallet/udb/addressmanager.go:475-541`).
    ///
    /// An account read from a committed snapshot is cached, as Go caches
    /// it; one read in a write transaction is returned without being
    /// cached, since that transaction may still roll back (QK-0733). What
    /// is cached is what the store held, and two operations change an
    /// existing account's keys. The coin type upgrade replaces account 0's
    /// keys once: it drops the cached entry, and from then on account 0 is
    /// cached only from a snapshot without the legacy coin type key.
    /// `SetAccountPassphrase` reseals an account's private key: it drops
    /// the cached entry, and the account is cached again only from a
    /// snapshot whose row holds the new sealed value ([`Inner::rekeyed`]).
    /// So a snapshot taken before either committed can never put the old
    /// keys back in the cache.
    ///
    /// An account with its own passphrase that is unlocked comes back as a
    /// copy carrying its private key ([`Inner::account_unlocks`]); the
    /// cache itself never holds such a key.
    fn load_account_info<S: StoreRead + ?Sized>(
        &mut self,
        tx: &mut S,
        chain: &ChainParams,
        account: u32,
    ) -> Result<Cow<'_, AccountInfo>> {
        if !self.accounts.contains_key(&account) {
            let mut info = self.read_account_info(tx, chain, account)?;
            if !self.cacheable(tx, account, &info)? {
                if let Some(u) = self.unlocked_entry(account, &info) {
                    info.key_priv = Some(u.key_priv.clone());
                }
                return Ok(Cow::Owned(info));
            }
            self.rekeyed.remove(&account);
            self.accounts.insert(account, info);
        }
        let info = &self.accounts[&account];
        Ok(match self.unlocked_entry(account, info) {
            Some(u) => {
                let mut copy = info.clone();
                copy.key_priv = Some(u.key_priv.clone());
                Cow::Owned(copy)
            }
            None => Cow::Borrowed(info),
        })
    }

    fn read_account_info<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        chain: &ChainParams,
        account: u32,
    ) -> Result<AccountInfo> {
        let row = fetch_account_row(tx, account)?;
        let keys = AccountKeys::decode(&row.keys).map_err(|e| bad_keys(account, e))?;
        let plain = self.crypto_key_pub.decrypt(&keys.enc_pub).map_err(|err| {
            crypto_msg(format!(
                "decrypt account {account} pubkey: {}",
                render_snacl(&err)
            ))
        })?;
        let key_pub = parse_key(plain.as_bytes(), chain).map_err(io_hd)?;
        let mut info = AccountInfo {
            account_type: row.account_type,
            key_encrypted: keys.enc_priv,
            key_pub,
            key_priv: None,
            unique_key: keys.kdf,
        };
        if !self.locked && !info.key_encrypted.is_empty() && info.unique_key.is_none() {
            info.key_priv = Some(self.open_account_priv(chain, account, &info.key_encrypted)?);
        }
        Ok(info)
    }

    /// Opens an account's extended private key sealed by the private
    /// crypto key.
    fn open_account_priv(
        &self,
        chain: &ChainParams,
        account: u32,
        sealed: &[u8],
    ) -> Result<ExtendedKey> {
        // Called only while unlocked, when the key is present.
        let crypto = self
            .crypto_key_priv
            .as_ref()
            .ok_or_else(|| AnyError::from(e!(Kind::Locked)))?;
        let plain = crypto.decrypt(sealed).map_err(|err| {
            crypto_msg(format!(
                "decrypt account {account} privkey: {}",
                render_snacl(&err)
            ))
        })?;
        parse_key(plain.as_bytes(), chain).map_err(io_hd)
    }
}

/// The keyed hash of `parts`, concatenated: HMAC-SHA256 under a 32-byte
/// hasher key, which HMAC zero-pads to SHA-256's 64-byte block; the padded
/// key is passed directly, which makes construction infallible.
fn keyed_hash(key: &[u8; 32], parts: &[&[u8]]) -> Zeroizing<[u8; 32]> {
    let mut padded = Key::<Hmac<Sha256>>::default();
    padded[..32].copy_from_slice(&key[..]);
    let mut mac = Hmac::<Sha256>::new(&padded);
    padded.as_mut_slice().zeroize();
    for p in parts {
        mac.update(p);
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(mac.finalize().as_bytes());
    out
}

/// The address manager of one wallet: dcrwallet's `udb.Manager`.
///
/// Every method that reads or writes the store takes the caller's
/// transaction (Go's bucket or `walletdb` transaction), so a call commits
/// with whatever else the caller does in that transaction; the manager
/// never begins one. In-memory state is behind one reader-writer lock,
/// always taken after the caller's transaction (spec 02 section 7).
pub struct Manager {
    chain: ChainParams,
    /// The processor count new Argon2id parameters record (where Go
    /// records `runtime.NumCPU()` in `kdf.NewArgon2idParams`; see
    /// [`Manager::with_num_cpu`]).
    num_cpu: usize,
    /// Go's `watchingOnly`: written only under the write lock (by
    /// [`Manager::convert_to_watching_only`]) and read under the lock by
    /// every method that takes it, so it cannot change under such a method;
    /// [`Manager::watching_only`] reads it without the lock, as Go's
    /// `WatchingOnly` does.
    watching_only: AtomicBool,
    limits: Limits,
    inner: RwLock<Inner>,
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("chain", &self.chain.name)
            .field("watching_only", &self.is_watching_only())
            .finish_non_exhaustive()
    }
}

/// `errors.E(errors.Crypto, errors.Errorf(...))`.
fn crypto_msg(msg: String) -> AnyError {
    e!(Kind::Crypto, msg).into()
}

/// `errors.E(errors.IO, err)` for an hdkeychain error.
fn io_hd(err: purse_hdkeychain::Error) -> AnyError {
    e!(Kind::Io, AnyError::foreign(err)).into()
}

/// A snacl error as `%v` formats it: rendered with the process-wide
/// separator, as Go's `Error()` reads `errors.Separator`.
fn render_snacl(err: &purse_snacl::Error) -> String {
    err.render(purse_errors::separator())
}

/// Go's `hdkeychain.NewKeyFromString(string(b), params)` for decrypted
/// bytes. Bytes that are not UTF-8 cannot be a base58 string; Go's decoder
/// returns nothing for them, hence `ErrInvalidKeyLen`.
fn parse_key(
    plain: &[u8],
    chain: &ChainParams,
) -> std::result::Result<ExtendedKey, purse_hdkeychain::Error> {
    let s = std::str::from_utf8(plain).map_err(|_| purse_hdkeychain::Error::InvalidKeyLen)?;
    ExtendedKey::new_key_from_string(s, &chain.hd)
}

/// Go's `fetchAccountRow` miss: `NotExist` `no account <n>`.
fn no_account(account: u32) -> AnyError {
    e!(Kind::NotExist, format!("no account {account}")).into()
}

fn fetch_account_row<S: StoreRead + ?Sized>(tx: &mut S, account: u32) -> Result<AccountRow> {
    tx.get::<Accounts>(&AccountKey(account))
        .map_err(store_err)?
        .ok_or_else(|| no_account(account))
}

fn to_parameters(p: &MasterKeyParams) -> Parameters {
    Parameters {
        salt: p.salt,
        digest: p.digest,
        n: p.n.cast_signed(),
        r: p.r.cast_signed(),
        p: p.p.cast_signed(),
    }
}

fn to_stored(p: &Parameters) -> MasterKeyParams {
    MasterKeyParams {
        salt: p.salt,
        digest: p.digest,
        n: p.n.cast_unsigned(),
        r: p.r.cast_unsigned(),
        p: p.p.cast_unsigned(),
    }
}

impl Manager {
    /// Go's `createAddressManager` (`wallet/udb/addressmanager.go:2895-3082`)
    /// as `Initialize` leaves it after its 25 upgrades (spec 02 section
    /// 4.14), written directly in its final form: the master key
    /// parameters, both sealed crypto keys, both coin types' sealed keys,
    /// account 0 under the legacy coin type, the reserved imported
    /// account, and account 0 under the SLIP0044 coin type parked for the
    /// coin type upgrade.
    ///
    /// Randomness is drawn in Go's order and sizes: the public and private
    /// master key salts (32 bytes each), the public and private crypto keys
    /// (32 each), then one 24-byte nonce per sealed value. Nothing is
    /// written unless everything succeeds, and the caller's transaction
    /// rolls back on an error.
    ///
    /// # Errors
    ///
    /// In Go's order: an existing manager, `Exist` `address manager already
    /// exists`; an empty private passphrase, `Invalid` `private passphrase
    /// may not be empty`; [`hd_keys_from_seed`]'s errors; snacl's errors for
    /// a failing random source or scrypt cost over `limits`; the store's.
    pub fn create<R: RandomSource + ?Sized>(
        tx: &mut WriteTx<'_>,
        chain: &ChainParams,
        seed: &[u8],
        pub_pass: &[u8],
        priv_pass: &[u8],
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
        if priv_pass.is_empty() {
            return Err(e!(Kind::Invalid, "private passphrase may not be empty").into());
        }
        let keys = hd_keys_from_seed(seed, chain)?;
        let acct_legacy_pub = keys.account0_legacy.neuter();
        let acct_slip_pub = keys.account0_slip0044.neuter();

        let (n, r, p) = chain.scrypt_cost();
        let master_pub = SecretKey::new_with(pub_pass, n, r, p, rng, limits)?;
        let master_priv = SecretKey::new_with(priv_pass, n, r, p, rng, limits)?;
        let crypto_pub = CryptoKey::generate_with(rng)?;
        let crypto_priv = CryptoKey::generate_with(rng)?;

        let seal = |key: &CryptoKey, what: &str, rng: &mut R, plain: &[u8]| -> Result<Vec<u8>> {
            key.encrypt_with(plain, rng)
                .map_err(|err| crypto_msg(format!("encrypt {what}: {}", render_snacl(&err))))
        };
        let crypto_pub_enc = master_pub
            .encrypt_with(crypto_pub.as_bytes(), rng)
            .map_err(|err| crypto_msg(format!("encrypt crypto pubkey: {}", render_snacl(&err))))?;
        let crypto_priv_enc = master_priv
            .encrypt_with(crypto_priv.as_bytes(), rng)
            .map_err(|err| crypto_msg(format!("encrypt crypto privkey: {}", render_snacl(&err))))?;
        let ct_legacy_pub = seal(
            &crypto_pub,
            "legacy cointype pubkey",
            rng,
            keys.coin_type_legacy.neuter().string().as_bytes(),
        )?;
        let ct_legacy_priv = seal(
            &crypto_priv,
            "legacy cointype privkey",
            rng,
            keys.coin_type_legacy.string().as_bytes(),
        )?;
        let ct_slip_pub = seal(
            &crypto_pub,
            "SLIP0044 cointype pubkey",
            rng,
            keys.coin_type_slip0044.neuter().string().as_bytes(),
        )?;
        let ct_slip_priv = seal(
            &crypto_priv,
            "SLIP0044 cointype privkey",
            rng,
            keys.coin_type_slip0044.string().as_bytes(),
        )?;
        let acct_legacy_pub_enc = seal(
            &crypto_pub,
            "account 0 pubkey",
            rng,
            acct_legacy_pub.string().as_bytes(),
        )?;
        let acct_legacy_priv_enc = seal(
            &crypto_priv,
            "account 0 privkey",
            rng,
            keys.account0_legacy.string().as_bytes(),
        )?;
        let acct_slip_pub_enc = seal(
            &crypto_pub,
            "account 0 pubkey",
            rng,
            acct_slip_pub.string().as_bytes(),
        )?;
        let acct_slip_priv_enc = seal(
            &crypto_priv,
            "account 0 privkey",
            rng,
            keys.account0_slip0044.string().as_bytes(),
        )?;

        let put = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
            tx.put_wallet_row::<LastAccount>(&AccountNumber(DEFAULT_ACCOUNT_NUM))?;
            tx.put_wallet_row::<MasterKeyPriv>(&to_stored(master_priv.parameters()))?;
            tx.put_wallet_row::<MasterKeyPub>(&to_stored(master_pub.parameters()))?;
            tx.put_wallet_row::<CryptoKeyPub>(&SealedKey(crypto_pub_enc))?;
            tx.put_wallet_row::<CryptoKeyPriv>(&SealedKey(crypto_priv_enc))?;
            tx.put_wallet_row::<CoinTypeLegacy>(&CoinTypeKeys {
                enc_pub: Some(ct_legacy_pub),
                enc_priv: Some(ct_legacy_priv),
            })?;
            tx.put_wallet_row::<CoinTypeSlip0044>(&CoinTypeKeys {
                enc_pub: Some(ct_slip_pub),
                enc_priv: Some(ct_slip_priv),
            })?;
            tx.put_wallet_row::<WatchOnly>(&false)?;
            // The reserved imported account: a BIP0044-type row with no
            // keys whose counters, after Initialize's upgrades, read 0
            // (spec 02 section 4.14, quirk 4).
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
                        enc_pub: acct_legacy_pub_enc,
                        enc_priv: acct_legacy_priv_enc,
                        kdf: None,
                    }
                    .encode(),
                },
            )?;
            tx.put_wallet_row::<Slip0044Account0>(&Slip0044Acct0Row {
                enc_pub: acct_slip_pub_enc,
                enc_priv: acct_slip_priv_enc,
                next_external: 0,
                next_internal: 0,
                name: DEFAULT_ACCOUNT_NAME.as_bytes().to_vec(),
            })
        };
        put(tx).map_err(store_err)
    }

    /// Go's `loadManager` (`wallet/udb/addressmanager.go:2733-2796`): opens
    /// the manager of the store, locked, with an empty account cache.
    ///
    /// The public master key is derived from `pub_pass` and the public
    /// crypto key opened with it; both stay in memory until the manager is
    /// dropped. The private master key's parameters are read but nothing is
    /// derived from them. The passphrase hasher's 32-byte key is drawn from
    /// `rng`, which the manager keeps for its later draws.
    ///
    /// # Errors
    ///
    /// In Go's order: no `watch_only` row, `I/O error: bad watching-only
    /// flag len 0`; no public master key parameters, `I/O error: missing
    /// master pubkey params`; no sealed public crypto key, `I/O error:
    /// missing encrypted crypto pubkey`; a wallet that is not watching-only
    /// without private master key parameters, `I/O error: unmarshal master
    /// privkey: secretkey.Unmarshal: invalid encoding: bad marshalled data
    /// len 0`; a wrong public passphrase (or any failure deriving the public
    /// master key), the bare `invalid passphrase`; a public crypto key that
    /// does not open, `Crypto` `decrypt crypto pubkey: ...`; the store's.
    pub fn open<S: StoreRead + ?Sized>(
        tx: &mut S,
        chain: &ChainParams,
        pub_pass: &[u8],
        mut rng: BoxedRandom,
        limits: Limits,
    ) -> Result<Manager> {
        let watching_only = tx
            .wallet_row::<WatchOnly>()
            .map_err(store_err)?
            .ok_or_else(|| AnyError::from(e!(Kind::Io, "bad watching-only flag len 0")))?;
        let mpub = tx
            .wallet_row::<MasterKeyPub>()
            .map_err(store_err)?
            .ok_or_else(|| AnyError::from(e!(Kind::Io, "missing master pubkey params")))?;
        let mpriv = tx.wallet_row::<MasterKeyPriv>().map_err(store_err)?;
        let cpub = tx
            .wallet_row::<CryptoKeyPub>()
            .map_err(store_err)?
            .ok_or_else(|| AnyError::from(e!(Kind::Io, "missing encrypted crypto pubkey")))?;
        let cpriv = tx.wallet_row::<CryptoKeyPriv>().map_err(store_err)?;

        let master_key_priv = if watching_only {
            None
        } else {
            match mpriv {
                Some(p) => Some(SecretKey::from_parameters(to_parameters(&p))),
                None => {
                    // Go unmarshals the absent (nil) value and reports its
                    // length; the store's typed row cannot hold any other
                    // malformed value.
                    let err = Parameters::unmarshal(&[])
                        .err()
                        .map(|e| render_snacl(&e))
                        .unwrap_or_default();
                    return Err(e!(Kind::Io, format!("unmarshal master privkey: {err}")).into());
                }
            }
        };

        let mut master_key_pub = SecretKey::from_parameters(to_parameters(&mpub));
        if master_key_pub.derive_key_with(pub_pass, &limits).is_err() {
            return Err(e!(Kind::Passphrase).into());
        }
        let plain = master_key_pub
            .decrypt(&cpub.0)
            .map_err(|err| crypto_msg(format!("decrypt crypto pubkey: {}", render_snacl(&err))))?;
        // Go's CopyBytes: up to 32 bytes into a zero key.
        let mut key_bytes = Zeroizing::new([0u8; 32]);
        let n = plain.len().min(32);
        key_bytes[..n].copy_from_slice(&plain.as_bytes()[..n]);
        let crypto_key_pub = CryptoKey::from_bytes(&key_bytes);

        let mut hasher_key = Zeroizing::new([0u8; 32]);
        rng.fill(&mut hasher_key[..]).map_err(|err| {
            AnyError::from(e!(
                Kind::Other,
                format!("passphrase hasher key: {}", err.message())
            ))
        })?;

        Ok(Manager {
            chain: *chain,
            num_cpu: purse_snacl::num_cpu(),
            watching_only: AtomicBool::new(watching_only),
            limits,
            inner: RwLock::new(Inner {
                locked: true,
                accounts: BTreeMap::new(),
                master_key_pub,
                master_key_priv,
                crypto_key_pub,
                crypto_key_priv_encrypted: cpriv.map(|c| c.0).unwrap_or_default(),
                crypto_key_priv: None,
                account_hasher_key: hasher_key.clone(),
                hasher_key,
                priv_pass_hash: None,
                rng,
                coin_type_upgraded: false,
                account_unlocks: BTreeMap::new(),
                rekeyed: BTreeMap::new(),
            }),
        })
    }

    /// The manager with `n` as the processor count that the Argon2id
    /// parameters of new per-account passphrases record
    /// ([`Manager::set_account_passphrase`],
    /// [`Manager::import_voting_account`]), where Go records
    /// `runtime.NumCPU()`.
    ///
    /// [`Manager::open`] takes the count from `purse_snacl::num_cpu`, which
    /// is `std::thread::available_parallelism`: the CPUs of the process's
    /// affinity mask, further limited by a cgroup CPU quota. Go's
    /// `runtime.NumCPU` counts the affinity mask only, so in a container
    /// with a CPU quota below its CPU count the parameters stored here
    /// record fewer threads than Go's would (PARITY.md, `purse-udb`). The
    /// count is stored with the parameters (truncated to a byte as Go
    /// truncates it, QK-0630) and caps the threads a derivation runs on;
    /// unlocking reads the stored count, and no key changes with it. The
    /// oracle runner sets it to the script's `setup.ncpu`.
    #[must_use]
    pub fn with_num_cpu(mut self, n: usize) -> Self {
        self.num_cpu = n;
        self
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The watching-only flag.
    fn is_watching_only(&self) -> bool {
        self.watching_only.load(Ordering::SeqCst)
    }

    /// Go's `ChainParams`: the parameters the manager was opened with.
    pub fn chain_params(&self) -> &ChainParams {
        &self.chain
    }

    /// Go's `WatchingOnly`.
    pub fn watching_only(&self) -> bool {
        self.is_watching_only()
    }

    /// Go's `IsLocked`: always true for a watching-only wallet, which can
    /// never be unlocked.
    pub fn is_locked(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .locked
    }

    /// Go's `Lock` (`wallet/udb/addressmanager.go:1453-1469`): drops every
    /// private key the manager holds, those of accounts their own
    /// passphrase unlocked included. A wallet that is already locked
    /// refuses and changes nothing, so such accounts stay unlocked.
    ///
    /// # Errors
    ///
    /// A watching-only wallet: the bare `watching only wallet`; a locked
    /// wallet: the bare `wallet locked`.
    pub fn lock(&self) -> Result<()> {
        let mut inner = self.write();
        if self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly).into());
        }
        if inner.locked {
            return Err(e!(Kind::Locked).into());
        }
        inner.lock();
        Ok(())
    }

    /// Go's `Unlock` (`wallet/udb/addressmanager.go:1518-1587`): derives the
    /// private master key from `passphrase`, opens the private crypto key
    /// and every cached account's private key that it seals.
    ///
    /// On an unlocked wallet a matching passphrase changes nothing and a
    /// different one locks the wallet and fails, as Go does (upstream's
    /// `testLockOnBadPassphrase` pins it; spec 02 section 9 item 17). Any
    /// other failure also leaves the wallet locked, and locking, here as in
    /// [`Manager::lock`], also locks every account its own passphrase
    /// unlocked; a successful unlock unlocks none of those. Cached accounts
    /// are opened in ascending account order, where Go ranges over a map;
    /// the order decides only which of several failing accounts is
    /// reported.
    ///
    /// # Errors
    ///
    /// A watching-only wallet: `watching only wallet: cannot unlock watching
    /// wallet`; a different passphrase on an unlocked wallet: the bare
    /// `invalid passphrase`; a wrong passphrase on a locked one:
    /// `secretkey.DeriveKey: invalid passphrase`; a private crypto key or
    /// account key that does not open: `Crypto` `decrypt crypto privkey:
    /// ...` or `decrypt account <n> privkey: ...`; an account key that does
    /// not parse: `IO` with hdkeychain's error.
    pub fn unlock(&self, passphrase: &[u8]) -> Result<()> {
        let mut guard = self.write();
        if self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly, "cannot unlock watching wallet").into());
        }
        let inner = &mut *guard;
        let hash = inner.pass_hash(passphrase);
        if !inner.locked {
            if !inner.hash_matches(&hash) {
                inner.lock();
                return Err(e!(Kind::Passphrase).into());
            }
            return Ok(());
        }
        let result = self.unlock_locked(inner, passphrase);
        match result {
            Ok(()) => {
                inner.locked = false;
                inner.priv_pass_hash = Some(hash);
                Ok(())
            }
            Err(err) => {
                inner.lock();
                Err(err)
            }
        }
    }

    fn unlock_locked(&self, inner: &mut Inner, passphrase: &[u8]) -> Result<()> {
        // Every wallet that is not watching-only has the key.
        let master = inner
            .master_key_priv
            .as_mut()
            .ok_or_else(|| AnyError::from(e!(Kind::WatchingOnly)))?;
        master.derive_key_with(passphrase, &self.limits)?;
        let plain = master
            .decrypt(&inner.crypto_key_priv_encrypted)
            .map_err(|err| crypto_msg(format!("decrypt crypto privkey: {}", render_snacl(&err))))?;
        let mut key_bytes = Zeroizing::new([0u8; 32]);
        let n = plain.len().min(32);
        key_bytes[..n].copy_from_slice(&plain.as_bytes()[..n]);
        inner.crypto_key_priv = Some(CryptoKey::from_bytes(&key_bytes));

        let mut opened = Vec::new();
        for (&account, info) in &inner.accounts {
            if info.key_encrypted.is_empty() || info.unique_key.is_some() {
                continue;
            }
            opened.push((
                account,
                inner.open_account_priv(&self.chain, account, &info.key_encrypted)?,
            ));
        }
        for (account, key) in opened {
            if let Some(info) = inner.accounts.get_mut(&account) {
                info.key_priv = Some(key);
            }
        }
        Ok(())
    }

    /// Go's `UnlockedWithPassphrase` (`wallet/udb/addressmanager.go:1488-1510`):
    /// whether the wallet is unlocked with this passphrase. Changes nothing.
    ///
    /// # Errors
    ///
    /// A watching-only wallet: `watching only wallet: watching wallets can
    /// not be unlocked`; a locked wallet: the bare `wallet locked`; a
    /// different passphrase: the bare `invalid passphrase`.
    pub fn unlocked_with_passphrase(&self, passphrase: &[u8]) -> Result<()> {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        if self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly, "watching wallets can not be unlocked").into());
        }
        if inner.locked {
            return Err(e!(Kind::Locked).into());
        }
        if !inner.hash_matches(&inner.pass_hash(passphrase)) {
            return Err(e!(Kind::Passphrase).into());
        }
        Ok(())
    }

    /// Go's `CoinType` (`wallet/udb/addressmanager.go:714-730`): the legacy
    /// coin type while its public key is stored, else the SLIP0044 one.
    ///
    /// # Errors
    ///
    /// Neither stored (a wallet created from an xpub): `watching only
    /// wallet: watching wallets do not record coin type keys`; the store's.
    pub fn coin_type<S: StoreRead + ?Sized>(&self, tx: &mut S) -> Result<u32> {
        let (legacy, slip0044) = coin_types(&self.chain);
        let has_pub = |k: Option<CoinTypeKeys>| k.is_some_and(|k| k.enc_pub.is_some());
        if has_pub(tx.wallet_row::<CoinTypeLegacy>().map_err(store_err)?) {
            return Ok(legacy);
        }
        if has_pub(tx.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?) {
            return Ok(slip0044);
        }
        Err(e!(
            Kind::WatchingOnly,
            "watching wallets do not record coin type keys"
        )
        .into())
    }

    /// Go's `LastAccount`: the last BIP0044 account created.
    ///
    /// # Errors
    ///
    /// No row: `I/O error: bad last account len 0`; the store's.
    pub fn last_account<S: StoreRead + ?Sized>(&self, tx: &mut S) -> Result<u32> {
        tx.wallet_row::<LastAccount>()
            .map_err(store_err)?
            .map(|a| a.0)
            .ok_or_else(|| e!(Kind::Io, "bad last account len 0").into())
    }

    /// Go's `LastImportedAccount`: the last imported xpub or voting
    /// account, the reserved imported account when none was imported.
    ///
    /// # Errors
    ///
    /// A stored value at or below [`MAX_ACCOUNT_NUM`]: `I/O error: bad
    /// imported xpub account value <n>`; the store's.
    pub fn last_imported_account<S: StoreRead + ?Sized>(&self, tx: &mut S) -> Result<u32> {
        match tx.wallet_row::<LastImportedAccount>().map_err(store_err)? {
            None => Ok(IMPORTED_ADDR_ACCOUNT),
            Some(AccountNumber(a)) if a <= MAX_ACCOUNT_NUM => {
                Err(e!(Kind::Io, format!("bad imported xpub account value {a}")).into())
            }
            Some(AccountNumber(a)) => Ok(a),
        }
    }

    /// Go's `AccountName`: the name of an account.
    ///
    /// # Errors
    ///
    /// No such account: `item does not exist: no account <n>`; the store's.
    pub fn account_name<S: StoreRead + ?Sized>(&self, tx: &mut S, account: u32) -> Result<Vec<u8>> {
        Ok(fetch_account_row(tx, account)?.name)
    }

    /// Go's `ForEachAccount`, collected: every account number in
    /// dcrwallet's order (the byte order of the little-endian number: 0,
    /// 0x80000000, 256, ..., 1, ..., 0x7FFFFFFF), the reserved imported
    /// account included.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn for_each_account<S: StoreRead + ?Sized>(&self, tx: &mut S) -> Result<Vec<u32>> {
        tx.account_numbers().map_err(store_err)
    }

    /// Go's `AccountProperties` (`wallet/udb/addressmanager.go:550-607`).
    ///
    /// For the reserved imported account the name is `imported`, the type
    /// 0 and the four counters 0 whatever is stored, and the imported key
    /// count is the number of its addresses. For any other account the
    /// account is loaded (and cached) and its stored counters reported.
    /// Go takes only the read lock here although loading inserts into the
    /// cache; this takes the write lock (spec 02 section 12 hazard 8).
    ///
    /// # Errors
    ///
    /// Loading the account: `item does not exist: no account <n>` and the
    /// errors of a sealed public key that does not open or parse; the
    /// store's.
    pub fn account_properties<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
    ) -> Result<AccountProperties> {
        let mut inner = self.write();
        let mut props = AccountProperties {
            account_number: account,
            ..AccountProperties::default()
        };
        if account == IMPORTED_ADDR_ACCOUNT {
            props.account_name = IMPORTED_ADDR_ACCOUNT_NAME.as_bytes().to_vec();
            let n = tx
                .count_prefix::<AddrByAcctSha>(&AddrAcctShaKey::account_prefix(account))
                .map_err(store_err)?;
            props.imported_key_count = u32::try_from(n).unwrap_or(u32::MAX);
        } else {
            let info = inner.load_account_info(tx, &self.chain, account)?;
            props.account_type = info.account_type;
            drop(info);
            let row =
                fetch_account_row(tx, account).map_err(|e| AnyError::from(e!(Kind::Io, e)))?;
            props.account_name = row.name;
            props.last_used_external_index = row.last_used[0];
            props.last_used_internal_index = row.last_used[1];
            props.last_returned_external_index = row.last_returned[0];
            props.last_returned_internal_index = row.last_returned[1];
        }
        (props.account_encrypted, props.account_unlocked) =
            Self::has_passphrase(&mut inner, tx, &self.chain, account);
        Ok(props)
    }

    /// Go's `accountHasPassphrase`: a failure to load the account answers
    /// `(false, false)`, as for the reserved imported account.
    fn has_passphrase<S: StoreRead + ?Sized>(
        inner: &mut Inner,
        tx: &mut S,
        chain: &ChainParams,
        account: u32,
    ) -> (bool, bool) {
        match inner.load_account_info(tx, chain, account) {
            Ok(info) => {
                let has = info.unique_key.is_some();
                (has, has && info.key_priv.is_some())
            }
            Err(_) => (false, false),
        }
    }

    /// Go's `AccountHasPassphrase` (`wallet/udb/addressmanager.go:1848-1868`):
    /// whether the account's private key is sealed by its own passphrase
    /// (Argon2id parameters stored with it), and if so whether it is
    /// unlocked ([`Manager::unlock_account`]). An account that does not
    /// load (no such account, the reserved imported account) answers
    /// `(false, false)`.
    pub fn account_has_passphrase<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
    ) -> (bool, bool) {
        let mut inner = self.write();
        Self::has_passphrase(&mut inner, tx, &self.chain, account)
    }

    /// Go's `AccountExtendedPubKey`: a copy of the account's extended
    /// public key (Go returns the cached key itself; a copy keeps a caller
    /// from changing the cache).
    ///
    /// # Errors
    ///
    /// The reserved imported account: `invalid operation: imported account
    /// has no extended pubkey`; loading the account.
    pub fn account_extended_pub_key<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
    ) -> Result<ExtendedKey> {
        if account == IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::Invalid, "imported account has no extended pubkey").into());
        }
        let mut inner = self.write();
        let info = inner.load_account_info(tx, &self.chain, account)?;
        Ok(info.key_pub.clone())
    }

    /// Go's `AccountExtendedPrivKey`: a copy of the account's extended
    /// private key, wiped when dropped.
    ///
    /// # Errors
    ///
    /// The reserved imported account: `invalid operation: imported address
    /// account has no extended privkey`; loading the account; no private key
    /// for an account above the reserved one (an imported xpub account, or
    /// a locked imported voting account): `invalid operation: imported xpub
    /// account has no extended privkey`; any other account while locked:
    /// `wallet locked: unable to access account extended privkey`.
    pub fn account_extended_priv_key<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
    ) -> Result<ExtendedKey> {
        if account == IMPORTED_ADDR_ACCOUNT {
            return Err(e!(
                Kind::Invalid,
                "imported address account has no extended privkey"
            )
            .into());
        }
        let mut inner = self.write();
        let info = inner.load_account_info(tx, &self.chain, account)?;
        match &info.key_priv {
            Some(k) => Ok(k.clone()),
            None if account > IMPORTED_ADDR_ACCOUNT => Err(e!(
                Kind::Invalid,
                "imported xpub account has no extended privkey"
            )
            .into()),
            None => Err(e!(Kind::Locked, "unable to access account extended privkey").into()),
        }
    }

    /// Go's `AccountBranchExtendedPubKey`: the extended public key of an
    /// account's branch. The branch is not checked, as in Go: any
    /// non-hardened index derives, a hardened one fails.
    ///
    /// # Errors
    ///
    /// [`Manager::account_extended_pub_key`]'s; hdkeychain's, unwrapped.
    pub fn account_branch_extended_pub_key<S: StoreRead + ?Sized>(
        &self,
        tx: &mut S,
        account: u32,
        branch: u32,
    ) -> Result<ExtendedKey> {
        self.account_extended_pub_key(tx, account)?
            .child(branch)
            .map_err(hd::hd_err)
    }

    /// Seals `plain` with the public crypto key (Go's
    /// `Manager.Encrypt(CKTPublic, ...)`), drawing the 24-byte nonce from
    /// the manager's random source. Go takes the write lock; so does this,
    /// since the draw changes the source.
    ///
    /// # Errors
    ///
    /// A failing random source: `Crypto` wrapping snacl's error.
    pub fn encrypt_public(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut guard = self.write();
        let Inner {
            crypto_key_pub,
            rng,
            ..
        } = &mut *guard;
        crypto_key_pub
            .encrypt_with(plain, &mut **rng)
            .map_err(|err| e!(Kind::Crypto, err.to_purse_error()).into())
    }

    /// Seals `plain` with the private crypto key (Go's
    /// `Manager.Encrypt(CKTPrivate, ...)`), drawing the nonce from the
    /// manager's random source.
    ///
    /// # Errors
    ///
    /// A locked or watching-only wallet: the bare `wallet locked`; a
    /// failing random source: `Crypto` wrapping snacl's error.
    pub fn encrypt_private(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut guard = self.write();
        let locked = guard.locked || self.is_watching_only();
        let Inner {
            crypto_key_priv,
            rng,
            ..
        } = &mut *guard;
        let key = match (crypto_key_priv.as_ref(), locked) {
            (Some(k), false) => k,
            _ => return Err(e!(Kind::Locked).into()),
        };
        key.encrypt_with(plain, &mut **rng)
            .map_err(|err| e!(Kind::Crypto, err.to_purse_error()).into())
    }

    /// Opens a value sealed by the public crypto key (Go's
    /// `Manager.Decrypt(CKTPublic, ...)`), wiped when dropped.
    ///
    /// # Errors
    ///
    /// `Crypto` wrapping snacl's error, which Go's `errors.E` promotes:
    /// `cryptokey.Decrypt: encryption/decryption error` for a box that does
    /// not authenticate; `cryptokey.Decrypt: encryption/decryption error::
    /// invalid operation: missing nonce` for one shorter than its nonce.
    pub fn decrypt_public(&self, sealed: &[u8]) -> Result<purse_snacl::SecretBytes> {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        inner
            .crypto_key_pub
            .decrypt(sealed)
            .map_err(|err| e!(Kind::Crypto, err.to_purse_error()).into())
    }

    /// Opens a value sealed by the private crypto key (Go's
    /// `Manager.Decrypt(CKTPrivate, ...)`), wiped when dropped.
    ///
    /// # Errors
    ///
    /// A locked or watching-only wallet: the bare `wallet locked` (Go
    /// reports `Locked` for both, spec 02 quirk 41); otherwise as
    /// [`Manager::decrypt_public`].
    pub fn decrypt_private(&self, sealed: &[u8]) -> Result<purse_snacl::SecretBytes> {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        let key = match (
            &inner.crypto_key_priv,
            inner.locked || self.is_watching_only(),
        ) {
            (Some(k), false) => k,
            _ => return Err(e!(Kind::Locked).into()),
        };
        key.decrypt(sealed)
            .map_err(|err| e!(Kind::Crypto, err.to_purse_error()).into())
    }
}

/// Writes an account row and its name index entry.
fn put_account(tx: &mut WriteTx<'_>, account: u32, row: &AccountRow) -> purse_store::Result<()> {
    tx.put::<Accounts>(&AccountKey(account), row)?;
    tx.put::<AccountNames>(&AccountName(row.name.clone()), &AccountNumber(account))
}

#[cfg(test)]
mod tests {
    //! Lock erases secrets: after every step of a random sequence of the
    //! operations that load, open or seal keys, a locked manager holds no
    //! private key, no private crypto key, no passphrase hash and a zeroed
    //! private master key, and an unlocked one holds the private key of
    //! every cached account the wallet passphrase protects; the cache never
    //! holds the key of an account with its own passphrase, and a wallet
    //! lock (or a failed unlock) drops every key such a passphrase opened;
    //! a private passphrase change on a locked manager leaves no derived
    //! master key, and on an unlocked one the new key and hash, and one
    //! whose commit's outcome is unknown keeps the invariant whatever
    //! another thread did while the manager's lock was released.
    //! This reads the manager's private state, which no public method
    //! shows.

    #![allow(clippy::arithmetic_side_effects, reason = "test code: small counts")]

    use super::*;
    use purse_snacl::RandomSourceError;
    use purse_store::Store;
    use purse_testutil::SplitMix64;

    fn source(rng: &mut SplitMix64) -> BoxedRandom {
        let mut r = SplitMix64(rng.next_u64());
        Box::new(
            move |buf: &mut [u8]| -> std::result::Result<(), RandomSourceError> {
                r.fill(buf);
                Ok(())
            },
        )
    }

    /// Checks the invariant; returns whether the manager is locked and how
    /// many accounts it caches.
    fn check(m: &Manager, what: &str) -> (bool, usize) {
        let inner = m.inner.read().unwrap_or_else(PoisonError::into_inner);
        let master_zero = inner
            .master_key_priv
            .as_ref()
            .is_none_or(|k| k.key().as_bytes().iter().all(|b| *b == 0));
        if inner.locked {
            assert!(inner.crypto_key_priv.is_none(), "{what}: crypto key kept");
            assert!(inner.priv_pass_hash.is_none(), "{what}: hash kept");
            assert!(master_zero, "{what}: master key kept");
            for (n, info) in &inner.accounts {
                assert!(info.key_priv.is_none(), "{what}: account {n} key kept");
            }
        } else {
            assert!(inner.crypto_key_priv.is_some(), "{what}: no crypto key");
            assert!(inner.priv_pass_hash.is_some(), "{what}: no hash");
            assert!(!master_zero, "{what}: no master key");
            for (n, info) in &inner.accounts {
                let protected = !info.key_encrypted.is_empty() && info.unique_key.is_none();
                assert_eq!(
                    info.key_priv.is_some(),
                    protected,
                    "{what}: account {n} key"
                );
            }
        }
        for (n, info) in &inner.accounts {
            if info.unique_key.is_some() {
                assert!(info.key_priv.is_none(), "{what}: account {n} cached key");
            }
        }
        (inner.locked, inner.accounts.len())
    }

    /// Whether the manager holds any key an account passphrase opened.
    fn account_keys_held(m: &Manager) -> bool {
        let inner = m.inner.read().unwrap_or_else(PoisonError::into_inner);
        !inner.account_unlocks.is_empty()
    }

    #[test]
    fn lock_erases_secrets() {
        let mut rng = SplitMix64::from_env("purse-udb-lock-erases");
        let chain = ChainParams::by_name("simnet").expect("simnet");
        let path =
            std::env::temp_dir().join(format!("purse-udb-lock-erases-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // Declared before the store, so dropped after it on every exit
        // path, a failed assertion's unwinding included: the file is closed
        // before it is removed, as Windows requires.
        let _cleanup = RemoveOnDrop(path.clone());
        let store = Store::create(&path).expect("create store");
        let mut src = source(&mut rng);
        let seed = [0x5au8; 32];
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &seed,
                    b"public",
                    b"private",
                    &mut *src,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        let m = {
            let tx = store.read().expect("read");
            Manager::open(
                &mut &tx,
                &chain,
                b"public",
                source(&mut rng),
                Limits::DEFAULT,
            )
            .expect("open")
        };
        let xpub = ExtendedKey::new_master(&[9u8; 32], &chain.hd)
            .expect("master")
            .child(0x8000_0000)
            .expect("child")
            .neuter();
        let mut seen = [0usize; 2];
        // The private passphrase, which the changes below toggle.
        let mut privpass: &[u8] = b"private";
        let mut changed = 0;
        for step in 0..400 {
            let what = format!("step {step}");
            match rng.below(13) {
                0 | 1 => {
                    let _ = m.unlock(privpass);
                }
                12 => {
                    let new: &[u8] = if privpass == b"private" {
                        b"other"
                    } else {
                        b"private"
                    };
                    let private = rng.below(3) != 0;
                    let locked = m.is_locked();
                    if private {
                        if m.change_passphrase(&store, privpass, new, true).is_ok() {
                            privpass = new;
                            changed += 1;
                        }
                    } else {
                        m.change_passphrase(&store, b"public", b"public", false)
                            .expect("public change");
                    }
                    assert_eq!(
                        m.is_locked(),
                        locked,
                        "{what}: a change kept the lock state"
                    );
                }
                2 => {
                    let _ = m.unlock(b"wrong");
                    assert!(!account_keys_held(&m), "{what}: a failed unlock");
                }
                3 => {
                    if m.lock().is_ok() {
                        assert!(!account_keys_held(&m), "{what}: lock");
                    }
                }
                10 => {
                    let acct = match rng.below(3) {
                        0 => 0x8000_0000,
                        n => u32::try_from(n - 1).expect("small"),
                    };
                    let pass: &[u8] = if rng.below(4) == 0 { b"" } else { b"p" };
                    let _ = store.update(|tx| {
                        let _ = m.set_account_passphrase(tx, acct, pass);
                        Ok::<_, purse_store::Error>(())
                    });
                }
                11 => {
                    let tx = store.read().expect("read");
                    let acct = u32::try_from(rng.below(2)).expect("small");
                    if rng.below(2) == 0 {
                        let _ = m.unlock_account(&mut &tx, acct, b"p");
                    } else {
                        let _ = m.lock_account(&mut &tx, acct);
                    }
                }
                4 => {
                    let _ = store.update(|tx| {
                        let _ = m.new_account(tx, format!("a{step}").as_bytes());
                        Ok::<_, purse_store::Error>(())
                    });
                }
                5 => {
                    let tx = store.read().expect("read");
                    let acct = match rng.below(4) {
                        0 => 0x8000_0000,
                        n => u32::try_from(n - 1).expect("small"),
                    };
                    let _ = m.account_extended_priv_key(&mut &tx, acct);
                    let _ = m.account_extended_pub_key(&mut &tx, acct);
                }
                6 => {
                    let idx = u32::try_from(rng.below(4)).expect("small");
                    let _ = store.update(|tx| {
                        let _ = m.sync_account_to_addr_index(tx, 0, idx, 0);
                        Ok::<_, purse_store::Error>(())
                    });
                    let tx = store.read().expect("read");
                    let child = m
                        .account_branch_extended_pub_key(&mut &tx, 0, 0)
                        .expect("branch")
                        .child(idx)
                        .expect("child");
                    let addr =
                        dcroxide_txscript::stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(
                            &dcroxide_txscript::stdaddr::hash160(child.serialized_pub_key()),
                            &chain,
                        )
                        .expect("p2pkh");
                    let _ = m.private_key(&mut &tx, &addr);
                }
                7 => {
                    let _ = store.update(|tx| {
                        let _ = m.upgrade_to_slip0044_coin_type(tx);
                        Ok::<_, purse_store::Error>(())
                    });
                }
                8 => {
                    let _ = m.encrypt_private(b"x");
                    let tx = store.read().expect("read");
                    let _ = m.coin_type_priv_key(&mut &tx);
                }
                _ => {
                    let _ = store.update(|tx| {
                        let _ = m.import_xpub_account(tx, format!("x{step}").as_bytes(), &xpub);
                        Ok::<_, purse_store::Error>(())
                    });
                    let tx = store.read().expect("read");
                    let _ = m.account_properties(&mut &tx, 0x8000_0000);
                }
            }
            let (locked, cached) = check(&m, &what);
            if cached >= 3 {
                seen[usize::from(locked)] += 1;
            }
        }
        // Both states were reached with several accounts cached, account
        // passphrases were set and the private passphrase was changed.
        assert!(seen.iter().all(|n| *n > 10), "{seen:?}");
        assert!(changed > 5, "{changed}");
        let tx = store.read().expect("read");
        let with_pass = (0..3)
            .filter(|n| m.account_has_passphrase(&mut &tx, *n).0)
            .count();
        eprintln!(
            "cached-account steps (unlocked, locked): {seen:?}; accounts with a passphrase: {with_pass}"
        );
    }

    /// A conversion to watching-only, from a manager with cached accounts
    /// and accounts unlocked by their own passphrases, leaves no secret in
    /// memory: no private key (those passphrases opened included, which
    /// Go keeps while its wallet is locked, QK-0766), no private crypto key
    /// or its sealed copy, no private master key, no passphrase hash. The cached accounts keep the sealed private keys the store
    /// keeps (QK-0761, QK-0764), sealed by the deleted crypto key.
    #[test]
    fn conversion_erases_secrets() {
        let mut rng = SplitMix64::from_env("purse-udb-conversion-erases");
        let chain = ChainParams::by_name("simnet").expect("simnet");
        let path = std::env::temp_dir().join(format!(
            "purse-udb-conversion-erases-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _cleanup = RemoveOnDrop(path.clone());
        let store = Store::create(&path).expect("create store");
        let mut src = source(&mut rng);
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &[0x6bu8; 32],
                    b"public",
                    b"private",
                    &mut *src,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        let open = |rng: &mut SplitMix64| {
            let tx = store.read().expect("read");
            Manager::open(&mut &tx, &chain, b"public", source(rng), Limits::DEFAULT).expect("open")
        };
        let m = open(&mut rng);
        m.unlock(b"private").expect("unlock");
        store
            .update(|tx| {
                m.new_account(tx, b"one").expect("new account");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        {
            let tx = store.read().expect("read");
            for n in [0, 1] {
                m.account_extended_priv_key(&mut &tx, n).expect("xpriv");
            }
        }
        // Keys opened by account passphrases while the wallet is locked,
        // which Go's conversion keeps (QK-0766).
        let voting = ExtendedKey::new_master(&[0x7cu8; 32], &chain.hd)
            .expect("master")
            .child(0x8000_0003)
            .expect("child");
        store
            .update(|tx| {
                m.set_account_passphrase(tx, 1, b"p").expect("set");
                m.import_voting_account(tx, &voting, b"v", b"vote")
                    .expect("voting");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        m.lock().expect("lock");
        {
            let tx = store.read().expect("read");
            m.unlock_account(&mut &tx, 1, b"p").expect("unlock account");
            m.unlock_account(&mut &tx, 0x8000_0000, b"v")
                .expect("unlock voting account");
        }
        assert!(account_keys_held(&m));
        m.convert_to_watching_only(&store).expect("convert");
        for (what, m) in [("live", m), ("reopened", open(&mut rng))] {
            let inner = m.inner.read().unwrap_or_else(PoisonError::into_inner);
            assert!(m.watching_only(), "{what}");
            assert!(inner.locked, "{what}");
            assert!(inner.master_key_priv.is_none(), "{what}");
            assert!(inner.crypto_key_priv.is_none(), "{what}");
            assert!(inner.crypto_key_priv_encrypted.is_empty(), "{what}");
            assert!(inner.priv_pass_hash.is_none(), "{what}");
            assert!(inner.account_unlocks.is_empty(), "{what}");
            for (n, info) in &inner.accounts {
                assert!(info.key_priv.is_none(), "{what}: account {n}");
            }
        }
    }

    /// A private passphrase change whose commit's outcome is unknown is
    /// installed after the manager's lock has been released with the
    /// transaction and taken again, so another thread can lock or unlock
    /// the manager in the gap. Whatever happened there, the installed state
    /// keeps the invariant [`lock_erases_secrets`] checks, and the new
    /// passphrase, and not the old, unlocks the manager afterwards. The
    /// store cannot be made to report an unknown outcome from outside
    /// `purse-store`, so this drives the two halves of
    /// `change_passphrase` (staging under the lock, the install after it)
    /// with each gap's operation between them; the store itself is not
    /// written, and every check reads memory only.
    #[test]
    fn an_unconfirmed_change_fits_the_lock_state_it_finds() {
        let mut rng = SplitMix64::from_env("purse-udb-unconfirmed-change");
        let chain = ChainParams::by_name("simnet").expect("simnet");
        let path = std::env::temp_dir().join(format!(
            "purse-udb-unconfirmed-change-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let _cleanup = RemoveOnDrop(path.clone());
        let store = Store::create(&path).expect("create store");
        let mut src = source(&mut rng);
        store
            .update(|tx| {
                Manager::create(
                    tx,
                    &chain,
                    &[0x3du8; 32],
                    b"public",
                    b"private",
                    &mut *src,
                    &Limits::DEFAULT,
                )
                .expect("create");
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        // (locked when staged, what the gap does, locked after the install)
        let cases: [(bool, Option<bool>, bool); 4] = [
            // No race: installed as staged.
            (false, None, false),
            (true, None, true),
            // Locked in the gap: Go's derived key and hash would be left in
            // a locked manager.
            (false, Some(true), true),
            // Unlocked with the old passphrase in the gap: a zeroed key and
            // no hash would be left in an unlocked manager.
            (true, Some(false), true),
        ];
        for (staged_locked, gap, locked_after) in cases {
            let what = format!("staged locked {staged_locked}, gap {gap:?}");
            let m = {
                let tx = store.read().expect("read");
                Manager::open(
                    &mut &tx,
                    &chain,
                    b"public",
                    source(&mut rng),
                    Limits::DEFAULT,
                )
                .expect("open")
            };
            // Account 0 cached, so the check covers an account's key.
            {
                let tx = store.read().expect("read");
                m.account_extended_pub_key(&mut &tx, 0).expect("xpub");
            }
            if !staged_locked {
                m.unlock(b"private").expect("unlock");
            }
            let staged = {
                let mut guard = m.write();
                m.stage_change(&mut guard, b"private", b"new", true)
                    .expect("stage")
            };
            match gap {
                Some(true) => m.lock().expect("lock in the gap"),
                Some(false) => m.unlock(b"private").expect("unlock in the gap"),
                None => {}
            }
            {
                let mut guard = m.write();
                m.install_unconfirmed(&mut guard, staged);
            }
            let (locked, _) = check(&m, &what);
            assert_eq!(locked, locked_after, "{what}");
            if locked {
                // `Lock` refuses only because the manager is locked, with
                // nothing left to erase.
                assert!(m.lock().is_err(), "{what}");
                assert!(m.unlock(b"private").is_err(), "{what}: old passphrase");
                m.unlock(b"new").expect("unlock with the new passphrase");
            }
            m.unlocked_with_passphrase(b"new")
                .unwrap_or_else(|err| panic!("{what}: {err}"));
            assert!(m.unlocked_with_passphrase(b"private").is_err(), "{what}");
            check(&m, &what);
            m.lock().expect("lock");
            check(&m, &what);
        }
    }

    /// Removes its file when dropped. A unit test has no
    /// `CARGO_TARGET_TMPDIR`, so [`lock_erases_secrets`] writes its store
    /// under the system's temporary directory and must not leave it there,
    /// whether it passes or fails.
    struct RemoveOnDrop(std::path::PathBuf);

    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
