// Copyright (c) 2015-2017 The btcsuite developers
// Copyright (c) 2015-2025 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The transaction store: a port of dcrwallet's `udb.Store`
//! (`wallet/udb/txmined.go`, `txunmined.go`, `txquery.go`, `cfilters.go`,
//! `kahnsort.go`, with the formats of `txdb.go`; specs 03 and 04) over
//! `purse-store`.
//!
//! This module is written in slices. The crate documentation has the table
//! of every Go entry point with the slice it belongs to; this page is the
//! design of the first slice, the rows it keeps, and the rules that bound
//! it, then of the credit slice ("The credit slice" below), which fills in
//! the steps the first slice left, of the commitment and multisig slice
//! ("The commitment and multisig slice" below), and of the selection slice
//! ("The selection slice" below), which ports input selection, `Spender`
//! and QK-0734's fix and with them the last of the store's listed
//! remainder. The first slice
//! is written as designed here; where the code departs from the design as
//! first drafted, this page says so. The sections on the first slice
//! describe it as it was written: where they say that a credit, commitment
//! or multisig table is empty or guarded, a later slice has since replaced
//! the step; since the commitment and multisig slice no table is guarded
//! and the guard (`LATER_TABLES`) is gone.
//!
//! # What bounds the first slice
//!
//! dcrwallet records where each wallet output's script lies in the
//! serialized transaction, `wire.MsgTx.PkScriptLocs`, in every credit row
//! it writes, and `purse-store`'s credit rows keep that location too
//! ([`purse_store::ScriptLoc`] in [`purse_store::CreditRow`],
//! [`purse_store::UnspentRow`] and [`purse_store::UnminedCreditRow`];
//! ADR-0003 Decision item 2). The port of `PkScriptLocs` in dcroxide was not
//! at dcr-purse's dcroxide pin when this slice was written, so no operation
//! that writes a credit row was ported, and this crate computes no script
//! offset of its own. *(Since 2026-11-23 the pin is dcroxide `1587ca1`,
//! which has `MsgTx::pk_script_locs`; nothing here calls it yet, and the
//! credit-writing operations are the next slice's.)*
//! Exactly two places in the Go code compute one:
//!
//! - `AddCredit` (`txmined.go:1384-1558`): every one of its three writes
//!   records `PkScriptLocs()[index]`, the invalidated credit (`ic`, `:1420`),
//!   the unmined credit (`mc`, `:1501`) and the mined credit (`c`, `:1534`).
//!   **Blocked on script offsets.**
//! - `Rollback`'s output step (`txmined.go:2122`): a mined credit that
//!   returns to the unmined set is rewritten as an unmined credit with the
//!   offset computed again. **This step is blocked on script offsets;** the
//!   rest of `Rollback` is in the first slice (see "Credit-dependent steps"
//!   below for how the two are kept apart).
//!
//! Every other reader of a script location (`outputCreditInfo`,
//! `fastCreditPkScriptLookup`, `moveMinedTx`'s script fetch,
//! `PreviousPkScripts`) reads rows only those two write, so it comes with
//! the credits. The credits in turn feed the balance tables ADR-0003
//! Decision item 4 keeps in the same transaction as the data (`balances`,
//! `balance_heights`, by class), and so do the ticket commitments
//! (`LockedByTickets`); and `Rollback` updates multisig records only inside
//! its credit and debit steps (`txmined.go:1972-1984`, `2080-2094`,
//! `2158-2169`), so the multisig lifecycle across a reorganisation cannot be
//! compared with Go before the credits exist. The first slice is therefore
//! the part of the store that holds no credit, no debit, no ticket
//! commitment and no multisig output:
//!
//! - store creation (`createStore` and the rows dcrwallet's upgrades 10, 11,
//!   15 and 26 add, written directly in their final form, as the address
//!   manager is);
//! - the main chain: headers, block records, committed filters, the tip, the
//!   processed-transactions marker, the birthday, with ADR-0019's decided
//!   store rules (FB4 as FB11 (i) widens it, FB9 and FB10, and FB12 (a)'s
//!   pending states);
//! - transaction records, mined and unmined: `NewTxRecord`, `InsertMinedTx`,
//!   `InsertMemPoolTx`, `RemoveUnconfirmed`, `PruneUnmined`, `SetPublished`,
//!   the unmined-inputs index, the ticket set, `Rollback` without its credit
//!   step, and the stake (in)validation flag `ExtendMainChain` sets on the
//!   parent block;
//! - the queries over them: transaction details and lookups, ranges over
//!   blocks and transactions, the unmined set in dependency order, the
//!   ticket iterator and `TicketDetails`, and the unspent count;
//! - the logical export's sections for those rows.
//!
//! # Credit-dependent steps
//!
//! The tables of the later slices are `credits`, `credits_invalidated`,
//! `unspent`, `unmined_credits`, `debits`, `debits_invalidated`,
//! `commitments`, `commitments_unspent`, `multisig_outs`,
//! `multisig_unspent`, `balances`, `balance_heights` and `acct_history`.
//! No operation of the first slice writes any of them, and the export
//! refuses a store in which one holds a row (ADR-0001 principle P1, as the
//! address manager's export refuses rows it does not export), so in every
//! store the first slice can produce they are empty.
//!
//! Where a ported Go algorithm reads or writes one of those tables at one
//! of the steps listed below, the port checks, before its first write,
//! that every table Go reads at those steps is empty: finding none holding a row, it goes on exactly as Go
//! does when each of its lookups misses (which is what Go does in a store
//! that holds no credit); finding a row, it returns an error of kind `Bug`
//! naming the step and the table, and has written nothing. Such a row can
//! only come from a write outside this module. The check is one emptiness
//! test per table and operation, where Go makes one lookup per input or
//! output (one per input of `InsertMinedTx`, one per output of
//! `RemoveUnconfirmed`, and so on): fewer reads than Go's, and none per
//! row. (The design as first drafted made Go's own lookups; the emptiness
//! test is cheaper and finds the same rows.) Stake (in)validation checks
//! only when the block holds wallet transactions, as Go reads the tables
//! only for those. `UnspentOutputCount` is not such a step: it only counts
//! the rows of `unspent`, as Go's `KeyN` does, so it has no guard and
//! counts a row there as Go would. The steps that later slices fill in
//! are, in Go's order:
//!
//! | Operation | Step left to a later slice |
//! |---|---|
//! | `ExtendMainChain` → `stakeValidate`, `stakeInvalidate` | moving a regular transaction's credits and debits between the live and invalidated tables, the unspent entries and the mined balance (spec 04 §5.4); the block's flag is in the first slice |
//! | `InsertMinedTx` | the debit loop over inputs that spend an unspent credit (spec 04 §5.6 step 6) |
//! | `moveMinedTx` | the unspent lookup of each input (dead in Go, Q-7) and the move of each unmined credit to a mined one (§5.7 step 6) |
//! | `Rollback` | the coinbase branch's credits and multisig records, the debit restoring of each input, the credit step (**blocked on script offsets**), the commitment flags of a vote or revocation, and the removal of unmined spenders of a dropped coinbase credit (§5.8) |
//! | `RemoveUnconfirmed` | the unmined credit of each output, a ticket's commitments, a vote's or revocation's commitment flags, the multisig record of each input (§5.11) |
//! | `TxDetails`, `UniqueTxDetails`, `RangeTransactions` | the credit and debit lists (§5.16); both are empty in the first slice |
//! | `TicketIterator.Next`, `TicketDetails` | the credit of the ticket's output 0 (§5.22); without one Go takes the unmined spender, which the first slice does |
//!
//! `Rollback` removes the unmined spenders of a dropped coinbase only for
//! outputs that were credits (`coinBaseCredits`, `txmined.go:1944-1951`,
//! `2205`), so in
//! a store without credits it removes none, as Go does.
//!
//! # Rows
//!
//! dcrwallet's `wtxmgr` bucket tree (spec 03 §4.3, §4.4) against the
//! `purse-store` tables (ADR-0003 Decision item 2 and its addenda "Chain
//! tables keyed by height, as built" and "Schema version 1 as built"). No
//! schema change is needed: schema version 1 has every table and row the
//! first slice writes.
//!
//! | dcrwallet (bbolt) | purse-store | First slice |
//! |---|---|---|
//! | `tip` (block hash) | `meta` row `tip`, [`purse_store::Tip`] (height and hash) | written by creation, `ExtendMainChain` and `Rollback` through the chain operations; `MainChainTip` reads one row where Go reads the tip and its header |
//! | `lasttxsblock` | `meta` row `processed` | creation (the genesis hash, as upgrade 11 copies the tip), `UpdateProcessedTxsBlockMarker`; absent reads as the zero hash, as in Go |
//! | `havecfilters` | `meta` row `have_cfilters` | 1 from creation on, never 0 (ADR-0019 FB9 (a): "no filter at or above the birthday height is missing") |
//! | `birthstate` (45 bytes) | `meta` row `birth`, [`purse_store::BirthState`] | creation (genesis, height 0, no flag, Go's zero time −62135596800, as upgrade 26 writes it, QK-0114), `SetBirthState` |
//! | `h` (every header ever connected, by hash) | `headers` (main chain, by height) and `headers_detached` (by hash), through `block_by_hash` | the chain operations ([`purse_store::chain`]); a lookup by hash is the main-chain row or the detached one, as Go's `h` holds both |
//! | `b` (height → hash, time, vote bits, invalid byte, transaction hashes) | `blocks` ([`purse_store::BlockRow`]: hash, time, vote bits, invalid flag, the count of wallet transactions) and `history` ([`purse_store::HistoryKey`] (height, position) → transaction hash) | one `history` row per wallet transaction of a block, in insertion order, where Go rewrites the block record with the hash appended; the block row's count is the next position |
//! | `cf` (by hash) | `cfilters` (main chain, by height) and `cfilters_detached` (by hash) | creation (the genesis filter), `ExtendMainChain`, `InsertMissingCFilters` and `ImportCFiltersV2` (`WriteTx::put_cfilter_at`) |
//! | `t` (hash ‖ height ‖ block hash → received ‖ transaction) | `tx_mined` ([`purse_store::TxRecKey`] (hash, height) → [`purse_store::TxMinedRow`]: position in the block, received time, the transaction's bytes) | the block hash is the main-chain block's at that height; Go's three-part key and this two-part one name the same records, because a block leaves the main chain only through `Rollback`, which deletes its records first |
//! | `m` | `tx_unmined` ([`purse_store::TxUnminedRow`]) | same key set and order as `m` |
//! | `up` | `unpublished` | presence only |
//! | `mi` | `unmined_inputs` (outpoint → spender hash) | same key set and order as `mi` |
//! | `tix` (value −1, never read) | `tickets` (no value) | written on first sight of a ticket, never deleted (QK-0056) |
//! | `bal` | — | not kept (ADR-0003, addendum "Schema version 1 as built"); the first slice never changes it in Go either, since every change to it is in a credit step |
//! | `date` | — | not kept (same addendum) |
//! | `c`, `ic`, `u`, `d`, `id`, `mc`, `cmt`, `cmu`, `ms`, `mu` | `credits`, `credits_invalidated`, `unspent`, `debits`, `debits_invalidated`, `unmined_credits`, `commitments`, `commitments_unspent`, `multisig_outs`, `multisig_unspent` | empty in the first slice (above) |
//! | — | `balances`, `balance_heights`, `acct_history` | empty in the first slice; maintained with the credits (ADR-0003 Decision item 4) |
//!
//! The rows that tie the chain tables to the wallet's: a block row's count
//! equals the number of `history` rows at its height; each `history` row
//! (h, p) names a hash whose `tx_mined` row (hash, h) records position p,
//! and each `tx_mined` row has its `history` row; every `unmined_inputs`
//! row names a `tx_unmined` row whose transaction spends that outpoint
//! (spec 03 I8); and, with the credits, every `unmined_credits` row belongs
//! to a `tx_unmined` row. Spec 03 I7, that a hash is never unmined while a mined
//! record has it, is not one of them: Go records one transaction mined in
//! two blocks when `InsertMinedTx` is called for each (its key holds the
//! block), and a `Rollback` of the higher block returns it to the unmined
//! set while the lower keeps its record, which the port reproduces
//! (QK-0781). The
//! converse of the I8 rule above, that every input of an unmined
//! transaction (a vote's stakebase excepted) has its row naming that
//! transaction, is spec 03 I8 too, but Go breaks it in two cases the port
//! reproduces: `Rollback` registers a returned transaction's inputs over
//! another unmined spender's rows, and `RemoveUnconfirmed` deletes the row
//! of every input without checking which transaction it names (QK-0060);
//! the property tests do not hold it, and the differential compares the
//! rows with Go's. A checker of the rest, beside the chain invariants,
//! runs in the property tests after every operation (`tests/txstore.rs`,
//! `check_rows`).
//!
//! Block rows change in place only in their flag and their count, through
//! the generic typed put of [`purse_store::Blocks`] with the hash kept, so
//! the chain invariants of [`purse_store::chain`] (heights and hashes) hold
//! and `ReadTx::check_chain` still checks them. Blocks connect and detach
//! only through the chain operations ([`purse_store::WriteTx::connect_block`],
//! [`purse_store::WriteTx::disconnect_from`]), which move headers and
//! filters between the main-chain and detached tables in the same
//! transaction as the wallet rows. A detached block's `history` rows and
//! its transactions' `tx_mined` rows are the caller's to remove (the chain
//! operations move chain rows only); `Rollback` removes them before it
//! detaches the blocks.
//!
//! # The operations
//!
//! A [`TxStore`] value ([`TxStore::new`], from the network's
//! [`crate::addrmgr::ChainParams`]) holds what Go's `Store` holds that the
//! first slice reads: the network's genesis block hash, its serialized
//! header and its committed filter (built once, when the value is made,
//! from `dcroxide-chaincfg`'s genesis block with `dcroxide-gcs`'s
//! `blockcf2::regular`, as upgrade 15 builds it, `upgrades.go:1314-1320`);
//! the later slices add the maturities. Like Go's, it has no other state:
//! every method takes the caller's transaction, a write transaction for a
//! change and either kind for a query ([`TxRead`], the address manager's
//! [`crate::addrmgr::StoreRead`] pattern widened to the range reads and
//! chain markers the queries need), never begins one, and leaves the
//! caller to abandon the transaction after an error, as Go's callers roll
//! back. [`TxStore::create`] writes the store's rows; a wallet's creating
//! transaction runs it after `Manager::create`, as Go's `Initialize` runs
//! both in one transaction (the free functions `initialize` and
//! `initialize_watch_only` of the first draft are not written: the
//! operation-script runner and the tests make the two calls).
//! There is no lock (spec 04 §7; ADR-0008). Go's `PruneUnmined` is the one
//! operation whose caller commits after an error (QK-0059): the port returns
//! the hashes removed before the failure with the error, and its writes so
//! far stay in the transaction, as Go's do.
//!
//! The first slice links, besides what the crate links already,
//! `dcroxide-stake` (with `default-features = false`, so neither
//! `dcroxide-database` nor redb comes with it: ADR-0006 rule 5, note of
//! 2026-11-20) for `determine_tx_type`, `is_sstx`, `is_ssgen` and
//! `ssgen_block_voted_on`; `dcroxide-gcs` (default features off) for
//! `FilterV2` and `blockcf2`; `dcroxide-standalone` for the coinbase rule
//! of `compat.IsEitherCoinBaseTx`; and `dcroxide-chainhash`. All are at the
//! workspace's dcroxide pin, which had no `pk_script_locs` when the slice
//! was written and has one since 2026-11-23 (`1587ca1`); the slice does not
//! call it.
//!
//! Records: `TxRecord::new(bytes, received)` is `NewTxRecord`. It keeps
//! the caller's bytes verbatim, trailing bytes after the transaction
//! included, as Go keeps the caller's slice and its deserializer stops at
//! the transaction's end, and computes the type once (Go's `InsertMinedTx`
//! computes it again and its other operations trust the record's field,
//! QK-0058, QK-0095; built by the constructors, the two always agree, and
//! the Rust record's fields are not writable). `TxRecord::from_msg_tx` is
//! `NewTxRecordFromMsgTx`, serializing as the message's serialization type
//! says. Received times are Unix seconds, as Go stores and returns them.
//!
//! Main chain, in Go's order of checks and with Go's texts:
//! `extend_main_chain` makes `ExtendMainChain`'s refusals itself (block 0,
//! the `havecfilters` gate, already recorded, not a child of the tip, the
//! wrong height) before it calls `connect_block`, so the store's own
//! refusals, whose order differs, are never reached for what Go refuses.
//! It flips the parent block's flag by the new header's vote bit, then
//! connects the block with an empty row and the filter keyed by the first
//! 16 bytes of the merkle root. The filter is optional (ADR-0019 FB4): a
//! block without one is accepted below the birthday height and refused at
//! or above it. The birthday height is the `birth` row's height when the
//! birthday is resolved or pending from a height (FB12 (a)), and 0 with no
//! row (FB10). While a birthday set from a time T alone is pending there
//! is none, and a block without its filter is accepted when its own
//! timestamp is not after T and refused otherwise, the block resolving the
//! birthday among them (FB11 (i)). A row pending from both a height and a
//! time, for which ADR-0019 states no rule, is read as below the birthday
//! only for a block below both (choice 4 below). Go requires a filter (a
//! nil one panics), and ADR-0002's scripts always pass one.
//!
//! `missing_cfilters_height` walks from the birthday height so read
//! (ADR-0019 FB9 (a)), where Go walks from genesis, and answers `NotExist`
//! while a birthday set from a time alone is pending (FB12 (a)): a
//! deliberate divergence, recorded in PARITY.md, visible only when a
//! filter is missing, which no operation script can make. Every other
//! filter operation is Go's, `InsertMissingCFilters` included (it always
//! refuses in a store dcr-purse creates, whose `have_cfilters` is 1, as it
//! does in one Go creates).
//!
//! `rollback(height)` reads the `history` rows from the rollback height up
//! in one range (a block without wallet transactions has none, so costs no
//! read, where Go reads its block record) and takes them block by block
//! from the tip down: it moves each non-coinbase transaction's record bytes verbatim to
//! `tx_unmined`, registers its inputs in `unmined_inputs` (a vote's input 0
//! excepted), drops coinbase-like transactions, deletes the `tx_mined` and
//! `history` rows, then detaches the blocks with `disconnect_from`, which
//! moves their headers and filters to the detached tables and sets the tip;
//! it returns each block's transactions in record order, highest block
//! first, for the blocks that had any (Go's result is a map with an entry
//! only for those, which the operation-script runner sorts by block hash).
//!
//! Unmined set: `unmined_txs` orders the records by Kahn's algorithm with
//! ties broken by hash in internal byte order (ADR-0012 Rule 3(a): an order
//! under a constraint is the constrained order with ties broken by key;
//! Go's follows its map iteration, QK-0057, QK-0092); a PARITY.md line
//! names the order. `remove_unconfirmed` keeps Go's visit order (the
//! spenders of each output, ascending, before the output's own deletes)
//! with an explicit stack instead of recursion (spec 04 §12.3 hazard 6).
//!
//! Queries: `iterate_tickets` returns the tickets as a vector (Go's
//! `TicketIterator` yields them one by one from a cursor; the answers and
//! their order are the same). `range_transactions` and `range_blocks` take
//! no context
//! (ADR-0008: the API is synchronous and takes none); the callback ends a
//! walk by returning `true`, as Go's does, and a caller that must stop on
//! cancellation does so in its callback. `unique_tx_details` with a block
//! reads that height's block row to compare its hash, since `tx_mined`'s
//! key has no block hash: one read per call that Go's 68-byte key makes
//! unnecessary. `iterate_tickets` reads, for the same reason, the block
//! row of each distinct block holding a mined ticket, to give the block's
//! hash (one read per block, not per ticket; Go reads none).
//!
//! # Panics and errors
//!
//! No method panics. Where Go panics (spec 04 §8.3), the port returns an
//! error under ADR-0012 Rule 2: kind `IO` for missing or corrupt stored
//! state, `Invalid` for an argument, with Go's text where the next line of
//! Go would have produced an error (QK-0086: `InsertMinedTx` and
//! `GetMainChainBlockHashes` with a stale header give the `Invalid` text of
//! the comparison that follows; `Rollback` to a height two or more above
//! the tip, or below 0, gives an `Invalid` error and writes nothing).
//! `MainChainTip` and `BlockInMainChain` return a `Result`, which Go's do
//! not (Rule 2(c)). `InsertMinedTx` of an unmined transaction into a block
//! already stake-invalidated, where Go panics with "unimplemented:
//! moveMinedTx called on a stake-invalidated tx", returns `Invalid` with
//! that text and writes nothing; the refusal stays, and whether the move is
//! recorded instead is decided in Phase 5 with the rescan (choice 2 below,
//! decided by the project owner on 2026-11-22 and 2026-11-24). Each such
//! difference gets its PARITY.md line, and the comparison accepts any
//! error against Go's `Panic` record by record (ADR-0002 section 3). Go
//! never tolerates a panic, so a group member whose operation returns such
//! an error may not tolerate its kind: the Rust runner refuses that script
//! as a script fault (`tests/support/runner/groups.rs`), since its group
//! would go on where Go's stops.
//!
//! Go's log lines (spec 04 §8.4) are not written: the crate has no logger
//! yet, and the address manager writes none either (choice 5 below).
//!
//! **A failed operation writes nothing.** Every refusal and every guard
//! stop is made before the operation's first write, so the rows after an
//! error are the rows before it, even in a transaction the caller then
//! commits. Go writes before two of its refusals, and its callers abandon
//! the transaction on the error, so the two agree on every store a
//! dcrwallet caller commits (that a refusal of these two writes nothing
//! was decided by the project owner on 2026-11-22): `InsertMissingCFilters` and
//! `ImportCFiltersV2` put each filter before they check the next block or
//! filter, where `insert_missing_cfilters` and `import_cfilters_v2` check
//! every one first, in Go's order and with Go's errors (an operation
//! script that tolerates such an error in a group sees Go's partial
//! writes, which is why the generated scripts never do). Two orders are
//! the port's own:
//! `insert_mined_tx` writes a ticket's row after the guard of the move or
//! removal that follows (Go records the ticket first and has no guard),
//! and `extend_main_chain` writes the tip's stake validity after the store
//! has connected the block, since the store refuses blocks Go would record
//! (a hash already in the main chain at another height, a filter over
//! bbolt's value limit; PARITY.md, `purse-store`), where Go writes it
//! first. The exceptions are errors of the store itself and stored state
//! that is corrupt (kind `IO`: a record that does not decode, a row that
//! another names and that is missing), which `rollback` and
//! `remove_unconfirmed` (and so `prune_unmined`, the removal of double
//! spends in `insert_mined_tx` and the vote eviction in
//! `insert_mem_pool_tx`) find only as they go; `PruneError` carries the
//! removals made before such an error, which Go's caller commits
//! (QK-0059). The vote eviction in `insert_mem_pool_tx` cannot be followed
//! by a refusal: a vote's stakebase is never indexed under another vote,
//! so a vote evicts only at its ticket input, its last, and a revocation
//! has a single input.
//!
//! # Speed
//!
//! The per-row costs added to Go's are the bounded over-read of the paged
//! walks and the block-row reads of `iterate_tickets`, one per distinct
//! block holding a mined ticket (both below). Existence tests read no
//! value, as Go's `Get(k) != nil` and key-only cursor loops copy none:
//! `exists_tx`, `exists_tx_mined_or_unmined`, `tx_block_height`,
//! `own_ticket`, `insert_mem_pool_tx`'s and `insert_mined_tx`'s checks
//! for a record already there, `unmined_tx_hashes` and
//! `missing_cfilters_height`'s per-block filter test use `TxRead::contains`
//! and `TxRead::keys` (purse-store's `contains` and `keys`), so no record
//! or filter is copied or decoded to learn that it exists
//! (`existence_tests_read_no_value` pins the counts). Against Go's layout:
//!
//! - recording a mined transaction writes one fixed-size `history` row and
//!   rewrites the 47-byte block row, where Go rewrites a block record that
//!   grows by 32 bytes per transaction, so filling a block of n wallet
//!   transactions costs O(n) bytes, not O(n²);
//! - `MainChainTip` is one read (height and hash in one row), where Go reads
//!   the tip and then its header;
//! - stake (in)validation rewrites the 47-byte block row whatever the
//!   block's wallet transactions, where Go rewrites the whole record;
//! - `rollback` moves record bytes from `tx_mined` to `tx_unmined` without
//!   re-serializing them, as Go copies the value, and reads no row of a
//!   block without wallet transactions;
//! - a record read back (`TxRecord::stored`) is decoded and typed, as Go's
//!   `readRawTxRecord` does, and serialized again only if a caller asks
//!   for its bytes (Go's has none), so no query pays a serialization;
//! - `range_blocks`, `range_transactions` and `missing_cfilters_height`
//!   walk their range a page at a time (`store::walk`, and for
//!   `missing_cfilters_height`, which needs only the heights,
//!   `store::walk_keys`; pages doubling from
//!   one row to 512), stopping where the callback or the first missing
//!   filter stops them, so a walk stopped after n rows reads at most
//!   max(2n − 1, n + 511) where Go's cursor reads n, and a full walk makes
//!   about one range lookup per 512 rows where Go's cursor makes one in
//!   all (`range_walks_read_what_they_visit` pins the counts);
//! - `iterate_tickets` reads one block row per distinct block holding a
//!   mined ticket, to give its hash, where Go's 68-byte `t` key carries it
//!   and `TicketIterator.Next` reads no block record: the price of
//!   ADR-0003's `tx_mined` key (hash, height), which leaves the block hash
//!   out. Each height's hash is kept once read, so 64 tickets mined in one
//!   block cost one such read, not 64
//!   (`iterate_tickets_reads_each_block_once`); tickets spread over many
//!   blocks still cost one each. Removing the read would put the block
//!   hash in a row ADR-0003 gives none (the `tickets` value or the
//!   `tx_mined` key); not proposed;
//! - a lookup by block hash (`GetBlockMetaForHash`, `BlockInMainChain`, a
//!   detached header) takes two reads (`block_by_hash`, then the row),
//!   where Go's `h` takes one: measured in ADR-0003's addendum "Chain tables
//!   keyed by height, as built" (1.62 µs against 1.00 µs warm), the price of
//!   height keys, which made initial sync 3.1 times faster;
//! - `range_transactions` walks the `history` rows of the range, then
//!   reads the `tx_mined` row per transaction and the block row per block
//!   that has records, where Go reads every block record in the range and
//!   then each record: on a chain of mostly empty blocks it reads only the
//!   blocks with wallet transactions, which Go cannot (ADR-0003's history
//!   index). A faster layout is possible under ADR-0003's tables: a `history` value
//!   carrying the record (received time and bytes) would make a range one
//!   scan with no second lookup, at the cost of moving the bytes out of
//!   `tx_mined` (keyed for `TxDetails` by hash). Not proposed before the
//!   measurement ADR-0003 M3 calls for at Phase 3's exit.
//!
//! No timed run is part of this slice.
//!
//! # How the first slice is verified
//!
//! As the address manager is (crate documentation, "How it is verified"):
//!
//! - **Operation scripts.** The Rust runner (`tests/support/runner.rs`,
//!   `tests/support/runner/txops.rs`) has every first-slice operation with
//!   Go's arguments and result shapes (ADR-0002 sections 2 and 4), the
//!   fixtures `tx.build` (regular, coinbase, ticket, vote and revocation
//!   outputs: `pay` with a stake tag, `script`, `commitment`, `voteblock`,
//!   `votebits`, `opreturn`) and `tx.raw`, and `store.extend_main_chain`'s
//!   header and filter builder and its `again` form (ADR-0002 section 6).
//!   Setup creates the transaction store with the address manager, as Go's
//!   `Initialize` does, so every script, the address manager's included,
//!   has the genesis rows on both sides. The `update` and `view` groups
//!   (with `tolerate`) take the slice's operations
//!   (`tests/support/runner/groups.rs`); `rt.checkpoint` is **not yet** in
//!   the runner, which still refuses it as a script fault, so no export is
//!   compared mid-script. The store operations draw
//!   no randomness, so their results compare exactly. A Go record of kind
//!   `Panic` is matched by any Rust error (ADR-0002 section 3), outside a
//!   tolerating group member: a member may not tolerate the kind of an
//!   error that stands in for a Go panic, and the runner refuses a script
//!   whose member does (`a_group_member_may_not_tolerate_a_panic_stand_in`,
//!   with Go's group ending at the panic).
//! - **Differential.** `tests/oracle.rs` runs the hand-written scripts
//!   `28-txstore-chain`, `29-txstore-records` and `30-txstore-stake` on
//!   simnet (chain, refusals, reorganisations and stake invalidation,
//!   headers and filters, mined and unmined records, conflicts and double
//!   spends, the vote eviction, pruning, tickets, votes and revocations,
//!   every query, the birthday and the marker, Go's panics),
//!   `31-txstore-groups` on testnet3 (groups that commit, that roll back
//!   on an error or on a Go panic, and members tolerating `Exist`,
//!   `DoubleSpend`, `Invalid` and `NotExist`), and seeded generated ones
//!   (`tests/support/txgen.rs`, `generate_txstore`: random chains with
//!   disapproving vote bits, regular, coinbase, ticket, vote and revocation
//!   transactions over wallet and foreign outpoints, inserted unmined,
//!   mined into the tip or lower blocks, removed, pruned, published,
//!   rolled back to every height including invalid ones, re-extended, with
//!   reopens, on all four networks, a quarter with the chain sections in
//!   digest mode; `PURSE_UDB_TX_SCRIPTS`) through Go's `oprunner` and the
//!   Rust runner; per operation the results must be equal, and the final
//!   exports equal under the masked profile, the chain and transaction
//!   sections compared. `generated_txstore_group_scripts_match_go` runs
//!   generated scripts with runs of operations in groups, some members
//!   tolerating `Exist`, `DoubleSpend` or `NotExist` (never on the two
//!   operations above whose failures write in Go): Go keeps a tolerated
//!   error's writes and so does the Rust runner inside a group, so equal
//!   exports show that those failures write nothing on either side. The generator draws every operation of the slice
//!   (`txstore_generator_draws_only_ported_operations` holds it to the
//!   runner's list). `an_unmined_coinbase_conflicts_with_votes` pins, on
//!   all four networks, Go's refusal of a vote while a coinbase or a
//!   treasury spend is unmined (QK-0780, reproduced) and `Rollback`
//!   dropping a mined treasury spend (QK-0087), and
//!   `a_transaction_mined_twice_and_rolled_back_is_mined_and_unmined` a
//!   transaction left mined and unmined (QK-0781, reproduced).
//! - **Export.** `COVERED` has `[chain]`, `[blocks]`, `[headers]`,
//!   `[cfilters]` (full and digest modes), `[txs]`, `[unpublished]`,
//!   `[uinputs]` and `[tickets]`, and the sections of the later slices'
//!   tables, empty because the export refuses a store in which one holds a
//!   row (`[credits]`, `[debits]`, `[ucredits]`, `[commitments]`,
//!   `[multisig]`); `[derived]` has `tipheight`, `missingcf` and
//!   `utxocount`, and compares `balance` and `unspentticket`, which neither
//!   side writes for a store without credits or commitments. `[agendas]`,
//!   `[treasury]` and `[vsp]` stay uncovered.
//! - **Frozen vectors.** Go's results and exports for the hand-written
//!   scripts (`tests/data/opvectors`) and for a fixed set of 24 generated
//!   scripts on every network (`tests/data/txstore_genvectors.txt`),
//!   checked in with counts and hashes (`tests/frozen.rs`).
//! - **Upstream tests** (`tests/txstore.rs`). `TestSetBirthState`
//!   (`txmined_test.go`), and `TestStakeInvalidationOfTip`
//!   (`stakevalidation_test.go`) up to what needs no credit: the
//!   `txcommon_test.go` header chain from the testnet3 genesis block
//!   (its zero `time.Time` serialized as wire does, 2288912640, and block
//!   1's hash pinned to Go's) driving `ExtendMainChain`, the two transactions inserted unmined and
//!   mined, and the third header invalidating the second block.
//!   `TestInsertsCreditsDebitsRollbacks` and `TestCoinbases` call
//!   `AddCredit` throughout and wait for it.
//! - **Properties** (`tests/txstore.rs`). Random sequences of regular,
//!   ticket, vote and revocation transactions inserted unmined and mined,
//!   removals, pruning, rollbacks and extensions: the row checker above and
//!   the chain invariants after every operation that succeeds, every
//!   failed operation having written nothing (the paragraph on failures
//!   above) and its abandoned transaction leaving the store as it was, and
//!   a reopened store holding the same rows and answering every query as
//!   the live one did; `TxRecord` round trips; the dependency order of
//!   `unmined_txs`, the same whatever the order of insertion; ADR-0019's
//!   filter rules, which no script reaches (a block without its filter
//!   below the birthday height, a refusal at or above it, the walk from
//!   the birthday height, the readers of a missing filter); the guard of
//!   all 13 later tables against every operation with a guard, firing and
//!   writing nothing where Go reads the table, and not firing elsewhere
//!   (removed with the guard by the commitment and multisig slice);
//!   the rows the range walks read (`range_walks_read_what_they_visit`: a
//!   handful when the callback stops at once on a chain of 2,048 blocks,
//!   none for a block without wallet transactions, the answers in both
//!   directions).
//!   The model of Go's rules the first draft planned is the
//!   differential's: the property tests hold invariants, not a second
//!   implementation.
//! - **Fuzz.** `fuzz/fuzz_targets/udb_tx_record.rs` feeds arbitrary bytes
//!   to `TxRecord::new` (the filters `ImportCFiltersV2` checks are
//!   `dcroxide-gcs`'s to fuzz).

//! # The credit slice
//!
//! Ported on 2026-11-23, after the first slice, with Go's texts and in
//! Go's order of writes except where stated (the plan of the change is
//! "The next slice" below, as narrowed):
//!
//! - `AddCredit` ([`TxStore::add_credit`]): the unmined, mined and
//!   invalidated paths. The script location is dcroxide's
//!   `MsgTx::pk_script_locs_iter` (no allocation), the script type
//!   (`pkScriptType`) and stake tag (`getStakeOpCode`) dcroxide's
//!   `determine_script_type`. The store's credit key is (hash, height,
//!   index), without Go's block hash, so a block that is not the main-chain
//!   block at its height, or that holds no record of the transaction, is
//!   refused (QK-0783, the refusal kept: choice 7 below), as is a script
//!   longer than the store's 16-bit script length.
//! - The credit and debit steps the table of "Credit-dependent steps" lists
//!   for `InsertMinedTx`, `moveMinedTx` (the expiry flag cleared, QK-0079),
//!   `Rollback` (the coinbase branch, the debit restoring with QK-0100's
//!   amount-0 rule, the credit step, and the deletion of a detached block's
//!   invalidated rows, ADR-0003 Decision item 5 and QK-0081),
//!   `RemoveUnconfirmed` (each output's unmined credit) and stake
//!   (in)validation (QK-0082 fixed under ADR-0012 Rule 7); the credit and
//!   debit lists of the details queries; the credit of a ticket's output 0
//!   in `TicketIterator.Next` and `TicketDetails`.
//! - The balance mutators of ADR-0003 Decision item 4 (`credit.rs`): every
//!   write of an unspent output, an unmined credit, an unmined-input entry
//!   and the unpublished mark goes through one function, which moves the
//!   running sums of `balances` (account, class) and, for the mined classes,
//!   `balance_heights` (account, class, height), counting an output only
//!   while no unmined transaction spends it, as `balanceFullScan` does. An
//!   unmined credit's class depends on its transaction's unpublished mark,
//!   so a change of the mark reclasses that transaction's unmined credits.
//!   [`TxStore::account_balances`] reads the sums and applies the depth and
//!   maturity rules at query time over the recent heights only.
//! - `acct_history`: (account, height, position) for every mined record
//!   with a live credit to the account, or a live debit spending a credit
//!   of the account (live or invalidated) that names that debit as its
//!   spender; brought up to date, for each record an operation changes the
//!   credits or debits of, by comparing its account set before and after.
//! - `UnspentTickets` ([`TxStore::unspent_tickets`]), and the export's
//!   `[credits]`, `[debits]`, `[ucredits]` and `[derived]` `balance` and
//!   `unspentticket` records.
//! - The queries over unspent outputs that need no input selection
//!   (`utxo.rs`): `ExistsUTXO`, `UnspentOutputs`, `UnspentOutput` and
//!   `ForEachUnspentOutpoint` ([`TxStore::exists_utxo`],
//!   [`TxStore::unspent_outputs`], [`TxStore::unspent_output`],
//!   [`TxStore::for_each_unspent_outpoint`], the outputs as [`Credit`]s in
//!   Go's order: the unspent outputs by outpoint, then the unmined credits
//!   by outpoint), with `outputCreditInfo`'s precedence kept (an outpoint
//!   with an unmined credit is given from it even where Go found it
//!   unspent, which a transaction mined and unmined at once, QK-0781, can
//!   reach); and spec 04 D-1's `IsUnspentOutpoint` and `PreviousPkScripts`
//!   ([`TxStore::is_unspent_outpoint`], [`TxStore::previous_pk_scripts`])
//!   ported as Go behaves (QK-0088 and QK-0089 reproduced, choice 6 below,
//!   decided 2026-11-24): `IsUnspentOutpoint` answers true for any outpoint
//!   neither unpublished nor spent by an unmined transaction, and
//!   `PreviousPkScripts` never answers without a block (`IO` where an input
//!   spends an unmined credit, else an `Invalid` error where Go
//!   dereferences the missing block and panics).
//!
//! Refusals found as Go goes are made before the first write here too:
//! `Rollback` checks every debit's credit first (`rollback_check`), and
//! stake validation every invalidated debit's (`stake_validate_check`,
//! where Go writes a zero-amount credit, QK-0101), so a failed operation
//! still writes nothing. Where a UTXO query meets missing stored state at
//! which Go panics (a mined output's record), it returns `IO` (ADR-0012
//! Rule 2).
//!
//! Speed: an input's debit makes three point reads of its own: the
//! outpoint's unspent row (which gives the credit's height, as Go's
//! unspent value gives it the credit key), the credit, which it rewrites
//! with its spender and from which the debit takes its amount (Go's
//! `spendCredit`), and the unmined-inputs entry the balance sums depend
//! on (a read Go does not make, the price of the running sums); the
//! unspent row is then deleted without being read again
//! (`del_unspent_row`). Each balance sum the change moves costs one more
//! point read and one write or delete (`bump`): two sums, `balances` and
//! `balance_heights`, for an output of a windowed mined class, one for
//! `VotingAuthority`, none for an output an unmined transaction spends.
//! A debited output of a windowed class that no unmined transaction
//! spends so makes five point reads and five writes or deletes (the
//! credit, the debit, the unspent row and the two sums), where Go's input
//! loop makes two reads and three writes per input (`existsUnspent`,
//! `spendCredit`, `putDebit`, `deleteRawUnspent`). The balance mutators
//! also make one read of the outpoint's unspent and unmined credit rows
//! per unmined-input change; `acct_history` costs two prefix
//! scans per changed record before and after the step, plus one credit read
//! per debit (the price of an index Go does not have, on no Go query path);
//! the details queries make two prefix reads per record, as Go's credit and
//! debit iterators seek once each. The UTXO queries walk `unspent` and
//! `unmined_credits` a page at a time (`store::walk`) and, per output, make
//! Go's unmined-input and unmined-credit lookups and read the record for
//! its received time and script, but no credit row (the unspent row holds
//! what Go reads from it) and one block row per distinct height (for the
//! hash Go's unspent value carries); consecutive outputs of one
//! transaction share one record read. Each record read copies the record's
//! bytes, where bbolt's `Get` gives Go a view of the page: a per-output
//! copy that a borrowed read in `purse-store` would remove (not proposed
//! before ADR-0003 M3's measurement). An unmined credit's script is cut
//! from its record at the stored location, where Go decodes the whole
//! transaction: the same bytes, no decoding (the location was computed
//! from that transaction, whose prefix the record holds verbatim).
//! `ForEachUnspentOutpoint` reads no record at all. No timed run is part
//! of this slice.
//!
//! Verified, as the first slice is, by:
//!
//! - the hand-written scripts `32-txstore-credits`, `33-txstore-credit-edges`
//!   and `34-txstore-utxo` (simnet; every record and the final exports equal
//!   to Go's; frozen vectors in `tests/data/opvectors`): credits unmined,
//!   mined and invalidated, published and unpublished, unmined and mined
//!   spends, moves, rollbacks, invalidation and revalidation, coinbase
//!   maturity, tickets, votes and revocations, zero-value credits, a
//!   transaction mined twice, scripts of every kind including a record with
//!   trailing bytes, and every UTXO query and `PreviousPkScripts` form;
//! - seeded generated scripts drawing the slice's operations too
//!   (`tests/support/txgen.rs`, `generate_txstore_credits`: `AddCredit`
//!   unmined and in the block an `InsertMinedTx` of the same `update` group
//!   has just recorded the transaction in, the UTXO queries, balances and
//!   `UnspentTickets` (with sync heights at Go's `ticketMatured` boundary
//!   and at the `int32` extremes, QK-0784), on all four networks, and the
//!   same in groups; `PURSE_UDB_TX_CREDIT_SCRIPTS`,
//!   `PURSE_UDB_TX_CREDIT_GROUP_SCRIPTS`). They avoid, by construction, the
//!   sequences on which dcr-purse departs from Go, under a rule or a
//!   decision (QK-0082, fixed under ADR-0012 Rule 7; QK-0081 with QK-0782,
//!   ADR-0003 Decision item 5; QK-0101 and QK-0783, whose refusals the
//!   project owner kept on 2026-11-24, choices 7 and 8 below); the
//!   generator's module documentation. A fixed set of 16 is frozen
//!   (`tests/data/txstore_credit_genvectors.txt`);
//! - the pinning tests `a_same_block_spend_chain_invalidated_leaves_no_dangling_unspent`
//!   (QK-0082), `credits_off_the_main_chain_or_without_a_record_are_refused`
//!   (QK-0783), `invalidated_credits_of_a_detached_block_do_not_come_back`
//!   (QK-0782) and
//!   `a_spender_below_its_funder_is_refused_where_go_fabricates_a_credit`
//!   (QK-0101), and two reproducing Go's defects:
//!   `a_revalidated_transaction_also_unmined_is_counted_twice` (QK-0734
//!   and QK-0736) and
//!   `a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again`
//!   (QK-0785);
//! - the upstream tests that need credits (`tests/txstore.rs`):
//!   `TestInsertsCreditsDebitsRollbacks`, `TestCoinbases` and the whole of
//!   `TestStakeInvalidationOfTip`;
//! - the property tests of `tests/txstore.rs`: random sequences with
//!   `AddCredit`, the row checker extended with the credit invariants (the
//!   balance sums and `acct_history` recomputed from the rows), the UTXO
//!   queries checked against the rows after every operation that succeeds
//!   (each output's script against a decoding of its record), an unspent
//!   output's credit naming no spender but in QK-0785's state, reopens
//!   answering balances, ticket and UTXO queries as before; three targeted
//!   tests, `an_overwritten_invalidated_credit_drops_its_old_spenders_account`
//!   (`acct_history`'s spender rule, in a state no random sequence
//!   reaches),
//!   `unspent_tickets_follow_ticket_matured_and_refuse_a_dangling_spender`
//!   (the maturity boundary, QK-0784's wrap, and QK-0048's `IO`) and
//!   `a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again`
//!   (QK-0785's state, its refused `Rollback` writing nothing); and unit
//!   tests in `credit.rs` and `balance.rs`.
//!
//! # The commitment and multisig slice
//!
//! Ported on 2026-11-24, with Go's texts and in Go's order of checks and
//! writes, after the credit slice: neither part records a script location,
//! and both are written by `Rollback` and `RemoveUnconfirmed`, whose guard
//! could go only once both were in. The tables are ADR-0003's
//! (`commitments`, `commitments_unspent`, `multisig_outs`,
//! `multisig_unspent`); the schema is unchanged.
//!
//! - **Ticket commitments** (`commitment.rs`): `AddTicketCommitment`
//!   ([`TxStore::add_ticket_commitment`]), `RedeemTicketCommitments`
//!   ([`TxStore::redeem_ticket_commitments`]), and the steps
//!   `RemoveUnconfirmed` makes (an unmined ticket's commitments at its odd
//!   outputs deleted with their unspent entries; a vote's or revocation's
//!   redeemed commitments marked not spent by an unmined transaction) and
//!   `Rollback` makes (a rolled-back vote's or revocation's marked spent by
//!   an unmined transaction), through Go's `originalTicketInfo` bound.
//!   `RemoveUnconfirmed` of a vote or revocation re-creates an unspent
//!   entry a mined spender's redemption had deleted, whichever transaction
//!   it is given (QK-0060, reproduced). Every write of an unspent entry and
//!   every deletion goes through one mutator, which moves the (account,
//!   `Locked`) running sum of `balances` in the same transaction, counting
//!   a commitment while its unspent entry exists without the unmined-spent
//!   flag, as `balanceFullScan`'s last pass does (ADR-0003 Decision item
//!   4), so `AccountBalance(s)` gives `LockedByTickets` from one row per
//!   account.
//! - **Multisig outputs** (`multisig.rs`): `AddMultisigOut`
//!   ([`TxStore::add_multisig_out`], the redeem script looked up as the
//!   address manager's `redeemScriptForHash160` does, through the
//!   `addr_by_hash` row and with its texts), `SpendMultisigOut`,
//!   `GetMultisigOutput` and `UnspentMultisigCreditsForAddress`
//!   ([`TxStore::spend_multisig_out`], [`TxStore::get_multisig_output`],
//!   [`TxStore::unspent_multisig_credits_for_address`]), and the marks
//!   `Rollback` (unmined, for each credit output, a dropped coinbase's
//!   included; unspent, for each restored debit) and `RemoveUnconfirmed`
//!   (unspent, for every input, whichever transaction spent the output)
//!   write. Go's defects are reproduced (ADR-0012 Rule 1): a spend or an
//!   unspend clears the stake-tree flag (QK-0054; pending owner, choice 12
//!   below); `AddMultisigOut` without a block over a mined record writes
//!   it again unmined and unspent (QK-0097); a multisig output is marked
//!   unspent while a transaction still spends it (QK-0786); and a record
//!   outlives the output it describes (QK-0787). The record is
//!   `purse-store`'s canonical 103-byte row (ADR-0003, addendum "The
//!   remaining records, as built"): Go's 135-byte value also holds the
//!   transaction hash, always the key's, and an unspent record's stale
//!   spender bytes (QK-0055), which no reader shows; `GetMultisigOutput`
//!   gives the spender only while spent ([`MultisigOut::spender`]), as
//!   `wallet/multisig.go` reads it, and the operation scripts' result shape
//!   says so (ADR-0002 section 4). A block of the zero hash at a height
//!   other than 0, which Go writes and the row cannot hold, is refused,
//!   writing nothing (ADR-0012 Rule 4: no caller passes one; PARITY.md,
//!   `purse-udb`).
//! - The guard of the four tables and the export's refusal of their rows
//!   are removed; the export writes `[commitments]` and `[multisig]`.
//!
//! Speed: the candidate commitments of a vote or revocation are read with
//! one range of the ticket's commitments, where Go makes one lookup per
//! candidate index (up to the bound, which a vote with trailing outputs
//! overestimates); `Rollback` and `RemoveUnconfirmed` test once per call
//! whether any multisig record exists, where Go looks a record up for
//! every credit output, restored debit and removed input, so a wallet
//! without multisig outputs makes no such lookup;
//! `UnspentMultisigCreditsForAddress` walks only the script's entries of
//! `multisig_unspent` (keyed by script hash first, ADR-0003) and looks the
//! redeem script up once, where Go walks every unspent multisig output and
//! looks it up per output. Each `Locked` change costs one balance read and
//! one write. No timed run is part of this slice.
//!
//! Verified by:
//!
//! - the hand-written scripts `35-txstore-commitments` and
//!   `36-txstore-multisig` (simnet; every record and the final exports equal
//!   to Go's; frozen vectors in `tests/data/opvectors`): every refusal of
//!   both parts in Go's order, commitments added unmined and mined, tracked
//!   twice, removed with an unmined ticket, its double spend mined, votes
//!   with two payouts, one payout (only output 1 a candidate) and three
//!   (the bound overestimating), revocations, unmined and mined
//!   redemptions, a redemption repeated, removed, rolled back and pruned,
//!   QK-0060's re-creation; multisig outputs of every script kind (P2SH of a
//!   multisig script, of `OP_TRUE`, of a script not imported, of an imported
//!   key's hash, stake-tagged), unmined, mined, marked again, overwritten
//!   (QK-0097), spent, spent again and double spent, a stake-tree output's
//!   flag cleared by a spend, by `RemoveUnconfirmed` of an unmined spender
//!   of an unspent record and by `Rollback`'s restored debit (QK-0054),
//!   unspent by `RemoveUnconfirmed` and `Rollback`
//!   while spent (QK-0786), and records outliving a removed transaction and
//!   a dropped coinbase (QK-0787), with reopens;
//! - seeded generated scripts drawing the slice's operations with the
//!   credit scripts' (`tests/support/txgen.rs`, `generate_txstore_cms`:
//!   imported multisig scripts paid by P2SH, one under a stake tag, the
//!   records `processTransactionRecord` would make for a transaction mined
//!   or unmined, the four writing operations and the two queries alone, on
//!   all four networks, and the same in groups; `PURSE_UDB_TX_CMS_SCRIPTS`,
//!   `PURSE_UDB_TX_CMS_GROUP_SCRIPTS`), every record and the final exports
//!   equal, a fixed set of 8 frozen
//!   (`tests/data/txstore_cms_genvectors.txt`); they never pass a block of
//!   the zero hash, the slice's one divergence;
//! - the oracle test `a_multisig_block_of_the_zero_hash_is_refused`
//!   (`tests/oracle.rs`), which shows Go's record beside dcr-purse's
//!   refusal;
//! - the property test
//!   `random_sequences_with_commitments_and_multisig_keep_the_row_invariants`
//!   (`tests/txstore.rs`): random sequences drawing the four writing
//!   operations and transactions paying multisig, stake-tagged and other
//!   P2SH scripts, with the row checker extended (every unspent commitment
//!   entry has its commitment, the `Locked` sums recomputed from the rows
//!   with the other balance sums, `multisig_unspent` holding exactly the
//!   unspent records under their script hashes), failures writing nothing,
//!   and reopens answering the multisig queries as before; the earlier
//!   random sequences draw as they did (until the selection slice's
//!   planned revalidations, below), with those checks too;
//! - unit tests of `originalTicketInfo` and of the P2SH script hash, and an
//!   export test of both sections (`tests/export.rs`).
//!
//! Not verified here: `redeemmultisigouts`, the RPC QK-0054's open
//! decision names, which is outside this crate. No upstream
//! test needs these parts but `TestUpgrades` (`upgrades_test.go`, which
//! reads `LockedByTickets` after dcrwallet's database upgrades), which has
//! no counterpart: dcr-purse's store has no upgrades and imports no
//! `wallet.db` (ADR-0013).
//!
//! # The selection slice
//!
//! Ported on 2026-11-24, after the commitment and multisig slice, with Go's
//! texts and in Go's order of reads and checks (`selection.rs`):
//!
//! - **`RandomUTXO`** ([`TxStore::random_utxo`]) with `randomUTXO`: 33
//!   bytes drawn (a 32-byte seek key and a direction bit), a walk of
//!   `unspent` from the key in that direction, and, finding nothing, the
//!   other way, collecting the outputs of the first transaction met that the
//!   skip test keeps (an output an unmined transaction spends, or not
//!   confirmed at the depth asked, is passed over without ending the
//!   walk), then one `IntN` among them; the answer is a [`Credit`] as
//!   [`TxStore::unspent_outputs`] gives it. `minConf` 0 is refused before
//!   any draw, with Go's text.
//! - **`MakeInputSource` and `SelectInputs`** ([`TxStore::make_input_source`],
//!   [`InputSource::select_inputs`]): the random phase (up to half the
//!   unspent outputs' number of `randomUTXO` tries over the source's life,
//!   only at a depth other than 0 and a target other than 0), then the
//!   enumeration phase (every unspent output the skip test keeps that the
//!   random phase did not return, and at depth 0 the unmined credits of
//!   published transactions without a stake tag, shuffled with
//!   `ShuffleSlice`), each candidate tested in Go's order (the credit and
//!   its record read, then the account, spent, zero, ticket, depth and
//!   maturity tests, the ignore function, the P2PKH or P2PK script class
//!   with Go's signature-script sizes). The answer is `purse-txauthor`'s
//!   `InputDetail`, cumulative over the calls, and the source implements
//!   `purse-txauthor`'s `InputSource`, so it feeds the authoring loop
//!   directly. Go's defects are reproduced (ADR-0012 Rule 1): the random
//!   phase is not account-aware and a try that finds nothing ends the call
//!   (QK-0096, the end unobservable, since such a try means no output is
//!   left to enumerate); the zero-value test applies to mined outputs only
//!   and an unmined credit an unmined transaction already spends is
//!   selected at depth 0 (QK-0105, both confirmed by the oracle); a source
//!   asked for everything and then for more than it holds selects outputs
//!   a second time (QK-0788, new; no dcrwallet caller asks a source twice in
//!   that order).
//! - **`Spender`** ([`TxStore::spender`]): the credit under the latest mined
//!   record of the output's transaction, its mined spender's record, or the
//!   unmined spender the unmined-inputs index names, searched for the input
//!   (hash and index, the tree not compared), with Go's kinds and texts.
//! - **QK-0734 fixed** under ADR-0012 Rule 7 (decided, project owner,
//!   2026-11-24; choice 10 below): when stake validation revalidates the
//!   record of a transaction that is also unmined, it merges the unmined
//!   copy into the record as `moveMinedTx` merges an unmined transaction
//!   into its block, each row through the balance mutators (`mined.rs`,
//!   `merge_unmined_copy`): the unmined-inputs entries of its inputs that
//!   name it are deleted (an entry naming another transaction stays); an
//!   input the record holds no debit for, whose output is an unspent wallet
//!   credit naming no spender (credited only after the record's first
//!   mining), is debited at the record's block; an unmined credit the
//!   record also holds is deleted, and one it does not hold (credited only
//!   by the later mining) is moved into a mined credit and an unspent
//!   output at the record's block; the unpublished mark and the unmined
//!   record are deleted. The multisig records the `Rollback` of the later
//!   block left (an output's record cleared whatever block it named,
//!   QK-0789; a spent input's record marked unspent, QK-0786) are marked
//!   mined in the revalidated block and spent by the input again (pending
//!   owner, choice 13 below). Go keeps both copies, and lists, counts and
//!   selects each output twice (PARITY.md, `purse-udb`, item (23)). QK-0736
//!   is unchanged: the transaction keeps its mined record in the disapproved
//!   block and `InsertMemPoolTx` still refuses it as already mined; the fix
//!   does not reach that step. Nor does it reach QK-0781's state (a
//!   transaction recorded mined in two blocks, the higher one rolled back,
//!   the output then credited without a block), which holds the same rows
//!   and stays reproduced: both sides list, count and select that output
//!   twice (`a_transaction_mined_twice_and_rolled_back_is_listed_counted_and_selected_twice`
//!   in `tests/oracle.rs`; QUIRKS.md).
//!
//! The random stream is a [`purse_snacl::RandomSource`], the address
//! manager's seam; `IntN` and `ShuffleSlice` are derived from it as dcrd's
//! `crypto/rand` derives them (`uniform.go`: `Uint64N`'s multiply-shift
//! with its rejection loop, and Go's shortcut past the division of the
//! rejection threshold, which saves the division and changes no draw; the
//! Fisher-Yates walk from the top), every buffer zeroed
//! before it is filled, as Go's callers pass fresh buffers to a `Read` that
//! XORs. Given the bytes Go's generator yields, the port draws what Go
//! draws and selects what Go selects, draw for draw (spec 23 §5.6.2). The
//! oracle's oprunner restarts its seeded stream under a key the random
//! operations may carry (`rand`, ADR-0002 section 2), and the Rust runner
//! keys dcrd's generator (dcroxide's `Prng::from_seed`) with the same key,
//! so the operation scripts compare the selections, the random outputs and
//! the bytes drawn exactly.
//!
//! The walk's order is bbolt's cursor's (spec 04 §5.1, Q-31): a seek lands
//! on the first key at or above the 32-byte seek key, and a `Prev` after it
//! gives the key below, or the last key when the seek found nothing, so the
//! forward walk is every key at or above the seek key and the backward walk
//! every key below it, in reverse; a 32-byte key sorts before the 36-byte
//! outpoints it prefixes, so both are ranges of the typed outpoint keys.
//!
//! Speed: the walks read `unspent` a page at a time (`store::walk`), the
//! pages doubling from one row, so a `randomUTXO` try that finds its
//! transaction in the first rows reads a handful, and a candidate's row
//! comes from the walk, where Go seeks the chosen key again. The
//! enumeration reads the unmined-inputs index once, in key order, beside
//! the walk of `unspent` (a merge), where Go looks every output up; each
//! candidate keeps its row, where Go reads it again before testing it.
//! Where the design trades speed for parity: a mined candidate's credit
//! and record are read before its account is compared, as Go reads them,
//! so a missing record fails the selection whichever account the output
//! belongs to; comparing the account first (the unspent row holds it)
//! would save a record read per output of another account. The credit is
//! read for its spent flag, which only QK-0785's state sets on an unspent
//! output. Each answer of [`InputSource::select_inputs`] is borrowed, and
//! the `purse-txauthor` source clones it, where Go's closure hands out the
//! slices it keeps appending to; a record read copies the record's bytes
//! (as in the credit slice). The measurement harness
//! (`tests/measure.rs`) times selection and `RandomUTXO` against
//! dcrwallet's on the same workload; no timed run is part of this slice.
//!
//! Verified by:
//!
//! - the hand-written script `37-txstore-selection` (simnet; every record,
//!   the bytes drawn and the final export equal to Go's; frozen vectors in
//!   `tests/data/opvectors`): `RandomUTXO` on an empty table, on one output
//!   with the seek key below and above it in both directions, at depths
//!   that skip it, and on six transactions with seek keys before the first
//!   key, past the last, inside a run of skipped outputs and at a
//!   transaction whose outputs are partly skipped, the keys chosen from the
//!   oracle's stream; `SelectInputs` with targets of 0, below, at and above
//!   the balance, at depths 0, 1 and 2, for two accounts, with an ignore
//!   list (a wrong tree included), coinbase, vote and ticket-change outputs
//!   at and after maturity, zero-value, P2PK, P2SH and stake-tagged
//!   outputs, unmined credits published and unpublished, spent by an
//!   unmined transaction (QK-0105) and of zero value, QK-0788's second
//!   selection, a `view` group continuing one stream across three random
//!   operations, a reopen, a `Rollback` and a `RemoveUnconfirmed`;
//!   `Spender` of mined and unmined spenders, unspent credits, outputs that
//!   are not credits and an outpoint of another tree;
//! - seeded generated scripts drawing the selections (each keyed or
//!   continuing the last one's stream) and `Spender` with the credit
//!   scripts' operations (`tests/support/txgen.rs`,
//!   `generate_txstore_select`; `PURSE_UDB_TX_SELECT_SCRIPTS`,
//!   `PURSE_UDB_TX_SELECT_GROUP_SCRIPTS`), avoiding, as every generator
//!   now does, the sequence of QK-0734's fix (a transaction is mined again
//!   only once no record of it can remain); a fixed set of 8 is frozen
//!   (`tests/data/txstore_select_genvectors.txt`), the random records
//!   compared exactly;
//! - the oracle tests `a_revalidated_transaction_drops_its_unmined_copy`
//!   (`tests/oracle.rs`), which shows Go's doubled output (listed twice,
//!   1,800 atoms counted and selected for one 900-atom output) beside
//!   dcr-purse's single one, and
//!   `a_revalidated_transaction_merges_what_only_its_unmined_copy_holds`,
//!   which shows Go's state beside dcr-purse's for the rows only the copy
//!   holds (an input debited only by the later mining, an output credited
//!   only by it, the transaction's multisig output and a multisig output it
//!   spends, the copy's unpublished mark, and an unmined-inputs entry naming
//!   another transaction), every record up to the approval equal, and, where
//!   the sequence then detaches the block again, every later record and the
//!   final exports equal;
//! - `a_revalidated_transaction_merges_its_unmined_copy_keeping_the_row_invariants`
//!   (`tests/txstore.rs`): the same sequence in eight variants (with an
//!   unmined spender of the copy's output, the copy unpublished, an earlier
//!   transaction of the same block re-mined with it, multisig records, and
//!   all together), the row invariants, the UTXO queries, the selections
//!   and a reopen checked after every step;
//! - the property tests of `tests/txstore.rs`: after every operation of the
//!   random sequences that succeeds, a selection of everything for two
//!   accounts at depths 0, 1 and 2 equals the outputs dcrwallet's rules
//!   select, recomputed from the rows, each once, with the amount their sum;
//!   a selection for a target takes only such outputs and all of them when
//!   it falls short; `RandomUTXO` answers an output no unmined transaction
//!   spends, confirmed, or none when there is none; `Spender` answers from
//!   each credit's spender or the unmined-inputs index; and a reopened store
//!   selects, picks and answers `Spender` as the live one did; and the
//!   sequences include planned revalidations (a block disapproved, a record
//!   of it mined again in the next block, both rolled back and the block
//!   approved by another), after each of which the record must be mined
//!   only, at least two per test (every merge then checked by the row
//!   invariants, among them that every unmined credit has its unmined
//!   record);
//!   `random_utxo_walks_as_bbolt_s_cursor_does` compares the walk with Go's
//!   `randomUTXO` written out over `purse-store`'s bbolt cursor, at seek
//!   keys of all zero and all one bits, each transaction's hash exactly and
//!   the values next to it, both directions, on an empty table and on one
//!   with skipped outputs;
//! - unit tests of the derived draws against dcroxide's port of
//!   `crypto/rand` on the same keyed stream (`selection.rs`).
//!
//! No upstream Go test at the pin calls input selection, `RandomUTXO` or
//! `Spender` (`txquery_test.go`, which is not built, does not either).
//!
//! # Choices left to the project owner
//!
//! Each was taken here as the ADR-0012 default or the narrowest reading,
//! and recorded. On 2026-11-22 the project owner decided 1, 2, 5 and 6 as
//! taken (accepting every recommendation then open) and
//! accepted 4 by signing the change that made it, after being told what
//! signing accepts; each is marked. Choice 3 is not decided. Choices 7 to
//! 11 are the credit slice's. On 2026-11-24 the project owner decided, each
//! as recommended, the points the credit slice put for choices 2 and 6 and
//! choices 7 to 11; each is marked. Choice 12 is the commitment and multisig
//! slice's, and is not decided. The selection slice adds choice 13 (the
//! multisig records in QK-0734's fix), not decided; otherwise what it
//! diverges in is decided (QK-0734, choice 10) or under a rule (PARITY.md,
//! `purse-udb`, items (23) to (25)), and the Go defects it meets are
//! reproduced by ADR-0012 Rule 1's default.
//!
//! 1. `Rollback` is ported without its credit step. In a store without
//!    credits that step never runs, in Go as here; the step itself, the one
//!    that computes a script offset, waits for `PkScriptLocs`. The other
//!    reading of the bound, leaving `Rollback` whole for the credit slice,
//!    would leave reorganisations untested until then. *Decided (project
//!    owner, 2026-11-22): ported without its credit step until the credit
//!    slice, which adds it.*
//! 2. `InsertMinedTx` of an unmined transaction into a stake-invalidated
//!    block, which Go does not implement (it panics), is refused. With
//!    credits, the alternative is to move the transaction and its credits
//!    into the invalidated tables, as a mined insert into such a block
//!    already records its debits there; decided with the credit slice.
//!    *Decided (project owner, 2026-11-22): it stays a refusal (`Invalid`),
//!    and whether to move it into the invalidated tables instead is
//!    decided with the credit slice.*
//!    *Evidence for that decision, gathered with the credit slice (decided
//!    2026-11-24, below):* the move would append the record to the block,
//!    record each input that spends an unspent wallet output as an invalidated
//!    debit (which `InsertMinedTx`'s input loop already does for a
//!    transaction not recorded unmined), write each unmined credit as an
//!    invalidated credit with no unspent output (as `AddCredit` does into
//!    such a block) and drop the unmined record; nothing in Go defines it,
//!    so it would be dcr-purse's own behaviour with no oracle to check it
//!    against. The wallet inserts a mined transaction into the block it is
//!    processing, which is invalidated only when the next block connects,
//!    so live sync never reaches it; a rescan over blocks already
//!    disapproved, of a transaction the wallet holds unmined, does, and Go
//!    then panics. The refusal returns `Invalid` and writes nothing, so a
//!    rescan's transaction is abandoned as Go's would be. Recommendation:
//!    keep the refusal now and decide the move with the rescan (Phase 5),
//!    where the case is reachable and a test can show what a wallet needs.
//!    *Decided (project owner, 2026-11-24): `InsertMinedTx` of an unmined
//!    regular transaction into a stake-invalidated block stays a refusal
//!    (`Invalid`, nothing written); whether to move it into the
//!    invalidated tables is decided in Phase 5 with the rescan, where the
//!    case can be reached and tested.*
//! 3. `GetMainChainCFilters` from a detached start block (QK-0751) answers
//!    as Go's does, with the main chain's filters from the detached
//!    header's height; the panic QK-0751 describes is in discovery
//!    (Phase 5).
//! 4. A `birth` row pending from both a height H and a time T, which
//!    dcrwallet resolves at whichever comes first and for which ADR-0019's
//!    FB11 (i) and FB12 (a) state no rule (they speak of a birthday set
//!    from a height or from a time): `extend_main_chain` accepts a block
//!    without its filter only when it lies below both, its height under H
//!    and its timestamp not after T, so no filter either reading keeps is
//!    dropped, and `missing_cfilters_height` walks from H. Neither path is
//!    reached by an operation script (they always pass a filter, and no
//!    filter is ever missing in one). *Accepted (project owner,
//!    2026-11-22), by signing the change that made it.*
//! 5. Go's log lines (spec 04 §8.4) are a surface under ADR-0012 Rule 1;
//!    none is written until the crate has a logger. *Decided (project
//!    owner, 2026-11-22): they wait for a logger.*
//! 6. Spec 04 D-1 (whether `IsUnspentOutpoint` and `PreviousPkScripts` are
//!    ported at all) is still open; both read credits and are left to the
//!    credit slice, where the spec's recommendation, not porting them,
//!    can be taken. *Decided (project owner, 2026-11-22): D-1 is decided
//!    with the credit slice.*
//!    *Taken with the credit slice (decided 2026-11-24, below):* both are
//!    ported as Go behaves (ADR-0012 Rule 1), reproducing QK-0088 (`IsUnspentOutpoint`
//!    tests that the unspent table exists instead of looking the outpoint
//!    up, so it answers true for any outpoint, recorded or not, that is
//!    neither unpublished nor spent by an unmined transaction) and QK-0089
//!    (`PreviousPkScripts` without a block fails: `IO`, a short record key,
//!    where an input spends an unmined credit, and otherwise a panic in
//!    Go, an `Invalid` error here). Evidence: dcrwallet at the pin has no
//!    caller of `IsUnspentOutpoint` at all, and of `PreviousPkScripts` only
//!    the build-ignored `txquery_test.go`, and dcr-purse exposes no Go API,
//!    so neither can reach a user until a caller is written; both are compared with
//!    Go operation by operation (`34-txstore-utxo`, the generated credit
//!    scripts) and cost a few dozen lines. The alternatives are not porting
//!    them (spec 04's recommendation; a later caller would then port them
//!    with a decision on their defects) or porting them with the defects
//!    fixed, a divergence under Rule 7 that no caller needs.
//!    Recommendation: keep them as ported, unexposed by any RPC, and decide
//!    on fixing them only if a caller is ever written. *Decided (project
//!    owner, 2026-11-24): both stay ported as Go behaves, QK-0088 and
//!    QK-0089 reproduced; nothing in dcrwallet calls either at the pin, and
//!    the question is revisited when a caller exists.*
//! 7. `AddCredit` under a block that is not the main-chain block at its
//!    height, or that holds no record of the transaction (QK-0783), where
//!    Go writes the credit under the block hash it is given: refused with
//!    `Invalid`, writing nothing. Brought to the project owner under
//!    ADR-0012 Rule 11 (decided 2026-11-24, below). The
//!    store's credit key holds the height and not the block hash, so it
//!    cannot hold such a credit apart from the main-chain block's at that
//!    height, and Go cannot be reproduced exactly. Evidence: the wallet
//!    calls `AddCredit` only after recording the transaction in the block
//!    it is processing (`processTransactionRecord`); Go's credit then
//!    makes later queries panic or fail (spec 04 §7). The alternative is
//!    keeping the block hash in the credit key (ADR-0003's schema).
//!    Recommendation: keep the refusal. *Decided (project owner,
//!    2026-11-24): the refusal of a block that is not the main-chain block
//!    at its height, or that holds no record of the transaction, is kept.*
//! 8. A credit that is not recorded where Go's `spendCredit` writes a
//!    zero-amount spent one (QK-0101): stake validation of a block whose
//!    invalidated debit names such a credit is refused with `IO` before
//!    anything is written, where Go connects the block and writes the
//!    credit; `Rollback` gives Go's own `IO` text, but before any write.
//!    Brought to the project owner under ADR-0012 Rule 11 (Go does not
//!    panic here, so Rule 2 does not cover it; decided 2026-11-24, below).
//!    Go's credit sits under the funding transaction's
//!    detached block, which the store's credit key cannot tell apart from
//!    the main-chain block at that height, so the code could not reproduce
//!    it while the ruling waited and kept the refusal. Evidence: the state is
//!    reached only through a spender recorded in a block below its funding
//!    transaction's, which a consensus-valid chain never holds (`Rollback`'s
//!    "missing credit" is also reached by a stake transaction's live debit
//!    of a regular credit of its own disapproved block, QK-0785, with Go's
//!    text exactly; stake validation's fabrication is not, since that debit
//!    is never invalidated); Go's
//!    continuation makes `AccountBalances` fail with `IO` once the
//!    spender's block is disapproved
//!    (`a_spender_below_its_funder_is_refused_where_go_fabricates_a_credit`).
//!    The alternatives: connect the block without the credit, taking it as
//!    absent (the call then succeeds as Go's does, and Go's zero-amount
//!    credit is missing from `[credits]`), or write a zero-amount spent
//!    credit under the height, which the funding transaction, mined again
//!    in a block at that height, would read as its own. Recommendation:
//!    keep the refusal. *Decided (project owner, 2026-11-24): the refusal
//!    (`IO`, before any write) where Go fabricates a zero-amount credit is
//!    kept.*
//! 9. ADR-0003 Decision item 5 (a detached block's invalidated rows
//!    deleted, QK-0081) stated that the difference is in nothing a client
//!    can read; QK-0782 shows that it is after the same block is connected
//!    again, its transaction mined in it again and the block disapproved
//!    and approved (ADR-0003, note of 2026-11-23 at item 5). Whether item 5
//!    stands was put to the project owner (decided 2026-11-24, below).
//!    Recommendation: it stands; the
//!    wallet adds the re-mined transaction's credits again, under the keys
//!    Go's stale rows hold, before a disapproval could revive them (read
//!    from the source, not run). *Decided (project owner, 2026-11-24):
//!    ADR-0003 Decision item 5 stands; `Rollback` deletes a detached
//!    block's invalidated rows, and the client-visible difference QK-0782
//!    shows is an accepted, recorded divergence (PARITY.md, `purse-udb`,
//!    item (11)).*
//! 10. QK-0734: a transaction of a disapproved block mined again in a later
//!     block, that block rolled back and the first block approved again, is
//!     mined and unmined at once, and its outputs are both unspent outputs and
//!     unmined credits: `UnspentOutputs` lists each twice, as unmined, and
//!     `AccountBalances` counts each twice. Reproduced, as the code stands
//!     (ADR-0012 Rule 11: the code reproduces Go until the ruling); whether
//!     it stays reproduced (Rule 1) or is fixed under Rule 7's first half, as
//!     ADR-0003 Open point 2 left it to be decided after an oracle run, was
//!     put to the project owner (decided 2026-11-24, below). Evidence: the
//!     oracle confirms it on simnet with every record
//!     and the export equal
//!     (`a_revalidated_transaction_also_unmined_is_counted_twice`: one 900-atom
//!     output, a balance of 1,800 at depth 0 and of 900 spendable plus 900
//!     unconfirmed at depth 1, the transaction unmined only, and
//!     `InsertMemPoolTx` refused as already mined, QK-0736, reproduced). A
//!     consensus-valid chain reaches it: a disapproved block's transaction
//!     mined again, then a reorganisation that approves the first block; the
//!     state outlasts a restart (how Go's input selection treats the doubled
//!     output is not run). A fix would have stake validation drop the unmined
//!     copy of a transaction whose record it revalidates (its unmined credits
//!     and unmined-input entries), as `moveMinedTx` does when an unmined
//!     transaction is mined; nothing in Go defines it, so only dcr-purse's own
//!     test would check it. Recommendation: fix it under Rule 7 in the slice
//!     after this one, with a PARITY.md line and Go's state shown beside
//!     dcr-purse's by the same test; it is ADR-0017's upstream-report candidate
//!     either way. QK-0735 is QK-0082 under another number, fixed with it;
//!     QK-0736 is reproduced (QUIRKS.md). *Decided (project owner,
//!     2026-11-24): QK-0734 is fixed under ADR-0012 Rule 7 in the next
//!     transaction-store slice ("The next slice" below), with a PARITY.md
//!     line and Go's state shown beside dcr-purse's; not in the change that
//!     records the decision, so until that slice the code still reproduces
//!     Go. It remains an upstream-report candidate under ADR-0017, whose
//!     reports the project owner delivers.* *Implemented (the selection
//!     slice, 2026-11-24): stake validation merges the unmined copy into
//!     the revalidated record as `moveMinedTx` merges an unmined
//!     transaction into its block, debiting an input only the later mining
//!     debited and moving a credit only the later mining made ("The
//!     selection slice" above); `a_revalidated_transaction_drops_its_unmined_copy`
//!     and `a_revalidated_transaction_merges_what_only_its_unmined_copy_holds`
//!     show Go's state beside dcr-purse's, and QK-0736 is unchanged. The
//!     multisig records are choice 13.*
//! 11. QK-0785: a stake transaction spending a regular credit of its own block,
//!     a state no consensus-valid chain holds. When the block is disapproved
//!     the credit moves to the invalidated table, spender kept, and the stake
//!     debit stays live (as does the debit of a spender recorded in a block
//!     below the credit's, QK-0101's state, which the approval below treats
//!     alike): `Rollback` over the block fails with Go's `IO` "missing credit"
//!     until the block is approved again (the refusal writes nothing, where
//!     Go's caller abandons its writes); when the block is approved again the
//!     credit comes back with an unspent entry although it is still spent, so
//!     `UnspentOutputs` lists it and `AccountBalances` counts it as spendable
//!     while `TxDetails` reports it spent. Reproduced (Rule 11's interim, Go
//!     reproduced); whether it stays reproduced (Rule 1) or Rule 7's first
//!     half, which names a spend wrongly recorded in persisted state, applies
//!     to a state no wallet reaches was put to the project owner (decided
//!     2026-11-24, below). Evidence:
//!     `a_revalidated_credit_a_ticket_of_its_block_spends_is_unspent_again` in
//!     `tests/oracle.rs` (every record and the export equal) and the test of
//!     that name in `tests/txstore.rs`; the generated credit scripts never mine
//!     a spender into a block a transaction it spends may be in, nor below it,
//!     and the row checker accepts an unspent output with a spent credit only
//!     in this state (the property tests reach the below-funder form,
//!     `PURSE_TEST_SEED=214`). The fix would be one condition in stake
//!     validation (no unspent entry for a credit naming a spender).
//!     Recommendation: keep it reproduced: no wallet reaches it, the
//!     reproduction costs nothing, and a fix would be dcr-purse's own
//!     behaviour, checked only by its own test. *Decided (project owner,
//!     2026-11-24): QK-0785 stays reproduced (ADR-0012 Rule 1); no
//!     consensus-valid chain reaches it.*
//!
//! 12. QK-0054: `SpendMultisigOut` and the unspend of `Rollback` and
//!     `RemoveUnconfirmed` clear a multisig record's stake-tree flag, so an
//!     output in the stake tree is reported in the regular tree by
//!     `GetMultisigOutput` and `UnspentMultisigCreditsForAddress` once it has
//!     been spent, and `redeemmultisigouts` would build its redemption on
//!     the wrong tree. Reproduced, as the code stands (ADR-0012 Rule 11's
//!     interim: the code reproduces Go until the ruling; the decision of
//!     2026-11-21 left the disposition to Phase 3 with an oracle run of
//!     `redeemmultisigouts` under Rule 1). Evidence: `36-txstore-multisig`
//!     shows the flag of a stake-tagged P2SH output cleared by a spend and
//!     staying clear after `RemoveUnconfirmed` unspends it, every record and
//!     the export equal to Go's; the run of `redeemmultisigouts` itself
//!     needs the wallet's RPC layer, which no crate has yet. The store's row
//!     keeps the flag apart from the spent flag, so either outcome can be
//!     written. Recommendation: keep the store reproducing Go now, and
//!     decide the disposition with `redeemmultisigouts`'s oracle run when
//!     the RPC is ported (the decision of 2026-11-21 stands); if it is
//!     fixed then, as a recorded divergence, the change is one assignment
//!     each in `spend_multisig_out` and `mark_unspent`. *Pending owner.*
//! 13. The multisig records in QK-0734's fix (QK-0789). `Rollback` of the
//!     later block marks unmined, by outpoint, the record of each output of
//!     the transaction it returns to the unmined set, even when the record
//!     names the earlier block, which it does not detach (QK-0789, a Go
//!     defect, reproduced), and marks unspent the record of each output its
//!     restored debits spent (QK-0786). With the transaction unmined, as Go
//!     keeps it, the cleared record matches Go's view; once the fix makes
//!     the transaction mined only, neither matches: an output of a mined
//!     transaction recorded unmined, and an output the mined transaction
//!     spends listed by `UnspentMultisigCreditsForAddress`, from which
//!     `redeemmultisigouts` would build a double spend. Nothing in Go
//!     decides it (Go never reaches the state), and no ADR-0012 rule names
//!     the multisig records. Taken here as recommended: the merge marks each
//!     output's unmined record mined in the revalidated block, as
//!     `AddMultisigOut` with that block would, and each input's unspent
//!     record spent by the input, as `SpendMultisigOut` would (the stake-tree
//!     flag cleared, QK-0054); a record another input spends is left.
//!     Evidence: `a_revalidated_transaction_merges_what_only_its_unmined_copy_holds`
//!     (sequences 3 and 4: Go's record unmined, at height 0, while dcr-purse's
//!     names block 3; Go's spent record listed as unspent, dcr-purse's spent
//!     by the transaction; every record before the approval and after the
//!     block is detached again equal, and the final exports equal). The
//!     alternative is to leave the records as the `Rollback` left them,
//!     which a later `AddMultisigOut` of the block would correct but which no
//!     wallet path makes, the block never being processed again.
//!     Recommendation: keep the re-marking, as the rest of the merge does
//!     with the credit rows. *Pending owner.*
//!
//! The owner's answer of 2026-11-22 also decided two more points this
//! slice records: a refused `InsertMissingCFilters` or `ImportCFiltersV2`
//! writes nothing ("A failed operation writes nothing" above), and
//! QK-0781 is reproduced (QUIRKS.md). With the same signing as choice 4
//! the owner accepted this slice's shipping before `PkScriptLocs` is at
//! the dcroxide pin; ADR-0006's deadline for it is restated as before the
//! slice that ports `AddCredit` and `Rollback`'s credit step (ADR-0006,
//! addendum "2026-11-22 (later)").
//!
//! # The next slice
//!
//! *As left by the credit slice (2026-11-23):*
//!
//! - **Ticket commitments** (`AddTicketCommitment`,
//!   `RedeemTicketCommitments`, `replaceTicketCommitmentUnminedSpent`,
//!   `removeUnspentTicketCommitments`, the commitment steps of
//!   `RemoveUnconfirmed` and `Rollback`, and the `Locked` balance class,
//!   whose sums `AccountBalance(s)` already reads but no operation writes):
//!   they record no script location and so never waited for one, but they
//!   add two tables, the `Locked` mutator, the redeem rules of votes and
//!   revocations and the export's `[commitments]`; left out to bound the
//!   credit slice's size.
//! - **Multisig outputs** (`AddMultisigOut`, `SpendMultisigOut`,
//!   `GetMultisigOutput`, `UnspentMultisigCreditsForAddress`, and the
//!   multisig steps of `Rollback` and `RemoveUnconfirmed`): they need the
//!   address manager's script lookup and the 135-byte record, and the
//!   export's `[multisig]`.
//! - **Input selection** (`MakeInputSource`, `InputSource.SelectInputs`,
//!   `RandomUTXO`): it needs spec 23 §5.6's random-stream discipline and
//!   the cursor behaviour of bbolt's seek and step back on `unspent`
//!   (Q-31), and is verified as sets of selectable outputs rather than
//!   record by record: a design of its own, on top of the UTXO queries of
//!   the credit slice.
//! - **`Spender`**: small (it reads the debits of mined spenders and the
//!   unmined-inputs index), but its natural tests are selection's
//!   reservation flows, so it moves with selection.
//!
//! *Added 2026-11-24 (choice 10, decided by the project owner):*
//!
//! - **QK-0734's fix** under ADR-0012 Rule 7: stake validation drops the
//!   unmined copy of a transaction whose record it revalidates (its unmined
//!   credits and unmined-input entries), as `moveMinedTx` does when an
//!   unmined transaction is mined, with a PARITY.md line and Go's state
//!   shown beside dcr-purse's by
//!   `a_revalidated_transaction_also_unmined_is_counted_twice`. Until the
//!   next slice the code reproduces Go.
//!
//! *Updated 2026-11-24 (the commitment and multisig slice and the
//! selection slice, made as one change):* the ticket commitments and the
//! multisig outputs are ported ("The commitment and multisig slice"
//! above), their guard removed and their export sections written; input
//! selection (`MakeInputSource`, `SelectInputs`, `RandomUTXO`), `Spender`
//! and QK-0734's fix are ported ("The selection slice" above). Nothing of the
//! store's listed remainder is left. Not part of it, and not in this crate:
//! the wallet-level flows that call these parts (`processTransactionRecord`,
//! transaction authoring with the wallet's locked outpoints, QK-0205) and
//! the RPCs `redeemmultisigouts` and `getmultisigoutinfo`, which choice 12
//! waits for.
//!
//! The store's dedicated-machine run of record (ADR-0003 Open point M2) is
//! to be made after the next slice, so that input selection is measured
//! too, and not before (decided, project owner, 2026-11-24; ADR-0014,
//! addendum of that date). It has not been made; with the slices above
//! ported, nothing it waited for is missing, and `tests/measure.rs` holds
//! the selection and `RandomUTXO` figures it is to time.
//!
//! *As first written:* once `pk_script_locs` is at the dcroxide pin (it is
//! since 2026-11-23, dcroxide `1587ca1`): `AddCredit`, the credit
//! steps of the table above with the balance mutators of ADR-0003 Decision
//! item 4 (and `acct_history`), the ticket commitments, the multisig
//! outputs, the UTXO queries, balances, input selection, `Spender`,
//! `UnspentTickets`, `ExistsUTXO`, the export's credit, debit, commitment,
//! multisig and balance records, and the three upstream tests that need
//! credits. Without the offsets, the ticket commitments and the multisig
//! outputs could form an intermediate slice, since neither records a
//! script location; the commitments would bring the balance tables'
//! `LockedByTickets` class with them.

mod balance;
mod chain;
mod commitment;
mod credit;
mod mined;
mod multisig;
mod query;
mod record;
mod selection;
mod store;
mod unmined;
mod utxo;

pub use balance::Balances;
pub use chain::{
    BlockCFilter, birth_state, extract_block_header_height, extract_block_header_parent_hash,
    extract_block_header_time, set_birth_state,
};
pub use credit::{OP_NONSTAKE, mined_class, tag_opcode, unmined_class};
pub use mined::RolledBack;
pub use multisig::{MultisigCredit, MultisigOut};
pub use record::{
    Block, BlockMeta, Credit, CreditRecord, DebitRecord, Ticket, TicketDetails, TxDetails,
    TxRecord, ZERO_TIME,
};
pub use selection::InputSource;
pub use store::{TxRead, TxStore};
pub use unmined::PruneError;
