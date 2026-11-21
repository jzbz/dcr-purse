# ADR-0003 — Wallet store engine and schema

- **Status:** Engine **Accepted**: redb (project owner, 2026-11-14, brief §9
  D2), major 5 (project owner, 2026-11-18; [ADR index](README.md);
  ADR-0006 rule 5 builds on it). The schema, the rules around it and the
  answers to the nine open points are **Accepted** as recommended
  (project owner, 2026-11-19); each open point is marked. Later the same
  day the owner decided main-chain headers and filters keyed by height,
  tables not merged, and the correction ADR-0018 asks for on backup and
  compaction (addendum "Decisions of the project owner", at the end).
- **Date:** 2026-11-19. Drafted before ADR-0013 and revised the same day
  after it, after the owner's statement that performance is a goal, and
  after an adversarial review of the spike; every measurement below is from
  the revised spike, run once more in one session.

## Context

**Where the store's content comes from.** dcr-purse creates its store
itself and never reads, opens, writes or imports a dcrwallet `wallet.db`;
a user moves by restoring the seed (ADR-0013 Decision items 1 and 2, which
supersede ADR-0007; brief §1, §3 W2). Every row is therefore written by
dcr-purse's own write path: chain sync, the rescan and the account and
address discovery a restore runs (brief §6 Phases 5 and 6), and the
operations clients ask for over RPC. A restored wallet's first work is
discovery plus a rescan, so history ingest in large batches and
header-plus-filter ingest are the paths a new user meets first; the
measurements below give both.

**What dropping the import frees the schema from.** The constraints
ADR-0007 placed on this record fall away (ADR-0013 Consequences, "ADRs"):
decoders for every bucket of schema version 26; states that only files
written by older dcrwallet releases hold (ADR-0007 Context); every carried
header and filter held as dcrwallet stored it; a bulk build of the derived
tables from foreign rows, with a verification report comparing them. The
store need hold only states dcr-purse itself writes, and its indexes are
maintained by its own write path from the first row on.

**What still binds it.**

- *Behaviour a client can observe.* Order that reaches a client is part of
  the schema: accounts iterate in the byte order of their little-endian
  numbers, input selection walks unspent outputs by raw transaction hash
  and big-endian index, seeks to the first key at or after a random 32-byte
  key and relies on "previous of end is last" (brief §6 Phase 2; spec 04
  §5.1, §5.19, §5.20). Byte-level row quirks are not reproduced, their
  RPC-visible consequences are (brief §6 Phase 3; ADR-0012 Rule 1).
- *Every state dcrwallet's own operations reach.* The store must represent
  what dcrwallet's store holds after the same operations, stake
  invalidation of a block and reorganisations included, because the Go and
  Rust wallets are compared by running the same operation scripts
  (ADR-0002) or restoring the same seed against the same chain (ADR-0013
  Decision item 10), and the comparison is the logical export
  (ADR-0001, in preparation), which covers the stake-invalidated buckets and
  the headers and filters of detached blocks (ADR-0001 §6.10, §6.13, §6.14).
- *Transaction boundaries.* What commits together in dcrwallet commits
  together here (brief §4 item 6): one atomic write per attached block in
  steady state, per `ChainSwitch` batch during sync (a reorganisation's
  rollback and its new blocks in one), per unmined insert.
- *The concurrency model.* A synchronous store API without a context, and
  readers that never wait for the writer (ADR-0008, "Constraints on
  others").
- *Performance is a goal* (project owner; ADR-0014, in preparation): speed
  is a design input, and never overrides correctness of keys and funds,
  constant-time arithmetic on secrets, byte-compatible RPC results, or
  being served correctly by nodes and peers. A store is invisible to the
  network, so here speed is limited only by the first three.

dcroxide's storage record bears on the engine: redb's B-tree fills pages to
0.62–0.65 and retains free pages, so a redb file runs well above its payload
(dcroxide ADR-0004, findings); commit cost is dominated by the pages a
commit touches, so keys in write order help (dcroxide ADR-0010); and redb
was kept over fjall on crash safety, not speed (dcroxide ADR-0009).

The spike behind this record, `spikes/store-redb` with its Go baseline
`tools/oracle/cmd/storebench`, asks: can a redb schema hold every state
dcrwallet's store reaches on its own operations, stake invalidation and
reorganisations included, and reproduce dcrwallet's observable behaviour
exactly; what does it gain or lose on dcrwallet's own workload, and why;
does it survive a killed writer and a power cut at any storage operation.

## What was measured, and how

**The workload.** A generator (`spikes/store-redb/src/workload.rs`) builds a
wallet history deterministically from a seed: receives (to fresh or reused
addresses), sends from one to three inputs (change to the same account, to
another account in about one send in seventeen, no change in one in
twenty-five, so transactions that only debit an account are common),
consolidations, ticket purchases (half with ticket change) and their
commitments, votes by mature tickets, and coinbases paying the wallet,
over Zipf-weighted BIP0044 accounts on simnet. Its addresses are derived
from the wallet seed with `purse-hdkeychain` at
m/44'/115'/account'/branch/index, and the Go side confirms every one:
dcrwallet's address manager, created from the same seed, records the same
HASH160 at the same account, branch and index for every address of every
size. During history sync transactions arrive with their blocks, 2,000
blocks per write transaction. Over the last 2,000 blocks each block is its
own write transaction, most non-stake transactions are first inserted
unmined (each its own write transaction) and mined one to three blocks
later, and every eight blocks one chain event runs, in rotation:

- a block that disapproves its parent (vote bits without `BlockValid`),
  so that dcrwallet's `ExtendMainChain` stake-invalidates the parent's
  regular tree (wallet/udb/txmined.go:234-241, `stakeInvalidate` at :767);
  each of the parent's regular transactions is then mined again in the same
  block, mined again one or two blocks later, or never mined again;
- a disapproval as above, all of the parent's regular transactions mined
  again in the disapproving block, followed by a one-block reorganisation
  to a block that approves the parent, so that `Rollback`
  (txmined.go:1874) detaches the disapproving block and `stakeValidate`
  (txmined.go:631) revalidates the parent;
- reorganisations of one, two and three blocks, written as `ChainSwitch`
  writes them: `Rollback`, then the new chain's blocks, in one write
  transaction (wallet/chainntfns.go:91, :160); the new blocks carry the old
  ones' transactions except their coinbases, and sometimes leave the last
  block's regular transactions unmined until the next block.

A block a scenario acts on always carries a receive and a send, so its
disapproval moves credits and debits. The last blocks carry coinbases at
depths 1, 15, 16 and 17 at the final tip, votes at depths 1 and 17 and
ticket change at depths 1 and 2, so the exported balances cross every
maturity boundary (simnet coinbase maturity 16, ticket change maturity 1).
A final 30, 60 or 200 transactions stay unmined, a third of the sends among
them unpublished.

| preset | blocks attached | wallet transactions | votes / tickets | disapprovals / reorganisations | unmined first | accounts | addresses | unspent outputs at the end |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 10k | 50,280 | 10,437 | 859 / 861 | 141 / 175 | 74 | 5 | 8,898 | 3,182 |
| 100k | 100,280 | 100,140 | 8,399 / 8,401 | 141 / 175 | 697 | 8 | 83,673 | 29,590 |
| 1m | 200,280 | 1,000,204 | 85,654 / 85,660 | 141 / 175 | 4,443 | 10 | 832,468 | 297,896 |

"Blocks attached" counts the blocks of the new chains that replace
rolled-back ones; "unspent outputs" are `UnspentOutputs` entries at the end.

**The baseline.** `storebench` links dcrwallet's `wallet/udb` at the parity
pin over bbolt v1.3.12, opened as dcrwallet opens it (`bolt.Open(path,
0600, nil)`: synchronous commits). It creates the wallet with
`udb.Initialize`, creates the accounts with `Manager.NewAccount` and records
the addresses with `SyncAccountToAddrIndex`, then ingests with the call
sequence of `wallet.extendMainChain`, `processTransactionRecord` and
`ChainSwitch` (wallet/chainntfns.go:29, :438, :91): `Rollback` for a
reorganisation; `ExtendMainChain` with the empty version-2 filter,
`GetBlockMetaForHash`, then per transaction `NewTxRecordFromMsgTx`,
`InsertMinedTx` or `InsertMemPoolTx`, and per output `ExtractAddrs`,
`StakeSubScriptType`, the commitment decode for ticket outputs,
`Manager.Address`, `AddCredit` or `AddTicketCommitment` and
`markUsedAddress` (`MarkUsed`, `AccountProperties`,
`SyncAccountToAddrIndex` to the last used child plus a gap of 20); votes then
`RedeemTicketCommitments`; each write transaction ends with
`UpdateProcessedTxsBlockMarker`. Left out, because they do not touch the
database or the workload never reaches them: checkpoints, notifications,
outpoint locks and the multisig paths.

udb validates nothing a synthetic history could fail: no signatures, no
proof of work, no merkle roots. `ExtendMainChain` checks only parent
linkage, height and the have-cfilters flag; `InsertMinedTx` needs the header
recorded; transaction types come from `stake.DetermineTxType`, which is
structural, so the generator writes structurally valid tickets and votes
(commitment and vote-reference scripts of the exact sizes, the stakebase
fraud-proof fields) and P2PKH-shaped signature scripts of random bytes.

**The prototype.** The same operations, in the same order and the same
transaction boundaries, through the schema below (`src/store.rs`), in a
release build with redb 5.0.0 at its defaults (`Durability::Immediate`,
one-phase commit with checksums, 1 GiB cache). Every query was repeated at
least five times and until a second had passed (at most 200 times); input
selection ran 50 times per target, rotating three accounts; the tables give
medians. A cold open first dropped the file from the page cache
(`dd iflag=nocache`); the warm open followed it. Bytes written are each
process's `write_bytes` from `/proc/<pid>/io` across the ingest: the bytes
it dirtied in the page cache, before the file system's copy-on-write and
compression, which change what reaches the device.

**One session, alternating.** Both stores were created and their addresses
recorded once, and a copy of each kept; the ingest then ran from those
copies twice on each side, alternating (dcrwallet, prototype, dcrwallet,
prototype), and the tables give both rounds of the whole-workload figure
and the median round's details. Queries ran on each side in turn, never at
the same time. The earlier draft of this record divided Go ingest figures
from one session by prototype figures from another; none of the figures
below are paired that way.

**The machine.** An x86-64 desktop (16-core AMD Zen 4, 93 GiB RAM) with an
NVMe drive under btrfs with zstd compression on dm-crypt, Linux, Go 1.27.4,
Rust 1.100.0. A synchronous 4 KiB write takes milliseconds there, so every
durable commit costs milliseconds on either side and dominates single-block
and single-insert figures, which drifted with the shared machine's load:
dcrwallet's next-address figure was 10.7 ms in one session of the first
draft and 20 ms in another. Ratios of single commits are within that noise;
ratios of several-fold are not. Raw output, both databases' exports and
logs are kept outside the repository; `spikes/store-redb/README.md` says
how to rerun everything.

## Results

"dcrwallet ÷ prototype" above 1 means the prototype is faster or smaller.

### 10k

| measurement | dcrwallet, bbolt | prototype, redb | dcrwallet ÷ prototype |
|---|---:|---:|---:|
| ingest, history sync, per block (2,000-block transactions) | 77.7 µs | 19.4 µs | 4.0x |
| ingest, block at the tip, one transaction, p50 | 17.93 ms | 9.60 ms | 1.9x |
| ingest, block at the tip, p99 | 277.40 ms | 147.84 ms | 1.9x |
| ingest, unmined insert, p50 | 11.33 ms | 9.49 ms | 1.2x |
| ingest, reorganisation (rollback, new blocks), p50 | 19.10 ms | 10.62 ms | 1.8x |
| ingest, whole workload (rounds 1, 2; ratio per round) | 57.78 s, 46.80 s | 35.89 s, 22.52 s | 1.6x, 2.1x |
| ingest, bytes written per write transaction | 611 KiB | 480 KiB | 1.3x |
| ingest, bytes written, whole workload | 1251.9 MiB | 984.4 MiB | 1.3x |
| balance, one account, minconf 1 (median over 3 accounts) | 5.83 ms | 1.5 µs | 3835x |
| balance, all accounts, minconf 0 | 5.11 ms | 6.6 µs | 778x |
| balance, all accounts, minconf 1 | 6.16 ms | 7.1 µs | 866x |
| balance, all accounts, minconf 6 | 5.57 ms | 7.0 µs | 798x |
| balance, all accounts, minconf 100 | 5.15 ms | 7.7 µs | 667x |
| balance, all accounts, minconf 25,000 | 5.16 ms | 89.1 µs | 57.8x |
| balance, all accounts, minconf 100,000 | 5.39 ms | 6.1 µs | 877x |
| UnspentOutputs walk | 4.66 ms | 1.09 ms | 4.3x |
| ForEachUnspentOutpoint walk | 2.04 ms | 267.8 µs | 7.6x |
| input selection, 0.01 DCR | 19.5 µs | 4.9 µs | 4.0x |
| input selection, 1 DCR | 30.1 µs | 6.2 µs | 4.8x |
| input selection, 100 DCR | 96.4 µs | 20.7 µs | 4.7x |
| range: 100 most recent transactions | 231.2 µs | 80.9 µs | 2.9x |
| range: last 1,000 blocks, backward | 908.3 µs | 328.2 µs | 2.8x |
| range: 1,000 blocks mid-chain, forward | 538.0 µs | 167.6 µs | 3.2x |
| range: unmined only | 368.8 µs | 105.4 µs | 3.5x |
| range: whole history | 35.93 ms | 11.28 ms | 3.2x |
| 100 most recent of account 0 | 1.11 ms | 103.8 µs | 10.6x |
| 100 most recent of the last account | 7.31 ms | 108.5 µs | 67.4x |
| 1,000 address lookups by script (one read transaction) | 38.80 ms | 92.0 µs | 422x |
| next address: record one child (one transaction) | 10.54 ms | 9.82 ms | 1.1x |
| open, cold | 1.56 ms | 9.26 ms | 0.2x |
| first balance query after open, cold | 206.13 ms | 1.50 ms | 138x |
| open, warm | 712.3 µs | 9.92 ms | 0.1x |
| first balance query after open, warm | 7.83 ms | 38.0 µs | 206x |
| hot backup to a new file, no other writer | 98.84 ms | 293.43 ms | 0.3x |
| hot backup with a concurrent writer (its commits meanwhile) | 42.57 ms (6) | 289.19 ms (39) | |
| file size after ingest | 65.9 MiB | 34.0 MiB | 1.9x |
| file size after the queries | 65.9 MiB | 34.0 MiB | 1.9x |
| redb file compacted (copy) | | 29.1 MiB | |
| redb stored / fragmented bytes, tree height | | 21.2 MiB / 10.3 MiB, 4 | |
| consistency check, whole store | | 90.86 ms | |


### 100k

| measurement | dcrwallet, bbolt | prototype, redb | dcrwallet ÷ prototype |
|---|---:|---:|---:|
| ingest, history sync, per block (2,000-block transactions) | 438.5 µs | 122.9 µs | 3.6x |
| ingest, block at the tip, one transaction, p50 | 12.90 ms | 9.69 ms | 1.3x |
| ingest, block at the tip, p99 | 96.56 ms | 63.67 ms | 1.5x |
| ingest, unmined insert, p50 | 12.40 ms | 7.90 ms | 1.6x |
| ingest, reorganisation (rollback, new blocks), p50 | 15.60 ms | 11.52 ms | 1.4x |
| ingest, whole workload (rounds 1, 2; ratio per round) | 94.34 s, 98.30 s | 46.15 s, 39.08 s | 2.0x, 2.5x |
| ingest, bytes written per write transaction | 1201 KiB | 896 KiB | 1.3x |
| ingest, bytes written, whole workload | 3222.0 MiB | 2402.0 MiB | 1.3x |
| balance, one account, minconf 1 (median over 3 accounts) | 70.29 ms | 2.6 µs | 27244x |
| balance, all accounts, minconf 0 | 73.84 ms | 20.2 µs | 3663x |
| balance, all accounts, minconf 1 | 68.21 ms | 18.3 µs | 3721x |
| balance, all accounts, minconf 6 | 79.44 ms | 28.1 µs | 2832x |
| balance, all accounts, minconf 100 | 77.69 ms | 18.8 µs | 4143x |
| balance, all accounts, minconf 50,000 | 68.85 ms | 799.7 µs | 86.1x |
| balance, all accounts, minconf 100,000 | 80.76 ms | 11.8 µs | 6832x |
| UnspentOutputs walk | 74.34 ms | 13.84 ms | 5.4x |
| ForEachUnspentOutpoint walk | 25.68 ms | 2.58 ms | 9.9x |
| input selection, 0.01 DCR | 79.0 µs | 9.1 µs | 8.7x |
| input selection, 1 DCR | 91.9 µs | 12.1 µs | 7.6x |
| input selection, 100 DCR | 254.2 µs | 32.0 µs | 8.0x |
| range: 100 most recent transactions | 273.1 µs | 96.5 µs | 2.8x |
| range: last 1,000 blocks, backward | 2.01 ms | 887.0 µs | 2.3x |
| range: 1,000 blocks mid-chain, forward | 4.20 ms | 2.03 ms | 2.1x |
| range: unmined only | 508.6 µs | 236.3 µs | 2.2x |
| range: whole history | 698.99 ms | 240.59 ms | 2.9x |
| 100 most recent of account 0 | 2.80 ms | 120.6 µs | 23.2x |
| 100 most recent of the last account | 29.94 ms | 130.7 µs | 229x |
| 1,000 address lookups by script (one read transaction) | 46.76 ms | 154.8 µs | 302x |
| next address: record one child (one transaction) | 10.78 ms | 16.70 ms | 0.6x |
| open, cold | 2.78 ms | 15.90 ms | 0.2x |
| first balance query after open, cold | 2.28 s | 6.87 ms | 333x |
| open, warm | 967.0 µs | 16.42 ms | 0.1x |
| first balance query after open, warm | 89.43 ms | 65.8 µs | 1360x |
| hot backup to a new file, no other writer | 362.32 ms | 2.57 s | 0.1x |
| hot backup with a concurrent writer (its commits meanwhile) | 192.84 ms (15) | 1.10 s (179) | |
| file size after ingest | 250.5 MiB | 266.0 MiB | 0.9x |
| file size after the queries | 250.5 MiB | 266.0 MiB | 0.9x |
| redb file compacted (copy) | | 146.9 MiB | |
| redb stored / fragmented bytes, tree height | | 97.4 MiB / 158.0 MiB, 4 | |
| consistency check, whole store | | 600.18 ms | |


### 1m

| measurement | dcrwallet, bbolt | prototype, redb | dcrwallet ÷ prototype |
|---|---:|---:|---:|
| ingest, history sync, per block (2,000-block transactions) | 1.70 ms | 508.5 µs | 3.3x |
| ingest, block at the tip, one transaction, p50 | 39.15 ms | 19.64 ms | 2.0x |
| ingest, block at the tip, p99 | 143.94 ms | 78.76 ms | 1.8x |
| ingest, unmined insert, p50 | 37.36 ms | 18.31 ms | 2.0x |
| ingest, reorganisation (rollback, new blocks), p50 | 40.70 ms | 20.30 ms | 2.0x |
| ingest, whole workload (rounds 1, 2; ratio per round) | 634.60 s, 439.89 s | 254.65 s, 222.98 s | 2.5x, 2.0x |
| ingest, bytes written per write transaction | 3411 KiB | 2572 KiB | 1.3x |
| ingest, bytes written, whole workload | 21792.5 MiB | 16434.7 MiB | 1.3x |
| balance, one account, minconf 1 (median over 3 accounts) | 758.12 ms | 1.6 µs | 465102x |
| balance, all accounts, minconf 0 | 757.46 ms | 13.1 µs | 57646x |
| balance, all accounts, minconf 1 | 763.28 ms | 15.2 µs | 50249x |
| balance, all accounts, minconf 6 | 755.09 ms | 15.1 µs | 50105x |
| balance, all accounts, minconf 100 | 754.41 ms | 19.1 µs | 39602x |
| balance, all accounts, minconf 100,000 | 755.47 ms | 6.48 ms | 117x |
| UnspentOutputs walk | 668.76 ms | 184.09 ms | 3.6x |
| ForEachUnspentOutpoint walk | 284.92 ms | 29.19 ms | 9.8x |
| input selection, 0.01 DCR | 690.8 µs | 9.8 µs | 70.6x |
| input selection, 1 DCR | 700.5 µs | 15.1 µs | 46.5x |
| input selection, 100 DCR | 1.18 ms | 42.5 µs | 27.8x |
| range: 100 most recent transactions | 347.0 µs | 125.7 µs | 2.8x |
| range: last 1,000 blocks, backward | 15.83 ms | 6.50 ms | 2.4x |
| range: 1,000 blocks mid-chain, forward | 28.02 ms | 14.79 ms | 1.9x |
| range: unmined only | 1.41 ms | 607.0 µs | 2.3x |
| range: whole history | 6.82 s | 3.37 s | 2.0x |
| 100 most recent of account 0 | 2.52 ms | 149.1 µs | 16.9x |
| 100 most recent of the last account | 26.45 ms | 152.2 µs | 174x |
| 1,000 address lookups by script (one read transaction) | 39.26 ms | 209.1 µs | 188x |
| next address: record one child (one transaction) | 11.12 ms | 5.47 ms | 2.0x |
| open, cold | 10.76 ms | 6.33 ms | 1.7x |
| first balance query after open, cold | 19.36 s | 1.59 ms | 12180x |
| open, warm | 1.47 ms | 5.28 ms | 0.3x |
| first balance query after open, warm | 811.57 ms | 105.2 µs | 7715x |
| hot backup to a new file, no other writer | 2.05 s | 6.40 s | 0.3x |
| hot backup with a concurrent writer (its commits meanwhile) | 1.18 s (59) | 6.24 s (1380) | |
| file size after ingest | 1722.6 MiB | 2371.6 MiB | 0.7x |
| file size after the queries | 1722.6 MiB | 2371.6 MiB | 0.7x |
| redb file compacted (copy) | | 1195.5 MiB | |
| redb stored / fragmented bytes, tree height | | 749.0 MiB / 1544.2 MiB, 5 | |
| consistency check, whole store | | 7.40 s | |

The deep balance rows are there for the cost model, not because a client
asks for them: minconf 25,000, 50,000 and 100,000 are half the tip at the
three sizes, the depth at which the query reads the most per-height rows
(Decision item 4).

## Correctness

**Cross-check.** After ingest both sides write a canonical export
(`src/export.rs`; Go's `export` command) and the two files are compared line
by line. It covers:

- the tip, the unspent key count (bbolt's `KeyN`), every account's
  counters in iteration order;
- `AccountBalances` at minconf 0, 1, 2, 6, 15, 16, 17, 100 and 100,000 in
  every category, with immature coinbase, immature stake generation and
  ticket change on both sides of their maturity at the final tip (the
  quick test asserts that immature coinbase and stake are non-zero);
- `UnspentOutputs` in order with height, amount, tree, coinbase flag and
  script, and `ForEachUnspentOutpoint` in order with its trees;
- bbolt cursor probes on the unspent index (seek, three steps one way, two
  back) from random keys, existing keys, their neighbours, the empty key,
  the extremes and the first and last three keys, so that steps run off
  each end at every count; and 122 scans of `randomUTXO`'s collection step
  in both directions with `RandomUTXO`'s skip;
- `MakeInputSource` for every account at minconf 0, 1 and 6, with target 0
  (the eligible list without the random phase) and with a target no balance
  reaches (every eligible output whatever the random draws): count, total
  and a digest of the sorted inputs, and at the two smaller sizes every
  input;
- `RangeTransactions` over every argument case of spec 04 §5.16 (and the
  whole history both ways at the two smaller sizes) with each transaction's
  credits and debits, a disapproved block's records listed with neither;
- each account's history (credits and debits), whole at the two smaller
  sizes and its 500 most recent transactions at 1m;
- the blocks marked invalid and the invalidated credits and debits of
  main-chain blocks;
- each account's addresses in `ForEachAccountAddress` order (a digest).

The files were identical at every size: 66,144 lines at 10k, 564,741 at 100k
and 602,625 at 1m (the 1m export leaves out the two whole-history ranges and
the input lists; it lists all 297,896 unspent outputs and outpoints), each
with 71 blocks marked invalid and between 108 and 305 invalidated credits
and debits.

**What the cross-check does not settle.** dcrwallet draws selection's
randomness from `crypto/rand` (txmined.go:2362, :3169), which a caller
cannot inject, so what is compared is the set of outputs selection may
take, not which of them a given random draw takes; the prototype consumes
randomness in `MakeInputSource`'s order, which the compared cursor probes
and scans support but do not prove.

**Mutations.** Each of these defects, introduced one at a time into a copy
of the prototype, was run through the consistency check and the
cross-check on the tiny and the 10k workloads (the first draft's
cross-check missed the first five at both sizes and caught the next two only
on the tiny one):

| defect | tiny | 10k |
|---|---|---|
| selection ignores the account | export (`select`) | export |
| selection ignores ticket-change maturity | export | export |
| selection takes outputs spent by unmined transactions | export | export |
| `ForEachUnspentOutpoint` takes outputs spent by unmined transactions | export (`fe`) | export |
| per-account history loses the debit's row | consistency check | consistency check |
| cursor: `prev` after `next` ran off the end | export (`probe`) | export |
| ticket change counted with coinbase maturity | export (`balance`) | export |
| immature-coinbase window one block short | **missed** | export |
| stake invalidation keeps the unspent output | consistency check | consistency check |
| stake validation keeps the spent output | consistency check | consistency check |
| rollback records no unmined inputs | **missed** | export |
| `UnspentOutputs` ignores the unmined twin of an output | export (`utxo`) | export |
| history reads a transaction's latest record, not its block's | export (`tx`) | export |
| rollback leaves a vote's commitments without the unmined-spent mark | **missed** | **missed** |

The misses are workload gaps, not blind spots of the comparison: the tiny
workload's depth-16 coinbase is spent by one of the transactions seen after
the last block, and its rolled-back transactions are all mined again within
a block, so neither state survives to the export; and no workload leaves a
rolled-back vote unmined, since every new chain mines its votes again. A
Phase 2 workload should reach both.

**Consistency check.** `R::check_balances` re-derives every denormalised
value from what it copies: each credit, invalidated credit and unmined
credit against its transaction's output and the address it pays (amount,
account, change bit, stake tag, coinbase bit, script location and kind);
each `unspent` value against its credit, and its balance class against the
class the output implies; each credit's spent state against the debit that
spends it and each debit against its transaction's input; the invalidated
rows against their block's flag; `history`, `acct_history` and the blocks'
transaction counts against the records; the commitments against their
tickets; the address and block tables against each other; and the sums and
the unspent count against a recomputation from all of those. The first
draft's check compared the sums with copies of the class field, so a wrong
class passed it; the quick test now changes one copied field at a time (a
class, an account, an amount, a script offset, a block's count, a history
row) and requires each to be reported. It holds after every write
transaction of the tiny workload, after ingest at every size (91 ms at 10k,
0.60 s at 100k, 7.4 s at 1m), and after every crash and power cut below.

**Crash safety: killed writers.** `spike crash` ingests the tiny preset (450
operations in 239 write transactions, among them 6 disapprovals and 6
reorganisations) into a reference store, recording a digest of every table
at every transaction boundary, then spawns a writer on a victim store and
kills it with SIGKILL, timed from the spawn for one kill in four and from
the writer's report that the store is open for the rest; it reopens and
requires a transaction boundary, every reported commit, equality with the
reference there, and the consistency check. In 300 kills on redb 5.0.0, over
five passes through the workload, 10 landed before the store was open
(process start and redb's open, which writes and fsyncs its header), 56
inside the first write transaction and 234 after at least one reported
commit; of those 234 the reopened store sat at the last reported commit 15
times and past it 219 times, the commit's writes having reached the page
cache before the writer printed. Reopening took 26 ms at the median, 273 ms
at the 99th percentile and 1.09 s at most. No reopen failed or lost a
reported commit, every digest matched and every consistency check held. The
first draft's digest skipped five of its tables and its count of 300
included kills that never reached the store; both are corrected. The quick
test repeats a 12-kill run. A kill leaves the page cache intact, so it tests
recovery on open and atomicity as seen through the page cache, and can
neither tear nor reorder a write: it could not have caught the repair fault
redb 4.6.0 fixed ("repair falling back to a commit slot that failed its
checksum", redb `CHANGELOG.md:92`).

**Crash safety: power cuts.** dcroxide's `PowerLossBackend` implemented redb
4's `StorageBackend` at the revision the spike was written against
(`5c6742f`), whose methods return `std::io::Error`; redb 5's return
`BackendError` (redb 5.0.0 `src/db.rs:72-90`), so it did not build against
redb 5, and linking `dcroxide-testutil` then would also have put redb 4
back in the graph. (dcroxide `f4b3d2e` moved it to redb 5 and the pin
moved there on 2026-11-19; the store's port is in `purse-store`'s tests,
see the addendum of that date.) The spike carries a port (`src/powerloss.rs`) over an
in-memory image, which counts every write, length change and sync. `spike
powerloss` lets the tiny workload run to each storage operation in turn,
cuts the power there, discarding every write since the last sync or, torn,
a random half of them in 4 KiB pieces (within redb's stated assumptions
about the media: atomic single-byte writes and no change outside a write's
range, redb `docs/design.md`, "Assumptions about underlying media"),
reopens the image and applies the checks above against the last durable
commit. The tiny workload makes 5,997 storage operations,
247 of them syncs. A cut at every one of them, untorn and torn (5,998 runs
each), always reopened at the last durable commit, with every commit
acknowledged before the cut present, the digest equal to the reference and
the consistency check holding; no reopen failed. With `Durability::None` on
all but every tenth transaction (2,572 operations, 32 syncs), every reopen
sat at the last durable commit, 2,556 of the 2,573 runs lost commits that
had returned without durability, as redb documents (`docs/design.md`,
"Non-durable commits"), and none lost a durable one. Against a backend that
acknowledges syncs without making anything durable, 850 of 857 cuts lost a
durable commit: the sweep can tell. This is redb 5.0.0's behaviour on one
small workload, in-process; it does not cover file systems or devices that
lie about a sync.

## Performance: where the prototype wins, where it loses, and why

**Robust wins** (several-fold at every size, CPU-bound, so not within the
session noise):

- **Balances: 670x to 465,000x at the depths the wallet asks for.** One
  account at minconf 1: 5.8 ms, 70 ms and 758 ms against 1.5, 2.6 and
  1.6 µs. dcrwallet walks the whole unspent index for every call, and for
  each entry reads the credit, fetches the script from the transaction
  record and resolves the account; the prototype reads one account's
  running sums and, for the depth-dependent categories, the per-height rows
  on the shorter side of the depth window. That makes the cost depend on
  the depth asked: it peaks at half the tip, 89 µs, 0.80 ms and 6.5 ms for
  all accounts, still 58x to 117x. The first draft measured only minconf 0,
  1 and 6.
- **Cold start: 138x to 12,000x.** dcrwallet's first balance query after
  opening reads the credit and the transaction record of every unspent
  output from a cold file: 19.4 s at 1m. The prototype's touches a few
  pages (1.6 ms).
- **Address lookups by script: 190x to 420x, mostly not the storage.**
  `Manager.Address` derives the child public key again for every hit; the
  prototype returns account, branch and index from one index row (Open
  point 7).
- **Input selection: 4x to 71x**, growing with size: `MakeInputSource`
  counts the unspent bucket with `KeyN`, a full walk, every time it is
  created; the prototype keeps the count in `meta`, and each candidate
  costs one read instead of three to four. (`purse-store` reads redb's
  table length instead, kept in the table's root at no cost, and has no
  counter row; addendum of 2026-11-19, "Schema version 1 as built".) The eligible set is now
  cross-checked (Correctness).
- **History by account: 11x to 230x**; dcrwallet has no such path, and a
  client walks history backward resolving every credit and debited output
  to an account, so the cost grows with how rare the account is.
- **Unspent walks and ranges: 3.6x to 10x and 1.9x to 3.5x**, from one read
  per output and a history index that skips empty blocks; the transaction
  decode, which both sides do, bounds the ranges.
- **Batched ingest: 3.3x to 4.0x per block** (78 µs, 439 µs and 1.70 ms
  against 19, 123 and 509 µs) and 1.6x to 2.5x for the whole workload in
  each round; header-and-filter ingest 2.3x to 3.0x (below). One part is
  measured: dcrwallet's `markUsedAddress` derives a key on every credit to
  check the gap, which `addr_by_path` answers with a lookup; the rest was
  not profiled on either side.

**Single commits** (tip block, unmined insert, reorganisation, next address)
are fsync-bound on either side: bbolt syncs twice per commit (pages, then
the meta page) and redb's one-phase commit with checksums once (redb
`docs/design.md`, "1-phase + checksum durable commits"). Here the tip block,
the unmined insert and the reorganisation (a rollback and its new blocks in
one transaction) took 1.2x to 2.0x less time at the median (tip block: 17.9
against 9.6 ms, 12.9 against 9.7, 39.2 against 19.6), and next address 1.1x,
0.6x and 2.0x. The same commit drifts between 5 and 20 ms from hour to hour
on this machine, so these ratios are not robust; the halved number of syncs
is.

**Where the prototype is slower, why, and whether it matters.**

- **Opening for writing: 5 to 16 ms against 0.7 to 2.8 ms** (warm at every
  size, cold at the two smaller ones; cold at 1m bbolt's open was the
  slower, 10.8 against 6.3 ms). redb's open writes its header, marking the
  file as needing recovery until a clean close, and syncs it; bbolt only
  reads its meta pages. It is paid once per process start, beside a start-up
  that derives account keys and, on unlock, runs scrypt (brief §6 Phase 13).
  It does not matter, and cannot be avoided for a writer within redb; a tool
  that only reads can open with redb's read-only database type instead.
- **Hot backup: 4x to 8x slower** (medians of three repeated runs per
  size: 5.8x, 4.1x, 8.0x): 242 to
  250 ms against 42 to 43 ms at 10k, 1.26 to 2.13 s against 0.15 to 0.37 s
  at 100k, 8.5 to 9.2 s against 0.97 to 1.13 s at 1m (single runs are in the
  tables). bbolt's `Tx.Copy` streams the file's pages; redb has no
  page-level copy, so the backup re-inserts every row of the snapshot into a
  new file and commits it once, writing the compacted size (1.2 GiB at 1m)
  and building every tree again. It blocks nobody: during the 1m copy a
  concurrent writer committed 610 to 746 times on the prototype, 78 to 93 on
  bbolt (whose copy was shorter). For a backup that runs in the background
  this matters little; a faster copy (several destination transactions under
  one read snapshot, or larger batches of sorted inserts) was not measured.
- **Recovery after a crash: 1.33 s** to reopen the 1m store after a killed
  writer, against 9 to 23 ms for a clean open of a warm file (388 ms
  once, after other runs had evicted it): redb repairs on open by
  walking the whole file to rebuild its allocator state. Quick repair (redb
  `set_quick_repair`, `CHANGELOG.md:576-577`) saves that state in every
  commit; on the 1m store it brought the reopen after a kill down to 10 ms
  and raised a one-address commit's median from 5.3 to 5.5 ms to 11.0 to
  20.3 ms. A crash costs its recovery once; quick repair costs every write
  (Open point 3).
- **File size at 1m: 1.38x bbolt's** (2,372 against 1,723 MiB); the
  prototype is half bbolt's size at 10k and 1.06x at 100k. Of the 1m file,
  redb counts 749 MiB stored and 1,544 MiB fragmented, and a compacted copy
  is 1,196 MiB: redb 5 gives back free space at the end of the file, but
  keeps free pages inside it for reuse, the amplification dcroxide ADR-0004
  recorded.

**What the schema costs.** Each write transaction rewrites, for every
table it changes, the pages from that table's root to each changed leaf
(both engines are copy-on-write), plus the engine's own bookkeeping. Across
the ingest the prototype dirtied 480 KiB per write transaction at 10k,
896 KiB at 100k and 2.5 MiB at 1m, against 611 KiB, 1.2 MiB and 3.3 MiB for
bbolt: 1.3x less, but a hundred times and more the few kilobytes a block
with one or two wallet transactions adds, and growing with the trees'
height. A block commit with a send changes about fifteen tables; five of
them (`balances`, `balance_heights`, `history`, `acct_history` and, when
addresses are recorded, `addr_by_acct_sha`) are this schema's derived
tables, which buy the read-side wins above. The breakdown per table was not
measured; fewer tables per commit is the lever (Open point 9). Space: see
the file sizes above.

**Restore and rescan.** A restore is account and address discovery plus a
rescan (ADR-0013 Decision item 6). The rescan's writes are the batched
history-sync path above: once the headers are present, each batch of blocks
with relevant transactions is one write transaction. On these workloads that
part of ingest (48,000, 98,000 and 198,000 blocks in batches of 2,000) took
3.7 s, 43 s and 337 s on dcrwallet against 0.9 s, 12 s and 101 s on the
prototype, every commit durable. Discovery is mostly key derivation and
requests to the node or peers, and was not measured. Before that, an SPV
restore downloads and stores every header and filter, measured next.

**Header and filter ingest (SPV initial sync).** 1,050,000 generated headers
(mainnet had passed height 1,030,629 when chaincfg v3.3.0, the release
dcrwallet's pin uses, was cut: `mainnetparams.go:145`), each with a
generated committed filter of 270 bytes on average (an assumption: mainnet's
average filter size was not measured here), 2,000 per write transaction, the
most one `headers` message carries (`MaxBlockHeadersPerMsg`, wire v1.8.0
`msgheaders.go:15`), through `ExtendMainChain` on both sides, twice each,
header and filter generation outside the timing:

| | dcrwallet, bbolt | prototype, every commit durable | prototype, every 50th commit durable |
|---|---:|---:|---:|
| whole ingest, runs 1 and 2 | 126.0 s, 113.5 s | 42.3 s, 49.1 s | 11.0 s, 10.0 s |
| one 2,000-header transaction, p50 | 157 ms, 142 ms | 49 ms, 59 ms | 10.9 ms, 10.8 ms |
| bytes written | 13.6 GiB, 14.3 GiB | 4.9 GiB, 5.2 GiB | 1.1 GiB, 1.1 GiB |
| file size | 1,020 MiB | 629 MiB | 1,079 MiB |

With every commit durable the prototype is 2.3x to 3.0x faster; with only
every 50th batch and the last durable (`Durability::None` otherwise), a
further 3.8x to 4.9x, and the file grows to 1,079 MiB, because a
non-durable commit cannot free pages the last durable commit may still use
(redb `docs/design.md`, "Non-durable commits"). What a crash then costs:
the store stays consistent and reopens at the last durable commit (the
power-loss sweep's relaxed mode, above), and up to 49 batches, 98,000
headers and their filters, are fetched and written again, about a second
at the relaxed rate. Brief §4 item 6 asks that every write be crash-safe:
each commit stays atomic and the store never stands between transactions;
what is given up is the durability of the last batches, which nothing
outside the process depends on if Decision item 9's rule is kept. Whether
to allow it is Open point 4.

## Decision (accepted: item 1 on 2026-11-14 and 2026-11-18, the rest on 2026-11-19)

1. **Engine: redb 5** (accepted). Pinned to one exact release and moved in
   step with dcroxide, per ADR-0006 rule 5. `Durability::Immediate` for
   every logical operation and redb's default one-phase commit with
   checksums, unless the owner accepts item 9's exception.
2. **Schema.** Every table maps raw bytes to raw bytes, so key order is byte
   order, as in bbolt; integers in keys are big-endian except where a client
   observes little-endian order. "Outpoint" is the 32-byte transaction hash
   in internal order followed by the output index, big-endian (dcrwallet's
   canonical outpoint); "credit key" is hash ‖ height ‖ index.

   | table | key | value | dcrwallet access path it serves |
   |---|---|---|---|
   | `meta` | name | schema version; tip (height, hash); unspent count; processed-transactions marker *(as built: no unspent count, which is the length of `unspent`; birth state and have-cfilters added; addendum of 2026-11-19)* | `MainChainTip`, `KeyN` of `u`, `ProcessedTxsBlockMarker` |
   | `wallet` *(added, addendum of 2026-11-19)* | name | master-key parameters, sealed crypto keys, coin-type keys, slip0044 account 0, watch-only, last account, last imported account | the address manager's wallet-wide rows (`waddrmgr/main`, `waddrmgr/meta`) |
   | `accounts` | account u32 **LE** | last used and last returned child per branch; name | account iteration in dcrwallet's order, `AccountProperties`, `MarkUsedChildIndex` |
   | `account_names` | name | account | `LookupAccount` |
   | `addr_by_hash` | HASH160 | account, branch, index | `Manager.Address`, `AddrAccount` by script; ownership on ingest |
   | `addr_by_path` | account BE, branch, index BE | HASH160 | `SyncAccountToAddrIndex` without derivation; a BIP0044 account's addresses in derivation order |
   | `addr_by_acct_sha` | account **LE**, SHA-256(HASH160) | HASH160 | `ForEachAccountAddress` order (`addracctidx`) |
   | `headers` | height (spike); block hash (Phase 2) *(as built: height, the main chain's headers only; addendum "Chain tables keyed by height, as built")* | 180-byte header | `GetBlockHeader` |
   | `blocks` | height | hash, time, vote bits, invalid flag, wallet transactions | `RangeBlocks`, `GetMainChainBlockHashForHeight`, `BlockInMainChain` |
   | `block_by_hash` | hash | height | `GetBlockMetaForHash` |
   | `cfilters` | height (spike); block hash (Phase 2) *(as built: height, the main chain's filters only; same addendum)* | key (16), filter | `GetMainChainCFilters`, `CFilterV2` |
   | `headers_detached` *(added, same addendum)* | block hash | 180-byte header | `GetBlockHeader` of a block not in the main chain (Open point 5) |
   | `cfilters_detached` *(added, same addendum)* | block hash | key (16), filter | `CFilterV2` of a block not in the main chain |
   | `tx_mined` | tx hash, height BE | position in block, received time, transaction | `TxDetails`, `Tx`, `ExistsTx` (highest height of a hash) |
   | `tx_unmined` | tx hash | received time, transaction | `UnminedTxs`, the unmined batch of `RangeTransactions` |
   | `unpublished` | tx hash | flag | `Unpublished`, `SetPublished` |
   | `history` | height BE, position BE | tx hash | `RangeTransactions` over wallet transactions only; the record is `tx_mined` at (hash, height) |
   | `acct_history` | account BE, height BE, position BE | tx hash | history by account (new): records with a live credit to, or debit from, the account |
   | `credits` | credit key | amount, flags, account, script offset and length, script kind, spender | credits of `TxDetails`, `Spender` (`c`) |
   | `credits_invalidated` | credit key | as `credits` | `ic` |
   | `unspent` | outpoint | height, account, amount, flags, balance class, script kind, script location | `UnspentOutputs`, `ForEachUnspentOutpoint`, `randomUTXO`, `MakeInputSource` (`u`) |
   | `unmined_credits` | outpoint | amount, flags, account, balance class, script kind and location | `mc` |
   | `unmined_inputs` | outpoint | spender tx hash | `mi`: double-spend checks, skips |
   | `debits` | tx hash, height BE, input BE | amount, the spent credit's key | debits of `TxDetails` (`d`) |
   | `debits_invalidated` | as `debits` | as `debits` | `id` |
   | `tickets` | ticket hash | — | `OwnTicket`, `IterateTickets`, `UnspentTickets` |
   | `commitments`, `commitments_unspent` | outpoint | amount, account; unmined-spent flag | `cmt`, `cmu`; locked-by-tickets |
   | `balances` | account BE, class | sum, count | `AccountBalance(s)` |
   | `balance_heights` | account BE, class, height BE | sum, count | the depth-dependent categories |

   The script stays in the transaction record and credits keep its offset
   and length, so dcroxide's `PkScriptLocs` stays needed (ADR-0006
   addendum item 1; ADR-0013 Open point 9). Phase 2 adds what the spike did
   not need: multisig, voting accounts, VSP, agenda and treasury records and
   the aux buckets of spec 05, each keyed for its own access path; imported
   keys and scripts have no (branch, index), so `addr_by_path` cannot hold
   them: they are found by `addr_by_hash` and listed by `addr_by_acct_sha`
   under the imported account (0x7FFFFFFF), the only account whose stored
   order reaches a client (`getaddressesbyaccount "imported"`,
   internal/rpc/jsonrpc/methods.go:983-988; a BIP0044 account's listing is
   derived from its extended key in index order, :976-1030).
3. **Observable order is kept by the keys, and by one cursor adapter.**
   `unspent`, `unmined_credits`, `tx_unmined` and `unmined_inputs` hold
   exactly the key sets of dcrwallet's `u`, `mc`, `m` and `mi`, in its key
   bytes; `accounts` is keyed little-endian; `addr_by_acct_sha` is
   `ForEachAccountAddress` order (SHA-256 of the HASH160 within the
   account's little-endian key, wallet/udb/addressdb.go:907, :1337-1350,
   :1420); `history` orders a block's transactions by insertion, as
   dcrwallet's block record does. A small adapter over redb's gap cursor
   (`ReadableTable::lower_bound`) reproduces bbolt v1.3.12's positioned
   cursor, including what a step returns after running off either end
   (bbolt stays on the last or first element, bbolt#733); it is checked
   against a model written from bbolt's source, against bbolt itself on
   random key sets through the oracle (the spike's
   `bolt_cursor_matches_bbolt_itself`, a first form of brief §6 Phase 2's
   cursor-conformance test), and through the cross-check. Selection follows
   `MakeInputSource`'s algorithm and its order of randomness consumption;
   what is verified against dcrwallet is the set of outputs it may select,
   not the outputs a given random draw selects (see Correctness).
4. **Indexes and balances change in the same transaction as the data,
   through a handful of mutators.** Inserting or deleting an unspent output,
   an unmined credit, an unmined-input entry or an unspent commitment goes
   through one function each, which applies the class delta, and the
   per-height delta where the class is depth-dependent, according to
   whether the output is excluded by an unmined spender. An output's class
   is fixed when it is written, and the depth rules of `balanceFullScan`
   are applied at query time, over the shorter side of the depth window.
   The consistency check of Correctness is a property test, runs after
   every injected crash and power cut, and is the only consistency check:
   there is no bulk index build, and a rescan rebuilds from the rows.
5. **Stake invalidation, validation and rollback move rows between keyed
   tables, as dcrwallet's do.** A disapproved block's regular transactions
   keep their records under it; their credits and debits move to the
   invalidated tables, their unspent outputs go and the outputs they spent
   come back; a transaction mined again later gets a second record under
   the later block. dcrwallet's own sequence of a re-mined transaction
   rolled back and its first block revalidated leaves the same outpoint in
   `unspent` and `unmined_credits` at once, and `UnspentOutputs` then
   reports it twice, both times as unmined (outputCreditInfo,
   txmined.go:2244, prefers the unmined credit), and both count in the
   balance; the keys hold that state and the cross-check compares it
   (Open, "dcrwallet defects"). Two divergences, neither in what a client
   can read: `Rollback` deletes the invalidated rows of a detached block,
   which dcrwallet leaves under that block's hash where no code reads them
   (the logical export must then skip them, Open); and the spike deletes
   detached headers and filters, which Phase 2 must not, since
   `getblockheader` returns a detached block's header from dcrwallet's
   `h` bucket, which is never pruned (internal/rpc/jsonrpc/methods.go:1222,
   wallet/wallet.go:821-829).
6. **Schema versioning, from the first release on.** `meta` carries the
   version. Opening checks it: a missing version or one newer than the
   build is refused with a typed error naming both (`Error::Version`; the
   spike implements and tests it); an older one is migrated, each step one
   transaction, the last step stamping the new version; the same check runs
   on a store restored from a backup (item 8). A fixture store of every
   released schema version is kept, and CI upgrades each to current (brief
   §6 Phases 2 and 13).
7. **An engine file-format change.** redb has broken its file format twice:
   2.0.0 ("not backwards compatible with 1.x", `CHANGELOG.md:637-639`) and
   3.0.0, which dropped format v2 and pointed to `Database::upgrade()` in
   redb 2.6 (`CHANGELOG.md:469-470`); 5.0.0 refuses an older format with
   `UpgradeRequired` (`src/error.rs:469`, :520-523). dcroxide has no
   releases, so its stores can be rebuilt; a wallet's store holds state a
   rescan does not give back (account names, imported keys, unmined and
   unpublished transactions, VSP records, preferences; ADR-0013 Decision
   item 4), so seed restore is a lossy last resort here, not a migration.
   Because ADR-0006 rule 5 allows one redb major in the graph, a format
   break is crossed by a standalone migrator in its own workspace (as
   `spikes/` are), linking the old and the new major, copying table by
   table as raw key and value bytes, one transaction per table or batch,
   into a new file that is stamped and renamed over the old one atomically.
   The logical export cannot serve as the copy: it masks clock- and
   randomness-derived values and digests headers and filters (ADR-0001), so
   it is not a reload format. The alternative, a `bans.skip` for the old
   major in the main workspace, would be recorded as such. CI keeps a
   fixture per redb file-format version the wallet has shipped on.
8. **Backup (D15).** redb allows a consistent copy while the wallet runs: a
   read transaction is a snapshot, writers proceed beside it, and its pages
   are not reused while it lives. There is no page-level copy as bbolt's
   `Tx.Copy`; the backup is a snapshot copied table by table into a new file
   in one durable commit (measured above, with and without a writer; copy
   verified equal to the snapshot by a digest of every table), which also
   compacts it *(it does not: a fresh copy is compact only after an
   explicit compaction; ADR-0018, and the addendum of 2026-11-19
   "Decisions of the project owner", item 4)*. A raw copy of the file is safe only while no process has it
   open. What a user restores from is such a copy, opened as the wallet's
   store after the version check.
9. **Relaxed durability only where nothing external depends on it.**
   Proposed for the owner (Open point 4): every logical operation stays
   `Durability::Immediate`, except header-and-filter batches during initial
   sync and rescan, which may commit with `Durability::None` provided every
   Kth batch, the last batch and every write that a client or a peer can
   observe the effect of commit durably.

## Consequences

- Balance, selection, cold-start and per-account history costs no longer
  grow with the wallet; ingest commits once per write instead of twice.
- `balances`, `balance_heights`, `history`, `acct_history`,
  `addr_by_path`, `addr_by_acct_sha` and the unspent counter (as built,
  no counter: redb's table length serves; addendum of 2026-11-19) are maintained
  only by the write path, in the same transaction as the data, including
  through stake invalidation, validation and rollback; nothing builds them
  in bulk.
- The file is larger than its payload: at 1m, 2,372 MiB after ingest
  for 749 MiB stored (3.2x), 1,196 MiB compacted (1.6x), and 1.38x
  bbolt's file. A compaction policy is Open point 6.
- Opening for writing costs one fsync; recovery after a crash costs a walk
  of the whole file unless quick repair is enabled (Open point 3).
- Phase 3 waits on dcroxide's move to redb 5 and its stake gate (ADR-0006
  rule 5); until then no wallet crate may link `dcroxide-stake` or
  `dcroxide-gcs`. The power-loss rig either moves to redb 5 with dcroxide's
  `dcroxide-testutil` in that move, or the spike's port goes into the
  wallet's own test utilities. *Since 2026-11-19: dcroxide's redb 5 move
  landed at `f4b3d2e` and the pin moved to it, so the redb majors match;
  the stake gate has not landed, so a wallet crate linking `dcroxide-stake`
  links `dcroxide-database` until it does (ADR-0006 addendum). The rig went
  into `purse-store`'s own tests (`tests/support/powerloss.rs`).* *Since
  2026-11-20: the gate landed at dcroxide `80423a0` and the pin moved to
  `2074168`; a wallet crate takes `dcroxide-stake` with
  `default-features = false` and links neither `dcroxide-database` nor redb
  through it (ADR-0006, rule 5's note of that date).*
- The spike's oracle command imports `blockchain/stake/v5`, `txscript/v4`
  and `gcs/v4`, which `tools/oracle/go.mod` lists as indirect; a future
  `go mod tidy` would mark them direct.

## What the spike did not test, and what remains open

- **Not prototyped:** double-spend removal (`removeDoubleSpends`,
  `RemoveUnconfirmed`, so also the removal of unmined spenders of a
  rolled-back coinbase), conflicting unmined transactions, revocations,
  multisig, imported keys and scripts, `SetPublished` (which moves an output
  between unmined classes), VSP records and the aux buckets; a regular
  transaction of a disapproved block spending a credit of the same block;
  a transaction mined into a block already marked invalid (the paths exist,
  `insert_mined` and `add_credit`, but the workload cannot reach them
  without a rescan into such a block); a reorganisation that detaches a
  disapproved block. The prototype fails loudly where the workload could
  reach the first group.
- **dcrwallet defects seen while reading, for `docs/QUIRKS.md` under
  ADR-0012** (read from the source at the pin; only the first was run):
  1. the state of Decision item 5, an outpoint both mined (revalidated) and
     unmined (rolled back), reported twice by `UnspentOutputs` and counted
     twice in the balance; the transaction then stays unmined, and
     `InsertMemPoolTx` refuses it as already mined
     (wallet/udb/txunmined.go:26-29);
  2. `stakeInvalidate` restores an unspent entry for a credit it has just
     moved to `ic` when the disapproved block also spends it
     (txmined.go:857-878), and `stakeValidate`'s `spendCredit` then
     writes a zero-filled credit for it (txmined.go:728; txdb.go:942-953);
  3. a regular transaction of a block disapproved and never mined again
     keeps its mined record, so `InsertMemPoolTx` refuses it as already
     mined and the wallet cannot track it in the mempool again.
- **Scale and platforms.** The largest wallet was 2.4 GiB (the 1m store
  after ingest); Windows and macOS, other file systems and spinning disks
  were not measured; neither was memory use, nor readers running against a
  writer under load.
- **Committed filters** in the transaction workload were the empty filter
  on both sides; the header measurement used generated filters of a
  stated size, not real ones.
- **Open points**, each for the owner, with a recommendation:
  1. *Schema.* **Decided as recommended (project owner, 2026-11-19).** Accept Decision items 2 to 8. Recommended: yes.
  2. *The double state of Decision item 5 and the other defects above.* **Decided as recommended (project owner, 2026-11-19).**
     Reproduce what a client sees (ADR-0012 Rule 1), or fix it under Rule 7
     as a spend wrongly recorded in persisted state? Recommended: record
     them in `docs/QUIRKS.md` and decide each in Phase 3 with an oracle
     run; the schema represents both outcomes.
  3. *Quick repair.* **Decided as recommended (project owner, 2026-11-19).** Recommended: off. On the 1m store it cut the reopen after a
     kill from 1.33 s to 10 ms and raised a small commit's median from 5.3
     to 5.5 ms to 11.0 to 20.3 ms: a cost on every write against a saving
     once per crash. Revisit if multi-gigabyte wallets repair much more
     slowly.
  4. *Relaxed durability for header and filter batches* **Decided as recommended (project owner, 2026-11-19).** (Decision item 9).
     Recommended: yes, for header-and-filter batches during
     initial sync and rescan only, with a durable commit at least every 50
     batches (a crash then re-fetches at most 98,000 headers), at the end of
     the sync, and for every write whose effect a client or a peer can see;
     nothing a node, a peer or a client observes changes.
  5. *Detached headers and orphan invalidated rows in the logical export.* **Decided as recommended (project owner, 2026-11-19).**
     Recommended: Phase 2 keeps detached headers and filters keyed by hash,
     as dcrwallet does, because `getblockheader` returns them; the
     invalidated rows of detached blocks are not kept, and ADR-0001 skips
     them on the Go side, with the call-site trace ADR-0012 Rule 5 asks for
     (the Go export of this spike already skips them).
  6. *Compaction policy* **Decided as recommended (project owner, 2026-11-19).** (on backup, which compacts anyway, and on demand).
     Recommended: compact only through backup in 1.0.
  7. *Whether `addr_by_hash` stores public keys* **Decided as recommended (project owner, 2026-11-19).** (33 bytes per address)
     so that address lookups need not derive them. Recommended: decide in
     Phase 3 against the RPCs that return keys.
  8. *Whether `credits` keeps spent credits forever* **Decided as recommended (project owner, 2026-11-19).** (dcrwallet does) or
     prunes them behind a history horizon. Recommended: keep them; history
     RPCs read them.
  9. *Fewer tables per commit.* **Decided as recommended (project owner, 2026-11-19).** Merging `headers`, `blocks`,
     `block_by_hash` and `cfilters`, or other tables every block commit
     touches, would cut the pages each commit writes (see "What the schema
     costs"); not measured. Recommended: measure in Phase 2 before the
     schema is frozen. *Measured (2026-11-19 addendum): each further table
     a commit touches costs about 6.9 KiB and well under 1 ms; not merging
     is recommended.* *Decided (project owner, 2026-11-19): not merged;
     addendum "Decisions of the project owner".*
- **Power-loss rig before acceptance.** The sweep above ran on redb 5.0.0
  at every storage operation of the tiny workload; it should be repeated
  on the exact 5.0.x the wallet pins when dcroxide moves (ADR-0006 rule 5),
  since the 4.6.0 and 5.0 repair changes mean a result on one release does
  not carry to another.

No disagreement between the specs read (02, 03, 04, 05) and the Go source
was found on the paths the spike exercised.

## Addendum, 2026-11-19 — Phase 2 measurements: headers and filters by height or by hash, and tables per commit

ADR-0014 Open point 6 and this ADR's Open point 9 were both decided as
"measure in Phase 2". The measurements are `crates/purse-store/tests/measure.rs`
(ignored tests, rerun with the command in its header) and the
2026-11-19 rows of [`docs/bench-ledger.md`](../bench-ledger.md).

**Method.** Every store is a real file written through `purse-store`'s own
commit path (`Store::update`, `Durability::Immediate`, redb 5.0.0 as
pinned) under a backend that wraps redb's file backend and counts bytes
written, write calls and syncs. Byte, write and sync counts are
deterministic and were identical in all three runs; times are medians of
three runs in one session with their range, on an x86_64 Linux desktop
(16 cores, NVMe SSD, btrfs with compression over an encrypted mapping)
that was carrying other work (load average 9 to 27), so a single durable
commit's sync drifted between 8 and 20 ms from run to run. Release build,
Rust 1.100.0. No Go was run: both questions are between layouts of the
Rust store.

**Headers and filters (ADR-0014 Open point 6).** 1,050,000 blocks (about
mainnet's height when the pin's chaincfg was cut), a 180-byte header and a
committed filter of 170 to 369 bytes (270 on average, the assumption of
"Header and filter ingest" above) each, in 525 commits of 2,000 blocks,
in three layouts: *hash*, the schema as built (`headers` and `cfilters` by
block hash, `blocks` by height, `block_by_hash`); *height* (`headers` and
`cfilters` by height, the other two unchanged); and *merged* (Open point
9: one table by height holding block row, header and filter, plus
`block_by_hash`). Then 200 single-block commits at the tip of the grown
file, and warm reads.

| | hash | height | merged |
|---|---:|---:|---:|
| whole ingest, median (range) | 90.0 s (89.3–91.5) | 32.4 s (30.1–45.5) | 32.5 s (28.8–43.2) |
| bytes written by the ingest | 13.99 GiB | 4.06 GiB | 4.05 GiB |
| write calls | 3,667,272 | 1,065,867 | 1,062,337 |
| one 2,000-block commit, last 50, p50 | 201.8 ms (198.0–212.8) | 71.9 ms (68.3–88.9) | 65.2 ms (64.8–69.2) |
| bytes per 2,000-block commit, last 50 | 32.25 MiB | 9.21 MiB | 9.18 MiB |
| one-block tip commit, p50 | 18.25 ms (9.73–18.78) | 18.28 ms (17.40–19.46) | 18.59 ms (17.01–19.84) |
| bytes and writes per tip commit | 85.2 KiB, 22.2 | 84.4 KiB, 22.0 | 52.5 KiB, 14.0 |
| header by height, warm | 1.59 µs | 0.65 µs | 0.80 µs |
| header by hash, warm | 0.87 µs | 1.57 µs | 1.87 µs |
| 2,000 consecutive filters by height, warm | 1.79 ms | 0.17 ms | 0.26 ms |
| file after the ingest; after close | 1,088 MiB; 1,088 MiB | 1,088 MiB; 1,029 MiB | 1,088 MiB; 1,031 MiB |

Keyed by hash, every batch of new headers lands on random leaves of two
grown trees, so a 2,000-block commit rewrites 3.5 times the bytes of the
same commit keyed by height, whose new rows append to the right edge;
initial sync with every commit durable takes 2.8 times as long. A
rescan's filter
reads by height are about ten times faster by height, one range scan
instead of a lookup per block. A commit of one block at the tip costs the
same either way: one leaf per table. What the height layout costs is a
header looked up by hash (`getblockheader`, `GetBlockMetaForHash`): two
lookups instead of one, 1.57 against 0.87 µs warm. Detached headers and
filters, which the height layout keeps in their own tables keyed by hash
(Open point 5), were not exercised: the measurement has no
reorganisation, and how a reorganisation moves rows between the main-chain
and detached tables is not designed yet.

*Answer to ADR-0014 Open point 6:* key main-chain headers and filters by
height, detached ones by hash in separate tables. The relaxed-durability
figures of "Header and filter ingest" above were measured on the spike's
height keys and assume this layout. The schema in `purse-store` still keys
them by hash; changing it adds tables to Decision item 2's list and changes
`SyncBatch`, and waits on the owner. *Decided (project owner, 2026-11-19):
height keys, in schema version 1, before Phase 3; addendum "Decisions of
the project owner".* *Done (2026-11-20): built in schema version 1;
addendum "Chain tables keyed by height, as built".*

**Tables per commit (Open point 9).** Fifteen tables of 100,000 rows each
(36-byte random keys, 60-byte values: the shape of credit and unspent
rows), 272 MiB; then commits of 30 new rows with random keys spread evenly
over 1, 2, 4, 8 or 15 tables chosen at random, 100 commits per spread, the
spreads interleaved.

| tables a commit touches | 1 | 2 | 4 | 8 | 15 |
|---|---:|---:|---:|---:|---:|
| bytes per commit | 222.6 KiB | 245.9 KiB | 264.6 KiB | 287.6 KiB | 319.8 KiB |
| write calls per commit | 56.6 | 62.4 | 67.1 | 72.8 | 80.9 |
| p50, median of three runs | 18.21 ms | 18.38 ms | 18.36 ms | 18.62 ms | 19.11 ms |

Each further table a commit touches adds about 6.9 KiB (under two pages:
its root-to-leaf path above the leaves the rows need anyway) and, within
any one run, 0.9 to 1.2 ms over fifteen tables, 5 to 15 % of a commit that
the single sync dominates. Merging `blocks`, `headers` and `cfilters`
into one table beside `block_by_hash` (four tables a commit to two: the
*merged* layout above) cuts a tip commit's bytes by 38 % (52.5 against
85.2 KiB) and changes nothing measurable in its time or in a 2,000-block
batch. Open point 9 also proposed folding `block_by_hash` in; that
variant, one table a commit, was not measured.

*Answer to Open point 9:* do not merge. A block commit that touches about
fifteen tables pays at most about 15 % in time and 44 % in bytes over one
that touched a single table, against the read-side gains the derived tables
buy (Performance, above: balances 670x and more, history by account 11x to
230x); the header-and-filter gain comes from height keys, not from fewer
tables. Revisit if bytes written per commit come to matter in their own
right (flash wear on small devices), which was not studied.

**The power-loss rig on the pinned release** ("Power-loss rig before
acceptance", above). The spike's port of dcroxide's backend now lives in
`purse-store`'s tests (`tests/support/powerloss.rs`, answering the last
Consequences bullet), and the sweep ran on redb 5.0.0 exactly as the
workspace pins it: a power cut at every storage operation of a workload
(creation, wallet-shaped commits, an initial sync with a reorganisation,
a bulk delete), clean, torn, and with every write kept as a killed process
leaves it; with every commit durable and with relaxed initial-sync
batches. Every reopened store held exactly the rows of one commit, at or
after the last acknowledged durable one, and relaxed commits were lost
only within their bound; a backend that acknowledges syncs without making
anything durable fails the sweep (`tests/powerloss.rs`). Migrations cut at
every write end at the old or the new version (`tests/migration.rs`).
These run on Linux in this measurement; CI runs them on every platform of
its matrix.

## Addendum, 2026-11-19 — Schema version 1 as built: wallet-wide rows, no unspent counter

Review of `purse-store` found two places where Decision item 2's schema,
as first built, did not match what the store must hold. Both are changed
in schema version 1 before it is released (no dcr-purse release exists,
and no wallet has been written at version 1, since `purse-udb` is not
built); the checked-in version-1 fixture
(`crates/purse-store/tests/data/schema-v1.redb`) was regenerated with them,
and its pinned SHA-256 in `tests/migration.rs` changed with it.

**Wallet-wide rows.** ADR-0001, accepted, exports the address manager's
wallet-wide state (§6.1 `[wallet]`: watch-only, last account, last
imported account; §6.2 `[masterkeys]`; §6.3 `[cryptokeys]`; §6.4
`[cointypes]` with the parked SLIP0044 account 0 row) and the transaction
store's birthday and have-cfilters rows (§6.8), and its restore contract
needs the active coin type and the birthday; §11 has the Rust producer
take every field from dcr-purse's own state. Neither Decision item 2's
table nor its "Phase 2 adds" list had a place for them, and the closed
`meta` names turned any such row into a corrupt-row error on a typed walk.
The schema now has:

- a `wallet` table keyed by name, one value type per name
  (`purse_store::wallet::WalletRecord`): `cointype_legacy` and
  `cointype_slip0044` (sealed extended public and private key, each
  optional), `crypto_pub` and `crypto_priv` (sealed bytes),
  `last_account` and `last_imported` (u32 BE), `master_pub` and
  `master_priv` (salt, digest, scrypt N, r, p; 88 bytes),
  `slip0044_acct0` (the two version-1 counters, sealed keys, name) and
  `watch_only` (one byte); the layouts are on the value types in
  `crates/purse-store/src/values.rs`;
- in `meta`, `birth` (dcrwallet's 45-byte `birthstate` layout: options,
  hash, height BE, time BE) and `have_cfilters` (one byte).

dcrwallet's `mgrcreated`, `wtxmgr/date` and `wtxmgr/bal` are not kept:
each is written from udb's own clock or balance and read by nothing at the
pin (ADR-0001 §6.1), so under ADR-0001 §11 comparisons of those fields use
the observable profile. The VSP host index (`vsphostindex`) belongs to the
VSP records, which come with them in a later schema version, as do the
other records "Phase 2 adds" names (multisig, voting accounts, agenda and
treasury policies, spec 05's aux buckets).

**No unspent counter.** The `meta` unspent count duplicated what redb
already keeps: every table's length is in its root and read in O(1)
(redb 5.0.0 `src/tree_store/btree.rs:937-938`, `:1226-1227`), the
uncommitted writes of a write transaction included. The counter row could
only drift from the table, and nothing would have noticed. It is gone;
`KeyN` of `u` is `ReadTx::unspent_count`, the length of `unspent`. In a
write transaction that count includes the transaction's own writes, where
bbolt's does not (QK-0003); dcrwallet's one such call, `txToMultisig`'s,
comes before any write to `unspent` in its transaction (PARITY.md,
`purse-store`).

## Addendum, 2026-11-19 — Decisions of the project owner: height keys, unmerged tables, backup, and the store's open items

On 2026-11-19 the project owner decided the questions the two addenda
above left open, accepted ADR-0018 (D15), whose Open point 9 calls for
this addendum, and settled smaller points about `purse-store`. Each is as
recommended. The text above stays as written, apart from the dated
pointers added to it.

1. **Headers and filters keyed by height** (ADR-0014 Open point 6; the
   first addendum's answer). Main-chain headers and committed filters are
   keyed by height; detached ones stay keyed by hash, in tables of their
   own, as Open point 5 decided. This is a change to schema version 1,
   made before Phase 3, while no dcr-purse release exists and no wallet
   has been written at version 1 (as the second addendum's changes were).
   It adds tables to Decision item 2's list, changes `SyncBatch`, and
   regenerates the checked-in version-1 fixture and its pinned digest. The
   tables' names, how a reorganisation moves rows between the main-chain
   and detached tables, and the measurement of reorganisations the first
   addendum did not make, come with that change. Grounds: by hash, initial
   sync wrote 3.4 to 3.5 times the bytes and took 2.8 times as long with
   durable commits; a header looked up by hash costs a second lookup.
   *Done (2026-11-20): addendum "Chain tables keyed by height, as built"
   below.*
2. **Tables are not merged** (Open point 9). Decision item 2's tables stay
   separate; revisited only if bytes written per commit come to matter in
   their own right.
3. **Confirmed** (project owner, 2026-11-19), as decided before: compaction
   only through backup in 1.0 (Open point 6), whether `addr_by_hash`
   stores public keys decided in Phase 3 (Open point 7), and spent credits
   kept (Open point 8).
4. **Backup is not compacting by itself** (ADR-0018 Open point 9).
   Decision item 8 says the snapshot copy "also compacts it". ADR-0018's
   measurements refute that: a fresh copy of a 649 MB store was 570 MB,
   and 412 to 422 MB only after an explicit compaction, where `Balanced`
   and `Maximum` gave the same file. Item 8 is corrected: a backup is the
   snapshot copied into a new file in one durable commit and then
   compacted by an explicit step, `CompactionMode::Balanced`, before it is
   published by rename (ADR-0018 Decision items 1 to 3). Open point 6's
   premise, "on backup, which compacts anyway", is restated accordingly:
   backup compacts because ADR-0018 makes compaction one of its steps. Its
   decision, compaction only through backup in 1.0 and never in place, is
   unchanged. What a user restores from, and how, is ADR-0018's (its
   Decision items 5, 6 and 8).
5. **The store's file name** (ADR-0018 Open point 6) is fixed in Phase 2
   with the schema, a name other than `wallet.db`. `purse-store` takes the
   path from its caller, so the name is still to be chosen. *2026-11-20:
   proposed as `wallet.redb` (`purse_store::STORE_FILE_NAME`), awaiting
   the owner: ADR-0018, addendum "The `purse-store` part, as built", Open
   point BK1.*
6. **Integrity check on open.** Whether the wallet opens its own store
   with redb's integrity check (`purse-store`'s
   `OpenOptions::check_integrity`, off by default) is decided in Phase 3.
   A file handed in from outside is always opened with it (ADR-0018 Decision item 6).
7. **`KeyN` of `unspent` in a write transaction.** The store's count
   includes the transaction's own writes, where bbolt's does not
   (QK-0003): accepted. Phase 3 carries the obligation the second addendum
   and PARITY.md state: `purse-udb` keeps `txToMultisig`'s count before
   any write to `unspent` in its transaction, or reproduces bbolt's
   committed-only count for that call.
8. **bbolt's cursor defects under mutation** (QK-0747 to QK-0750) are not
   reproduced: accepted.
9. **Tests.** The model-based property test runs 48 sequences by default
   (`PURSE_STORE_MODEL_CASES`), with a large count in a nightly job, which
   CI does not have yet. The `store_open` fuzz target runs in CI without
   debug assertions, as the wallet ships, because redb 5.0.0's debug
   assertions fire on some damaged files; it is run once by hand with
   them. Fuzz soaks of the store (with those of `sntrup4591761` and
   `purse-txauthor`) come before its PARITY row moves to pinned.
10. **`dcroxide-stake`'s storage gate** is scheduled to land in dcroxide,
    and the pin to reach it, before Phase 3 (ADR-0006 rule 5; the addendum
    to ADR-0006 of 2026-11-19 on the owner's scheduling). *Landed at
    dcroxide `80423a0`; the pin reached it at `2074168` on 2026-11-20.*

## Addendum, 2026-11-19 — Chain tables keyed by height, as built

The project owner decided on 2026-11-19, after the Phase 2 measurement
addendum above, that schema version 1 keys main-chain headers and
committed filters by height, before Phase 3, and that the tables are not
merged (the addendum "Decisions of the project owner", items 1 and 2). No store has been released and
no wallet written at version 1, so version 1 itself changes and no
migration from the hash layout exists. The checked-in version-1 fixture
(`crates/purse-store/tests/data/schema-v1.redb`) was regenerated with the
documented command (`write_current_schema_fixture` in
`tests/migration.rs`, run twice, byte-identical) and its pinned SHA-256
changed with it.

**Layout.** Six chain tables (`purse_store::chain`, whose module
documentation is the reference):

| table | key | value |
|---|---|---|
| `blocks` | height (u32 BE) | block record (unchanged) |
| `headers` | height | 180-byte header of the main-chain block |
| `cfilters` | height | committed filter of the main-chain block, when it has one |
| `block_by_hash` | block hash | height: the hash index of the three above |
| `headers_detached` | block hash | header of a block not in the main chain |
| `cfilters_detached` | block hash | its filter |

Seven invariants hold between them: `blocks` holds the heights 0 to the
tip without a gap; `headers` holds exactly the heights of `blocks`, each
header stating its height and, above genesis, naming the block below;
`cfilters` holds only heights of `blocks` (a block may lack its filter);
`block_by_hash` is exactly the index of `blocks`; the `tip` row is the
last block, absent with no chain; no detached row is of a block in
`block_by_hash`; every detached filter has its detached header. The store
reads two header fields (previous block at bytes 4 to 36, height
little-endian at 128 to 132, checked against dcroxide-wire's
serialization) and does not hash headers.

**Operations.** The tables change only through typed operations of
`WriteTx`, each checking everything it can refuse before it writes,
bbolt's value limit on every filter it is given included (the one put
limit a chain row can reach), so a refused call writes nothing and the
transaction stays usable; an engine error part-way through the writes
(kind `Io`) may leave some of them, and the caller abandons the
transaction, as `Store::update` does on any error: connect
blocks above the tip (or from genesis), promote a detached block from its
stored header and filter, disconnect from a height (dcrwallet's
`Rollback`; genesis is never detached), reorganise at a fork point (detach
and connect in one call, a block of the detached branch allowed back),
record a detached header and filter, add a main-chain block's missing
filter. A main-chain block delivered again with its own header is
refused with dcrwallet's kind `Exist` and text, checked as
`ExtendMainChain` checks it: at the height the header states, before the
height the caller gives and the parent (PARITY.md has the order and the
two refusals dcrwallet lacks). A header stating height 0 above genesis,
or 2^31 or more (a negative `int32` to dcrwallet), gets dcrwallet's
`block 0 cannot be added` before that, so no operation puts a block above
height 2^31 - 1. A detach moves each block's header and filter to the detached
tables and removes its block row and index row in the transaction that
detaches it; a connect takes a detached block's rows back. A
reorganisation is therefore atomic with whatever else its transaction
writes, the wallet's rows included. `SyncBatch` offers the same operations
under relaxed durability and refuses to connect or detach a block row
with wallet transactions or the invalidation flag; the 50-commit rule
(Decision item 9) is unchanged. A lookup by hash goes through
`block_by_hash` to the main-chain row, or to the detached table: two reads
for a main-chain block where dcrwallet's `h` and `cf` buckets take one.
The generic `put` and `delete` still reach these tables, for migrations
and tests, unchecked; `ReadTx::check_chain` detects what they break.

**Tests.** The model-based property test runs random connects,
disconnects, reorganisations, promotions, detached inserts and missing
filters, both in relaxed header batches and in durable `Store::update`
transactions committed or abandoned, with block rows carrying wallet
state that a header batch must refuse to move; after every operation the
store's whole contents equal the model's and the chain tables pass both
`check_chain` and a checker of the tests' own (`tests/support/chain.rs`),
which reads the tables as raw rows and checks every correspondence in
both directions (each height row has its index row and each index row its
height row; heights contiguous from 0 to the tip). The power-loss rig
cuts one reorganisation at every storage operation from the open to the
close, clean, torn and with every write kept: one block replaced at the
tip and forty, durably with wallet rows in the same transaction, and one
and four as a relaxed batch made durable by `finish`, the new branch
taking a detached branch back. Every reopened image holds the old chain
or the new whole (the store's digest, both checks, and the chain's hashes
by height, index, filters and detached rows exactly the old or the new),
and a reorganisation acknowledged as durable is never lost. The
workload-wide sweep, the migration sweeps (whose version-1 image now
holds a reorganised chain), the newer-store refusal and the fixture test
run both checks too. The refusal tests of `tests/store.rs` pin the order
of the refusals by their text, a filter over bbolt's value limit refused
before the first write (a 2 GiB filter allocated zeroed and never
written, on 64-bit targets only), a header stating 2^31 or more refused
with dcrwallet's text (live against `ExtendMainChain` through the
`oprunner` oracle), and a tip written raw at height `u32::MAX`, above
which nothing connects. Run on Linux only in this work.

**Measured** (`tests/measure.rs`, the bench ledger's entry "the chain
tables keyed by height, as built": three runs on the shared desktop of the
Phase 2 addendum, load average between 7 and 25, Go not run). Through the
chain operations the ingest of 1,050,000 blocks in durable 2,000-block
commits writes exactly what the raw height layout writes, 4.06 GiB
(9.21 MiB a commit; 84.4 KiB a tip commit), against 13.99 GiB keyed by
hash, 3.45 times fewer bytes; it took 33.9 s (30.1–36.4) at the median
against 103.9 s (97.9–134.8) keyed by hash, 3.1 times faster, and against
37.5 s (33.0–53.1) for the raw height rows, so the operations' extra reads
and checks cost no I/O and no time this session could resolve. As initial
sync runs it, with relaxed batches, the same ingest writes 1.09 GiB in 29
syncs and took 9.3 s (8.5–10.8). Warm, a header by height takes 0.73 µs,
by hash 1.62 µs (two lookups; 1.00 µs keyed by hash), and 2,000
consecutive filters 0.16 ms (1.98 ms keyed by hash). A durable
reorganisation at the tip of the grown file writes 100.4 KiB in 26 writes
at depth 1 and 184.4 KiB in 47 at depth 6, against 84.4 KiB in 22 for a
one-block connect, and took 10.2 and 11.2 ms at the median, a commit's
sync on that machine being about 10 ms.

**Not done or not measured.** A sync cannot start at a checkpoint: the
main chain is contiguous from height 0, as dcrwallet's always is. Cold
reads, headers of a real chain, reorganisations deeper than 6 blocks on a
grown file, and platforms other than Linux on x86_64 were not measured.
The 2.8x initial-sync figure of the Phase 2 addendum was measured on raw
rows; this addendum's 3.1x is through the operations as built, on the
same machine under different load.

## Addendum, 2026-11-20 — Decision of the project owner: the address manager before the stake gate

Item 10 of the addendum "Decisions of the project owner" scheduled
`dcroxide-stake`'s storage gate, and the pin reaching it, before Phase 3.
On 2026-11-20 the project owner accepted the address manager slice of
Phase 3 (`purse-udb`, commit `94afd6e`, over this store's schema version 1
unchanged) ahead of the pin reaching the gate, an exception to that
schedule (ADR-0006, addendum of 2026-11-20, item 1). The slice links
neither `dcroxide-stake` nor `dcroxide-gcs`. The pin moved onto the gate
the same day, at dcroxide `2074168` (item 10's dated note), so the
transaction store, the first wallet code to link them, has the gate in
the pin. Nothing in this record's schema or its other addenda changes.

## Addendum, 2026-11-20 — The remaining records, as built

Decision item 2 has Phase 2 add "multisig, voting accounts, VSP, agenda
and treasury records and the aux buckets of spec 05, each keyed for its
own access path", and imported keys and scripts found by `addr_by_hash`
and listed by `addr_by_acct_sha` under the imported account. This
addendum lists every bucket and record dcrwallet stores at the parity pin
that `purse-store` did not yet hold, says how each is now held, and
records the tables built for them. They are added to schema version 1:
no dcr-purse release exists and no wallet has been written at version 1,
so version 1 itself changes and no migration exists for the change, as
for the two earlier changes to version 1 (addenda "Schema version 1 as
built" and "Chain tables keyed by height, as built"). The checked-in
version-1 fixture (`crates/purse-store/tests/data/schema-v1.redb`) was
regenerated with the documented command (`write_current_schema_fixture`,
run twice, byte-identical; 282,624 bytes) and its pinned SHA-256 in
`tests/migration.rs` changed with it.

**What was not held, and who reads it.** From specs 02, 03 and 05 and the
Go source at the pin (`wallet/udb/txdb.go`, `vsp.go`, `treasury.go`,
`agendas.go`, `addressdb.go`, `addressmanager.go`):

| dcrwallet bucket or record | key | value | read by | order reaches a client |
|---|---|---|---|---|
| `wtxmgr/ms` (multisig outputs) | outpoint (hash ‖ index BE, 36) | 135 bytes: script HASH160, m, n, flags (bit 0 spent, bit 1 stake tree), block hash, height BE, amount BE, spender hash, spender index BE, transaction hash (`valueMultisigOut`, txdb.go:1755-1806) | `GetMultisigOutput` (`FetchP2SHMultiSigOutput`, wallet/multisig.go:33, for `getmultisigoutinfo`; chainntfns.go:529); `AddMultisigOut` (createtx.go:314, chainntfns.go:635); `SpendMultisigOut`; `Rollback` and `RemoveUnconfirmed` rewrite it | no: lookups by outpoint |
| `wtxmgr/mu` (unspent multisig) | outpoint | one byte 0 | `UnspentMultisigCreditsForAddress` walks the whole bucket and keeps one script's outputs (txmined.go:2875-2927) | **yes**: `redeemmultisigouts` returns them in that walk's order (internal/rpc/jsonrpc/methods.go:4186-4215) |
| `vsp` (VSP tickets) | ticket hash | 40 bytes: fee hash, fee status BE, host id BE (vsp.go:147-179) | `GetVSPTicket` (wallet.go:5656, :5673, :5691; vspticket.go:321), `SetVSPTicket` (vspticket.go:249-282), `DeleteVSPTicket` (chainntfns.go:270), `GetVSPTicketsByFeeStatus` (wallet.go:5618, for gRPC `GetVSPTicketsByFeeStatus`, rpcserver/server.go:4065, :4081) | no: lookups, and a Go map whose order is random (QK-0125) |
| `vsphost` (VSP hosts) | host id BE | length BE ‖ host; plus the counter row `vsphostindex` → highest id BE, and id 0 → empty host seeded by upgrade 23 (vsp.go:52-68, :185-234) | `SetVSPTicket`, `GetVSPTicket`, `GetVSPHost` | no |
| `vsppubkey` | host bytes | host id BE ‖ length BE ‖ public key (vsp.go:241-289) | `SetVSPTicket` (is the host known), `GetVSPTicket`, `GetVSPPubKey` | no |
| `tspendpolicy` | tspend hash | one byte, any non-zero (0 is the key's absence; QK-0127) | `TSpendPolicy`; `TSpendPolicies` loads every row into a map at start (wallet.go:379) | no: map |
| `treasurypolicy` | pi key bytes | one byte | `TreasuryKeyPolicy`, `TreasuryKeyPolicies` (wallet.go:389) | no: map |
| `vsptspendpolicy` | ticket ‖ tspend hash | one byte | `VSPTSpendPolicy`, `VSPTSpendPolicies` (wallet.go:383) | no: map |
| `vsptreasurypolicy` | ticket ‖ pi key | one byte | `VSPTreasuryKeyPolicy`, `VSPTreasuryKeyPolicies` (wallet.go:393) | no: map |
| `agendaprefs` | version BE ‖ agenda id | choice id | `DefaultAgendaPreference` (wallet.go:330, :440) | no: lookups |
| `ticketsagendaprefs/<ticket>/` (one nested bucket per ticket) | version BE ‖ agenda id | choice id | `TicketAgendaPreference` (wallet.go:361, :442) | no: lookups |
| `waddrmgr/addr` rows of type 1 and 2 (imported keys and scripts) | SHA-256 of the HASH160 | type, account, add time, sealed keys or sealed hash and script | `Address`, `RedeemScript`, `PrivateKey`, `ForEachAccountAddress` | yes, for the imported account: `getaddressesbyaccount "imported"` (methods.go:983-988) |
| `waddrmgr/acct` rows of type 2 and `acctvars/<n>/kdf-params` (imported voting accounts) | account LE | sealed keys; Argon2id parameters | the address manager | as every account: account order |
| `wstakemgr` | — | dead data, not created at the pin (spec 05 §4.5, QK-0126) | nothing | — |

The last three need no new table. An imported key or script is an
`AddrRow::Imported` row of `addr_by_hash` with its account, and its
`addr_by_acct_sha` row keys it by the account's little-endian number and
the SHA-256 of its HASH160, so one prefix walk of account 0x7FFFFFFF is
dcrwallet's `ForEachAccountAddress` order (`purse-udb` writes both rows,
`addresses.rs`; `tests/records.rs` holds the order on 50 imported rows).
A voting account is an `accounts` row of its own type (2) whose key
material carries the Argon2id parameters (`purse-udb`'s `AccountKeys`).
`wstakemgr` is never read, and no dcrwallet file is imported (ADR-0013).
The `tix` bucket's value, the picked height, is always written as -1 and
never read at the pin (txmined.go:1333-1336, txunmined.go:120-123), which
is why `tickets` keeps no value.

**The tables** (`crates/purse-store/src/records.rs`, whose module
documentation is the reference; layouts on the key and value types in
`keys.rs` and `values.rs`):

| table | key | value | serves |
|---|---|---|---|
| `multisig_outs` | outpoint | `MultisigOutRow`, 103 bytes: script HASH160 ‖ m ‖ n ‖ flags (bit 0 spent, bit 1 stake tree) ‖ block hash ‖ height BE ‖ amount BE ‖ spender hash ‖ spender input BE | `ms` |
| `multisig_unspent` | script HASH160 ‖ outpoint | empty | `mu`, one prefix walk per script |
| `vsp_tickets` | ticket hash | `VspTicketRow`, dcrwallet's 40 bytes | `vsp` |
| `vsp_fee_status` | fee status BE ‖ ticket hash | empty | the tickets of one status |
| `vsp_hosts` | host id BE | host bytes | `vsphost` without its counter |
| `vsp_pubkeys` | host bytes | host id BE ‖ public key | `vsppubkey` |
| `tspend_policies` | tspend hash | `TreasuryVote` (one byte, never 0) | `tspendpolicy` |
| `treasury_key_policies` | pi key bytes | `TreasuryVote` | `treasurypolicy` |
| `vsp_tspend_policies` | ticket ‖ tspend hash | `TreasuryVote` | `vsptspendpolicy` |
| `vsp_treasury_key_policies` | ticket ‖ pi key | `TreasuryVote` | `vsptreasurypolicy` |
| `agenda_prefs` | version BE ‖ agenda id | choice bytes | `agendaprefs` |
| `ticket_agenda_prefs` | ticket ‖ version BE ‖ agenda id | choice bytes | `ticketsagendaprefs`, flattened |

Where dcrwallet's order reaches a client, the keys keep it: within one
script, `multisig_unspent`'s order is the outpoint order of dcrwallet's
`mu` walk, so `redeemmultisigouts` reads one script's entries instead of
every unspent multisig output of the wallet, in the same sequence. The
policy and preference tables keep dcrwallet's key bytes (their readers
are lookups by key, and the per-ticket rows of one ticket are contiguous,
as its nested bucket was). `vsp_fee_status` is new: dcrwallet walks every
VSP ticket for `GetVSPTicketsByFeeStatus`; the index reads only the
tickets of the status asked, at one more row written per ticket change.

Differences from dcrwallet's layout, none of which a client can observe:

- `multisig_outs` drops the transaction hash, which is always the key's,
  and is canonical: an unspent output's spender bytes and an unmined
  one's block bytes are zero, where dcrwallet's unspend leaves one byte
  of the old spender hash and writes index 0xFFFFFFFF (QK-0055); nothing
  reads the spender of an unspent output (`wallet/multisig.go:38-90`,
  `getmultisigoutinfo` reports it only when spent, methods.go:2296-2299).
  The stake-tree flag is a field of its own, so that dcrwallet's outcome
  (its spend and unspend clear it, QK-0054, which then reaches
  `redeemmultisigouts` as the output's tree) and the corrected one can
  both be written; the transaction store chooses in Phase 3. Because an
  all-zero block hash means unmined in both layouts, a mined output's
  block (`MultisigBlock`) cannot carry that hash: its fields are private
  and `MultisigBlock::new` returns no block for it. Before that, a row
  with a block of hash zero was accepted by `put_multisig_out` and then
  read back as unmined (height 0) or refused as corrupt (any other
  height), which broke the codec's round trip and made every backup of
  such a store fail verification; `tests/records.rs` and the encodings
  generator now cover the zero hash.
- No `vsphostindex` row: hosts are never deleted and each new id is the
  highest plus one, so the last key of `vsp_hosts` is the counter (as the
  length of `unspent` is `KeyN`). No id 0 with the empty host either:
  `GetVSPTicket` treats an absent host and an empty one alike
  (vsp.go:112-123); `GetVSPHost(0)`, which no wallet code calls, is
  answered with the empty host by the layer above (the differential's
  interpreter does so, and `purse-udb` will). The one udb-level path
  where the counter and the last key could differ is `SetVSPHost` called
  with an id out of order, which has no caller at the pin (Open point R2).
- The public-key and host values drop their length prefixes (the length
  is the rest of the value).
- A treasury vote of 0 is refused by the value type, being dcrwallet's
  spelling of an absent row; any other byte is kept, as dcrwallet's
  setters keep it (only the wallet layer limits it to 1 or 2, QK-0127).

**Operations.** `WriteTx::put_multisig_out` and `delete_multisig_out`
keep `multisig_unspent`; `add_vsp_host` gives the next id to a new host
(1 for the first) and writes `vsp_hosts` and `vsp_pubkeys` together, or
returns a known host's id, keeping its first public key (QK-0122);
`put_vsp_ticket` and `delete_vsp_ticket` keep `vsp_fee_status`; the
readers `vsp_host_id`, `vsp_tickets_with_fee_status` and
`unspent_multisig_for` are on both transaction types. Each operation
checks what it can refuse before it writes, so a refusal leaves the
transaction usable: an empty host (bbolt's `key required`, where
dcrwallet has already written the host row and counter and its caller
abandons the transaction, QK-0122), a host over bbolt's key limit, a new
host when id `u32::MAX` is taken (dcrwallet's counter would wrap to 0),
a ticket naming a host id the store does not hold. The policy and
preference tables have no invariant and use the generic typed `put` and
`delete`. `ReadTx::check_records` checks the four invariants (each
index exactly matches its table; the host tables are each other's
inverse with no id 0 and no empty host; every ticket's host id is 0 or
known).

**Tests.** `tests/records_oracle.rs` runs generated scripts of the VSP
(ticket set, get, delete, by fee status; host and public key gets),
treasury (all twelve) and agenda (all four) operations through
dcrwallet's own udb via the `oprunner` oracle and through an interpreter
over these tables standing where `purse-udb` will: 12 scripts by default,
857 operations equal (87 of them errors, empty hosts and pi keys among
them); a run of 80 scripts at another seed, 5,835 equal. The store's
host ids, values, deletes and listings agree with Go's on every one;
the error texts and result shapes are the interpreter's.
`tests/conformance.rs` holds the unspent multisig listing against bbolt's
walk of dcrwallet's `mu` keys (40 key sets of up to 400 outpoints, the
indexes chosen so that big-endian order matters), and runs the named
cursor cases and a corpus with the cursor on each of the twelve tables.
`tests/records.rs` holds the host ids, both indexes through every
transition, each refusal writing nothing, each broken invariant caught by
both checks, and the imported-address and voting-account rows.
`tests/encodings.rs` round-trips and orders every new key, round-trips
every new value, feeds every decoder random bytes and pins the layouts
and decode errors. The model-based test runs random record operations in
committed and abandoned transactions with power cuts and reopens around
them, and checks after each both the store's `check_records` and the
tests' own checker on the model (`tests/support/records.rs`, which reads
the raw rows from the documented layouts). The power-loss sweep's
workload writes, moves and deletes VSP tickets, multisig outputs and
treasury policies through the operations (26 commits, 482 storage
operations), and every reopened image passes both record checks. The
migration sweeps carry record rows and a dummy step that changes both
indexes, and the fixture holds a row in every table. The `store_decode`
and `store_open` fuzz targets reach the new types and `check_records`.
After the last change to the record types (the multisig block's
constructor, below), each was run once for 60 seconds on an x86_64 Linux
desktop as CI's smoke job runs it: `store_decode` with debug assertions
(23,668,733 inputs), `store_open` without them (31,496 inputs), no crash
in either; an earlier one-minute run of `store_decode` on the new types
left no log. All on an x86_64 Linux desktop only.

**Not done.** No measurement: nothing in the brief's exit names these
records, and what a VSP ticket or multisig write costs (two tables each)
was not measured. The multisig differential against Go is the order test
alone: `oprunner`'s multisig operations need transaction records and
imported scripts, which belong to the transaction store (Phase 3). No
frozen vectors were checked in for the records differential. The logical
export (ADR-0001) of these records is `purse-udb`'s, not built.

**Open points**, each for the owner, with a recommendation:

- R1. *Accept the twelve tables, their layouts and operations as built in
  schema version 1.* Recommended: accept.
- R2. *No `vsphostindex` counter row and no seeded host 0.* A port of
  dcrwallet's udb-level `SetVSPHost`, which no wallet code calls outside
  `SetVSPTicket`, could write an id out of order, after which dcrwallet's
  counter and the last key would give different next ids. Recommended:
  accept; `purse-udb` ports `SetVSPHost` and `SetVSPPubKey` only through
  `add_vsp_host`, and its operation-script generator does not draw them
  alone.
- R3. *The `vsp_fee_status` index* (one more row written per ticket
  change, against a walk of every VSP ticket per query). Recommended:
  keep it; the cost was not measured and is one small row in a commit
  that writes the ticket anyway.
- R4. *The multisig stake-tree flag after a spend and unspend*
  (QK-0054, candidate). The store can hold either outcome. Recommended:
  decide in Phase 3 with an oracle run of `redeemmultisigouts`, under
  ADR-0012 Rule 1 (the tree reaches a client in that result).
