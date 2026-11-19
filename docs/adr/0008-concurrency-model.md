# ADR-0008 — Concurrency model

- **Status:** Accepted (decision D6; project owner, 2026-11-19): OS threads
  everywhere, with an async runtime allowed only inside the gRPC listener,
  and only if the transport spike (ADR-0004) calls for it. Channels and
  multi-way `select` come from `crossbeam-channel`, not an in-tree
  implementation. Open points 2 to 4 were decided as proposed. The
  repository facts under Context are not outcomes of it.
- **Date:** 2026-11-18

## Context

**The question.** What replaces goroutines, channels, `select` and
`context.Context`, and whether an async runtime may run anywhere in the
process. The brief recommends OS threads and a close port of the Go structure,
so that lock and delivery order can be audited against the specs, revisiting
only if D5 forces a runtime in (brief §9, D6).

**By when.** Phase 0 exits with D1–D10 decided (brief §6). The cancellation
pattern is needed first: Phase 3 fixes `purse-udb`'s API (Go's `View` and
`Update` take a context, `wallet/walletdb/interface.go:203`, `:230`) and Phase
4 defines `NetworkBackend`, whose every method takes one
(`wallet/network.go:35-56`).

**dcroxide.** OS threads over `std::sync::mpsc`, no `async fn`, because
threads mirror dcrd thread for goroutine and keep the parity audit tractable
(ADR-0005 `:53-60`, `:109-139`). Release aborts on panic since Rust mutexes
poison (`:84-92`); a refused thread must not abort the process
(`:70-75`; `crates/dcroxide-node/src/runtime.rs:41-61`). Its code injects
wall time, monotonic time and randomness into sans-I/O state
(`crates/dcroxide-peer/src/peer.rs:25-50`), and tears a connection down with a
cancel flag that reads check in 1 s slices plus a socket shutdown, which alone
does not wake a blocked `recv` under Winsock
(`crates/dcroxide-node/src/transport.rs:19-28`, `:69-106`, `:181-184`). It has
no context type (`crates/dcroxide-rpc/src/worksem.rs:1-40`).

**The load in the Go.** At the pin, outside tests and generated code: 77 `go`
statements, 31 errgroup `Go` calls, 661 context parameters. Long-lived: about
8 SPV syncer tasks, the wsrpc reader, writer and watcher, the mixing epoch
ticker and `NumCPU` workers, the ticket buyer, a serve loop per listener
(14 §6.1, 15 §7.1, 12 §7.3, 11 §7.2, 16 §7.1). Per connection: six per SPV
peer, one able to outlive it (14 §6.1); three per websocket session (16 §7.1);
one per gRPC call, streams lasting as long as the client (19 §7.1, 20 §7).
Per item: Decision item 3. Timed waits: a 30 s stall timer on 13 request
paths (`p2p/peering.go:52`), pong 15 s, dial 10 s (14 §5); wsrpc ping 60 s
(15 §6); mixing stage 30 s, epoch 10 min (12 §6). Spec 16 derives that the
10 s HTTP read deadline cancels a POST handler's context; this is UNVERIFIED
(16 §7.3, O1) until a golden capture of a handler running past 10 s.

**What a port must keep observable.**

- *Lock spans across I/O and transactions*: `lockedOutpointMu` across a view
  calling `LoadTxFilter` (06 §7.1); `addressBuffersMu` across `LoadTxFilter`
  under `gapPolicyIgnore` (10 §9 item 40; 13 §7.2); `sidechainMu` across
  transactions and RPC round trips (14 §6.2, 15 §7.2); `remotesMu` across
  30 s sends (14 §6.2); network calls inside write transactions (09 §6.2).
- *What commits together* (09 §6.2; brief §4 principle 6).
- *Notification order*: per client, production order, unbuffered sends under
  the server mutex, some inside a write transaction (08 §7.1); sync callbacks
  block the syncer (`chain/sync.go:86-88`) and gRPC sends inside them
  back-pressure it (20 §7).
- *The context each call runs under*, including deliberate `Background` roots
  (12 §7.3, 15 §7.4, 11 §7.2, 19 §7.1), and Go's cancellation texts (16 §7.3,
  19 §7.5).
- *Timing peers see*: a cancelled `CFilterV2` stays registered until its stall
  timer fires (`p2p/peering.go:1728`, `:1734-1739`); mixing deadlines (W3).
- *Deadlock-freedom resting on the engine*: `NotificationServer.mu` (08
  §7.1) and `stakeSettingsLock` (11 §7.1) are each taken in both orders
  against a transaction, safe only because readers never wait for the writer.

**Facts.** Existing crates are synchronous and inject randomness
(`crates/purse-seed/src/walletseed.rs:76`). Release aborts on panic
(`Cargo.toml:32-40`); `deny.toml:23` refuses duplicates; `unsafe_code =
"forbid"` binds workspace crates only (`Cargo.toml:43-44`). redb 5 (ADR-0003)
has one blocking writer and readers that coexist with it (redb 5.0.0
`src/db.rs:612-613`, `:1890-1892`); its transactions are `Send + Sync`
(`tests/integration_tests.rs:4960-4980`); pages freed by a write are not
reused while a live reader needs them (`CHANGELOG.md:310-311`). A Go panic
ends the process except in an HTTP handler (Go 1.27.4
`src/net/http/server.go:1960`); dcrwallet's gRPC interceptors do not recover
(`rpcserver.go:328-331`, `:406`, `:432`). Go's `select` draws from the
runtime's generator (`src/runtime/select.go:191`), which the oracle's det
build leaves unseeded (23 §5.6.2); scheduling-dependent orders are compared as
partial orders (23 §5.6.5). `spikes/grpc-intree` and `spikes/grpc-tonic` are
in progress; neither has recorded results.

## Options considered

- **A.** OS threads, channels and condition variables throughout.
- **B.** tokio throughout.
- **C.** Threads, tokio confined to the gRPC transport (ADR-0004's tonic
  variant).
- **D.** Threads, a runtime confined to the gRPC server, dcrd websocket client
  and mixing client behind channel seams (spec 25 §7.3, §13.1 D-1).
- **E.** No runtime crate: sans-I/O state machines driven by one in-tree event
  loop over a readiness crate; threads for blocking work.

| | A | B | C | D | E |
|---|---|---|---|---|---|
| Audit against Go | goroutine → thread, same lock spans | callers of every wait turn `async`; spans across network calls inside redb transactions need async mutexes | A; the transport is tonic's | A in the core; mixing stages and websocket client in futures | furthest from Go's shape |
| Cancellation | context type (below) | a drop cancels at any `.await`, which Go never does | two models, one seam | two models, three seams | explicit |
| Transactions | calling thread | a worker held across `.await`, or a `spawn_blocking` hop | threads | threads | off the loop |
| Threads | about 100 (item 3) | few | A + workers | A + workers | fewest |
| Determinism | interleavings vary; fake clock never self-advances | current-thread runtime replays interleavings and self-advances, not across blocking tasks (tokio 1.54.1 `src/time/clock.rs:102-133`) | as A | as A | one loop replays exactly |
| Dependencies | a channel crate | tokio's graph; duplicates unverified | tonic 0.14.9, 27 dependencies | C + websocket stack | a readiness crate; a loop to own |

B and E buy reproducible interleavings, the strongest case against A; A
answers with partial-order comparison and timing logic factored into state
machines (below). D doubles the seams and moves mixing timing, audited
against Go's `select` code (W3), into futures.

## Decision

Option A:

1. **No runtime, with one permitted seam.** No dcr-purse crate depends on an
   async runtime or contains an `async fn`; concurrency uses `std::thread`,
   `std::sync` and `crossbeam-channel`. The one exception the project owner
   allows (2026-11-19): a runtime confined to the gRPC listener (option C at
   that seam only), and only if ADR-0004's transport spike calls for it.
   Everything behind the listener, the handlers included, stays on threads
   and the wallet's blocking API.
2. **Goroutine for thread where it outlives its call**, with Go's lifetime and
   exit conditions. Locks keep Go's scope, order and span; transactions keep
   Go's boundaries; notifications are generated at Go's points.
3. **Per-item goroutines** map as below. A mapping whose outcomes Go could
   also produce (an order its scheduler permits, the same requests and
   deadlines) is documented at the site; one that changes a Go outcome, and
   every refusal path, is a `PARITY.md` divergence.

| Go (spec) | Port | OS refuses a thread |
|---|---|---|
| 1 per received P2P message (14 §4.4) | runs on the reader in arrival order (14 §6.5 permits any); waited hand-offs (syncer fan-in, priority queue) become unbounded FIFO pushes; a full capacity-1 reply slot drops the message (only Q-09; outcome unchanged) | — |
| Syncer: 1 per getdata, ≤ 3 per inv, 1 per announcement (14 §6.1) | per peer, one worker for invs and announcements, one for getdata | disconnect the peer |
| Mempool timer per tx (14 §4.25) | deadline queue on `handleMempool`'s thread | — |
| 1 per queued dcrd notification, run serially (15 §5.10.4; QK-0412) | unbounded FIFO, one notifier thread | — |
| Waited I/O fan-outs: cfilters, `Blocks`, `CFiltersV2` (15 §7.1); `addrFinder.filter`, `existsAddrIndexFinder` (09 §6.3) | requests pipelined from one thread; the same set reaches the peer | — |
| CPU fan-outs (09 §6.3, 12 §7.3) | `thread::scope`, `available_parallelism` | caller drains the rest |
| Per ticket in `ProcessUnprocessedTickets` (11 §7.2), per output in `MixAccount` (13 §7.1), per confirmation client (08 §7.1) | scoped thread per item | rest run in turn |
| 1 per websocket request (16 §7.1) | thread per request | runs on the responder |
| Purchases, change mix (11 §7.2); mixing sessions (12 §7.3) | thread each, unjoined | that operation fails |
| Fee-payment `AfterFunc` (11 §7.2) | `clock.after_fn` | re-armed, same delay |
| 1 per `WrapContext` (13 §7.1, 15 §7.1) | none (below) | — |
| Connections, streams, dials | thread(s) each | dropped or failed (dcroxide `spawn_conn_thread`) |

   SPV mode with a few clients runs about 100 threads: 8 per peer (64 at 8
   peers), the mixing workers, 3 per websocket session (at most 25, 16
   §5.1.1), 2 per gRPC connection and 1 per stream. Peaks follow local counts
   (tickets, outputs) or client load.
4. **Refused threads.** Every thread started after start-up goes through one
   spawn helper returning the refusal as an error, handled per the table;
   `std::thread::spawn`, whose panic aborts, is for start-up only.
5. **Panics.** Release keeps `panic = "abort"`; `catch_unwind` is not
   recovery. Restart is the operator's service manager's, as for Go
   dcrwallet, which also exits on a panic outside an HTTP handler. Whether
   Decrediton restarts a wallet that exits is UNVERIFIED until its launcher is
   read (brief §6, Phase 0).
6. **Transactions take no context** (Go uses it only for tracing, 06 §7.3).
   They run to commit or rollback on the calling thread; a network call inside
   takes the caller's `Ctx`, and its error aborts through the closure's
   result.
7. **Deadlocking lock-order inversions** are fixed, not reproduced (open
   point 2).

This confirms the brief's recommendation and sharpens it with items 3 to 7
and the pattern below.

### Cancellation, time and timeouts

One crate (name per D1) provides `Ctx`, `Clock` and the spawn helper.

| Go | dcr-purse |
|---|---|
| `time.Now()` | `clock.now_wall()`, Unix nanoseconds (epochs, 12 §5.4; persisted times, 23 §5.6.3), and `clock.now_mono()`; timers and deadlines use the monotonic reading |
| `Background`, `WithCancel`, `WithTimeout`, `WithDeadline` | `Ctx::background(&clock)` where Go uses it; `ctx.with_cancel()` → `(Ctx, Cancel)`, `with_timeout`, `with_deadline`; dropping `Cancel` cancels; children are cancelled synchronously |
| `Done()`, `Err()` | a `recv(ctx.done())` arm that stays ready; two `purse-errors` sentinels with Go's texts (`crates/purse-errors/src/lib.rs:44-49`) |
| `WithValue` (remote address, `internal/rpc/jsonrpc/context.go:11-21`) | an explicit parameter |
| `context.AfterFunc` | `ctx.after_func(f)`: `f` runs on the cancelling thread and must not block |
| `WrapContext` (`chain/backend.go:94-106`) | `with_cancel()` plus `backend_done.after_func(cancel)`; no thread |
| `time.After`, `NewTimer` | `clock.after(d)`, a receiver the clock thread fires |
| `time.AfterFunc` | `clock.after_fn(d, f)`: `f` runs on a new thread from the helper, as Go's runs on its own goroutine; steps overlap, and `reset` does not wait (11 §7.2) |
| socket read | `ctx.after_func` raises the flag and shuts the socket; 1 s read slices (dcroxide `Teardown`) |
| dial | a dial thread from the helper; cancellation returns `context canceled` and abandons it (dcroxide `outbound.rs:595`); std's resolver has no timeout |
| `errgroup.WithContext` | a scoped group whose first error cancels it |

A wait Go writes as a `select` on the context is never a bare `recv`. The
clock thread only fires channel timers and starts `after_fn` threads, so no
callback delays a deadline. The fake clock advances wall and monotonic time
together and can step wall time alone; `advance` fires due timers in deadline
order and starts due callbacks on their own threads, returning handles to
join. Timing tested exhaustively (stall rules, mixing stages, fee-payment
retries) is a state machine fed clock readings inside a thin loop
(`crates/dcroxide-peer/src/deadline.rs`). Phase 5's equal notification
sequences (brief §6) come from driving the wallet from one scripted backend
thread.

### ADR-0004 and D11

This ADR admits no runtime. Any gRPC transport runs services as synchronous
code taking a `Ctx`, cancelled on client cancellation, stream end, connection
loss and server stop (19 §7.5, 20 §7), one handler per call, concurrently
(19 §7.1). A stream send returns once the transport has accepted the message
within a per-stream quota equal to grpc-go's 64 KiB write quota (grpc v1.83.2
`internal/transport/defaults.go:46`, `http2_server.go:1178`), so the syncer
and notifier are back-pressured as in Go (19 §7.3, 20 §7, 08 §7.1); a buffer
counted in messages is a divergence tied to D11. If ADR-0004 chooses tonic, it
amends this ADR to admit option C at that seam only, provided `deny.toml`
passes, tokio stays in `purse-grpc`'s transport module, wallet calls run on
std threads, a guard per handler future cancels its `Ctx` on drop, and sends
follow the rule above.

D11 (no ADR drafted yet) chooses between Go's unbuffered delivery under the
server mutex and bounded per-client queues (brief §9). A expresses either;
this ADR fixes only generation at Go's points, per-client order, and the
mutex's place in Go's lock order.

### Addendum (ADR-0014)

Accepted (project owner, 2026-11-19; ADR-0014 Open point 7). Items 2 and 3
stand, with these permitted departures, made for speed. Each is a
`PARITY.md` divergence with its own row, and none changes a wallet outcome:

- **Lock span (item 2).** Header validation and the chain switch run
  outside the lock the fetchers wait on (ADR-0014 S3). What a commit
  contains is unchanged except as ADR-0014 Open point 2 allows.
- **Request sets (item 3).** SPV sync may send requests Go's scheduler
  would not: headers more than one batch ahead (S5); a sliding window of
  `getcfsv2` with no barrier between batches (S6); one warm header peer
  (S7) and peers beyond 8 for header and filter requests (S9, split form),
  each only if its measurement shows a gain; parallel header streams from
  shipped anchors (S8); filter matching that overlaps fetching, with the
  next range prefetched (R6); matched blocks split across the 8 peers (R7).

Nothing else in items 2 and 3 is relaxed: lock order, transaction
boundaries and the points at which notifications are generated stay Go's.

## Consequences

- **Work created.** The context crate, before Phase 3's API settles. Item 3's
  mappings in the P2P, sync, JSON-RPC, staking and mixing crates, with their
  `PARITY.md` rows. User documentation that a panic stops the daemon, and a
  `PARITY.md` row for the case Go survives, a panicking POST handler.
- **Work avoided.** No runtime, no async bridges; dcroxide's crates and
  patterns carry over.
- **Risks accepted.** Whole-daemon timing tests run on real time. Threads grow
  with peers, sessions, streams and items, each reserving 2 MiB of address
  space. `GetTransactions`, `GetTickets` and `GetCFilters` send inside a read
  transaction (19 §7.4), so a slow client grows the redb file; proposed:
  accept it, as Go behaves so, rather than copy rows first (a divergence).
- **Constraints on others.** ADR-0003: a synchronous store API without a
  context, readers never waiting for the writer. ADR-0004, D11: as above.
  Every crate from `purse-udb` on: `Ctx`, `Clock`, the spawn helper.
- **Revisit** if ADR-0004 adopts tonic (by amendment, at that seam), if a
  realistic measurement shows threads to be a bottleneck (open point 4), or
  if dcroxide adopts a runtime.

## Open points

All four are decided by the project owner: point 1, for
`crossbeam-channel`, on 2026-11-18; points 2 to 4, as proposed, on 2026-11-19.

1. *Which channel and `select`?* Decided (project owner, 2026-11-18):
   `crossbeam-channel`, pinned, pure Rust, one dependency. Its `select!`
   shuffles ready operations with a per-thread xorshift seeded by a constant
   (0.5.17 `src/utils.rs:9-34`; `src/select.rs:199`, `:350`), which is a
   named exception to spec 23 §5.6.2's single randomness source. Go's
   `select` randomness is outside the deterministic build too, so its orders
   are compared as partial orders; where a choice is externally observable
   (spec 15 Q35) the port fixes it in code, per the standing quirk policy
   (`docs/QUIRKS.md`, preamble). Rejected: an in-tree channel whose `select`
   draws from `purse-rand`. dcroxide's lock holds no `crossbeam` crate, so
   this is a new dependency for the two projects.
2. *Which order resolves QK-0255?* Decided as proposed (project owner, 2026-11-19). Proposed: write transaction →
   `addressBuffersMu`, as `wallet/addresses.go:515-520` documents and every
   other site does (10 §6, ordering items 1, 2, 4); `DiscoverActiveAddresses`
   opens its transaction before the lock (`wallet/discovery.go:893-895`).
   Spec 09 §6.1 prescribes the reverse, against 06 §7.1 item 5; 09 is
   corrected under brief §4 principle 1, with a `PARITY.md` row.
3. *Are Go's goroutine leaks reproduced* Decided as proposed (project owner, 2026-11-19). (rescan saver, 09 §6.3; writer's
   inner task, 14 §6.1; an endless confirmation client, 08 §7.1)? Proposed:
   their observable effects are, such as the 30 s registration of a cancelled
   request; the leaked threads are not.
4. *Is multi-client RPC concurrency measured against Go before M3?* Decided as proposed (project owner, 2026-11-19). Proposed:
   yes, at Phase 7's exit (dcroxide left it open, ADR-0005 `:131-139`); a
   poor result leads to fixes behind the seams, not a runtime.
