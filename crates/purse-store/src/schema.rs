// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The tables of ADR-0003 Decision item 2, each with its key and value type.

use crate::codec::Codec;
use crate::keys::{
    AccountKey, AccountName, AccountNumber, AcctHistoryKey, AddrAcctShaKey, AddrPathKey, AgendaKey,
    BalanceHeightKey, BalanceKey, CreditKey, Hash32, Hash160, Height, HistoryKey, MetaKey,
    MultisigUnspentKey, Outpoint, PiKey, TicketAgendaKey, TxRecKey, VspFeeStatusKey, VspHost,
    VspHostId, VspTSpendKey, VspTreasuryKey, WalletKey,
};
use crate::values::{
    AccountRow, AddrRow, AgendaChoice, BalanceRow, BlockRow, CFilterRow, CommitmentRow,
    CommitmentUnspentRow, CreditRow, DebitRow, Empty, HeaderBytes, MultisigOutRow, RawValue,
    TreasuryVote, TxMinedRow, TxUnminedRow, UnminedCreditRow, UnpublishedFlag, UnspentRow,
    VspPubKeyRow, VspTicketRow,
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
    /// `headers`: main-chain height → serialized header. Exactly the
    /// heights of `blocks`. A header found by hash goes through
    /// `block_by_hash`, then `headers_detached` (the chain methods,
    /// [`crate::chain`]).
    Headers = Headers, "headers", Height, HeaderBytes;
    /// `blocks`: height → [`BlockRow`], the main chain only. Serves
    /// `RangeBlocks`, `GetMainChainBlockHashForHeight`, `BlockInMainChain`.
    Blocks = Blocks, "blocks", Height, BlockRow;
    /// `block_by_hash`: block hash → height, main-chain blocks: the
    /// hash-to-height index of `blocks`, `headers` and `cfilters`. Serves
    /// `GetBlockMetaForHash`, `BlockInMainChain`, and every main-chain
    /// lookup by hash.
    BlockByHash = BlockByHash, "block_by_hash", Hash32, Height;
    /// `cfilters`: main-chain height → [`CFilterRow`], at heights of
    /// `blocks` (a block may lack its filter while `have_cfilters` is
    /// unset). Serves `GetMainChainCFilters` as one range scan, and
    /// `CFilterV2` with `block_by_hash` and `cfilters_detached`.
    CFilters = CFilters, "cfilters", Height, CFilterRow;
    /// `headers_detached`: block hash → serialized header, for blocks not
    /// in the main chain: detached by a reorganisation, or recorded before
    /// they are connected. dcrwallet's `h` bucket keeps such headers, which
    /// `getblockheader` returns, and nothing prunes them (ADR-0003 Open
    /// point 5). No hash is both here and in `block_by_hash`.
    HeadersDetached = HeadersDetached, "headers_detached", Hash32, HeaderBytes;
    /// `cfilters_detached`: block hash → [`CFilterRow`] of a block in
    /// `headers_detached` (dcrwallet's `cf` bucket keeps them too).
    CFiltersDetached = CFiltersDetached, "cfilters_detached", Hash32, CFilterRow;
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
    /// `multisig_outs`: [`Outpoint`] → [`MultisigOutRow`] (dcrwallet's
    /// `ms`). Serves `GetMultisigOutput`, `AddMultisigOut`,
    /// `SpendMultisigOut`. Written through
    /// [`WriteTx::put_multisig_out`](crate::WriteTx::put_multisig_out),
    /// which keeps `multisig_unspent` ([`crate::records`]).
    MultisigOuts = MultisigOuts, "multisig_outs", Outpoint, MultisigOutRow;
    /// `multisig_unspent`: [`MultisigUnspentKey`] (script HASH160 ‖
    /// outpoint) → nothing (dcrwallet's `mu`, prefixed by the script).
    /// Serves `UnspentMultisigCreditsForAddress` as one prefix walk in
    /// dcrwallet's order.
    MultisigUnspent = MultisigUnspent, "multisig_unspent", MultisigUnspentKey, Empty;
    /// `vsp_tickets`: ticket hash → [`VspTicketRow`] (dcrwallet's `vsp`).
    /// Serves `GetVSPTicket`, `SetVSPTicket`, `DeleteVSPTicket`. Written
    /// through [`WriteTx::put_vsp_ticket`](crate::WriteTx::put_vsp_ticket),
    /// which keeps `vsp_fee_status`.
    VspTickets = VspTickets, "vsp_tickets", Hash32, VspTicketRow;
    /// `vsp_fee_status`: [`VspFeeStatusKey`] (status ‖ ticket) → nothing:
    /// the index of `vsp_tickets` by fee status (no dcrwallet equivalent).
    /// Serves `GetVSPTicketsByFeeStatus`.
    VspFeeStatus = VspFeeStatus, "vsp_fee_status", VspFeeStatusKey, Empty;
    /// `vsp_hosts`: [`VspHostId`] → [`VspHost`] (dcrwallet's `vsphost`,
    /// without its counter row). Serves `GetVSPHost` and `GetVSPTicket`'s
    /// host.
    VspHosts = VspHosts, "vsp_hosts", VspHostId, VspHost;
    /// `vsp_pubkeys`: [`VspHost`] → [`VspPubKeyRow`] (dcrwallet's
    /// `vsppubkey`). Serves `GetVSPPubKey` and the host lookup of
    /// `SetVSPTicket`.
    VspPubKeys = VspPubKeys, "vsp_pubkeys", VspHost, VspPubKeyRow;
    /// `tspend_policies`: tspend hash → [`TreasuryVote`] (dcrwallet's
    /// `tspendpolicy`). Serves `TSpendPolicy`, `TSpendPolicies`.
    TSpendPolicies = TSpendPolicies, "tspend_policies", Hash32, TreasuryVote;
    /// `treasury_key_policies`: [`PiKey`] → [`TreasuryVote`] (dcrwallet's
    /// `treasurypolicy`). Serves `TreasuryKeyPolicy`, `TreasuryKeyPolicies`.
    TreasuryKeyPolicies = TreasuryKeyPolicies, "treasury_key_policies", PiKey, TreasuryVote;
    /// `vsp_tspend_policies`: [`VspTSpendKey`] → [`TreasuryVote`]
    /// (dcrwallet's `vsptspendpolicy`). Serves `VSPTSpendPolicy`,
    /// `VSPTSpendPolicies`.
    VspTSpendPolicies = VspTSpendPolicies, "vsp_tspend_policies", VspTSpendKey, TreasuryVote;
    /// `vsp_treasury_key_policies`: [`VspTreasuryKey`] → [`TreasuryVote`]
    /// (dcrwallet's `vsptreasurypolicy`). Serves `VSPTreasuryKeyPolicy`,
    /// `VSPTreasuryKeyPolicies`.
    VspTreasuryKeyPolicies = VspTreasuryKeyPolicies, "vsp_treasury_key_policies", VspTreasuryKey, TreasuryVote;
    /// `agenda_prefs`: [`AgendaKey`] → [`AgendaChoice`] (dcrwallet's
    /// `agendaprefs`). Serves `DefaultAgendaPreference`.
    AgendaPrefs = AgendaPrefs, "agenda_prefs", AgendaKey, AgendaChoice;
    /// `ticket_agenda_prefs`: [`TicketAgendaKey`] → [`AgendaChoice`]
    /// (dcrwallet's `ticketsagendaprefs`, its nested buckets flattened).
    /// Serves `TicketAgendaPreference`.
    TicketAgendaPrefs = TicketAgendaPrefs, "ticket_agenda_prefs", TicketAgendaKey, AgendaChoice;
}

impl TableId {
    /// The table's index in [`TableId::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The number of tables.
pub const TABLE_COUNT: usize = TableId::ALL.len();

/// The tables of the records Phase 2 adds to the spike's schema
/// ([`crate::records`]): multisig outputs, VSP records, treasury and agenda
/// policies.
pub const RECORD_TABLES: [TableId; 12] = [
    TableId::MultisigOuts,
    TableId::MultisigUnspent,
    TableId::VspTickets,
    TableId::VspFeeStatus,
    TableId::VspHosts,
    TableId::VspPubKeys,
    TableId::TSpendPolicies,
    TableId::TreasuryKeyPolicies,
    TableId::VspTSpendPolicies,
    TableId::VspTreasuryKeyPolicies,
    TableId::AgendaPrefs,
    TableId::TicketAgendaPrefs,
];

/// The tables a header-and-filter batch of initial sync may write
/// ([`crate::SyncBatch`]), besides the tip and processed-marker rows of
/// `meta`: the chain tables of [`crate::chain`].
pub const SYNC_TABLES: [TableId; 6] = [
    TableId::Headers,
    TableId::Blocks,
    TableId::BlockByHash,
    TableId::CFilters,
    TableId::HeadersDetached,
    TableId::CFiltersDetached,
];
