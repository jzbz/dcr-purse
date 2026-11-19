# dcr-purse — Developer Brief & Project Plan

**A full Rust re-implementation of the Decred wallet daemon (`dcrwallet`), built as a drop-in replacement, including SPV, StakeShuffle coin mixing and staking.**

Prepared for the implementing team. "Drop-in" covers everything outside the process: the network, both RPC APIs, the command line and configuration. It does **not** cover the database file: dcr-purse stores its data in its own format and **imports** an existing dcrwallet `wallet.db` (project owner's decision, 2026-11-14; see W2). Parity target: **dcrwallet v2.2**, which upstream has not yet released, so the pin follows dcrwallet master: now **`54b85b33`** (2026-11-05, version `2.2.0-pre`, module `decred.org/dcrwallet/v5`). JSON-RPC API version **11.0.0**, gRPC API version **10.0.0**, dcrwallet database version **26** (the import source), P2P protocol version advertised in SPV mode **11**, required dcrd JSON-RPC API **8.3.0 or later in major 8**.

"dcr-purse" is a working name taken from the repository directory; see decision D1.

This brief is the plan. The specifications it rests on are the 25 documents in [specs/](specs/README.md), about 55,000 lines written from the Go source at the pin and then checked against it twice (an accuracy pass and a completeness pass). Where this brief and a spec disagree on a fact, the spec is closer to the source; where a spec and the Go source disagree, the Go source wins and the spec has a bug.

---

## 1. Project statement (the short version)

dcr-purse is a from-scratch Rust implementation of the Decred wallet daemon. The goal is that a user can stop `dcrwallet`, import its `wallet.db` once, start dcr-purse with the same command line and the same `dcrwallet.conf`, and nothing else notices: `dcrd` (or dcroxide) sees the same JSON-RPC client, SPV peers see the same protocol-11 peer, Decrediton and `dcrctl --wallet` get the same gRPC and JSON-RPC answers, a VSP sees the same fee payments, other StakeShuffle participants complete mixes with it, and — above all — the same seed yields the same keys, and an imported wallet has the same accounts, addresses, balances, history, tickets and preferences it had in dcrwallet. The import never modifies the dcrwallet database. Going back to dcrwallet on the original is clean only until dcr-purse has changed wallet state; after that the original is stale (addresses issued, accounts created, tickets, VSP records, preferences), and anything imported into dcr-purse by key, script or xpub is not in it and cannot be recovered from the seed. dcr-purse says so at import; whether it also offers an export of imported keys and scripts for the way back is decision D14.

Scope is the complete feature set of dcrwallet at the parity target:

- **The wallet database**: dcr-purse's own storage format, designed for the wallet's access patterns instead of inherited; the same logical content as dcrwallet's (the address manager with BIP0044 accounts, imported keys/scripts/xpubs, voting accounts, per-account passphrases and watching-only wallets; the transaction store; VSP, agenda and treasury-policy records).
- **Import from dcrwallet**: a read-only reader for the bbolt file format and for every bucket, key and value encoding of dcrwallet's schema, converting an existing wallet — encrypted keys included, without needing the private passphrase — into the new format.
- **Key handling**: Decred's `hdkeychain` derivation including its non-BIP32 treatment of keys with leading zero bytes, PGP-word-list seeds, scrypt/secretbox and Argon2id/XChaCha20-Poly1305 key encryption, lock/unlock with timeouts.
- **Transaction construction**: coin selection, fee and dust rules, size estimation, change handling, signing for every script class the wallet spends, multisig, sweeps, treasury transactions.
- **Both sync modes**: RPC mode through a trusted dcrd over a JSON-RPC websocket, and **SPV mode** with the wallet's own P2P peer implementation, headers-first sync, version-2 committed filters with header-commitment proofs, and the wallet's own header and difficulty validation.
- **Staking**: ticket purchase (plain and mixed), voting with agenda, tspend and treasury-key preferences, the VSP client and its fee-payment state machine, and the automatic ticket buyer.
- **StakeShuffle mixing**: the full mixing **client** (the part dcroxide left out as wallet-side): pair requests, session formation, the six-stage DC-net run, blame, the coinjoin, the post-quantum key agreement, and the account mixer.
- **Both RPC servers**: JSON-RPC (103 implemented methods plus websocket) and gRPC (11 services, 88 RPCs, mutual TLS), byte-compatible in results and errors.
- **Operational compatibility**: all 81 options of `dcrwallet.conf` and the CLI, data directory layout, the interactive creation prompts, the pipe IPC protocol Decrediton uses to supervise the wallet, log format and rotation, TLS certificate and client-certificate handling, signals and exit codes.

Explicit non-goals: Go **library** API parity (dcrwallet's packages are imported by other Go programs; a Rust port cannot serve them), opening or writing a dcrwallet `wallet.db` in place and exporting back to that format (import is one-way; see W2), databases from before the unified layout (dcrwallet ≤ 0.8.2), and the two command-line tools that do not work at the pin (`movefunds`, `sweepaccount`).

"Full test coverage" is defined in §7. The situation differs from dcroxide's in one important way: dcrd carries about 134,000 lines of tests for 168,000 lines of code, while dcrwallet carries about 11,000 for 66,000 hand-written lines, and whole subsystems (SPV, P2P, the gRPC server, the JSON-RPC method handlers, ticket purchase, voting, the VSP client, the ticket buyer, config) have **no upstream tests at all**; mixing integration has a single test of coinjoin detection. There is no test corpus to port. The oracle rig is therefore not one layer among several here; it is how almost every behaviour gets pinned.

---

## 2. Source material, scale, and honest sizing

| Item | Fact |
|---|---|
| Reference implementation | [decred/dcrwallet](https://github.com/decred/dcrwallet), Go, ISC license, about 2,550 commits since 2013 |
| Parity target | dcrwallet v2.2 (unreleased); pin follows master, now `54b85b33` (2.2.0-pre). Latest release tag is `release-v2.1.7` |
| Implementation size | about 85,000 lines of non-test Go, of which about 18,700 are generated protobuf code: roughly **66,000 hand-written lines** |
| Upstream tests | about 11,000 lines; concentrated in the storage layer (`wallet/udb`, `wallet/walletdb`, `wallet/internal/bdb`) and `rpc/jsonrpc/types`, with small suites in `wallet`, `wallet/txauthor`, `wallet/txrules`, `wallet/txsizes` |
| Linked code that is not in the repository | the mixing client (`dcrd/mixing` v0.7.5 `mixclient`, about 3,500 lines), `sntrup4591761` (about 950), `hdkeychain` v3.1.3 (about 640), the read path of `bbolt` v1.3.12 for the importer (well under 1,000), `jrick/wsrpc` plus the client half of a websocket library, the `vspd` client (about 500), `go-flags` v1.5.0, and the server half of gRPC over HTTP/2 |
| dcrwallet's database (import source) | bbolt v1.3.12, opened with default options; schema version 26; 25 upgrade steps from version 1 |
| APIs | JSON-RPC 11.0.0: 103 implemented methods, 8 dead compatibility names, websocket transport. gRPC 10.0.0: 11 services, 88 RPCs (76 unary, 11 server-streaming, 1 bidirectional), about 220 messages |
| Configuration | 81 options in three groups (plus go-flags' built-in help group); no environment variables |
| Sister projects | dcroxide (dcrd in Rust, 35 crates, at dcrd master `46e9df48`) and dcr-rs (Decred primitives) — see §2.1 |
| License plan | ISC, matching dcrwallet, dcrd, dcroxide and dcr-rs, with attribution preserved when porting code or vectors |

Sizing reality check: this is smaller than dcroxide in lines and has no consensus engine, but it is not proportionally easier. Three things make it hard in its own way. First, **funds are at stake on every write**: a wallet that derives one wrong key, records one spend wrongly, or imports one account counter wrongly loses track of money without any network noticing. Second, the surface that must match is unusually wide for the code size: two complete RPC APIs whose error strings and JSON number formatting are client-visible, plus a faithful import of a third-party file format, plus a multi-party cryptographic protocol where any deviation gets the wallet blamed and excluded. Third, there is almost nothing upstream to test against, so the team builds its own oracle for nearly everything. Treat it as a multi-engineer-year project; the phase plan in §6 is ordered so that each milestone is a usable, verifiable thing (libraries → store and importer → syncing wallet → JSON-RPC drop-in → staking → SPV → mixing → gRPC and Decrediton).

The per-spec effort estimates (S/M/L/XL relative to each other) are: fourteen L, seven M, one M–L (15), one S–M (22), one S (25), and one XL — spec 23 rates the oracle rig as a whole XL, "comparable to a large implementation crate". The other XL items appear when specs are grouped into phases (§6): the store plus importer as a whole, the JSON-RPC server as a whole, mixing as a whole, and gRPC including its transport. The XL items appear when specs are grouped into phases (§6): the store plus importer as a whole, the JSON-RPC server as a whole, mixing as a whole, and gRPC including its transport.

### 2.1 What dcroxide and dcr-rs give us (and what they don't)

Detail and the symbol-level inventory are in [specs/24-reuse-dcroxide.md](specs/24-reuse-dcroxide.md) and [specs/25-reuse-dcr-rs-and-conventions.md](specs/25-reuse-dcr-rs-and-conventions.md). The summary:

**dcroxide covers most of what dcrwallet imports from dcrd.** Of the dcrd modules in dcrwallet's `go.mod`:

| dcrd module (as pinned by dcrwallet) | dcroxide crate | Coverage for the wallet | What is missing |
|---|---|---|---|
| `wire` v1.8.0 | `dcroxide-wire` | full | `MsgTx.PkScriptLocs` (credit rows in dcrwallet's database, and so the importer, depend on it), a public `write_var_string`, `MAX_BLOCKS_PER_MSG`, the `TX_VERSION_SEQ_LOCK`/`TX_VERSION_TREASURY` constants, command-name constants, `MsgVersion::add_user_agent` |
| `chaincfg` v3.3.0, `chainhash` | `dcroxide-chaincfg`, `dcroxide-chainhash` | full | one extra mainnet HTTPS seeder at dcrd master (four hosts against v3.3.0's three), to be decided under D7; hdkeychain version bytes are read through a trait in the new crate |
| `txscript` v4.1.2 with `stdaddr`, `stdscript`, `sign` | `dcroxide-txscript` | full | — (addresses are an enum, not interfaces) |
| `blockchain/stake` v5.0.3, `blockchain/standalone` v2.3.0 | `dcroxide-stake`, `dcroxide-standalone` | full | `dcroxide-stake` pulls `dcroxide-database` and `redb` 4 transitively (a feature gate is wanted); `SubsidyCache` needs external locking |
| `gcs` v4.1.1 with `blockcf2` | `dcroxide-gcs` | full | none of its own; inherits `dcroxide-stake`'s dependency on `dcroxide-database` |
| `dcrec`, `dcrec/secp256k1` v4.4.1, `edwards` | `dcroxide-dcrec` | partial | private-key serialization, a reducing constructor, key generation from a reader, ECDH, scalar/point entry points for HD derivation, Ed25519 key from scalar |
| `crypto/blake256`, `ripemd160`, `rand` | `dcroxide-crypto` | full / full / partial | `Int32N`, `Int64N`, `BigInt`, `Reader` |
| `dcrjson` v4.2.0, `rpc/jsonrpc/types` v4.4.0 | `dcroxide-dcrjson`, `dcroxide-rpctypes` | full for commands, help and response marshalling; partial for the envelope | the wallet's own command and result descriptors; the request decoder (`unmarshal_request`, `RawRequest`) sits in `dcroxide-rpc` and must move down or be re-exported; no marshaller for a bare `Response` with an empty `jsonrpc`; dcrd master's types module is v5, so the 19 reused commands and the result types must be compared field by field with v4.4.0 |
| `mixing` v0.7.5 root, `mixpool`, `utxoproof` | `dcroxide-mixing` | partial | key agreement (ECDH + sntrup4591761); a blocking `Receive`; dcroxide's pool is deliberately stricter than dcrd's in three places, chosen for a relaying node, to be relaxed or parameterised for a wallet |
| `mixing/mixclient` | — | none | the whole client (spec 12) |
| `addrmgr` v3.0.0, `connmgr` v3.1.4 | `dcroxide-addrmgr` (ports addrmgr v4), `dcroxide-connmgr` | full for what the wallet calls, at a newer generation | v4 adds a Tor v3 address type and a type filter; the port has no Start/Stop (the wallet owns load, timed save and locking); from connmgr only `DynamicBanScore` and the Tor lookup are used, and the Tor dialing wrapper sits in `dcroxide-node` |
| `certgen` v1.2.0 | `dcroxide-certgen` | partial | P-384; client-CA and client-certificate shapes |
| `base58`, `container/lru` | `dcroxide-base58`, `dcroxide-containers` | full | — |
| `hdkeychain` v3.1.3 | — | none | whole package |
| `dcrutil` v4.0.3 | — (pieces scattered) | none | `Amount`/`NewAmount`, WIF, `txsort`, app-dir |
| `rpcclient` v8.1.0 | — | n/a | not linked at runtime; dcrwallet has its own dcrd client |

The wire protocol version is not a skew: dcrwallet and dcroxide both use `wire` v1.8.0, which defines protocol 12; dcrwallet's own `p2p` package advertises 11, and `dcroxide-wire` takes the version as a parameter.

dcroxide also supplies things the wallet needs that are not dcrd modules: a Go-compatible `encoding/json` emitter over a runtime type model (ADR-0007), a go-flags/INI compatibility layer (for v1.6.1; dcrwallet pins **v1.5.0**, which differs), the slog log format and a log rotator (inside `dcroxide-node`, not yet a library; the rotator ports logrotate v1.1.2 where dcrwallet pins v1.0.0, whose pruning loop and compression path differ), an in-tree TLS provider, HTTP/1.1 and websocket **server** framing, a SOCKS dialer, and the whole oracle-rig method (ADR-0001).

**dcr-rs cannot be linked as it stands.** Its only curve backend is the libsecp256k1 C binding, which dcroxide's `deny.toml` bans by name under the pure-Rust rule (ADR-0012). For every concern both projects cover, the dcroxide crate is the dcrd-parity implementation and the wallet takes that one. What dcr-rs contributes is design and vectors: its `hd` module already models the "stored stripped" key provenance that Decred's hardened derivation depends on (about one seed in 128 derives differently from plain BIP32 somewhere on the wallet path), and its test vectors come from dcrd. The wallet's `hdkeychain` should be a new crate over `dcroxide-dcrec` that takes that design, with dcrd's exact parsing and error behaviour. Note also that dcrwallet seeds are **not** BIP39: they are raw 16–64 byte seeds shown as hex or as PGP-word-list mnemonics with a one-word checksum.

**Neither project has**: a bbolt reader, a wallet store (dcroxide's `redb`-based storage per its ADR-0004 is the obvious starting point for the engine), `hdkeychain`, a websocket or JSON-RPC **client**, HTTP/2, gRPC, protobuf, the mixing client, sntrup4591761, a root solver over the mixing field, a VSP API client (dcroxide's graph has `ureq` 3.5.1 for its seeder, bound to `dcroxide-tlscrypto`; whether the VSP client uses it is a Phase 0 decision), scrypt/Argon2id/secretbox wiring, or terminal prompt handling.

---

## 3. The compatibility contract — what "drop-in replacement" means, precisely

Every task serves one of seven surfaces. They are in priority order; a W1 failure loses money.

**W1 — Keys and funds (absolute).** The same seed and passphrases produce the same extended keys, account keys, addresses and scripts as dcrwallet, for every account type and every branch, including the leading-zero hardened-derivation variant and the SLIP0044 coin-type upgrade. Every output dcrwallet would recognise as the wallet's, dcr-purse recognises; every output dcrwallet could spend, dcr-purse can spend, with signatures that dcrd accepts. No sequence of operations or crashes, and no import, may leave funds unspendable or silently unrecorded. Switching back to dcrwallet is outside this guarantee (§1). This is the surface where "close" is not a grade.

**W2 — Import of a dcrwallet database (lossless, one-way, non-destructive).** dcr-purse does not open `wallet.db` in place and does not write that format. It keeps its own database, and `wallet.db` is an import source. The contract is on the import: given a dcrwallet database at schema version 26 (decision D4), the imported wallet answers every JSON-RPC and gRPC query as dcrwallet would on the original — accounts and their names and numbers, every address with its branch and index, the last-used and last-returned counters (so the next address issued is the one dcrwallet would have issued), imported keys, scripts and xpubs, voting accounts, per-account passphrases, balances by category, full transaction history with block placement, unmined transactions with their credits and spent inputs and the unpublished flag, stake-invalidated credits and debits, tickets and their commitments, multisig records, agenda preferences (wallet-wide and per ticket), tspend and treasury-key policies (wallet-wide and per VSP ticket), VSP ticket records, the birthday and rescan markers, and the stored headers and committed filters (or a defined re-fetch of them). State dcrwallet keeps only in memory is not in the file and is not imported: outpoint locks, in-flight VSP fee payments, address-buffer cursors and side chains start as they would after a dcrwallet restart, and fee payments resume from the imported VSP ticket records. Encrypted material is carried over still encrypted under the user's existing passphrases, so the import needs the public passphrase at most, never the private one or the seed. The import takes the lock dcrwallet takes (spec 01) with a single non-blocking attempt or a short bounded wait; if dcrwallet holds it, the import stops with a message that dcrwallet must be stopped first, and never waits indefinitely as bbolt's own open does. It opens the file read-only and never changes it; an import that fails leaves no half-built wallet behind. The specs of dcrwallet's on-disk format ([01](specs/01-walletdb-bbolt.md), [02](specs/02-address-manager.md), [03](specs/03-txstore-formats.md), [05](specs/05-db-migrations-aux-buckets.md)) are the importer's specification, not the store's. What W2 no longer requires: bbolt write compatibility, schema-26 byte layouts in dcr-purse's own store, the upgrade chain on the creation path, or any of the byte-level quirks of dcrwallet's rows — only the behaviour those rows produce through the APIs.

**W3 — What the network sees.** Transactions, tickets and votes are built with the same structure, fees, sizes and script forms, so they relay and mine as dcrwallet's do. In RPC mode the wallet is the same JSON-RPC websocket client to dcrd: same methods, same notification handling, same API-version check. In SPV mode it is the same peer: protocol version 11 (not 12), the same handshake, requests, timeouts and relay behaviour, such that dcrd and dcroxide nodes neither ban it nor serve it differently. To a VSP it sends the same signed requests in the same order. To other mixers it is indistinguishable: any deviation in pairing data, message bytes, PRNG consumption or blame logic is detected by honest peers and gets the wallet excluded from the mix.

**W4 — JSON-RPC API 11.0.0.** All 103 methods plus the 8 dead names, HTTP POST and websocket transports, HTTP Basic and client-certificate authentication, positional-parameter parsing with Go's leniencies, results byte-compatible with Go's `encoding/json` (member order, omitted members, `null` versus `[]`, float formatting of amounts such as `1e-08`), the same error codes, and error messages that match dcrwallet's own text (see D9 for where the line is drawn on text that Go's standard library authored). Acceptance oracle: `dcrctl --wallet` and existing JSON-RPC integrations work unmodified.

**W5 — gRPC API 10.0.0.** All 11 services and 88 RPCs over HTTP/2 with mutual TLS, the same protobuf messages (tag-ordered output, presence of empty sub-messages), the same status codes and messages, the same streaming behaviour (ordering, readiness gating, what a half-close does). Acceptance oracle: Decrediton operates unmodified, including wallet creation, sync in both modes, ticket purchase, mixing and voting-preference changes.

**W6 — CLI, configuration, lifecycle.** Every option with the same names, defaults, precedence and validation errors under go-flags v1.5.0 semantics; the same data directory resolution per OS and network; the interactive `--create` flows prompt for prompt; exit codes; signals; and the pipe IPC protocol (`--pipetx`, `--piperx`, `--rpclistenerevents`, `--issueclientcert`) that Decrediton relies on to learn the listener addresses and receive its client certificate.

**W7 — Operational artefacts.** `rpc.cert`/`rpc.key` and `clients.pem` handling, the log line format and rotation, subsystem tags and the `--debuglevel` grammar, and the SPV address-manager peers file.

Where dcrwallet's behaviour is a defect, the default is to reproduce it when an external observer could depend on it, and to record it in [QUIRKS.md](QUIRKS.md) with a pinning test. The deep dive recorded more than seven hundred candidate behaviours (732 entries in QUIRKS.md), each still to be confirmed by an oracle run. Three classes get a standing rule instead of case-by-case treatment (decision D10): where Go **panics** on malformed data, dcr-purse returns an error; where Go's result order is **random** (map iteration), dcr-purse picks a deterministic order; and where reproducing a defect would **lose funds or leak secrets**, dcr-purse fixes it and records the divergence in [PARITY.md](PARITY.md).

---

## 4. Engineering principles

These are dcroxide's, adjusted where a wallet differs. [specs/25-reuse-dcr-rs-and-conventions.md](specs/25-reuse-dcr-rs-and-conventions.md) lists each dcroxide ADR and whether it carries over unchanged, needs a wallet decision, or does not apply.

1. **dcrwallet is the specification.** The specs in `specs/` are a map of it, not a replacement for it. When a spec is unclear, read the Go at the pin and reproduce it — including quirks — then fix the spec.
2. **Oracle-driven development, more than ever.** Every module ships with vectors generated by Go shim programs linked against dcrwallet's own packages at the pin, and, where feasible, a differential test that runs the same script of operations through Go and Rust and compares the replies and a logical export of the resulting wallet state (§7). Nothing is "done" on the strength of a hand-written expected value.
3. **No hand-rolled cryptography for standard algorithms.** scrypt, Argon2id, XSalsa20-Poly1305, XChaCha20-Poly1305, BLAKE2b, SHA-2, HMAC and curve arithmetic come from reviewed crates under dcroxide's dependency rules. The project owns only what has no maintained implementation: `hdkeychain`'s Decred variant, sntrup4591761 at its 2017 round-1 parameters (byte-compatible down to how randomness is consumed), the mixing DC-net and key agreement, and the root solver.
4. **Memory safety, and secrets treated as secrets.** `#![forbid(unsafe_code)]` in every dcr-purse crate; the bbolt reader uses positional reads rather than `mmap`, so the rule holds there too. Private keys, seeds, passphrases and derived keys are zeroed on drop, never formatted, and never logged. One upstream behaviour echoes caller-supplied WIF private keys in a `signrawtransaction` error string (spec 08); that is one of the fixes the third standing rule of §3 covers.
5. **Wallet code is boring code.** Explicit integer widths; Go's wrapping arithmetic reproduced with `wrapping_*` exactly where the Go relies on it (child-index counters with the `0xffffffff` sentinel, fee arithmetic) and checked arithmetic everywhere else; no floating point except at the RPC edge where the API defines amounts as floats; deterministic iteration orders.
6. **Every database write is crash-safe.** One atomic transaction per logical operation, with the same boundaries as Go's (what commits together in dcrwallet commits together here, because RPC-visible state depends on it); fault-injection tests on the store. The import is atomic and leaves its source untouched.
7. **Pin, then track.** All parity claims name one upstream commit. The pin follows dcrwallet master until v2.2 is tagged, then release tags, as dcroxide's ADR-0013 does for dcrd. A `PARITY.md` ledger maps each dcrwallet package to its crate and status.
8. **Interoperate before you ship.** No milestone is accepted on unit tests alone: each has a simnet scenario against real Go peers (dcrd, dcrwallet, vspd); from M3 on against `dcrctl --wallet`; and at M7 the scenarios of M2 to M6 are repeated through Decrediton.

---

## 5. Proposed workspace architecture

Crate boundaries mirror dcrwallet's packages where they have proven boundaries and split the `wallet` package (16,000 lines in one Go package) along the lines the specs found. Crate prefix `purse-` is provisional (D1). Crates marked † are candidates to live in, or be contributed to, the dcroxide workspace because the node can use them too (D3).

| Crate | Mirrors | Contents | Spec |
|---|---|---|---|
| `purse-errors` | `errors/` | 21 kinds, Op/Kind/Err model, promote/collapse rules, the `":: "` rendering that reaches RPC clients | 22 |
| `purse-seed` | `pgpwordlist/`, `walletseed/` | seed generation, hex and mnemonic decode/encode with checksum word | 22, 20 |
| `purse-hdkeychain` † | dcrd `hdkeychain` v3.1.3 | extended keys with Decred's stored-stripped hardened derivation, dcrd's parse rules and errors | 02, 25 |
| `purse-dcrutil` † | dcrd `dcrutil` v4.0.3 | `Amount`/`NewAmount` (with Go's float conversion), WIF, `txsort` | 24 |
| `purse-snacl` | `wallet/internal/snacl`, `kdf/` | scrypt + secretbox key sealing; Argon2id + XChaCha20-Poly1305 for account passphrases | 02 |
| `purse-boltread` | `bbolt` v1.3.12 (read path only) | read-only reader: meta page selection and checksum, branch/leaf/overflow pages, nested and inline buckets, ordered iteration, the file lock | 01 |
| `purse-import` | `wallet/udb` on-disk formats | decoders for every dcrwallet bucket, key and value at schema version 26; conversion into the new store; verification report | 01, 02, 03, 05 |
| `purse-store` | — (new design; engine per D2) | dcr-purse's own schema and transactions | — |
| `purse-udb` | `wallet/udb` (logic, not layout) | address manager and transaction store behaviour over `purse-store`: accounts, counters, credits, debits, rollback, stake invalidation, balances, queries, input source, VSP/agenda/treasury records | 02, 04, 05 |
| `purse-txrules`, `purse-txauthor` | `wallet/txsizes`, `txrules`, `txauthor` | size estimates, fee and dust rules, the fee/input/change loop | 10 |
| `purse-chain` | `wallet/sidechains.go`, `difficulty.go`, `chainntfns.go`, `rescan.go`, `discovery.go`, `validate/`, `deployments/` | sidechain forest, header and difficulty validation, chain switch, rescan, discovery | 09, 06 |
| `purse-wallet` | rest of `wallet/` | lifecycle, locking, accounts, queries, signing, publishing, address buffers, transaction creation, notifications, the `NetworkBackend` trait | 06, 07, 08, 10 |
| `purse-stake` | ticket/vote parts of `wallet/` | ticket and vote construction, purchase flow, vote orchestration, policies | 11 |
| `purse-vsp` | `wallet/vsp*.go`, `feepayment.go`, vspd client | VSP HTTP API client, fee-payment state machine | 11 |
| `purse-ticketbuyer` | `ticketbuyer/` | automatic buyer | 11 |
| `sntrup4591761` † | companyzero/sntrup4591761 | the KEM, byte-compatible | 12 |
| `purse-mixclient` † | dcrd `mixing/mixclient` | the mixing client; key agreement added to `dcroxide-mixing` | 12 |
| `purse-mixsolver` | cspp `solverrpc`, `csppsolver` | root finding over F = 2¹²⁷−1 (native, and/or the external solver protocol) | 12 |
| `purse-wsrpc` † | jrick/wsrpc + websocket client half | JSON-RPC 2.0 over a websocket client | 15 |
| `purse-dcrd-client`, `purse-rpcsync` | `rpc/client/dcrd`, `chain/` | typed dcrd client, the RPC-mode syncer | 15 |
| `purse-p2p`, `purse-spv` | `p2p/`, `spv/`, `lru/` | the wallet's peer implementation and the SPV syncer | 14 |
| `purse-rpctypes` | `rpc/jsonrpc/types` | command and result descriptors over `dcroxide-dcrjson` | 16 |
| `purse-jsonrpc` | `internal/rpc/jsonrpc`, `internal/rpchelp` | transport, auth, dispatch, the 103 handlers, help data | 16, 17, 18 |
| `purse-walletrpc` | `rpc/api.proto`, `rpc/walletrpc` | protobuf messages and codec | 19, 20 |
| `purse-grpc` | grpc-go server subset, `internal/rpc/rpcserver` | HTTP/2 + gRPC transport (per D5) and the 11 services | 19, 20 |
| `purse-config`, `purse-logging`, `purse-ipc`, `purse-prompt`, `purse-loader` | `config.go`, `params.go`, `internal/cfgutil`, `internal/netparams`, `log.go`, `internal/loggers`, `ipc.go`, `internal/prompt`, `internal/loader`, `version/` | options and validation, slog format and rotation, pipe protocol, creation prompts, wallet load/unload, version strings | 21 |
| `dcr-purse` (bin) | `dcrwallet.go`, `walletsetup.go`, `rpcserver.go`, `signal*.go` | daemon assembly | 21, 16 |
| `purse-tools` (bins) | `cmd/treasurykey`, `cmd/repaircfilters` | the two tools that work at the pin | 22 |
| — (not ported) | `wallet/walletdb`, `wallet/internal/bdb`, `wallet/drivers/bdb`, `rpc/client/dcrwallet`, `rpc/rpc_example`, `rpc/grpc_example`, `cmd/movefunds`, `cmd/sweepaccount` | replaced by `purse-store` and `purse-boltread`, or Go-library and example code outside the contract | 01, 22 |
| `tools/oracle` (Go) | n/a | shim binaries linking dcrwallet at the pin: database dumper, logical exporter, operation-script runner, RPC golden capture, vector generators | 23 |

Changes needed in dcroxide. Additive: `dcroxide-wire` (`PkScriptLocs` and the small items of spec 24 section 12.3), `dcroxide-dcrec` (the key-handling entry points listed in §2.1, a numeric `SignatureType` conversion, `GeneratePrivateKey`), `dcroxide-crypto` (`rand` helpers), `dcroxide-dcrjson` (the request decoder moved down from `dcroxide-rpc`; a bare-`Response` marshaller), `dcroxide-mixing` (key agreement, a client-usable pool interface without the three node-policy rules), `dcroxide-certgen` (P-384 and a template parameter for client-CA and client certificates), `dcroxide-stake` (a feature gate so a wallet build can drop `dcroxide-database`). Refactors of `dcroxide-node`/`dcroxide-rpc`, not additive: extraction into library crates of the HTTP/1.1 server and websocket framing, the SOCKS dialer (which also needs `Pool` and `ProxiedAddr`), the go-flags layer, the slog backend and log rotator, the Go-compatible client-certificate verifier, and the Tor-resolver socket wrapper, with `DynamicBanScore` re-exported from a small crate.

---

## 6. Phase plan

Phases are ordered by dependency and sized S/M/L/XL including tests. Every exit criterion is testable. Each phase names the specs it implements; a phase is not started until its specs' "Open questions" sections have been triaged.

### Phase 0 — Scaffolding, oracle rig, decisions *(M)*

- Cargo workspace under dcroxide's lint, deny and toolchain policy; CI on Linux, macOS and Windows; fuzz infrastructure.
- `tools/oracle`: Go module pinned to dcrwallet `54b85b33`. First shims: **database dumper** (whole bucket tree as sorted hex; used to check `purse-boltread`), **logical exporter** (the canonical, format-independent text form of a wallet, covering every item W2 lists, with masking rules for clock- and randomness-derived fields; its format is fixed in an ADR before Phase 1, and its Rust counterpart lives in `purse-udb`), **operation-script runner** (a JSON script of `udb`/`wallet` calls with injected randomness and time → logical export plus results), **address deriver**.
- Pin the acceptance clients and peers by release: the Decrediton, `dcrctl`, vspd and dcrd versions are recorded next to the dcrwallet pin and move with it (D12). Decrediton's wallet launcher and its gRPC call inventory at that release are read and added to specs 19–21 before Phase 8; the specs note that Decrediton's source was not among the inputs of the deep dive.
- Docs skeleton: this brief, `specs/`, `PARITY.md`, `QUIRKS.md`, an ADR directory.
- Decision ADRs D1–D10 of §9 drafted. `purse-boltread` dumps a real `wallet.db` identically to the Go dumper (the importer's foundation). Two spikes inform the costliest decisions: a `redb` schema prototype measured on a large wallet's workload (for D2), and a minimal gRPC server answering `VersionService.Version` to an unmodified grpc-go client and to Decrediton over mutual TLS (for D5).
- **Exit:** CI green on three platforms; the Rust reader's dump of Go-written databases (all fixtures in `wallet/udb/testdata` plus a funded simnet wallet) equals the Go dumper's; D1–D10 decided.

### Phase 1 — Foundations *(M)* — specs 22, 02 (keys), 25, 24

- `purse-errors` first: it fixes the error text every later crate emits.
- `purse-seed`, `purse-hdkeychain`, `purse-dcrutil`, `purse-snacl`; the `dcroxide-dcrec`, `dcroxide-crypto` and `dcroxide-wire` additions.
- **Exit:** for at least 10⁶ random seeds plus every seed whose path contains a leading-zero key in a targeted search, account xpubs and the first addresses of both branches equal the Go oracle's; all `hdkeychain`, `pgpwordlist`, `walletseed`, `snacl` upstream vectors pass; secretbox/Argon2 blobs written by Go open in Rust and the reverse; error rendering equals Go's for a generated corpus of nested errors.

### Phase 2 — Store design and the bbolt reader *(M)* — spec 01, D2

- `purse-store`: the engine chosen in D2 and a schema designed for the wallet's hot paths, which the specs identify in dcrwallet's layout as costs to remove: balances recomputed by a full scan of unspent outputs on every call, range queries that walk every block record, no secondary indexes for address and account lookups.
- **Order that reaches a client is part of the schema.** Several API results and every coin selection follow bbolt's raw key order: accounts iterate in the byte order of their 4-byte little-endian number (0, 0x80000000, 256, …, 1, …), and input selection walks unspent outputs by raw transaction hash then big-endian output index, seeks to the first key ≥ a random 32-byte key, and relies on "previous of end is last" (spec 04 §5.1). The store keeps an index with dcrwallet's key bytes and those cursor semantics wherever order is observable, so that results are ordered as Go's and, given the same randomness, the same outputs are spent.
- The schema is versioned from day one with its own migration mechanism: each step is one atomic transaction, a database newer than the binary is refused, and a fixture of every released schema version is kept from the first release on.
- `purse-boltread`: complete read path with the file lock; hardened against corrupt and hostile files.
- **Exit:** the reader's dump equals Go's for every fixture and for large real-world-shaped databases at page sizes 4096 and 16384; fuzzing the page decoder on corrupt files yields errors, never panics or unbounded allocation; the store passes fault injection (a crash at any point leaves the last committed state) and a model-based property test; a cursor-conformance test runs the spec 04 §5.1 cases against `purse-store` and against bbolt through the oracle; a migration test applies a dummy version step with a crash injected at every write and ends in the old or the new version, never between; the schema ADR records the measured gains against dcrwallet on the same workload.

### Phase 3 — Wallet database logic and the importer *(XL)* — specs 02, 03, 04, 05

- Address manager behaviour over the new store: account types, counters, lock/unlock, imports, watching-only. Key encryption stays format-compatible at the blob level (scrypt + secretbox, Argon2id + XChaCha20-Poly1305), so imported keys are carried over without decryption.
- Transaction store behaviour: mined and unmined logic, rollback, stake invalidation, balances, queries, the input source. Byte-level row quirks are not reproduced; RPC-visible consequences are (spec 04 separates the two).
- Wallet creation directly in the new format.
- `purse-import`: decoders for schema version 26 (older versions are refused, per D4); conversion; a verification report (counts and balances per account as read from the source and as stored); the import front end as a test binary (final form in Phase 8).
- **Exit:** the operation-script differential passes on return values and logical export for a corpus covering every `Manager` and `Store` method; a wallet created by Rust and one created by Go from the same seed and passphrases have equal logical exports; every database of a version-26 import corpus generated by the Go oracle and checked in by recipe (the upstream fixtures upgraded to 26 by Go, plus real-world-shaped simnet/testnet wallets: many accounts, imported keys and scripts, voting accounts, tickets in every state, multisig, VSP records, unmined and unpublished transactions) imports to a logical export equal to Go's on the original, and a sweep of the `wallet`-level query methods returns equal results; the sweep is repeated through JSON-RPC at Phase 7 and gRPC at Phase 12; every upstream fixture below version 26 is refused with the D4 message;; the source file's hash is unchanged after import; the portable upstream `udb` logic tests pass.

### Phase 4 — Transaction construction and signing *(L)* — spec 10, parts of 08

- The `NetworkBackend` trait with a scripted fake implementation, and the `Wallet` skeleton with its database handle and passphrase lock state — here rather than in Phase 5 because address buffers and `createtx` call both.
- `purse-txrules`, `purse-txauthor`, address buffers and gap policies, `createtx` (selection wrappers, split transactions, multisig, consolidation), `SignTransaction`, `CreateSignature`, message signing.
- **Exit:** with randomness injected identically and the stores holding the same UTXO set (the Rust side reached by importing the Go database), Go and Rust build byte-identical transactions for a scenario corpus (plain sends, change and no-change, dust edges, high-fee guard, multisig, sweeps, treasury); every produced transaction passes dcrd's and dcroxide's mempool acceptance on simnet; upstream `txauthor`/`txrules`/`txsizes` vectors pass.

### Phase 5 — Chain processing and wallet core *(L)* — specs 06, 07, 08, 09

- Sidechain forest, header and difficulty validation (EMA and ASERT via `dcroxide-standalone`), chain switch, transaction ingestion, rescan, account and address discovery, the processed-blocks marker and birthday state.
- The rest of the wallet lifecycle, accounts, queries with their exact orderings, the notification server (delivery model per D11).
- **Exit:** replaying a recorded simnet history (including reorgs, tickets, votes and revocations) through Go and Rust wallets with a scripted fake backend yields equal logical exports and equal notification sequences; header validation agrees with Go on mainnet and testnet3 header ranges covering both difficulty algorithms and the transition.

### Phase 6 — RPC-mode sync *(L)* — spec 15

- `purse-wsrpc` (websocket client, JSON-RPC 2.0 framing), `purse-dcrd-client` (the 25 dcrd methods and 5 notifications), `purse-rpcsync`.
- **Exit — milestone M2:** dcr-purse imports a Go-created simnet wallet, syncs through dcrd **and** through dcroxide, tracks sends and receives, survives dcrd restarts and reorgs, and agrees on balances and history with a Go dcrwallet running on the original database against the same chain. Then the same on testnet3 with a wallet restored from seed (discovery), compared account by account with Go.

### Phase 7 — JSON-RPC server *(XL)* — specs 16, 17, 18

- Transport (HTTP/1.1 POST and websocket in dcroxide's in-tree style), authentication, dispatch, passthrough to dcrd, `purse-rpctypes` descriptors over `dcroxide-dcrjson`, help data, then the 103 handlers in dependency order. Handlers whose wallet code belongs to a later phase (ticket purchase and VSP, mixing, the SPV branches of `getpeerinfo` and `syncstatus`) get their descriptors, argument parsing and pre-backend error exits here; their success paths close in Phases 9, 10 and 11, each of which adds "golden suite green for its methods" to its exit.
- Golden-capture harness: for every method, success and each documented error exit, recorded from Go dcrwallet against prepared databases and replayed against dcr-purse with raw-byte comparison.
- **Exit:** golden suite green for every method and error exit of specs 17 and 18 that does not need staking, SPV or mixing, with the deferred set listed in PARITY.md.

### Phase 8 — Daemon assembly *(L)* — spec 21

- Options with go-flags v1.5.0 semantics, the 39-step validation pipeline, logging, creation prompts, pipe IPC, signals, loader, TLS material; the import command and the first-start import offer (D1).
- **Exit — milestone M3:** a config-compatibility suite generated from Go (argv and INI inputs → resolved config or exact error) passes; the `--create` flows match a scripted terminal session transcript; the `dcrctl --wallet` command sweep against Go and Rust shows no unexplained difference; after an import, an existing `dcrctl` configuration with the old `rpc.cert` connects unchanged; dcr-purse runs as a JSON-RPC drop-in for a dcrd-backed wallet on simnet and testnet3 for a multi-week soak alongside a Go wallet on a copy of the same seed, with periodic logical-export comparison.

### Phase 9 — Staking *(L)* — spec 11

- Ticket and vote construction, the purchase flow, vote orchestration with agenda/tspend/treasury policies, VSP client and fee-payment state machine, ticket buyer.
- **Exit — milestone M4:** on simnet, dcr-purse buys tickets (solo and through a real vspd), votes on every block it is called for across an agenda vote and a tspend vote, and its votes are byte-equal to those a Go wallet with the same keys and preferences produces for the same blocks; the VSP request sequence equals Go's for scripted scenarios including fee expiry and restart.

### Phase 10 — SPV *(L)* — spec 14

- `purse-p2p` (handshake at protocol 11, roughly thirty request/handler state machines with their stall rules), `purse-spv` (three-stage initial sync, steady state, rescan, relay), address manager at v3.0.0 semantics (D7).
- A fake-peer harness, since upstream has no tests here; adversarial peers (stalling, lying about filters, invalid headers).
- **Exit — milestone M5:** SPV sync of mainnet and testnet3 from genesis against dcrd and dcroxide peers ends with the same headers, filters and wallet state as a Go SPV wallet; mixed fleets show no bans; restored-from-seed discovery finds the same accounts as Go (including the non-bisecting probe sequence, per the quirk decision).

### Phase 11 — Mixing *(XL)* — specs 12, 13

- `sntrup4591761`, key agreement, `purse-mixclient` (pairing, session formation, the KE/CT/SR/FP/DC/CM run, blame, coinjoin), the solver (D8), the wallet's mixing integration, account mixer, mixed ticket purchase, and the ticket buyer's mixed-purchase and change-mixing paths.
- **Exit — milestone M6:** on simnet and testnet3, dcr-purse completes mixes (in RPC mode, and in SPV mode once M5 is reached) in sessions whose other participants are all Go dcrwallets, at every denomination and with mixed ticket purchases; in induced-failure runs (a peer dropping at each stage) it blames the same peers Go does and is never blamed itself; KEM and PRNG vectors from Go match byte for byte.

### Phase 12 — gRPC *(XL)* — specs 19, 20

- Transport per D5, the protobuf codec, mutual TLS with Go-compatible client-certificate verification, the 11 services, the loader service and its sync streams.
- **Exit — milestone M7:** golden unary and stream captures match; Decrediton at the pinned release runs unmodified against dcr-purse on all three operating systems through wallet creation, restore, both sync modes, sending, ticket purchase with a VSP, mixing, and voting preferences, including being launched and supervised over the pipe IPC; the scenarios of M2 to M6 are repeated through it.

### Phase 13 — Hardening, performance, release *(L, then continuous)*

- External security review focused on key handling, the storage engine, RPC authentication and the mixing client; fuzz and differential soaks promoted to scheduled runs; mutation testing on the database and transaction-construction crates.
- Performance against Go: open time (scrypt dominates), rescan and discovery throughput, SPV initial sync, RPC latency on large wallets, memory.
- Backup and recovery: a documented, tested procedure for backing up the dcr-purse database (D15), and a standing CI job that upgrades every released schema version to current.
- Release engineering under dcroxide's ADR-0014 policy.
- **Exit — 1.0:** all W1–W7 suites green at the pin; a multi-month mainnet soak of paired Go and Rust wallets on copies of the same seeds with zero divergence in logical export; review findings resolved.

---

## 7. Testing & verification strategy

The full design, with the inventory of every upstream test file, is [specs/23-upstream-tests-oracle-plan.md](specs/23-upstream-tests-oracle-plan.md). The layers:

**Layer 1 — Ported upstream tests.** Everything upstream has that still applies: the `udb` logic tests (not the upgrade tests, whose fixture databases at versions 1–24 are kept as bolt-reader inputs and as import-refusal cases; upstream ships no version-26 fixture), `txauthor`/`txrules`/`txsizes`, `rpc/jsonrpc/types`, `pgpwordlist`, `walletseed`, `snacl`, `errors`, and the few tests in `wallet`. The `walletdb` interface tests exercise a read-write bucket API dcr-purse does not implement and are not ported. This is necessary and nowhere near sufficient.

**Layer 2 — Oracle vectors and operation-script differentials.** The core of the plan. Go shims linked against dcrwallet at the pin, with `crypto/rand` and the clock replaced by injected sources, run the same scripted operations as the Rust code; the comparison is every return value plus a **logical export** both sides can produce (accounts, addresses and counters, credits and debits, balances, tickets, preferences in a canonical form), since the two databases no longer share a byte format. This is what pins the transaction store, the address manager, transaction construction and wallet creation. The importer gets the strictest form of the same test: import a Go-written database, then compare the logical export and a full RPC query sweep against Go running on the original. The CI fails rather than skips when the oracle is missing, as dcroxide's does.

**Layer 3 — Golden captures of both RPC surfaces.** Requests and raw response bytes recorded from Go dcrwallet against prepared databases on simnet (via the dcrtest harness), for every method and every error exit the specs enumerate, replayed against dcr-purse. Where a response contains nondeterministic content, the capture records which bytes are masked and why.

**Layer 4 — Property tests.** Every codec round-trips; `purse-store` against an in-memory model; `purse-boltread` against Go-written files of random content; address-buffer and counter invariants; balance equals the sum over unspent credits under random histories.

**Layer 5 — Fuzzing.** Bolt page decoding on corrupt files, every database row decoder, JSON-RPC request parsing, protobuf decoding, HTTP/2 frames and HPACK if in-tree, P2P message handling from a hostile peer, VSP responses, mix messages, config and INI parsing, seed and extended-key parsing.

**Layer 6 — Import.** Go builds wallets of every shape (at schema version 26; older fixture databases are used to test the refusal); Rust imports each; the logical export and a full RPC query sweep are compared with Go on the original. Standing CI job. Also: import while a Go dcrwallet holds the database open (refused, promptly), import of a database left by a crash mid-commit (bbolt's two meta pages), import refusal on corrupt input, and the source left byte-identical.

**Layer 7 — Ecosystem acceptance on simnet and testnet3.** The matrix: backend {dcrd, dcroxide} × sync mode {RPC, SPV} × scenario {send/receive, reorg, restore from seed, ticket purchase solo and VSP, vote, agenda and tspend votes, mixing with Go peers, mixed tickets} × client {`dcrctl --wallet`, Decrediton, a gRPC golden client}. Voting scenarios run in RPC mode only: dcrwallet refuses `--spv` together with `--enablevoting` ("SPV voting is not possible"), and dcr-purse reproduces that refusal. Paired-wallet soaks (Go and Rust on copies of one seed) with logical-export comparison are the wallet's equivalent of dcroxide's per-block comparator.

**Nondeterminism** is the main obstacle to differential testing and is handled deliberately: every use of `crypto/rand` (coin selection, output shuffling, nonces, jitter, mixing PRNG seeds), every clock read stored in the database, and every map iteration that reaches an output is catalogued in the specs; the Rust code takes its randomness and time from injectable sources so that tests can align the two implementations.

**Gates.** Crates on the W1/W2 surfaces (`purse-hdkeychain`, `purse-snacl`, `purse-import`, `purse-store`, `purse-udb`, `purse-txauthor`, the signing paths, `sntrup4591761`, `purse-mixclient`) require differential coverage against the Go oracle, a fuzz soak, and an adversarial review of every change, under the same single-maintainer arrangement dcroxide's §7 records.

---

## 8. Dependency policy & candidate crates

Policy is dcroxide's: pure Rust (ADR-0012), `cargo-deny` on licenses, advisories, sources and bans, a dependency-ledger row per load-bearing crate, no duplicate versions. Candidates, to be validated in Phase 0–1; claims about third-party crates in the specs are marked UNVERIFIED where they could not be checked offline.

| Need | Candidate | Notes |
|---|---|---|
| Decred primitives | dcroxide crates (D3) | additive changes listed in §5 |
| secp256k1 | via `dcroxide-dcrec` (`k256` + the in-tree vartime crate) | HD derivation needs scalar-add and point-add entry points |
| scrypt, Argon2id | RustCrypto `scrypt`, `argon2` | scrypt N=2¹⁸ needs about 256 MiB per derivation; stored parameters vary per database and must be honoured. `argon2` 0.5.3 pulls `blake2` 0.10 on `digest` 0.10, a duplicate of dcroxide's `digest` 0.11 that `multiple-versions = "deny"` refuses: a `digest`-0.11 release line of both, or a recorded skip, to verify in Phase 0 |
| secretbox, XChaCha20-Poly1305 | RustCrypto `crypto_secretbox` (to verify against dcroxide's `poly1305`/`cipher` versions); `chacha20poly1305` 0.11.0 is already in dcroxide's graph through `dcroxide-tlscrypto` | — |
| BLAKE2b, SHA-2, HMAC | RustCrypto `blake2` (same `digest`-generation question as Argon2id); `sha2` 0.11 and `hmac` 0.13 are already in the graph | — |
| sntrup4591761 | in-tree port | no crate confirmed byte-compatible with the 2017 parameters and the Go randomness consumption |
| Storage | `redb` 4 (D2) — dcroxide locks 4.5.0 per its ADR-0004, and the wallet already links it through `dcroxide-stake`/`dcroxide-gcs`, so the wallet takes the same major | the wallet's workload (small, read-heavy, many secondary lookups) differs from the node's; measure before adopting. `dcroxide-testutil`'s power-loss storage backend is the starting point for the store's fault-injection tests |
| bbolt import | in-tree `purse-boltread` | read-only; no third-party bbolt-format crate was verified |
| TLS | `rustls` 0.23 on `dcroxide-tlscrypto`, every configuration built with `builder_with_provider` (dcroxide's `clippy.toml` refuses the default-provider builders) | client-certificate verification must match Go's for Decrediton's self-signed client certificates: dcroxide's Go-style verifier exists but is private to `dcroxide-node` and must be extracted; the provider refuses RSA server keys that Go accepts, a divergence to record or close |
| HTTP/1.1, websocket server | dcroxide's in-tree framing | extracted to a library crate |
| Websocket and HTTP client | websocket: in-tree (`purse-wsrpc`); HTTPS for the VSP API: `ureq` =3.5.1 as dcroxide's seeder uses it, or a small in-tree client if proxy and dialer control require it | handshake fidelity is decision-level (spec 15); trust roots for outbound HTTPS need a decision |
| HTTP/2, gRPC, protobuf | D5 | the largest dependency decision in the project |
| JSON | `dcroxide-dcrjson` Go-compatible emitter | wallet types written as descriptors; no serde in the RPC crates |
| Config | dcroxide's go-flags layer, taught v1.5.0 | v1.5.0 and v1.6.1 differ in integer bases and boolean values |
| Terminal | a minimal no-echo prompt | Windows console and Unix termios without `unsafe` in our crates |
| Test/QA | `proptest`, `cargo-fuzz` (separate `fuzz/` workspace), `cargo-deny`; benchmarks through a purpose-built harness and a bench ledger | as dcroxide, which does not use `criterion` |

---

## 9. Decisions the team must surface before or during Phase 0

Each becomes an ADR. Recommendations are the brief's, not settled outcomes; the specs' "decisions needed" lists hold the many smaller ones.

- **D1 — Name and drop-in identity.** dcroxide runs under its own name and data directory, which means an existing `dcrd.conf` must be pointed at. A wallet has a harder version of that question because the database is the point: Decrediton launches a binary called `dcrwallet` and the user's funds are in `~/.dcrwallet`. With its own database format, sharing dcrwallet's data directory would put two different `wallet.db`-like files side by side, so the question becomes where dcr-purse keeps its data and how it finds a wallet to import. Options: (a) own name, own data directory, an explicit import command; (b) the same, plus an offer to import when a dcrwallet directory exists and dcr-purse has no wallet yet; (c) also installable as `dcrwallet` for Decrediton, which then needs the import to happen without a prompt. *Recommendation:* (b), with (c) designed in Phase 8 against Decrediton's actual launch sequence; and decide what `version` and the P2P user agent report. The decision also fixes the import front end (the command that runs it, its prompts, report and exit codes) and which files besides the database an import carries over or reuses (`dcrwallet.conf`, `rpc.cert`/`rpc.key`, `clients.pem`, the SPV peers file), since regenerating the TLS certificate breaks clients that pinned the old one.
- **D2 — Storage engine and schema.** *Engine decided (project owner, 2026-11-14): `redb` is the default candidate, as in dcroxide's ADR-0004, to be confirmed by a Phase 0 prototype measured against a large wallet's workload.* Still open: the schema. The specs show where dcrwallet's layout costs time: full unspent scans for every balance, no secondary indexes for address and account lookups, history queries that walk block records. Keep the transaction boundaries of the Go code so observable ordering is unchanged; version the schema from the start.
- **D3 — How dcroxide crates are consumed.** Wallet crates inside the dcroxide workspace, a sibling workspace with pinned git dependencies, or published crates. *Recommendation:* a separate workspace depending on dcroxide at a pinned revision, with the additive changes landed in dcroxide first; this needs an allow-listed git source in `deny.toml`.
- **D4 — Which dcrwallet databases can be imported.** *Decided (project owner, 2026-11-14): schema version 26 only*, which every dcrwallet since 2.0.0 writes. An older database is refused with a message telling the user to open it once with a current dcrwallet, which upgrades it in place; pre-unified databases are refused likewise. The 25 upgrade steps are therefore not ported; spec 05's account of them is reference material only. Still open under this heading: whether an import carries over stored headers and committed filters or re-fetches them, and whether to offer a seed-only restore as the documented fallback.
- **D5 — gRPC transport and protobuf.** (A) `tonic`/`prost` on `tokio`/`hyper`/`h2`, which imports the async runtime dcroxide's ADR-0005 decided against; (C) in-tree HTTP/2, HPACK, gRPC framing and a protobuf codec, consistent with dcroxide's in-tree HTTP/1.1 and websocket; (B) something between. The proto is plain proto3 with no maps, oneofs or optional fields, so the codec is small either way; HTTP/2 is the cost. *Recommendation:* decide on the Phase 0 spike; lean to (C) with (A) confined to the gRPC listener as the fallback if the spike shows HTTP/2 flow control and keepalive parity to be out of proportion.
- **D6 — Concurrency model.** dcroxide is OS-threaded throughout. The wallet has more blocking request/response state machines (about thirty in the P2P layer, the mixing client's timed stages, long-lived gRPC streams) and Go code that holds mutexes across network I/O and across database transactions. *Recommendation:* keep OS threads and port the Go structure closely, so that lock ordering and delivery ordering can be audited against the specs; revisit only if D5 forces a runtime in.
- **D7 — Version skew against dcroxide.** dcroxide tracks dcrd master; dcrwallet links tagged dcrd modules. Known deltas that matter: `addrmgr` v3.0.0 against the v4 generation dcroxide ports; the mainnet HTTPS seeder list (four hosts at dcrd master, three in `chaincfg` v3.3.0); `rpc/jsonrpc/types` v4.4.0 against master's v5; go-flags v1.5.0 against v1.6.1 and logrotate v1.0.0 against v1.1.2; and, separately from any dcrd skew, `dcroxide-mixing`'s three node-policy rules that are stricter than dcrd's own pool (a per-identity key-exchange cap, one-message-per-type applied to un-orphaned messages, the expiry latch). The P2P protocol version is a parameter (11), not a skew. *Recommendation:* per module, a recorded diff between the tag dcrwallet pins and the commit dcroxide ports; reuse where the diff is empty or irrelevant to a client, parameterise where it is not.
- **D8 — Mixing root solver.** Spawn the external `csppsolver` (needs a Go `net/rpc` + gob client), ship a native solver, or both. *Recommendation:* native solver, verified against the Go solver's outputs, with `--csppsolver` still accepted; timing at 64 peers must fit the stage deadline.
- **D9 — Error-message fidelity.** Messages reach clients verbatim and embed operation names, kind strings and, at the leaves, text from Go's standard library. *Recommendation:* byte-exact for everything dcrwallet authored (Op names, kinds, separators, its own format strings); exact code plus best-effort text for leaves authored by Go libraries, with the list of such leaves kept in PARITY.md.
- **D10 — Standing quirk policy.** As stated at the end of §3: reproduce by default; errors instead of panics; deterministic order instead of map order; fix what would lose funds or leak secrets.
- **D11 — Notification delivery.** Go sends on unbuffered channels while holding the notification server's mutex, sometimes inside an open database write transaction, so a slow gRPC subscriber stalls the wallet. *Recommendation:* preserve per-client ordering and the points at which notifications are generated, but decouple delivery with bounded per-client queues; record the divergence.
- **D12 — Upstream tracking.** Follow dcrwallet master until v2.2 is tagged, then release tags; the dcrwallet and dcrd pins move together.
- **D13 — Upstream defects worth reporting.** The deep dive found behaviours that look like real defects in dcrwallet (among them a possible nil dereference in the wallet's difficulty validation (`isAncestorOf`, reached by a header that forks below the cached anchor; whether a peer can deliver one is unconfirmed), caller-supplied WIF private keys echoed in a `signrawtransaction` error string, and outpoint locks leaked on partial VSP purchases (fee inputs for the unpublished tickets stay locked until restart)). Decide who reports them upstream and when; none has been independently confirmed by running code.
- **D14 — The way back to dcrwallet.** Import is one-way. Decide whether dcr-purse offers an export of what cannot be re-derived from the seed (imported keys, scripts, xpubs, account names, preferences) so that a user can rebuild a dcrwallet wallet, and what the user documentation promises.
- **D15 — Backup and recovery of the own-format store.** File copy when stopped, a consistent snapshot while running if `redb` allows it, and what a user restores from.

---

## 10. Top risks & mitigations

- **R1 — Key-derivation divergence.** A fixed 32-byte private-key type erases the stripped/unstripped distinction and about one seed in 128 derives wrong keys somewhere on the wallet path. Mitigation: the Phase 1 exit criterion, targeted leading-zero vectors, and a restore-from-seed comparison against Go in every later milestone.
- **R2 — Lossy or wrong import.** An account counter, an imported key, a ticket or a spent flag carried over wrongly, and the wallet reissues addresses, misses funds or misstates its balance. Mitigation: the import never touches the source; the verification report; the logical-export and RPC-sweep comparison against Go as a standing job over wallets of every shape; restore-from-seed as an independent cross-check of what the import produced. The new store's own corruption risk is covered by an established engine plus fault injection.
- **R3 — Untested upstream behaviour.** Most of what must match has no upstream test, and the specs were derived by reading. Mitigation: the oracle rig precedes implementation in every phase; spec statements flagged UNVERIFIED or "derived from reading only" are settled by an oracle run before code depends on them.
- **R4 — Silent mixing incompatibility.** Any deviation makes honest peers blame the Rust wallet; the user sees only mixes that never complete. Mitigation: byte-level KEM and PRNG vectors, sessions against Go peers at every denomination, induced-failure runs.
- **R5 — The gRPC transport.** Either an async runtime enters a wallet process or the project owns HTTP/2. Mitigation: the Phase 0 spike and D5; gRPC is last in the plan so it cannot hold the JSON-RPC drop-in hostage.
- **R6 — Go-specific observable behaviour.** Float formatting, `nil` versus empty slices in JSON, Go's unstable sort order in `listunspent`, float-to-integer overflow in `NewAmount` (differs by CPU architecture), HTML escaping in JSON strings. Mitigation: dcroxide's Go-JSON emitter; golden captures; each case has a decision in specs 16–18.
- **R7 — Lost votes or unregistered tickets.** A wrong vote or a broken fee-payment sequence costs users rewards. Mitigation: vote byte-equality against Go on simnet across agenda states; VSP sequence tests against a real vspd including restarts.
- **R8 — Filter completeness.** One missed `loadtxfilter` or cfilter match-set entry and the wallet silently misses its own transactions. Mitigation: paired-wallet soaks with logical-export comparison; discovery and rescan differentials.
- **R9 — Version skew with dcroxide** (D7) and **a moving upstream** (D12).
- **R10 — Underestimation and review scarcity.** Milestones are ordered so partial completion is still useful: M1 gives the Rust ecosystem `hdkeychain`, seeds and a `wallet.db` reader and importer; M3 is a usable JSON-RPC wallet.

---

## 11. Milestones

| Milestone | Phases | Demo / acceptance |
|---|---|---|
| **M0 — Rig** | P0 | CI, oracle shims, bbolt reader's dump equal to Go's, D1–D10 decided |
| **M1 — Wallet libraries, store and importer** | P1–P3 | keys and addresses equal to Go's across 10⁶ seeds; Go-written wallets of every shape import losslessly; operation-script differential green |
| **M2 — Syncing wallet (RPC mode)** | P4–P6 | Go-created wallet imported, synced through dcrd and dcroxide, sends and receives, in agreement with Go on the original |
| **M3 — JSON-RPC drop-in** | P7–P8 | golden suite (non-deferred methods) and `dcrctl --wallet` sweep green; config and IPC parity; paired-wallet soak |
| **M4 — Staking** | P9 | tickets, VSP, votes byte-equal to Go's on simnet |
| **M5 — SPV** | P10 | mainnet and testnet3 SPV sync equal to Go's; no bans in mixed fleets |
| **M6 — Mixing** | P11 | mixes complete with all-Go peer sessions; blame parity |
| **M7 — gRPC and Decrediton** | P12 | Decrediton unmodified on three operating systems |
| **M8 — Hardened 1.0** | P13 | external review resolved; multi-month paired mainnet soak clean; signed release |

Sizing guidance, not a promise: P3, P7, P11 and P12 dominate. Dropping in-place database compatibility removed what had been a whole L-sized phase (a write-compatible bbolt engine) and the upgrade chain from the creation path; the importer that replaces them is read-only and much smaller. Calendar estimates should come from the team after Phase 1, when the oracle rig gives real porting-velocity data. P9 and P10 are independent of each other after M3 and can run in parallel. P11's protocol work (`sntrup4591761`, `purse-mixclient`, the solver) can start alongside them, but its wallet integration needs P9 and, for mixing in SPV mode, P10; P12's transport spike starts in Phase 0 so that its answer is known long before it is needed.

---

## 12. Working agreements — definition of done for every task

A task is done when: (1) the implementation matches dcrwallet at the pin, with any quirk reproduced or its divergence recorded; (2) the corresponding upstream tests, where any exist, are ported and passing; (3) the behaviour is pinned by oracle-generated vectors or a differential test, not by hand-written expectations; (4) new parsers and state machines have fuzz and property targets in CI; (5) code on the W1/W2 surfaces has been through an adversarial review and lands in a commit the project owner signs; (6) `PARITY.md`, `QUIRKS.md`, the relevant spec and any ADR are updated — a spec found wrong is fixed in the same change; (7) secrets handled by the change are zeroed and unformattable; (8) public items are documented.

Suggested first sprint: the Phase 0 rig with the database dumper and `purse-boltread`, `purse-errors`, `purse-hdkeychain` with the leading-zero vectors, and the two spikes behind D2 and D5. That sequence produces a parity signal within days and forces the two most expensive decisions while they are still cheap.

---

*Reference links: [decred/dcrwallet](https://github.com/decred/dcrwallet) · [decred/dcrd](https://github.com/decred/dcrd) · [decred/vspd](https://github.com/decred/vspd) · [jzbz/dcroxide](https://github.com/jzbz/dcroxide) · [jzbz/dcr-rs](https://github.com/jzbz/dcr-rs) · gRPC API: `rpc/api.proto` and `rpc/documentation/` in the dcrwallet repo · Integration harness: [decred/dcrtest](https://github.com/decred/dcrtest)*
