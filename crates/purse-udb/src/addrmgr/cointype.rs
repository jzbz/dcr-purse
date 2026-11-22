// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The coin type: its private key and the SLIP0044 upgrade
//! (`wallet/udb/addressmanager.go:667-866`, spec 02 section 5.10).

use purse_errors::{AnyError, Kind, e};
use purse_store::wallet::{CoinTypeLegacy, CoinTypeSlip0044, Slip0044Account0};
use purse_store::{AccountRow, WriteTx};

use super::addresses::rederive_account_rows;
use super::rows::store_err;
use super::{
    ACCOUNT_TYPE_BIP0044, AccountKeys, DEFAULT_ACCOUNT_NUM, ExtendedKey, Manager, Result,
    StoreRead, coin_types, crypto_msg, parse_key, put_account, render_snacl,
};

/// Go's `fetchCoinTypeKeys` (`wallet/udb/addressdb.go:367-391`): the
/// sealed pair of the active coin type, the legacy one while its public
/// half is stored.
fn fetch_coin_type_keys<S: StoreRead + ?Sized>(tx: &mut S) -> Result<(Vec<u8>, Vec<u8>)> {
    let legacy = tx.wallet_row::<CoinTypeLegacy>().map_err(store_err)?;
    let (enc_pub, enc_priv) = match legacy {
        Some(k) if k.enc_pub.is_some() => (k.enc_pub, k.enc_priv),
        _ => {
            let slip = tx.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?;
            match slip {
                Some(k) => (k.enc_pub, k.enc_priv),
                None => (None, None),
            }
        }
    };
    let enc_pub =
        enc_pub.ok_or_else(|| AnyError::from(e!(Kind::Io, "missing encrypted cointype pubkey")))?;
    let enc_priv = enc_priv
        .ok_or_else(|| AnyError::from(e!(Kind::Io, "missing encrypted cointype privkey")))?;
    Ok((enc_pub, enc_priv))
}

impl Manager {
    /// Go's `CoinTypePrivKey` (`wallet/udb/addressmanager.go:667-693`): the
    /// private key of the active coin type, `m/44'/<coin type>'`, parsed
    /// from its stored string (so a key stored stripped is the 32-byte
    /// key, QK-0028), wiped when dropped.
    ///
    /// # Errors
    ///
    /// A locked wallet: the bare `wallet locked` (a watching-only wallet is
    /// always locked, so Go's `WatchingOnly` check after it is never
    /// reached); no sealed coin type key: `I/O error: missing encrypted
    /// cointype pubkey` or `privkey`; a sealed key that does not open:
    /// `Crypto` `decrypt cointype privkey: ...`; one that does not parse:
    /// `IO` with hdkeychain's error; the store's.
    pub fn coin_type_priv_key<S: StoreRead + ?Sized>(&self, tx: &mut S) -> Result<ExtendedKey> {
        let inner = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.locked {
            return Err(e!(Kind::Locked).into());
        }
        if self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly).into());
        }
        let (_, enc_priv) = fetch_coin_type_keys(tx)?;
        let crypto = inner
            .crypto_key_priv
            .as_ref()
            .ok_or_else(|| AnyError::from(e!(Kind::Locked)))?;
        let plain = crypto.decrypt(&enc_priv).map_err(|err| {
            crypto_msg(format!("decrypt cointype privkey: {}", render_snacl(&err)))
        })?;
        parse_key(plain.as_bytes(), &self.chain).map_err(super::io_hd)
    }

    /// Go's `UpgradeToSLIP0044CoinType` (`wallet/udb/addressmanager.go:736-866`):
    /// moves a wallet on the legacy coin type that has handed out no
    /// address to the SLIP0044 coin type. The legacy coin type keys are
    /// deleted, account 0 is rewritten with the SLIP0044 account 0 keys
    /// parked at creation (keeping its name), and the parked row is
    /// deleted.
    ///
    /// Account 0's four counters become 0, not "none", as in Go: the parked
    /// row is in dcrwallet's version 1 layout, which has no such fields,
    /// and Go copies the unset zeros (QK-0027, reproduced). Child 0 of
    /// either branch is then never handed out.
    ///
    /// On regnet both coin types are 1, so `CoinType` reports the legacy
    /// one after an upgrade too and a second upgrade gets past the first
    /// check, as in Go (QK-0046); it then fails on the zeroed counters.
    ///
    /// The address rows account 0 recorded under the legacy coin type
    /// (discovery records them up to the gap limit before it upgrades) are
    /// replaced by the SLIP0044 children at the same paths, where Go leaves
    /// them, so that Go claims each legacy address and signs for it with
    /// another address's key (QK-0760; ADR-0012 Rule 7; PARITY.md,
    /// `purse-udb`). After the upgrade a legacy address is no address of
    /// the wallet, and the SLIP0044 child at a recorded path is. A payment
    /// to a legacy address that an earlier instance of the seed handed out
    /// is then neither recognised nor spendable until the seed is restored
    /// again, as in Go, whose filter misses it too; keeping the legacy keys
    /// and rows instead is open for the owner (QK-0760, residual case).
    /// On regnet, where both coin types are 1 and the two account keys are
    /// the same (QK-0046), the rows already derive from the new key and are
    /// left as in Go.
    ///
    /// Account 0 is written without Argon2id parameters: an account 0 given
    /// its own passphrase is protected by the wallet passphrase afterwards,
    /// as the parked key is sealed. Go leaves the parameters, after which no
    /// passphrase opens account 0 and its funds cannot be spent after a
    /// restart (QK-0767, ADR-0012 Rule 7; PARITY.md, `purse-udb`).
    ///
    /// Go replaces account 0's cached keys in the manager's memory at the
    /// end of the call, before the caller's transaction commits. Here the
    /// cached entry is dropped instead, and account 0 is loaded again from
    /// the store at its next use (QK-0733): after a rollback memory still
    /// agrees with the store. The new keys are still opened here, with
    /// Go's texts, so a sealed key that does not open fails the upgrade as
    /// in Go.
    ///
    /// # Errors
    ///
    /// In Go's order: [`Manager::coin_type`]'s; a wallet already on the
    /// SLIP0044 coin type: `invalid operation: SLIP0044 coin type upgrade
    /// only possible on legacy coin type wallets`; a SLIP0044 half missing:
    /// `invalid operation: missing keys for SLIP0044 coin type upgrade`;
    /// [`Manager::last_account`]'s and [`Manager::account_properties`]'s;
    /// a last account other than 0, or a returned address on either branch
    /// of account 0: `invalid operation: wallets with returned addresses
    /// may not be upgraded to SLIP0044 coin type` (last-used indexes are not
    /// consulted, as in Go); no parked row: `I/O error: missing SLIP0044
    /// coin type account row`; the new keys not opening: `Crypto` `decrypt
    /// SLIP0044 account 0 xpub: ...` or `xpriv: ...`, or `IO` with
    /// hdkeychain's error; the store's. The caller's transaction must roll
    /// back on an error, as Go's `walletdb.Update` does.
    pub fn upgrade_to_slip0044_coin_type(&self, tx: &mut WriteTx<'_>) -> Result<()> {
        let coin_type = self.coin_type(tx)?;
        let (legacy, slip0044) = coin_types(&self.chain);
        if coin_type != legacy {
            return Err(e!(
                Kind::Invalid,
                "SLIP0044 coin type upgrade only possible on legacy coin type wallets"
            )
            .into());
        }
        let slip = tx.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?;
        if !slip.is_some_and(|k| k.enc_pub.is_some() && k.enc_priv.is_some()) {
            return Err(e!(Kind::Invalid, "missing keys for SLIP0044 coin type upgrade").into());
        }
        let last = self.last_account(tx)?;
        let props = self.account_properties(tx, last)?;
        if last != 0
            || props.last_returned_external_index != u32::MAX
            || props.last_returned_internal_index != u32::MAX
        {
            return Err(e!(
                Kind::Invalid,
                "wallets with returned addresses may not be upgraded to SLIP0044 coin type"
            )
            .into());
        }

        tx.delete_wallet_row::<CoinTypeLegacy>()
            .map_err(store_err)?;
        let parked = tx
            .wallet_row::<Slip0044Account0>()
            .map_err(store_err)?
            .ok_or_else(|| {
                AnyError::from(e!(Kind::Io, "missing SLIP0044 coin type account row"))
            })?;
        // The parked row's type byte is 0 by construction in purse-store's
        // typed row, so Go's "invalid SLIP0044 account 0 row type" check
        // has nothing to refuse.
        let row = AccountRow {
            account_type: ACCOUNT_TYPE_BIP0044,
            // QK-0027: Go copies the version-1 row's unset fields.
            last_used: [0, 0],
            last_returned: [0, 0],
            name: props.account_name,
            // No Argon2id parameters, even where account 0 had its own
            // passphrase: the parked key is sealed by the private crypto
            // key. Go's putNewBIP0044Account leaves the old `kdf-params`,
            // after which no passphrase opens account 0 (QK-0767, ADR-0012
            // Rule 7).
            keys: AccountKeys {
                enc_pub: parked.enc_pub.clone(),
                enc_priv: parked.enc_priv.clone(),
                kdf: None,
            }
            .encode(),
        };
        put_account(tx, DEFAULT_ACCOUNT_NUM, &row).map_err(store_err)?;
        tx.delete_wallet_row::<Slip0044Account0>()
            .map_err(store_err)?;

        let mut inner = self.write();
        let plain = inner
            .crypto_key_pub
            .decrypt(&parked.enc_pub)
            .map_err(|err| {
                crypto_msg(format!(
                    "decrypt SLIP0044 account 0 xpub: {}",
                    render_snacl(&err)
                ))
            })?;
        let xpub = parse_key(plain.as_bytes(), &self.chain).map_err(super::io_hd)?;
        if !inner.locked {
            let crypto = inner
                .crypto_key_priv
                .as_ref()
                .ok_or_else(|| AnyError::from(e!(Kind::Locked)))?;
            let plain = crypto.decrypt(&parked.enc_priv).map_err(|err| {
                crypto_msg(format!(
                    "decrypt SLIP0044 account 0 xpriv: {}",
                    render_snacl(&err)
                ))
            })?;
            parse_key(plain.as_bytes(), &self.chain).map_err(super::io_hd)?;
        }
        // QK-0760 (ADR-0012 Rule 7): the rows recorded under the legacy
        // coin type are derived again from the SLIP0044 account key. Where
        // both coin types are one number (regnet, QK-0046) the two account
        // keys are the same key and every row already derives from it.
        if legacy != slip0044 {
            rederive_account_rows(tx, DEFAULT_ACCOUNT_NUM, &xpub)?;
        }
        inner.accounts.remove(&DEFAULT_ACCOUNT_NUM);
        inner.coin_type_upgraded = true;
        // The upgrade's own rule now decides when account 0 is cached
        // again: a passphrase set on it earlier in this process committed
        // or rolled back before this write transaction began, and the new
        // row replaces whichever it was (without its Argon2id parameters,
        // QK-0767).
        inner.rekeyed.remove(&DEFAULT_ACCOUNT_NUM);
        Ok(())
    }
}
