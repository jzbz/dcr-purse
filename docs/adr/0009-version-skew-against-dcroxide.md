# ADR-0009 — Version skew between dcrwallet's dependencies and dcroxide

- **Status:** Accepted as proposed (decision D7; project owner,
  2026-11-18), the answers proposed under Open points included. It builds
  on facts in use: dcroxide consumed at one pinned revision (ADR-0006)
  and reference Go go1.27.4 (decided by the project owner, 2026-11-18,
  spec 23 §13 item 1).
- **Date:** 2026-11-18

## Context

**The question.** dcrwallet links tagged releases of dcrd's modules;
dcroxide ports dcrd master. A reused dcroxide crate therefore carries code
written against a different version than dcrwallet links. D7 asks how
each difference is found, recorded and handled, and settles the known
ones (brief §9 D7, §10 R9).

**Why now.** D1–D10 are a Phase 0 exit condition (brief §6). Adoptions
are under way: `purse-hdkeychain` links `dcroxide-crypto` and
`dcroxide-dcrec`, optionally `dcroxide-chaincfg`; `purse-dcrutil` links
`dcroxide-chainhash`, `-dcrec`, `-dcrjson` and `-wire`; both take
`dcroxide-base58` as a test-only dependency (`crates/purse-hdkeychain/Cargo.toml:13-38`,
`crates/purse-dcrutil/Cargo.toml:12-34`). The specific choices block
Phase 8 (go-flags, logrotate), Phase 10 (seeders, address manager; the
brief writes "address manager at v3.0.0 semantics (D7)", §6) and Phase 11
(mixing pool).

**The two sides.** dcrwallet at `54b85b33` requires 22 dcrd modules
directly and four more indirectly (dcrwallet `go.mod:7-28`, `:50-53`),
with no `replace` (spec 23 §5.4.2 item 1). dcr-purse consumes dcroxide as
git dependencies at one revision since the first crates (`3237b79`); the
revision is `5c6742f` since `b40513c` (`Cargo.toml`,
`[workspace.dependencies]`). Its dcrd pin is master `46e9df48` (dcroxide
`docs/PARITY.md:3-4`). dcroxide moves that pin at upstream milestones and
on its owner's word, and each move brings frozen batteries due (dcroxide
`docs/PARITY.md:5-7`; dcroxide `docs/adr/README.md`, ADR-0001 entry). An
addition is landed in dcroxide first, then the rev moves: done once, for
the HD entry points in `dcroxide-dcrec` (dcroxide `5c6742f`, whose vectors
were recomputed against secp256k1 v4.4.1 with no difference; dcroxide
ADR-0016, addendum of 2026-11-18).

**The deltas.** Spec 24 §4.1 diffed the module cache and left rows
UNVERIFIED (its O-1, O-2). They were settled for this record in a dcrd
clone with `git diff <module tag> 46e9df48 -- <dir> ':!*_test.go'`.
"Observable" means by a peer, seeder, mixer, RPC client or file reader.

| dcrd module (dcrwallet's version) | dcroxide side | Delta | Observable | Disposition |
|---|---|---|---|---|
| `wire` v1.8.0, `chainhash` v1.0.5, `ripemd160` v1.0.2, `dcrec` v1.0.1, `edwards/v2` v2.0.4, `base58` v1.0.6, `dcrjson/v4` v4.2.0, `container/lru` v1.0.0, `stake/v5` v5.0.3 | tag or in-tree | none (`stake`: a test file) | — | reuse |
| `blake256` v1.1.0, `rand` v1.0.1, `txscript/v4` v4.1.2, `gcs/v4` v4.1.1, `types/v4` v4.4.0, `certgen` v1.2.0 | pin; `types` as v5 | comments; `types` also its module path; `certgen` also `+build` lines | no | reuse |
| `database/v3` v3.0.3 (ind.; imported by `stake/v5` `tickets.go`) | `dcroxide-database`, via `dcroxide-stake` | comments (`driver.go`) | no | reuse, or drop behind brief §5's `dcroxide-stake` feature gate |
| `chaincfg/v3` v3.3.0 | pin | one mainnet seeder added | to the seeder operator | wallet-side shim |
| `secp256k1/v4` v4.4.1 | pin | 16 Go and assembly files (22 with READMEs and proof scripts): 64-bit field backends, `ParsePubKey` as a method (spec 24 §4.1) | none found; HD entry points equal | reuse; re-run (item 4) |
| `standalone/v2` v2.3.0 | pin | `subsidy.go` cache-insert guard | no (same values) | reuse; re-run |
| `mixing` v0.7.5 | in-tree | none | yes: dcroxide's three pool rules | parameterise in dcroxide |
| `addrmgr/v3` v3.0.0 | `addrmgr` v4 | new major (below) | see below | see Decision 5 |
| `connmgr/v3` v3.1.4 | rewritten | whole package; ban-score constants equal | no, for the two items used | reuse (spec 24 §3.16) |
| `hdkeychain/v3` v3.1.3, `dcrutil/v4` v4.0.3, `mixclient` v0.7.5 | not ported | — | — | own port |
| `blockchain/v5` v5.1.1, `rpcclient/v8` v8.1.0 | not used | — | not linked (spec 24 §3.12, §3.21) | none |

Two neighbouring libraries differ (spec 24 §4.1): go-flags v1.5.0 against
dcroxide's v1.6.1 layer, logrotate v1.0.0 against its v1.1.2 rotator
(dcrwallet `go.mod:34`, `:36`; dcroxide `crates/dcroxide-node/src/flags.rs:2`, `logrotate.rs:2`).
`slog`, `go-socks` and `gorilla/websocket` are the same releases.

A second axis is the Go toolchain (spec 24 §4.1, last paragraph).
Behaviour taken from Go's standard library follows the reference Go:
go1.27.3 for dcroxide (dcroxide `.github/workflows/ci.yml:42`), go1.27.4
for dcr-purse (`crates/purse-testutil/src/lib.rs:36`). dcroxide's
Go-JSON error texts are still Go 1.26's, an interim divergence there.
`purse-dcrutil` already takes Go's `FormatFloat` from `dcroxide-dcrjson`
(`crates/purse-dcrutil/Cargo.toml:18-20`). dcroxide's tie check covers
`strconv` across 1.27.0, 1.27.1 and 1.27.3, not 1.27.4 (dcroxide
`docs/adr/README.md`, ADR-0007 entry); `purse-dcrutil`'s differential
against the go1.27.4 oracle will settle it.

**What the oracle enforces today.** `tools/oracle/go.mod` requires
dcrwallet at the pin, so every module it links resolves to dcrwallet's
version (spec 23 §5.4.2). Modules no oracle command imports are absent
from it: `addrmgr`, `connmgr`, `certgen`, `blockchain/v5`, `rpcclient`,
go-flags (`tools/oracle/go.mod:5-53`). One differential crosses the skew:
`purse-hdkeychain`'s (`tests/oracle_diff.rs`) drives `dcroxide-dcrec`'s
HD entry points and `dcroxide-crypto`'s hashes against hdkeychain v3.1.3
on secp256k1 v4.4.1; it covers the crate's own base58 codec, not
dcroxide's. Spec 23 §5.4.2 item 2's check is not built: `purse-testutil`
checks only the dcrwallet pin (`crates/purse-testutil/src/lib.rs:469-477`),
and `build.info` reports only the Go release. Spec 23 §5.4.2 item 4 and
§12.2 require re-running every reused crate's differentials against the
wallet oracle. dcroxide's handlers are `package main`
(dcroxide `tools/oracle/*.go`), and its Rust differentials spawn
dcroxide's own oracle through `dcroxide-testutil`.

**The specific differences.**

- *Seeders.* dcroxide lists four mainnet hosts
  (dcroxide `crates/dcroxide-chaincfg/src/mainnet.rs:286-291`), chaincfg v3.3.0
  three (`mainnetparams.go:604-608`); dcrwallet queries each
  (`p2p/peering.go:348-366`; QK-0679). For four: no peer sees a seeder
  query, so W3's parties (brief §3) are untouched; dcrd lists the host;
  it adds bootstrap resilience. Against: the seeder operator observes
  every request, and spec 23 §5.4.2 item 4 already requires v3.3.0's list.
- *addrmgr.* A reading of `git diff addrmgr/v3.0.0 46e9df48 -- addrmgr`
  (non-test: 686 insertions, 43 deletions; v3.0.0's source is about
  1,900 lines) finds, for the five functions spec 14 §12 item 6 names:
  `addOrUpdateAddress`, `expireNew`, `Good` gain per-bucket statistics
  only; `GroupKey` gains a Tor v3 branch; `GetAddress` takes a type filter
  and draws among non-empty buckets, where v3 draws any bucket and
  retries. Outside them: `IsRoutable` drops ORCHIDv2, 2001:20::/28
  (`network.go:285-289` at the pin, against v3.0.0 `:221-226`), and the
  loader keeps a Tor v3 entry where v3 rejects it and deletes the file
  (`addrmgr/v3@v3.0.0/netaddress.go:151-168`, `addrmanager.go:493-511`).
  With an IPv4/IPv6 filter the draw has v3's distribution but consumes
  the random stream differently, and Q-20(c)'s spin on stale counters
  (spec 14 Q-20) cannot occur. Q-20(a) and the counter half of (c) are in
  v4 too; dcroxide reproduces them (dcroxide QK-0004, QK-0023). dcrwallet
  speaks protocol 11 and never learns a Tor v3 address
  (`p2p/peering.go:48`). This is a reading, not a differential.
- *Mixing pool.* dcroxide's pool adds three rules (64 key exchanges per
  identity, one message per type also for un-orphaned messages, an expiry
  latch; dcroxide `crates/dcroxide-mixing/src/mixpool.rs:9-15`, `:225`).
  A wallet pool refusing what dcrwallet's accepts changes which peers a
  mix sees (QK-0689); W3 asks the wallet to be indistinguishable.
- *go-flags.* Only the integer base reaches dcrwallet's options: v1.5.0
  parses base 10, v1.6.1 base 0 (`go-flags@v1.5.0/convert.go:223,237`;
  spec 21 §12.2). v1.6.1's boolean values are opt-in and unused
  (dcroxide `flags.rs:1561`); the help and map changes need commands or
  map options dcrwallet lacks (`config.go:394-428`).
- *logrotate.* dcrwallet calls `rotator.New(logFile, logSize, false, 0)`
  (`internal/loggers/loggers.go:74`). maxRolls = 0 means no limit: every
  roll is kept and the pruning loop, guarded by `maxRolls > 0`, never runs
  (v1.0.0 `rotator.go:190`, v1.1.2 `:216`). Both gzip at the default
  level. What remains is v1.0.0 compressing without a lock, invisible in
  file contents (spec 21 §12.2).
- *JSON-RPC types.* With identical source, the brief's field-by-field
  comparison is a fidelity question for `dcroxide-rpctypes`, owned by
  spec 16 and Phase 7's golden captures, not a version question.

## Options considered

1. **No ledger; rely on the mandatory wallet-oracle differentials** (brief
   §4 item 2, §7). Costs nothing extra. But reused dcroxide code is
   covered only where a wallet crate drives it; policy (the seeder list,
   pool rules) escapes function-level differentials; and nothing records
   why a delta was judged harmless.
2. **Port every module at dcrwallet's version.** No skew by construction;
   duplicates about 17 tested crates, loses dcroxide's fixes (spec 24
   §12.3, form C), and widens what `multiple-versions = "deny"` must keep
   single (`deny.toml:23`).
3. **A recorded diff per module, then reuse, parameterise or shim** (the
   brief's recommendation). Costs a ledger, a redo at every pin move, and
   two-repository changes. Keeps one implementation per package.
4. **dcroxide tracks dcrwallet's tags.** Removes the skew at the source;
   contradicts dcroxide's target, master until v2.2 then dcrd tags
   (dcroxide ADR-0013).

For the address manager alone: (a) reuse `dcroxide-addrmgr` unchanged;
(b) reuse it with a v3 policy value added in dcroxide; (c) port v3.0.0
in dcr-purse, about 1,800 lines of Go plus a differential, with exact v3
semantics, no dcroxide change, and no coupling to dcroxide's addrmgr.

## Decision

Option 3, operational. It confirms the brief and sharpens it in the
ledger's form and check and the dispositions. It departs from spec 23
§5.4.2 item 4 in the re-run rule (item 4).

1. **Ledger.** `docs/SKEW.md`, linked from PARITY.md. Header: dcrwallet
   commit, dcroxide rev, its dcrd pin, both reference Go releases. One row
   per dcrd module dcrwallet's `go.mod` names, the two neighbours and the
   Go toolchain: both versions, the diff command, the delta, observable or
   not, disposition, covering differential. Dated sections record redos.
2. **Who and when.** The change moving either pin (dcrwallet's under D12,
   dcroxide's rev, or either reference Go) carries the redo, run by its
   author from a dcrd clone (the module cache holds nested modules only at
   their tags, spec 24 §4.1); the owner reviews it in the signed commit.
3. **Checks** (spec 23 §5.4.2 item 2). (a) A verbatim copy of dcrwallet's
   `go.mod` at the pin, `tools/oracle/dcrwallet.go.mod`, whose Go `h1:`
   hash a test compares with `tools/oracle/go.sum:4`; the ledger's
   dcrwallet column must equal its requirements. (b) `build.info` reports
   `debug.ReadBuildInfo` dependencies; no module may resolve above (a). A
   ledger row absent from the oracle's graph passes and shows "no
   differential yet". (c) The header matches `DCRWALLET_PARITY_COMMIT`,
   the rev in `Cargo.toml` and both `GO_VERSION` values.
4. **Re-runs.** Before a wallet path through a reused crate is marked
   pinned in PARITY.md, dcroxide's differential classes that path reaches
   run against the wallet oracle when the crate's module, or a module it
   calls, has a code delta, or when it carries Go standard-library
   behaviour and the reference Go releases differ. Today: secp256k1
   (signing, recovery, Schnorr, key parsing; HD entry points done), the
   subsidy cache, and `dcroxide-dcrjson`'s float formatting. Rows with no
   or comment-only delta are not re-run. This narrows spec 23's "every
   dcroxide crate": identical source behaves identically, and each re-run
   needs handler code compiled against dcrwallet's versions.
5. **Dispositions.**
   - *Seeders:* chaincfg v3.3.0's three hosts, following dcrwallet's list
     as its pin moves; `purse-p2p` takes them from a constant, not from
     `dcroxide-chaincfg`. Testnet lists are equal.
   - *addrmgr:* option (a), reuse `dcroxide-addrmgr` with an IPv4/IPv6
     filter, the wallet owning load and save (spec 24 §3.15). ORCHIDv2
     routability, the kept Tor v3 entry, the draw's use of the random
     stream and the absent Q-20(c) spin are recorded as divergences under
     ADR-0012: nobody observes them (no ORCHIDv2 address is reachable;
     neither wallet writes a Tor v3 entry; Go's `math/rand/v2` is randomly seeded; spec 14
     §12 item 5 rules out the hang). Precondition: a Phase 10
     differential of the five functions against v3.0.0. A delta beyond
     these switches the disposition to option (c).
   - *Mixing pool:* `dcroxide-mixing`'s client pool without the three node
     rules, already in brief §5.
   - *go-flags, logrotate:* assume brief §5's extraction into dcroxide
     crates (spec 24 D-2). The go-flags crate takes a version-quirk
     parameter; dcr-purse selects v1.5.0, base-10 integers; Phase 8's
     config suite is the differential. The v1.1.2 rotator is reused as is
     with maxRolls = 0 (no limit: every roll kept, the pruning loop never
     runs) and gzip.
6. **Pin moves.** If dcrwallet's `go.mod` requires a dcrd module newer
   than dcroxide's pin carries, dcroxide's pin moves first, on its owner's
   word; failing that, the row becomes a shim or own port with the owner's
   approval. A rev move that dcr-purse needs for an addition but that
   widens an observable delta lands only with that row's shim or dcroxide
   parameter. D12's "dcrwallet and dcrd pins" are read as dcrwallet and
   the acceptance dcrd release (brief §6, Phase 0); dcroxide's rev is a third
   pin, moved under this item.

## Consequences

- **Work created:** `docs/SKEW.md`, its checks and the `go.mod` copy;
  `build.info` dependency reporting; a per-module diff script under
  `tools/`; in dcroxide, oracle handlers moved into an importable package
  (or copied, with a sync rule) and compiled against secp256k1 v4.4.1, and
  `dcroxide-testutil` able to spawn an external oracle (or the tests ported
  here); the go-flags version parameter within its extraction; the Phase 10
  addrmgr differential. No new additive dcroxide change: the mixing pool
  is in brief §5.
- **Work avoided:** a v3 addrmgr port and a dcroxide policy value, unless
  the differential finds more; a skew review of the JSON-RPC types,
  `mixing`, `dcrjson`, `certgen` and `lru` (identical, or differing in
  comments and build lines); separate go-flags and logrotate ports.
- **Risks accepted:** comment-only classification is a reviewer's
  judgement where dcroxide's oracle uses byte identity (dcroxide
  `tools/oracle/go.mod:39-49`). secp256k1's delta is covered only by
  dcroxide's differential until item 4's re-run lands. The addrmgr
  disposition rests on a reading until Phase 10.
- **Constrains:** `purse-p2p`, `purse-config`, `purse-logging`, the
  mixing integration, and every "pinned" status through a reused crate
  with a delta. Until dcroxide moves to redb 5 or the `dcroxide-stake`
  gate lands, linking `dcroxide-stake` or `dcroxide-gcs` puts redb 4 and
  5 in one graph, which `deny.toml:23` refuses; one must land before SPV
  filters or staking adopt them (ADR-0003).
- **Documents to amend on acceptance:** brief §2.1 and D7 (logrotate,
  types), §8 Config row (booleans); spec 24 §4.1 UNVERIFIED rows, O-1,
  O-2, D-4, D-5; spec 23 §5.4.2 item 4 (re-run rule); spec 14 §12 item 6;
  PARITY.md go-flags and logrotate rows (ledger link); QUIRKS.md
  QK-0679 ("Disposition: diverges from dcroxide; v3.3.0's three hosts, a
  `purse-p2p` constant") and QK-0689 ("Disposition: dcrd v0.7.5's rules;
  the client pool omits the three node rules").
- **Revisit when:** dcrwallet's pin requires new dcrd tags; dcroxide moves
  to dcrd tags (ADR-0013); a differential finds a skew the ledger called
  unobservable.

## Open points

All seven were decided as proposed (project owner, 2026-11-18). They are
kept as the record of what was asked and answered.

1. Where does the ledger live? Proposed: `docs/SKEW.md`; it changes on
   pins of its own. Alternative: a PARITY.md section.
2. Which mainnet seeder list? Proposed: v3.3.0's three hosts, as spec 23
   §5.4.2 item 4 requires.
3. Reuse `dcroxide-addrmgr` and record four divergences, or port v3.0.0?
   Proposed: reuse, conditional on the Phase 10 differential.
4. Version parameter in the extracted go-flags crate, or a separate
   v1.5.0 port? Proposed: the parameter; the integer base is the only
   difference reached.
5. Which rows need a wallet-oracle re-run? Proposed: code deltas and the
   Go-toolchain rule only. Alternative: spec 23's rule, every reused crate,
   including the five with comment-only deltas.
6. Is the wallet's mixing pool held to dcrd v0.7.5's rules? Proposed:
   yes; the node keeps its rules.
7. May dcr-purse request additions on a dcroxide branch at an older pin?
   Proposed: no; additions land on dcroxide master (ADR-0006), and a
   widened skew is handled by item 6.

## Addendum, 2026-11-18 — the first ledger

The first `docs/SKEW.md` was written and independently re-derived the same
day. Three things it found bear on this record:

- **Decision item 5, addrmgr.** The reason given, that ORCHIDv2 routability
  is a divergence nobody observes, is wrong. dcrd's addrmgr v3.0.0 stores a
  gossiped 2001:20::/28 address and may draw it as a dial candidate; the v4
  generation dcroxide ports drops it, and the IPv4/IPv6 filter does not
  remove the difference. It is observable in the peers file (W7) and in the
  candidate set (W3). The project owner accepted it as a recorded divergence
  (2026-11-18); the disposition, reuse of `dcroxide-addrmgr`, stands.
- **Context table, dcrec.** dcrec v1.0.1 is not reused from dcroxide:
  `purse-dcrutil` defines `SignatureType` itself. The ledger has the row
  right.
- **The toolchain axis has a second value.** Besides the Go release,
  GODEBUG defaults differ per main module. dcrwallet's daemon builds with
  `default=go1.25`; the oracle now does too, by a `godebug` line in
  `tools/oracle/go.mod` (project owner, 2026-11-18).

The mainnet seeder dcrd added after chaincfg v3.3.0 is `dcr-seed.jz.bz`; the
ledger names it at the owner's direction.
