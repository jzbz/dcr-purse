// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dcrwallet's `wallet/udb` behaviour over dcr-purse's own store
//! (`purse-store`, ADR-0003), and the Rust producer of the logical export
//! (ADR-0001 section 11).
//!
//! This is the crate the brief's crate plan names for Phase 3 (brief
//! section 5, `purse-udb`: the address manager and transaction store
//! logic). It ports behaviour, not layout: dcr-purse never reads or writes
//! a dcrwallet `wallet.db` (ADR-0013), so what must match Go is what the
//! API returns and what the logical export shows, never the bytes of a
//! bbolt bucket (brief section 3, W2).
//!
//! It holds the address manager's creation from a seed, its opening, the
//! wallet lock, the coin type (its private key and the SLIP0044 upgrade),
//! BIP0044 and imported xpub accounts, addresses (derivation and
//! recording, the used and returned indexes, lookups, private keys and
//! redeem scripts), imported keys and scripts, `Encrypt`/`Decrypt`,
//! watching-only wallets (creation from an account's extended public key,
//! conversion, public key imports), per-account passphrases (set, change,
//! remove, unlock, lock), imported voting accounts and changes of the
//! private and public passphrases: every entry point of the address
//! manager. The transaction store ([`txstore`]) is written in slices; the
//! first (the main chain, headers and filters, mined and unmined
//! transaction records, `Rollback` without its credit step, and the queries
//! over them) is written, and its entry points are in the second table
//! below ("Transaction store entry points"). The first
//! table is the complete inventory of the address manager's Go entry
//! points at the parity pin (dcrwallet `54b85b33`; spec 02 sections 3.3
//! and 3.4), each with its status.
//!
//! # Go entry points
//!
//! "Ported" is this crate as it stands; "later" names what is not written,
//! and "not ported" what never will be. Every "ported" row is checked
//! against Go through the oracle (see "How it is verified"). The one row
//! still "later", `Wallet.UpgradeToSLIP0044CoinType`, is a method of the
//! wallet, not of the address manager: it decides when to call
//! [`Manager::upgrade_to_slip0044_coin_type`], which is ported, and belongs
//! to the wallet crate (Phase 5).
//!
//! | Go (`wallet/udb`) | Rust | Status |
//! |---|---|---|
//! | `Initialize` (address manager part: `createAddressManager`) | [`Manager::create`] | ported |
//! | `InitializeWatchOnly` (`createWatchOnly`) | [`Manager::create_watch_only`] | ported (an extended private key is refused where Go stores it, QK-0762, decided by the project owner on 2026-11-22) |
//! | `Open` (address manager part: `loadManager`) | [`Manager::open`] | ported |
//! | `HDKeysFromSeed`, `CoinTypes` | [`addrmgr::hd_keys_from_seed`], [`addrmgr::coin_types`] | ported |
//! | `ValidateAccountName`, `IsImportedVoting` | [`addrmgr::validate_account_name`], [`addrmgr::is_imported_voting`] | ported |
//! | `Manager.WatchingOnly`, `ChainParams`, `IsLocked` | [`Manager::watching_only`], [`Manager::chain_params`], [`Manager::is_locked`] | ported |
//! | `Manager.Lock`, `Unlock`, `UnlockedWithPassphrase` | [`Manager::lock`], [`Manager::unlock`], [`Manager::unlocked_with_passphrase`] | ported |
//! | `Manager.CoinType` | [`Manager::coin_type`] | ported |
//! | `Manager.LastAccount`, `LastImportedAccount`, `AccountName`, `ForEachAccount` | [`Manager::last_account`], [`Manager::last_imported_account`], [`Manager::account_name`], [`Manager::for_each_account`] | ported |
//! | `Manager.AccountProperties`, `AccountHasPassphrase` | [`Manager::account_properties`], [`Manager::account_has_passphrase`] | ported |
//! | `Manager.AccountExtendedPubKey`, `AccountExtendedPrivKey`, `AccountBranchExtendedPubKey` | [`Manager::account_extended_pub_key`], [`Manager::account_extended_priv_key`], [`Manager::account_branch_extended_pub_key`] | ported |
//! | `Manager.Encrypt`, `Decrypt` (`CKTPublic`, `CKTPrivate`) | [`Manager::encrypt_public`], [`Manager::encrypt_private`], [`Manager::decrypt_public`], [`Manager::decrypt_private`] | ported (one method per key: Go's invalid key type cannot be passed) |
//! | `Manager.Close` | dropping the [`Manager`] (keys are wiped on drop) | ported (as drop; `rt.reopen` drops and opens) |
//! | `Manager.CoinTypePrivKey`, `UpgradeToSLIP0044CoinType` | [`Manager::coin_type_priv_key`], [`Manager::upgrade_to_slip0044_coin_type`] | ported (the upgrade writes Go's zero counters, QK-0027, drops account 0's cached entry instead of replacing its keys before the commit, QK-0043, derives account 0's recorded address rows again from the SLIP0044 key where Go keeps the legacy rows, QK-0760, and writes account 0 without the Argon2id parameters of a passphrase of its own, which Go leaves over a key that passphrase no longer opens, and reports that it did, QK-0767) |
//! | `Manager.NewAccount`, `RenameAccount`, `LookupAccount` | [`Manager::new_account`], [`Manager::rename_account`], [`Manager::lookup_account`] | ported |
//! | `Manager.ConvertToWatchingOnly` | [`Manager::convert_to_watching_only`] | ported (runs its own write transaction and changes memory only after the commit, QK-0733; the store keeps every account's sealed private key as Go does, QK-0761, except the key a BIP0044 account's own passphrase seals, which is removed from its row, QK-0761 and QK-0766, decided by the project owner on 2026-11-22 (the file's freed pages may keep earlier copies of what a conversion removes until they are reused, QK-0761); `PrivateKey` afterwards answers from the store: for an imported key an error where Go panics until a reopen, QK-0763; for an account Go had cached, the locked account's text where Go's is `no private key for ...` until a reopen, QK-0764) |
//! | `Manager.ChangePassphrase` | [`Manager::change_passphrase`] | ported (runs its own write transaction and installs the new master key only after the commit, QK-0733; an empty new private passphrase is refused with creation's error where Go accepts it, QK-0768, decided by the project owner on 2026-11-22) |
//! | `Manager.ImportXpubAccount` | [`Manager::import_xpub_account`] | ported |
//! | `Manager.ImportVotingAccount` (`importAccount`) | [`Manager::import_voting_account`] | ported (the key's network is not checked, as in Go; the account number cannot wrap onto account 0, QK-0041; on a manager converted in this process it succeeds where Go panics, QK-0765) |
//! | `Manager.SetAccountPassphrase` (`removeAccountPassphrase`), `UnlockAccount`, `LockAccount` | [`Manager::set_account_passphrase`], [`Manager::unlock_account`], [`Manager::lock_account`] | ported (memory follows the store when a transaction that set a passphrase rolls back, QK-0733; a conversion locks every account, QK-0766; Argon2id's refusal of a zero thread count is an error where Go panics, QK-0630) |
//! | `Manager.Address`, `AddrAccount`, `ExistsHash160` | [`Manager::address`], [`Manager::addr_account`], [`Manager::exists_hash160`] | ported |
//! | `Manager.ImportPrivateKey`, `ImportPublicKey`, `ImportScript` | [`Manager::import_private_key`], [`Manager::import_public_key`], [`Manager::import_script`] | ported |
//! | `Manager.PrivateKey`, `HavePrivateKey`, `RedeemScript` | [`Manager::private_key`], [`Manager::have_private_key`], [`Manager::redeem_script`] | ported (`redeemScriptForHash160` is the body of `redeem_script`; its one other caller is the transaction store's) |
//! | `Manager.SyncAccountToAddrIndex`, `RecordDerivedAddress` | [`Manager::sync_account_to_addr_index`], [`Manager::record_derived_address`] | ported (`RecordDerivedAddress` refuses a branch other than 0 or 1, QK-0759) |
//! | `Manager.MarkUsed`, `MarkUsedChildIndex`, `MarkReturnedChildIndex` | [`Manager::mark_used`], [`Manager::mark_used_child_index`], [`Manager::mark_returned_child_index`] | ported (an account with no row is an `IO` error, where Go panics, QK-0039) |
//! | `Manager.ForEachAccountAddress`, `ForEachActiveAccountAddress`, `ForEachActiveAddress` | [`Manager::for_each_account_address`], [`Manager::for_each_active_account_address`], [`Manager::for_each_active_address`] | ported |
//! | `Wallet.UpgradeToSLIP0044CoinType` (`wallet/cointype.go`) | — | later (wallet crate) |
//! | `Migrate`, `upgradeManager`, the 25 database upgrades | — | not ported: no dcrwallet database is read (ADR-0013); a new wallet is written in its final form |
//! | `ObtainUserInputFunc`, `Manager.closed` | — | not ported: unused at the pin (spec 02 quirk 22) |
//!
//! # Auxiliary record entry points
//!
//! dcrwallet's auxiliary buckets at the parity pin (`wallet/udb/vsp.go`,
//! `treasury.go`, `agendas.go`; spec 05 sections 3.3, 3.4 and 5.7 to
//! 5.8), ported on 2026-11-26 in [`vsp`], [`treasury`] and [`agendas`],
//! free functions over the caller's transaction as Go's are. Every
//! "ported" row is checked against Go through the oracle (hand-written
//! scripts `39-aux-vsp`, `40-aux-treasury`, `41-aux-agendas`, a
//! key-limit script and generated scripts with and without groups,
//! records and exports; see "How it is verified").
//!
//! | Go (`wallet/udb`) | Rust | Status |
//! |---|---|---|
//! | `SetVSPTicket` | [`vsp::set_vsp_ticket`] | ported (returns the host id Go writes into the record; a known host keeps its first public key, QK-0122; an empty or over-long host refused before anything is written, where Go has written the host row and counter, QK-0122) |
//! | `DeleteVSPTicket`, `GetVSPTicket` | [`vsp::delete_vsp_ticket`], [`vsp::get_vsp_ticket`] | ported (the public key a copy, QK-0124) |
//! | `GetVSPTicketsByFeeStatus` | [`vsp::get_vsp_tickets_by_fee_status`] | ported (ticket hash order where Go returns a map, QK-0125; Go's `int` argument as `i64`) |
//! | `GetVSPHost`, `GetVSPPubKey` | [`vsp::get_vsp_host`], [`vsp::get_vsp_pub_key`] | ported (id 0 is upgrade 23's empty host; `GetVSPHost`'s text names the id where Go prints a pointer, QK-0123) |
//! | `SetVSPHost`, `SetVSPPubKey` | — | not ported: no caller at the pin outside `SetVSPTicket`, which reaches the host tables through `purse_store::WriteTx::add_vsp_host` (ADR-0003 R2, decided by the project owner on 2026-11-21; QK-0047) |
//! | `FeeStatus`, `VSPFeeProcessStarted` ... `VSPFeeProcessConfirmed`, `VSPTicket` | [`vsp::VSP_FEE_PROCESS_STARTED`] ... [`vsp::VSP_FEE_PROCESS_CONFIRMED`] (`u32`, the stored type), [`vsp::VspTicket`] | ported |
//! | `SetTSpendPolicy`, `SetVSPTSpendPolicy`, `SetTreasuryKeyPolicy`, `SetVSPTreasuryKeyPolicy` | [`treasury::set_tspend_policy`], [`treasury::set_vsp_tspend_policy`], [`treasury::set_treasury_key_policy`], [`treasury::set_vsp_treasury_key_policy`] | ported (0 deletes, any other byte stored, QK-0127) |
//! | `TSpendPolicy`, `VSPTSpendPolicy`, `TreasuryKeyPolicy`, `VSPTreasuryKeyPolicy` | [`treasury::tspend_policy`], [`treasury::vsp_tspend_policy`], [`treasury::treasury_key_policy`], [`treasury::vsp_treasury_key_policy`] | ported (Go's `invalid length` answer unreachable: the value type is one non-zero byte, QK-0121) |
//! | `TSpendPolicies`, `VSPTSpendPolicies`, `TreasuryKeyPolicies`, `VSPTreasuryKeyPolicies`, `VSPTSpend`, `VSPTreasuryKey` | [`treasury::tspend_policies`], [`treasury::vsp_tspend_policies`], [`treasury::treasury_key_policies`], [`treasury::vsp_treasury_key_policies`] (keys `purse_store::VspTSpendKey`, `purse_store::VspTreasuryKey`) | ported (key order where Go returns maps; an error without the partial map, QK-0127) |
//! | `SetDefaultAgendaPreference`, `SetTicketAgendaPreference` | [`agendas::set_default_agenda_preference`], [`agendas::set_ticket_agenda_preference`] | ported (every failure wrapped in `IO`, QK-0067; a ticket's key limit is Go's nested key's) |
//! | `DefaultAgendaPreference`, `TicketAgendaPreference` | [`agendas::default_agenda_preference`], [`agendas::ticket_agenda_preference`] | ported (a `Result`, ADR-0012 Rule 2(c)) |
//!
//! Their rows, against dcrwallet's buckets:
//!
//! | dcrwallet (bbolt) | purse-store | Notes |
//! |---|---|---|
//! | `vsp` | `vsp_tickets` (dcrwallet's 40 bytes) and its index `vsp_fee_status` (status ‖ ticket) | the index has no dcrwallet counterpart (ADR-0003 R3); the export folds it away and refuses a store where it disagrees with the tickets |
//! | `vsphost` (id → host) and `vsppubkey` (host → id, public key) | `vsp_hosts`, `vsp_pubkeys` | each the other's inverse ([`purse_store::ReadTx::check_records`]) |
//! | `vsphost/vsphostindex`, `vsphost/00000000` | — | not kept: the last host id is the counter, and `GetVSPHost(0)` answers upgrade 23's empty host itself (ADR-0003 R2) |
//! | `tspendpolicy`, `treasurypolicy`, `vsptspendpolicy`, `vsptreasurypolicy` | `tspend_policies`, `treasury_key_policies`, `vsp_tspend_policies`, `vsp_treasury_key_policies` | dcrwallet's keys; one non-zero byte per row |
//! | `agendaprefs`, `ticketsagendaprefs/<ticket>/` | `agenda_prefs`, `ticket_agenda_prefs` (the ticket leading the key) | an empty nested bucket of dcrwallet's has no row here, as the export does not show it (ADR-0001 section 9, N7) |
//! | `wstakemgr` | — | not kept: dead at the pin, neither created, read nor written (spec 05 section 4.5, QK-0126) |
//!
//! # Transaction store entry points
//!
//! Every exported entry point of dcrwallet's transaction store at the
//! parity pin (`wallet/udb/txmined.go`, `txunmined.go`, `txquery.go`,
//! `cfilters.go`; spec 04 section 3), with the slice it belongs to. The
//! design of the first slice, the rows it keeps and the rules that bound it
//! are in [`txstore`]. "Slice 1" is ported: checked against Go through the
//! oracle on all four networks; "credit slice" is ported by the credit
//! slice (2026-11-23), which uses dcroxide's `MsgTx::pk_script_locs` for
//! `wire.MsgTx.PkScriptLocs` and is checked against Go by hand-written
//! scripts on simnet and generated ones on all four networks; "commitment
//! and multisig slice" is ported by the third slice (2026-11-24), checked
//! against Go by hand-written scripts on simnet; "selection slice" by the
//! fourth (2026-11-24), checked against Go by hand-written scripts on
//! simnet and generated ones on all four networks, the random selections
//! and the bytes they draw exactly; "later" is a later slice,
//! with the reason ([`txstore`], "The next slice"). Rust names are methods of [`txstore::TxStore`]
//! unless stated, in snake case of Go's.
//!
//! | Go (`wallet/udb`) | Rust | Status |
//! |---|---|---|
//! | `Initialize`, `InitializeWatchOnly` (transaction store part: `createStore`, and the rows upgrades 10, 11, 15 and 26 add) | [`txstore::TxStore::create`], run in the creating transaction after [`Manager::create`] or [`Manager::create_watch_only`], as Go's `Initialize` runs both in one | slice 1 |
//! | `Open` (transaction store part: the `Store` value) | `TxStore::new` | slice 1 |
//! | `NewTxRecord`, `NewTxRecordFromMsgTx` | [`txstore::TxRecord::new`], [`txstore::TxRecord::from_msg_tx`] | slice 1 |
//! | `Store.MainChainTip` | `main_chain_tip` | slice 1 (returns a `Result`, ADR-0012 Rule 2(c)) |
//! | `Store.ExtendMainChain` | `extend_main_chain` | slice 1 (the filter is optional below the birthday height, ADR-0019 FB4; the parent's stake validation or invalidation: the block's flag in slice 1; the move of its credits and debits in the credit slice, QK-0082 fixed; the unmined copy of a revalidated transaction dropped in the selection slice, QK-0734 fixed) |
//! | `Store.ProcessedTxsBlockMarker`, `UpdateProcessedTxsBlockMarker` | `processed_txs_block_marker`, `update_processed_txs_block_marker` | slice 1 |
//! | `SetBirthState`, `BirthState` | [`txstore::set_birth_state`], [`txstore::birth_state`] (free functions, as in Go) | slice 1 |
//! | `Store.IsMissingMainChainCFilters`, `MissingCFiltersHeight` | `is_missing_main_chain_cfilters`, `missing_cfilters_height` | slice 1 (the walk starts at the birthday height, ADR-0019 FB9 and FB10: a PARITY.md line) |
//! | `Store.InsertMissingCFilters`, `GetMainChainCFilters`, `CFilterV2`, `ImportCFiltersV2` | `insert_missing_cfilters`, `get_main_chain_cfilters`, `cfilter_v2`, `import_cfilters_v2` | slice 1 (`GetMainChainCFilters` from a detached start block answers as Go's, QK-0751) |
//! | `ExtractBlockHeaderParentHash`, `ExtractBlockHeaderHeight`, `ExtractBlockHeaderTime`, `RawBlockHeader.Height` | free functions over a 180-byte header | slice 1 |
//! | `Store.GetMainChainBlockHashForHeight`, `GetSerializedBlockHeader`, `GetBlockHeaderTime`, `GetBlockHeader`, `BlockInMainChain`, `GetBlockMetaForHash`, `GetMainChainBlockHashes` | the same names in snake case | slice 1 (a stale header gives the error Go's next comparison gives, where Go panics, QK-0086) |
//! | `Store.Rollback` | `rollback` (the blocks with transactions, as Go's map) | slice 1; its credit and debit steps in the credit slice (the invalidated rows of a detached block deleted, QK-0081); its commitment and multisig steps in the commitment and multisig slice |
//! | `Store.InsertMinedTx` | `insert_mined_tx` | slice 1 (the debit loop and `moveMinedTx`'s credit move in the credit slice; into a stake-invalidated block, an unmined transaction is refused where Go panics) |
//! | `Store.InsertMemPoolTx`, `SetPublished`, `RemoveUnconfirmed`, `PruneUnmined` | `insert_mem_pool_tx`, `set_published`, `remove_unconfirmed`, `prune_unmined` | slice 1 (`RemoveUnconfirmed`'s unmined credits, and the balance classes of the unmined inputs and the unpublished mark, in the credit slice; its commitment and multisig steps in the commitment and multisig slice, QK-0060 reproduced) |
//! | `Store.AddCredit` | `add_credit` | credit slice (a block that is not the main-chain block at its height, or holds no record of the transaction, is refused, QK-0783) |
//! | `Store.AddTicketCommitment`, `RedeemTicketCommitments` | `add_ticket_commitment`, `redeem_ticket_commitments` | commitment and multisig slice (the `Locked` running sum kept with every write of an unspent commitment, ADR-0003 Decision item 4) |
//! | `Store.AddMultisigOut`, `SpendMultisigOut`, `GetMultisigOutput`, `UnspentMultisigCreditsForAddress` | `add_multisig_out`, `spend_multisig_out`, `get_multisig_output` (the spender only while spent, QK-0055), `unspent_multisig_credits_for_address` | commitment and multisig slice (QK-0054 reproduced, pending owner; a block of the zero hash at another height than 0 refused, ADR-0012 Rule 4) |
//! | `Store.UnspentOutputCount` | `unspent_output_count` | slice 1 (the length of `unspent`, ADR-0003, addendum "Schema version 1 as built") |
//! | `Store.UnspentOutputs`, `UnspentOutput`, `ForEachUnspentOutpoint`, `ExistsUTXO` | `unspent_outputs`, `unspent_output`, `for_each_unspent_outpoint` (the callback returns a `Result`), `exists_utxo` | credit slice (`ExistsUTXO` returns a `Result`, ADR-0012 Rule 2(c)) |
//! | `Store.RandomUTXO` | `random_utxo` (drawing from a [`purse_snacl::RandomSource`] as Go draws from its generator) | selection slice (bbolt's cursor walk, spec 04 Q-31) |
//! | `Store.IsUnspentOutpoint` | `is_unspent_outpoint` | credit slice, as Go behaves (QK-0088 reproduced; spec 04 D-1 taken so, decided by the project owner on 2026-11-24, [`txstore`] choice 6) |
//! | `Store.MakeInputSource`, `InputSource.SelectInputs` | `make_input_source` (returns a `Result`, Rule 2(c)), [`txstore::InputSource::select_inputs`] (answers `purse-txauthor`'s `InputDetail`; the source is a `purse-txauthor` input source) | selection slice (Go's draws, spec 23 §5.6.2; QK-0096, QK-0105 and QK-0788 reproduced) |
//! | `Store.AccountBalance`, `AccountBalances` | `account_balance`, `account_balances` (a vector in account order) | credit slice (running sums, ADR-0003 Decision item 4; `LockedByTickets` from the commitment and multisig slice) |
//! | `Store.UnspentTickets` | `unspent_tickets` | credit slice |
//! | `Store.OwnTicket`, `IterateTickets`, `TicketIterator.Next`, `Err`, `Close`, `TicketDetails` | `own_ticket`, `iterate_tickets` (a vector), `ticket_details` | slice 1 (the credit of output 0 in the credit slice) |
//! | `Store.TxDetails`, `TxDetails.Height`, `UniqueTxDetails`, `Tx`, `ExistsTx`, `ExistsTxMinedOrUnmined`, `TxBlockHeight` | the same names in snake case | slice 1 (the credit and debit lists in the credit slice) |
//! | `Store.RangeTransactions`, `RangeBlocks` | `range_transactions`, `range_blocks` | slice 1 (no context argument, ADR-0008) |
//! | `Store.PreviousPkScripts` | `previous_pk_scripts` | credit slice, as Go behaves (QK-0089 reproduced: never answers without a block; spec 04 D-1 taken so, decided by the project owner on 2026-11-24, [`txstore`] choice 6) |
//! | `Store.Spender` | `spender` | selection slice |
//! | `Store.UnminedTxs`, `UnminedTxHashes` | `unmined_txs`, `unmined_tx_hashes` | slice 1 (ties in the dependency order broken by hash, ADR-0012 Rule 3(a); QK-0092) |
//!
//! # Store rows
//!
//! dcrwallet's `waddrmgr` bucket tree (spec 02 section 4) against the
//! `purse-store` tables this crate uses. No schema addition is needed:
//! schema version 1 already has every row the ported operations write.
//!
//! | dcrwallet (bbolt) | purse-store | Notes |
//! |---|---|---|
//! | `main/mpub`, `main/mpriv` | `wallet` rows `master_pub`, `master_priv` ([`purse_store::MasterKeyParams`]) | the 88-byte snacl parameters, typed |
//! | `main/cpub`, `main/cpriv` | `wallet` rows `crypto_pub`, `crypto_priv` | the sealed crypto keys, verbatim |
//! | `main/ctpub`, `ctpriv`, `ctpub-slip0044`, `ctpriv-slip0044` | `wallet` rows `cointype_legacy`, `cointype_slip0044` ([`purse_store::CoinTypeKeys`]) | each half optional, as in dcrwallet; the legacy row's public half decides the active coin type |
//! | `main/slip0044acct0` | `wallet` row `slip0044_acct0` ([`purse_store::Slip0044Acct0Row`]) | the two version-1 counters (0 at creation) kept unconverted, for the coin type upgrade (spec 02 quirk 1) |
//! | `main/watchonly` | `wallet` row `watch_only` | `true` for a wallet made by [`Manager::create_watch_only`] or converted by [`Manager::convert_to_watching_only`] |
//! | `meta/lastaccount`, `meta/lastimportedaccount` | `wallet` rows `last_account`, `last_imported` | an absent `last_imported` reads as 0x7FFFFFFF, as `fetchLastImportedAccount` reads it |
//! | `acct/<n>` (type, sealed xpub, sealed xpriv) and `acctvars/<n>/` (`extused`, `intused`, `extret`, `intret`, `name`, `kdf-params`) and `acctididx/<n>` | `accounts` row `<n LE>` ([`purse_store::AccountRow`]): type, the four counters, the name, and the key material ([`addrmgr::AccountKeys`]: sealed xpub, sealed xpriv, optional Argon2id parameters) | the name is stored once by number here (the row) where dcrwallet keeps two copies (`acctvars`, `acctididx`); with the name index below, two copies against dcrwallet's three (ADR-0001 section 11 allows rendering the three from these); the private key's scheme and the parameters live in one value, so they change together |
//! | `acctnameidx` | `account_names` (name → account number) | the second copy of the name, as the key, as in dcrwallet |
//! | `addr` (key SHA-256 of the HASH160: type, account, add time, raw data) | `addr_by_hash` row `<HASH160>` ([`purse_store::AddrRow`]): a derived row's account, branch and index; an imported row's account and data ([`addrmgr::ImportedData`]: a key's sealed public and private keys, or a script's sealed HASH160 and the script in the clear) | no add time: written by dcrwallet, read by nothing (spec 02 section 10 item 22); the export renders it `0`. dcrwallet's bucket order (SHA-256 of the HASH160) is recovered by sorting where it reaches an output |
//! | `addracctidx/<hash>` (flat) | — | the account of a row is in the row, and dcrwallet repoints the flat entry on every insert, so the two never differ |
//! | `addracctidx/<account>/` (per account) | `addr_by_acct_sha` row `<account LE> ‖ <SHA-256 of the HASH160>` → HASH160 | the key is dcrwallet's nested key, so a prefix walk is `ForEachAccountAddress` order; an entry left by a row recorded again under another account stays, as in dcrwallet (QK-0757) |
//! | — | `addr_by_path` | not written: no operation reads it (ADR-0003's gap-limit use belongs to the wallet crate); the export refuses a store where it is not empty |
//! | `main/mgrver`, `main/mgrcreated`, `meta/addrpool*`, `usedaddrs` | — | not kept: dead or write-only at the pin (ADR-0001 section 9; ADR-0003, addendum "Schema version 1 as built") |
//!
//! The transaction store's rows (dcrwallet's `wtxmgr` bucket tree) are in
//! [`txstore`], "Rows".
//!
//! # Locking and secrets
//!
//! Concurrency follows ADR-0008: a synchronous API, no runtime. A
//! [`Manager`] is shared by reference; its in-memory state (lock state,
//! account cache, keys, the keys account passphrases opened) is behind one
//! `RwLock`, always taken after the
//! caller's store transaction, never the reverse (Go's order: transaction,
//! then `Manager.mtx`; spec 02 section 7). Methods whose Go version takes
//! the read lock but inserts into the cache (`AccountProperties`,
//! `AccountHasPassphrase`) take the write lock. Every method that touches
//! the store takes the caller's transaction ([`addrmgr::StoreRead`] for
//! reads, `purse_store::WriteTx` for writes), as Go's take a bucket, so the
//! manager never begins a transaction and its writes commit or roll back
//! with the caller's. The two exceptions are
//! [`Manager::convert_to_watching_only`] and [`Manager::change_passphrase`],
//! which take the store and run their own write transaction, so that they
//! change the manager's memory only once the change has committed
//! (QK-0733): `Unlock`, which takes no transaction, must never accept a
//! passphrase the store does not hold. Go's only caller of the conversion
//! is a test, and its production callers of `ChangePassphrase` run it alone
//! in a transaction of their own.
//!
//! What is held decrypted, and when (purse-snacl's "Secrets" section
//! applies to every key type named here):
//!
//! | Secret | Held | Wiped |
//! |---|---|---|
//! | public master key (scrypt-derived) | from [`Manager::open`] until drop | on drop |
//! | public crypto key | from open until drop | on drop |
//! | account extended public keys | in the cache from first use until drop | on drop |
//! | private master key's derived key | from a successful [`Manager::unlock`] until lock (a [`Manager::change_passphrase`] of the private passphrase on an unlocked wallet replaces it with the new one's; one whose commit's outcome is unknown, during which another thread locked or unlocked the manager, locks it and installs none) | zeroed by lock and on drop; the old one dropped (wiped) by the change |
//! | the key derived from the old passphrase during [`Manager::change_passphrase`] (Go's temporary `secretKey`), the opened private crypto key it re-seals, and a new master key's derived key on a locked wallet | for the call only | on return, success or error |
//! | private crypto key | while unlocked | dropped (wiped) by lock |
//! | account extended private keys | in the cache while unlocked | dropped (wiped) by lock, every account included |
//! | the extended private key of an account with its own passphrase | from [`Manager::unlock_account`] (or a [`Manager::set_account_passphrase`] that leaves it unlocked) until [`Manager::lock_account`], any wallet lock or a failed [`Manager::unlock`], tied to the sealed value it belongs to (never in the cache) | dropped (wiped) then, by a conversion, and on drop |
//! | the Argon2id-derived key of an account passphrase, and the opened key string | for the call only ([`Manager::unlock_account`], [`Manager::set_account_passphrase`], [`Manager::import_voting_account`]) | on return (`purse_snacl::AccountKey`, `SecretBytes` and the key strings wipe themselves) |
//! | the account passphrase's keyed hash | with the unlocked account's key | with it |
//! | passphrase hash and hasher key | hash while unlocked, key from open (a private passphrase change replaces both, as Go replaces its hasher; the account passphrases' hasher key, a copy of the key drawn at open, is kept until drop) | hash dropped by lock; both wiped on drop and when replaced |
//! | seed, coin type and account keys during [`Manager::create`] | for the call only | on return, success or error |
//! | the account key and its string during [`Manager::create_watch_only`] (a private key the caller passes is parsed, then refused, QK-0762) | for the call only | on return, success or error |
//! | what a conversion makes unusable: the private master key, the private crypto key and its sealed copy, every cached account's private key, every key an account passphrase opened (QK-0766) | until [`Manager::convert_to_watching_only`] commits (the manager is locked inside its transaction) | wiped by the lock and on drop; the store, and the cache with it, keep the accounts' sealed private keys, sealed by the deleted crypto key (QK-0761, QK-0764), but not those BIP0044 accounts' own passphrases sealed, which the conversion removes from their rows (QK-0761, QK-0766); the store file's freed pages may keep earlier copies of every removed value until later commits reuse them, and a backup of the converted store holds none (QK-0761) |
//! | the coin type private key and the new account's private key during [`Manager::new_account`] | for the call only | on return, success or error |
//! | the coin type private key from [`Manager::coin_type_priv_key`] | by the caller | when the caller drops it ([`purse_hdkeychain::ExtendedKey`] wipes itself); the decrypted string on return |
//! | account 0's new private key during [`Manager::upgrade_to_slip0044_coin_type`] (opened to check it, as Go opens it to cache it) | for the call only | on return; the dropped cache entry's keys on drop |
//! | the plaintext of [`Manager::decrypt_private`] | by the caller | when the caller drops it (`purse_snacl::SecretBytes` wipes itself) |
//! | a derived address's or imported key's private key from [`Manager::private_key`] | by the caller | when the caller drops it ([`addrmgr::ReducedPrivateKey`] wipes itself); the decrypted imported key bytes on return |
//! | a WIF's private key in [`Manager::import_private_key`] | borrowed from the caller, sealed, never copied | left to the caller (Go zeroes the caller's WIF, QK-0040) |
//!
//! Methods that return keys return owned copies ([`purse_hdkeychain::ExtendedKey`]
//! wipes itself on drop), where Go returns the cached pointers, so no
//! caller can zero or keep alive the cache's keys (spec 02 section 12
//! hazard 6). Passphrases are borrowed and left to the caller. The
//! passphrase hasher is HMAC-SHA256 under a 32-byte key drawn at open, in
//! place of Go's keyed BLAKE2b-256: it is memory-only, so any keyed PRF
//! serves (spec 02 section 12 hazard 3); the draw is Go's size, 32 bytes.
//!
//! What the dependencies free unwiped (scrypt's tables and buffers, the
//! HMAC and PBKDF2 stack copies, k256's internal copies, stack copies the
//! compiler makes when values move) is listed in the "Secrets" sections of
//! `purse-snacl` and `purse-hdkeychain` and applies here unchanged; this
//! crate adds no buffer of its own that holds a secret unwiped.
//!
//! Memory stays consistent with the store: dcr-purse does not reproduce the
//! in-memory state Go keeps after a rolled-back transaction (ADR-0002,
//! decision item 2; QK-0733). The account cache holds what the store held
//! in a committed snapshot (keys and type), filled only from read
//! transactions, never from a write transaction that may still roll back;
//! an account's name, which `RenameAccount` changes, is read from its row
//! wherever Go reads its cached copy. The two operations that change an
//! existing account's key material, the coin type upgrade and
//! `SetAccountPassphrase`, drop the cached entry, and the account is cached
//! again only from a snapshot that shows the change. The key an account
//! passphrase opened is tied to the sealed value it belongs to, so after a
//! rolled-back `SetAccountPassphrase` the account reads as the store has
//! it, unlocked or not as it was before.
//!
//! # Randomness
//!
//! Every draw comes from a caller-supplied `purse_snacl::RandomSource`, in
//! Go's order and sizes: creation draws the two master key salts and the
//! two crypto keys (32 bytes each) and one 24-byte nonce per sealed value;
//! open draws the 32-byte hasher key; after open, each sealed value (an
//! account's keys, an imported key's two halves, a script's HASH160, an
//! [`Manager::encrypt_public`] or [`Manager::encrypt_private`] call) draws
//! its 24-byte nonce from the source the manager keeps, and a new account
//! passphrase ([`Manager::set_account_passphrase`],
//! [`Manager::import_voting_account`]) draws its 16-byte Argon2id salt
//! there, in Go's order; a passphrase change
//! ([`Manager::change_passphrase`]) draws the new master key's 32-byte
//! salt, for the private passphrase the 32-byte key of a new passphrase
//! hasher, and the seal's 24-byte nonce, in Go's order. Go also draws 32
//! bytes for a per-account
//! passphrase hasher at each load of an account with its own passphrase and
//! at each `SetAccountPassphrase`; dcr-purse hashes account passphrases
//! with the manager's hasher under a prefix naming the account, which draws
//! nothing (the hash only recognises an unlocked account's passphrase in
//! memory; spec 02 section 12 hazard 3). Go's
//! `Initialize` also runs its upgrades, which this crate does not, so the
//! draw streams of the two sides are not yet aligned (ADR-0002 open item
//! 2): comparisons use the masked profile.
//!
//! # The oracle operation scripts
//!
//! The operation-script format is ADR-0002's (`purse-opscript 1`), and the
//! Go side is the existing oracle `tools/oracle/cmd/oprunner`, which runs
//! dcrwallet's own udb on a bbolt database with injected randomness, clock
//! and processor count, and writes `results.jsonl` and a logical export
//! per checkpoint. A script is one JSON object:
//!
//! ```json
//! {"format": "purse-opscript 1", "name": "example",
//!  "setup": {"network": "simnet", "seed": "<hex>", "pubpass": "public",
//!            "privpass": "private", "rand": "<64 hex>", "clock": 1794990000},
//!  "ops": [{"op": "mgr.unlock", "pass": "private"},
//!          {"op": "mgr.account_extended_priv_key", "account": 0},
//!          {"op": "rt.reopen"}]}
//! ```
//!
//! and each operation's result is `{"i", "op", "ok"}` or `{"i", "op",
//! "err": {"kind", "text"}}`, the text rendered with the daemon's
//! separator `":: "`. The test runner of this crate
//! (`tests/support/runner.rs`) reads the same scripts and writes the same
//! records over [`Manager`] and `purse-store`; it implements the ported
//! operations (setup with a seed or an extended key; `udb.coin_types`,
//! `udb.hd_keys_from_seed`,
//! `udb.validate_account_name`, `udb.is_imported_voting`; every `mgr.`
//! operation of the "ported" rows above; `rt.reopen`, `rt.clock_set`,
//! `rt.clock_advance`; and the transaction store's operations of the
//! first slice with the fixtures `tx.build` and `tx.raw` and the header
//! builder of `store.extend_main_chain`, `tests/support/runner/txops.rs`),
//! and the auxiliary records' operations but `udb.set_vsp_host` and
//! `udb.set_vsp_pub_key` (`tests/support/runner/auxops.rs`, members of
//! groups too, except an `update` group member tolerating the refusal of
//! `udb.set_vsp_ticket`, after which Go keeps a partial write, QK-0122),
//! with Go's address reference forms and import
//! labels, Go's bookkeeping of account passphrases (the export's
//! `[privcheck]` unlocks each account with its own passphrase with the one
//! last set or imported, as Go's runner passes them), `setup.ncpu` as
//! the processor count new Argon2id parameters record
//! ([`Manager::with_num_cpu`]) and Go's bookkeeping of the private and
//! public passphrases (a successful `mgr.change_passphrase` replaces the
//! one later reopens and the export use), the upgrade's report of a
//! dropped account 0 passphrase as `mgr.upgrade_to_slip0044_coin_type`'s
//! `{"passphrase_dropped": true}` (QK-0767; Go's runner states it from Go's
//! state before the call), `update` and `view` groups
//! whose members are transaction store operations
//! (`tests/support/runner/groups.rs`: each member in its own transaction,
//! committed even when the member fails, and the store file put back when
//! an `update` group ends on an untolerated error, which is one
//! transaction's outcome), `rt.checkpoint` (the export of the store as
//! it stands, written beside it as Go's runner writes its snapshot's),
//! and refuses every other operation, address manager operations in a
//! group included, as not yet ported, so a script can never pass by
//! skipping one. A setup may create a watching-only wallet from
//! `xpub` (a key spec, neutered, or `{"raw": string}`, the string passed
//! unparsed); its exports carry no `[privcheck]`, as Go's runner passes no
//! private passphrase for it or after a conversion.
//!
//! # How it is verified
//!
//! - `tests/oracle.rs`: the live differential. Each script runs through
//!   Go's `oprunner` and through the Rust runner; the per-operation results
//!   must be equal (value, or error kind and text; `~r` values, which
//!   follow each side's random stream, by length), and the two exports at
//!   each `rt.checkpoint` (a checkpoint's record is compared by its file
//!   and section names, since each side's digests are of its own export),
//!   then the two final exports, must be equal, section by section, under
//!   the masked
//!   observable profile (ADR-0001 section 8) for every section [`export()`]
//!   covers, with every `[privcheck]` verdict `yes` (a watching-only
//!   wallet's export has no verdict to give: its public keys are proved
//!   equal through the decrypted `xpub` fields). Hand-written scripts
//!   cover each operation's paths; seeded generated scripts
//!   (`tests/support/generator.rs`) vary network (all four), coin type
//!   (half upgrade to SLIP0044 first), seed, passphrases, operation order
//!   and account, address and import arguments (`PURSE_TEST_SEED`,
//!   `PURSE_UDB_SCRIPTS`); a search finds seeds whose purpose or coin type
//!   key is stored stripped (QK-0028) and compares those, account 1
//!   included; generated watching-only scripts (`generate_watching`) cover
//!   wallets created from an account extended public key of either coin
//!   type and wallets created from a seed on either coin type and
//!   converted, on all four networks (`PURSE_UDB_WO_SCRIPTS`); generated
//!   passphrase scripts (`generate_passphrase`) set, change and remove
//!   account passphrases, unlock and lock accounts and import voting
//!   accounts among the other operations, on wallets created from a seed
//!   (either coin type) or from an account extended public key, on all four
//!   networks, with the processor count varied (`PURSE_UDB_PASS_SCRIPTS`);
//!   generated passphrase-change scripts (`generate_change`) change either
//!   passphrase, with the current old passphrase or another, among the
//!   other operations, on wallets created from a seed (either coin type,
//!   with per-account passphrases), from an account extended public key,
//!   or from a seed and converted, on all four networks
//!   (`PURSE_UDB_CHANGE_SCRIPTS`);
//!   the Argon2id parameters, whose thread count the masked profile masks,
//!   are compared unmasked at several processor counts; a setup Go refuses
//!   (an empty private passphrase, a short seed, an extended key that does
//!   not parse or is another network's) fails both runs with the same
//!   text; the recorded divergences (QK-0039, QK-0630, QK-0759, QK-0760,
//!   QK-0761 to QK-0768) are pinned on both sides, and the generated
//!   scripts avoid exactly their inputs; account 0's private field of a
//!   watching-only wallet is opened from both sides' exports after public
//!   passphrase changes, with the creation's public passphrase (QK-0037;
//!   for a wallet made from an extended private key, which only Go
//!   creates, from Go's export, QK-0762).
//! - `tests/frozen.rs`: Go's results and exports for the hand-written
//!   scripts, for a fixed set of 48 generated scripts (every network, both
//!   coin types), for a fixed set of 32 generated watching-only scripts
//!   (every network, every kind), for a fixed set of 24 generated
//!   passphrase scripts (every network, both kinds of wallet) and for a
//!   fixed set of 24 generated passphrase-change scripts (every network,
//!   every kind), checked in with counts and hashes, so the comparison runs
//!   without Go.
//! - `tests/upstream.rs`: the ported upstream tests the crate reaches
//!   (`TestCoinTypes` and `TestCoinTypeUpgrade` of `cointype_test.go`;
//!   `testManagerAPI`, `TestManagerWatchingOnly`, `TestManagerImports` and
//!   `TestManagerEncryptDecrypt` of `addressmanager_test.go` on testnet3
//!   with upstream's seed, passphrases and import vectors; `TestAddresses`'
//!   address vectors, locked and unlocked, on the seed's wallet, on that
//!   wallet converted with `ConvertToWatchingOnly` (upstream's
//!   watching-only runs) and from a wallet created from account 0's
//!   extended public key; `TestImportVotingAccount`, and
//!   `TestImportAccount`'s cases through the voting import;
//!   `TestChangePassphrase`, and its
//!   watching-only branches, which upstream never runs, on the same wallet
//!   converted).
//! - `tests/properties.rs`: create, open and lock invariants over random
//!   seeds and passphrases, the two codecs, the counters against a model of
//!   Go's rules, the order and content of the address lists, the account
//!   cache across rolled-back transactions and the coin type upgrade, the
//!   upgrade leaving no address signed for with another address's key
//!   across a restart (QK-0760), `Unlock`'s choice among failing accounts
//!   (QK-0755),
//!   derivation against `purse-hdkeychain` on every network and coin type,
//!   a manager opened again answering every read as the live one did
//!   after a generated script (watching-only ones included), a wallet
//!   created from an account extended public key answering as the seed's
//!   wallet on every network and coin type (QK-0037, and after a public
//!   passphrase change the creation's passphrase still opening account 0's
//!   private field, and the account's extended private key refused,
//!   writing nothing and drawing nothing, QK-0762), and a conversion
//!   keeping every public answer while the store loses every private key
//!   but the accounts' that the private crypto key or a voting account's
//!   passphrase seals (QK-0761, QK-0766), leaving no removed value in any
//!   row or in a backup of the converted store and an account row that
//!   does not decode as it was, or, when it fails, changing nothing, not
//!   even the rows of accounts with their own passphrase it had already
//!   rewritten; the coin type upgrade reporting exactly the account 0
//!   passphrase it drops
//!   (QK-0767);
//!   per-account passphrases following a model of Go's rules through random
//!   sets, changes, removals, unlocks, locks, wallet locks and reopens,
//!   with rolled-back changes leaving store and memory as they were
//!   (QK-0733); a snapshot from before a passphrase change keeping the
//!   cache current; voting account numbers never wrapping onto account 0
//!   (QK-0041); passphrase changes following a model of Go's rules through
//!   random changes of either passphrase, locks, unlocks and reopens, with
//!   the lock state kept, every key the crypto keys seal unchanged, a failed
//!   change, or a refused change to an empty private passphrase (QK-0768),
//!   writing and drawing nothing and a successful one writing only its
//!   half's two rows and drawing Go's sizes, and an
//!   account its own passphrase unlocked staying unlocked. Unit
//!   tests in `src/addrmgr/mod.rs` check that a locked
//!   manager holds no private key, crypto key, passphrase hash or master
//!   key through random sequences (account passphrases and private
//!   passphrase changes included: a wallet lock drops every key the account
//!   passphrases opened, and the cache never holds one), and
//!   that a converted one holds none of them, nor any sealed private key,
//!   nor any key an account passphrase opened, nor the sealed key of a
//!   BIP0044 account with its own passphrase.
//! - The transaction store: `tests/oracle.rs` runs the hand-written
//!   scripts `28-txstore-chain`, `29-txstore-records`, `30-txstore-stake`
//!   and `31-txstore-groups` (`update` and `view` groups) of the first
//!   slice, `32-txstore-credits`, `33-txstore-credit-edges` and
//!   `34-txstore-utxo` of the credit slice, and generated ones
//!   (`tests/support/txgen.rs`, all four networks, `PURSE_UDB_TX_SCRIPTS`,
//!   and drawing the credit slice's operations too,
//!   `PURSE_UDB_TX_CREDIT_SCRIPTS`; the same with runs of operations in
//!   groups, `PURSE_UDB_TX_GROUP_SCRIPTS`,
//!   `PURSE_UDB_TX_CREDIT_GROUP_SCRIPTS`), comparing results and the
//!   chain and transaction sections of the export, credits, debits and
//!   balances included (a Go `Panic` matched by any Rust error, outside a
//!   tolerating group member, which the runner refuses:
//!   `a_group_member_may_not_tolerate_a_panic_stand_in`); four oracle
//!   tests pin Go defects, two reproduced on all four networks,
//!   `an_unmined_coinbase_conflicts_with_votes` (QK-0780, with coinbases
//!   and treasury spends; QK-0087) and
//!   `a_transaction_mined_twice_and_rolled_back_is_mined_and_unmined`
//!   (QK-0781), and four divergences on simnet,
//!   `a_same_block_spend_chain_invalidated_leaves_no_dangling_unspent`
//!   (QK-0082, fixed),
//!   `credits_off_the_main_chain_or_without_a_record_are_refused`
//!   (QK-0783),
//!   `invalidated_credits_of_a_detached_block_do_not_come_back` (QK-0782)
//!   and `a_spender_below_its_funder_is_refused_where_go_fabricates_a_credit`
//!   (QK-0101);
//!   `tests/frozen.rs` replays the hand-written scripts and two
//!   fixed generated sets (`tests/data/txstore_genvectors.txt`,
//!   `tests/data/txstore_credit_genvectors.txt`); `tests/txstore.rs` holds
//!   the upstream tests (`TestSetBirthState`, `TestStakeInvalidationOfTip`,
//!   `TestInsertsCreditsDebitsRollbacks`, `TestCoinbases`, on Go's header
//!   chain; the hand-written script `38-upstream-tx-tests` replays the
//!   steps and queries of the last three through the oracle, and
//!   `tests/guards.rs` holds the `TestCoinbases` port's balances to Go's)
//!   and the property tests (row
//!   and credit invariants after every
//!   operation of random sequences, the UTXO queries against the rows,
//!   failed operations writing nothing, reopens, record round trips, the
//!   dependency order of `UnminedTxs`, ADR-0019's filter rules, the guard
//!   of the later tables, the rows the range walks read); and
//!   `fuzz/fuzz_targets/udb_tx_record.rs` fuzzes `TxRecord::new`. Details
//!   in [`txstore`], "How the first slice is verified" and "The credit
//!   slice".
//! - The auxiliary records: `tests/oracle.rs` runs the hand-written
//!   scripts `39-aux-vsp`, `40-aux-treasury` and `41-aux-agendas` (every
//!   operation, `update` and `view` groups, reopens) and
//!   `auxiliary_records_at_the_key_limit_match_go`, comparing records and
//!   the `[agendas]`, `[treasury]`, `[vsp]` and `[orphans]` sections, and
//!   pins QK-0122's partial write
//!   (`a_tolerated_vsp_ticket_refusal_is_refused_by_the_rust_runner`);
//!   generated scripts (`tests/support/auxgen.rs`: every auxiliary
//!   operation over small pools of tickets, spends, pi keys, hosts and
//!   agendas, with reopens, on all four networks), alone
//!   (`generated_aux_scripts_match_go`) and with their operations in
//!   `update` and `view` groups tolerating `NotExist`, `Invalid` or `IO`
//!   (`generated_aux_group_scripts_match_go`); `tests/frozen.rs` replays
//!   the hand-written scripts and a fixed generated set of 8
//!   (`aux_genvectors.txt`); `tests/upstream.rs` holds `vsp_test.go`;
//!   `tests/auxrecords.rs` checks every reader against a model of
//!   dcrwallet's rules through random operations, refusals writing
//!   nothing, abandoned transactions and reopens (rows and export equal
//!   across them); unit tests are in each module. The seeds and counts of
//!   the campaigns are in `docs/PARITY.md`.
//! - `tests/export.rs`: the export parses under `purse-export-check`, its
//!   registry equals the parser's, it writes the auxiliary records in the
//!   format's order, it refuses a store holding rows it does not export or
//!   a `vsp_fee_status` index that disagrees with its tickets, and its
//!   options' `Debug` hides the passphrases.
//! - `tests/measure.rs`: not a parity test, but the measurement of
//!   `purse-store` and this address manager against dcrwallet's udb over
//!   bbolt on the same generated wallet (ADR-0003, addendum "`purse-store`
//!   against dcrwallet on the same workload"). Its small run holds the two
//!   wallets' canonical exports equal line for line and every timed
//!   query's answer equal, in every test run; the measured run is ignored.

#![forbid(unsafe_code)]

pub mod addrmgr;
pub mod agendas;
pub mod export;
pub mod treasury;
pub mod txstore;
pub mod vsp;

pub use addrmgr::Manager;
pub use export::export;
