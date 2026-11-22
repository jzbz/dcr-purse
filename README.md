# dcr-purse

A from-scratch Rust implementation of the Decred wallet daemon, built as a
drop-in replacement for [dcrwallet](https://github.com/decred/dcrwallet),
including SPV, StakeShuffle mixing and staking.

Parity target: **dcrwallet v2.2**, which upstream has not yet released: until
it is tagged, the pin follows dcrwallet master, now **`54b85b33`** (version
2.2.0-pre) — JSON-RPC API 11.0.0, gRPC API 10.0.0.
dcrwallet's behaviour at the pinned commit is the specification. The plan is
in [dcr-purse-project-brief.md](docs/dcr-purse-project-brief.md), the 25
specifications it rests on are in [docs/specs](docs/specs/README.md),
[PARITY.md](docs/PARITY.md) maps every dcrwallet package to its crate and
status, [QUIRKS.md](docs/QUIRKS.md) lists the upstream behaviours that are
reproduced or deliberately not, [bench-ledger.md](docs/bench-ledger.md)
records the performance measurements, and [UPSTREAM.md](docs/UPSTREAM.md)
holds dated notes on what dcrwallet has done past the pin.

**Status: early. This is not a wallet yet, and nothing here should hold
funds.** The first libraries exist, the wallet's store among them with no
wallet logic over it yet; there is no daemon, no sync and no RPC server.
Phase 0 (the oracle rig and decisions D1 to D10) was closed on
2026-11-22 ([PHASE0-EXIT.md](docs/PHASE0-EXIT.md)), and Phases 1 and 2
on 2026-11-21 ([PHASE1-EXIT.md](docs/PHASE1-EXIT.md),
[PHASE2-EXIT.md](docs/PHASE2-EXIT.md)); each record lists what it leaves
to later phases. Phase 0's closing followed the project owner's
decisions of 2026-11-22 on the last open points of D1 to D10, among them
ADR-0019's FB1 to FB13 and ADR-0013's remaining open points, and a rule
that questions arising later in working out ADR-0019 are settled in the
phase that implements it.

## What "drop-in" means

Everything outside the process matches dcrwallet: the network (a dcrd
JSON-RPC client, an SPV peer at protocol 11, a VSP client, a mixing peer),
both RPC APIs with their exact results and error text, the command line and
the configuration options. The same seed gives the same keys and addresses
and, once the wallet has synced, the same balances and history. dcr-purse
runs under its own name, with its own data directory (`~/.dcr-purse` on Unix),
configuration file (`dcr-purse.conf`) and log (`dcr-purse.log`), and names
itself to peers in its P2P user agent (`/dcrwire:1.0.0/dcr-purse:0.1.0/`),
while the JSON-RPC `version` reply keeps dcrwallet's `dcrwallet` key
([ADR-0005](docs/adr/0005-name-data-directory-and-import-front-end.md)).

The database is the exception. dcr-purse keeps its own format (project
owner, 2026-11-14) and neither opens nor imports dcrwallet's `wallet.db`
(2026-11-18, ADR-0013). A wallet is created new or restored from its seed;
a restore runs account and address discovery and a rescan, as dcrwallet's
does, which takes time. Everything not derived from the seed is not
recovered, and must be re-entered by hand if wanted: imported private keys,
scripts and extended public keys; voting accounts; account names (accounts
are rediscovered by number); per-account passphrases; mixed-account
configuration; agenda, tspend and treasury-key voting preferences; VSP
ticket records and fee-payment state; unmined and unpublished transactions;
outpoint locks; the last-returned address counters as dcrwallet keeps them,
so an address handed out but never paid can be handed out again; and the
birthday, unless one is given at the restore prompt, so that the rescan
starts from genesis (the last two added on 2026-11-22, ADR-0013 Open point
8). A user who no longer has the seed cannot move to dcr-purse,
and coins beyond the discovery gap limit are found only once it is raised.
Under Decrediton, the wallet is created or restored through Decrediton's
own flow. ADR-0013 records the decision and these costs. The store is
also to keep the committed filters the wallet fetches only from the
wallet's birthday on, where dcrwallet keeps one for every block, which
makes a new wallet's file smaller by an amount not yet measured (project
owner, 2026-11-21; ADR-0019); this is not implemented yet. Under the
rules decided on 2026-11-22, a rescan or discovery in SPV mode that
starts below the birthday fetches from peers the filters the store does
not hold, about a million on mainnet for a rescan from height 0, at a
cost not yet measured.

## What exists

| Crate | Ports | State |
|---|---|---|
| `purse-errors` | dcrwallet `errors`: kinds, the Op/Kind/Err model, the rendering that reaches RPC clients | implemented, differential against Go |
| `purse-seed` | `pgpwordlist`, `walletseed`: PGP word list mnemonics, hex and mnemonic seed input, seed generation | implemented, differential against Go |
| `purse-hdkeychain` | dcrd `hdkeychain` v3.1.3: extended keys with Decred's stored-stripped hardened derivation | implemented, differential against Go |
| `purse-boltread` | the read path of bbolt v1.3.12: a read-only, `mmap`-free reader of the format of dcrwallet's `wallet.db`, hardened against corrupt files. A test utility only, not shipped in the wallet and no longer the foundation of an importer: tests use it to read databases that dcrwallet wrote. The owner may later remove it (ADR-0013 Open point 2) | implemented, differential against Go |
| `purse-snacl` | dcrwallet `snacl` and `kdf`: scrypt and secretbox key sealing, Argon2id and XChaCha20-Poly1305 for account passphrases | implemented, differential against Go |
| `purse-dcrutil` | dcrd `dcrutil` v4.0.3: `Amount` with Go's float conversion, WIF, transaction sorting | implemented, differential against Go |
| `purse-txrules` | dcrwallet `wallet/txsizes` and `wallet/txrules`: worst-case transaction sizes, the relay fee, dust, output checks, the high-fee guard and the VSP fee | implemented, differential against Go |
| `purse-txauthor` | dcrwallet `wallet/txauthor`: the fee, input and change loop over abstract input and change sources, change-position randomization drawn through dcroxide's generator (process-wide, or a caller's seeded `Prng`), and the input-signing loop | implemented, differential against Go |
| `purse-mixsolver` | the StakeShuffle root solver over F = 2¹²⁷ − 1, in place of dcrwallet's external `csppsolver` | implemented, differential against the FLINT-backed reference |
| `sntrup4591761` | `companyzero/sntrup4591761`: the Streamlined NTRU Prime 4591^761 KEM of the mixing key exchange, byte-compatible down to the randomness it draws | implemented, differential against Go; 13 to 44 times faster than Go, constant time |
| `purse-store` | the wallet's own store on redb 5 in place of `walletdb` and bbolt (ADR-0003): every table of the schema with documented encodings, main-chain headers and filters keyed by height and detached ones by hash, typed transactions, bbolt-ordered cursors, schema versioning and migrations, relaxed durability confined to initial sync | Phase 2 closed by the project owner on 2026-11-21, its five exit clauses held ([PHASE2-EXIT.md](docs/PHASE2-EXIT.md); what it lists as left carried to later phases): cursors conform to bbolt through the oracle; a power cut at every storage operation, a model-based property test and a migration cut at every write pass (brief Phase 2's exit tests); the multisig, VSP, agenda and treasury records (spec 05's aux buckets) have their tables and typed operations; the VSP, agenda and treasury records are checked against dcrwallet's udb through the `oprunner` oracle, the multisig records only for the one order a client sees, against bbolt through `boltcursor` (their udb differential waits for Phase 3's transaction records); voting accounts and imported keys and scripts are rows of the existing tables; the backup of ADR-0018 is built as library functions (snapshot copy, backup record, publication through `.partial/`, a verify path that never trusts the file), files created owner-only on Unix as dcrwallet's are, tested with a power cut at every storage operation of a backup, a killed verify and damaged and foreign files; its JSON-RPC method and tool come in Phases 7 and 8; measured against dcrwallet's udb on the same wallet (100,075 transactions), with the transaction-store rows written by the test until Phase 3: balances 36,800 to 102,500 times faster, ranges over blocks 1.86 to 2.05 times, address and account lookups 1.8 to 4.0 times, commits 1.6 to 1.9 times, on a shared desktop ([bench-ledger.md](docs/bench-ledger.md), 2026-11-20); the store's file name is `wallet.redb` (project owner, 2026-11-21) |
| `purse-udb` | dcrwallet `wallet/udb` over `purse-store`: the address manager and, later, the transaction store, plus the Rust producer of the logical export (ADR-0001) | in progress: the address manager's creation, opening and wallet lock, the coin type (its private key and the SLIP0044 upgrade), its accounts (BIP0044 and imported xpub), addresses (sync, recording, lookups, private keys, the used and returned indexes), imported keys and scripts, and `Encrypt`/`Decrypt` pass the operation-script differential against Go on all four networks and both coin types (results and masked exports; the divergences recorded in `docs/PARITY.md` are pinned on both sides), with frozen vectors, the reachable upstream tests and property tests; watching-only wallets, voting accounts, per-account passphrases and passphrase changes are listed as not written in its crate documentation |
| `purse-export-check` | — | the logical export's strict parser and conformance tests (ADR-0001), and the operation-script corpus run through the Go runner (ADR-0002); a test utility until the store exists |
| `purse-testutil` | — | the oracle harness, the version-skew checks and test helpers |
| `purse-bench` | — | the benchmark harness of [ADR-0014](docs/adr/0014-performance.md): dcr-purse's workloads against their Go counterparts (`tools/oracle/cmd/bench`) on the same inputs, alternating in one session; not shipped. Results: [bench-ledger.md](docs/bench-ledger.md). Phase 1's speed row met on every clause on the dedicated machine on 2026-11-21, and Phase 1 closed that day ([PHASE1-EXIT.md](docs/PHASE1-EXIT.md)) |

Decred primitives (BLAKE-256, RIPEMD-160, secp256k1 through `dcroxide-dcrec`,
chain parameters, wire, txscript, the subsidy functions and, later, stake, gcs
and the rest) come from
[dcroxide](https://github.com/jzbz/dcroxide) at one pinned revision.

## How correctness is established

dcrwallet has few tests for most of what must match, so almost nothing here
is pinned by a hand-written expectation.

- **The oracle.** `tools/oracle` is a Go module pinned to dcrwallet at the
  parity commit. Each directory under its `cmd/` is a small binary that links
  dcrwallet's own packages (or the exact dependency versions it links) and
  answers requests over stdin and stdout, one JSON object per line. Rust tests
  build a binary on demand and compare their answers with Go's on generated
  inputs.
- **Frozen vectors.** Each crate also checks in vectors generated by the
  oracle, so the suite has pinned coverage without a Go toolchain. Each file
  records how it was produced.
- **Ported upstream tests**, where upstream has any.
- **Fuzzing.** `fuzz/` holds cargo-fuzz targets for the parsers.
- **Wallet state, from the store on.** dcr-purse's database and dcrwallet's
  share no format, so wallet state is compared through a logical export that
  both sides write (ADR-0001), after both have reached the
  same state by running the same operation script (ADR-0002) or by restoring the same seed against the same chain. No
  comparison imports a Go-written database.

Without Go, differential tests skip with a notice. With
`PURSE_REQUIRE_ORACLE=1`, as in CI, a missing oracle is a failure; an oracle
that is present and does not build is always a failure.

## Building and testing

The toolchain is pinned in `rust-toolchain.toml`. The oracle builds with Go
1.26 or later; where it is required, it must be built with the reference Go,
1.27.4.

```sh
cargo test --workspace
PURSE_REQUIRE_ORACLE=1 cargo test --workspace
```

Environment variables the tests read:

| Variable | Effect |
|---|---|
| `PURSE_REQUIRE_ORACLE` | a missing Go toolchain fails differential tests instead of skipping them, and so does an oracle built with any Go release but the reference one |
| `PURSE_ORACLE_DIR` | use prebuilt `purse-oracle-<name>` binaries from this directory instead of building them |
| `PURSE_TEST_SEED` | run seed for generated test inputs (decimal or `0x` hex); unset, every test uses the same fixed inputs on every run; each test mixes it with its own label, so tests with different labels start from different states; every run prints the value to set to reproduce it; a value recorded before the run seed was mixed with each test's label (2026-11-19) no longer reproduces its inputs, which the project owner accepted |
| `PURSE_HD_SEEDS` | number of random seeds in the `purse-hdkeychain` differential |
| `PURSE_ERRORS_PROGRAMS` | number of random programs in the `purse-errors` differential |
| `PURSE_SNTRUP_CASES` | multiplies the random instances of the `sntrup4591761` differential and property tests |
| `PURSE_SNTRUP_HEAVY` | runs the `sntrup4591761` differential and property tests at their optimised-build sizes in a debug build |
| `PURSE_SNTRUP_BENCH_REPS` | repetitions per round of the ignored `sntrup4591761` timing comparison against Go (`speed_against_go`, default 41) |
| `PURSE_SNTRUP_DUDECT_SAMPLES` | samples per class of the ignored `sntrup4591761` timing-independence check (`secret_independence`, default 20000) |
| `PURSE_TXRULES_SCALE` | size of the random parts of the `purse-txrules` differential corpora (default 10) |
| `PURSE_TXRULES_CASES` | multiplier of the random instances in the `purse-txrules` property tests (default 1) |
| `PURSE_TXAUTHOR_SCALE` | size of the random parts of the `purse-txauthor` differential corpora (default 10) |
| `PURSE_TXAUTHOR_CASES` | multiplier of the random instances in the `purse-txauthor` property tests (default 1) |
| `PURSE_UDB_SCRIPTS` | number of generated operation scripts in the `purse-udb` differential against Go (default 24; all four networks, simnet most often, and half of them upgraded to the SLIP0044 coin type) |
| `PURSE_UDB_CASES` | number of random wallets or generated scripts in each `purse-udb` property test (default 24) |
| `PURSE_STORE_BENCH_DIR` | directory for the files of the ignored store measurements (`purse-store`'s `tests/measure.rs`, `purse-udb`'s `tests/measure.rs`); required by them, and outside the repository |
| `PURSE_UDB_MEASURE_BLOCKS`, `PURSE_UDB_MEASURE_ROUNDS` | chain length (default 40,000 blocks) and rounds per side (default 3) of the ignored measurement of `purse-store` against dcrwallet's udb (`purse-udb`'s `tests/measure.rs`) |
| `PURSE_REQUIRE_FLINT` | the `purse-mixsolver` comparison against the FLINT-backed reference solver fails instead of skipping when that oracle is missing; it needs FLINT installed and the oracle built with `-tags flint` into `PURSE_ORACLE_DIR` |

Fuzz targets build on nightly with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz):

```sh
RUSTUP_TOOLCHAIN=nightly cargo fuzz run --fuzz-dir fuzz seed_decode
```

## Benchmarks

Speed is measured against Go on the same machine in one session
([ADR-0014](docs/adr/0014-performance.md)); results are entries in
[bench-ledger.md](docs/bench-ledger.md), under the rules at its top. The
driver builds the Go side from `tools/oracle`, runs the two sides
alternately (at least 3 processes each), checks that both produced the same
outputs, and prints the table; the raw output goes to the directory named,
which must be new or empty:

```sh
cargo build --release -p purse-bench
target/release/purse-bench drive --out <new directory> --runs 5 --cpus 12-15 --go-tags flint
```

`--cpus` pins every measured process with `taskset -c` (Linux);
`--go-tags flint` adds the mixing solver's Go reference, which needs FLINT
installed (see `tools/oracle/cmd/mixsolver`); `--only` takes comma-separated
substrings of workload names; `--smoke` runs each workload once at reduced
sizes, which the test suite does. The CI gate is the same driver with
`--gate crates/purse-bench/gate-baseline.tsv`: it fails when a ratio of
Rust's time to Go's is more than twice its baseline. The workloads and
their criteria are listed in `crates/purse-bench/src/lib.rs`.

The store is measured against dcrwallet's own database code (`wallet/udb`
over bbolt, through `tools/oracle/cmd/storebench`) by an ignored test that
builds the same wallet on both sides, checks that their exports and every
query's answer agree, and alternates the two sides in one session (ADR-0003,
addendum "`purse-store` against dcrwallet on the same workload"):

```sh
PURSE_STORE_BENCH_DIR=<new directory> PURSE_REQUIRE_ORACLE=1 taskset -c 12-15 \
  cargo test --release -p purse-udb --test measure -- --ignored --nocapture --test-threads 1
```

## Moving the pin

The pin moves under [ADR-0016](docs/adr/0016-upstream-tracking.md): until
dcrwallet tags v2.2.0, to master's head when dcrwallet tags a point
release, when master fixes a defect on ADR-0017's list or a
security-relevant one, when a phase about to start needs a change master
made, or on the owner's word; from v2.2.0, to every release tag of the
newest line. A move is one change, made by an engineer and signed by the
owner, following the checklist of ADR-0016 item 5. The acceptance releases
that move with it (dcrd and its `dcrctl`, vspd, Decrediton) are recorded in
the table at the head of [PARITY.md](docs/PARITY.md).

`tools/pinmove` is the tooling for it, a Go module of its own that reads a
dcrwallet clone and never writes to it
([its README](tools/pinmove/README.md)):

```sh
git clone https://github.com/decred/dcrwallet.git ../dcrwallet
cd tools/pinmove
go run . report    -upstream ../../../dcrwallet -to origin/master -o report.md
go run . batteries -upstream ../../../dcrwallet -to origin/master
go run . cite      -upstream ../../../dcrwallet -to origin/master      # list; -w rewrites
```

`report` is the review list a move starts from: the upstream commits in
the range, the files and lines changed, every `path:line` citation in this
README, the Markdown files under `docs/` and the comments of the Rust and
Go files under `crates/`, `fuzz/` and `tools/oracle` (the oracle's copies
of upstream source left out) that resolves to an upstream file, classified
as unchanged, shifted, touched (to re-read) or gone, the QUIRKS entries
citing changed files, the `go.mod` delta (replace directives included),
the frozen files and oracle copies to regenerate, and the text naming the
old pin. A line citation of a name that is a file both upstream and here
(dcrwallet's `README.md`, `go.mod`) is classified against the upstream
file and listed, never rewritten.
Citations that resolve to no one upstream file (a bare name several files
share, a `:N` after prose) are counted but not classified; `pinmove
survey` lists them.
`batteries` lists every frozen file under `crates/*/tests/data`, how it
names the pin and which regenerator a range requires, by the packages each
oracle command links. `cite -w` rewrites the shifted citations in the
specs, QUIRKS.md, PARITY.md, SKEW.md, the brief and this README, except
those whose enclosing declaration differs at the new revision, lists the
rest, the citations in code comments among them, which are corrected by
hand, and leaves a `pinmove-cited-at` comment in each document it covers
so that a second run skips it; ADRs keep the citations of the pin in force
on their date and are never rewritten. `cite` runs at ADR-0016 item 5.8,
after item 5.3 has bumped `DCRWALLET_PARITY_COMMIT`, which `-from`
defaults to, so it is given the old pin as `-from`; it refuses an empty
range, and `report` flags one. Once a month the workflow `upstream.yml`
runs the report against dcrwallet's master and newest release tag and
publishes it as a job artifact; what it shows is written into the dated
read-ahead notes of [UPSTREAM.md](docs/UPSTREAM.md), which are not parity
claims.

## Conventions

- `unsafe` is forbidden in every crate, and every public item is documented.
- No hand-rolled cryptography for standard algorithms; pure Rust only.
- Seeds, private keys and passphrases are wiped on drop and never formatted.
- Where dcrwallet's behaviour is a defect an outside observer could depend
  on, it is reproduced and recorded in [QUIRKS.md](docs/QUIRKS.md). The
  standing exceptions are those of
  [ADR-0012](docs/adr/0012-standing-quirk-policy.md): where Go panics on
  external input an error is returned; where Go's order is random, replies
  are sorted, while orders other parties see on the wire stay shuffled;
  anything that would lose funds or leak secrets is fixed, and so, by
  default, is a defect that leaves funds unavailable until a restart or a
  manual unlock; and where Go's
  answer depends on the CPU, one answer is given on every platform. Each
  divergence is listed in [PARITY.md](docs/PARITY.md).

## License

ISC, as dcrwallet, dcrd and dcroxide. Ported code and vectors keep their
upstream copyright notices; see the `NOTICE` files in the crates that carry
third-party attribution.
