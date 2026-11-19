// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The tables of ADR-0003 Decision item 2, each with its key and value type.

use crate::codec::Codec;
use crate::keys::{
    AccountKey, AccountName, AccountNumber, AcctHistoryKey, AddrAcctShaKey, AddrPathKey,
    BalanceHeightKey, BalanceKey, CreditKey, Hash32, Hash160, Height, HistoryKey, MetaKey,
    Outpoint, TxRecKey, WalletKey,
};
use crate::values::{
    AccountRow, AddrRow, BalanceRow, BlockRow, CFilterRow, CommitmentRow, CommitmentUnspentRow,
    CreditRow, DebitRow, Empty, HeaderBytes, RawValue, TxMinedRow, TxUnminedRow, UnminedCreditRow,
    UnpublishedFlag, UnspentRow,
};

/// A table of the schema: its name in the file and its key and value
/// types. Implemented by one zero-sized type per table.
pub trait Table: 'static {
    /// The table's identifier.
    const ID: TableId;
    /// The key type.
    type Key: Codec;
    /// The value type.
    type Value: Codec;
}

macro_rules! schema {
    ($(
        $(#[$doc:meta])*
        $ty:ident = $id:ident, $name:literal, $key:ty, $value:ty;
    )*) => {
        /// Every table of the schema, by name. Its discriminant indexes
        /// [`TableId::ALL`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum TableId {
            $(
                #[doc = concat!("`", $name, "`: see [`", stringify!($ty), "`].")]
                $id,
            )*
        }

        impl TableId {
            /// Every table, in declaration order.
            pub const ALL: &'static [TableId] = &[$(TableId::$id),*];

            /// The table's name in the file.
            pub const fn name(self) -> &'static str {
                match self {
                    $(TableId::$id => $name,)*
                }
            }
        }

        $(
            $(#[$doc])*
            #[derive(Clone, Copy, Debug)]
            pub struct $ty;

            impl Table for $ty {
                const ID: TableId = TableId::$id;
                type Key = $key;
                type Value = $value;
            }
        )*
    };
}

schema! {
    /// `meta`: [`MetaKey`] → a value whose type depends on the row (read and
    /// written through the transactions' typed accessors).
    Meta = Meta, "meta", MetaKey, RawValue;
    /// `wallet`: [`WalletKey`] → the address manager's wallet-wide rows,
    /// each with its own value type (read and written through
    /// [`WalletRecord`](crate::wallet::WalletRecord)).
    Wallet = Wallet, "wallet", WalletKey, RawValue;
    /// `accounts`: account number, **little-endian** ([`AccountKey`]) →
    /// [`AccountRow`]. Serves account iteration in dcrwallet's order,
    /// `AccountProperties`, `MarkUsedChildIndex`.
    Accounts = Accounts, "accounts", AccountKey, AccountRow;
    /// `account_names`: name → account number. Serves `LookupAccount`.
    AccountNames = AccountNames, "account_names", AccountName, AccountNumber;
    /// `addr_by_hash`: HASH160 → [`AddrRow`]. Serves `Manager.Address` and
    /// `AddrAccount` by script, and ownership checks on ingest.
    AddrByHash = AddrByHash, "addr_by_hash", Hash160, AddrRow;
    /// `addr_by_path`: [`AddrPathKey`] → HASH160. Serves
    /// `SyncAccountToAddrIndex` without key derivation.
    AddrByPath = AddrByPath, "addr_by_path", AddrPathKey, Hash160;
    /// `addr_by_acct_sha`: [`AddrAcctShaKey`] → HASH160. Serves
    /// `ForEachAccountAddress` order.
    AddrByAcctSha = AddrByAcctSha, "addr_by_acct_sha", AddrAcctShaKey, Hash160;
    /// `headers`: block hash → serialized header. Main-chain and detached
    /// headers alike, as dcrwallet's `h` bucket, which `getblockheader`
    /// reads and nothing prunes (ADR-0003 Open point 5).
    Headers = Headers, "headers", Hash32, HeaderBytes;
    /// `blocks`: height → [`BlockRow`], the main chain only. Serves
    /// `RangeBlocks`, `GetMainChainBlockHashForHeight`, `BlockInMainChain`.
    Blocks = Blocks, "blocks", Height, BlockRow;
    /// `block_by_hash`: block hash → height, main-chain blocks. Serves
    /// `GetBlockMetaForHash`.
    BlockByHash = BlockByHash, "block_by_hash", Hash32, Height;
    /// `cfilters`: block hash → [`CFilterRow`]. Serves
    /// `GetMainChainCFilters`, `CFilterV2`.
    CFilters = CFilters, "cfilters", Hash32, CFilterRow;
    /// `tx_mined`: [`TxRecKey`] → [`TxMinedRow`]. Serves `TxDetails`, `Tx`,
    /// `ExistsTx` (the highest height of a hash).
    TxMined = TxMined, "tx_mined", TxRecKey, TxMinedRow;
    /// `tx_unmined`: transaction hash → [`TxUnminedRow`] (dcrwallet's `m`).
    /// Serves `UnminedTxs` and the unmined batch of `RangeTransactions`.
    TxUnmined = TxUnmined, "tx_unmined", Hash32, TxUnminedRow;
    /// `unpublished`: transaction hash → flag (dcrwallet's `up`). Serves
    /// `Unpublished`, `SetPublished`.
    Unpublished = Unpublished, "unpublished", Hash32, UnpublishedFlag;
    /// `history`: [`HistoryKey`] → transaction hash. Serves
    /// `RangeTransactions` over wallet transactions only; the record is
    /// `tx_mined` at (hash, height).
    History = History, "history", HistoryKey, Hash32;
    /// `acct_history`: [`AcctHistoryKey`] → transaction hash. History by
    /// account (no dcrwallet equivalent): records with a live credit to, or
    /// debit from, the account.
    AcctHistory = AcctHistory, "acct_history", AcctHistoryKey, Hash32;
    /// `credits`: [`CreditKey`] → [`CreditRow`] (dcrwallet's `c`).
    Credits = Credits, "credits", CreditKey, CreditRow;
    /// `credits_invalidated`: as `credits`, for credits of a regular
    /// transaction whose block its child disapproved (dcrwallet's `ic`).
    CreditsInvalidated = CreditsInvalidated, "credits_invalidated", CreditKey, CreditRow;
    /// `unspent`: [`Outpoint`] → [`UnspentRow`]. Exactly the key set of
    /// dcrwallet's `u`, in its order. Serves `UnspentOutputs`,
    /// `ForEachUnspentOutpoint`, `randomUTXO`, `MakeInputSource`.
    Unspent = Unspent, "unspent", Outpoint, UnspentRow;
    /// `unmined_credits`: [`Outpoint`] → [`UnminedCreditRow`] (dcrwallet's
    /// `mc`).
    UnminedCredits = UnminedCredits, "unmined_credits", Outpoint, UnminedCreditRow;
    /// `unmined_inputs`: [`Outpoint`] → spending transaction's hash
    /// (dcrwallet's `mi`): double-spend checks.
    UnminedInputs = UnminedInputs, "unmined_inputs", Outpoint, Hash32;
    /// `debits`: [`CreditKey`] (transaction hash, height, input index) →
    /// [`DebitRow`] (dcrwallet's `d`).
    Debits = Debits, "debits", CreditKey, DebitRow;
    /// `debits_invalidated`: as `debits`, for debits of a disapproved
    /// block's regular transactions (dcrwallet's `id`).
    DebitsInvalidated = DebitsInvalidated, "debits_invalidated", CreditKey, DebitRow;
    /// `tickets`: ticket hash → nothing. Serves `OwnTicket`,
    /// `IterateTickets`, `UnspentTickets`.
    Tickets = Tickets, "tickets", Hash32, Empty;
    /// `commitments`: [`Outpoint`] → [`CommitmentRow`] (dcrwallet's `cmt`).
    Commitments = Commitments, "commitments", Outpoint, CommitmentRow;
    /// `commitments_unspent`: [`Outpoint`] → [`CommitmentUnspentRow`]
    /// (dcrwallet's `cmu`); locked-by-tickets balances.
    CommitmentsUnspent = CommitmentsUnspent, "commitments_unspent", Outpoint, CommitmentUnspentRow;
    /// `balances`: [`BalanceKey`] → [`BalanceRow`]. Serves
    /// `AccountBalance(s)`.
    Balances = Balances, "balances", BalanceKey, BalanceRow;
    /// `balance_heights`: [`BalanceHeightKey`] → [`BalanceRow`], the
    /// depth-dependent categories.
    BalanceHeights = BalanceHeights, "balance_heights", BalanceHeightKey, BalanceRow;
}

impl TableId {
    /// The table's index in [`TableId::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The number of tables.
pub const TABLE_COUNT: usize = TableId::ALL.len();

/// The tables a header-and-filter batch of initial sync may write
/// ([`crate::SyncBatch`]), besides the tip and processed-marker rows of
/// `meta`.
pub const SYNC_TABLES: [TableId; 4] = [
    TableId::Headers,
    TableId::CFilters,
    TableId::Blocks,
    TableId::BlockByHash,
];
