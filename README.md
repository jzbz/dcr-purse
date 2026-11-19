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
status, and [QUIRKS.md](docs/QUIRKS.md) lists the upstream behaviours that are
reproduced or deliberately not.

**Status: early. This is not a wallet yet, and nothing here should hold
funds.** The first libraries exist; there is no daemon, no store, no sync and
no RPC server.

## What "drop-in" means

Everything outside the process matches dcrwallet: the network (a dcrd
JSON-RPC client, an SPV peer at protocol 11, a VSP client, a mixing peer),
both RPC APIs with their exact results and error text, the command line and
the configuration options. The same seed gives the same keys and addresses
and, once the wallet has synced, the same balances and history. dcr-purse
runs under its own name, with its own data directory (`~/.dcr-purse` on Unix),
configuration file (`dcr-purse.conf`) and log (`dcr-purse.log`)
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
outpoint locks. A user who no longer has the seed cannot move to dcr-purse,
and coins beyond the discovery gap limit are found only once it is raised.
Under Decrediton, the wallet is created or restored through Decrediton's
own flow. ADR-0013 records the decision and these costs.

## What exists

| Crate | Ports | State |
|---|---|---|
| `purse-errors` | dcrwallet `errors`: kinds, the Op/Kind/Err model, the rendering that reaches RPC clients | implemented, differential against Go |
| `purse-seed` | `pgpwordlist`, `walletseed`: PGP word list mnemonics, hex and mnemonic seed input, seed generation | implemented, differential against Go |
| `purse-hdkeychain` | dcrd `hdkeychain` v3.1.3: extended keys with Decred's stored-stripped hardened derivation | implemented, differential against Go |
| `purse-boltread` | the read path of bbolt v1.3.12: a read-only, `mmap`-free reader of the format of dcrwallet's `wallet.db`, hardened against corrupt files. A test utility only, not shipped in the wallet and no longer the foundation of an importer: tests use it to read databases that dcrwallet wrote. The owner may later remove it (ADR-0013 Open point 2) | implemented, differential against Go |
| `purse-snacl` | dcrwallet `snacl` and `kdf`: scrypt and secretbox key sealing, Argon2id and XChaCha20-Poly1305 for account passphrases | implemented, differential against Go |
| `purse-dcrutil` | dcrd `dcrutil` v4.0.3: `Amount` with Go's float conversion, WIF, transaction sorting | implemented, differential against Go |
| `purse-mixsolver` | the StakeShuffle root solver over F = 2¹²⁷ − 1, in place of dcrwallet's external `csppsolver` | implemented, differential against the FLINT-backed reference |
| `purse-export-check` | — | the logical export's strict parser and conformance tests (ADR-0001), and the operation-script corpus run through the Go runner (ADR-0002); a test utility until the store exists |
| `purse-testutil` | — | the oracle harness, the version-skew checks and test helpers |
| `purse-bench` | — | the benchmark harness of [ADR-0014](docs/adr/0014-performance.md): dcr-purse's workloads against their Go counterparts (`tools/oracle/cmd/bench`) on the same inputs, alternating in one session; not shipped. Results: [bench-ledger.md](docs/bench-ledger.md) |

Decred primitives (BLAKE-256, RIPEMD-160, secp256k1 through `dcroxide-dcrec`,
chain parameters and, later, wire, txscript, stake, gcs and the rest) come from
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
| `PURSE_TEST_SEED` | run seed for generated test inputs (decimal or `0x` hex); unset, every test uses the same fixed inputs on every run; each test mixes it with its own label, so tests with different labels start from different states; every run prints the value to set to reproduce it |
| `PURSE_HD_SEEDS` | number of random seeds in the `purse-hdkeychain` differential |
| `PURSE_ERRORS_PROGRAMS` | number of random programs in the `purse-errors` differential |
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
  anything that would lose funds or leak secrets is fixed; and where Go's
  answer depends on the CPU, one answer is given on every platform. Each
  divergence is listed in [PARITY.md](docs/PARITY.md).

## License

ISC, as dcrwallet, dcrd and dcroxide. Ported code and vectors keep their
upstream copyright notices; see the `NOTICE` files in the crates that carry
third-party attribution.
