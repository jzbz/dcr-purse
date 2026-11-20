# ADR-0015 — D11: Notification delivery

- **Status:** **Accepted** as proposed (decision D11; project owner,
  2026-11-19): option B, with the seven open points answered as
  recommended; see "Decision by the project owner" before "What was not
  tested". The repository facts and measurements under Context are not
  outcomes of it.
- **Date:** 2026-11-19

## Context

**The question.** How the wallet hands notifications to the clients that
subscribe to them: whether delivery keeps Go's unbuffered sends under the
notification server's mutex, which can make one client stall the wallet,
or is decoupled from the wallet by a queue per client, and if so what a
full queue does. The brief recommends keeping per-client order and the
points at which notifications are generated, decoupling delivery with
bounded per-client queues, and recording the divergence (brief §9, D11).

**By when.** Phase 5 builds the notification server, and its exit compares
notification sequences with Go's (brief §6 Phase 5); Phase 12 builds the
gRPC streams that carry them. The choice fixes the notification server's
interface, so it is needed before Phase 5 starts.

**Decisions in force.**

- [ADR-0008](0008-concurrency-model.md) (Accepted): OS threads and
  `crossbeam-channel`; "notifications are generated at Go's points"
  (Decision item 2); its section "ADR-0004 and D11" fixes generation at
  Go's points, per-client order and the server mutex's place in Go's lock
  order, and leaves to this record the choice between unbuffered delivery
  under the mutex and bounded per-client queues. A gRPC stream send returns
  once the transport has accepted the message within a 64 KiB per-stream
  quota, grpc-go's write quota; "a buffer counted in messages is a
  divergence tied to D11". Its addendum for ADR-0014 keeps "the points at
  which notifications are generated" Go's.
- [ADR-0004](0004-grpc-transport.md) (Accepted): tonic and prost, the
  tokio runtime confined to the listener; streaming bodies are fed through
  bounded channels within that quota; "D11 is decided inside the streaming
  bridge".
- [ADR-0003](0003-wallet-store-engine-and-schema.md) (Accepted): redb 5,
  one writer at a time, readers that never wait for it (ADR-0008, Facts).
- [ADR-0012](0012-standing-quirk-policy.md) (Accepted): a defect is
  reproduced when an external observer could depend on it (Rule 1);
  timing other than specified timeouts is not a surface; cases no rule
  fits go to the owner (Rule 11).
- [ADR-0014](0014-performance.md) (Accepted): speed is a goal everywhere,
  never at the expense of byte-compatible RPC results.

### What dcrwallet does at the pin

All Go citations are to dcrwallet `54b85b33` unless another module is
named.

**Who receives notifications.**

- *JSON-RPC clients receive none.* The websocket transport carries only
  replies: a reader, a responder and a writer per connection, replies
  handed to the writer over an unbuffered channel and written with a 2 s
  write deadline, after which the connection is dropped
  (`internal/rpc/jsonrpc/server.go:30-58`, `:338-499`, deadline `:475`).
  Spec 16 §4.1 item 4 records that the server has no notion of a
  notification. Notification-registration methods sent by a client
  (`notifyblocks`, `session`, …) are passed through to dcrd and act on the
  wallet's own dcrd connection (spec 16 §5.5.1, Q15; QK-0445), and
  whatever dcrd then sends is handled or silently ignored by the syncer
  (spec 15 §9 item 24). JSON-RPC therefore needs nothing from this
  decision, and the passthrough stays as it is.
- *gRPC `WalletService`*: `TransactionNotifications` and
  `AccountNotifications` (server streams) and `ConfirmationNotifications`
  (bidirectional), registered on stream start
  (`internal/rpc/rpcserver/server.go:2388-2414`, `:2416-2442`,
  `:2444-2509`).
- *gRPC `WalletLoaderService`*: the sync streams `RpcSync` and `SpvSync`
  send syncer progress from callbacks (`server.go:2832`, `:2984`; sends
  `:2883-2967`, `:3027-3131`).
- *In process*: the ticket buyer subscribes to main-tip changes
  (`ticketbuyer/tb.go:71`). `RemovedTransactionNotifications` has no
  in-tree subscriber (spec 08 §3.2).

**Where notifications are generated** (spec 08 §5.3, which this table
condenses):

| Notification | Generated in | Inside a write transaction | Wallet locks held |
|---|---|---|---|
| attached and detached blocks, mined transactions (accumulated) | `ChainSwitch`, `extendMainChain`, `processTransactionRecord` (`wallet/chainntfns.go:46-52`, `:125-146`, `:666-672`) | yes | `lockedOutpointMu` (`:110`) |
| unmined transaction (sent at once) | `processTransactionRecord` (`:659-665`), reached from publishing, authoring, ticket purchase and voting | yes | `lockedOutpointMu`, except voting (`:904`) |
| removed transaction | `ChainSwitch` (`:261-262`); `AbandonTransaction` (`wallet/wallet.go:5090`) | yes; no | `lockedOutpointMu`; none |
| account properties | `RenameAccount`, `NextAccount`, `ImportPrivateKey`, `ImportPublicKey` (`wallet/wallet.go:2130`, `:2236`, `:3734`, `:3771`) | no, after commit | none |
| main tip changed, then confirmation results, then the accumulated block notification | end of `ChainSwitch` (`wallet/chainntfns.go:312-313`) | no, after commit | none of the wallet's; in SPV mode the caller holds the syncer's `sidechainMu` (`spv/sync.go:1795-1835`) |

**How they are delivered.** One mutex `mu` guards the client lists and the
accumulated block notification (`wallet/notifications.go:40-51`). Every
client channel is unbuffered (`:489` and the three equivalents), and every
`notify*` method sends the same pointer to each client of its kind, in
list order, blocking on each, while holding `mu` (`:258-260`, `:351-355`,
`:584-586`, `:624-626`, `:733-735`). On a tip change the server also runs
`process` for every confirmation client, one goroutine each, and waits
for all of them to hand over their result, still holding `mu`
(`:737-747`). The only send outside `mu` is `Watch`'s immediate result
(`:889-892`), which blocks the gRPC request reader instead (spec 08
§5.27). Lock order: `lockedOutpointMu` → write transaction → `mu`, and
`mu` → read transaction (spec 08 §7.1). Per client, notifications arrive
in the order generated; between kinds and between clients there is no
guarantee except the sequence at the end of `ChainSwitch` (spec 08 §5.3,
§7.1; the type's own comment, `wallet/notifications.go:36-39`).
Deregistration is asynchronous: `Done` starts a goroutine that drains the
channel and one that removes it under `mu` (`:502-523`).

**What the gRPC layer adds.** Each streaming handler loops on receive,
marshal, `svr.Send` (`server.go:2396-2413`), so the wallet waits for a
client's handler only while that handler is still sending the previous
notification. grpc-go's `Send` returns once the message is accepted
against the stream's write quota of 64 KiB (grpc v1.83.2
`internal/transport/defaults.go:46`, `http2_server.go:1178`); the check
admits a message whenever any quota is left, so the quota can be
overdrawn by one message (`flowcontrol.go:54-67`), and quota comes back
only when the writer has put the bytes on the wire within the peer's
HTTP/2 window (`controlbuf.go:957-988`). dcrwallet sets no window, buffer
or keepalive option (`rpcserver.go:328-332`), so grpc-go's defaults hold:
a keepalive ping after 2 h idle with a 20 s timeout, and on Linux only a
20 s `TCP_USER_TIMEOUT` (`defaults.go:38-39`; `http2_server.go:238-248`;
`internal/syscall/syscall_nonlinux.go:67-70`).

**What a client that stops reading does to the wallet.** Measured below:
about 200 KB of messages are accepted for a stream whose client does not
read; then the handler blocks in `Send`, then the wallet blocks in its
next send to that client, holding `mu`. From there, read from the source
and not run against dcrwallet:

1. Every other notification and every registration and deregistration
   waits for `mu`, whatever its kind: an account or confirmation client
   that stops reading stalls transaction notifications too.
2. An unmined-transaction notification waits inside its write transaction
   with `lockedOutpointMu` held, so every write transaction of the wallet
   (one writer at a time) and everything that takes `lockedOutpointMu`
   (sending, coin selection, ticket purchase, chain switches) waits too.
   Readers carry on.
3. A stalled tip-change or block notification holds the syncer in
   `ChainSwitch`; in SPV mode with `sidechainMu` held. Block processing
   stops.
4. A confirmation client that stops reading blocks every tip change
   (`wallet/notifications.go:737-747`), and its `Watch` blocks its own
   request reader.
5. The stall lasts until the client reads, cancels or disconnects. A peer
   that vanishes with data unacknowledged is cut after 20 s on Linux; a
   peer that is alive but does not read is never cut.

The amount buffered is small against the notifications: a chain switch in
initial sync attaches the blocks of one header batch (at most 2,000, dcrd
`wire` v1.8.0 `msgheaders.go:15`), and each attached block with no wallet
transaction adds about 85 bytes to the `TransactionNotifications` message
(the fields of spec 08 §4.2; computed, not measured), so one or two such
messages fill a stalled stream's 200 KB.

**Sync streams.** Syncer callbacks run synchronously on the syncer
(`chain/sync.go:86-88`), and the loader's handlers call `svr.Send` inside
them, ignoring its error (spec 20 §7, "Stream sends"; QK-0571). A client
of `RpcSync` or `SpvSync` that stops reading stalls the sync it started;
`SpvSync`'s header progress is sent with `sidechainMu` held
(`spv/sync.go:1833`). Ending such a stream cancels its context, and with
it the sync (spec 20 §7, "Contexts").

**Existing ledger entries.** QK-0212 (unmined and removal notifications
sent inside a write transaction that may still fail; a slow subscriber
holds the writer open), QK-0200 (latent nil dereference in
`sendAttachedBlockNotification`), QK-0206, QK-0211, QK-0219, QK-0222,
QK-0224, QK-0529, QK-0543 (streams holding read transactions across
`Send`, which ADR-0008 Consequences proposes to reproduce), QK-0571. The
whole-wallet stall of points 1 to 5 had no entry; it is added as QK-0737.

### Measurements

Both on an x86_64 Linux desktop under load from other work; neither is a
benchmark of dcrwallet itself. Programs and raw output are kept outside
the repository.

**grpc-go stream buffering.** A grpc-go v1.83.2 server configured as
dcrwallet's apart from TLS (no options), one server-streaming method, and
a grpc-go client on the loopback interface that opens the stream and
never receives. The handler sends fixed-size messages until it makes no
progress for 2 s.

| message payload | sends completed before `Send` blocked | payload bytes |
|---:|---:|---:|
| 100 B | 1,838 | 183,800 |
| 1,000 B | 196 | 196,000 |
| 10,000 B | 20 | 200,000 |
| 100,000 B | 2 | 200,000 |

A client that read ten messages and then stopped gave the same 196. About
200 KB, independent of message size: more than the 64 KiB write quota and
the 65,535-byte initial stream window together; where the rest is held
(grpc-go's client can grow its windows by bandwidth-delay estimation) was
not traced. Decrediton's client library was not measured.

**Cost to the notifier.** One producer thread delivering 20,000
notifications to N subscriber threads, each subscriber spending 0 or 5 µs
per notification, with 20 µs of other work between notifications;
rendezvous channels (`crossbeam-channel` `bounded(0)`, Go's model) against
a bounded queue per subscriber. Time the producer spends per
notification, which in Go it spends holding `mu` and, for unmined
transactions, the write transaction:

| subscribers | subscriber work | rendezvous p50 / p99 | queue p50 / p99 |
|---:|---:|---:|---:|
| 1 | 0 µs | 0.8 / 30 µs | 0.04 / 2.8 µs |
| 1 | 5 µs | 2.0 / 205 µs | 0.06 / 2.9 µs |
| 4 | 5 µs | 17 / 1,142 µs | 1.1 / 8.5 µs |
| 16 | 5 µs | 46 / 1,558 µs | 2.3 / 30 µs |

The tail of the rendezvous column is thread wake-up under load; it is the
part a lock holder pays.

## Options considered

- **A. Go's model.** A rendezvous channel per client, sent to under `mu`
  in list order; the gRPC bridge's 64 KiB quota as ADR-0008 requires.
- **B. A bounded queue per client, counted in bytes; a full queue ends
  that client's stream.** Generation, content and enqueue order are Go's;
  the wallet never waits for a client.
- **C. A bounded queue per client; a full queue blocks the producer.**
  Go's model with a larger buffer.
- **D. A bounded queue per client; a full queue drops or coalesces
  notifications.**
- **E. An unbounded queue per client.**
- **F. B with a grace period**: a producer finding a queue full waits up
  to a deadline for room, then ends that stream.

| | A | B | C | D | E | F |
|---|---|---|---|---|---|---|
| Wallet blocked by a client | after about 200 KB (grpc-go; tonic to be measured), indefinitely | never | after the queue fills, indefinitely | never | never | up to the deadline per full queue |
| Notifications lost | never | the stalled client's stream ends (past its cap, or as the furthest behind when the total budget is full); it misses what follows until it resubscribes | never | silently | never | as B, after the deadline |
| Per-client order | Go's | Go's | Go's | broken | Go's | Go's |
| Memory | ~200 KB in the transport, then the wallet stalls | 8 MiB per client and 256 MiB for all notification streams together, queues and transport quota counted, shared contents once (Decision item 3) | the cap per stream, times the streams | the cap per stream, times the streams | unbounded: one stalled authenticated client can exhaust memory | as B |
| Cost under the locks | a handoff per client (table above) | a push per client | a push per client | a push per client | a push per client | a push, or a wait |
| PARITY entries | none, if the bridge buffers like grpc-go (else one line naming the bytes) | 1 to 3 below | one line: the buffer size | a broken delivery guarantee (spec 19 §7.3) | one line | as B, plus the deadline |

D is rejected: spec 19 §7.3 records that no notification is dropped or
coalesced for a slow client, and a client keeping its own wallet state
from the stream would be wrong without knowing it. E is rejected: an
authenticated client that stops reading can make the process grow until
the operating system kills it. A cap per stream alone does not escape
that objection: ADR-0004's caps (1,024 connections, 64 handler calls and
4,096 advertised streams per connection) bound the number of streams,
not the bytes queued for them, so 8 MiB per stream is 512 MiB per
connection at 64 handler calls (32 GiB at 4,096 streams, if long-lived
streaming handlers turn out not to hold handler-call slots under tonic,
which Phase 12 settles) and about 512 GiB across 1,024 connections. A
stalled client does not even need the wallet to be busy: Go's `Watch`
returns one result per requested hash, unknown hashes included
(`wallet/notifications.go:820-885`), so two 4 MiB requests of about
120,000 hashes each, on a bidirectional stream the client does not
read, fill a confirmation queue to 8 MiB. B is therefore B only with a
budget for all queues together (Decision item 3), which makes the
worst case a constant no number of clients can exceed. C keeps every stall of points 1 to 5 and
only delays it. F differs from B only for clients more than a full queue
behind; with the cap proposed below such a client is many times further
behind than one that already stalls dcrwallet, so the deadline buys
little and puts a timer on the write path.

That leaves A and B. B departs from spec 19 §12.3 item 5, which says the
notification streams "must block the wallet rather than buffer or drop"
(`docs/specs/19-grpc-wallet-service.md:2621-2623`): that hazard records
Go's behaviour as the port's requirement, while spec 08 §12 Hazard 1
leaves the same choice to one decision ("Decide once (§13)",
`docs/specs/08-wallet-core-part3.md:2072-2078`), which is this record. A
is exact but carries dcrwallet's failure mode: a
hung or malicious client holding a valid certificate stops block
processing, every write and every send, in SPV mode the sync too, and the
notifier pays a thread handoff per subscriber inside the write
transaction. B removes the stall; what it costs is a divergence visible
only to a client that falls far behind, and a stream that ends where Go's
would have stayed open.

**PARITY entries B needs**, each under `purse-wallet` or `purse-grpc`
with its QUIRKS entry:

1. A `TransactionNotifications`, `AccountNotifications` or
   `ConfirmationNotifications` stream whose client falls more than the cap
   behind, or holds the most queued bytes when the budget for all streams
   is full, is ended with `ResourceExhausted` and a message of dcr-purse's
   own (ADR-0011; no Go original); dcrwallet instead blocks the wallet
   until the client reads (spec 08 §7.1, spec 19 §7.3; QK-0737).
2. `ConfirmationNotificationsClient.Watch` returns without waiting for the
   client to take its immediate result (spec 08 §5.27 step 4), and a tip
   change does not wait for confirmation clients (spec 08 §5.26 step 2).
3. If Open point 3 is answered yes: notifications generated inside a write
   transaction that then fails are not delivered (QK-0212).

## Decision (accepted, 2026-11-19)

Option B, applied as follows.

1. **Generation stays Go's.** Every notification is generated at Go's
   point, under the notification server's mutex, in Go's lock order
   (`lockedOutpointMu` → write transaction → mutex; mutex → read
   transaction), with Go's content: computed in the same transaction from
   the same state, including the unmined hashes and balances read inside
   the write transaction (spec 08 §5.23) and the accumulation rules of
   spec 08 §5.24 with their quirks. The fixed sequence at the end of
   `ChainSwitch` is generated in Go's order: removals inside the
   transaction, then the tip change, then every confirmation client's
   result, then the block notification. This is ADR-0008's constraint,
   unchanged.
2. **A queue per client replaces the channel.** Generating a notification
   pushes one shared, immutable value onto the queue of every client of
   its kind, in list order, under the mutex; nothing in the push waits for
   a client. Per-client order is therefore generation order, which is
   Go's.
3. **Queues are counted in bytes, per client and in total.** Each value
   carries the encoded size of the protobuf message it becomes, computed
   once when generated. The cap is 8 MiB per client: about forty times
   what a stalled grpc-go stream already holds, and about fifty times the
   largest initial-sync block notification estimated above. Besides it,
   one budget of 256 MiB covers every notification stream of the process
   together. A value is charged to the budget once, when it is first
   pushed, and released when the last client holding it has had it
   accepted by the bridge and the bridge has returned its quota (the
   bytes are on the wire); a value shared by many clients is charged
   once, a confirmation client's results are its own. The budget
   therefore counts what the queues and the bridge's per-stream quota
   actually hold, whatever the number of streams. The worst case is
   256 MiB for the notification path, plus the fixed cost of each open
   stream (its handler thread and buffers, bounded by ADR-0004's stream
   and handler caps), against dcrwallet's ~200 KB before it stalls.
4. **A full queue ends that client's stream, never the wallet's work.**
   The push that would exceed a client's cap drops that client's queue,
   removes the client under the mutex as `Done` would, and marks the
   client overflowed; its handler then ends the stream with
   `ResourceExhausted` and a message of dcr-purse's own (PARITY entry 1).
   A push that would exceed the total budget does the same to the client
   holding the most bytes, repeatedly until the push fits; that is the
   client furthest behind, which may be the one being pushed to. Other
   clients are untouched. The overflow is logged at warning level with
   the stream's method, the client's address and which limit was hit.
5. **The gRPC side drains the queue through the bridge.** The handler
   thread takes values in order, converts them as Go's handler does
   (spec 08 §4.2; `server.go:2398-2404`, `:2426-2432`, `:2473-2488`) and
   sends through ADR-0004's bridge within the 64 KiB per-stream quota.
   Back-pressure from the client stops at the queue.
6. **Confirmation clients.** `Watch`'s immediate result and each tip
   change's `process` result go onto the same per-client queue, in
   generation order, which is an interleaving Go's two goroutines permit
   (spec 08 §5.27, "interleave in channel-rendezvous order"). On a tip
   change the server runs every client's `process` (ADR-0008 item 3,
   "scoped thread per item") and continues once all have pushed, without
   waiting for delivery (PARITY entry 2).
7. **The sync streams keep Go's back-pressure.** `RpcSync` and `SpvSync`
   send from the syncer's callback through the bridge, blocking within
   the quota, as Go does. Ending the stream would cancel the sync (spec 20
   §7), and dropping progress would change what the client sees; the
   stall stays confined to the sync that client started (Open point 4).
8. **In-process subscribers keep Go's rendezvous.** The ticket buyer, the
   only one at the pin (`ticketbuyer/tb.go:71`), receives main-tip changes
   on a channel of capacity 0 (`crossbeam-channel` `bounded(0)`), sent to
   under the mutex in list order as Go does
   (`wallet/notifications.go:692-699`, `:730-734`). The case for B is a
   client that stops reading; the ticket buyer is the wallet's own code,
   and its step per notification is short and takes nothing the producer
   holds: a rescan-point read and a header read, each a read transaction,
   cancelling earlier purchases, updating `nextIntervalStart` and
   `expiry`, and starting purchases and change mixing on other threads
   (`tb.go:91-212`). The rendezvous keeps Go's bound on how far the
   wallet runs ahead of the buyer: `ChainSwitch` calls are serialised (in
   SPV mode under `sidechainMu`; the RPC syncer is serial), so the
   producer can commit tip N+1 while the buyer handles tip N but then
   waits, holding the mutex, in N+1's send; the state the buyer reads is
   at most one tip past its notification. A queue, even a blocking one,
   would let the wallet commit many tips meanwhile, and the buyer would
   cancel purchases and set `expiry` from a height several tips old.
   The cost is Go's: while the buyer finishes a step, the tip change's
   confirmation results, the block notification and every other
   notification wait, bounded by the buyer's own step and not by any
   client. No PARITY entry (Open point 7).
9. **Not changed:** what is notified and when, the content and the
   protobuf mapping, the empty confirmation result on every tip change
   (QK-0529), the absence of replay for a client that subscribes late,
   swap-removal of clients (which no client can observe once sends no
   longer block), the ticket buyer's rendezvous (item 8), and the
   notifications QK-0212 describes (Open point 3).

## Consequences

- **The stall is gone for the notification streams.** No client of
  `TransactionNotifications`, `AccountNotifications` or
  `ConfirmationNotifications` can make the wallet's writer,
  `lockedOutpointMu`, the chain switch or the syncer wait. The cost under
  the locks becomes a push per client: microseconds where the rendezvous
  measured up to 1.6 ms at the 99th percentile with sixteen subscribers.
- **What a client sees changes only past the cap.** A client within 8 MiB
  of the wallet sees exactly Go's sequence, unless the 256 MiB budget is
  full and it is the furthest behind of all. One further behind loses its
  stream with `ResourceExhausted`, where dcrwallet would by then have
  been stalled for as long. Whether Decrediton resubscribes or reports an
  error is not known (Open point 2).
- **Memory** of the notification path is bounded by the 256 MiB budget,
  whatever the number of streams, with no client holding more than
  8 MiB; the fixed cost per open stream is bounded by ADR-0004's caps.
  The budget is a divergence only in when a far-behind stream ends.
- **ADR-0004 gains an addendum if B is accepted.** ADR-0004 says "D11 is
  decided inside the streaming bridge" (`docs/adr/0004-grpc-transport.md:711-712`,
  `:746-747`). B places D11's mechanism ahead of the bridge: the byte-counted
  queue, the per-client cap, the total budget and the overflow removal
  live in `purse-wallet`'s notification server, because the queue must be
  filled and the client removed under the server's mutex; the bridge stays
  ADR-0008's plain 64 KiB quota sender and reports when quota comes back,
  which the budget needs. The addendum says so. Under A nothing moves:
  the bridge's buffering is all there is (Open point 1).
- **Phase 5's comparison** of notification sequences compares, per
  subscriber, the generated sequence with Go's delivered sequence; they
  are equal by construction unless a queue overflows, which the scripted
  backend's subscribers never let happen.
- **Specifications, if B is accepted.** Spec 19 §12.3 item 5 is listed in
  `docs/specs/README.md` among the superseded passages, kept as written:
  superseded by this record for `TransactionNotifications`,
  `AccountNotifications` and `ConfirmationNotifications`, while the sync
  streams keep Go's back-pressure (Decision item 7) and the ticket buyer
  its rendezvous (Decision item 8). Spec 08 §12 Hazard 1's "Decide once"
  is answered by this record, which the same list says. Under A neither
  passage changes.
- **Work created.** The queue type in `purse-wallet` with byte accounting,
  the total budget and overflow; the bridge's report of returned quota;
  the addendum to ADR-0004; the superseded-passage entries above; the handler loops and the overflow status in `purse-grpc`;
  tests: per-client order under concurrent producers, overflow of one
  client leaving the others' sequences intact, a stalled client not
  delaying a concurrent write transaction (a timing test with a
  generous bound), the sync streams' back-pressure, and the ticket
  buyer's bound: with the buyer held inside its step for tip N, a test
  that drives further chain switches finds the producer blocked in tip
  N+1's send, never committing N+2; QK-0737's disposition
  and the PARITY lines when the code exists.
- **Revisit** if Decrediton turns out to treat an ended notification
  stream as fatal (Open point 2), or if Phase 12's measurement of the
  tonic bridge shows a buffering very different from grpc-go's.

## Open points

Each for the owner, with a recommendation. All seven were decided as
recommended (project owner, 2026-11-19); each is marked.

1. *Option B rather than A?* **Decided as recommended (project owner, 2026-11-19).** Recommended: B, as above. A is exact, but it
   lets one hung or hostile client with a valid certificate stop the
   wallet's writes, sends and block processing (QK-0737), and puts a
   thread handoff per subscriber inside write transactions; B diverges
   only for a client already far enough behind to have stalled
   dcrwallet. If A is chosen, ADR-0008's quota rule is all that is
   needed, and the bridge's buffering is measured against the ~200 KB
   above.
2. *What Decrediton does when a notification stream ends.* **Decided as recommended (project owner, 2026-11-19).**
   Decrediton's
   source was not among the inputs of the specs (brief §6 Phase 0).
   Recommended: read its handling of `TransactionNotifications`,
   `AccountNotifications` and `ConfirmationNotifications` errors with its
   call inventory before Phase 12, and choose the status code then:
   `ResourceExhausted` names the cause; `Unavailable` is the code
   clients conventionally retry. Until then `ResourceExhausted` is the
   proposal.
3. *Hold notifications generated inside a write transaction until it
   commits?* **Decided as recommended (project owner, 2026-11-19).** This would fix QK-0212: a client is never told of an unmined
   transaction whose recording failed, and a client that queries right
   after a notification always finds the transaction, where in Go the
   query can run before the commit and miss it. Keeping per-client order
   consistent with what readers can see then needs the transaction to
   commit while holding the server's mutex, so that no notification
   generated from a later read overtakes the held ones. Recommended: not
   in 1.0. The defect needs a failed write transaction, no client can
   depend on it either way, and the change touches the store's commit
   path; reproduce it (Rule 1) and revisit with Phase 5's code.
4. *Sync streams.* **Decided as recommended (project owner, 2026-11-19).** Recommended: keep Go's back-pressure (Decision item
   7). Alternatives: a queue that ends the stream on overflow, which
   cancels the sync; or coalescing progress callbacks, which changes the
   sequence a client sees. The stall harms only the client's own sync.
5. *The cap and the budget.* **Decided as recommended (project owner, 2026-11-19).** Without a total, the per-client cap bounds
   nothing useful: at ADR-0004's caps 8 MiB per stream is 512 MiB per
   connection and about 512 GiB in all (Options considered), and two large
   `Watch` requests reach a client's cap with an idle wallet.
   Recommended: 8 MiB per client and 256 MiB for all notification streams
   together, queues and the bridge's quota counted, shared values once,
   the client furthest behind ended when the total is full; constants,
   revisited with measurements of real notification sizes in Phase 5
   (wallets with many unmined transactions make every transaction
   notification larger, since each carries all unmined hashes, spec 08
   §5.23). Alternatives: count confirmation queues against ADR-0004's
   64 MiB per-connection request budget, which bounds memory per
   connection but not in total and mixes request and notification
   accounting; or a cap on the number of notification streams, which adds
   a refusal dcrwallet does not have. A configuration option is not
   proposed: dcrwallet has none to mirror.
6. *QK-0737 upstream.* **Decided as recommended (project owner, 2026-11-19).** A candidate for D13: low severity, an
   authenticated client can stall the wallet. Recommended: leave it to
   ADR-0017. *ADR-0017's route was decided otherwise (project owner,
   2026-11-19): no bug bounty and no public issue filed by this project;
   the owner delivers the reports to the developers privately. QK-0737
   follows that route.*
7. *In-process subscribers: rendezvous or queue?* **Decided as recommended (project owner, 2026-11-19).** Recommended: the
   rendezvous (Decision item 8), Go's exact interleaving, with no PARITY
   entry; the ticket buyer is trusted code with a short step, so the
   objection to A does not apply to it. Alternative: a queue that blocks
   the producer when full, at the client cap, which takes the buyer's
   step off the mutex and the syncer but lets the wallet run any number
   of tips ahead of the buyer; under ADR-0008 item 3 it then needs a
   PARITY entry, since the buyer can start purchases with an `expiry` and
   cancellations computed at a stale height, which Go's scheduler never
   allows past one tip.

## Decision by the project owner (2026-11-19)

Accepted as proposed, every open point as recommended (the owner's one
exception that day concerns reporting upstream, ADR-0017, and reaches this
record only through Open point 6). The answers:

1. *Option B.* Yes: a byte-counted queue per client, the wallet never
   waiting for a notification-stream client.
2. *Decrediton's handling of an ended stream.* Read with Decrediton's call
   inventory before Phase 12, and the status code chosen then;
   `ResourceExhausted` until then.
3. *Holding notifications until their write transaction commits.* Not in
   1.0: QK-0212 is reproduced (ADR-0012 Rule 1) and revisited with Phase
   5's code.
4. *Sync streams.* Go's back-pressure is kept (Decision item 7).
5. *Limits.* 8 MiB per client and 256 MiB for all notification streams
   together, queues and the bridge's quota counted, shared values once,
   the client furthest behind ended when the total is full; as constants,
   revisited with Phase 5's measurements of real notification sizes.
6. *QK-0737 upstream.* Left to ADR-0017, and so to the owner's private
   delivery of reports decided there.
7. *In-process subscribers.* The ticket buyer keeps Go's rendezvous, with
   no PARITY entry (Decision item 8).

Written by the same change, as Consequences prescribes for acceptance:
ADR-0004's addendum of 2026-11-19 (ADR-0015); spec 19 §12.3 item 5 and spec 08 §12
Hazard 1 listed among the superseded passages in `docs/specs/README.md`;
QK-0737's and QK-0212's dispositions in `docs/QUIRKS.md`. The rest of
"Work created" (the queue type, the bridge's report of returned quota,
the handlers and their tests, the PARITY lines) comes with the code in
Phases 5 and 12.

## What was not tested

- No dcrwallet process was run: the whole-wallet stall (points 1 to 5)
  is read from the source; only grpc-go's transport buffering was
  measured, with a grpc-go client, not Decrediton's.
- The tonic bridge's buffering is not measured; it does not exist yet.
- The 85-byte figure per attached block is computed from field
  encodings.
- The notifier cost table is a model with spinning subscribers, not the
  wallet.
