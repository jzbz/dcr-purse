// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Key types: what each table is keyed by, and the bytes that order it.
//!
//! Integers in keys are big-endian, so byte order is numeric order, except
//! where the order a client observes is dcrwallet's little-endian order
//! ([`AccountKey`], [`AddrAcctShaKey`]). Hashes are in internal byte order,
//! not the reversed display order, as dcrwallet stores them (spec 04
//! §5.1 item 1). Every key type's [`Ord`] equals the byte order of its
//! encoding.

use std::cmp::Ordering;

use sha2::{Digest as _, Sha256};

use crate::codec::{ArrayWriter, Codec, DecodeError, Reader};
use crate::values::BalanceClass;

/// A 32-byte hash in internal byte order: a transaction or block hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash32(pub [u8; 32]);

impl Codec for Hash32 {
    type Bytes = [u8; 32];

    fn encode(&self) -> [u8; 32] {
        self.0
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("hash", bytes);
        let h = r.array()?;
        r.finish()?;
        Ok(Hash32(h))
    }
}

/// A 20-byte HASH160 (RIPEMD-160 of BLAKE-256) of a public key or script.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash160(pub [u8; 20]);

impl Codec for Hash160 {
    type Bytes = [u8; 20];

    fn encode(&self) -> [u8; 20] {
        self.0
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("hash160", bytes);
        let h = r.array()?;
        r.finish()?;
        Ok(Hash160(h))
    }
}

/// A block height, big-endian: the key of `blocks`, the value of
/// `block_by_hash`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Height(pub u32);

impl Codec for Height {
    type Bytes = [u8; 4];

    fn encode(&self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("height", bytes);
        let h = r.u32_be()?;
        r.finish()?;
        Ok(Height(h))
    }
}

/// An account number, big-endian: the value of `account_names`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountNumber(pub u32);

impl Codec for AccountNumber {
    type Bytes = [u8; 4];

    fn encode(&self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("account number", bytes);
        let a = r.u32_be()?;
        r.finish()?;
        Ok(AccountNumber(a))
    }
}

/// The key of `accounts`: the account number, **little-endian**.
///
/// dcrwallet keys its account rows with `uint32ToBytes`, little-endian
/// (`wallet/udb/addressdb.go:269`), and lists accounts in the bbolt order
/// of those bytes, so the order a client sees is 0, 0x80000000, 256, …, 1,
/// … (brief §6 Phase 2; spec 04 §5.1). [`Ord`] on this type is that order,
/// not numeric order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccountKey(pub u32);

impl Ord for AccountKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.to_le_bytes().cmp(&other.0.to_le_bytes())
    }
}

impl PartialOrd for AccountKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Codec for AccountKey {
    type Bytes = [u8; 4];

    fn encode(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("account key", bytes);
        let a = r.u32_le()?;
        r.finish()?;
        Ok(AccountKey(a))
    }
}

/// The key of `account_names`: the account name's bytes, as given.
///
/// Names are kept as bytes, not as Rust strings, so that whatever name the
/// address manager accepted round-trips unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountName(pub Vec<u8>);

impl Codec for AccountName {
    type Bytes = Vec<u8>;

    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(AccountName(bytes.to_vec()))
    }
}

/// The key of `addr_by_path`: account (BE) ‖ branch ‖ child index (BE).
///
/// A BIP0044 account's addresses in derivation order; the address
/// manager's gap and sync logic reads it instead of deriving keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddrPathKey {
    /// Account number.
    pub account: u32,
    /// Branch: 0 external, 1 internal.
    pub branch: u8,
    /// Child index.
    pub index: u32,
}

impl Codec for AddrPathKey {
    type Bytes = [u8; 9];

    fn encode(&self) -> [u8; 9] {
        ArrayWriter::new()
            .put(&self.account.to_be_bytes())
            .put(&[self.branch])
            .put(&self.index.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("address path key", bytes);
        let k = AddrPathKey {
            account: r.u32_be()?,
            branch: r.u8()?,
            index: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `addr_by_acct_sha`: account (**LE**) ‖ SHA-256(HASH160).
///
/// dcrwallet's per-account address index `addracctidx` nests one bucket per
/// account, keyed by the little-endian account number, holding SHA-256 of
/// each address's HASH160 (`wallet/udb/addressdb.go:907`, `:1337-1350`,
/// `:1420`). A walk of one account's prefix is therefore
/// `ForEachAccountAddress` order, which a client sees for the imported
/// account (`getaddressesbyaccount "imported"`). [`Ord`] is the byte order:
/// account in little-endian byte order first, then the digest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AddrAcctShaKey {
    /// Account number.
    pub account: u32,
    /// SHA-256 of the address's HASH160.
    pub sha: [u8; 32],
}

impl AddrAcctShaKey {
    /// The key of `hash160` under `account`.
    pub fn for_address(account: u32, hash160: &Hash160) -> AddrAcctShaKey {
        AddrAcctShaKey {
            account,
            sha: Sha256::digest(hash160.0).into(),
        }
    }

    /// The 4-byte prefix shared by every key of `account`.
    pub fn account_prefix(account: u32) -> [u8; 4] {
        account.to_le_bytes()
    }
}

impl Ord for AddrAcctShaKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.encode().cmp(&other.encode())
    }
}

impl PartialOrd for AddrAcctShaKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Codec for AddrAcctShaKey {
    type Bytes = [u8; 36];

    fn encode(&self) -> [u8; 36] {
        ArrayWriter::new()
            .put(&self.account.to_le_bytes())
            .put(&self.sha)
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("account address key", bytes);
        let k = AddrAcctShaKey {
            account: r.u32_le()?,
            sha: r.array()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `tx_mined`: transaction hash ‖ height (BE).
///
/// One hash can have two mined records at once after stake invalidation
/// (ADR-0003 Decision item 5), so records are keyed by hash and block. The
/// last record of a hash's prefix is its highest height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxRecKey {
    /// Transaction hash.
    pub hash: Hash32,
    /// Height of the block that mined it.
    pub height: u32,
}

impl Codec for TxRecKey {
    type Bytes = [u8; 36];

    fn encode(&self) -> [u8; 36] {
        ArrayWriter::new()
            .put(&self.hash.0)
            .put(&self.height.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("transaction record key", bytes);
        let k = TxRecKey {
            hash: Hash32(r.array()?),
            height: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `history`: height (BE) ‖ position in the block (BE).
///
/// Positions are in insertion order, as dcrwallet's block record lists its
/// transactions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryKey {
    /// Block height.
    pub height: u32,
    /// Position of the transaction among the block's wallet transactions.
    pub position: u32,
}

impl Codec for HistoryKey {
    type Bytes = [u8; 8];

    fn encode(&self) -> [u8; 8] {
        ArrayWriter::new()
            .put(&self.height.to_be_bytes())
            .put(&self.position.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("history key", bytes);
        let k = HistoryKey {
            height: r.u32_be()?,
            position: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `acct_history`: account (BE) ‖ height (BE) ‖ position (BE).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AcctHistoryKey {
    /// Account number.
    pub account: u32,
    /// Block height.
    pub height: u32,
    /// Position, as in [`HistoryKey`].
    pub position: u32,
}

impl Codec for AcctHistoryKey {
    type Bytes = [u8; 12];

    fn encode(&self) -> [u8; 12] {
        ArrayWriter::new()
            .put(&self.account.to_be_bytes())
            .put(&self.height.to_be_bytes())
            .put(&self.position.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("account history key", bytes);
        let k = AcctHistoryKey {
            account: r.u32_be()?,
            height: r.u32_be()?,
            position: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `credits`, `credits_invalidated`, `debits` and
/// `debits_invalidated`: transaction hash ‖ height (BE) ‖ index (BE).
///
/// For a credit the index is the output's; for a debit it is the input's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CreditKey {
    /// Transaction hash.
    pub hash: Hash32,
    /// Height of the block that mined the transaction.
    pub height: u32,
    /// Output index (credits) or input index (debits).
    pub index: u32,
}

impl CreditKey {
    /// The outpoint a credit key names.
    pub fn outpoint(&self) -> Outpoint {
        Outpoint {
            hash: self.hash,
            index: self.index,
        }
    }
}

impl Codec for CreditKey {
    type Bytes = [u8; 40];

    fn encode(&self) -> [u8; 40] {
        ArrayWriter::new()
            .put(&self.hash.0)
            .put(&self.height.to_be_bytes())
            .put(&self.index.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("credit key", bytes);
        let k = CreditKey {
            hash: Hash32(r.array()?),
            height: r.u32_be()?,
            index: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// An outpoint: transaction hash ‖ output index (BE), dcrwallet's canonical
/// outpoint (`canonicalOutPoint`, wallet/udb/txdb.go).
///
/// The key of `unspent`, `unmined_credits`, `unmined_inputs`,
/// `commitments` and `commitments_unspent`, which hold exactly the key sets
/// of dcrwallet's `u`, `mc`, `mi`, `cmt` and `cmu` in its key bytes
/// (ADR-0003 Decision item 3): input selection walks them in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Outpoint {
    /// Transaction hash.
    pub hash: Hash32,
    /// Output index.
    pub index: u32,
}

impl Codec for Outpoint {
    type Bytes = [u8; 36];

    fn encode(&self) -> [u8; 36] {
        ArrayWriter::new()
            .put(&self.hash.0)
            .put(&self.index.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("outpoint", bytes);
        let k = Outpoint {
            hash: Hash32(r.array()?),
            index: r.u32_be()?,
        };
        r.finish()?;
        Ok(k)
    }
}

/// The key of `balances`: account (BE) ‖ balance class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BalanceKey {
    /// Account number.
    pub account: u32,
    /// Balance class.
    pub class: BalanceClass,
}

impl Codec for BalanceKey {
    type Bytes = [u8; 5];

    fn encode(&self) -> [u8; 5] {
        ArrayWriter::new()
            .put(&self.account.to_be_bytes())
            .put(&[self.class as u8])
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("balance key", bytes);
        let account = r.u32_be()?;
        let class = BalanceClass::from_u8(r.u8()?).ok_or_else(|| r.field("unknown class"))?;
        r.finish()?;
        Ok(BalanceKey { account, class })
    }
}

/// The key of `balance_heights`: account (BE) ‖ balance class ‖ height
/// (BE), for the classes whose category depends on depth
/// ([`BalanceClass::is_windowed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BalanceHeightKey {
    /// Account number.
    pub account: u32,
    /// Balance class.
    pub class: BalanceClass,
    /// Height of the blocks whose outputs the row sums.
    pub height: u32,
}

impl Codec for BalanceHeightKey {
    type Bytes = [u8; 9];

    fn encode(&self) -> [u8; 9] {
        ArrayWriter::new()
            .put(&self.account.to_be_bytes())
            .put(&[self.class as u8])
            .put(&self.height.to_be_bytes())
            .finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new("balance height key", bytes);
        let account = r.u32_be()?;
        let class = BalanceClass::from_u8(r.u8()?).ok_or_else(|| r.field("unknown class"))?;
        let height = r.u32_be()?;
        r.finish()?;
        Ok(BalanceHeightKey {
            account,
            class,
            height,
        })
    }
}

/// The names of the `meta` table's rows: the store's own (format, schema
/// version) and the chain and sync state of dcrwallet's `wtxmgr` root
/// (`txdb.go:155-163`). Each has its own value type; the typed accessors of
/// the transactions read and write them. Variants are declared in the byte
/// order of their names, so [`Ord`] is that order.
///
/// dcrwallet's `KeyN` of `u` has no row: redb keeps every table's length in
/// its root, so [`ReadTx::len`](crate::ReadTx::len) of `unspent` is the
/// count at no cost, and a counter beside it could only drift (ADR-0003,
/// addendum "Schema version 1 as built").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetaKey {
    /// `birth`: the wallet's birthday state,
    /// [`BirthState`](crate::BirthState) (dcrwallet's `birthstate`,
    /// `SetBirthState`, `txmined.go:335-362`). No row: none set.
    Birth,
    /// `format`: [`FORMAT_MAGIC`](crate::FORMAT_MAGIC), which marks the
    /// file as a dcr-purse wallet store.
    Format,
    /// `have_cfilters`: one byte, 1 once every main-chain block has its
    /// committed filter (dcrwallet's `havecfilters`,
    /// `IsMissingMainChainCFilters`, `txmined.go:400-406`). No row: filters
    /// missing, as in dcrwallet.
    HaveCFilters,
    /// `processed`: the processed-transactions block marker, a block hash
    /// (`ProcessedTxsBlockMarker`), which a rescan advances.
    ProcessedMarker,
    /// `tip`: the main chain tip, [`Tip`](crate::values::Tip).
    Tip,
    /// `version`: the schema version, u32 BE.
    Version,
}

impl MetaKey {
    /// Every meta key.
    pub const ALL: [MetaKey; 6] = [
        MetaKey::Birth,
        MetaKey::Format,
        MetaKey::HaveCFilters,
        MetaKey::ProcessedMarker,
        MetaKey::Tip,
        MetaKey::Version,
    ];

    /// The row's name, its key bytes.
    pub fn name(self) -> &'static [u8] {
        match self {
            MetaKey::Birth => b"birth",
            MetaKey::Format => b"format",
            MetaKey::HaveCFilters => b"have_cfilters",
            MetaKey::ProcessedMarker => b"processed",
            MetaKey::Tip => b"tip",
            MetaKey::Version => b"version",
        }
    }
}

impl Codec for MetaKey {
    type Bytes = &'static [u8];

    fn encode(&self) -> &'static [u8] {
        self.name()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        MetaKey::ALL
            .into_iter()
            .find(|k| k.name() == bytes)
            .ok_or_else(|| DecodeError::field("meta key", "unknown name"))
    }
}

/// The names of the `wallet` table's rows: the address manager's
/// wallet-wide state, dcrwallet's `waddrmgr/main` and `waddrmgr/meta` rows
/// (`addressdb.go:230-261`) that ADR-0001's `[wallet]`, `[masterkeys]`,
/// `[cryptokeys]` and `[cointypes]` sections export. Each name has one
/// value type, fixed by its [`WalletRecord`](crate::WalletRecord); the
/// variants are declared in the byte order of their names, so [`Ord`] is
/// that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalletKey {
    /// `cointype_legacy`: dcrwallet's `ctpub` and `ctpriv`.
    CoinTypeLegacy,
    /// `cointype_slip0044`: dcrwallet's `ctpub-slip0044` and
    /// `ctpriv-slip0044`.
    CoinTypeSlip0044,
    /// `crypto_priv`: dcrwallet's `cpriv`.
    CryptoKeyPriv,
    /// `crypto_pub`: dcrwallet's `cpub`.
    CryptoKeyPub,
    /// `last_account`: dcrwallet's `lastaccount`.
    LastAccount,
    /// `last_imported`: dcrwallet's `lastimportedaccount`.
    LastImportedAccount,
    /// `master_priv`: dcrwallet's `mpriv`.
    MasterKeyPriv,
    /// `master_pub`: dcrwallet's `mpub`.
    MasterKeyPub,
    /// `slip0044_acct0`: dcrwallet's `slip0044acct0`.
    Slip0044Account0,
    /// `watch_only`: dcrwallet's `watchonly`.
    WatchOnly,
}

impl WalletKey {
    /// Every wallet key.
    pub const ALL: [WalletKey; 10] = [
        WalletKey::CoinTypeLegacy,
        WalletKey::CoinTypeSlip0044,
        WalletKey::CryptoKeyPriv,
        WalletKey::CryptoKeyPub,
        WalletKey::LastAccount,
        WalletKey::LastImportedAccount,
        WalletKey::MasterKeyPriv,
        WalletKey::MasterKeyPub,
        WalletKey::Slip0044Account0,
        WalletKey::WatchOnly,
    ];

    /// The row's name, its key bytes.
    pub fn name(self) -> &'static [u8] {
        match self {
            WalletKey::CoinTypeLegacy => b"cointype_legacy",
            WalletKey::CoinTypeSlip0044 => b"cointype_slip0044",
            WalletKey::CryptoKeyPriv => b"crypto_priv",
            WalletKey::CryptoKeyPub => b"crypto_pub",
            WalletKey::LastAccount => b"last_account",
            WalletKey::LastImportedAccount => b"last_imported",
            WalletKey::MasterKeyPriv => b"master_priv",
            WalletKey::MasterKeyPub => b"master_pub",
            WalletKey::Slip0044Account0 => b"slip0044_acct0",
            WalletKey::WatchOnly => b"watch_only",
        }
    }
}

impl Codec for WalletKey {
    type Bytes = &'static [u8];

    fn encode(&self) -> &'static [u8] {
        self.name()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        WalletKey::ALL
            .into_iter()
            .find(|k| k.name() == bytes)
            .ok_or_else(|| DecodeError::field("wallet key", "unknown name"))
    }
}
