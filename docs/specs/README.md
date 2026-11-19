# dcrwallet specifications

Twenty-five specifications of dcrwallet at the parity pin, master commit `54b85b33` (2.2.0-pre), each written from the Go source and its pinned dependencies. They are what the dcr-purse team implements from; the plan that orders the work is the [project brief](../dcr-purse-project-brief.md).

Every spec has the same thirteen sections: role, public surface inventory, data formats, algorithms, constants, concurrency, errors, compatibility-critical behaviours, quirks, upstream tests, Rust porting notes, open questions. Citations are `path:line` relative to the dcrwallet repository root, or `module@version/path:line` for a dependency.

## How to read them after the storage decision

The specs were written before the project owner decided, on 2026-11-14, that dcr-purse keeps its own database format on `redb` and **imports** a dcrwallet `wallet.db` at schema version 26 instead of opening it in place. They describe dcrwallet, so they remain correct, but four of them change role:

- **01** (bbolt and walletdb): the file-format sections specify the read-only importer's reader. The write path (commit, spill, rebalance, freelist allocation, file growth) and the cursor-under-mutation semantics are reference material only.
- **02** and **03** (address manager and transaction store formats): the bucket, key and value layouts specify what the importer decodes. dcr-purse's own schema is not bound by them. The key-encryption blob formats in 02 are kept, so that imported keys carry over without decryption.
- **04** (transaction store behaviour): the logic is still the contract wherever it is visible through an RPC result. Byte-level row quirks are not.
- **05** (schema versions and upgrades): the upgrade steps are not ported; version 26 is the only importable version. The auxiliary-bucket formats still specify what the importer reads.

Porting notes and decision lists inside those specs that assume a write-compatible bbolt engine or byte-equal databases are superseded by the brief.

## Verification status

Each spec got two further passes against the source after it was written: an adversarial accuracy check that corrected it in place, and a mechanical completeness check that inventoried the source with grep and added what was missing. All 25 have had both. In total the accuracy passes checked about 6480 claims and made 240 corrections, and the completeness passes made 371 additions. "Gaps left" are items the completeness pass listed as still uncovered; they are in each spec's open-questions section. None of this was verified by running code: every statement is from reading the source, and statements the specs mark UNVERIFIED or "derived from reading" need an oracle run before code depends on them.

| Spec | Subject | Lines | Status |
|---|---|---|---|
| [01](01-walletdb-bbolt.md) | 01 — Database layer: walletdb interface, the bdb driver, and the bbolt on-disk file format | 2041 | accuracy pass: 235 claims checked, 11 corrections (high confidence); completeness pass: 15 additions, 7 gaps left |
| [02](02-address-manager.md) | 02 — Address manager: keys, accounts, derivation, encryption, and the `waddrmgr` buckets | 1414 | accuracy pass: 260 claims checked, 7 corrections (high confidence); completeness pass: 8 additions, 7 gaps left |
| [03](03-txstore-formats.md) | 03 — Transaction store: bucket layout and byte formats | 2237 | accuracy pass: 330 claims checked, 7 corrections (high confidence); completeness pass: 11 additions, 6 gaps left |
| [04](04-txstore-behavior.md) | 04 — Transaction store: mined transaction logic, balances, queries | 1468 | accuracy pass: 330 claims checked, 5 corrections (high confidence); completeness pass: 9 additions, 5 gaps left |
| [05](05-db-migrations-aux-buckets.md) | 05 — Database versions, migrations, and the auxiliary buckets (VSP, treasury policy, legacy stake) | 2308 | accuracy pass: 260 claims checked, 15 corrections (high confidence); completeness pass: 14 additions, 6 gaps left |
| [06](06-wallet-core-part1.md) | 06 — Wallet core, part 1 (`wallet/wallet.go` lines 1–2393) | 2001 | accuracy pass: 235 claims checked, 8 corrections (high confidence); completeness pass: 13 additions, 6 gaps left |
| [07](07-wallet-core-part2.md) | 07 — Wallet core, part 2: queries, listings, key import/export, stake info, outpoint locks | 2331 | accuracy pass: 235 claims checked, 3 corrections (high confidence); completeness pass: 12 additions, 8 gaps left |
| [08](08-wallet-core-part3.md) | 08 — Wallet core, part 3 (`wallet/wallet.go` 4429–5828) and the notification server | 2151 | accuracy pass: 215 claims checked, 10 corrections (high confidence); completeness pass: 13 additions, 7 gaps left |
| [09](09-chain-sync-rescan-discovery.md) | 09 — Wallet-side chain processing: block/header ingestion, sidechains, rescans, address discovery, difficulty validation | 2550 | accuracy pass: 310 claims checked, 6 corrections (high confidence); completeness pass: 15 additions, 8 gaps left |
| [10](10-tx-construction.md) | 10 — Transaction construction: coin selection, fees, sizes, change, signing helpers, address buffers | 2155 | accuracy pass: 235 claims checked, 8 corrections (high confidence); completeness pass: 13 additions, 8 gaps left |
| [11](11-staking-vsp.md) | 11 — Staking: ticket purchase, voting, revocation, VSP client, ticket buyer | 1533 | accuracy pass: 215 claims checked, 8 corrections (high confidence); completeness pass: 29 additions, 9 gaps left |
| [12](12-mixclient-protocol.md) | 12 — StakeShuffle mixing client protocol (dcrd mixing module as linked by dcrwallet) | 1618 | accuracy pass: 260 claims checked, 9 corrections (high confidence); completeness pass: 16 additions, 9 gaps left |
| [13](13-wallet-mixing-integration.md) | 13 — Wallet-side mixing integration: account mixer, mixed accounts, change handling | 1237 | accuracy pass: 235 claims checked, 11 corrections (high confidence); completeness pass: 20 additions, 8 gaps left |
| [14](14-spv-p2p.md) | 14 — SPV mode: the p2p peer implementation and the SPV syncer | 2661 | accuracy pass: 265 claims checked, 4 corrections (high confidence); completeness pass: 11 additions, 9 gaps left |
| [15](15-dcrd-rpc-sync.md) | 15 — RPC mode: syncing through a dcrd node over JSON-RPC websocket | 1260 | accuracy pass: 235 claims checked, 8 corrections (high confidence); completeness pass: 13 additions, 6 gaps left |
| [16](16-jsonrpc-transport-types-help.md) | 16 — JSON-RPC server: transport, authentication, command/result types, help text, error codes | 3155 | accuracy pass: 430 claims checked, 12 corrections (high confidence); completeness pass: 9 additions, 7 gaps left |
| [17](17-jsonrpc-methods-part1.md) | 17 — JSON-RPC methods, part 1 (`internal/rpc/jsonrpc/methods.go` lines 1-2797) | 3009 | accuracy pass: 235 claims checked, 13 corrections (high confidence); completeness pass: 11 additions, 7 gaps left |
| [18](18-jsonrpc-methods-part2.md) | 18 — JSON-RPC methods, part 2 (`getvotechoices` … `getcoinjoinsbyacct`) | 2850 | accuracy pass: 235 claims checked, 12 corrections (high confidence); completeness pass: 12 additions, 7 gaps left |
| [19](19-grpc-wallet-service.md) | 19 — gRPC API: transport, VersionService and WalletService | 2717 | accuracy pass: 310 claims checked, 8 corrections (high confidence); completeness pass: 23 additions, 8 gaps left |
| [20](20-grpc-other-services.md) | 20 — gRPC API: loader, account mixer, ticket buyer, seed, agenda, voting, message verification, network and decode services | 2551 | accuracy pass: 215 claims checked, 8 corrections (high confidence); completeness pass: 11 additions, 9 gaps left |
| [21](21-config-cli-lifecycle.md) | 21 — Process: configuration, CLI, startup/shutdown, wallet creation prompts, IPC, logging | 2618 | accuracy pass: 235 claims checked, 14 corrections (high confidence); completeness pass: 16 additions, 8 gaps left |
| [22](22-aux-packages-tools.md) | 22 — Auxiliary packages and command-line tools | 2468 | accuracy pass: 235 claims checked, 5 corrections (high confidence); completeness pass: 13 additions, 8 gaps left |
| [23](23-upstream-tests-oracle-plan.md) | 23 — Upstream test inventory and the differential-testing (oracle) plan | 2768 | accuracy pass: 185 claims checked, 18 corrections (high confidence); completeness pass: 25 additions, 10 gaps left |
| [24](24-reuse-dcroxide.md) | 24 — Reuse assessment: dcrd modules used by dcrwallet versus dcroxide crates | 2248 | accuracy pass: 330 claims checked, 11 corrections (high confidence); completeness pass: 16 additions, 9 gaps left |
| [25](25-reuse-dcr-rs-and-conventions.md) | 25 — Reuse assessment: dcr-rs, and the dcroxide engineering conventions the wallet inherits | 1319 | accuracy pass: 215 claims checked, 19 corrections (high confidence); completeness pass: 23 additions, 10 gaps left |
