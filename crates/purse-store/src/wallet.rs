// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The rows of the `wallet` table: the address manager's wallet-wide
//! state, one type per row.
//!
//! dcrwallet keeps these in `waddrmgr/main` and `waddrmgr/meta`
//! (`wallet/udb/addressdb.go:230-261`); ADR-0001 exports them as
//! `[wallet]`, `[masterkeys]`, `[cryptokeys]` and `[cointypes]`, and its
//! restore contract needs the active coin type among them. The store keeps
//! them typed and leaves their meaning (which coin type is active, how a
//! key is sealed) to the address manager logic (`purse-udb`). A row that is
//! absent is absent in dcrwallet too: no private master or crypto key in a
//! watching-only wallet, no SLIP0044 keys before wallet creation writes
//! them.
//!
//! dcrwallet's `mgrcreated` is not kept: it is written once from udb's own
//! clock and never read (ADR-0001 §6.1 and §11, which then compare under
//! its observable profile).
//!
//! ```
//! use purse_store::wallet::{LastAccount, WatchOnly};
//! use purse_store::{AccountNumber, Error, OpenOptions};
//!
//! # fn main() -> Result<(), Error> {
//! let store = OpenOptions::new()
//!     .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))?;
//! store.update(|tx| {
//!     tx.put_wallet_row::<WatchOnly>(&false)?;
//!     tx.put_wallet_row::<LastAccount>(&AccountNumber(2))
//! })?;
//! let tx = store.read()?;
//! assert_eq!(tx.wallet_row::<LastAccount>()?, Some(AccountNumber(2)));
//! # drop(tx);
//! # store.close()?;
//! # Ok(())
//! # }
//! ```

use crate::codec::Codec;
use crate::keys::{AccountNumber, WalletKey};
use crate::values::{CoinTypeKeys, MasterKeyParams, SealedKey, Slip0044Acct0Row};

/// One row of the `wallet` table: its name and its value type. Implemented
/// by one zero-sized type per row; the transactions read and write a row
/// through it ([`ReadTx::wallet_row`](crate::ReadTx::wallet_row),
/// [`WriteTx::put_wallet_row`](crate::WriteTx::put_wallet_row)).
pub trait WalletRecord: 'static {
    /// The row's name.
    const KEY: WalletKey;
    /// The row's value type.
    type Value: Codec;
}

macro_rules! records {
    ($(
        $(#[$doc:meta])*
        $ty:ident = $key:ident, $value:ty;
    )*) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, Debug)]
        pub struct $ty;

        impl WalletRecord for $ty {
            const KEY: WalletKey = WalletKey::$key;
            type Value = $value;
        }
    )*};
}

records! {
    /// `cointype_legacy`: the legacy coin type's sealed extended keys
    /// (dcrwallet's `ctpub`, `ctpriv`).
    CoinTypeLegacy = CoinTypeLegacy, CoinTypeKeys;
    /// `cointype_slip0044`: the SLIP0044 coin type's sealed extended keys
    /// (dcrwallet's `ctpub-slip0044`, `ctpriv-slip0044`).
    CoinTypeSlip0044 = CoinTypeSlip0044, CoinTypeKeys;
    /// `crypto_priv`: the private-data crypto key, sealed by the private
    /// master key (dcrwallet's `cpriv`).
    CryptoKeyPriv = CryptoKeyPriv, SealedKey;
    /// `crypto_pub`: the public-data crypto key, sealed by the public
    /// master key (dcrwallet's `cpub`).
    CryptoKeyPub = CryptoKeyPub, SealedKey;
    /// `last_account`: the last BIP0044 account created (dcrwallet's
    /// `lastaccount`).
    LastAccount = LastAccount, AccountNumber;
    /// `last_imported`: the last imported extended-key account
    /// (dcrwallet's `lastimportedaccount`; absent is the imported account
    /// 0x7FFFFFFF, as `fetchLastImportedAccount` reads an absent row).
    LastImportedAccount = LastImportedAccount, AccountNumber;
    /// `master_priv`: the private master key's parameters (dcrwallet's
    /// `mpriv`).
    MasterKeyPriv = MasterKeyPriv, MasterKeyParams;
    /// `master_pub`: the public master key's parameters (dcrwallet's
    /// `mpub`).
    MasterKeyPub = MasterKeyPub, MasterKeyParams;
    /// `slip0044_acct0`: account 0 under the SLIP0044 coin type, parked
    /// until the wallet moves to it (dcrwallet's `slip0044acct0`).
    Slip0044Account0 = Slip0044Account0, Slip0044Acct0Row;
    /// `watch_only`: whether the wallet is watching-only (dcrwallet's
    /// `watchonly`).
    WatchOnly = WatchOnly, bool;
}
