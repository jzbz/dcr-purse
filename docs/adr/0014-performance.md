# ADR-0014 — Performance as a goal

- **Status:** The goal is **Accepted** (project owner, 2026-11-19): "yes
  faster spv sync is an important goal. speed everywhere actually". Its
  precedence over being indistinguishable from dcrwallet on the network is
  **Accepted** (project owner, 2026-11-19): "let's reverse that, faster is
  more important". Open points 1 to 12 were all decided as recommended
  (project owner, 2026-11-19); each is marked. Every number is a starting
  value, fixed from measurements; items 13 to 22 are not yet established.
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
| S15 | No filters below a typed birthday (`spv/sync.go:1736-1748`) | E: most of 453 MB for a recent date | SHAPE | DO (Open point 5) |
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
| O1 | Secret generator multiplication via k256 tables (dcroxide-dcrec `secp256k1.rs:375`, `ecdsa.rs:538`, `schnorr.rs:223`) | M: 36.8 → 19.2 µs; E: signing 1.02–1.11x Go's time | LOCAL | per Open point 8 |
| O3 | Public derivation: cached parent, batch inversion | E: ~15% | LOCAL | when profiled |
| O4 | scrypt in parallel with store open and seeding | M: 0.74 of 2.7 s | LOCAL | DO |
| O5 | Cheaper scrypt for new wallets | — | — | REJECT (Open point 11) |
| O6 | Maintained balances; `AccountBalances` not quadratic; unused `NewBalances` dropped (specs 06 Q14, 08 Q13) | not measured | LOCAL | DO in ADR-0003 |
| O7 | Range queries, `findEligibleOutputs` by index | ADR-0003 | LOCAL | DO; order kept |
| O8 | Notifications outside write transactions | — | LOCAL | brief D11's ADR |

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

  Phase 5 runs both sides on a mainnet-scale scripted backend; served from
  memory it gates local work, and the wire gains are gated at Phase 10.
- **Regressions.** CI gates exactly only deterministic counters (filter
  passes, filters decoded, commits, fsyncs, bytes written), request counts
  by bounds, and micro ratios at a 2x regression. The 15%-warn and 25%-fail
  ratio gates, exit criteria and wall-clock runs are on a dedicated
  machine, at each phase exit and weekly, each a ledger row.

### 5. Amendments to the brief (1, 2, 3 and 5 made, and 4 for Phases 5 and 10; the rest of 4 and 6 follow with the harness)

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
6. *S4 against ADR-0003 item 2* **Decided as recommended (project owner, 2026-11-19).** (Phase 2 keys by hash). Recommended:
   decide in Phase 2 from a grown-file measurement of both, detached rows
   by hash (ADR-0003 Open point 5).
7. *ADR-0008 addendum* **Decided as recommended (project owner, 2026-11-19).** Recorded as ADR-0008's
   "Addendum (ADR-0014)". (items 2 and 3, Accepted). Recommended: permit S3's
   lock span and S5–S9, R6, R7's request sets, each with its PARITY row.
8. *O1 in dcroxide:* **Decided as recommended (project owner, 2026-11-19).** reverses dcroxide-dcrec's documented choice
   (`Cargo.toml:19-27`), adding `precomputed-tables` plus `std` (pulls
   `getrandom`) or `critical-section`. Recommended: propose under
   ADR-0009, variant per dcroxide's dependency review.
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
results, which Phase 2–3's N needs.
