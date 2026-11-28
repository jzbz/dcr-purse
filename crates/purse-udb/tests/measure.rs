// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! `purse-store`, with `purse-udb`'s address manager and transaction
//! store, against dcrwallet's own database code on the same workload: the
//! measurement the brief's Phase 2 exit asks the schema ADR
//! to record (brief §6 Phase 2, "the schema ADR records the measured gains
//! against dcrwallet on the same workload"; ADR-0003, addendum "purse-store
//! against dcrwallet on the same workload"; `docs/bench-ledger.md`).
//!
//! **The two sides.** Go is `tools/oracle/cmd/storebench`: dcrwallet's
//! `wallet/udb` at the parity pin over bbolt, opened as dcrwallet opens it
//! (every commit synchronous), fed through the call sequence of the wallet
//! (`setup`, `ingest`) and timed through udb's own API (`hot`). Rust is
//! this crate's address manager ([`Manager`], the port of udb's) and
//! transaction store ([`TxStore`]) over `purse-store`, every commit
//! `Durability::Immediate`, fed with the same call sequence through their
//! own functions: `TxStore::create` beside `Manager::create`, and per
//! block the hash computed from the decoded header and an empty version 2
//! filter built (as `extendMainChain` does both, inside the timing on both
//! sides), `extend_main_chain`, `get_block_meta_for_hash`, per transaction
//! `TxRecord::from_msg_tx` and `insert_mined_tx` (`insert_mem_pool_tx`
//! without a block), per output paying a recorded address the address
//! lookup, `add_credit` (`add_ticket_commitment` for a ticket's
//! commitment output), `MarkUsed` and the gap-limit sync of the branch,
//! `redeem_ticket_commitments` for a vote or revocation, and per write
//! transaction holding a block `update_processed_txs_block_marker` and
//! `prune_unmined` with the ticket price 0 (storebench's `extendMainChain`
//! and `processTx`, and the end of `ChainSwitch`, which passes 0 when the
//! next price is unknown); a reorganisation is `rollback` followed, in the
//! same write transaction, by the new chain's blocks (`ChainSwitch`); an
//! unmined transaction is `process_tx` without a block (the wallet's own
//! published sends, tickets and votes, `Unpublished` marked as the
//! workload says); an abandon is `AbandonTransaction`'s `tx_details` and
//! `remove_unconfirmed`. Every timed query calls the transaction store's
//! own query functions (`account_balance`, `account_balances`,
//! `range_transactions`, `make_input_source`, `random_utxo`,
//! `unspent_tickets`, `iterate_tickets`), so the figures measure the code
//! that ships; so do the export's tip, unspent count, balances, unspent
//! outputs, ranges, invalid blocks and outpoint walk (`unspent_outputs`,
//! `for_each_unspent_outpoint`, `block_in_main_chain`, ...). The export's
//! account and address lines come from the address manager, and its
//! invalidated credit and debit lines read the store's two tables of them
//! directly, as storebench reads udb's `ic` and `id` buckets.
//!
//! **What changed against the earlier entries.** Until ADR-0003's Open
//! point M3 the test wrote each block's transaction-store rows by its own
//! composition and answered balances and ranges by its own functions (the
//! spike's `account_balance` and `range_transactions` moved onto
//! `purse-store`'s tables); only input selection and `RandomUTXO` called
//! the transaction store; and the workload held no stake transaction, no
//! unmined transaction and no reorganisation. The 21 figures of the earlier
//! entries keep their names, so the bench ledger's entries of 2026-11-20
//! and 2026-11-26 stay comparable by name, but every one of them now runs
//! over the extended workload below (a larger history, with tickets, votes,
//! revocations, unmined transactions and their outputs), and these also
//! change what they call: `balance/*` and `balance_all/*` (now
//! `TxStore::account_balance(s)`), `range/*` (now
//! `TxStore::range_transactions`, each batch copied out of the callback,
//! and each credit's unmined spender asked of the unmined-inputs index),
//! `ingest/history_sync_per_block` and `commit/tip_block` (now the
//! transaction store's write path above, `InsertMinedTx`'s and
//! `AddCredit`'s own reads, `PruneUnmined` and, at the tip, the moves of
//! unmined transactions into blocks and the removal of double spends
//! included). `select/*` and `random_utxo/minconf=1` run the same code as
//! before, over the rows the transaction store now writes; `lookup/*` and
//! `commit/next_address` are the address manager's code, unchanged. Six
//! figures are new: `commit/tip_block_stake`, `commit/reorg`,
//! `commit/unmined_insert`, `commit/unmined_remove`, `tickets/unspent` and
//! `tickets/iterate` (below).
//!
//! **The same data on both sides.** A generator builds a simnet wallet
//! history from a fixed seed, over Zipf-weighted BIP0044 accounts:
//! receives to fresh or reused external addresses; sends of one to three
//! of the wallet's spendable outputs (regular ones, ticket change, and vote
//! and revocation payouts once `CoinbaseMaturity` deep) with change to a
//! fresh internal address; from the stake-enabled height, ticket purchases
//! at the block's ticket price (20 to 40 coins, fixed per stake difficulty
//! window, the price in each header's `SBits`) from one regular output, the
//! commitment and the voting address internal addresses of the buyer's
//! account, the change above dust to a third; from the stake-validation
//! height, each mature live ticket called to vote in a block with chance
//! 1/64 (5 votes of a pool of 64 × 5 tickets), the vote spending the
//! ticket and paying the price and a subsidy to the commitment's address,
//! a share of the called (1% at full size) missing their vote and revoked
//! in the next block, and tickets past `TicketExpiry` revoked (automatic
//! revocations, version 2, no fee). In the history sync, from two blocks
//! past the stake-validation height, one block in 200 (one in 7 at the
//! small size) disapproves its parent, its header's vote bits and its
//! votes saying so, and mines the parent's regular transactions again
//! first, as a miner does with the transactions a disapproval returns to
//! its mempool (likely more often than mainnet disapproves, not measured
//! here, chosen so the stake invalidation and the second record of each
//! transaction weigh in the history sync); every other block approves its
//! parent. At the tip (the last 300 blocks, one write transaction each)
//! the wallet also acts between blocks: it publishes sends unmined (their
//! fates in rotation: eight in ten mined one to three blocks later, one in
//! ten removed by a double spend of its first input mined in the block
//! after the next, one in ten abandoned after the next block, before the
//! one after it; one in ten recorded unpublished and mined later),
//! publishes its tickets unmined for the next block, and records its votes
//! on the block just connected unmined before the next block mines them
//! (as `VoteOnOwnedTickets` records them); after the last block it
//! publishes two more sends, the first recorded unpublished (so one
//! unpublished record is unmined when the exports are compared and the
//! queries run), and its ticket and votes, if any, which all stay unmined.
//! Every 30 blocks at the tip a reorganisation replaces the last one, two
//! or three blocks (depths in rotation, six, three and one in ten):
//! `Rollback` of the detached blocks, then blocks with new headers holding
//! the same transactions at the same heights, but one, and one more block.
//! The one left out is a send the generator adds to the last block each
//! reorganisation detaches, whose change nothing spends: `Rollback`
//! returns it to the unmined transactions and it stays there to the end,
//! so the exports and the answers compare what `Rollback` does to a
//! detached transaction (its unmined record, its inputs' spends in the
//! unmined-inputs index, its unmined credit), and a `Rollback` that
//! skipped that work would fail the comparison rather than time faster.
//! The detached transactions mined again in the new blocks leave no trace
//! of the move. That is likely more often than mainnet reorganises (not
//! measured here), chosen so a round times ten. The generator never calls a wallet vote in a
//! block a reorganisation detaches above its fork, nor records one unmined
//! on the block a reorganisation follows: such a vote would be on a block
//! that leaves the main chain.
//!
//! Sequences avoided by construction, where dcr-purse's decided
//! divergences (`docs/PARITY.md`, `purse-udb` transaction store) or Go's
//! reproduced defects would make the two exports differ, as the generated
//! credit scripts avoid them (item (19)). A plain disapproval keeps both
//! exports and every answer equal, so the workload has them, and only
//! what the stake-invalidation items need beyond one is kept out: no
//! disapproved block holds a spend of an output of its own block (item
//! (10), QK-0082, fixed, with QK-0735's first half; and QK-0785, a stake
//! transaction's debit of such an output, item (14)), no block a
//! reorganisation detaches disapproves its parent or is disapproved, so no
//! `Rollback` meets invalidated rows or revalidates a block (items (11),
//! QK-0081, and (23), QK-0734, fixed), and no detached block is ever
//! connected again, the new chain's headers being new (item (11),
//! QK-0782). A disapproved block's regular transactions are recorded mined
//! in it and again in the next block, but neither block is rolled back,
//! so the reproduced QK-0781 is not reached; none of them is offered to
//! the mempool in between (QK-0736, reproduced, is not met). No
//! transaction spends an output of a later block, so item (14) (QK-0101)
//! is not reached; nor does any spend an output of its own block, a
//! modelling choice (the wallet's sends and tickets wait a block) that
//! also keeps item (10)'s spend chain out of every disapproved block.
//! `AddCredit` follows `InsertMinedTx` in the block being processed, as in
//! the wallet, so item (12) (QK-0783) is not reached; no script is longer
//! than 65,535 bytes (item (13)) and no output is multisig (items (20) to
//! (22), QK-0789). No unmined transaction carries an expiry, every unmined
//! ticket and vote is mined in the block after it, and no vote is on a
//! block that leaves the main chain, so `PruneUnmined` removes nothing on
//! either side (the test checks the counts equal).
//!
//! The addresses are derived from the wallet seed with
//! `purse-hdkeychain`, the accounts after the first from the coin-type key
//! parsed back from its string, as the address manager derives them from
//! the key it stores (a coin-type private key that `child` returns with a
//! leading zero byte stripped, about one seed in 256, derives other
//! accounts in memory: `generator_accounts_are_the_address_managers`), and
//! both sides record them through their address managers, each checking
//! that every one is recorded at the generator's account, branch and
//! index. The history is written in dcrwallet's
//! `PURSEWL1` format (the spike's, with one operation kind added: 5, the
//! hash of an unmined transaction to abandon) as two files: the history
//! sync (blocks in batches of 2,000 per write transaction) and the tip
//! (blocks, unmined inserts and abandons one write transaction each, a
//! reorganisation's `Rollback` and blocks one). Before anything is timed
//! the two databases are compared through dcrwallet's canonical export
//! (`storebench export`, the spike's format): tip, unspent count, account
//! counters, balances at minconf 0, 1 and 6, every unspent output with its
//! tree, coinbase flag and script (unmined ones included), the timed
//! ranges and two more holding the unmined transactions (from -1 down to
//! 50 blocks below the tip, and from there up to -1) with every credit and
//! debit, the stake-invalidated blocks, credits and debits, the outpoint
//! walk with trees and each account's addresses in
//! `ForEachAccountAddress` order must be equal line for line; and every
//! timed query's answer must equal Go's in every round.
//!
//! **What is timed** (the brief's hot paths): `AccountBalance` of the
//! largest and the smallest account at minconf 1 and 6, and
//! `AccountBalances` at minconf 1; `RangeTransactions` over the last 1,000
//! blocks backward, 1,000 blocks mid-chain forward, the 100 most recent
//! transactions and the whole history; 1,000 address lookups by script
//! through `Manager.Address` and through `AddrAccount`, 1,000 account
//! lookups by name and 1,000 `AccountProperties`, each batch in one read
//! transaction; input selection, `MakeInputSource` and `SelectInputs` of
//! every output of account 0 at minconf 1 (target 0), for one coin at
//! minconf 1 from account 0 and from the last account, and for 50 coins at
//! minconf 0, and 1,000 `RandomUTXO` calls at minconf 1 in one read
//! transaction (the random stream seeded from the operating system on
//! both sides, as the wallet seeds it, so only the selection of everything
//! is compared, as a set, and the others for covering their target);
//! `UnspentTickets` at the tip with immature tickets (`tickets/unspent`)
//! and a walk of `IterateTickets` over every ticket (`tickets/iterate`),
//! each in one read transaction; and commits: each block at the tip (its
//! own durable write transaction, `commit/tip_block`), those of them
//! holding a stake transaction (`commit/tip_block_stake`, a subset), each
//! reorganisation (`commit/reorg`: the `Rollback` and the new blocks, one
//! write transaction), each unmined insert (`commit/unmined_insert`), each
//! abandon (`commit/unmined_remove`), the history sync per block, and 20
//! commits recording one more external address. Queries repeat at least 5
//! times and until a second has passed (at most 200 times), as
//! storebench's `repeat` does; a round reports each query's median. A
//! query whose answer is kept from one repetition to the next (balances,
//! ranges, lookup lines, selection checks, tickets) has the previous
//! answer released before the timing starts on the Rust side (cleared,
//! keeping the vector's capacity, where storebench resets a slice with
//! `[:0]`), so no Rust repetition pays for freeing the last one's answer,
//! which Go leaves to its collector; what a repetition allocates and drops
//! within itself (a selection's input source, a lookup's decoded address)
//! Rust frees inside the timing and Go's collector reclaims later, partly
//! on other threads of the same pinned cores and partly as assists that
//! may fall inside a timed window, a difference the test does not remove.
//! `IterateTickets` returns a new vector each call (one allocation, sized
//! to the ticket count), where storebench appends to its reused slice. The
//! timed work is the same on both sides: a range decodes and keeps every
//! transaction it returns, and the text its check compares is built after
//! the timing on both sides (both copy the store's batches out of the
//! callback and format them afterwards: udb reuses its slice and
//! storebench copies the details, which share their transactions' memory
//! with udb's; `TxStore` lends a batch it drops after the call, so the
//! Rust copy is a deep one, every decoded transaction and list allocated
//! again); a ticket query keeps what it returns (storebench copies each
//! `Ticket` out of the iterator) and is formatted after the timing; a
//! lookup batch builds its check text inside the timing on both sides.
//!
//! **A session** (`hot_paths_against_dcrwallet`, ignored): generate the
//! workload, set up both wallets and ingest the history sync into each
//! (timed once per side), keep both files, then `PURSE_UDB_MEASURE_ROUNDS`
//! rounds (default 3), alternating which side goes first (Rust then Go,
//! Go then Rust, ...): each round copies its side's file, ingests the tip
//! and runs the queries, Go in a fresh oracle process, Rust on a freshly
//! opened store. Release build, one test thread, the whole test pinned to
//! the same cores (both sides inherit them), `PURSE_STORE_BENCH_DIR` an
//! absolute path (the test runs in the crate's directory):
//!
//! ```sh
//! PURSE_STORE_BENCH_DIR=/some/dir/outside/the/repository PURSE_REQUIRE_ORACLE=1 \
//!   taskset -c 12-15 cargo test --release -p purse-udb --test measure -- \
//!   --ignored --nocapture --test-threads 1
//! ```
//!
//! Each session writes `session-<unix time>/` under `PURSE_STORE_BENCH_DIR`
//! with `results.tsv` (one line per side, round and figure), `notes.txt`
//! (start and end time, load averages, CPU affinity, versions, sizes, the
//! workload's operation counts) and `table.md`. `PURSE_UDB_MEASURE_BLOCKS`
//! changes the chain length (default 40,000 blocks, about 107,000 wallet
//! transactions).
//!
//! `same_data_and_answers_on_both_sides` runs the same pipeline at a small
//! size (240 blocks, 16 at the tip, two reorganisations, 11
//! disapprovals), one repetition of everything, in every test run: it
//! holds the export comparison and the query answers equal, checks that
//! every kind of operation and every figure is there with as many samples
//! on both sides, that the export holds invalidated blocks, credits and
//! debits, that an unpublished record is unmined in it and that each send
//! a reorganisation left out is unmined in it and mined nowhere, and its
//! times mean nothing.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "measurement code: counters, sums of generated amounts, sizes and statistics"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dcroxide_gcs::{FilterV2, blockcf2};
use dcroxide_stake::TxType;
use dcroxide_txscript::stdaddr;
use dcroxide_txscript::stdscript::{self, ScriptType};
use dcroxide_wire::{BlockHeader, MsgTx, OutPoint, TxIn, TxOut};
use purse_dcrutil::Amount;
use purse_errors::{AnyError, Kind};
use purse_hdkeychain::ExtendedKey;
use purse_snacl::{Limits, OsRandom};
use purse_store::{
    CreditsInvalidated, DebitsInvalidated, Error as StoreError, Hash32, HeaderBytes, ReadTx, Store,
    WriteTx,
};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{Oracle, SplitMix64, hex, oracle_bin_or_skip, oracle_or_skip};
use purse_udb::Manager;
use purse_udb::addrmgr::{ChainParams, IMPORTED_ADDR_ACCOUNT as IMPORTED_ACCOUNT, ManagedAddress};
use purse_udb::txstore::{Balances, BlockMeta, TxDetails, TxRecord, TxStore};
use sha2::{Digest as _, Sha256};

const HARDENED: u32 = 0x8000_0000;
/// The gap limit both sides' ingest syncs past the last used child
/// (dcrwallet's default, `wallet.DefaultGapLimit`).
const GAP: u32 = 20;
const PUB_PASS: &[u8] = b"public";
const PRIV_PASS: &[u8] = b"private";

// ---------------------------------------------------------------------------
// The workload

/// What to generate.
#[derive(Clone, Debug)]
struct Preset {
    /// BIP0044 accounts (0 to `accounts - 1`).
    accounts: u32,
    /// Blocks after genesis.
    blocks: u32,
    /// The last `tip_blocks` blocks are one write transaction each.
    tip_blocks: u32,
    /// Blocks per write transaction before them.
    batch: u32,
    /// Receives and sends per block: uniform in `0..txs_bound`.
    txs_bound: u64,
    /// Share of them that are sends (when the account has spendable
    /// outputs).
    send_share: f64,
    /// Chance per block, from the stake-enabled height, that the wallet
    /// buys a ticket.
    ticket_rate: f64,
    /// Share of the wallet's tickets called to vote whose vote is missed;
    /// the ticket is revoked in the next block.
    miss_share: f64,
    /// Chance, between two blocks at the tip, that the wallet publishes a
    /// send unmined (and a second one, with this chance squared).
    unmined_rate: f64,
    /// One reorganisation every `reorg_every` blocks at the tip (0: none).
    reorg_every: u32,
    /// From two blocks past the stake-validation height to three blocks
    /// before the tip, each block at a height that is a multiple of
    /// `disapprove_every` disapproves its parent (0: none; never 1, so a
    /// disapproved block never disapproves its own parent).
    disapprove_every: u32,
}

impl Preset {
    fn small() -> Preset {
        Preset {
            accounts: 3,
            blocks: 240,
            tip_blocks: 16,
            batch: 50,
            txs_bound: 4,
            send_share: 0.3,
            ticket_rate: 0.3,
            miss_share: 0.25,
            unmined_rate: 0.5,
            reorg_every: 8,
            disapprove_every: 7,
        }
    }

    fn measure() -> Preset {
        let blocks = std::env::var("PURSE_UDB_MEASURE_BLOCKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40_000);
        Preset {
            accounts: 4,
            blocks,
            tip_blocks: 300,
            batch: 2_000,
            txs_bound: 6,
            send_share: 0.3,
            ticket_rate: 0.08,
            miss_share: 0.01,
            unmined_rate: 0.5,
            reorg_every: 30,
            disapprove_every: 200,
        }
    }
}

/// When a wallet output may be spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutKind {
    /// A regular output, or a ticket's change (`SStxChangeMaturity` is 1
    /// on simnet): in any later block.
    Regular,
    /// A vote's or a revocation's payout: `CoinbaseMaturity` blocks later.
    StakePayout,
}

#[derive(Clone, Debug)]
struct Utxo {
    hash: [u8; 32],
    index: u32,
    tree: i8,
    value: i64,
    height: u32,
    kind: OutKind,
}

/// A wallet output a transaction creates: account, output index, amount,
/// tree and kind.
type Out = (u32, u32, i64, i8, OutKind);

/// A generated transaction and the wallet outputs it creates.
type Built = (MsgTx, Vec<Out>);

/// A ticket of the wallet: its commitment pays `reward`, an internal
/// address of `account`.
#[derive(Clone, Debug)]
struct Ticket {
    hash: [u8; 32],
    account: u32,
    price: i64,
    height: u32,
    reward: [u8; 20],
}

/// What happens to a transaction the wallet published unmined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    /// Mined in block `at`.
    Mine,
    /// A transaction double-spending its first input is mined in block
    /// `at`, which removes it (`InsertMinedTx`'s double-spend removal).
    Conflict,
    /// Abandoned (`AbandonTransaction`) before block `at`.
    Abandon,
}

/// The fates of the wallet's unmined sends, in rotation, with the blocks
/// after their publication at which they happen: eight in ten mined (in
/// the next block, the one after or the one after that), one replaced by
/// a double spend, one abandoned.
const FATES: [(Fate, u32); 10] = [
    (Fate::Mine, 0),
    (Fate::Mine, 1),
    (Fate::Conflict, 1),
    (Fate::Mine, 2),
    (Fate::Abandon, 1),
    (Fate::Mine, 0),
    (Fate::Mine, 1),
    (Fate::Mine, 2),
    (Fate::Mine, 0),
    (Fate::Mine, 1),
];

/// The depths of the reorganisations, in rotation: six in ten replace one
/// block, three two blocks, one three blocks.
const REORG_DEPTHS: [u32; 10] = [1, 2, 1, 1, 3, 1, 2, 1, 1, 2];

/// A transaction the wallet holds unmined.
struct Pending {
    msg: MsgTx,
    at: u32,
    fate: Fate,
    account: u32,
    inputs: Vec<Utxo>,
    outputs: Vec<Out>,
    ticket: Option<Ticket>,
    /// Recorded `Unpublished`.
    unpublished: bool,
}

#[derive(Clone, Debug)]
struct Block {
    height: u32,
    hash: [u8; 32],
    /// The header, decoded, as the wallet holds it (the workload file
    /// carries it serialized).
    hd: BlockHeader,
    /// The transactions, serialized (as the workload file carries them)
    /// and decoded (as the wallet hands them to `NewTxRecordFromMsgTx`).
    txs: Vec<Vec<u8>>,
    msgs: Vec<MsgTx>,
}

#[derive(Clone, Debug)]
struct UnminedTx {
    tx: Vec<u8>,
    msg: MsgTx,
    unpublished: bool,
}

/// One operation of a workload file.
#[derive(Clone, Debug)]
enum Op {
    /// Kind 1 (history sync) or 3 (at the tip): a block, `commit` ending
    /// the write transaction.
    Block {
        block: Block,
        tip: bool,
        commit: bool,
    },
    /// Kind 2: a transaction inserted unmined with its credits, a write
    /// transaction of its own.
    Unmined(UnminedTx),
    /// Kind 4: `Rollback` from this height, in the write transaction of
    /// the blocks that follow.
    Reorg(u32),
    /// Kind 5, added to the spike's format: `AbandonTransaction` of an
    /// unmined transaction, a write transaction of its own.
    Abandon([u8; 32]),
}

impl Op {
    fn commit(&self) -> bool {
        match self {
            Op::Block { commit, .. } => *commit,
            Op::Reorg(_) => false,
            Op::Unmined(_) | Op::Abandon(_) => true,
        }
    }

    fn block(&self) -> Option<&Block> {
        match self {
            Op::Block { block, .. } => Some(block),
            _ => None,
        }
    }
}

/// What the generator made.
#[derive(Clone, Debug, Default)]
struct Stats {
    receives: u64,
    sends: u64,
    tickets: u64,
    votes: u64,
    revoked_missed: u64,
    revoked_expired: u64,
    unmined_sends: u64,
    unmined_tickets: u64,
    unmined_votes: u64,
    unpublished: u64,
    mined_from_unmined: u64,
    conflicts: u64,
    abandoned: u64,
    left_unmined: u64,
    /// Of those left unmined, the ones recorded `Unpublished`.
    unpublished_left_unmined: u64,
    /// Sends in a block a reorganisation detaches that its new chain
    /// leaves out, unmined from the `Rollback` to the end.
    detached_left_unmined: u64,
    /// Blocks disapproving their parent, and the regular transactions of
    /// the disapproved parents mined again in them.
    disapprovals: u64,
    invalidated_remined: u64,
    /// Reorganisations of depth 1, 2 and 3.
    reorgs: [u64; 3],
    detached_blocks: u64,
    remined: u64,
}

#[derive(Clone, Debug)]
struct AddrRange {
    account: u32,
    branch: u32,
    hashes: Vec<[u8; 20]>,
}

#[derive(Debug)]
struct Workload {
    seed: [u8; 32],
    accounts: u32,
    ranges: Vec<AddrRange>,
    /// The history sync: blocks only.
    sync: Vec<Op>,
    /// At the tip: blocks, unmined inserts, abandons and reorganisations.
    tip: Vec<Op>,
    stats: Stats,
    /// The hashes of the sends left unmined by a reorganisation.
    left_out: Vec<[u8; 32]>,
}

impl Workload {
    fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.sync.iter().chain(&self.tip).filter_map(Op::block)
    }

    fn tip_height(&self) -> u32 {
        self.blocks().last().map_or(0, |b| b.height)
    }

    fn tip_ops(&self, kind: fn(&Op) -> bool) -> usize {
        self.tip.iter().filter(|o| kind(o)).count()
    }

    fn transactions(&self) -> usize {
        self.blocks().map(|b| b.txs.len()).sum::<usize>()
            + self.tip_ops(|o| matches!(o, Op::Unmined(_)))
    }

    fn addresses(&self) -> usize {
        self.ranges.iter().map(|r| r.hashes.len()).sum()
    }
}

struct Account {
    branches: [ExtendedKey; 2],
    derived: [Vec<[u8; 20]>; 2],
    next: [u32; 2],
    used_ext: Vec<u32>,
}

struct Gen {
    rng: SplitMix64,
    accts: Vec<Account>,
    utxos: Vec<Vec<Utxo>>,
    /// Mined tickets not yet voted or revoked.
    live: Vec<Ticket>,
    /// Tickets to revoke, with the block that revokes them.
    revoke: Vec<(u32, Ticket)>,
    pending: Vec<Pending>,
    coinbase_maturity: u32,
    stats: Stats,
}

impl Gen {
    fn unit(&mut self) -> f64 {
        (self.rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.rng.below(n as u64)).expect("fits")
    }

    fn hash32(&mut self) -> [u8; 32] {
        let mut h = [0; 32];
        self.rng.fill(&mut h);
        h
    }

    fn hash20(&mut self) -> [u8; 20] {
        let mut h = [0; 20];
        self.rng.fill(&mut h);
        h
    }

    /// Account `i` with weight `1/(i+1)`.
    fn pick_account(&mut self) -> u32 {
        let n = u32::try_from(self.accts.len()).expect("accounts");
        let total: f64 = (1..=n).map(|i| 1.0 / f64::from(i)).sum();
        let mut x = self.unit() * total;
        for i in 0..n {
            x -= 1.0 / f64::from(i + 1);
            if x <= 0.0 {
                return i;
            }
        }
        n - 1
    }

    fn address(&mut self, account: u32, branch: usize, index: u32) -> [u8; 20] {
        let a = &mut self.accts[account as usize];
        while a.derived[branch].len() <= index as usize {
            let i = u32::try_from(a.derived[branch].len()).expect("index");
            let child = a.branches[branch].child(i).expect("child derivation");
            a.derived[branch].push(stdaddr::hash160(child.serialized_pub_key()));
        }
        a.derived[branch][index as usize]
    }

    fn fresh(&mut self, account: u32, branch: usize) -> [u8; 20] {
        let a = &mut self.accts[account as usize];
        let idx = a.next[branch];
        a.next[branch] = idx + 1;
        if branch == 0 {
            a.used_ext.push(idx);
        }
        self.address(account, branch, idx)
    }

    fn receive_address(&mut self, account: u32) -> [u8; 20] {
        let used = self.accts[account as usize].used_ext.len();
        if used > 0 && self.unit() < 0.4 {
            let j = self.below(used);
            let idx = self.accts[account as usize].used_ext[j];
            self.address(account, 0, idx)
        } else {
            self.fresh(account, 0)
        }
    }

    fn sig_script(&mut self) -> Vec<u8> {
        // A P2PKH signature script's shape: a 72-byte push, a 33-byte push.
        let mut s = Vec::with_capacity(107);
        s.push(0x48);
        s.extend_from_slice(&self.rng.bytes(72));
        s.push(0x21);
        s.extend_from_slice(&self.rng.bytes(33));
        s
    }

    fn input(&mut self, u: &Utxo) -> TxIn {
        let mut previous_out_point = OutPoint::default();
        previous_out_point.hash.0 = u.hash;
        previous_out_point.index = u.index;
        previous_out_point.tree = u.tree;
        TxIn {
            previous_out_point,
            sequence: u32::MAX,
            value_in: u.value,
            block_height: u.height,
            block_index: self.rng.below(20) as u32,
            signature_script: self.sig_script(),
        }
    }

    fn amount(&mut self) -> i64 {
        // Log-uniform between 0.001 and 1,000 coins.
        let e = 5.0 + self.unit() * 6.0;
        10f64.powf(e) as i64
    }

    /// Whether `u` may be spent in block `height`.
    fn spendable(&self, u: &Utxo, height: u32) -> bool {
        match u.kind {
            OutKind::Regular => u.height < height,
            OutKind::StakePayout => u.height + self.coinbase_maturity <= height,
        }
    }

    /// A receive: one foreign input, one output to the wallet and one
    /// foreign change output, in random order.
    fn receive(&mut self, height: u32) -> Built {
        let account = self.pick_account();
        let mut tx = MsgTx::default();
        let value = self.amount();
        let foreign_in = Utxo {
            hash: self.hash32(),
            index: self.rng.below(4) as u32,
            tree: 0,
            value: value + 100_000,
            height: height.saturating_sub(10),
            kind: OutKind::Regular,
        };
        let inp = self.input(&foreign_in);
        tx.tx_in.push(inp);
        let to = self.receive_address(account);
        let foreign = self.hash20();
        let wallet_out = TxOut {
            value,
            version: 0,
            pk_script: p2pkh(&to),
        };
        let change = TxOut {
            value: 97_500,
            version: 0,
            pk_script: p2pkh(&foreign),
        };
        let wallet_index = if self.unit() < 0.5 {
            tx.tx_out.push(wallet_out);
            tx.tx_out.push(change);
            0
        } else {
            tx.tx_out.push(change);
            tx.tx_out.push(wallet_out);
            1
        };
        self.stats.receives += 1;
        (
            tx,
            vec![(account, wallet_index, value, 0, OutKind::Regular)],
        )
    }

    /// A payment to a foreign address from `ins` (outputs of `account`)
    /// and, unless the change is dust, change to a fresh internal address
    /// of the same account.
    fn pay_from(&mut self, account: u32, ins: &[Utxo]) -> Built {
        let mut tx = MsgTx::default();
        let mut total = 0i64;
        for u in ins {
            let inp = self.input(u);
            tx.tx_in.push(inp);
            total += u.value;
        }
        let fee = 2_000 + 1_500 * ins.len() as i64;
        let pay = ((total - fee) as f64 * (0.2 + 0.75 * self.unit())) as i64;
        let change = total - fee - pay;
        let payee = self.hash20();
        let pay_out = TxOut {
            value: pay,
            version: 0,
            pk_script: p2pkh(&payee),
        };
        let mut outputs = Vec::new();
        if change >= 6_000 {
            let to = self.fresh(account, 1);
            let change_out = TxOut {
                value: change,
                version: 0,
                pk_script: p2pkh(&to),
            };
            let ci = if self.unit() < 0.5 {
                tx.tx_out.push(change_out);
                tx.tx_out.push(pay_out);
                0
            } else {
                tx.tx_out.push(pay_out);
                tx.tx_out.push(change_out);
                1
            };
            outputs.push((account, ci, change, 0, OutKind::Regular));
        } else {
            tx.tx_out.push(pay_out);
        }
        (tx, outputs)
    }

    /// A send from one account: one to three of its outputs spendable in
    /// this block (and not held by an unmined transaction), paid as
    /// [`Gen::pay_from`] pays them. The account and the outputs spent come
    /// with it.
    fn send(&mut self, height: u32) -> Option<(Built, u32, Vec<Utxo>)> {
        let account = self.pick_account();
        let n = match self.below(10) {
            0..=6 => 1,
            7..=8 => 2,
            _ => 3,
        };
        let mut ins = Vec::new();
        let mut tries = 0;
        while ins.len() < n && tries < n * 8 + 8 {
            tries += 1;
            let len = self.utxos[account as usize].len();
            if len == 0 {
                break;
            }
            let i = self.below(len);
            let u = &self.utxos[account as usize][i];
            if self.spendable(u, height) && u.value >= 20_000 {
                ins.push(self.utxos[account as usize].swap_remove(i));
            }
        }
        if ins.is_empty() {
            return None;
        }
        let built = self.pay_from(account, &ins);
        self.stats.sends += 1;
        Some((built, account, ins))
    }

    /// A ticket purchase at `price` in block `height`: one regular output
    /// of the account worth at least the price and the fee, the ticket's
    /// output 0 to a fresh internal address (the voting address), its
    /// commitment to another (the reward address) for the amount the input
    /// contributes, and its change, when above dust, to a third (else a
    /// zero-value change output to a foreign address, the dust going to
    /// the fee).
    fn ticket(&mut self, height: u32, price: i64) -> Option<(Built, Ticket, Vec<Utxo>)> {
        const FEE: i64 = 3_000;
        let account = self.pick_account();
        let len = self.utxos[account as usize].len();
        if len == 0 {
            return None;
        }
        let start = self.below(len);
        let i = (0..len).map(|k| (start + k) % len).find(|&i| {
            let u = &self.utxos[account as usize][i];
            u.kind == OutKind::Regular && u.height < height && u.value >= price + FEE
        })?;
        let u = self.utxos[account as usize].swap_remove(i);
        let mut tx = MsgTx::default();
        let inp = self.input(&u);
        tx.tx_in.push(inp);
        let vote_addr = self.fresh(account, 1);
        let reward = self.fresh(account, 1);
        let change = u.value - price - FEE;
        let (change_value, change_addr) = if change >= 10_000 {
            (change, self.fresh(account, 1))
        } else {
            (0, self.hash20())
        };
        tx.tx_out.push(TxOut {
            value: price,
            version: 0,
            pk_script: tagged(OP_SSTX, &vote_addr),
        });
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: commitment(&reward, u.value - change_value),
        });
        tx.tx_out.push(TxOut {
            value: change_value,
            version: 0,
            pk_script: tagged(OP_SSTXCHANGE, &change_addr),
        });
        assert_eq!(dcroxide_stake::determine_tx_type(&tx), TxType::SStx);
        let mut outputs = Vec::new();
        if change_value > 0 {
            outputs.push((account, 2, change_value, 1, OutKind::Regular));
        }
        let t = Ticket {
            hash: tx.tx_hash().0,
            account,
            price,
            height,
            reward,
        };
        self.stats.tickets += 1;
        Some(((tx, outputs), t, vec![u]))
    }

    /// The vote of ticket `t` in block `height`, on its parent `prev`:
    /// the stakebase, the ticket's output 0, the block reference, the vote
    /// bits approving the parent (or not, `approve`), and the payout of the
    /// price and the vote subsidy to the commitment's address.
    fn vote(&mut self, t: &Ticket, height: u32, prev: &[u8; 32], approve: bool) -> Built {
        const SUBSIDY: i64 = 3_000_000;
        let mut tx = MsgTx::default();
        let stakebase = OutPoint {
            index: u32::MAX,
            ..OutPoint::default()
        };
        tx.tx_in.push(TxIn {
            previous_out_point: stakebase,
            sequence: u32::MAX,
            value_in: SUBSIDY,
            block_height: 0,
            block_index: u32::MAX,
            signature_script: vec![0x00, 0x00],
        });
        let inp = self.input(&ticket_output(t));
        tx.tx_in.push(inp);
        let mut reference = Vec::with_capacity(36);
        reference.extend_from_slice(prev);
        reference.extend_from_slice(&(height - 1).to_le_bytes());
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: nulldata(&reference),
        });
        tx.tx_out.push(TxOut {
            value: 0,
            version: 0,
            pk_script: nulldata(&[u8::from(approve), 0x00]),
        });
        let payout = t.price + SUBSIDY;
        tx.tx_out.push(TxOut {
            value: payout,
            version: 0,
            pk_script: tagged(OP_SSGEN, &t.reward),
        });
        assert_eq!(dcroxide_stake::determine_tx_type(&tx), TxType::SSGen);
        self.stats.votes += 1;
        (tx, vec![(t.account, 2, payout, 1, OutKind::StakePayout)])
    }

    /// The automatic revocation (version 2, no signature, no fee) of
    /// ticket `t`.
    fn revocation(&mut self, t: &Ticket) -> Built {
        let mut tx = MsgTx {
            version: 2,
            ..MsgTx::default()
        };
        let mut inp = self.input(&ticket_output(t));
        inp.signature_script = Vec::new();
        tx.tx_in.push(inp);
        tx.tx_out.push(TxOut {
            value: t.price,
            version: 0,
            pk_script: tagged(OP_SSRTX, &t.reward),
        });
        assert_eq!(dcroxide_stake::determine_tx_type(&tx), TxType::SSRtx);
        (tx, vec![(t.account, 0, t.price, 1, OutKind::StakePayout)])
    }

    /// The wallet's live tickets called to vote in block `height`: each
    /// mature one with chance `call`. A share `miss` of those called miss
    /// their vote and are revoked in the next block; the rest are
    /// returned.
    fn call(&mut self, height: u32, call: f64, maturity: u32, miss: f64) -> Vec<Ticket> {
        let mut voters = Vec::new();
        let mut i = 0;
        while i < self.live.len() {
            if height > self.live[i].height + maturity + 1 && self.unit() < call {
                let t = self.live.swap_remove(i);
                if self.unit() < miss {
                    self.revoke.push((height + 1, t));
                    self.stats.revoked_missed += 1;
                } else {
                    voters.push(t);
                }
            } else {
                i += 1;
            }
        }
        voters
    }

    /// Moves the live tickets past their expiry in block `height` to the
    /// revocations of that block.
    fn expire(&mut self, height: u32, maturity: u32, expiry: u32) {
        let mut i = 0;
        while i < self.live.len() {
            if height > self.live[i].height + maturity + expiry + 1 {
                let t = self.live.swap_remove(i);
                self.revoke.push((height, t));
                self.stats.revoked_expired += 1;
            } else {
                i += 1;
            }
        }
    }
}

const OP_SSTX: u8 = 0xba;
const OP_SSGEN: u8 = 0xbb;
const OP_SSRTX: u8 = 0xbc;
const OP_SSTXCHANGE: u8 = 0xbd;

fn p2pkh(hash: &[u8; 20]) -> Vec<u8> {
    let mut s = Vec::with_capacity(25);
    s.extend_from_slice(&[0x76, 0xa9, 0x14]);
    s.extend_from_slice(hash);
    s.extend_from_slice(&[0x88, 0xac]);
    s
}

/// A P2PKH script behind a stake opcode.
fn tagged(op: u8, hash: &[u8; 20]) -> Vec<u8> {
    let mut s = vec![op];
    s.extend_from_slice(&p2pkh(hash));
    s
}

/// `OP_RETURN` and one push of `data` (at most 75 bytes).
fn nulldata(data: &[u8]) -> Vec<u8> {
    let mut s = vec![0x6a, u8::try_from(data.len()).expect("a short push")];
    assert!(data.len() <= 75);
    s.extend_from_slice(data);
    s
}

/// A ticket commitment to a P2PKH address for `amount`, with the default
/// fee limits.
fn commitment(hash: &[u8; 20], amount: i64) -> Vec<u8> {
    let mut payload = hash.to_vec();
    payload.extend_from_slice(&amount.to_le_bytes());
    payload.extend_from_slice(&0x5800u16.to_le_bytes());
    nulldata(&payload)
}

/// A ticket's output 0 as the vote or revocation spending it sees it.
fn ticket_output(t: &Ticket) -> Utxo {
    Utxo {
        hash: t.hash,
        index: 0,
        tree: 1,
        value: t.price,
        height: t.height,
        kind: OutKind::Regular,
    }
}

/// The ticket price of block `height`: 20 to 40 coins, the same within
/// each stake difficulty window of 8 blocks (simnet's).
fn ticket_price(height: u32) -> i64 {
    let mut r = SplitMix64(0x7469_636b_6574_7300 ^ u64::from(height / 8));
    2_000_000_000 + i64::try_from(r.next_u64() % 2_000_000_000).expect("fits")
}

fn simnet_genesis() -> BlockHeader {
    dcroxide_chaincfg::simnet_params().genesis_block.header
}

/// The generator's random stream, which gives the wallet seed first.
const WORKLOAD_STREAM: u64 = 0x6d65_6173_7572_6532;

/// The public keys of accounts `0..n` of the wallet from `seed`, derived
/// as the address manager derives them: account 0 from the coin-type key
/// as `ExtendedKey::child` returns it (the wallet's creation derives it in
/// memory), every later one from the coin-type key as the address manager
/// stores it and parses it back (`NextAccount` decrypts the stored key and
/// calls `NewKeyFromString`), which pads a private key that `child`
/// returned with a leading zero byte stripped back to 32 bytes and so
/// derives other children from it (`purse-hdkeychain`'s module
/// documentation).
fn account_keys(seed: &[u8; 32], chain: &ChainParams, n: u32) -> Vec<ExtendedKey> {
    let master = ExtendedKey::new_master(seed, &chain.hd).expect("master key");
    let coin = master
        .child(44 + HARDENED)
        .and_then(|k| k.child(chain.legacy_coin_type + HARDENED))
        .expect("coin type key");
    let stored = ExtendedKey::new_key_from_string(&coin.string(), &chain.hd)
        .expect("coin type key parsed back");
    (0..n)
        .map(|a| {
            let parent = if a == 0 { &coin } else { &stored };
            parent.child(a + HARDENED).expect("account key").neuter()
        })
        .collect()
}

fn unmined_op(msg: &MsgTx, unpublished: bool) -> Op {
    Op::Unmined(UnminedTx {
        tx: msg.serialize(),
        msg: msg.clone(),
        unpublished,
    })
}

/// The generated history. Before the tip, blocks only (history sync, which
/// records what is mined). At the tip the wallet also acts between blocks,
/// in this order: it abandons the unmined sends whose fate that is, publishes
/// sends unmined (`unmined_rate`), publishes a ticket unmined for the next
/// block (`ticket_rate`), and records unmined its votes on the block just
/// connected (as `VoteOnOwnedTickets` records them); then the next block
/// comes, or a reorganisation: `Rollback` of one to three blocks and, in the
/// same write transaction, blocks with new headers holding the same
/// transactions at the same heights but the send added to the old chain's
/// last block to stay unmined, and one more block. Votes are only ever
/// on a block that stays in the main chain: no wallet vote is called in a
/// block a reorganisation detaches above its fork, nor recorded unmined on
/// the block a reorganisation follows (the new block holds the votes on the
/// new parent). After the last block the wallet acts once more, and what
/// it publishes then stays unmined.
fn generate(p: &Preset, chain: &ChainParams) -> Workload {
    let params = dcroxide_chaincfg::simnet_params();
    let maturity = u32::from(params.ticket_maturity);
    let expiry = params.ticket_expiry;
    let stake_enabled = u32::try_from(params.stake_enabled_height).expect("height");
    let svh = u32::try_from(params.stake_validation_height).expect("height");
    // TicketsPerBlock of a pool of TicketPoolSize * TicketsPerBlock.
    let call = 1.0 / f64::from(params.ticket_pool_size);
    let mut rng = SplitMix64(WORKLOAD_STREAM);
    let mut seed = [0u8; 32];
    rng.fill(&mut seed);
    let accts = account_keys(&seed, chain, p.accounts)
        .into_iter()
        .map(|acct| Account {
            branches: [
                acct.child(0).expect("branch"),
                acct.child(1).expect("branch"),
            ],
            derived: [Vec::new(), Vec::new()],
            next: [0, 0],
            used_ext: Vec::new(),
        })
        .collect();
    let mut g = Gen {
        rng,
        accts,
        utxos: vec![Vec::new(); p.accounts as usize],
        live: Vec::new(),
        revoke: Vec::new(),
        pending: Vec::new(),
        coinbase_maturity: u32::from(params.coinbase_maturity),
        stats: Stats::default(),
    };
    let tip_start = p.blocks.saturating_sub(p.tip_blocks) + 1;
    // Reorganisations: at height b, the blocks from b - depth up are
    // replaced; no wallet vote is called in the replaced blocks above the
    // fork (they would vote on blocks that leave the main chain).
    let mut reorgs: BTreeMap<u32, u32> = BTreeMap::new();
    let mut no_votes: BTreeSet<u32> = BTreeSet::new();
    if p.reorg_every > 0 {
        let mut b = tip_start + 4;
        let mut k = 0;
        while b <= p.blocks {
            let depth = REORG_DEPTHS[k % REORG_DEPTHS.len()];
            reorgs.insert(b, depth);
            no_votes.extend(b - depth + 1..b);
            b += p.reorg_every;
            k += 1;
        }
    }
    let genesis = simnet_genesis();
    let mut prev = genesis.block_hash().0;
    let mut recent: BTreeMap<u32, (BlockHeader, Block)> = BTreeMap::new();
    let mut sync = Vec::new();
    let mut tip = Vec::new();
    let mut fate_i = 0usize;
    // The sends the generator puts in the last block of each
    // reorganisation's old chain and leaves out of its new chain.
    let mut leave_out: BTreeSet<[u8; 32]> = BTreeSet::new();
    // The regular transactions of the last block, which the next one mines
    // again when it disapproves that block.
    let mut parent_regular: Vec<MsgTx> = Vec::new();
    assert_ne!(p.disapprove_every, 1, "a disapproved block disapproving");
    // One pass more than there are blocks: the wallet acts after the last
    // block too, and what it publishes then stays unmined.
    for height in 1..=p.blocks + 1 {
        let in_tip = height >= tip_start;
        // The wallet acts between two blocks at the tip.
        let between = height > tip_start;
        let reorg = reorgs.get(&height).copied();
        let may_vote = height >= svh && !no_votes.contains(&height);
        if between {
            let mut i = 0;
            while i < g.pending.len() {
                if g.pending[i].fate == Fate::Abandon && g.pending[i].at == height {
                    let pd = g.pending.remove(i);
                    tip.push(Op::Abandon(pd.msg.tx_hash().0));
                    g.utxos[pd.account as usize].extend(pd.inputs);
                    g.stats.abandoned += 1;
                } else {
                    i += 1;
                }
            }
            // Two after the last block, so the wallet ends with some.
            let sends = if height > p.blocks {
                2
            } else {
                usize::from(g.unit() < p.unmined_rate)
                    + usize::from(g.unit() < p.unmined_rate * p.unmined_rate)
            };
            // After the last block, the first send is recorded unpublished,
            // so one such record is still unmined when the two exports are
            // compared and the queries run (the minconf-0 balances, the
            // unspent outputs and the selection at minconf 0 skip it).
            let mut unpublished_after_last = height > p.blocks;
            for _ in 0..sends {
                let Some(((msg, outputs), account, inputs)) = g.send(height) else {
                    continue;
                };
                let (fate, delay) = FATES[fate_i % FATES.len()];
                // One in ten is recorded unpublished (and mined later).
                let unpublished = fate_i % FATES.len() == 7 || unpublished_after_last;
                unpublished_after_last = false;
                fate_i += 1;
                g.stats.unmined_sends += 1;
                g.stats.unpublished += u64::from(unpublished);
                tip.push(unmined_op(&msg, unpublished));
                g.pending.push(Pending {
                    msg,
                    at: height + delay,
                    fate,
                    account,
                    inputs,
                    outputs,
                    ticket: None,
                    unpublished,
                });
            }
            if height >= stake_enabled
                && g.unit() < p.ticket_rate
                && let Some(((msg, outputs), t, inputs)) = g.ticket(height, ticket_price(height))
            {
                g.stats.unmined_tickets += 1;
                tip.push(unmined_op(&msg, false));
                g.pending.push(Pending {
                    msg,
                    at: height,
                    fate: Fate::Mine,
                    account: t.account,
                    inputs,
                    outputs,
                    ticket: Some(t),
                    unpublished: false,
                });
            }
            if may_vote && reorg.is_none() {
                for t in g.call(height, call, maturity, p.miss_share) {
                    let (msg, outputs) = g.vote(&t, height, &prev, true);
                    g.stats.unmined_votes += 1;
                    tip.push(unmined_op(&msg, false));
                    g.pending.push(Pending {
                        msg,
                        at: height,
                        fate: Fate::Mine,
                        account: t.account,
                        inputs: Vec::new(),
                        outputs,
                        ticket: None,
                        unpublished: false,
                    });
                }
            }
        }
        if height > p.blocks {
            break;
        }
        if let Some(depth) = reorg {
            let fork = height - depth;
            tip.push(Op::Reorg(fork));
            g.stats.reorgs[depth as usize - 1] += 1;
            let mut chain_prev = recent
                .get(&(fork - 1))
                .map(|(_, b)| b.hash)
                .expect("the fork's parent");
            for h in fork..height {
                let (mut hd, old) = recent.remove(&h).expect("a recent block");
                hd.prev_block.0 = chain_prev;
                hd.nonce = g.rng.next_u64() as u32;
                // The new chain leaves out the transaction the generator put
                // in the old chain's last block to stay unmined.
                let (txs, msgs): (Vec<Vec<u8>>, Vec<MsgTx>) = old
                    .txs
                    .into_iter()
                    .zip(old.msgs)
                    .filter(|(_, m)| !leave_out.contains(&m.tx_hash().0))
                    .unzip();
                let block = Block {
                    height: h,
                    hash: hd.block_hash().0,
                    hd,
                    txs,
                    msgs,
                };
                chain_prev = block.hash;
                g.stats.detached_blocks += 1;
                g.stats.remined += block.txs.len() as u64;
                tip.push(Op::Block {
                    block: block.clone(),
                    tip: true,
                    commit: false,
                });
                recent.insert(h, (hd, block));
            }
            prev = chain_prev;
        }
        // The block's transactions: regular ones, then stake ones.
        let mut regular = Vec::new();
        // A disapproval, in the history sync only (never in a block a
        // reorganisation detaches, nor in one it detaches back to): the
        // block's votes disapprove its parent and the block mines the
        // parent's regular transactions again, first, as a miner does with
        // the transactions a disapproval returns to its mempool. Their
        // outputs now date from this block, so nothing here spends them.
        let disapprove = p.disapprove_every > 0
            && height >= svh + 2
            && height + 2 < tip_start
            && height % p.disapprove_every == 0;
        if disapprove {
            g.stats.disapprovals += 1;
            let moved: BTreeSet<[u8; 32]> = parent_regular.iter().map(|m| m.tx_hash().0).collect();
            for u in g.utxos.iter_mut().flatten() {
                if moved.contains(&u.hash) {
                    u.height = height;
                }
            }
            g.stats.invalidated_remined += parent_regular.len() as u64;
            regular.append(&mut parent_regular);
        }
        let mut stake = Vec::new();
        let mut new_utxos: Vec<(u32, Utxo)> = Vec::new();
        let mut returned: Vec<(u32, Utxo)> = Vec::new();
        let mut mined_tickets = Vec::new();
        let keep = |msg: MsgTx, outs: Vec<Out>, new_utxos: &mut Vec<(u32, Utxo)>| -> MsgTx {
            let hash = msg.tx_hash().0;
            for (account, index, value, tree, kind) in outs {
                new_utxos.push((
                    account,
                    Utxo {
                        hash,
                        index,
                        tree,
                        value,
                        height,
                        kind,
                    },
                ));
            }
            msg
        };
        if may_vote && !(between && reorg.is_none()) {
            for t in g.call(height, call, maturity, p.miss_share) {
                let (msg, outs) = g.vote(&t, height, &prev, !disapprove);
                stake.push(keep(msg, outs, &mut new_utxos));
            }
        }
        g.expire(height, maturity, expiry);
        let (due, later): (Vec<_>, Vec<_>) = g.revoke.drain(..).partition(|(at, _)| *at == height);
        g.revoke = later;
        for (_, t) in due {
            let (msg, outs) = g.revocation(&t);
            stake.push(keep(msg, outs, &mut new_utxos));
        }
        let (now, later): (Vec<_>, Vec<_>) = g
            .pending
            .drain(..)
            .partition(|pd| pd.at == height && pd.fate != Fate::Abandon);
        g.pending = later;
        for pd in now {
            match pd.fate {
                Fate::Mine => {
                    g.stats.mined_from_unmined += 1;
                    let is_stake = dcroxide_stake::determine_tx_type(&pd.msg) != TxType::Regular;
                    if let Some(t) = pd.ticket {
                        mined_tickets.push(t);
                    }
                    let msg = keep(pd.msg, pd.outputs, &mut new_utxos);
                    if is_stake {
                        stake.push(msg);
                    } else {
                        regular.push(msg);
                    }
                }
                Fate::Conflict => {
                    g.stats.conflicts += 1;
                    let (first, rest) = pd.inputs.split_first().expect("an input");
                    let (msg, outs) = g.pay_from(pd.account, std::slice::from_ref(first));
                    regular.push(keep(msg, outs, &mut new_utxos));
                    returned.extend(rest.iter().map(|u| (pd.account, u.clone())));
                }
                Fate::Abandon => unreachable!("abandoned between blocks"),
            }
        }
        let ntx = g.below(p.txs_bound as usize);
        for _ in 0..ntx {
            let built = if g.unit() < p.send_share {
                g.send(height).map(|(b, _, _)| b)
            } else {
                None
            };
            let (msg, outs) = built.unwrap_or_else(|| g.receive(height));
            regular.push(keep(msg, outs, &mut new_utxos));
        }
        // In the last block a reorganisation detaches, one send more, whose
        // change nothing spends: the new chain leaves it out, so `Rollback`
        // returns it to the unmined transactions and it stays there to the
        // end, when the exports are compared and the queries run.
        if reorgs.contains_key(&(height + 1))
            && let Some(((msg, _), _, _)) = g.send(height)
        {
            leave_out.insert(msg.tx_hash().0);
            g.stats.detached_left_unmined += 1;
            regular.push(msg);
        }
        if !between
            && height >= stake_enabled
            && g.unit() < p.ticket_rate
            && let Some(((msg, outs), t, _)) = g.ticket(height, ticket_price(height))
        {
            mined_tickets.push(t);
            stake.push(keep(msg, outs, &mut new_utxos));
        }
        for (account, u) in new_utxos.into_iter().chain(returned) {
            g.utxos[account as usize].push(u);
        }
        for mut t in mined_tickets {
            t.height = height;
            g.live.push(t);
        }
        let mut header = genesis;
        let mut roots = [0u8; 64];
        g.rng.fill(&mut roots);
        header.prev_block.0 = prev;
        header.merkle_root.0.copy_from_slice(&roots[..32]);
        header.stake_root.0.copy_from_slice(&roots[32..]);
        header.vote_bits = u16::from(!disapprove);
        header.voters = 5;
        header.sbits = ticket_price(height);
        header.height = height;
        header.timestamp = genesis.timestamp + height * 300;
        header.nonce = g.rng.next_u64() as u32;
        parent_regular.clone_from(&regular);
        let msgs: Vec<MsgTx> = regular.into_iter().chain(stake).collect();
        let block = Block {
            height,
            hash: header.block_hash().0,
            hd: header,
            txs: msgs.iter().map(MsgTx::serialize).collect(),
            msgs,
        };
        prev = block.hash;
        if in_tip {
            recent.insert(height, (header, block.clone()));
            recent.retain(|&h, _| h + 4 >= height);
            tip.push(Op::Block {
                block,
                tip: true,
                commit: true,
            });
        } else {
            let commit = height % p.batch == 0 || height + 1 == tip_start;
            sync.push(Op::Block {
                block,
                tip: false,
                commit,
            });
        }
    }
    g.stats.left_unmined = g.pending.len() as u64;
    g.stats.unpublished_left_unmined = g.pending.iter().filter(|pd| pd.unpublished).count() as u64;
    let mut ranges = Vec::new();
    for account in 0..p.accounts {
        for branch in 0..2usize {
            // Every child up to the last used one plus the gap, so the
            // ingest's gap-limit syncs record nothing new on either side.
            let n = g.accts[account as usize].next[branch] + GAP;
            let hashes = (0..n).map(|i| g.address(account, branch, i)).collect();
            ranges.push(AddrRange {
                account,
                branch: u32::try_from(branch).expect("branch"),
                hashes,
            });
        }
    }
    Workload {
        seed,
        accounts: p.accounts,
        ranges,
        sync,
        tip,
        stats: g.stats,
        left_out: leave_out.into_iter().collect(),
    }
}

/// Writes `ops` as a `PURSEWL1` file (the spike's format, which storebench
/// reads, with operation kind 5 added): the account count and address
/// ranges, then each operation (kinds 1 and 3, a block with its commit
/// flag; 2, an unmined transaction with its unpublished flag; 4, a
/// reorganisation's rollback height; 5, the hash of a transaction to
/// abandon).
fn write_workload(path: &Path, w: &Workload, ops: &[Op]) {
    let mut f = Vec::new();
    f.extend_from_slice(b"PURSEWL1");
    let u32_len = |n: usize| u32::try_from(n).expect("fits").to_le_bytes();
    f.extend_from_slice(&w.accounts.to_le_bytes());
    f.extend_from_slice(&u32_len(w.ranges.len()));
    for r in &w.ranges {
        f.extend_from_slice(&r.account.to_le_bytes());
        f.extend_from_slice(&r.branch.to_le_bytes());
        f.extend_from_slice(&u32_len(r.hashes.len()));
        for h in &r.hashes {
            f.extend_from_slice(h);
        }
    }
    f.extend_from_slice(&u32_len(ops.len()));
    for op in ops {
        match op {
            Op::Block { block, tip, commit } => {
                f.push(if *tip { 3 } else { 1 });
                f.push(u8::from(*commit));
                f.extend_from_slice(&block.hd.serialize());
                f.extend_from_slice(&u32_len(block.txs.len()));
                for tx in &block.txs {
                    f.extend_from_slice(&u32_len(tx.len()));
                    f.extend_from_slice(tx);
                }
            }
            Op::Unmined(u) => {
                f.push(2);
                f.push(u8::from(u.unpublished));
                f.extend_from_slice(&u32_len(u.tx.len()));
                f.extend_from_slice(&u.tx);
            }
            Op::Reorg(height) => {
                f.push(4);
                f.extend_from_slice(&height.to_le_bytes());
            }
            Op::Abandon(hash) => {
                f.push(5);
                f.extend_from_slice(hash);
            }
        }
    }
    fs::write(path, f).expect("write workload");
}

// ---------------------------------------------------------------------------
// Timing

#[derive(Clone, Copy, Debug, Default)]
struct Summary {
    n: usize,
    min: f64,
    p50: f64,
    p90: f64,
    max: f64,
}

/// Nearest-rank percentiles, as storebench's `summarize`.
fn summarize(d: &[f64]) -> Summary {
    if d.is_empty() {
        return Summary::default();
    }
    let mut s = d.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = |p: usize| s[((p * s.len()).div_ceil(100)).max(1) - 1];
    Summary {
        n: s.len(),
        min: s[0],
        p50: rank(50),
        p90: rank(90),
        max: s[s.len() - 1],
    }
}

fn go_summary(v: &Value) -> Summary {
    let f = |k: &str| {
        v[k].as_f64()
            .unwrap_or_else(|| panic!("summary field {k} in {v}"))
    };
    Summary {
        n: v["n"].as_u64().expect("n") as usize,
        min: f("min"),
        p50: f("p50"),
        p90: f("p90"),
        max: f("max"),
    }
}

#[derive(Clone, Copy, Debug)]
struct Reps {
    min: usize,
    max: usize,
    budget: Duration,
}

impl Reps {
    fn json(self) -> Value {
        json!({"min_reps": self.min, "max_reps": self.max, "budget_ms": self.budget.as_millis() as u64})
    }
}

/// storebench's `repeat`: at least `min` runs and until the budget has
/// passed, at most `max`, each timed.
fn repeat(r: Reps, mut f: impl FnMut()) -> Summary {
    let mut d = Vec::new();
    let start = Instant::now();
    while d.len() < r.max && (d.len() < r.min || start.elapsed() < r.budget) {
        let t0 = Instant::now();
        f();
        d.push(t0.elapsed().as_nanos() as f64);
    }
    summarize(&d)
}

/// [`repeat`] for a query whose answer is kept in `out`: `reset` runs
/// before each timed `f`, outside the timing, and releases the previous
/// repetition's answer (or clears it, keeping its capacity, as storebench
/// resets a slice with `[:0]`), so no repetition's time holds freeing the
/// last one's answer, which Go leaves to its collector.
fn repeat_kept<T>(
    r: Reps,
    out: &mut T,
    mut reset: impl FnMut(&mut T),
    mut f: impl FnMut(&mut T),
) -> Summary {
    let mut d = Vec::new();
    let start = Instant::now();
    while d.len() < r.max && (d.len() < r.min || start.elapsed() < r.budget) {
        reset(out);
        let t0 = Instant::now();
        f(out);
        d.push(t0.elapsed().as_nanos() as f64);
    }
    summarize(&d)
}

/// Clears a lookup batch's kept lines, keeping their capacity, and its
/// hit count (for [`repeat_kept`]).
fn clear_lines(kept: &mut (Vec<String>, usize)) {
    kept.0.clear();
    kept.1 = 0;
}

/// Drops a kept answer (for [`repeat_kept`]).
fn release<T: Default>(v: &mut T) {
    drop(std::mem::take(v));
}

fn ns(d: Duration) -> f64 {
    d.as_nanos() as f64
}

// ---------------------------------------------------------------------------
// The Rust side: the address manager and the transaction store of
// purse-udb over purse-store, fed with the wallet's call sequence

struct RustWallet {
    store: Store,
    mgr: Manager,
    ts: TxStore,
    chain: ChainParams,
}

/// The received time of every record the Rust side ingests. Go records
/// `time.Now()`; neither the export nor any check reads it.
const RECEIVED: i64 = 1_700_000_000;

fn open_manager(store: &Store, chain: &ChainParams) -> Manager {
    let mut tx = store.read().expect("read");
    Manager::open(
        &mut tx,
        chain,
        PUB_PASS,
        Box::new(OsRandom),
        Limits::DEFAULT,
    )
    .expect("open manager")
}

fn open_rust(path: &Path, chain: &ChainParams) -> RustWallet {
    let store = Store::open(path).expect("open store");
    let mgr = open_manager(&store, chain);
    RustWallet {
        store,
        mgr,
        ts: TxStore::new(chain).expect("txstore"),
        chain: *chain,
    }
}

/// Panics with `what` and the transaction store's error.
fn must<T>(r: Result<T, AnyError>, what: &str) -> T {
    r.unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// `setup` on the Rust side: the wallet from the seed (the address
/// manager's rows and the transaction store's, as `udb.Initialize` creates
/// both), the accounts, every address of the ranges; then every address
/// checked.
fn rust_setup(path: &Path, w: &Workload, chain: &ChainParams) -> (RustWallet, usize) {
    let store = Store::create(path).expect("create store");
    let ts = TxStore::new(chain).expect("txstore");
    store
        .update(|tx| {
            Manager::create(
                tx,
                chain,
                &w.seed,
                PUB_PASS,
                PRIV_PASS,
                &mut OsRandom,
                &Limits::DEFAULT,
            )
            .expect("create manager");
            must(ts.create(tx), "create the transaction store");
            Ok::<_, StoreError>(())
        })
        .expect("commit creation");
    let mgr = open_manager(&store, chain);
    store
        .update(|tx| {
            mgr.unlock(PRIV_PASS).expect("unlock");
            for a in 1..w.accounts {
                let n = mgr
                    .new_account(tx, format!("acct-{a}").as_bytes())
                    .expect("new account");
                assert_eq!(n, a);
            }
            for r in &w.ranges {
                if let Some(last) = r.hashes.len().checked_sub(1) {
                    mgr.sync_account_to_addr_index(
                        tx,
                        r.account,
                        u32::try_from(last).expect("index"),
                        r.branch,
                    )
                    .expect("sync");
                }
            }
            mgr.lock().expect("lock");
            Ok::<_, StoreError>(())
        })
        .expect("commit setup");
    let mut mismatches = 0;
    {
        let mut tx = store.read().expect("read");
        for r in &w.ranges {
            for (i, h) in r.hashes.iter().enumerate() {
                let addr = stdaddr::new_address_pub_key_hash_ecdsa_secp256k1_v0(h, chain)
                    .expect("address");
                match mgr.address(&mut tx, &addr) {
                    Ok(ma)
                        if ma.account() == r.account
                            && u32::from(ma.internal()) == r.branch
                            && ma.index() == Some(u32::try_from(i).expect("index")) => {}
                    _ => mismatches += 1,
                }
            }
        }
    }
    (
        RustWallet {
            store,
            mgr,
            ts,
            chain: *chain,
        },
        mismatches,
    )
}

/// `markUsedAddress` (wallet/addresses.go:718, storebench's): `MarkUsed`,
/// then the gap-limit sync of the address's branch.
fn mark_used_address(tx: &mut WriteTx<'_>, rw: &RustWallet, ma: &ManagedAddress) {
    rw.mgr.mark_used(tx, ma.address()).expect("mark used");
    let account = ma.account();
    if account == IMPORTED_ACCOUNT {
        return;
    }
    let props = rw
        .mgr
        .account_properties(&mut *tx, account)
        .expect("account properties");
    let (last_used, branch) = if ma.internal() {
        (props.last_used_internal_index, 1)
    } else {
        (props.last_used_external_index, 0)
    };
    rw.mgr
        .sync_account_to_addr_index(
            tx,
            account,
            (HARDENED - 1).min(last_used.wrapping_add(GAP)),
            branch,
        )
        .expect("gap sync");
}

/// `processTransactionRecord` (wallet/chainntfns.go:438) as storebench's
/// `processTx` runs it, through the transaction store's own functions:
/// `InsertMinedTx` with a block, `InsertMemPoolTx` without one (an
/// existing mined record is no error); then, per output paying a recorded
/// address, `AddTicketCommitment` for a ticket's commitment output and
/// `AddCredit` otherwise, each followed by `markUsedAddress`; then
/// `RedeemTicketCommitments` for a vote or revocation.
fn process_tx(tx: &mut WriteTx<'_>, rw: &RustWallet, rec: &TxRecord, block: Option<&BlockMeta>) {
    match block {
        None => match rw.ts.insert_mem_pool_tx(tx, rec) {
            Err(e) if e.is_kind(Kind::Exist) => return,
            r => must(r, "InsertMemPoolTx"),
        },
        Some(b) => must(
            rw.ts.insert_mined_tx(tx, rec, &b.block.hash),
            "InsertMinedTx",
        ),
    }
    let msg = rec.msg_tx();
    // Go computes and discards these (the multisig paths, which the
    // workload never reaches); the same work here.
    for input in &msg.tx_in {
        std::hint::black_box(stdscript::is_multi_sig_sig_script_v0(
            &input.signature_script,
        ));
    }
    let tx_type = rec.tx_type();
    for (i, out) in msg.tx_out.iter().enumerate() {
        let (class, mut addrs) = stdscript::extract_addrs(out.version, &out.pk_script, &rw.chain);
        if class == ScriptType::NonStandard {
            continue;
        }
        std::hint::black_box(purse_txrules::stake_sub_script_type(class));
        let index = u32::try_from(i).expect("output index");
        let commitment = tx_type == TxType::SStx && i % 2 == 1;
        if commitment {
            match dcroxide_stake::addr_from_sstx_pk_scr_commitment(&out.pk_script, &rw.chain) {
                Ok(a) => addrs = vec![a],
                Err(_) => continue,
            }
        } else if out.value == 0 {
            continue;
        }
        for addr in &addrs {
            let ma = match rw.mgr.address(&mut *tx, addr) {
                Ok(ma) => ma,
                Err(e) if e.is_kind(Kind::NotExist) => continue,
                Err(e) => panic!("address lookup: {e}"),
            };
            if commitment {
                must(
                    rw.ts.add_ticket_commitment(tx, rec, index, ma.account()),
                    "AddTicketCommitment",
                );
            } else {
                must(
                    rw.ts
                        .add_credit(tx, rec, block, index, ma.internal(), ma.account()),
                    "AddCredit",
                );
            }
            mark_used_address(tx, rw, &ma);
        }
    }
    if matches!(tx_type, TxType::SSGen | TxType::SSRtx) {
        must(
            rw.ts.redeem_ticket_commitments(tx, rec, block),
            "RedeemTicketCommitments",
        );
    }
}

/// `extendMainChain` (wallet/chainntfns.go:29, storebench's): the block's
/// hash computed from its decoded header (`header.BlockHash()`), an empty
/// version 2 filter built (`gcs.FromBytesV2` of no bytes), the block
/// through `ExtendMainChain` with the serialized header and that filter's
/// bytes (Go's `ExtendMainChain` serializes both itself; a block already
/// in the main chain is no error), its `BlockMeta` read back, and each
/// transaction recorded from its decoded form, as `NewTxRecordFromMsgTx`
/// serializes it, then processed.
fn ingest_block(tx: &mut WriteTx<'_>, rw: &RustWallet, b: &Block) -> Hash32 {
    let hash = Hash32(b.hd.block_hash().0);
    let header = HeaderBytes(b.hd.serialize());
    let filter =
        FilterV2::from_bytes(blockcf2::B, blockcf2::M, &[]).expect("an empty version 2 filter");
    match rw
        .ts
        .extend_main_chain(tx, &header, &hash, Some(filter.bytes()))
    {
        Err(e) if e.is_kind(Kind::Exist) => {}
        r => must(r, "ExtendMainChain"),
    }
    let meta = must(rw.ts.get_block_meta_for_hash(tx, &hash), "block meta");
    for msg in &b.msgs {
        let rec = must(TxRecord::from_msg_tx(msg.clone(), RECEIVED), "record");
        process_tx(tx, rw, &rec, Some(&meta));
    }
    hash
}

/// `AbandonTransaction` (wallet/wallet.go:5071, storebench's): the
/// transaction's details, refused if mined, then `RemoveUnconfirmed`.
fn abandon(tx: &mut WriteTx<'_>, rw: &RustWallet, hash: &[u8; 32]) {
    let hash = Hash32(*hash);
    let d = must(rw.ts.tx_details(&mut *tx, &hash), "TxDetails");
    assert_eq!(d.height(), -1, "abandoning a mined transaction");
    must(
        rw.ts.remove_unconfirmed(tx, d.record.msg_tx(), &hash),
        "RemoveUnconfirmed",
    );
}

/// Whether a block holds a stake transaction.
fn has_stake(b: &Block) -> bool {
    b.msgs
        .iter()
        .any(|m| dcroxide_stake::determine_tx_type(m) != TxType::Regular)
}

/// The commit times of an ingest, by the kind of the first operation of
/// each write transaction, as storebench's `ingest` reports them.
#[derive(Debug, Default)]
struct Ingested {
    sync_ns: f64,
    sync_blocks: usize,
    sync_commits: usize,
    tip_block: Vec<f64>,
    tip_block_stake: Vec<f64>,
    reorg: Vec<f64>,
    unmined_insert: Vec<f64>,
    unmined_remove: Vec<f64>,
    /// Transactions `PruneUnmined` removed.
    pruned: usize,
}

/// Applies `ops` as storebench's `ingest` applies them: one durable write
/// transaction per run of operations up to and including the next one that
/// commits; a reorganisation's `Rollback`, then the blocks after it
/// (`ChainSwitch`, wallet/chainntfns.go:160); an unmined transaction through
/// `process_tx` without a block; an abandon through [`abandon`]; a block
/// through [`ingest_block`]; and in each write transaction holding a block,
/// the processed-transactions marker moved to the last one and
/// `PruneUnmined`.
fn rust_ingest(rw: &RustWallet, ops: &[Op]) -> Ingested {
    let mut out = Ingested::default();
    let mut i = 0;
    while i < ops.len() {
        let mut j = i;
        while j < ops.len() && !ops[j].commit() {
            j += 1;
        }
        if j == ops.len() {
            j -= 1;
        }
        let group = &ops[i..=j];
        let t0 = Instant::now();
        rw.store
            .update(|tx| {
                let mut last = None;
                for op in group {
                    match op {
                        Op::Reorg(h) => {
                            let h = i32::try_from(*h).expect("height");
                            must(rw.ts.rollback(tx, h), "Rollback");
                        }
                        Op::Unmined(u) => {
                            let rec =
                                must(TxRecord::from_msg_tx(u.msg.clone(), RECEIVED), "record")
                                    .with_unpublished(u.unpublished);
                            process_tx(tx, rw, &rec, None);
                        }
                        Op::Abandon(hash) => abandon(tx, rw, hash),
                        Op::Block { block, .. } => last = Some(ingest_block(tx, rw, block)),
                    }
                }
                if let Some(last) = last {
                    must(
                        rw.ts.update_processed_txs_block_marker(tx, &last),
                        "UpdateProcessedTxsBlockMarker",
                    );
                    // ChainSwitch's last step (wallet/chainntfns.go:255),
                    // with the ticket price 0 it passes when the next one
                    // is unknown, as storebench passes it.
                    match rw.ts.prune_unmined(tx, 0) {
                        Ok(removed) => out.pruned += removed.len(),
                        Err(e) => panic!("PruneUnmined: {}", e.error),
                    }
                }
                Ok::<_, StoreError>(())
            })
            .expect("ingest commit");
        let d = ns(t0.elapsed());
        match &group[0] {
            Op::Block { tip: false, .. } => {
                out.sync_ns += d;
                out.sync_blocks += group.len();
                out.sync_commits += 1;
            }
            Op::Block { block, .. } => {
                out.tip_block.push(d);
                if has_stake(block) {
                    out.tip_block_stake.push(d);
                }
            }
            Op::Reorg(_) => out.reorg.push(d),
            Op::Unmined(_) => out.unmined_insert.push(d),
            Op::Abandon(_) => out.unmined_remove.push(d),
        }
        i = j + 1;
    }
    out
}

// ---------------------------------------------------------------------------
// The Rust side's answers, as storebench writes Go's

fn balance_text(b: &Balances) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        b.spendable,
        b.immature_coinbase_rewards,
        b.immature_stake_generation,
        b.locked_by_tickets,
        b.voting_authority,
        b.unconfirmed,
        b.total
    )
}

/// `RangeTransactions` from `begin` to `end`, appending every batch to
/// `batches` (the store lends each batch to the callback, so a caller
/// keeping them copies them, as storebench copies udb's), stopping after
/// `limit` transactions when it is above 0.
fn range_batches(
    rw: &RustWallet,
    rtx: &mut ReadTx,
    begin: i32,
    end: i32,
    limit: usize,
    batches: &mut Vec<Vec<TxDetails>>,
) {
    let mut n = 0usize;
    must(
        rw.ts.range_transactions(rtx, begin, end, |d| {
            batches.push(d.to_vec());
            n += d.len();
            Ok(limit > 0 && n >= limit)
        }),
        "RangeTransactions",
    );
}

/// The batches as storebench's export and `hot` write them.
fn range_lines(batches: &[Vec<TxDetails>]) -> Vec<String> {
    let mut lines = Vec::new();
    for batch in batches {
        lines.push(format!("batch {}", batch.len()));
        for d in batch {
            let mut s = format!("tx {} {}", hex(&d.record.hash().0), d.height());
            for c in &d.credits {
                let _ = write!(s, " c{}:{}:{}:{}", c.index, c.amount, c.spent, c.change);
            }
            for db in &d.debits {
                let _ = write!(s, " d{}:{}", db.index, db.amount);
            }
            lines.push(s);
        }
    }
    lines
}

fn digest_text(n: usize, lines: &[String]) -> String {
    let mut h = Sha256::new();
    for l in lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    format!("{n} {}", hex(&h.finalize()))
}

// ---------------------------------------------------------------------------
// The queries both sides run

#[derive(Clone, Debug)]
struct RangeSpec {
    name: &'static str,
    /// A height, or -1 for the unmined transactions.
    begin: i32,
    end: i32,
    limit: usize,
}

#[derive(Clone, Debug)]
struct HotArgs {
    reps: Reps,
    balances: Vec<(u32, i32)>,
    all_minconfs: Vec<i32>,
    ranges: Vec<RangeSpec>,
    scripts: Vec<Vec<u8>>,
    names: Vec<String>,
    props: Vec<u32>,
    next_addr: usize,
    /// `MakeInputSource` and `SelectInputs(target)` for (account, minconf,
    /// target), the tip as the sync height, one source per repetition.
    selects: Vec<(u32, i32, i64)>,
    /// `RandomUTXO` calls at minconf 1, all in one read transaction per
    /// repetition.
    random_utxos: usize,
    /// `UnspentTickets(tip, includeImmature)` and a walk of
    /// `IterateTickets`, each one call per repetition.
    tickets: bool,
    /// The tip after the ingest.
    tip: i32,
}

impl HotArgs {
    fn new(w: &Workload, reps: Reps) -> HotArgs {
        let tip = i32::try_from(w.tip_height()).expect("height");
        let mut rng = SplitMix64(0x6c6f_6f6b_7570_7331);
        let mut scripts = Vec::with_capacity(1_000);
        let all: Vec<&[u8; 20]> = w.ranges.iter().flat_map(|r| &r.hashes).collect();
        for i in 0..1_000u32 {
            if i % 10 == 9 {
                let mut h = [0u8; 20];
                rng.fill(&mut h);
                scripts.push(p2pkh(&h));
            } else {
                let j = usize::try_from(rng.below(all.len() as u64)).expect("fits");
                scripts.push(p2pkh(all[j]));
            }
        }
        let mut names: Vec<String> = (0..w.accounts)
            .map(|a| {
                if a == 0 {
                    "default".to_owned()
                } else {
                    format!("acct-{a}")
                }
            })
            .collect();
        names.push("no-such-account".to_owned());
        let names = (0..1_000).map(|i| names[i % names.len()].clone()).collect();
        let props = (0..1_000u32).map(|i| i % w.accounts).collect();
        let last = w.accounts - 1;
        HotArgs {
            reps,
            balances: vec![(0, 1), (0, 6), (last, 1), (last, 6)],
            all_minconfs: vec![1],
            ranges: vec![
                RangeSpec {
                    name: "last_1000_blocks_backward",
                    begin: tip,
                    end: (tip - 999).max(0),
                    limit: 0,
                },
                RangeSpec {
                    name: "mid_1000_blocks_forward",
                    begin: tip / 2,
                    end: tip / 2 + 999,
                    limit: 0,
                },
                RangeSpec {
                    name: "recent_100_transactions",
                    begin: tip,
                    end: 0,
                    limit: 100,
                },
                RangeSpec {
                    name: "whole_history",
                    begin: 0,
                    end: tip,
                    limit: 0,
                },
            ],
            scripts,
            names,
            props,
            next_addr: 20,
            selects: vec![
                (0, 1, 0),
                (0, 1, 100_000_000),
                (0, 0, 5_000_000_000),
                (last, 1, 100_000_000),
            ],
            random_utxos: 1_000,
            tickets: true,
            tip,
        }
    }

    /// The export's ranges: the timed ones, and two that hold the unmined
    /// transactions (`RangeTransactions` from -1 down to 50 blocks below
    /// the tip, and from there up to -1), which no timed range reaches.
    fn export_ranges(&self) -> Vec<RangeSpec> {
        let low = (self.tip - 49).max(0);
        let mut v = self.ranges.clone();
        v.push(RangeSpec {
            name: "unmined_then_last_50_blocks",
            begin: -1,
            end: low,
            limit: 0,
        });
        v.push(RangeSpec {
            name: "last_50_blocks_then_unmined",
            begin: low,
            end: -1,
            limit: 0,
        });
        v
    }

    fn ranges_json(ranges: &[RangeSpec]) -> Value {
        Value::Array(
            ranges
                .iter()
                .map(|r| json!({"name": r.name, "begin": r.begin, "end": r.end, "limit": r.limit}))
                .collect(),
        )
    }

    fn json(&self, db: &Path) -> Value {
        let mut v = self.reps.json();
        v["db"] = json!(db.to_str().expect("utf-8 path"));
        v["balances"] = Value::Array(
            self.balances
                .iter()
                .map(|(a, mc)| json!({"account": a, "minconf": mc}))
                .collect(),
        );
        v["all_minconfs"] = json!(self.all_minconfs);
        v["ranges"] = Self::ranges_json(&self.ranges);
        v["scripts"] = json!(self.scripts.iter().map(|s| hex(s)).collect::<Vec<_>>());
        v["names"] = json!(self.names);
        v["props"] = json!(self.props);
        v["next_addr"] = json!(self.next_addr);
        v["selects"] = Value::Array(
            self.selects
                .iter()
                .map(|(a, mc, t)| json!({"account": a, "minconf": mc, "target": t}))
                .collect(),
        );
        v["random_utxos"] = json!(self.random_utxos);
        v["tickets"] = json!(self.tickets);
        v
    }
}

type Figures = BTreeMap<String, Summary>;
type Checks = BTreeMap<String, String>;

/// dcrd's generator as the transaction store's random stream.
struct OsStream(dcroxide_crypto::rand::Prng);

impl purse_snacl::RandomSource for OsStream {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), purse_snacl::RandomSourceError> {
        self.0.read(buf);
        Ok(())
    }
}

/// The Rust half of storebench's `hot`.
fn rust_hot(rw: &RustWallet, a: &HotArgs) -> (Figures, Checks) {
    let mut figs = Figures::new();
    let mut checks = Checks::new();
    let tip = {
        let mut rtx = rw.store.read().expect("read");
        must(rw.ts.main_chain_tip(&mut rtx), "MainChainTip").1
    };
    for &(account, mc) in &a.balances {
        let name = format!("balance/account={account}/minconf={mc}");
        let mut got = Balances::default();
        let s = repeat_kept(a.reps, &mut got, release, |got| {
            let mut rtx = rw.store.read().expect("read");
            *got = must(
                rw.ts.account_balance(&mut rtx, mc, account),
                "AccountBalance",
            );
        });
        figs.insert(name.clone(), s);
        checks.insert(name, balance_text(&got));
    }
    for &mc in &a.all_minconfs {
        let name = format!("balance_all/minconf={mc}");
        let mut got = Vec::new();
        let s = repeat_kept(a.reps, &mut got, release, |got| {
            let mut rtx = rw.store.read().expect("read");
            *got = must(rw.ts.account_balances(&mut rtx, mc), "AccountBalances");
        });
        figs.insert(name.clone(), s);
        let parts: Vec<String> = got
            .iter()
            .map(|b| format!("{}:{}", b.account, balance_text(b)))
            .collect();
        checks.insert(name, parts.join(";"));
    }
    for r in &a.ranges {
        let name = format!("range/{}", r.name);
        let mut got = Vec::new();
        let s = repeat_kept(a.reps, &mut got, Vec::clear, |got| {
            let mut rtx = rw.store.read().expect("read");
            range_batches(rw, &mut rtx, r.begin, r.end, r.limit, got);
        });
        figs.insert(name.clone(), s);
        let n = got.iter().map(Vec::len).sum();
        checks.insert(name, digest_text(n, &range_lines(&got)));
    }
    if !a.scripts.is_empty() {
        let mut kept = (Vec::new(), 0usize);
        let s = repeat_kept(a.reps, &mut kept, clear_lines, |(lines, hits)| {
            let mut rtx = rw.store.read().expect("read");
            for script in &a.scripts {
                let (_, addrs) = stdscript::extract_addrs(0, script, &rw.chain);
                let mut line = "-".to_owned();
                if let [addr] = addrs.as_slice()
                    && let Ok(ma) = rw.mgr.address(&mut rtx, addr)
                {
                    line = format!(
                        "{} {} {}",
                        ma.account(),
                        u8::from(ma.internal()),
                        ma.index().unwrap_or(0)
                    );
                    *hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/address".to_owned(), s);
        checks.insert("lookup/address".to_owned(), digest_text(kept.1, &kept.0));
        let s = repeat_kept(a.reps, &mut kept, clear_lines, |(lines, hits)| {
            let mut rtx = rw.store.read().expect("read");
            for script in &a.scripts {
                let (_, addrs) = stdscript::extract_addrs(0, script, &rw.chain);
                let mut line = "-".to_owned();
                if let [addr] = addrs.as_slice()
                    && let Ok(acct) = rw.mgr.addr_account(&mut rtx, addr)
                {
                    line = acct.to_string();
                    *hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/addr_account".to_owned(), s);
        checks.insert(
            "lookup/addr_account".to_owned(),
            digest_text(kept.1, &kept.0),
        );
    }
    if !a.names.is_empty() {
        let mut kept = (Vec::new(), 0usize);
        let s = repeat_kept(a.reps, &mut kept, clear_lines, |(lines, hits)| {
            let mut rtx = rw.store.read().expect("read");
            for name in &a.names {
                let mut line = "-".to_owned();
                if let Ok(acct) = rw.mgr.lookup_account(&mut rtx, name.as_bytes()) {
                    line = acct.to_string();
                    *hits += 1;
                }
                lines.push(line);
            }
        });
        figs.insert("lookup/account_by_name".to_owned(), s);
        checks.insert(
            "lookup/account_by_name".to_owned(),
            digest_text(kept.1, &kept.0),
        );
    }
    if !a.props.is_empty() {
        let mut lines = Vec::new();
        let s = repeat_kept(a.reps, &mut lines, Vec::clear, |lines| {
            let mut rtx = rw.store.read().expect("read");
            for &acct in &a.props {
                let p = rw
                    .mgr
                    .account_properties(&mut rtx, acct)
                    .expect("account properties");
                lines.push(format!(
                    "{} {} {} {} {} {}",
                    p.account_number,
                    String::from_utf8_lossy(&p.account_name),
                    p.last_used_external_index,
                    p.last_used_internal_index,
                    p.last_returned_external_index,
                    p.last_returned_internal_index
                ));
            }
        });
        figs.insert("lookup/account_properties".to_owned(), s);
        checks.insert(
            "lookup/account_properties".to_owned(),
            digest_text(lines.len(), &lines),
        );
    }
    // Input selection. The random stream is dcrd's generator seeded from
    // the operating system, as Go's is, so a selection for a target is
    // checked only for covering it; a selection of everything (target 0)
    // is checked as the set of outputs, which the shuffle does not change.
    let ts = &rw.ts;
    let tip_i = tip;
    for &(account, mc, target) in &a.selects {
        let name = format!("select/account={account}/minconf={mc}/target={target}");
        let mut check = String::new();
        let s = repeat_kept(a.reps, &mut check, release, |check| {
            let mut rtx = rw.store.read().expect("read");
            let mut rng = OsStream(dcroxide_crypto::rand::Prng::new().expect("os random"));
            let mut src = ts
                .make_input_source(&mut rtx, &mut rng, account, mc, tip_i, None)
                .expect("source");
            let d = src.select_inputs(Amount(target)).expect("select");
            *check = if target == 0 {
                let mut lines: Vec<String> = d
                    .inputs
                    .iter()
                    .map(|i| {
                        format!(
                            "{} {} {}",
                            i.previous_out_point.hash, i.previous_out_point.index, i.value_in
                        )
                    })
                    .collect();
                lines.sort();
                digest_text(lines.len(), &lines)
            } else {
                (d.amount.0 >= target).to_string()
            };
        });
        figs.insert(name.clone(), s);
        checks.insert(name, check);
    }
    if a.random_utxos > 0 {
        let mut found = 0usize;
        let s = repeat(a.reps, || {
            found = 0;
            let mut rtx = rw.store.read().expect("read");
            let mut rng = OsStream(dcroxide_crypto::rand::Prng::new().expect("os random"));
            for _ in 0..a.random_utxos {
                if ts
                    .random_utxo(&mut rtx, &mut rng, 1, tip_i)
                    .expect("RandomUTXO")
                    .is_some()
                {
                    found += 1;
                }
            }
        });
        figs.insert("random_utxo/minconf=1".to_owned(), s);
        checks.insert("random_utxo/minconf=1".to_owned(), found.to_string());
    }
    if a.tickets {
        // The answers are formatted after the timing on both sides.
        let mut got = Vec::new();
        let s = repeat_kept(a.reps, &mut got, release, |got| {
            let mut rtx = rw.store.read().expect("read");
            *got = must(ts.unspent_tickets(&mut rtx, tip_i, true), "UnspentTickets");
        });
        figs.insert("tickets/unspent".to_owned(), s);
        let lines: Vec<String> = got.iter().map(|h| hex(&h.0)).collect();
        checks.insert(
            "tickets/unspent".to_owned(),
            digest_text(lines.len(), &lines),
        );
        let mut got = Vec::new();
        let s = repeat_kept(a.reps, &mut got, release, |got| {
            let mut rtx = rw.store.read().expect("read");
            *got = must(ts.iterate_tickets(&mut rtx), "IterateTickets");
        });
        figs.insert("tickets/iterate".to_owned(), s);
        let lines: Vec<String> = got
            .iter()
            .map(|t| {
                format!(
                    "{} {} {}",
                    hex(&t.record.hash().0),
                    t.block.height,
                    hex(&t.spender.0)
                )
            })
            .collect();
        checks.insert(
            "tickets/iterate".to_owned(),
            digest_text(lines.len(), &lines),
        );
    }
    if a.next_addr > 0 {
        let mut next = 0u32;
        {
            let mut rtx = rw.store.read().expect("read");
            rw.mgr
                .for_each_account_address(&mut rtx, 0, |ma| {
                    if let Some(i) = ma.index()
                        && !ma.internal()
                        && i >= next
                    {
                        next = i + 1;
                    }
                    Ok(())
                })
                .expect("account addresses");
        }
        let first = next;
        let mut d = Vec::new();
        for _ in 0..a.next_addr {
            let child = next;
            next += 1;
            let t0 = Instant::now();
            rw.store
                .update(|tx| {
                    rw.mgr
                        .sync_account_to_addr_index(tx, 0, child, 0)
                        .expect("record one address");
                    Ok::<_, StoreError>(())
                })
                .expect("commit");
            d.push(ns(t0.elapsed()));
        }
        figs.insert("commit/next_address".to_owned(), summarize(&d));
        checks.insert("commit/next_address".to_owned(), format!("{first} {next}"));
    }
    (figs, checks)
}

/// The lines of storebench's canonical export, each read through the
/// transaction store's own queries as storebench reads Go's (no probes,
/// scans, per-account histories or selections are asked for, and nothing
/// is imported).
fn rust_export(rw: &RustWallet, minconfs: &[i32], ranges: &[RangeSpec]) -> Vec<String> {
    let ts = &rw.ts;
    let mut lines = Vec::new();
    let mut rtx = rw.store.read().expect("read");
    let (tip_hash, tip) = must(ts.main_chain_tip(&mut rtx), "MainChainTip");
    lines.push(format!("tip {tip} {}", hex(&tip_hash.0)));
    lines.push(format!(
        "keyn {}",
        must(ts.unspent_output_count(&mut rtx), "UnspentOutputCount")
    ));
    let accounts: Vec<u32> = rw
        .mgr
        .for_each_account(&mut rtx)
        .expect("accounts")
        .into_iter()
        .filter(|&a| a != IMPORTED_ACCOUNT)
        .collect();
    for &a in &accounts {
        let p = rw.mgr.account_properties(&mut rtx, a).expect("properties");
        lines.push(format!(
            "account {a} {} {} {} {}",
            p.last_used_external_index,
            p.last_used_internal_index,
            p.last_returned_external_index,
            p.last_returned_internal_index
        ));
    }
    for &mc in minconfs {
        for b in must(ts.account_balances(&mut rtx, mc), "AccountBalances") {
            lines.push(format!("balance {mc} {} {}", b.account, balance_text(&b)));
        }
    }
    let credits = must(ts.unspent_outputs(&mut rtx), "UnspentOutputs");
    lines.push(format!("unspent {}", credits.len()));
    for c in &credits {
        lines.push(format!(
            "utxo {} {} {} {} {} {} {}",
            hex(&c.outpoint.hash.0),
            c.outpoint.index,
            c.outpoint.tree,
            c.block.block.height,
            c.amount,
            c.from_coinbase,
            hex(&c.pk_script)
        ));
    }
    for r in ranges {
        lines.push(format!(
            "range {} {} {} {}",
            r.name, r.begin, r.end, r.limit
        ));
        let mut batches = Vec::new();
        range_batches(rw, &mut rtx, r.begin, r.end, r.limit, &mut batches);
        lines.extend(range_lines(&batches));
    }
    // Stake invalidation: the main-chain blocks marked invalid, then the
    // invalidated credits and debits (which the store keeps for
    // main-chain blocks only: Rollback deletes a detached block's, where
    // Go leaves them unread and its export skips them).
    for h in 0..=tip {
        let hash = must(
            ts.get_main_chain_block_hash_for_height(&mut rtx, h),
            "block hash",
        );
        if must(ts.block_in_main_chain(&mut rtx, &hash), "BlockInMainChain").1 {
            lines.push(format!("invalid {h} {}", hex(&hash.0)));
        }
    }
    for row in rtx
        .iter::<CreditsInvalidated>()
        .expect("invalidated credits")
    {
        let (k, c) = row.expect("invalidated credit");
        lines.push(format!(
            "inv_credit {} {} {} {} {}",
            hex(&k.hash.0),
            k.height,
            k.index,
            c.amount,
            c.spender.is_some()
        ));
    }
    for row in rtx.iter::<DebitsInvalidated>().expect("invalidated debits") {
        let (k, d) = row.expect("invalidated debit");
        lines.push(format!(
            "inv_debit {} {} {} {} {} {}",
            hex(&k.hash.0),
            k.height,
            k.index,
            d.amount,
            hex(&d.credit.hash.0),
            d.credit.index
        ));
    }
    let mut fe = Vec::new();
    must(
        ts.for_each_unspent_outpoint(&mut rtx, |op| {
            fe.push(format!("fe {} {} {}", hex(&op.hash.0), op.index, op.tree));
            Ok(())
        }),
        "ForEachUnspentOutpoint",
    );
    lines.push(format!("foreach {}", fe.len()));
    lines.extend(fe);
    for &a in &accounts {
        let mut h = Sha256::new();
        let mut n = 0;
        rw.mgr
            .for_each_account_address(&mut rtx, a, |ma| {
                h.update(ma.addr_hash());
                n += 1;
                Ok(())
            })
            .expect("account addresses");
        lines.push(format!("addrs {a} {n} {}", hex(&h.finalize())));
    }
    lines
}

// ---------------------------------------------------------------------------
// A session

const EXPORT_MINCONFS: [i32; 3] = [0, 1, 6];

/// The commit figures of the ingest at the tip, with the key of storebench's
/// `ingest` result that holds each.
const TIP_FIGURES: [(&str, &str); 5] = [
    ("commit/tip_block", "tip_block"),
    ("commit/tip_block_stake", "tip_block_stake"),
    ("commit/reorg", "reorg"),
    ("commit/unmined_insert", "unmined"),
    ("commit/unmined_remove", "unmined_remove"),
];

fn spawn_go(bin: &Path) -> Oracle {
    Oracle::spawn_at(bin)
}

fn go_export_args(db: &Path, out: &Path, a: &HotArgs) -> Value {
    json!({
        "db": db.to_str().expect("utf-8 path"),
        "out": out.to_str().expect("utf-8 path"),
        "minconfs": EXPORT_MINCONFS,
        "probes": [],
        "scans": [],
        "ranges": HotArgs::ranges_json(&a.export_ranges()),
        "by_account": [],
        "select_accounts": 0,
        "select_detail": false,
        "select_minconfs": [],
        "select_targets": [],
    })
}

fn file_size(p: &Path) -> u64 {
    fs::metadata(p).map_or(0, |m| m.len())
}

fn load_average() -> String {
    fs::read_to_string("/proc/loadavg")
        .map_or_else(|_| "unknown".to_owned(), |s| s.trim().to_owned())
}

fn affinity() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Cpus_allowed_list"))
                .map(|l| l.split_whitespace().last().unwrap_or("").to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One line per side, round and figure.
struct Record {
    lines: Vec<String>,
}

impl Record {
    fn add(&mut self, side: &str, round: &str, name: &str, s: &Summary) {
        self.lines.push(format!(
            "{side}\t{round}\t{name}\t{}\t{:.0}\t{:.0}\t{:.0}\t{:.0}",
            s.n, s.p50, s.p90, s.min, s.max
        ));
    }
}

/// Compares two sets of check values, panicking at the first difference.
fn assert_checks_equal(go: &Checks, rust: &Checks, round: usize) {
    assert_eq!(
        go.keys().collect::<Vec<_>>(),
        rust.keys().collect::<Vec<_>>(),
        "round {round}: the two sides ran different queries"
    );
    for (k, g) in go {
        assert_eq!(Some(g), rust.get(k), "round {round}: {k} differs");
    }
}

fn compare_exports(go_path: &Path, rust: &[String]) {
    let go = fs::read_to_string(go_path).expect("read Go export");
    let go: Vec<&str> = go.lines().collect();
    for (i, (g, r)) in go.iter().zip(rust).enumerate() {
        assert_eq!(*g, r.as_str(), "export line {} differs", i + 1);
    }
    assert_eq!(go.len(), rust.len(), "export lengths differ");
}

struct SessionResult {
    record: Record,
    notes: String,
    table: String,
    stats: Stats,
    left_out: Vec<[u8; 32]>,
}

fn run_session(dir: &Path, p: &Preset, reps: Reps, rounds: usize) -> Option<SessionResult> {
    let mut first_oracle = oracle_or_skip("storebench")?;
    let bin = oracle_bin_or_skip("storebench")?;
    let info = first_oracle.build_info();
    drop(first_oracle);
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let mut notes = String::new();
    let _ = writeln!(notes, "started (unix): {}", unix_now());
    let _ = writeln!(notes, "load average at start: {}", load_average());
    let _ = writeln!(notes, "cpus allowed: {}", affinity());
    let _ = writeln!(notes, "go: {}", info.go);
    for m in &info.deps {
        if m.path.contains("dcrwallet") || m.path.contains("bbolt") {
            let _ = writeln!(notes, "go module: {} {}", m.path, m.version);
        }
    }
    let _ = writeln!(notes, "redb: {}", purse_store::REDB_VERSION);
    let _ = writeln!(notes, "preset: {p:?}; reps: {reps:?}; rounds: {rounds}");

    let t0 = Instant::now();
    let w = generate(p, &chain);
    let _ = writeln!(
        notes,
        "workload: {} blocks ({} block operations at the tip), {} transactions, {} addresses; generated in {:.1} s",
        w.tip_height(),
        w.tip_ops(|o| matches!(o, Op::Block { .. })),
        w.transactions(),
        w.addresses(),
        t0.elapsed().as_secs_f64()
    );
    let _ = writeln!(notes, "workload operations: {:?}", w.stats);
    let wl_sync = dir.join("sync.wl");
    let wl_tip = dir.join("tip.wl");
    write_workload(&wl_sync, &w, &w.sync);
    write_workload(&wl_tip, &w, &w.tip);
    let path_str = |p: &Path| p.to_str().expect("utf-8 path").to_owned();
    let mut record = Record { lines: Vec::new() };

    // Setup and history sync, once per side.
    let go_dir = dir.join("go");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&go_dir).expect("mkdir");
    fs::create_dir_all(&rust_dir).expect("mkdir");
    let go_base = go_dir.join("base.db");
    let rust_base = rust_dir.join("base.redb");
    // What PruneUnmined removed in each ingest, per side (nothing, by the
    // generator's construction; equal either way).
    let mut go_pruned = Vec::new();
    let mut rust_pruned = Vec::new();
    {
        let mut go = spawn_go(&bin);
        let r = go.call(
            "setup",
            json!({"db": path_str(&go_base), "workload": path_str(&wl_sync), "seed": hex(&w.seed)}),
        );
        assert_eq!(
            r["mismatches"],
            json!(0),
            "Go recorded other addresses: {r}"
        );
        let r = go.call(
            "ingest",
            json!({"db": path_str(&go_base), "workload": path_str(&wl_sync), "gap": GAP}),
        );
        let blocks = r["sync_blocks"].as_f64().expect("sync_blocks").max(1.0);
        let per_block = r["sync_ns"].as_f64().expect("sync_ns") / blocks;
        let s = Summary {
            n: 1,
            min: per_block,
            p50: per_block,
            p90: per_block,
            max: per_block,
        };
        record.add("go", "setup", "ingest/history_sync_per_block", &s);
        let _ = writeln!(notes, "go history sync: {r}");
        go_pruned.push(r["pruned"].as_u64().expect("pruned"));
    }
    {
        let (rw, mismatches) = rust_setup(&rust_base, &w, &chain);
        assert_eq!(
            mismatches, 0,
            "the Rust address manager recorded other addresses"
        );
        let ing = rust_ingest(&rw, &w.sync);
        rust_pruned.push(ing.pruned as u64);
        let total = ing.sync_ns;
        let per_block = total / ing.sync_blocks.max(1) as f64;
        let s = Summary {
            n: 1,
            min: per_block,
            p50: per_block,
            p90: per_block,
            max: per_block,
        };
        record.add("rust", "setup", "ingest/history_sync_per_block", &s);
        let _ = writeln!(
            notes,
            "rust history sync: {} commits, {} blocks, {:.3} s",
            ing.sync_commits,
            ing.sync_blocks,
            total / 1e9
        );
        rw.store.close().expect("close");
    }
    let _ = writeln!(
        notes,
        "file sizes after the history sync: go {} bytes, rust {} bytes",
        file_size(&go_base),
        file_size(&rust_base)
    );

    let args = HotArgs::new(&w, reps);
    let mut go_checks: Option<Checks> = None;
    let mut rust_checks: Option<Checks> = None;
    let mut go_export: Option<PathBuf> = None;
    let mut rust_export_lines: Option<Vec<String>> = None;
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            ["rust", "go"]
        } else {
            ["go", "rust"]
        };
        for side in order {
            let label = round.to_string();
            if side == "go" {
                let work = go_dir.join("work.db");
                fs::copy(&go_base, &work).expect("copy");
                let mut go = spawn_go(&bin);
                let r = go.call(
                    "ingest",
                    json!({"db": path_str(&work), "workload": path_str(&wl_tip), "gap": GAP}),
                );
                for (name, key) in TIP_FIGURES {
                    record.add("go", &label, name, &go_summary(&r[key]));
                }
                go_pruned.push(r["pruned"].as_u64().expect("pruned"));
                if round == 0 {
                    let _ = writeln!(notes, "go tip ingest: {r}");
                }
                if go_export.is_none() {
                    let out = dir.join("export-go.txt");
                    go.call("export", go_export_args(&work, &out, &args));
                    go_export = Some(out);
                }
                let r = go.call("hot", args.json(&work));
                for t in r["timings"].as_array().expect("timings") {
                    let name = t["name"].as_str().expect("name");
                    record.add("go", &label, name, &go_summary(&t["summary"]));
                }
                let checks: Checks = r["checks"]
                    .as_object()
                    .expect("checks")
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().expect("check text").to_owned()))
                    .collect();
                if let Some(rc) = &rust_checks {
                    assert_checks_equal(&checks, rc, round);
                }
                go_checks.get_or_insert(checks);
                drop(go);
                let _ = fs::remove_file(&work);
            } else {
                let work = rust_dir.join("work.redb");
                fs::copy(&rust_base, &work).expect("copy");
                let rw = open_rust(&work, &chain);
                let ing = rust_ingest(&rw, &w.tip);
                rust_pruned.push(ing.pruned as u64);
                for (name, times) in [
                    ("commit/tip_block", &ing.tip_block),
                    ("commit/tip_block_stake", &ing.tip_block_stake),
                    ("commit/reorg", &ing.reorg),
                    ("commit/unmined_insert", &ing.unmined_insert),
                    ("commit/unmined_remove", &ing.unmined_remove),
                ] {
                    record.add("rust", &label, name, &summarize(times));
                }
                if rust_export_lines.is_none() {
                    rust_export_lines =
                        Some(rust_export(&rw, &EXPORT_MINCONFS, &args.export_ranges()));
                }
                let (figs, checks) = rust_hot(&rw, &args);
                for (name, s) in &figs {
                    record.add("rust", &label, name, s);
                }
                if let Some(gc) = &go_checks {
                    assert_checks_equal(gc, &checks, round);
                }
                rust_checks.get_or_insert(checks);
                rw.store.close().expect("close");
                let _ = fs::remove_file(&work);
            }
            if let (Some(g), Some(r)) = (&go_export, &rust_export_lines) {
                compare_exports(g, r);
            }
        }
    }
    assert_eq!(go_pruned, rust_pruned, "PruneUnmined removed other counts");
    let _ = writeln!(notes, "pruned by PruneUnmined, per ingest: {go_pruned:?}");
    let _ = writeln!(notes, "load average at end: {}", load_average());
    let _ = writeln!(notes, "ended (unix): {}", unix_now());
    let table = table(&record);
    Some(SessionResult {
        record,
        notes,
        table,
        stats: w.stats,
        left_out: w.left_out,
    })
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn human(nanos: f64) -> String {
    if nanos >= 1e9 {
        format!("{:.2} s", nanos / 1e9)
    } else if nanos >= 1e6 {
        format!("{:.2} ms", nanos / 1e6)
    } else if nanos >= 1e3 {
        format!("{:.1} µs", nanos / 1e3)
    } else {
        format!("{nanos:.0} ns")
    }
}

/// Per figure: each side's median of its rounds' medians, their range, and
/// Rust's median over Go's.
fn table(record: &Record) -> String {
    let mut by: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
    for l in &record.lines {
        let f: Vec<&str> = l.split('\t').collect();
        let p50: f64 = f[4].parse().expect("p50");
        by.entry(f[2].to_owned())
            .or_default()
            .entry(f[0].to_owned())
            .or_default()
            .push(p50);
    }
    let mut t = String::from(
        "| figure | Go median (range) | Rust median (range) | Rust/Go |\n|---|---|---|---|\n",
    );
    for (name, sides) in &mut by {
        let mut cell = |side: &str| -> (String, f64) {
            match sides.get_mut(side) {
                Some(v) if !v.is_empty() => {
                    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = v.iter().copied().fold(0.0, f64::max);
                    let m = median(v);
                    (format!("{} ({}–{})", human(m), human(lo), human(hi)), m)
                }
                _ => ("—".to_owned(), 0.0),
            }
        };
        let (g, gm) = cell("go");
        let (r, rm) = cell("rust");
        let ratio = if gm > 0.0 {
            format!("{:.4}", rm / gm)
        } else {
            "—".to_owned()
        };
        let _ = writeln!(t, "| {name} | {g} | {r} | {ratio} |");
    }
    t
}

fn scratch(label: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-udb")
        .join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The whole pipeline at a small size, one repetition of everything: the
/// two databases' exports equal line for line, and every query's answer
/// equal in both rounds of each side.
#[test]
fn same_data_and_answers_on_both_sides() {
    let dir = scratch("measure-small");
    let reps = Reps {
        min: 1,
        max: 1,
        budget: Duration::ZERO,
    };
    let Some(result) = run_session(&dir, &Preset::small(), reps, 2) else {
        return;
    };
    eprintln!("{}", result.notes);
    // Every figure of both sides was produced.
    let names = |side: &str| -> Vec<String> {
        let mut v: Vec<String> = result
            .record
            .lines
            .iter()
            .filter(|l| l.starts_with(&format!("{side}\t")))
            .map(|l| l.split('\t').nth(2).expect("name").to_owned())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(names("go"), names("rust"));
    assert_eq!(names("go").len(), 27, "{:?}", names("go"));
    // Every timed operation of the extended workload ran at least once,
    // and as often on both sides, in every round.
    let mut samples: BTreeMap<(String, String), BTreeMap<String, String>> = BTreeMap::new();
    for l in &result.record.lines {
        let f: Vec<&str> = l.split('\t').collect();
        assert_ne!(f[3], "0", "no sample: {l}");
        samples
            .entry((f[1].to_owned(), f[2].to_owned()))
            .or_default()
            .insert(f[0].to_owned(), f[3].to_owned());
    }
    for (k, sides) in &samples {
        assert_eq!(sides.get("go"), sides.get("rust"), "samples of {k:?}");
    }
    // The workload holds every kind of operation.
    let st = &result.stats;
    for (what, n) in [
        ("tickets", st.tickets),
        ("votes", st.votes),
        ("missed votes revoked", st.revoked_missed),
        ("unmined sends", st.unmined_sends),
        ("unmined tickets", st.unmined_tickets),
        ("unmined votes", st.unmined_votes),
        ("unpublished sends", st.unpublished),
        ("unmined transactions mined", st.mined_from_unmined),
        ("double spends mined", st.conflicts),
        ("abandons", st.abandoned),
        ("transactions left unmined", st.left_unmined),
        (
            "sends a reorganisation left unmined",
            st.detached_left_unmined,
        ),
        (
            "unpublished records left unmined",
            st.unpublished_left_unmined,
        ),
        ("disapprovals", st.disapprovals),
        (
            "disapproved transactions mined again",
            st.invalidated_remined,
        ),
        ("reorganisations of one block", st.reorgs[0]),
        ("reorganisations of two blocks", st.reorgs[1]),
        ("transactions mined again", st.remined),
    ] {
        assert!(n > 0, "no {what}: {st:?}");
    }
    // The export the two sides matched covers credits, debits, spent and
    // unspent outputs, change, stake outputs, unmined outputs and
    // transactions, and every account's addresses.
    let export = fs::read_to_string(dir.join("export-go.txt")).expect("Go export");
    let count = |prefix: &str| export.lines().filter(|l| l.starts_with(prefix)).count();
    assert!(count("utxo ") > 50, "{} unspent outputs", count("utxo "));
    assert!(count("tx ") > 200, "{} transaction lines", count("tx "));
    assert_eq!(count("addrs "), 3);
    assert_eq!(count("balance "), 9);
    for needle in [" d0:", ":true:false", ":false:true", ":true:true"] {
        assert!(export.contains(needle), "no {needle:?} in the export");
    }
    let field = |prefix: &str, i: usize, v: &str| {
        export
            .lines()
            .filter(|l| l.starts_with(prefix))
            .any(|l| l.split(' ').nth(i) == Some(v))
    };
    assert!(field("utxo ", 3, "1"), "no unspent stake output");
    assert!(field("utxo ", 4, "-1"), "no unmined unspent output");
    assert!(field("tx ", 2, "-1"), "no unmined transaction in a range");
    // Each send a reorganisation's new chain left out is in the export as
    // unmined (`Rollback` returned it to the unmined transactions, with its
    // inputs' spends and its change), and nowhere as mined.
    assert_eq!(result.left_out.len() as u64, st.detached_left_unmined);
    for hash in &result.left_out {
        let h = hex(hash);
        let heights: Vec<&str> = export
            .lines()
            .filter(|l| l.starts_with("tx ") && l.split(' ').nth(1) == Some(h.as_str()))
            .filter_map(|l| l.split(' ').nth(2))
            .collect();
        assert!(
            !heights.is_empty() && heights.iter().all(|&x| x == "-1"),
            "the send {h} a reorganisation left out is at {heights:?} in the export"
        );
    }
    // Stake invalidation: disapproved blocks, with wallet credits and
    // debits moved to the invalidated tables.
    for prefix in ["invalid ", "inv_credit ", "inv_debit "] {
        assert!(count(prefix) > 0, "no {prefix:?} line in the export");
    }
    // Some balance holds an amount locked by tickets.
    assert!(
        export
            .lines()
            .filter(|l| l.starts_with("balance "))
            .any(|l| l.split(' ').nth(6) != Some("0")),
        "no locked by tickets"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The generator derives every account's addresses as the address
/// manager does, also for a seed whose coin-type private key comes out of
/// `ExtendedKey::child` with a leading zero byte stripped (about one seed
/// in 256), where deriving the accounts after the first from that key in
/// memory gives other addresses than the wallet's (`setup` then fails on
/// both sides).
#[test]
fn generator_accounts_are_the_address_managers() {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let coin = |seed: &[u8; 32]| {
        ExtendedKey::new_master(seed, &chain.hd)
            .and_then(|m| m.child(44 + HARDENED))
            .and_then(|k| k.child(chain.legacy_coin_type + HARDENED))
            .expect("coin type key")
    };
    // The first stream, from the generator's on, whose seed's coin-type
    // key is stripped.
    let seed = (0..4096u64)
        .map(|x| {
            let mut seed = [0u8; 32];
            SplitMix64(WORKLOAD_STREAM ^ x).fill(&mut seed);
            seed
        })
        .find(|seed| coin(seed).serialized_priv_key().expect("private").len() < 32)
        .expect("a seed with a stripped coin-type key");
    let accounts = 3;
    let keys = account_keys(&seed, &chain, accounts);
    // The derivation the generator used before: every account from the
    // stripped key in memory, which differs after account 0.
    let c = coin(&seed);
    for (a, k) in (0..accounts).zip(&keys) {
        let in_memory = c.child(a + HARDENED).expect("account key").neuter();
        assert_eq!(
            *in_memory.string() == *k.string(),
            a == 0,
            "account {a}: the stripped key is not what this test exercises"
        );
    }
    let mut ranges = Vec::new();
    for (account, k) in (0..accounts).zip(&keys) {
        for branch in 0..2u32 {
            let b = k.child(branch).expect("branch");
            let hashes = (0..3)
                .map(|i| stdaddr::hash160(b.child(i).expect("child").serialized_pub_key()))
                .collect();
            ranges.push(AddrRange {
                account,
                branch,
                hashes,
            });
        }
    }
    let w = Workload {
        seed,
        accounts,
        ranges,
        sync: Vec::new(),
        tip: Vec::new(),
        stats: Stats::default(),
        left_out: Vec::new(),
    };
    let dir = scratch("measure-accounts");
    let (rw, mismatches) = rust_setup(&dir.join("w.redb"), &w, &chain);
    assert_eq!(
        mismatches, 0,
        "the address manager recorded other addresses"
    );
    rw.store.close().expect("close");
    let _ = fs::remove_dir_all(&dir);
}

/// One measured session (see the module documentation).
#[test]
#[ignore = "a measurement: release build, PURSE_STORE_BENCH_DIR, pinned, minutes"]
fn hot_paths_against_dcrwallet() {
    let base = PathBuf::from(
        std::env::var_os("PURSE_STORE_BENCH_DIR")
            .expect("set PURSE_STORE_BENCH_DIR to a directory outside the repository"),
    );
    let dir = base.join(format!("session-{}", unix_now()));
    fs::create_dir_all(&dir).expect("create session directory");
    let rounds = std::env::var("PURSE_UDB_MEASURE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let reps = Reps {
        min: 5,
        max: 200,
        budget: Duration::from_secs(1),
    };
    let Some(result) = run_session(&dir, &Preset::measure(), reps, rounds) else {
        return;
    };
    let mut tsv = String::from("side\tround\tfigure\tn\tp50_ns\tp90_ns\tmin_ns\tmax_ns\n");
    for l in &result.record.lines {
        tsv.push_str(l);
        tsv.push('\n');
    }
    fs::write(dir.join("results.tsv"), tsv).expect("write results");
    fs::write(dir.join("notes.txt"), &result.notes).expect("write notes");
    fs::write(dir.join("table.md"), &result.table).expect("write table");
    for f in ["go/base.db", "rust/base.redb", "sync.wl", "tip.wl"] {
        let _ = fs::remove_file(dir.join(f));
    }
    eprintln!("{}\n{}", result.notes, result.table);
}
