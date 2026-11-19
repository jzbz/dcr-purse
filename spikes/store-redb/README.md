# store-redb — the D2 storage spike

A Phase 0 spike for decision D2 (brief §9, §6 Phase 0 and Phase 2): a
prototype of dcr-purse's own wallet store on redb, measured against
dcrwallet's `wallet/udb` over bbolt on the same generated workload, with a
correctness cross-check, a consistency check, a crash check and a power-loss
sweep. Its findings are in
[ADR-0003](../../docs/adr/0003-wallet-store-engine-and-schema.md).

This is throwaway code. It is a workspace of its own (the root manifest
excludes `spikes/`), with its own `Cargo.lock`, and nothing in the main
workspace depends on it.

## What is here

| path | contents |
|---|---|
| `src/workload.rs` | the deterministic workload generator and its file format, which the Go baseline reads too |
| `src/chain.rs` | the script and transaction shapes the generator writes; simnet maturity rules |
| `src/store.rs` | the prototype schema: tables, encodings, ingest (`InsertMinedTx`, `InsertMemPoolTx`, `moveMinedTx`, `AddCredit`, commitments, address bookkeeping), stake invalidation and validation of the parent block, `Rollback`, incremental balances, the unspent index with bbolt cursor semantics, `MakeInputSource`, `RangeTransactions`, the consistency check, the schema version check |
| `src/export.rs` | the canonical export both sides write for the cross-check |
| `src/bench.rs` | the Rust half of the timings, header ingest |
| `src/powerloss.rs` | a power-loss storage backend for redb 5 (a port of dcroxide's) |
| `src/bin/spike.rs` | the driver: `gen`, `run`, `crash`, `repair-time`, `powerloss`, `headers` |
| `tests/spike.rs` | quick tests (tiny workload) |
| `../../tools/oracle/cmd/storebench` | the Go baseline: dcrwallet's own `udb` at the parity pin |

## Rerunning

The toolchain is the repository's (`rust-toolchain.toml` at the root
applies here). The Go baseline builds with the oracle's Go; without Go,
`run` measures the prototype alone and the tests that need Go skip (with
`PURSE_REQUIRE_ORACLE=1` they fail instead).

Quick checks (seconds in a release build, longer in a debug one):

```sh
cargo test
```

The measured runs write everything under the directory given with `--dir`
(generated workloads, both databases, exports, results); use a directory
outside the repository, with a few GiB free for the `1m` preset:

```sh
cargo run --release -- run --dir "$OUT" --preset 10k --rounds 2
cargo run --release -- run --dir "$OUT" --preset 100k --rounds 2
cargo run --release -- run --dir "$OUT" --preset 1m --rounds 2
cargo run --release -- repair-time --db "$OUT/1m/redb-5.0.0/rust/wallet.redb"
cargo run --release -- crash --dir "$OUT" --kills 300
cargo run --release -- powerloss --dir "$OUT" --step 1
cargo run --release -- powerloss --dir "$OUT" --step 1 --torn
cargo run --release -- powerloss --dir "$OUT" --step 1 --relaxed 10
cargo run --release -- powerloss --dir "$OUT" --step 7 --dishonest
cargo run --release -- headers --dir "$OUT" --n 1050000 --filter-len 256
```

`run` generates `$OUT/workload-<preset>.bin` (and a `.json` describing it)
unless it exists. It creates the prototype store and records the workload's
addresses; creates the dcrwallet wallet through the oracle and records the
same addresses (checking that dcrwallet derives the same HASH160 for every
one); keeps a copy of each store as setup left it; then ingests the
workload from those copies `--rounds` times, Go then Rust in each round, so
that both sides' ingest figures come from one session. After the last round
it runs the consistency check on the prototype, writes both exports and
compares them line by line, then times the queries on each side in turn,
the opens cold (after dropping the file from the page cache with `dd
iflag=nocache`, so Linux only) and warm, a hot backup on each side with and
without a concurrent writer, and a compaction of a copy of the redb file.
The Go wallet lives in `$OUT/<preset>/go/`; everything of the prototype's,
the exports and the results (`results.json`, `summary.md`) go to
`$OUT/<preset>/redb-<version>/`. `--no-bench` stops after the cross-check,
`--no-go` skips the baseline, and `--reuse-go` keeps the Go wallet (and its
recorded setup and ingest results) from an earlier run of the same preset.

`crash` ingests the `tiny` workload into a reference store, recording a
digest of every table at every transaction boundary, then repeatedly
spawns a writer on a victim store and kills it (`Child::kill`: SIGKILL, or
TerminateProcess on Windows) at a random moment within `--max-ms` (40)
milliseconds, timed from the spawn for one kill in four and from the
writer's report that the store is open for the rest. After each kill it
reopens the store and requires that it is at a transaction boundary, holds
every transaction the writer reported committed, equals the reference at
that boundary, and passes the consistency check. `crash.json` splits the
kills into those before the store was open, those inside the first write
transaction and those after a reported commit. A kill leaves the page
cache intact, so it cannot tear or drop a write; `powerloss` can.

`powerloss` runs the `tiny` workload on an in-memory storage backend that
counts every write, length change and sync, and, for every `--step`th
count, lets the run proceed to that operation, cuts the power there
(discarding every write since the last sync, or with `--torn` a random part
of them at 4 KiB granularity), reopens the image and applies the same checks
as `crash`, against the last durable commit. `--relaxed K` commits with
`Durability::None` except every Kth transaction; `--dishonest` acknowledges
syncs without making anything durable, which the sweep must catch.

`headers` measures header-plus-filter ingest (SPV initial sync, and the
part of a rescan from scratch before any transaction): `--n` generated
headers with committed filters of about `--filter-len` bytes, `--batch`
(2,000) per write transaction, on both sides, twice; the prototype once
with every commit durable and once with only every `--relaxed`th.

`repair-time` kills a writer on an existing store and times the reopen,
which then has to repair the file.

## Presets

| preset | target transactions | blocks | accounts | blocks at the tip | blocks per sync transaction | left unmined |
|---|---:|---:|---:|---:|---:|---:|
| `tiny` | 600 | 300 | 3 | 100 | 50 | 12 |
| `10k` | 10,000 | 50,000 | 5 | 2,000 | 2,000 | 30 |
| `100k` | 100,000 | 100,000 | 8 | 2,000 | 2,000 | 60 |
| `1m` | 1,000,000 | 200,000 | 10 | 2,000 | 2,000 | 200 |

The generator draws receives, sends (one to three inputs; change to the
same account, to another account in about one send in seventeen, none in one in
twenty-five), consolidations, ticket purchases (half with ticket change),
votes by mature tickets and coinbases paying the wallet, across
Zipf-weighted accounts, with fresh or reused receive addresses and a fresh
change address per send. Transactions arrive with their block during
history sync. In the blocks at the tip, nine in ten non-stake transactions
are first inserted unmined and mined one to three blocks later; the last
few are never mined, about a third of the sends among them unpublished.
Every eight blocks at the tip one chain event runs, in rotation: a block
disapproving its parent (the parent's regular transactions mined again in
the same block, again later, or never); a disapproval followed by a
one-block reorganisation to a block that approves the parent (stake
validation after a rollback); and reorganisations of one, two and three
blocks (coinbases dropped, the last block's regular transactions sometimes
left unmined for a block). The final blocks carry coinbases at depths 1,
15, 16 and 17, votes at depths 1 and 17 and ticket change at depths 1 and
2, so the export's balances cross every maturity boundary. Signature
scripts are random bytes of a P2PKH signature script's shape: neither side
validates them.

## The export

One line per item, identical on both sides:

```text
tip <height> <hash hex, internal order>
keyn <keys in the unspent index>
account <n> <last used ext> <last used int> <last returned ext> <last returned int>
balance <minconf> <account> <spendable> <immature coinbase> <immature stake> <locked> <voting authority> <unconfirmed> <total>
unspent <count>
utxo <tx hash> <index> <tree> <height> <amount> <coinbase> <pkScript>
probe <key> <seek> <next> <next> <next> <prev> <prev> | <prev> <prev> <prev> <next> <next>
scan <key> next <keys> prev <keys>
range <name> <begin> <end> <limit>
batch <n>
tx <hash> <height> c<index>:<amount>:<spent>:<change>... d<index>:<amount>...
by_account <name> <account> <limit> <hashes>
invalid <height> <block hash>
inv_credit <tx hash> <height> <index> <amount> <spent>
inv_debit <tx hash> <height> <input> <amount> <spent tx hash> <spent index>
foreach <count>
fe <tx hash> <index> <tree>
select <account> <minconf> <target> <inputs> <total> <SHA-256 of the sorted input lines>
sel <tx hash> <index> <tree> <amount>
addrs <account> <count> <SHA-256 of the HASH160s in ForEachAccountAddress order>
```

Accounts are in the address manager's iteration order, balances in
ascending account order for the accounts `AccountBalances` reports, unspent
outputs in `UnspentOutputs` order, ranges as `RangeTransactions` calls back.
Probes are bbolt cursor walks on the unspent bucket (spec 04 §5.1) from
random keys, existing keys and their neighbours, the empty key, the
extremes and the first and last three keys; scans are `randomUTXO`'s
collection step in each direction from a key, with `RandomUTXO`'s skip at
one confirmation. `by_account` lists an account's whole history at the two
smaller sizes and its 500 most recent transactions at 1m. The invalidated
rows are those of main-chain blocks: dcrwallet leaves the rows of a
detached disapproved block in its `ic` and `id` buckets, where nothing
reads them, and the prototype deletes them. `fe` lines are
`ForEachUnspentOutpoint`. `select` runs `MakeInputSource` for every
account at minconf 0, 1 and 6 with target 0 (the eligible list, no random
phase) and with a target no balance reaches (every eligible output whatever
the random draws), so the result does not depend on randomness, which
dcrwallet draws from `crypto/rand` and cannot be given; `sel` lines list
the inputs at the two smaller sizes. The probe and scan keys and the ranges
are drawn by the driver and sent to both sides.
