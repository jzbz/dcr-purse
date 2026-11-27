# ADR-0014 — Performance as a goal

- **Status:** The goal is **Accepted** (project owner, 2026-11-19): "yes
  faster spv sync is an important goal. speed everywhere actually". Its
  precedence over being indistinguishable from dcrwallet on the network is
  **Accepted** (project owner, 2026-11-19): "let's reverse that, faster is
  more important". Open points 1 to 12 were all decided as recommended
  (project owner, 2026-11-19); each is marked. Every number is a starting
  value, fixed from measurements; items 13 to 22 are not yet established
  *(item 22's query results were measured on 2026-11-20 on the shared
  desktop and N was set from them on 2026-11-21; the run of record is
  still owed: the dated note under item 22)*. *2026-11-26: that run of
  record was made: the addendum "The store's run of record".*
  The owner's decisions on the Phase 1 speed row and the first
  measurements' findings (2026-11-19) are the addendum of that date, "The
  Phase 1 speed row and the first ledger entry".
  *Phase 1's speed row was measured again on 2026-11-20 at the dcroxide
  pin `2074168`: the addendum of that date. The pin moved to `48f5b5e` on
  2026-11-21, which changes `dcroxide-base58`'s conversion and not its
  outputs (`docs/SKEW.md`, 2026-11-21 section); the row has no
  measurement of record at that pin.* *On 2026-11-21 the owner set N
  for the row of Phases 2 and 3 and decided what counts as the dedicated
  machine; no timed run of record is started until the owner says when
  and where: the addendum of that date.* *Phase 1's dedicated-machine run
  was made later on 2026-11-21 and met every clause of the row, base58check
  decode at 0.78: the addendum "Phase 1's run of record". The row of
  Phases 2 and 3 has not had its dedicated-machine run.* *Also on
  2026-11-21 the owner decided that the store is to keep no committed
  filter the wallet fetches below its birthday, which widens S15 (Open
  point 5): ADR-0019, decided, not implemented.* *On 2026-11-22 the owner
  decided ADR-0019's FB1 and FB2 as recommended: the initial sync fetches
  no filter below the birthday however set, and rescans and discoveries
  from below it in SPV mode fetch on demand and keep none (Open point
  5).* *The dcroxide pin moved to `1587ca1` on 2026-11-23, which only adds
  `dcroxide-wire`'s `pk_script_locs`, called by no dcr-purse code, so no
  measured path changed; no measurement of record was made at that pin,
  and the gate baselines are unchanged (`docs/SKEW.md`, 2026-11-23
  section).* *On 2026-11-24 the owner said when the store's
  dedicated-machine run for the row of Phases 2 and 3 is made: after the
  next transaction-store slice, so that input selection is measured too,
  and not before; it has not been made: the addendum of that date.*
  *2026-11-26: the store's dedicated-machine run for the row of Phases 2
  and 3 was made, after that slice, on the project owner's word that the
  machine was quiet: every clause of the row met, a balance 33,966 times
  faster at the lowest, a range query 1.82 times, and none of the 21
  figures slower than Go, the worst Rust's time 0.617 of Go's (the
  tip-block commit). It settles ADR-0003's Open point M2; its balance,
  range and commit rows still run the measurement's own composition of
  the transaction store's rows, and only input selection and
  `RandomUTXO` run the transaction store's code, so the measurement with
  that store's own functions (ADR-0003 Open point M3, at Phase 3's exit)
  is not done and stays owed: the addendum "The store's run of record".*
- **Date:** 2026-11-19

## Context

**What the plan said.** Speed was checked late: Phase 13 lists
"Performance against Go: open time ..., SPV initial sync, RPC latency on
large wallets, memory" (brief §6). Earlier gates: Phase 2's schema ADR
records measured gains against dcrwallet; multi-client RPC concurrency is
measured against Go at Phase 7's exit (ADR-0008 Open point 4, decided); the
mixing solver must fit the stage deadline at 64 peers (brief §9 D8,
ADR-0010). §4 has no speed principle; §8 chooses "a purpose-built harness
and a bench ledger". W3 required "the same peer: protocol version 11 ...
the same handshake, requests, timeouts and relay behaviour" (brief §3).
ADR-0013 makes seed restore the only migration path, so restore speed
reaches every user who switches.

**Evidence** (2026-11-19; recorded in full as the ledger's first rows,
Decision item 4):

- **[S]** dcrwallet `54b85b33` SPV sync from genesis, mainnet (height
  1,133,862) and testnet3, each a new wallet restored from a random seed
  without birthday, 8 peers (dcrd 2.1.x, protocol 11); then full rescans at
  42, 302 and 40,002 addresses. pprof CPU, block and mutex profiles;
  `/proc` and TCP counters every 5 s; every P2P message logged. A 32-thread
  x86-64 desktop, NVMe, btrfs over disk encryption, load average 20–55,
  traffic through a VPN (median RTT 123 ms). CPU-seconds are robust to the
  load; CPU-bound wall times are not.
- **[H]** Rust 1.100.0 (wallet release profile) against Go 1.27.4 at
  dcrwallet's module versions; medians of 14 alternating repetitions;
  outputs cross-checked equal; loaded 16-core desktop (ratios ±10–20%).
- **[R]** Source reading: dcrwallet `54b85b33`, dcrd `46e9df48` and
  `release-v2.1.7`, dcroxide's serving crates. Nothing run.

**Where dcrwallet's sync spends its time** (mainnet [S]; testnet3 2,320 of
2,694 s in headers and filters):

| Stage | Wall | CPU (cores) | Bound by |
|---|---|---|---|
| Startup | 2.7 s | 0.2 s | scrypt |
| Headers + cfilters | 2,030 s | 167 s (0.08) | round trips, pipeline shape |
| Discovery, two coin-type rounds | 103 s | 612 s (5.9) | `MatchAny` 96% of CPU (decoding 43%); block request outstanding 61% |
| Rescan from genesis, 42 addresses | 127 s | 27 s | read transaction per filter; fsync per range |
| **Total** | **2,263 s** | 807 s | headers + cfilters 90% |

- **One getheaders in the wallet:** stage 1 fetches one 2,000-header batch
  at a time (`spv/sync.go:1666-1711`); 1,085 s of wire time (p50 1.31 s).
- **A barrier per 2,000 filters:** 20 getcfsv2 of 100
  (`spv/backend.go:157-167`), waiting for the slowest. p50 893 ms, p99
  5.2 s; about 6.7 in flight (559 filters/s at 1.2 s mean), up to 7–8 per
  peer.
- **Commits stall fetchers:** the chain switch holds `sidechainMu` across
  one bbolt update per batch (`spv/sync.go:1795-1813`); fetchers need it
  (`spv/backend.go:249-253`); header validation holds it too (`:347-391`).
  Commits 325 s; lock waits 2,161 s summed over goroutines.
- **Disk:** 15.8 GB written for a 1.42 GB file; header and filter tables
  keyed by hash (spec 03, lines 642, 657); fdatasync about 0.75 s.
- **Bytes:** 205 MB headers, 453 MB filters (about 40 KB per reply).
- **Peers:** 7 of 8 dropped at the end by the straggler check
  (`spv/sync.go:1588-1607`).

**Rescans** [S] (`spv/backend.go:523-601`: match a range, one getdata to one
peer, next range): 42 addresses cold, 186 s (per-block read transactions
48%, cold-file page faults 35%); 302 warm, 117 s, 81% waiting on blocks;
40,002 warm, 1,300 s, 55,998 false-positive blocks, 980 s waiting and 320 s
matching, never overlapped.

**Serving nodes** [R] (dcrd `46e9df48`; v2.1.7 and dcroxide differ where
stated): getheaders and getcfsv2 carry no ban score or rate limit; more
than 100 filters goes unanswered (`internal/blockchain/headercmt.go:210-245`).
Each peer's messages are handled one at a time, replies in order
(`peer/peer.go:1535-1593`). Queued replies: disconnect above 40 MiB
(`peer/peer.go:43`; none in v2.1.7); dcroxide drops replies above 128
messages or 4 MiB (`peerloop.rs:722`, `:749`). getdata: ban score
floor(n·99/50000), 0 at n ≤ 505 (`server.go:1434`), served serially. A
second getinitstate bans (`server.go:1229-1236`). Connections: 5 per host;
inbound 0.2/s per group, burst 3; loopback and whitelisted exempt
(`internal/connmgr/connmanager.go:93-94`, `:1009-1012`, `:1649-1654`).
Protocol 12 adds only addrv2 (`wire/protocol.go:60-63`).

**Rust against Go** [H] (Go time ÷ Rust time). Faster: public derivation
1.9x; header checks in dcrwallet's order 6.5x; `MatchAny` 1.35x (W=100) to
4.5x (W=10⁶); block decode 5–7x; verify 4.4x; redb 2.4x per durable commit
over bbolt. Slower: generator times a secret (private and hardened
derivation) 0.53–0.72x; RFC6979 signing 0.58x; BLAKE-256 0.6–0.94x (0.03 s
per chain). Go is variable time on secrets, signing nonce included
(`dcrec/secp256k1` v4.4.1 `ecdsa/signature.go:599`); k256's precomputed
tables took Rust's constant-time multiplication from 36.8 to 19.2 µs (Go
23.1 µs).

## Decision

### 1. The principle and its precedence

Speed is a design input everywhere. In a conflict this order decides,
without asking except at tier 3:

1. **Never traded:** W1, including crash safety: anything a client, peer or
   VSP can see the effect of (an address handed out, a transaction
   broadcast, a notification sent) is durable before that effect;
   constant-time arithmetic on secrets and their wiping (brief §4
   principle 4); byte-compatible RPC results (W4, W5); the W2 restore
   outcome; StakeShuffle and the VSP request sequence exactly as
   dcrwallet's.
2. **Never traded: interoperability.** Every dcrd and dcroxide node tested
   serves dcr-purse correctly: nothing gets it banned, throttled,
   disconnected or deprioritised, needs a message or version they refuse,
   or loads a peer as an operator would call a denial of service.
3. **The owner decides, case by case: the owner's privacy.** Anything
   telling a peer or observer more than dcrwallet's method does about which
   addresses, transactions or blocks are the wallet's, or its birthday.
4. **No longer a constraint: blending in.** A wire-visible speed-up with a
   real gain is done and recorded in `PARITY.md` (ADR-0012 Rules 10, 11)
   as "(project owner, 2026-11-19, ADR-0014)".
5. **Profile first.** Nothing lands without a measurement naming its
   bottleneck and a ledger row showing the gain; "not established" means
   measure first.

### 2. SPV initial sync

Gain: **M** measured, **E** estimated. Class [R]: **LOCAL** (only gaps
change), **TIMING** (more in flight), **SHAPE** (another pattern),
**OUT** (refused or penalised).

| # | Candidate | Gain | Class | Disposition |
|---|---|---|---|---|
| S1 | Rust header checks; pending batch not re-validated (`spv/backend.go:348-353`) | M 6.5x [H] | LOCAL | DO |
| S2 | Next locator before the batch validates | E | TIMING | DO; nothing used unvalidated |
| S3 | Chain switch and validation out of the fetchers' lock | M: waits 2,161 s, commits 325 s | LOCAL | DO; commit contents only per Open point 2 |
| S4 | redb; main-chain headers and filters keyed by height | M: redb 2.4x [H]; random-key amplification not measured on redb | LOCAL | per Open point 6 |
| S5 | Headers run more than one batch ahead | E: stage near 1,085 s | TIMING | DO |
| S6 | Sliding window of K getcfsv2, no barrier; none to the header peer while its getheaders is outstanding | E: K=40, ≤6 per peer, ~340 s, upper bound | TIMING | DO; ~240 KB queued per peer |
| S7 | One warm header peer, rotated on a schedule | not established | SHAPE | measure first |
| S8 | Parallel header streams from shipped anchors, as locator hints only | E: header time ÷ streams | SHAPE | DO; a segment is used only once linked to genesis and validated |
| S9 | Peers beyond 8 carry only header and filter requests; relay and block getdata stay on 8 | not established (8 × 6 ≥ K) | SHAPE | measure first; unsplit: Open point 9 |
| S10 | Requests to the fastest peers, a random share kept | E | SHAPE | DO, with Risks' mitigations |
| S11 | Hedged request past a latency threshold | not established after S6 | SHAPE | DO only if the tail still bounds throughput after S6 |
| S12 | Peers kept after sync, heights from headers | M: 7 of 8 dropped | SHAPE | DO |
| S13 | getcfsv2 always, never 2,000 pipelined getcfilterv2 (`p2p/peering.go:1787-1811`) | avoids dcroxide's drop | SHAPE | DO |
| S14 | Several connections to a node the user names | E | SHAPE | opt-in: ≤5 to a loopback or whitelisted node, else ≤3, ≥5 s apart |
| S15 | No filters below a typed birthday (`spv/sync.go:1736-1748`) | E: most of 453 MB for a recent date | SHAPE | DO (Open point 5) *(2026-11-21, decided, not implemented: the store is to keep no fetched filter below the wallet's birthday however set, ADR-0019; whether the sync fetches them for a birthday nobody typed is its open question FB1)* *(2026-11-22: it does not, in either mode, FB1, project owner; not implemented)* |
| S16 | Several connections to a public node; >100 filters; second getinitstate; getdata >505 | none | OUT | REJECT |
| S17 | Protocol 12; SIMD BLAKE-256 | none; 0.03 s | — | REJECT |

**What "faster" can mean** (mainnet seconds; rows past the first are
**upper bounds**: they assume latency per request holds at K = 40, and
since each node answers one peer's messages in order, they rely on S6's
header-peer rule; Open point 13):

| Option | Headers + filters | Discovery | Rescan | Total | ÷ dcrwallet |
|---|---|---|---|---|---|
| dcrwallet [S] | 2,030 | 103 | 127 | 2,263 | 1x |
| LOCAL rows (S1, S3, S4, R3–R5)¹ | 1,700–1,950 | 65–80 | 15–30 | 1,790–2,070 | **1.1–1.3x** |
| + wire-visible DO rows but S8 | ~1,085 | 10–30 | 5–15 | 1,100–1,135 | **≤ 2x** |
| + S8 | 400–700 | 10–30 | 5–15 | 420–750 | **≤ 3–5x** |

¹ R5 and S3's commit part only under Open point 2.

Local work alone gives little: 90% of the time is a pipeline at 0.08
cores. Discovery cells: R1 removes a round and its re-fetches, R2 cuts
~28 passes per round to 1 + F, R3 cuts CPU 1.35–4.5x, R6–R8 overlap and
spread the block waits. Measured over 120–200 ms links; on a fast link or
the user's own node (S14) the ratio is unknown.

### 3. Restore, rescan and other hot paths

| # | Candidate | Gain | Class | Disposition |
|---|---|---|---|---|
| R1 | One discovery for both coin types; SLIP0044 reuses legacy's passes and blocks (`wallet/discovery.go:743`, `:954-979`) | M: rounds 65 s, 37 s; E: ~half | SHAPE | DO |
| R2 | Remaining "not found" probe chain matched in one pass, bisection replayed | E: 1 + F passes, not ~28 | SHAPE | DO; same decisions and outcomes |
| R3 | Rust `MatchAny` on all cores | M 1.35–4.5x | LOCAL | DO |
| R4 | Rescan filters in one read transaction (`spv/backend.go:528-535`) | M: 48% cold; page faults (35%) are I/O, kept | LOCAL | DO |
| R5 | Rescan marker (`wallet/rescan.go:340-345`) relaxed | M: fsync 42% of the thread | LOCAL | per Open point 2 |
| R6 | Matching overlaps fetching; next range prefetched | E: up to 320 s at 40,002 | TIMING | DO |
| R7 | Matched blocks split across the 8 peers, ≤500 per getdata | M: waiting 75–81% | SHAPE | DO |
| R8 | Discovery's blocks kept for the rescan | M: 5,418 blocks | SHAPE | DO |
| R9 | Discovery resumable; counters commit at the end (spec 09 §4.16.1) | on interruption | LOCAL | DO |
| R10 | Block request duplicated to a second peer | tail only | SHAPE | REJECT (Open point 10) |
| O1 | Secret generator multiplication via k256 tables (dcroxide-dcrec `secp256k1.rs:375`, `ecdsa.rs:538`, `schnorr.rs:223`) | M: 36.8 → 19.2 µs; E: signing 1.02–1.11x Go's time | LOCAL | per Open point 8; *done in dcroxide `5ace7a2`, in the pin since 2026-11-20: Phase 1's secret paths 0.84–0.86 of Go's time (addendum of 2026-11-20)* |
| O3 | Public derivation: cached parent, batch inversion | E: ~15% | LOCAL | when profiled |
| O4 | scrypt in parallel with store open and seeding | M: 0.74 of 2.7 s | LOCAL | DO |
| O5 | Cheaper scrypt for new wallets | — | — | REJECT (Open point 11) |
| O6 | Maintained balances; `AccountBalances` not quadratic; unused `NewBalances` dropped (specs 06 Q14, 08 Q13) | not measured | LOCAL | DO in ADR-0003 |
| O7 | Range queries, `findEligibleOutputs` by index | ADR-0003 | LOCAL | DO; order kept |
| O8 | Notifications outside write transactions | — | LOCAL | brief D11's ADR; *ADR-0015 (accepted 2026-11-19): generation stays inside, delivery is a push onto a queue; holding until commit not in 1.0 (its Open point 3)* |

REJECTED: Go's variable-time secret arithmetic (tier 1); sorting the query
set once per rescan (the SipHash key changes per block,
`spv/sync.go:1322`; the no-sort variant gains ≤20% [H]); a smaller watch
set (W2); lazy child public keys (small gain [H]).

**R2's estimate.** A branch bisects 2³¹/20 segments in about 27 probes
(`wallet/discovery.go:209-314`). A probe after "not found" is fixed in
advance; one after "found" is not, since blocks must rule out false
positives. Matching the remaining not-found chain in one pass gives 1 + F
passes per round, F the "found" outcomes, at most about log2(s+1)+1 for
highest used segment s: 1–2 passes below index 20, ≤6 below 320. Scripts
per pass grow as passes shrink, so CPU saved is about the decoding share
(43%) plus the sequential round trips.

At 40,002 addresses, R3, R6 and R7 are estimated to take the rescan from
1,300 s to 250–400 s.

### 4. Enforcement

- **Harness.** `purse-bench` (not shipped) and a Go counterpart in
  `tools/oracle` linking dcrwallet's packages, on the same micro, store
  (ADR-0003's) and end-to-end workloads. End-to-end runs end when the
  logical export (ADR-0001) equals Go's, not at a log line. SPV arms: at
  least 8 local serving nodes (dcrd release and pin, dcroxide), one
  connection each except in a labelled S14 arm (loopback skips dcrd's
  limits); a relay at 20, 50 and 150 ms RTT as dcroxide used (its
  `docs/bench-ledger.md`, "A delayed link"), plus bandwidth-cap and loss
  arms, because a pure delay makes throughput scale with requests in flight
  by construction; a scheduled public testnet3 and mainnet arm with Go in
  the same session; counts of bans, disconnects, stalls, dropped replies
  and per-peer queueing.
- **Ledger.** `docs/bench-ledger.md` under dcroxide's rules: append-only;
  machine, both commits, corpus, network path, raw data; Go and Rust
  alternating in one session, ≥3 runs, median and range. Memory is
  anonymous RSS or peak heap on both sides; Go's 2.66 GB `VmHWM` [S]
  includes its memory-mapped database.
- **Exit criteria** against Go, same machine and session; all numbers
  starting values (Open points 1, 4, 12). "Secret paths" (Go fast through
  variable-time secret arithmetic) are bounded at ≤1.15× Go's time, never
  "no slower".

  | Phase | No slower than Go on | ≥N times faster on |
  |---|---|---|
  | 1 | public derivation, address, script, HMAC, hash160, base58; secret paths (master, hardened and private child with public key) ≤1.15× | 20,000 addresses, N=1.5 (measured 1.9) |
  | 2–3 | ADR-0003's workload | balance, range queries (N from ADR-0003) |
  | 4 | `createtx` end to end; signing ≤1.15× (now 1.72×) | — |
  | 5 | discovery, rescan; peak memory | discovery + rescan from genesis, N=3, local work only |
  | 6, 8 | RPC-mode sync and rescan; open to RPC ready | — |
  | 7, 12 | p50/p99 per method, 1 and 8 clients (bounds on ADR-0008 Open point 4's measurement); secret paths ≤1.15× | — |
  | 10 | peak memory; zero bans, disconnects, dropped replies | sync from genesis at 20–150 ms (Open point 1) |

  *2026-11-20: Phase 1's row measured again at the dcroxide pin
  `2074168`; the secret paths meet it, base58 decoding is still at parity
  (addendum of 2026-11-20). Signing two inputs through `purse-txauthor`
  measured 0.86 of Go's time at the same pin, indicatively (best rounds,
  not this item's method; bench ledger, "Signing at dcroxide `2074168`"),
  against the 1.72× in Phase 4's row.*

  *2026-11-20: the row for Phases 2 and 3 measured for `purse-store` as
  built against dcrwallet's udb on one generated wallet, on the shared
  desktop (ADR-0003, addendum "`purse-store` against dcrwallet on the
  same workload"; bench ledger, entry of that date). No figure was slower
  than Go. A balance was 36,800 to 102,500 times faster, and a range over
  blocks 1.86 to 2.05 times. N is not set yet (ADR-0003 Open point M1), and
  the dedicated-machine run is owed (its Open point M2).* *Decided
  2026-11-21 (project owner): N = 1,000 for a balance query and N = 1.5
  for a range query; the dedicated-machine run before Phase 3's exit,
  when and where the owner says (addendum of that date).* *2026-11-24
  (project owner): that run after the next transaction-store slice, so
  that input selection is measured too, and not before (addendum of that
  date); not made.* *2026-11-26: made on the dedicated machine after that
  slice; the row met on every clause, a balance 33,966x and a range query
  1.82x faster at their lowest, no figure slower than Go (the worst 0.617,
  the tip-block commit); M3, the measurement through the transaction
  store's own functions, stays owed (addendum "The store's run of
  record").*

  *2026-11-21: Phase 1's row met on every clause on the dedicated machine
  (addendum "Phase 1's run of record").*

  Phase 5 runs both sides on a mainnet-scale scripted backend; served from
  memory it gates local work, and the wire gains are gated at Phase 10.
- **Regressions.** CI gates exactly only deterministic counters (filter
  passes, filters decoded, commits, fsyncs, bytes written), request counts
  by bounds, and micro ratios at a 2x regression. The 15%-warn and 25%-fail
  ratio gates, exit criteria and wall-clock runs are on a dedicated
  machine, at each phase exit and weekly, each a ledger row. *What counts
  as the dedicated machine: addendum of 2026-11-21, item 2.*

### 5. Amendments to the brief (1, 2, 3 and 5 made; 4 made for Phases 1, 5 and 10, and 6 for §8, with the harness; the rest of 4 and 6 follow)

1. **§3 W3 replaced:** "**W3 — Interoperability on the network.**
   Transactions, tickets and votes are built as dcrwallet's are, so they
   relay and mine alike. In RPC mode the wallet is the same JSON-RPC
   client to dcrd. In SPV mode it speaks protocol 11, and every dcrd and
   dcroxide node accepts and serves it correctly: nothing it sends gets it
   banned, throttled, disconnected or deprioritised, needs a message or
   version they refuse, or abuses a peer. Where it can be faster than
   dcrwallet within those bounds, it is, and each wire-visible difference
   is recorded in PARITY.md. It reveals no more about which addresses,
   transactions or blocks are the wallet's than dcrwallet does, unless the
   owner decides otherwise. To a VSP it sends the same signed requests in
   the same order; to other mixers it is indistinguishable." §1's "the same
   protocol-11 peer" becomes "served as dcrwallet's protocol-11 peer is".
2. **§4 principle 9,** "Fast by design": Decision item 1.
3. **§4 principle 6,** only under Open point 2: "except header-and-filter
   batches and the rescan marker (ADR-0003 item 9, ADR-0014)".
4. **§6:** each exit gains its criterion; Phase 13 re-measures them all.
5. **§7 Layer 6, §11 M5, Phase 10 exit:** where R1, R2, R6–R8 apply,
   compare "the same probe decisions and outcomes", not "the requests the
   backend saw" or "the non-bisecting probe sequence"; under S15, "the same
   filters" means above a typed birthday; M5 gains "N times faster".
   *(2026-11-21: from the wallet's birthday on, however set, ADR-0019; the
   Phase 10 exit and the §11 M5 row gain the clause with this pointer.)*
6. **§8** names `purse-bench` and the ledger; **§10** gains the first two
   risks below.

## Consequences

**Work.** Phase 0–1: harness, ledger, O1 if accepted. 2–3: S4, O6, O7, the
grown-file commit measurement. 5: R1–R4, R9, the scripted backend. 10:
S1–S14, R6–R8, the serving-node arms, the interop gate.

**Auditing against Go gets harder.** ADR-0008 ports Go's structure closely.
Collisions: S3 changes `sidechainMu`'s span (item 2); S5–S9, R6, R7 break
item 3's "the same set reaches the peer"; S4, R5 regroup commits (brief
principle 6); R1, R2 replace spec 09 §4.16's passes; parallel matching
risks scheduling-dependent order. Each is a `PARITY.md` row with a ledger
row, an ADR-0012 Rule 5 trace showing no RPC-visible or W2 change, and a
differential at 1 and N threads; failing the trace, Go's structure stays and
the case goes to the owner (Rule 11).

**Risks.**

- **Refused later.** dcrd added its 40 MiB limit after v2.1.7 (`589dd4c5`).
  Stay under the strictest known limit; re-run the interop gate at every pin
  move.
- **Peer capture.** S7 and S10 let a well-placed peer take most requests,
  stall sync, or serve a valid stale chain so the wallet stops at a low tip
  with stale balances; dcrwallet picks randomly per request
  (`spv/sync.go:742-773`, `spv/backend.go:50-61`). Mitigation: sync is
  declared only once the tip matches at least two peers' advertised
  heights; a random share stays; the header peer rotates on a schedule.
- **Crash during relaxed commits:** progress since the last durable commit
  is redone; the state stays consistent; external effects were durable.
- **Privacy:** R7 and R8 disclose no more than dcrwallet at its 8 peers; S9
  only in its split form.
- **Determinism:** peer choice uses a source apart from `purse-rand`'s;
  parallel results return to block order before reaching state.
- **Gaming:** loopback-only figures, warm caches, best runs; countered by
  the ledger rules, relay arms and Go in the same session.

## Open points

Most valuable first.

1. *Phase 10's N.* **Decided as recommended (project owner, 2026-11-19).** Recommended: 1.5, fixed later from the relay and
   public arms; 2 and 3–5 are upper bounds.
2. *Relaxed durability and commit boundaries* **Decided as recommended (project owner, 2026-11-19).** (ADR-0003 Open point 4;
   principle 6; R5, S3). Recommended: yes within ADR-0003 item 9 plus the
   rescan marker, external effects durable first, enabled only where a
   ledger row shows gain ([H] 79 ms per fresh 2,000-block redb commit; [S]
   0.66 s per grown bbolt commit).
3. *W3 text and principle 9.* **Decided as recommended (project owner, 2026-11-19).** Recommended: yes; the brief otherwise
   contradicts the 2026-11-19 decision.
4. *Phase 5's N.* **Decided as recommended (project owner, 2026-11-19).** Recommended: 3 (R1 halves passes, R3 1.35–4.5x, R2 most
   decoding), with amendment 5, without which R1 and R2 fail Layer 6.
5. *S15* **Decided as recommended (project owner, 2026-11-19).** tells peers the wallet has no interest before a date.
   Recommended: yes, fetching on demand for rescans below it.
   *2026-11-21 (project owner): "we will drop filters below the wallet's
   birthday", which widens S15 from a typed birthday to the wallet's
   birthday however set, and from what the sync fetches to what the store
   keeps; ADR-0019, whose open questions FB1 and FB2 ask whether the
   fetching and the on-demand rule follow it.* *2026-11-22 (project
   owner): both follow it, FB1 and FB2 decided as recommended. The
   initial sync, in SPV and RPC mode, requests no filter below the
   wallet's birthday however set, a birthday pending above the tip
   included; rescans and discoveries from below the birthday in SPV mode
   fetch the filters of that range from peers, check them against the
   stored headers where DCP0005's commitment applies, use them and keep
   none. The cost of such a rescan with dcrwallet's defaults, about a
   million filters fetched on mainnet each time, was not measured.
   ADR-0019, addendum of 2026-11-22; not implemented.*
6. *S4 against ADR-0003 item 2* **Decided as recommended (project owner, 2026-11-19).** (Phase 2 keys by hash). Recommended:
   decide in Phase 2 from a grown-file measurement of both, detached rows
   by hash (ADR-0003 Open point 5). *Measured (ADR-0003, addendum of
   2026-11-19): by hash, initial sync writes 3.4 times the bytes and takes
   2.8 times as long with durable commits; height keys are recommended,
   and the schema change waits on the owner.* *Decided (project owner,
   2026-11-19): height keys, changed in schema version 1 before Phase 3
   (ADR-0003, addendum "Decisions of the project owner").* *Done
   (2026-11-20): built in schema version 1 (ADR-0003, addendum "Chain
   tables keyed by height, as built").*
7. *ADR-0008 addendum* **Decided as recommended (project owner, 2026-11-19).** Recorded as ADR-0008's
   "Addendum (ADR-0014)". (items 2 and 3, Accepted). Recommended: permit S3's
   lock span and S5–S9, R6, R7's request sets, each with its PARITY row.
8. *O1 in dcroxide:* **Decided as recommended (project owner, 2026-11-19).** reverses dcroxide-dcrec's documented choice
   (`Cargo.toml:19-27`), adding `precomputed-tables` plus `std` (pulls
   `getrandom`) or `critical-section`. Recommended: propose under
   ADR-0009, variant per dcroxide's dependency review. *Done
   (2026-11-20): dcroxide `5ace7a2` adds the default feature
   `precomputed-tables` with `primeorder/std`, taking neither `k256/std`
   (it would enable `getrandom`) nor `critical-section`, and the pin moved
   to `2074168`; Phase 1's secret paths measured 0.84 to 0.86 times Go's
   time (addendum of 2026-11-20).*
9. *S9 unsplit:* **Decided as recommended (project owner, 2026-11-19).** 16 parties see first transaction announcements
   (`spv/backend.go:469-474`, `spv/sync.go:716-728`) and part of the match
   set, for no established gain. Recommended: no.
10. *R10* **Decided as recommended (project owner, 2026-11-19).** shows matched blocks to another peer for a tail gain on ~54
    blocks. Recommended: no.
11. *O5* **Decided as recommended (project owner, 2026-11-19).** weakens the seed's sealing to save ~0.7 s per open. Recommended:
    no.
12. *Numbers:* **Decided as recommended (project owner, 2026-11-19).** K=40, ≤6 per peer; hedging at p95, ≤5% extra; 1.15×; CI 2x,
    dedicated 15/25%. Recommended: start there, fix from measurements.

**Not established:** (13) latency and per-peer queueing as requests in
flight rise: arms at K = 20, 40, 80 with a bandwidth cap; (14) TCP slow
start's share of header latency: pinned against rotating peers; (15) the
tail after S6; (16) clean wall times: [S] on an idle machine, no VPN;
(17) the kernel half of headers-stage CPU: kernel profiling; (18) dcroxide
drops under pipelining: the interop gate; (19) redb commit cost in a grown
file by hash and by height: Phase 2, with ADR-0003 Open point 9's table
merging; (20) Go's anonymous-memory baseline: `RssAnon` from [S]'s samples;
(21) real restore pass counts F: Layer 6's corpus; (22) ADR-0003's query
results, which Phase 2–3's N needs. *(22) measured 2026-11-20 on the
shared desktop while other work ran on it (ADR-0003, addendum
"`purse-store` against dcrwallet on the same workload"); N set from those
figures on 2026-11-21 (addendum of that date, item 1); the run of record
on the dedicated machine (item 2 of that addendum) has not been made.*
*It is to be made after the next transaction-store slice, and not before
(project owner, 2026-11-24; addendum of that date).* *Made 2026-11-26
(addendum "The store's run of record"): (22) is measured on the
dedicated machine for the store with the measurement's own composition
of the transaction store's rows; the measurement through the
transaction store's own functions (ADR-0003 Open point M3) is not.*

## Addendum, 2026-11-19 — The Phase 1 speed row and the first ledger entry

The ledger's first entry (`docs/bench-ledger.md`, 2026-11-19) measured
Phase 1's row of the exit table under item 4 and some workloads outside
it, and the Phase 2 measurements answered Open point 6 (ADR-0003). On
2026-11-19 the project owner decided, each as recommended:

1. **Phase 1's exit.** Its parity clauses are closed (brief §6 Phase 1;
   `docs/PHASE1-EXIT.md`). Its speed clause stays open until the dcroxide
   pin includes dcroxide `5ace7a2`, which builds k256 with its
   precomputed tables (O1; Open point 8), and the row is then measured
   again. The secret paths measured 1.57 to 1.59 times Go's time against
   at most 1.15, about 90% of a hardened derivation being k256's
   multiplication of the generator without those tables. *Measured
   again on 2026-11-20 (addendum of that date): the secret paths meet the
   row at 0.84 to 0.86; the clause stays open on base58 decoding (item 2).*
   *Met later on 2026-11-21 on the dedicated machine: base58check decode
   0.78 and every other clause met, so the speed clause is met (addendum
   "Phase 1's run of record").*
2. **base58 decoding** measured at parity with Go (0.98 to 1.02 over ten
   sessions on a shared desktop). It is settled by a measurement on a
   dedicated machine first; only if that misses does it call for a
   dcroxide change (about half of Rust's time is dcroxide-crypto's
   portable BLAKE-256). *2026-11-20: 1.01 over three more sessions
   (0.99 to 1.02 by session); still to be settled on a dedicated machine
   (addendum of 2026-11-20).* *2026-11-21: what counts as the dedicated
   machine is decided, and the run waits for the owner's word on when and
   where (addendum of 2026-11-21, item 2).* *Settled later on 2026-11-21:
   0.78 on the dedicated machine, cycling through 16,384 addresses; met,
   so no dcroxide change is called for (addendum "Phase 1's run of
   record").*
3. **secretbox at about 3 times Go's time** (2.9 to 4.0; the `poly1305`
   crate's AVX2 backend calls its intrinsics out of line) is accepted for
   now. It was recommended that the RustCrypto project be told later;
   telling an upstream project is covered by the owner's exception of the
   same day on reporting upstream (ADR-0017, "Decision by the project
   owner"), so whether and how it is reported is the owner's, and this
   project files nothing.
4. **`Amount`'s string form at 2 times Go's time** is accepted for now; the
   remaining cost is dcroxide-dcrjson's `format_float_f`.
5. **The `bench-gate` CI job** (micro ratios at a 2x regression, item 4)
   is not a required check at first.
6. **`sntrup4591761`'s timing comparison against Go** gets no CI gate for
   now (deferred); it stays an ignored test, run by hand.
7. **Argon2id with several lanes** (2.37 times Go's time with 4 lanes):
   the `argon2` crate's `parallel` feature is to be turned on in
   `purse-snacl`. That is a separate change, not made with this addendum.

dudect-style checks and a micro benchmark for dcroxide's secp256k1 are a
follow-up in dcroxide (ADR-0006, addendum of 2026-11-19).

## Addendum, 2026-11-20 — Phase 1's speed row at dcroxide `2074168`

The dcroxide pin moved from `f4b3d2e` to `2074168` on 2026-11-20
(`docs/SKEW.md`, 2026-11-20 section; ADR-0009, addendum of that date).
The range includes dcroxide `5ace7a2`, which multiplies a secret scalar by
the generator through k256's basepoint table (O1, Open point 8), so
Phase 1's row was measured again, as item 1 of the addendum of 2026-11-19
requires: the bench ledger's entry "The secret paths at dcroxide
`2074168`" (2026-11-20), three sessions of the first entry's command on
the same shared desktop, 15 processes per side, measured processes pinned
to four cores, one-minute load average between 2.8 and 22.9.

| clause | criterion | first entry (2026-11-19) | 2026-11-20 | verdict |
|---|---|---|---|---|
| master key | ≤ 1.15 | 1.59 | 0.86 | met |
| hardened child | ≤ 1.15 | 1.58 | 0.84 | met |
| private child | ≤ 1.15 | 1.57 | 0.85 | met |
| public derivation | ≤ 1.00 | 0.55 | 0.54 | met |
| address | ≤ 1.00 | 0.56 | 0.56 | met |
| script | ≤ 1.00 | 0.55 | 0.55 | met |
| HMAC-SHA512 | ≤ 1.00 | 0.86 | 0.83 | met |
| hash160 | ≤ 1.00 | 0.84 | 0.81 | met |
| base58check encode | ≤ 1.00 | 0.95 | 0.94 | met |
| base58check decode | ≤ 1.00 | 0.98 (1.00 over ten sessions) | 1.01 (1.02, 1.01, 0.99 by session) | undecided, at parity |
| 20,000 addresses | ≥ 1.5x faster | 1.80x | 1.80x | met |

Figures are Rust's time over Go's (below 1, Rust is faster), the pooled
medians of the entry. **The secret paths now meet the row**, as this
record's measurement of O1 predicted. **The speed clause is not closed:**
base58check decode, the one workload short of it, measured 1.01 against
at most 1.00, missed in two of the three sessions by the driver's verdict;
no base58 or BLAKE-256 code changed on either side since the first entry.
Under item 2 of the addendum of 2026-11-19 it is settled by a measurement
on a dedicated machine first, which is also to confirm the rest of the row
(item 4, "Exit criteria"); no such machine has measured it yet. *What
counts as the dedicated machine was decided on 2026-11-21 (addendum of
that date, item 2); the run has not been made.* *Made later on
2026-11-21: base58check decode 0.78 and every other clause of the row
met, so the speed clause is met (addendum "Phase 1's run of record").*

The CI gate's baselines for the three secret paths
(`crates/purse-bench/gate-baseline.tsv`) are lowered to the entry's
ratios (0.861, 0.837 and 0.853), so the gate now fails above about 1.7
times Go's time on them instead of about 3.2; one local run of the gate's
command passed with them. Not run on GitHub's runners.

This addendum records a measurement; it decides nothing.

## Addendum, 2026-11-21 — Decisions of the project owner: N for Phases 2 and 3, and the dedicated machine

On 2026-11-21 the project owner accepted ADR-0003's Open points M1 to M3
as recommended (ADR-0003, addendum of that date) and decided what counts
as the dedicated machine item 4 names. The text above stays as written,
apart from the dated pointers added to it.

1. **N for the row of Phases 2 and 3** (item 4, "Exit criteria"; ADR-0003
   Open point M1). The row's "≥N times faster on balance, range queries
   (N from ADR-0003)" reads N = 1,000 for a balance query and N = 1.5 for
   a range query. "No slower than Go on ADR-0003's workload" is
   unchanged. The figures recorded so far (ADR-0003, addendum
   "`purse-store` against dcrwallet on the same workload": 36,800x and
   1.86x at their lowest) are from the shared desktop while other work
   ran on it (lightly loaded) and are not the run of record. That run is the one of item 2, before Phase 3's
   exit (ADR-0003 Open point M2), together with the Phase 1 run still
   owed; the store is measured again with Phase 3's transaction store as
   part of Phase 3's exit (Open point M3). *Later on 2026-11-21 the
   Phase 1 run was made (addendum "Phase 1's run of record"); the store's
   is not made and stays owed.* *2026-11-24: the store's is to be made
   after the next transaction-store slice, and not before (addendum of
   that date).* *2026-11-26: the store's was made after that slice
   (addendum "The store's run of record"); the measurement with Phase
   3's transaction store (M3) is not done and stays owed.*
2. **What counts as the dedicated machine.** A run on the shared desktop
   on which the measurements so far were taken (the bench ledger's
   machine d1) counts as the dedicated-machine run when nothing else is
   running on it; the run may instead be made on another machine. No timed run of record is started until the owner
   says when and where. None has been made, so:
   - Phase 1's speed clause stays open on base58check decode
     (`docs/PHASE1-EXIT.md`, "Speed"; item 2 of the addendum of
     2026-11-19). Phase 1 is not closed, and no timing figure is recorded
     by this addendum. *Later the same day the owner said when and where,
     and the run was made: the addendum below.*
   - The row of Phases 2 and 3 is not yet measured on such a run.
     *On 2026-11-24 the owner said when: after the next transaction-store
     slice (addendum of that date).* *Measured on such a run on
     2026-11-26: the addendum "The store's run of record".*

This addendum records decisions; it measures nothing.

## Addendum, 2026-11-21 — Phase 1's run of record

On 2026-11-21 the project owner said the shared desktop was quiet and
told the run of record to be made on it. The run was made that day under
item 2 of the addendum above: on the bench ledger's machine d1 with no
other job running on it (the entry lists the processes a `top` snapshot
showed), the dedicated-machine run for Phase 1's exit criteria that
item 4 ("Exit criteria"; "Regressions" puts exit criteria on a dedicated
machine at each phase exit) and item 2 of the addendum of 2026-11-19
call for. It is the ledger's entry "Phase 1's row on the
dedicated machine at dcroxide `48f5b5e`": dcr-purse `1f7316e`, three
sessions of the ledger's command back to back, 08:40:11 to 08:54:01 UTC,
15 processes per side alternating, measured processes pinned to four
cores, one-minute load average 0.63 at the start and 0.76 to 1.14
during the sessions (the measured processes themselves). The base58 and
extended-key workloads cycle through 16,384 different inputs on both
sides, so the decode clause is measured on varied input for the first
time with `purse-bench`.

| clause | criterion | first entry (2026-11-19) | 2026-11-20 | 2026-11-21, dedicated machine | verdict |
|---|---|---|---|---|---|
| master key | ≤ 1.15 | 1.59 | 0.86 | 0.86 | met |
| hardened child | ≤ 1.15 | 1.58 | 0.84 | 0.85 | met |
| private child | ≤ 1.15 | 1.57 | 0.85 | 0.85 | met |
| public derivation | ≤ 1.00 | 0.55 | 0.54 | 0.55 | met |
| address | ≤ 1.00 | 0.56 | 0.56 | 0.55 | met |
| script | ≤ 1.00 | 0.55 | 0.55 | 0.56 | met |
| HMAC-SHA512 | ≤ 1.00 | 0.86 | 0.83 | 0.87 | met |
| hash160 | ≤ 1.00 | 0.84 | 0.81 | 0.85 | met |
| base58check encode | ≤ 1.00 | 0.95 (256 hashes, last byte varied) | 0.94 (the same) | 0.45 (16,384 inputs) | met |
| base58check decode | ≤ 1.00 | 0.98 (one address) | 1.01 (one address) | 0.78 (16,384 addresses) | met |
| 20,000 addresses | ≥ 1.5x faster | 1.80x | 1.80x | 1.80x | met |

Figures are Rust's time over Go's (below 1, Rust is faster), the pooled
medians of each entry; on 2026-11-21 every session's own ratio gives the
same verdict, base58check decode being 0.78 in each. **Phase 1's speed
row is met on every clause on the dedicated machine.** The 2026-11-19
and 2026-11-20 base58check figures are for workloads that repeated one
input and were measured before the dcroxide pin reached `48f5b5e`'s limb
conversion, so they are not comparable with the last column; the old
decode workload, kept as `crypto.base58check_decode_one`, measured 0.78
in this run. Outside the criteria, secretbox (2.86 to 3.99 times Go's
time), `Amount`'s string form (2.26) and Argon2id with 4 lanes (1.14)
remain slower than Go, as items 3, 4 and 7 of the addendum of 2026-11-19
record. The closing of Phase 1 on these figures is recorded in
`docs/PHASE1-EXIT.md` ("Speed").

The CI gate's baselines for the four cycling workloads
(`crates/purse-bench/gate-baseline.tsv`) are the entry's pooled ratios:
base58check encode 0.454, decode 0.780, `hd.xpub_string` 0.955 and
`hd.parse_xpub` 0.576. The file's rules (lower a value on a lasting gain)
do not call for re-deriving the other fourteen from this run, and they
are unchanged.

**Not applied: the 15%-warn and 25%-fail ratio gates.** Item 4
("Regressions") puts them on the dedicated machine at each phase exit
too. None was applied to this run: no earlier dedicated-machine run
exists to compare it with, and this run is the first such reference.
Set informally against the two earlier entries, both from the same
desktop while other work ran on it, the largest rise is
`hd.parse_xpub`'s, a workload that now cycles through 16,384 keys
instead of repeating one, by about 15%: 0.576 against the first entry's
0.501 (15.0%, between 14.9% and 15.1% within the rounding of 0.501, so
whether it passes 15% cannot be settled) and 14.1% to 14.6% against the
entry of 2026-11-20 (whose medians put its ratio between 0.503 and
0.505); the two-decimal ratios, 0.58 against 0.50, overstate it. Of the
others the largest rise is `dcrutil.amount_string`, whose code is
unchanged, 2.260 against 1.997 in the first entry (13.2%) and 2.16 on
2026-11-20. None rose by 25% or more.

**Not made: the dedicated-machine run for the row of Phases 2 and 3**
(the store against dcrwallet's udb; ADR-0003 Open point M2, item 1 of the
addendum above). It stays owed, before Phase 3's exit. *2026-11-24: to
be made after the next transaction-store slice, and not before (the
addendum below).* *2026-11-26: made (the addendum "The store's run of
record").* Not measured by
this run: other machines and processors, other architectures and
operating systems, more than 4 threads, and GitHub's runners, on which
the CI gate runs.

This addendum records a measurement; it decides nothing.

## Addendum, 2026-11-24 — Decision of the project owner: when the store's run of record is made

On 2026-11-24 the project owner decided when the dedicated-machine run
for the row of Phases 2 and 3 (the store against dcrwallet's udb;
ADR-0003 Open point M2; item 1 of the addendum of 2026-11-21, "Decisions
of the project owner") is made: after the next transaction-store slice
of `purse-udb`, which ports input selection, so that input selection is
measured too, and not before (crate `txstore`, "The next slice"). The
text above stays as written, apart from the dated pointers added to it.

- The run is authorised for that point and has not been made; no timing
  figure is recorded by this addendum, and the row of Phases 2 and 3 is
  still not measured on a run of record. *2026-11-26: made at that
  point (the addendum "The store's run of record").*
- The decision names when. It does not name the machine; what counts as
  the dedicated machine stays as item 2 of the addendum of 2026-11-21
  defines it.
- The rest is unchanged: the run comes before Phase 3's exit, and the
  store is measured again with Phase 3's transaction store as part of
  that exit (ADR-0003 Open point M3).

**Places amended with this addendum:** the Status line, the note under
item 4 ("Exit criteria", the row of Phases 2 and 3), the note under item
22, items 1 and 2 of the addendum of 2026-11-21 "Decisions of the project
owner" and the closing paragraph of the addendum "Phase 1's run of
record", by dated pointers; ADR-0003 (Status line, Open point M2 and
items 2 and 4 of its addendum of 2026-11-21); `docs/PHASE2-EXIT.md`;
`docs/PHASE0-EXIT.md` (the D2 row of its decision table, point 4 of
"Verdict" and the list of what the close defers); the project brief
(Phase 2's Status and §9 D2); the ADR index; `purse-udb`'s crate
`txstore`, "The next slice".

This addendum records a decision; it measures nothing.

*Note, 2026-11-24 (the selection slice):* that slice is the one this
decision names. It ports input selection, `RandomUTXO` and `Spender`, and
`crates/purse-udb/tests/measure.rs` now times `MakeInputSource` with
`SelectInputs` (every output of one account, one coin from two accounts,
50 coins at depth 0) and 1,000 `RandomUTXO` calls on both sides, the
transaction store's own code on the Rust side, its answers checked
against Go's in every round (the selection of everything as a set; the
others, whose random choices follow each side's operating-system-seeded
stream, for covering their target). The run of record is not made; no
timing of these figures is recorded. *2026-11-26: made, with these
figures' first timings: the addendum below.*

## Addendum, 2026-11-26 — The store's run of record

On 2026-11-26, on the project owner's word that the machine was quiet,
the run of record for the row of Phases 2 and 3 (the store against
dcrwallet's udb; ADR-0003 Open point M2; item 1 of the addendum of
2026-11-21, "Decisions of the project owner") was made at the point the
addendum of 2026-11-24 names: after the transaction-store slice that
ported input selection (`4c8a134`), measured at dcr-purse `d61c5d4`, a
clean checkout. It was made under item 2 of the addendum of 2026-11-21:
on the bench ledger's machine d1 with no other job running on it (the
entry lists what a `top` snapshot before and after the sessions showed),
the dedicated-machine run that item 4 ("Exit criteria"; "Regressions"
puts exit criteria on a dedicated machine) calls for. It is the ledger's
entry "2026-11-26 — `purse-store` against dcrwallet's udb on the
dedicated machine, with input selection": three sessions of the command
of the entry of 2026-11-20 back to back, 05:04:43 to 05:12:35 UTC, the
whole test pinned to four cores, 9 round medians per side for each
figure (3 for the history sync), one-minute load average 0.51 at the
start and 1.03 to 1.14 at the sessions' ends (the measured processes
themselves). Its workload, data checks and method are those of 2026-11-20
(ADR-0003, addendum "`purse-store` against dcrwallet on the same
workload"), with five figures added: four input selections
(`MakeInputSource` with `SelectInputs`) and 1,000 `RandomUTXO` calls.

| clause | criterion | 2026-11-20, shared desktop | 2026-11-26, dedicated machine | verdict |
|---|---|---|---|---|
| balance query | ≥ 1,000x faster | 36,800x at the lowest (all accounts); five figures 36,800x to 102,500x | 33,966x at the lowest (all accounts); five figures 33,966x to 94,263x | met |
| range query | ≥ 1.5x faster | 1.86x at the lowest (1,000 blocks mid-chain, forward); four figures 1.86x to 2.05x | 1.82x at the lowest (the last 1,000 blocks, backward); four figures 1.82x to 2.04x | met |
| no slower than Go on ADR-0003's workload | Rust/Go ≤ 1.00 on every figure | worst 0.627 (tip-block commit), 16 figures | worst 0.617 (tip-block commit), 21 figures | met |

The first two rows give dcrwallet's time over purse-store's, the third
Rust's time over Go's, from the pooled medians of each entry. Every
session's own figures give the same verdicts: the lowest balance figure
in any session 33,574x, the lowest range figure 1.80x (1,000 blocks
mid-chain, forward, the first session) and the worst ratio 0.625 (the
tip-block commit, the third). On no figure do the two sides' ranges
overlap. **The row of Phases 2 and 3 is met on every clause on the
dedicated machine, for what this run measures.**

**What this run measures, and what it does not.** As the header of
`crates/purse-udb/tests/measure.rs` states, the transaction-store rows
are still written by the measurement's own composition, and the balance
and range queries read them through it, so those figures, the lookups
and the commit and history-sync figures measure the schema and the
store, plus the address manager as built. Only input selection and
`RandomUTXO` run `purse-udb`'s transaction-store code. The run therefore
settles M2 and
gives input selection's first figures: 2.53 to 12.57 times faster than
Go over the four selections, and `RandomUTXO` 1.84 times. It does not
measure the transaction store's own write path or its balance and range
queries, nor stake, coinbase, unmined, stake-invalidation and
reorganisation operations: that is M3 (ADR-0003 Open point M3, the same
test calling `purse-udb`'s transaction-store functions with those
operations added, as part of Phase 3's exit), which is not done and
stays owed. Both sides seed selection's random stream from the operating
system, so only the selection of every output is compared, as a set;
the other three selections are checked for covering their target and
`RandomUTXO` by how many of its calls found an output, a count the two
sides must agree on (the `random_utxo/minconf=1` check in `measure.rs`
and in storebench's `hot.go`), and those four figures time the same
operation over the same rows, not the same choices.

**Against the shared desktop's figures of 2026-11-20.** The lowest
balance figure was 36,800x there and is 33,966x here; the lowest range
figure 1.86x there and 1.82x here (the last 1,000 blocks backward, 1.87x
there); the worst ratio 0.627 there and 0.617 here, both the tip-block
commit. Rust's balances of account 0 measured 1.45 and 1.57 µs at the
median against 930 ns and 1.01 µs there, their round medians lying near
either value (the ledger entry gives them); the other balance ratios are
8% to 9% above those of 2026-11-20, and of the eleven figures that are
not balances, five ratios are 0.4% to 2.4% above and six 1.6% to 6.6%
below. Between the two runs the machine went from lightly loaded to
quiet, and dcr-purse from `a5cdb88` with uncommitted changes to
`d61c5d4` and dcroxide from `2074168` to `1587ca1`; what accounts for
the differences was not examined.

**Not applied: the 15%-warn and 25%-fail ratio gates** (item 4,
"Regressions"): this is the store's first dedicated-machine run, so no
earlier one exists to compare it with. Not measured by this run: M3, as
above; cold reads; the spike's 1m size; store open, backup and crash
recovery; memory; bytes written on the Rust side; more than 4 threads,
other machines and processors, other architectures, operating systems
and file systems.

**Places amended with this addendum:** the Status line, the note under
item 4 ("Exit criteria", the row of Phases 2 and 3), the note under item
22, items 1 and 2 of the addendum of 2026-11-21 "Decisions of the
project owner", the closing paragraph of the addendum "Phase 1's run of
record", and the first item and the closing note of the addendum of
2026-11-24, by dated pointers; ADR-0003 (Status line, "What this does
not show" and Open points M2 and M3 of its addendum of 2026-11-20, and
items 2 and 4 of its addendum of 2026-11-21); `docs/bench-ledger.md`
(the entry of 2026-11-26, and a dated note under the entry of
2026-11-20); `docs/PHASE2-EXIT.md`; `docs/PHASE0-EXIT.md` (the D2 row of
its decision table, point 4 of "Verdict" and the list of what the close
defers); the project brief (Phase 2's Status and §9 D2); the ADR index;
the README (`purse-store`'s row); `purse-udb`'s crate `txstore`, the
paragraph on the run of record.

This addendum records a measurement; it decides nothing.
