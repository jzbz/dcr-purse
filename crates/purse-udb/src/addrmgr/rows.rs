// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! How the address manager's state lies in `purse-store`'s tables: the
//! key material of an account row, read access shared by read and write
//! transactions, and the conversion of store errors.

use purse_errors::{AnyError, Kind, e};
use purse_snacl::{ARGON2ID_MARSHALED_LEN, Argon2idParams};
use purse_store::wallet::WalletRecord;
use purse_store::{AccountKey, Accounts, ReadTx, Table, WriteTx};

/// What an imported address row keeps, the `data` of
/// [`purse_store::AddrRow::Imported`] (opaque to the store).
///
/// Layout: kind u8 (1 key, 2 script) ‖ first length u32 BE ‖ first field ‖
/// second length u32 BE ‖ second field, nothing after. For a key the fields
/// are the serialized public key sealed by the public crypto key and the
/// private key sealed by the private crypto key (empty when none was
/// imported); for a script, its HASH160 sealed by the public crypto key and
/// the script in the clear. These are the raw data of dcrwallet's `addr`
/// rows of types 1 and 2 (`wallet/udb/addressdb.go:1190-1295`, spec 02
/// section 4.10), with big-endian lengths and the kind moved inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportedData {
    /// An imported public key, with or without its private key.
    Key {
        /// The serialized public key, sealed by the public crypto key.
        enc_pub: Vec<u8>,
        /// The 32-byte private key, sealed by the private crypto key;
        /// empty when the import had none.
        enc_priv: Vec<u8>,
    },
    /// An imported script (P2SH).
    Script {
        /// The script's HASH160, sealed by the public crypto key.
        enc_hash: Vec<u8>,
        /// The redeem script, in the clear (dcrwallet stores it so since
        /// its database version 14).
        script: Vec<u8>,
    },
}

/// Why a stored imported address value does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportedDataError {
    /// The value is empty or its kind byte is neither 1 nor 2.
    Kind,
    /// A length field runs past the end of the value.
    Truncated,
    /// Bytes are left after the second field.
    Trailing(usize),
}

impl ImportedData {
    /// The stored form.
    pub fn encode(&self) -> Vec<u8> {
        let (kind, a, b) = match self {
            ImportedData::Key { enc_pub, enc_priv } => (1u8, enc_pub, enc_priv),
            ImportedData::Script { enc_hash, script } => (2u8, enc_hash, script),
        };
        let mut v = Vec::with_capacity(9usize.saturating_add(a.len()).saturating_add(b.len()));
        v.push(kind);
        for f in [a, b] {
            // A field longer than u32::MAX bytes cannot be stored: redb
            // refuses the value before anything is written.
            let len = u32::try_from(f.len()).unwrap_or(u32::MAX);
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(f);
        }
        v
    }

    /// Reads the stored form. Never panics, whatever the input.
    ///
    /// # Errors
    ///
    /// [`ImportedDataError`] for a value no writer of this crate produces.
    pub fn decode(bytes: &[u8]) -> Result<Self, ImportedDataError> {
        let (&kind, mut rest) = bytes.split_first().ok_or(ImportedDataError::Kind)?;
        if kind != 1 && kind != 2 {
            return Err(ImportedDataError::Kind);
        }
        let mut field = || -> Result<Vec<u8>, ImportedDataError> {
            let (len, tail) = rest
                .split_first_chunk::<4>()
                .ok_or(ImportedDataError::Truncated)?;
            let len = usize::try_from(u32::from_be_bytes(*len))
                .map_err(|_| ImportedDataError::Truncated)?;
            if tail.len() < len {
                return Err(ImportedDataError::Truncated);
            }
            let (f, tail) = tail.split_at(len);
            rest = tail;
            Ok(f.to_vec())
        };
        let a = field()?;
        let b = field()?;
        if !rest.is_empty() {
            return Err(ImportedDataError::Trailing(rest.len()));
        }
        Ok(if kind == 1 {
            ImportedData::Key {
                enc_pub: a,
                enc_priv: b,
            }
        } else {
            ImportedData::Script {
                enc_hash: a,
                script: b,
            }
        })
    }
}

/// The key material of an account, the `keys` field of its `accounts` row
/// ([`purse_store::AccountRow::keys`], opaque to the store).
///
/// Layout: public key length u32 BE ‖ sealed extended public key ‖ private
/// key length u32 BE ‖ sealed extended private key ‖ the Argon2id
/// parameters of a per-account passphrase (absent, or 25 bytes in kdf's
/// marshalled layout: salt (16) ‖ time u32 LE ‖ memory u32 LE ‖ threads).
///
/// dcrwallet keeps the two sealed keys in its `acct` row
/// (`wallet/udb/addressdb.go:99-137`) and the parameters in the account's
/// `acctvars/<n>/kdf-params` (`addressdb.go:1058-1068`); here they are one
/// value, so the presence of the parameters and the scheme of the sealed
/// private key change together, as spec 02 section 9 item 27 requires.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountKeys {
    /// The extended public key sealed by the public crypto key; empty for
    /// the reserved imported account.
    pub enc_pub: Vec<u8>,
    /// The extended private key sealed by the private crypto key, or by
    /// the account's own passphrase key when `kdf` is present; empty for
    /// imported xpub accounts and the reserved imported account.
    pub enc_priv: Vec<u8>,
    /// The per-account passphrase's Argon2id parameters.
    pub kdf: Option<Argon2idParams>,
}

/// Why a stored key-material value does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountKeysError {
    /// A length field runs past the end of the value.
    Truncated,
    /// Bytes are left after the private key that are not 25 bytes of
    /// Argon2id parameters.
    Trailing(usize),
}

impl AccountKeys {
    /// The stored form.
    pub fn encode(&self) -> Vec<u8> {
        let kdf_len = if self.kdf.is_some() {
            ARGON2ID_MARSHALED_LEN
        } else {
            0
        };
        let mut v = Vec::with_capacity(
            8usize
                .saturating_add(self.enc_pub.len())
                .saturating_add(self.enc_priv.len())
                .saturating_add(kdf_len),
        );
        for k in [&self.enc_pub, &self.enc_priv] {
            // A sealed key longer than u32::MAX bytes cannot arise: the
            // longest plaintext is a 111-character extended key string.
            let len = u32::try_from(k.len()).unwrap_or(u32::MAX);
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(k);
        }
        if let Some(kdf) = &self.kdf {
            v.extend_from_slice(&kdf.marshal_binary());
        }
        v
    }

    /// Reads the stored form. Never panics, whatever the input.
    ///
    /// # Errors
    ///
    /// [`AccountKeysError`] for a value no writer of this crate produces.
    pub fn decode(bytes: &[u8]) -> Result<Self, AccountKeysError> {
        let mut rest = bytes;
        let mut field = || -> Result<Vec<u8>, AccountKeysError> {
            let (len, tail) = rest
                .split_first_chunk::<4>()
                .ok_or(AccountKeysError::Truncated)?;
            let len = usize::try_from(u32::from_be_bytes(*len))
                .map_err(|_| AccountKeysError::Truncated)?;
            if tail.len() < len {
                return Err(AccountKeysError::Truncated);
            }
            let (f, tail) = tail.split_at(len);
            rest = tail;
            Ok(f.to_vec())
        };
        let enc_pub = field()?;
        let enc_priv = field()?;
        let kdf = match rest.len() {
            0 => None,
            ARGON2ID_MARSHALED_LEN => Some(
                Argon2idParams::unmarshal_binary(rest)
                    .map_err(|_| AccountKeysError::Trailing(rest.len()))?,
            ),
            n => return Err(AccountKeysError::Trailing(n)),
        };
        Ok(AccountKeys {
            enc_pub,
            enc_priv,
            kdf,
        })
    }
}

/// Reads the store from a read or a write transaction, which `purse-store`
/// gives different receivers. Implemented for [`ReadTx`], `&ReadTx` and
/// [`WriteTx`], and for a mutable reference to any of them.
pub trait StoreRead {
    /// The value of a `wallet` row.
    ///
    /// # Errors
    ///
    /// The store's.
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>>;

    /// The value at `key` of table `T`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>>;

    /// Every account number with a row, in the store's key order
    /// (dcrwallet's little-endian `acct` order).
    ///
    /// # Errors
    ///
    /// The store's.
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>>;

    /// The number of rows of table `T` whose key begins with `prefix`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64>;

    /// The number of rows of `table`.
    ///
    /// # Errors
    ///
    /// The store's.
    fn table_len(&mut self, table: purse_store::TableId) -> purse_store::Result<u64>;

    /// Every row of table `T` whose key begins with `prefix`, in key
    /// order.
    ///
    /// # Errors
    ///
    /// The store's.
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>>;

    /// Whether this is a committed snapshot (a read transaction), which
    /// can never be rolled back. The manager caches what it loads only
    /// from a snapshot, so its memory never holds state a caller's write
    /// transaction may still roll back (ADR-0002 decision item 2,
    /// QK-0733).
    fn is_snapshot(&self) -> bool;
}

fn collect_accounts<'a>(
    iter: impl Iterator<Item = purse_store::Result<(AccountKey, purse_store::AccountRow)>> + 'a,
) -> purse_store::Result<Vec<u32>> {
    iter.map(|r| r.map(|(k, _)| k.0)).collect()
}

fn count<'a, T>(
    iter: impl Iterator<Item = purse_store::Result<T>> + 'a,
) -> purse_store::Result<u64> {
    let mut n = 0u64;
    for r in iter {
        r?;
        n = n.saturating_add(1);
    }
    Ok(n)
}

impl StoreRead for &ReadTx {
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>> {
        ReadTx::wallet_row::<R>(self)
    }
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        ReadTx::get::<T>(self, key)
    }
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>> {
        collect_accounts(ReadTx::iter::<Accounts>(self)?)
    }
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64> {
        count(ReadTx::prefix::<T>(self, prefix)?)
    }
    fn table_len(&mut self, table: purse_store::TableId) -> purse_store::Result<u64> {
        ReadTx::len(self, table)
    }
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        ReadTx::prefix::<T>(self, prefix)?.collect()
    }
    fn is_snapshot(&self) -> bool {
        true
    }
}

impl StoreRead for ReadTx {
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>> {
        ReadTx::wallet_row::<R>(self)
    }
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        ReadTx::get::<T>(self, key)
    }
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>> {
        (&*self).account_numbers()
    }
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64> {
        (&*self).count_prefix::<T>(prefix)
    }
    fn table_len(&mut self, table: purse_store::TableId) -> purse_store::Result<u64> {
        (&*self).table_len(table)
    }
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        (&*self).rows_with_prefix::<T>(prefix)
    }
    fn is_snapshot(&self) -> bool {
        true
    }
}

impl StoreRead for WriteTx<'_> {
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>> {
        WriteTx::wallet_row::<R>(self)
    }
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        WriteTx::get::<T>(self, key)
    }
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>> {
        collect_accounts(WriteTx::iter::<Accounts>(self)?)
    }
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64> {
        count(WriteTx::prefix::<T>(self, prefix)?)
    }
    fn table_len(&mut self, table: purse_store::TableId) -> purse_store::Result<u64> {
        WriteTx::len(self, table)
    }
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        WriteTx::prefix::<T>(self, prefix)?.collect()
    }
    fn is_snapshot(&self) -> bool {
        false
    }
}

impl<S: StoreRead + ?Sized> StoreRead for &mut S {
    fn wallet_row<R: WalletRecord>(&mut self) -> purse_store::Result<Option<R::Value>> {
        (**self).wallet_row::<R>()
    }
    fn get<T: Table>(&mut self, key: &T::Key) -> purse_store::Result<Option<T::Value>> {
        (**self).get::<T>(key)
    }
    fn account_numbers(&mut self) -> purse_store::Result<Vec<u32>> {
        (**self).account_numbers()
    }
    fn count_prefix<T: Table>(&mut self, prefix: &[u8]) -> purse_store::Result<u64> {
        (**self).count_prefix::<T>(prefix)
    }
    fn table_len(&mut self, table: purse_store::TableId) -> purse_store::Result<u64> {
        (**self).table_len(table)
    }
    fn rows_with_prefix<T: Table>(
        &mut self,
        prefix: &[u8],
    ) -> purse_store::Result<Vec<(T::Key, T::Value)>> {
        (**self).rows_with_prefix::<T>(prefix)
    }
    fn is_snapshot(&self) -> bool {
        (**self).is_snapshot()
    }
}

/// A store error in the wallet-wide model: the kind of the nearest bbolt
/// condition and the engine's text (ADR-0011 ET-006), where dcrwallet's
/// bucket primitives would return their driver's error.
pub(crate) fn store_err(err: purse_store::Error) -> AnyError {
    err.to_purse_error().into()
}

/// An imported address row whose data does not decode. No writer of this
/// crate produces one; udb panics or fails with `IO` on the nearest
/// malformed `addr` rows (spec 02 section 4.10; QK-0042), so the kind is
/// `IO` and the text is dcr-purse's own (ADR-0012 Rule 2(a)).
pub(crate) fn bad_imported(err: ImportedDataError) -> AnyError {
    let what = match err {
        ImportedDataError::Kind => "unknown kind".to_owned(),
        ImportedDataError::Truncated => "truncated".to_owned(),
        ImportedDataError::Trailing(n) => format!("{n} trailing bytes"),
    };
    e!(Kind::Io, format!("imported address data: {what}")).into()
}

/// An account row whose key material does not decode. No writer of this
/// crate produces one; udb panics or fails with `IO` on the nearest
/// malformed `acct` rows (spec 02 section 4.6.3), so the kind is `IO` and
/// the text is dcr-purse's own (ADR-0012 Rule 2(a)).
pub(crate) fn bad_keys(account: u32, err: AccountKeysError) -> AnyError {
    let what = match err {
        AccountKeysError::Truncated => "truncated".to_owned(),
        AccountKeysError::Trailing(n) => format!("{n} trailing bytes"),
    };
    e!(Kind::Io, format!("account {account} key material: {what}")).into()
}
