// Copyright (c) 2014 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Accounts: `NewAccount`, `RenameAccount`, `LookupAccount` and
//! `ImportXpubAccount` of `udb.Manager` (`wallet/udb/addressmanager.go`;
//! spec 02 sections 5.12 to 5.14).

use purse_errors::{AnyError, Kind, e};
use purse_store::wallet::{CoinTypeLegacy, CoinTypeSlip0044, LastAccount, LastImportedAccount};
use purse_store::{AccountKey, AccountName, AccountNames, AccountNumber, AccountRow, Accounts};
use purse_store::{CoinTypeKeys, WriteTx};

use super::rows::{AccountKeys, StoreRead, store_err};
use super::{
    ACCOUNT_TYPE_BIP0044, ExtendedKey, IMPORTED_ADDR_ACCOUNT, MAX_ACCOUNT_NUM, Manager, NO_INDEX,
    Result, crypto_msg, derive_account_key, fetch_account_row, io_hd, parse_key, put_account,
    render_snacl, validate_account_name,
};

/// Go's `strconv.Quote` (the `%q` verb) of a name's bytes: valid UTF-8 as
/// `dcroxide-dcrjson`'s port quotes it, and each byte that begins no valid
/// encoding as `\xNN`, as Go's `appendQuotedWith` writes it.
pub(crate) fn go_quote_bytes(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len().saturating_add(2));
    out.push('"');
    for chunk in b.utf8_chunks() {
        let q = dcroxide_dcrjson::gojson::go_quote(chunk.valid());
        // The quoted string without its two quote characters.
        out.push_str(q.get(1..q.len().saturating_sub(1)).unwrap_or_default());
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02x}"));
        }
    }
    out.push('"');
    out
}

/// Whether an account is named `name` (Go's `fetchAccountByName` without
/// error).
pub(super) fn name_in_use<S: StoreRead + ?Sized>(tx: &mut S, name: &[u8]) -> Result<bool> {
    Ok(tx
        .get::<AccountNames>(&AccountName(name.to_vec()))
        .map_err(store_err)?
        .is_some())
}

/// `errors.E(errors.Exist, errors.Errorf("account named %q already
/// exists", name))`.
pub(super) fn name_exists(name: &[u8]) -> AnyError {
    e!(
        Kind::Exist,
        format!("account named {} already exists", go_quote_bytes(name))
    )
    .into()
}

impl Manager {
    /// Go's `LookupAccount`: the number of the account named `name`.
    ///
    /// # Errors
    ///
    /// No such account: `item does not exist: no account <quoted name>`
    /// (Go's `%q`); the store's.
    pub fn lookup_account<S: StoreRead + ?Sized>(&self, tx: &mut S, name: &[u8]) -> Result<u32> {
        tx.get::<AccountNames>(&AccountName(name.to_vec()))
            .map_err(store_err)?
            .map(|n| n.0)
            .ok_or_else(|| {
                e!(
                    Kind::NotExist,
                    format!("no account {}", go_quote_bytes(name))
                )
                .into()
            })
    }

    /// Go's `NewAccount` (`wallet/udb/addressmanager.go:2084-2175`): creates
    /// the next BIP0044 account, its keys derived from the active coin
    /// type's private key as parsed from its string (so in BIP32's layout,
    /// whatever account 0 used; QK-0028), and returns its number. The
    /// account is not cached.
    ///
    /// # Errors
    ///
    /// In Go's order: a watching-only wallet, the bare `watching only
    /// wallet`; a locked one, the bare `wallet locked`;
    /// [`validate_account_name`]'s; a name in use, `item already exists:
    /// account named <quoted name> already exists`; no last account row,
    /// `I/O error: bad last account len 0`; no coin type key, `I/O error:
    /// missing encrypted cointype pubkey` (or `privkey`); a coin type key
    /// that does not open, `Crypto` `decrypt cointype privkey: ...`, or
    /// does not parse, `IO` with hdkeychain's error; the account derivation
    /// ([`derive_account_key`]'s: past the last account number, `invalid
    /// operation: account 2147483647`); the store's.
    pub fn new_account(&self, tx: &mut WriteTx<'_>, name: &[u8]) -> Result<u32> {
        let mut guard = self.write();
        let inner = &mut *guard;
        if self.is_watching_only() {
            return Err(e!(Kind::WatchingOnly).into());
        }
        if inner.locked {
            return Err(e!(Kind::Locked).into());
        }
        validate_account_name(name)?;
        if name_in_use(tx, name)? {
            return Err(name_exists(name));
        }
        let account = self.last_account(tx)?.wrapping_add(1);

        // Go's fetchCoinTypeKeys: the legacy keys while the legacy public
        // key is stored, else the SLIP0044 ones.
        let has_pub = |k: &Option<CoinTypeKeys>| k.as_ref().is_some_and(|k| k.enc_pub.is_some());
        let legacy = tx.wallet_row::<CoinTypeLegacy>().map_err(store_err)?;
        let keys = if has_pub(&legacy) {
            legacy
        } else {
            let slip = tx.wallet_row::<CoinTypeSlip0044>().map_err(store_err)?;
            if !has_pub(&slip) {
                return Err(e!(Kind::Io, "missing encrypted cointype pubkey").into());
            }
            slip
        };
        let enc_priv = keys
            .and_then(|k| k.enc_priv)
            .ok_or_else(|| AnyError::from(e!(Kind::Io, "missing encrypted cointype privkey")))?;

        let crypto = inner
            .crypto_key_priv
            .as_ref()
            .ok_or_else(|| AnyError::from(e!(Kind::Locked)))?;
        let plain = crypto.decrypt(&enc_priv).map_err(|err| {
            crypto_msg(format!("decrypt cointype privkey: {}", render_snacl(&err)))
        })?;
        let coin_type_key = parse_key(plain.as_bytes(), &self.chain).map_err(io_hd)?;
        drop(plain);
        let acct_priv = derive_account_key(&coin_type_key, account)?;
        drop(coin_type_key);
        let enc_pub = inner.seal_public(
            acct_priv.neuter().string().as_bytes(),
            "encrypt account pubkey",
        )?;
        let enc_priv = inner.seal_priv(acct_priv.string().as_bytes(), "encrypt account privkey")?;

        tx.put_wallet_row::<LastAccount>(&AccountNumber(account))
            .map_err(store_err)?;
        put_account(
            tx,
            account,
            &AccountRow {
                account_type: ACCOUNT_TYPE_BIP0044,
                last_used: [NO_INDEX, NO_INDEX],
                last_returned: [NO_INDEX, NO_INDEX],
                name: name.to_vec(),
                keys: AccountKeys {
                    enc_pub,
                    enc_priv,
                    kdf: None,
                }
                .encode(),
            },
        )
        .map_err(store_err)?;
        Ok(account)
    }

    /// Go's `RenameAccount` (`wallet/udb/addressmanager.go:2343-2399`):
    /// gives an account a new name. Imported xpub and voting accounts can
    /// be renamed; the reserved imported account cannot.
    ///
    /// The name is checked against the names in use before it is
    /// validated, as in Go (QK-0038): `imported` and the account's current
    /// name give `Exist`. The name in the account's row and the name index
    /// (`account_names`) change together; Go keeps a third copy (its
    /// `acctvars` and `acctididx` copies by number are the row's one name
    /// here) and changes all three.
    ///
    /// # Errors
    ///
    /// In Go's order: the reserved imported account, `invalid operation:
    /// reserved account`; a name in use, `item already exists: account
    /// named <quoted name> already exists`; [`validate_account_name`]'s; no
    /// such account, `item does not exist: no account <n>`; the store's.
    pub fn rename_account(&self, tx: &mut WriteTx<'_>, account: u32, name: &[u8]) -> Result<()> {
        if account == IMPORTED_ADDR_ACCOUNT {
            return Err(e!(Kind::Invalid, "reserved account").into());
        }
        if name_in_use(tx, name)? {
            return Err(name_exists(name));
        }
        validate_account_name(name)?;
        let mut row = fetch_account_row(tx, account)?;
        let old = std::mem::replace(&mut row.name, name.to_vec());
        let put = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
            tx.put::<Accounts>(&AccountKey(account), &row)?;
            tx.delete::<AccountNames>(&AccountName(old))?;
            tx.put::<AccountNames>(&AccountName(name.to_vec()), &AccountNumber(account))
        };
        put(tx).map_err(store_err)
    }

    /// Go's `ImportXpubAccount` (`wallet/udb/addressmanager.go:1384-1436`):
    /// records an extended key as the next imported account (the first is
    /// 0x80000000), with no private key. As in Go, the key is not checked:
    /// neither that it is public nor its network (QK-0033), and the wallet
    /// may be locked or watching-only.
    ///
    /// # Errors
    ///
    /// In Go's order: [`validate_account_name`]'s; a name in use, `item
    /// already exists: account name in use`; [`Manager::last_imported_account`]'s;
    /// no account number left, `invalid operation: exhausted possible
    /// imported accounts`; the store's.
    pub fn import_xpub_account(
        &self,
        tx: &mut WriteTx<'_>,
        name: &[u8],
        xpub: &ExtendedKey,
    ) -> Result<()> {
        let mut inner = self.write();
        validate_account_name(name)?;
        if name_in_use(tx, name)? {
            return Err(e!(Kind::Exist, "account name in use").into());
        }
        let account = self.last_imported_account(tx)?.wrapping_add(1);
        if account < MAX_ACCOUNT_NUM {
            return Err(e!(Kind::Invalid, "exhausted possible imported accounts").into());
        }
        let enc_pub = inner.seal_public(xpub.string().as_bytes(), "encrypt account pubkey")?;
        drop(inner);
        put_account(
            tx,
            account,
            &AccountRow {
                account_type: ACCOUNT_TYPE_BIP0044,
                last_used: [NO_INDEX, NO_INDEX],
                last_returned: [NO_INDEX, NO_INDEX],
                name: name.to_vec(),
                keys: AccountKeys {
                    enc_pub,
                    enc_priv: Vec::new(),
                    kdf: None,
                }
                .encode(),
            },
        )
        .map_err(store_err)?;
        tx.put_wallet_row::<LastImportedAccount>(&AccountNumber(account))
            .map_err(store_err)
    }
}

#[cfg(test)]
mod tests {
    use super::go_quote_bytes;

    #[test]
    fn quotes_as_go_does() {
        // Go: fmt.Sprintf("%q", s) for each.
        for (input, want) in [
            (&b"default"[..], r#""default""#),
            (b"", r#""""#),
            (b"a\"b\\c", r#""a\"b\\c""#),
            (b"tab\there\n", r#""tab\there\n""#),
            (b"\x00\x7f", r#""\x00\x7f""#),
            (b"\xff", r#""\xff""#),
            (b"\xe2\x82a", r#""\xe2\x82a""#),
            ("pässwörd ✓".as_bytes(), "\"pässwörd ✓\""),
            ("\u{a0}".as_bytes(), "\"\\u00a0\""),
            ("a\u{a0}b\u{1}".as_bytes(), "\"a\\u00a0b\\x01\""),
        ] {
            assert_eq!(go_quote_bytes(input), want, "{input:?}");
        }
    }

    /// The generator's last account name (`tests/support/generator.rs`,
    /// `account_name`) tells Go's `%q` apart from Rust's `Debug`: Go
    /// writes `\u00a0` and `\x01` (as in the frozen generated vectors),
    /// Rust `\u{a0}` and `\u{1}`.
    #[test]
    fn the_generators_name_separates_go_quoting_from_debug() {
        let name = "a\u{a0}b\u{1}";
        let go = go_quote_bytes(name.as_bytes());
        assert_eq!(go, "\"a\\u00a0b\\x01\"");
        assert_eq!(format!("{name:?}"), "\"a\\u{a0}b\\u{1}\"");
        assert_ne!(go, format!("{name:?}"));
        assert!(!go.contains('\u{a0}'), "Go escapes U+00A0");
    }
}
