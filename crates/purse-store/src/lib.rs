// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcr-purse's own wallet store: the schema of ADR-0003 on redb 5.
//!
//! dcr-purse never reads or writes a dcrwallet `wallet.db` (ADR-0013); every
//! row of this store is written by dcr-purse itself. The store is the
//! layer under the address manager and transaction store logic
//! (`purse-udb`, brief §6 Phase 3): it knows tables, key and value
//! encodings, transactions, cursors, the schema version and durability,
//! and nothing of wallet rules.
//!
//! # The schema
//!
//! Every table maps raw bytes to raw bytes, so key order is byte order, as
//! in bbolt (ADR-0003 Decision item 2). Integers in keys are big-endian
//! except where a client observes dcrwallet's little-endian order. Each
//! table is a type implementing [`Table`], whose key and value types
//! implement [`Codec`] with the exact layout in their documentation:
//!
//! | table | key | value | dcrwallet access path it serves |
//! |---|---|---|---|
//! | [`Meta`] | [`MetaKey`] | per row | `MainChainTip`, `ProcessedTxsBlockMarker`, `BirthState`, `IsMissingMainChainCFilters` |
//! | [`Wallet`] | [`WalletKey`] | per row ([`WalletRecord`]) | the address manager's master, crypto and coin-type keys, `watchonly`, `lastaccount`, `lastimportedaccount`, `slip0044acct0` |
//! | [`Accounts`] | [`AccountKey`] (u32 **LE**) | [`AccountRow`] | account iteration, `AccountProperties`, `MarkUsedChildIndex` |
//! | [`AccountNames`] | [`AccountName`] | [`AccountNumber`] | `LookupAccount` |
//! | [`AddrByHash`] | [`Hash160`] | [`AddrRow`] | `Manager.Address`, `AddrAccount` |
//! | [`AddrByPath`] | [`AddrPathKey`] | [`Hash160`] | `SyncAccountToAddrIndex` without derivation |
//! | [`AddrByAcctSha`] | [`AddrAcctShaKey`] (account **LE**) | [`Hash160`] | `ForEachAccountAddress` order |
//! | [`Headers`] | [`Height`] | [`HeaderBytes`] | `GetBlockHeader` of a main-chain block |
//! | [`Blocks`] | [`Height`] | [`BlockRow`] | `RangeBlocks`, `GetMainChainBlockHashForHeight` |
//! | [`BlockByHash`] | block [`Hash32`] | [`Height`] | `GetBlockMetaForHash`, `BlockInMainChain`; every main-chain lookup by hash |
//! | [`CFilters`] | [`Height`] | [`CFilterRow`] | `GetMainChainCFilters`, `CFilterV2` of a main-chain block |
//! | [`HeadersDetached`] | block [`Hash32`] | [`HeaderBytes`] | `GetBlockHeader` of a block not in the main chain |
//! | [`CFiltersDetached`] | block [`Hash32`] | [`CFilterRow`] | `CFilterV2` of a block not in the main chain |
//! | [`TxMined`] | [`TxRecKey`] | [`TxMinedRow`] | `TxDetails`, `Tx`, `ExistsTx` |
//! | [`TxUnmined`] | tx [`Hash32`] | [`TxUnminedRow`] | `UnminedTxs` |
//! | [`Unpublished`] | tx [`Hash32`] | [`UnpublishedFlag`] | `Unpublished`, `SetPublished` |
//! | [`History`] | [`HistoryKey`] | tx [`Hash32`] | `RangeTransactions` |
//! | [`AcctHistory`] | [`AcctHistoryKey`] | tx [`Hash32`] | history by account (new) |
//! | [`Credits`], [`CreditsInvalidated`] | [`CreditKey`] | [`CreditRow`] | `c`, `ic` |
//! | [`Unspent`] | [`Outpoint`] | [`UnspentRow`] | `UnspentOutputs`, `MakeInputSource` (`u`) |
//! | [`UnminedCredits`] | [`Outpoint`] | [`UnminedCreditRow`] | `mc` |
//! | [`UnminedInputs`] | [`Outpoint`] | spender tx [`Hash32`] | `mi` |
//! | [`Debits`], [`DebitsInvalidated`] | [`CreditKey`] (input index) | [`DebitRow`] | `d`, `id` |
//! | [`Tickets`] | ticket [`Hash32`] | [`Empty`] | `OwnTicket`, `IterateTickets` |
//! | [`Commitments`] | [`Outpoint`] | [`CommitmentRow`] | `cmt` |
//! | [`CommitmentsUnspent`] | [`Outpoint`] | [`CommitmentUnspentRow`] | `cmu` |
//! | [`Balances`] | [`BalanceKey`] | [`BalanceRow`] | `AccountBalance(s)` |
//! | [`BalanceHeights`] | [`BalanceHeightKey`] | [`BalanceRow`] | depth-dependent categories |
//! | [`MultisigOuts`] | [`Outpoint`] | [`MultisigOutRow`] | `ms`: `GetMultisigOutput` |
//! | [`MultisigUnspent`] | [`MultisigUnspentKey`] (script ‖ outpoint) | [`Empty`] | `mu`: `UnspentMultisigCreditsForAddress` |
//! | [`VspTickets`] | ticket [`Hash32`] | [`VspTicketRow`] | `vsp`: `GetVSPTicket`, `SetVSPTicket` |
//! | [`VspFeeStatus`] | [`VspFeeStatusKey`] | [`Empty`] | `GetVSPTicketsByFeeStatus` (an index) |
//! | [`VspHosts`] | [`VspHostId`] | [`VspHost`] | `vsphost` |
//! | [`VspPubKeys`] | [`VspHost`] | [`VspPubKeyRow`] | `vsppubkey` |
//! | [`TSpendPolicies`], [`TreasuryKeyPolicies`] | tspend [`Hash32`], [`PiKey`] | [`TreasuryVote`] | `tspendpolicy`, `treasurypolicy` |
//! | [`VspTSpendPolicies`], [`VspTreasuryKeyPolicies`] | [`VspTSpendKey`], [`VspTreasuryKey`] | [`TreasuryVote`] | `vsptspendpolicy`, `vsptreasurypolicy` |
//! | [`AgendaPrefs`], [`TicketAgendaPrefs`] | [`AgendaKey`], [`TicketAgendaKey`] | [`AgendaChoice`] | `agendaprefs`, `ticketsagendaprefs` |
//!
//! `unspent`, `unmined_credits`, `tx_unmined` and `unmined_inputs` hold
//! exactly the key sets of dcrwallet's `u`, `mc`, `m` and `mi`, in its key
//! bytes, and [`ReadCursor`] reproduces bbolt's cursor, so order that
//! reaches a client, input selection's included, is dcrwallet's (ADR-0003
//! Decision item 3; spec 04 §5.1). Main-chain headers and filters are
//! keyed by height beside `blocks`, and detached ones by hash in their own
//! tables, which nothing prunes (ADR-0003 Open point 5; ADR-0014 Open point
//! 6, decided for height keys on 2026-11-19 after the measurement in
//! ADR-0003's Phase 2 addendum). The chain tables change only through the
//! chain operations, which keep their invariants and move a reorganised
//! block's rows between the two layouts in one transaction: [`chain`].
//!
//! dcrwallet's `KeyN` of `u` is [`ReadTx::unspent_count`], the length redb
//! keeps in the table's root; there is no counter row to maintain.
//!
//! The records ADR-0003 Decision item 2 has Phase 2 add beyond the spike's
//! tables (multisig outputs, VSP records, agenda and treasury policies:
//! spec 05's auxiliary buckets) are the last twelve tables above, with the
//! operations that keep their two indexes: [`records`]. Imported keys and
//! scripts are `addr_by_hash` rows listed by `addr_by_acct_sha` under their
//! account, and voting accounts are `accounts` rows; neither needs a table
//! of its own. dcrwallet's `vsphostindex` counter is the last key of
//! `vsp_hosts`. Schema version 1 gained these tables before any release
//! (ADR-0003, addendum "The remaining records, as built").
//! dcrwallet's write-only rows `mgrcreated`, `date` and `bal`, which no
//! reader at the pin observes, are not kept (ADR-0001 §11; ADR-0003,
//! addendum "Schema version 1 as built").
//!
//! # Versions and migrations
//!
//! `meta` holds a format marker ([`FORMAT_MAGIC`]) and the schema version
//! ([`SCHEMA_VERSION`]). Opening refuses a file that is not a dcr-purse
//! store, a store with no version, and a store newer than the build, the
//! last with dcrwallet's own kind and text ([`Error::newer_version_info`]
//! names both versions). An older store is migrated one [`Migration`] at a
//! time, each step one write transaction that also records the new version
//! (ADR-0003 Decision item 6), so a crash leaves the store at a version,
//! never between two. A store of every released schema version is checked
//! in under `tests/data/` and opened by the current build in the tests.
//!
//! # Transactions and durability
//!
//! [`Store::read`] and [`Store::view`] give snapshots that never wait;
//! [`Store::update`] runs one write transaction at a time and commits it
//! with `Durability::Immediate`. The only relaxed commits are those of
//! [`InitialSync`], whose [`SyncBatch`] can do nothing but the chain
//! operations on block rows without wallet state, and advance the rescan
//! marker, with a durable commit at least every
//! [`MAX_DURABLE_INTERVAL`] commits counted over every sync of the handle
//! (ADR-0003 Decision item 9; ADR-0014 Open point 2). The API is
//! synchronous and takes no context (ADR-0008).
//!
//! A failed write transaction keeps nothing, except when the engine fails
//! inside the commit itself: the commit is then kept or lost as a whole,
//! which is known only after a reopen, and the error says so
//! ([`Error::commit_outcome_unknown`]; [`Store::update`]).
//!
//! ```
//! use purse_store::{AccountKey, AccountRow, Accounts, Error, OpenOptions};
//!
//! # fn main() -> Result<(), Error> {
//! // A store in memory; `Store::create(path)` makes one in a file.
//! let store = OpenOptions::new()
//!     .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))?;
//! store.update(|tx| {
//!     for n in [0u32, 1, 256] {
//!         let row = AccountRow { name: format!("acct-{n}").into_bytes(), ..AccountRow::default() };
//!         tx.put::<Accounts>(&AccountKey(n), &row)?;
//!     }
//!     Ok::<_, Error>(())
//! })?;
//! let tx = store.read()?;
//! let order: Vec<u32> = tx.iter::<Accounts>()?.map(|r| r.map(|(k, _)| k.0)).collect::<Result<_, _>>()?;
//! assert_eq!(order, [0, 256, 1]); // dcrwallet's little-endian account order
//! # drop(tx);
//! # store.close()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Backup
//!
//! [`Store::backup`] copies one read transaction's snapshot of every table
//! into a new store file with a backup record ([`BackupRecord`]: network,
//! schema and dcr-purse versions, time, row count and BLAKE-256 digest of
//! the rows), commits it durably, compacts it and publishes it by rename
//! from the backups directory's `.partial/`, without blocking the wallet's
//! readers or writers; [`verify_backup`] checks a file handed in from
//! outside on a copy opened with redb's integrity check, never trusting it
//! (ADR-0018 Decision items 1 to 3 and 6). The store's file name in its
//! network directory is [`STORE_FILE_NAME`]. The JSON-RPC method and the
//! tool that call these are later phases: [`backup`].
//!
//! # Cursors and ranges
//!
//! [`ReadTx::cursor`] and [`WriteTx::cursor`] give cursors with bbolt
//! v1.3.12's positioning (spec 04 §5.1): `seek` to the first key at or
//! above a key, "previous of end is last", a step off either end leaving
//! the cursor on the last or first entry. A [`WriteCursor`] can delete the
//! entry it stands on, and the transaction can be written between its moves
//! ([`WriteCursor::tx`]); each move then reads the table as it is, so a
//! deleted entry is never returned and none is skipped or repeated, where
//! bbolt's answer depends on its page layout, and where bbolt's cursor
//! misbehaves around leaves emptied in the transaction (QK-0747, QK-0749,
//! QK-0750) the store's does not (the rule is on [`WriteCursor`];
//! docs/PARITY.md). [`TypedCursor`] decodes what a cursor
//! returns. Bounded ranges ([`ReadTx::range`], [`ReadTx::range_raw`],
//! [`ReadTx::prefix`], the same on [`WriteTx`]) iterate from either end.
//! `tests/conformance.rs` holds all of it against bbolt itself through the
//! `boltcursor` oracle. Puts keep bbolt's limits: no empty key, keys up to
//! [`MAX_KEY_SIZE`], values up to [`MAX_VALUE_SIZE`].
//!
//! # How it is tested
//!
//! Besides the encoding, store and cursor suites: `tests/powerloss.rs` cuts
//! the power at every storage operation of a workload (clean, torn, and
//! with every write kept, as a killed process leaves it) and requires the
//! reopened store to hold exactly some commit, no acknowledged durable
//! commit lost, relaxed commits lost only within their bound;
//! `tests/model.rs` runs random sequences of transactions, reopens and power
//! cuts against an in-memory model and shrinks a failing sequence;
//! `tests/migration.rs` cuts migrations at every write and opens the
//! fixtures. The record tables are in all three, and `tests/records.rs`
//! holds their operations and checks; `tests/records_oracle.rs` runs the
//! VSP, treasury and agenda operations against dcrwallet's own udb through
//! the `oprunner` oracle. The power-loss backend
//! (`tests/support/powerloss.rs`) is a port of dcroxide's.
//! `tests/measure.rs` holds the measurements of ADR-0003's addendum, as
//! ignored tests. `tests/backup.rs` holds the backup: a backup under a
//! concurrent writer equals a committed state, a power cut at every
//! storage operation of a backup never leaves a file that verifies as
//! anything but the complete backup, a stop at every step publishes
//! nothing, and verification refuses damaged and foreign files.
//!
//! # Errors
//!
//! [`Error::kind`] is the dcrwallet [`purse_errors::Kind`] the nearest bbolt
//! condition has (spec 01 §8.1); the text of an engine error is redb's
//! (ADR-0011 ET-006). Errors convert into [`purse_errors::Error`].

pub mod backup;
pub mod chain;
mod codec;
mod cursor;
mod error;
mod keys;
pub mod records;
mod schema;
mod store;
mod sync;
mod txn;
mod values;
pub mod wallet;

pub use backup::{
    BACKUP_RECORD_KEY, BACKUPS_DIR, Backup, BackupInfo, BackupRecord, BackupStep, PARTIAL_DIR,
    Verified, check_backup_name, verify_backup, verify_backup_on_backend,
};
pub use chain::{ChainBlock, GENESIS_AGAIN_TEXT, IN_MAIN_CHAIN_TEXT};
pub use codec::{Codec, DecodeDetail, DecodeError};
pub use cursor::{RawCursor, ReadCursor, TypedCursor, WriteCursor};
pub use error::{Error, MISSING_FILE_TEXT, NEWER_VERSION_TEXT, NOT_INITIALIZED_TEXT, Result};
pub use keys::{
    AccountKey, AccountName, AccountNumber, AcctHistoryKey, AddrAcctShaKey, AddrPathKey, AgendaKey,
    BalanceHeightKey, BalanceKey, CreditKey, Hash32, Hash160, Height, HistoryKey, MetaKey,
    MultisigUnspentKey, Outpoint, PiKey, TicketAgendaKey, TxRecKey, VspFeeStatusKey, VspHost,
    VspHostId, VspTSpendKey, VspTreasuryKey, WalletKey,
};
pub use records::{UNKNOWN_VSP_HOST_TEXT, VSP_HOST_IDS_EXHAUSTED_TEXT};
pub use schema::{
    AccountNames, Accounts, AcctHistory, AddrByAcctSha, AddrByHash, AddrByPath, AgendaPrefs,
    BalanceHeights, Balances, BlockByHash, Blocks, CFilters, CFiltersDetached, Commitments,
    CommitmentsUnspent, Credits, CreditsInvalidated, Debits, DebitsInvalidated, Headers,
    HeadersDetached, History, Meta, MultisigOuts, MultisigUnspent, RECORD_TABLES, SYNC_TABLES,
    TABLE_COUNT, TSpendPolicies, Table, TableId, TicketAgendaPrefs, Tickets, TreasuryKeyPolicies,
    TxMined, TxUnmined, UnminedCredits, UnminedInputs, Unpublished, Unspent, VspFeeStatus,
    VspHosts, VspPubKeys, VspTSpendPolicies, VspTickets, VspTreasuryKeyPolicies, Wallet,
};
pub use store::{
    DIR_MODE, FILE_MODE, FORMAT_MAGIC, MIGRATIONS, Migration, OpenOptions, SCHEMA_VERSION,
    STORE_FILE_NAME, Schema, Store,
};
pub use sync::{InitialSync, MAX_DURABLE_INTERVAL, SyncBatch};
pub use txn::{Iter, MAX_KEY_SIZE, MAX_VALUE_SIZE, RawEntry, RawIter, ReadTx, WriteTx};
pub use values::{
    AccountRow, AddrRow, AgendaChoice, BalanceClass, BalanceRow, BirthState, BlockRow, CFilterRow,
    CoinTypeKeys, CommitmentRow, CommitmentUnspentRow, CreditFlags, CreditRow, DebitRow, Empty,
    HeaderBytes, MasterKeyParams, MultisigBlock, MultisigOutRow, MultisigSpender, OutputTag,
    RawValue, ScriptLoc, SealedKey, Slip0044Acct0Row, Spender, Tip, TreasuryVote, TxMinedRow,
    TxUnminedRow, UnminedCreditRow, UnpublishedFlag, UnspentRow, VspPubKeyRow, VspTicketRow,
};
pub use wallet::WalletRecord;

/// The redb release the store links (the root manifest pins it exactly).
pub const REDB_VERSION: &str = "5.0.0";

/// Re-exported for storage backends of fault-injection tests: redb's
/// backend trait and its error.
pub use redb::{BackendError, StorageBackend};
