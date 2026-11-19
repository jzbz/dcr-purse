# Parity ledger

Maps every dcrwallet package, and every linked dependency the port has to supply, to its planned dcr-purse crate, its specification and its status. Parity pin: dcrwallet master `54b85b33` (2.2.0-pre). Crate names are provisional (project brief, decision D1).

Status values: **not started**, **in progress**, **ported** (implemented, upstream tests ported), **pinned** (differential coverage against the Go oracle in CI), **n/a** (deliberately not ported, with the reason).

The first libraries are implemented: `errors`, `pgpwordlist` and `walletseed`, dcrd's `hdkeychain`, and the read path of bbolt. Everything else is not started. The rows marked "pinned" below have their upstream tests ported and a differential against the Go oracle that CI runs on Linux (x86-64 and arm64), macOS and Windows, with frozen Go-generated vectors beside it.

## dcrwallet packages

| dcrwallet package | Go lines (non-test) | dcr-purse crate | Spec | Upstream tests | Status |
|---|---|---|---|---|---|
| `errors` | ~360 | `purse-errors` | [22](specs/22-aux-packages-tools.md) | yes (small) | pinned |
| `pgpwordlist`, `walletseed` | ~680 | `purse-seed` | [22](specs/22-aux-packages-tools.md), [20](specs/20-grpc-other-services.md) | yes | pinned; its error type is local and not yet bridged to `purse-errors` |
| `kdf`, `wallet/internal/snacl` | ~250 | `purse-snacl` | [02](specs/02-address-manager.md) | snacl only | not started |
| `wallet/walletdb`, `wallet/internal/bdb`, `wallet/drivers/bdb` | ~450 | `purse-boltread` (read-only, for import); interface replaced by `purse-store` | [01](specs/01-walletdb-bbolt.md) | yes | `purse-boltread` pinned (no upstream tests apply to a reader; differential against bbolt itself); write path n/a (own format); `purse-store` not started |
| `wallet/udb` — address manager | ~5,300 | `purse-udb` (logic), `purse-import` (decoders) | [02](specs/02-address-manager.md) | partial | not started |
| `wallet/udb` — transaction store formats | ~3,100 | `purse-import` (decoders) | [03](specs/03-txstore-formats.md) | none on raw bytes | not started |
| `wallet/udb` — transaction store logic | ~4,400 | `purse-udb` | [04](specs/04-txstore-behavior.md) | thin | not started |
| `wallet/udb` — upgrades, migration | ~1,800 | — | [05](specs/05-db-migrations-aux-buckets.md) | yes (fixtures) | n/a: schema 26 is the only importable version |
| `wallet/udb` — VSP, treasury, agenda buckets | ~650 | `purse-udb`, `purse-import` | [05](specs/05-db-migrations-aux-buckets.md), [03](specs/03-txstore-formats.md) | vsp only | not started |
| `wallet/txsizes`, `wallet/txrules` | ~450 | `purse-txrules` | [10](specs/10-tx-construction.md) | yes | not started |
| `wallet/txauthor` | ~190 | `purse-txauthor` | [10](specs/10-tx-construction.md) | yes | not started |
| `wallet` — `wallet.go` | 5,828 | `purse-wallet` | [06](specs/06-wallet-core-part1.md), [07](specs/07-wallet-core-part2.md), [08](specs/08-wallet-core-part3.md) | two tests | not started |
| `wallet` — `notifications.go` | 988 | `purse-wallet` | [08](specs/08-wallet-core-part3.md) | none | not started |
| `wallet` — `createtx.go`, `addresses.go`, `multisig.go`, `utxos.go` | ~3,400 | `purse-wallet`, `purse-stake` | [10](specs/10-tx-construction.md), [11](specs/11-staking-vsp.md) | none | not started |
| `wallet` — `chainntfns.go`, `rescan.go`, `discovery.go`, `sidechains.go`, `difficulty.go`, `checkpoints.go`; `validate`; `deployments` | ~4,300 | `purse-chain` | [09](specs/09-chain-sync-rescan-discovery.md), [06](specs/06-wallet-core-part1.md) | thin | not started |
| `wallet` — `vsp.go`, `vspticket.go`, `feepayment.go`, `vspclientloader.go`, `tickets.go` | ~1,500 | `purse-vsp`, `purse-stake` | [11](specs/11-staking-vsp.md) | none | not started |
| `wallet` — `mixing.go` | 597 | `purse-wallet` (mixing module) | [13](specs/13-wallet-mixing-integration.md) | none | not started |
| `ticketbuyer` | ~330 | `purse-ticketbuyer` | [11](specs/11-staking-vsp.md), [13](specs/13-wallet-mixing-integration.md) | none | not started |
| `p2p`, `lru` | ~2,400 | `purse-p2p` | [14](specs/14-spv-p2p.md) | none | not started |
| `spv` | ~2,750 | `purse-spv` | [14](specs/14-spv-p2p.md) | none | not started |
| `chain` | ~1,100 | `purse-rpcsync` | [15](specs/15-dcrd-rpc-sync.md) | one small | not started |
| `rpc/client/dcrd` | ~1,000 | `purse-dcrd-client` | [15](specs/15-dcrd-rpc-sync.md) | codecs only | not started |
| `rpc/jsonrpc/types` | ~1,800 | `purse-rpctypes` | [16](specs/16-jsonrpc-transport-types-help.md) | yes | not started |
| `internal/rpc/jsonrpc` — transport, dispatch, help; `internal/rpchelp` | ~2,100 | `purse-jsonrpc` | [16](specs/16-jsonrpc-transport-types-help.md) | one | not started |
| `internal/rpc/jsonrpc` — `methods.go` | 5,863 | `purse-jsonrpc` | [17](specs/17-jsonrpc-methods-part1.md), [18](specs/18-jsonrpc-methods-part2.md) | none | not started |
| `rpc/walletrpc` (generated), `rpc/api.proto` | 18,700 generated from 1,341 | `purse-walletrpc` | [19](specs/19-grpc-wallet-service.md), [20](specs/20-grpc-other-services.md) | none | not started |
| `internal/rpc/rpcserver` | ~4,400 | `purse-grpc` | [19](specs/19-grpc-wallet-service.md), [20](specs/20-grpc-other-services.md) | none | not started |
| `internal/loader` | ~440 | `purse-loader` | [20](specs/20-grpc-other-services.md), [21](specs/21-config-cli-lifecycle.md) | none | not started |
| package `main` — `config.go`, `internal/cfgutil`, `internal/netparams` | ~1,400 | `purse-config` | [21](specs/21-config-cli-lifecycle.md) | none | not started |
| package `main` — `log.go`, `internal/loggers` | ~180 | `purse-logging` | [21](specs/21-config-cli-lifecycle.md) | none | not started |
| package `main` — `ipc.go` | 203 | `purse-ipc` | [21](specs/21-config-cli-lifecycle.md) | none | not started |
| package `main` — `walletsetup.go`, `internal/prompt` | ~820 | `purse-prompt`, `dcr-purse` | [21](specs/21-config-cli-lifecycle.md) | none | not started |
| package `main` — `dcrwallet.go`, `rpcserver.go`, `signal*.go`, `version` | ~1,400 | `dcr-purse` (bin) | [21](specs/21-config-cli-lifecycle.md), [16](specs/16-jsonrpc-transport-types-help.md) | x509 only | not started |
| `cmd/treasurykey`, `cmd/repaircfilters` | ~280 | `purse-tools` | [22](specs/22-aux-packages-tools.md) | none | not started |
| `cmd/movefunds`, `cmd/sweepaccount` | ~650 | — | [22](specs/22-aux-packages-tools.md) | none | n/a: do not work at the pin |
| `rpc/client/dcrwallet`, `rpc/rpc_example`, `rpc/grpc_example` | ~1,700 | optional typed client for tests | [22](specs/22-aux-packages-tools.md) | small | undecided |

## Linked dependencies the port must supply

| Dependency (version dcrwallet pins) | Supplied by | Spec | Status |
|---|---|---|---|
| dcrd `hdkeychain` v3.1.3 | new `purse-hdkeychain`, over `dcroxide-dcrec` | [02](specs/02-address-manager.md), [25](specs/25-reuse-dcr-rs-and-conventions.md) | pinned |
| dcrd `dcrutil` v4.0.3 (`Amount`, WIF, `txsort`) | new `purse-dcrutil` | [24](specs/24-reuse-dcroxide.md) | not started |
| dcrd `mixing/mixclient` v0.7.5 | new `purse-mixclient` | [12](specs/12-mixclient-protocol.md) | not started |
| dcrd `mixing` key agreement | addition to `dcroxide-mixing` | [12](specs/12-mixclient-protocol.md) | not started |
| `companyzero/sntrup4591761` | new in-tree port | [12](specs/12-mixclient-protocol.md) | not started |
| `decred.org/cspp/v2` `solverrpc` and the `csppsolver` program | `purse-mixsolver` (decision D8) | [12](specs/12-mixclient-protocol.md) | not started |
| `vspd/client` v4.0.2, `vspd/types` v3.0.0 | `purse-vsp` | [11](specs/11-staking-vsp.md) | not started |
| `jrick/wsrpc` v2.3.8 + websocket client | `purse-wsrpc` | [15](specs/15-dcrd-rpc-sync.md) | not started |
| `go.etcd.io/bbolt` v1.3.12 | `purse-boltread`, read path only | [01](specs/01-walletdb-bbolt.md) | pinned |
| `jessevdk/go-flags` v1.5.0 | dcroxide's layer, taught v1.5.0 | [21](specs/21-config-cli-lifecycle.md) | not started |
| `decred/slog` v1.2.0, `jrick/logrotate` v1.0.0 | extracted from `dcroxide-node` | [21](specs/21-config-cli-lifecycle.md) | not started |
| `google.golang.org/grpc` v1.83.2 (server), `protobuf` v1.36.11 | decision D5 | [19](specs/19-grpc-wallet-service.md), [25](specs/25-reuse-dcr-rs-and-conventions.md) | not started |
| dcrd modules already ported by dcroxide | dcroxide crates, with the additions the brief lists | [24](specs/24-reuse-dcroxide.md) | reuse |

## Deliberate divergences from dcrwallet

Recorded as they are decided. So far, by decision of the project owner on 2026-11-14:

- **The database.** dcr-purse does not open, write or produce dcrwallet's `wallet.db`. It keeps its own format (engine: `redb`, pending the Phase 0 prototype) and imports a dcrwallet database at schema version 26, one-way, without modifying it. Older schema versions are refused; the 25 upgrade steps are not ported.

Divergences in the crates implemented so far. Each has a test, and none changes a value dcrwallet produces on input it accepts:

- **`purse-errors`.** Where Go's `errors.Is` panics inside `match` on a nil level ([QK-0628](QUIRKS.md)), the level does not match and the walk goes on, so `is` can be true where Go gives no answer. A typed-nil `*Error`, a kind outside the 21, and an `E()` call with no arguments cannot be written. A foreign (non-wallet) error is an opaque leaf: matching does not follow its `source()`, where Go follows a foreign `Unwrap` ([QK-0728](QUIRKS.md)). Stacks are Rust backtraces, not Go's `debug.Stack` bytes.
- **`purse-seed`.** Inputs are `&str`, so text that is not valid UTF-8 cannot reach the decoders. A failing random source is an error; Go's cannot fail. The seed words dcrwallet echoes in decode errors are echoed here too, because clients see that text. The daemon's log will not carry them, where dcrwallet's does: decided 2026-11-19, to be implemented with the gRPC server ([QK-0729](QUIRKS.md)).
- **`purse-hdkeychain`.** Keys own their bytes: `neuter` and `zero` do not alias as Go's do ([QK-0713](QUIRKS.md)), and `serialized_priv_key` lends a read-only slice instead of the key's own storage ([QK-0714](QUIRKS.md)); every dcrwallet call site that zeroes a key was traced and none depends on the aliasing. `priv_key_bytes` on the empty private form, which only a zero child sum produces, is an error where Go's `PrivKeyFromBytes` would give the zero scalar.
- **`purse-boltread`.** One non-blocking shared lock instead of bbolt's blocking wait, and on Windows a share-mode open instead of bbolt's byte-range lock. Page sizes below 1024 or not a power of two are refused; bbolt reads them, dcrwallet never writes them. When both meta pages are invalid the fallback page size is 4096, not the host's, which changes only which error is reported. On files bbolt did not write: a tree deeper than 64 levels, a page reached twice in a walk, keys out of order under a cursor, and a logical page over a configurable 256 MiB are errors; wherever Go panics or reads outside a page, an error is returned ([QK-0730](QUIRKS.md)).

Standing policy for the classes of divergence the brief's decision D10 proposes (errors in place of Go panics, deterministic order in place of map-iteration order, fixes where reproducing a defect would lose funds or leak secrets) takes effect when D10 is ratified; individual cases are then listed here with their [QUIRKS.md](QUIRKS.md) entry.
