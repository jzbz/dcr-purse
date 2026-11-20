# ADR-0004 — D5: gRPC transport and protobuf

- **Status:** **Accepted** (project owner, 2026-11-19), against the proposal: option (A),
  tonic, not (B1); the other questions as recommended. See "Decision by
  the project owner" below; the proposal is kept as the record.
- **Date:** 2026-11-19. Revised the same day after an adversarial review of
  the spike: the first draft's case for the in-tree server left out the I/O
  and concurrency work a production server needs, and stated more than its
  tests showed in several places (listed under "Corrections to the first
  draft"). The in-tree spike was fixed where the fix was contained, every
  case was re-run against grpc-go, and the comparison below is rebuilt from
  those runs. The recommendation changed.

## Context

dcrwallet serves its gRPC API (eleven services, 88 RPCs, 222 messages in
`rpc/api.proto`; 11 server-streaming RPCs and one bidirectional) over HTTP/2
with mandatory mutual TLS, through grpc-go v1.83.2 configured with
`grpc.Creds` and two interceptors and nothing else
([spec 19](../specs/19-grpc-wallet-service.md) section 4.1, section 5.1.1;
dcrwallet `rpcserver.go:328-332`). Surface W5 of the brief requires
Decrediton to run unmodified against dcr-purse, which makes grpc-go's
observable behaviour the specification: statuses and their texts, headers
and trailers, and transport policy such as keepalive enforcement.

Decision D5 of the brief offers three options and leaned to (C) with (A)
confined to the gRPC listener as the fallback, to be settled by a Phase 0
spike (brief section 9, D5); risk R5 is the same question ("either an async
runtime enters a wallet process or the project owns HTTP/2", brief R5).

Decisions in force that bear on it, of 2026-11-18 and 2026-11-19:

- **Concurrency** ([ADR-0008](0008-concurrency-model.md), Accepted): OS
  threads everywhere and `crossbeam-channel`; an async runtime is allowed
  only inside the gRPC listener, and only if this spike calls for it.
  Everything behind the listener, the handlers included, stays on threads
  and the wallet's blocking API; a guard per handler future cancels its
  `Ctx` on drop, and a stream send is back-pressured by a 64 KiB per-stream
  quota, grpc-go's write quota (ADR-0008, "ADR-0004 and D11"; grpc
  `internal/transport/defaults.go:46`). The policy objection to (A) is
  therefore gone; what remains is its dependency graph, its bridge to the
  blocking API, and its behaviour.
- **Error texts** ([ADR-0011](0011-error-message-fidelity.md), Accepted):
  status codes are always exact; message text is byte-exact by default, with
  named exceptions in a ledger. ET-003 covers grpc-go transport statuses not
  in spec 20 section 8.1: exact once this spike enumerates them.
- **Quirks** ([ADR-0012](0012-standing-quirk-policy.md), Accepted): a defect
  is reproduced when an external observer could depend on it (Rule 1).
- **dcroxide crates** ([ADR-0006](0006-consuming-dcroxide-crates.md),
  Accepted): one pinned revision; what the wallet needs from dcroxide (the
  client-certificate verifier among them, rule 3's table, Phase 7) lands in
  dcroxide first; a third-party major that dcroxide's lock also holds is
  taken at dcroxide's version, or a `bans.skip` is mirrored or justified
  (rule 5).
- **Version skew** ([ADR-0009](0009-version-skew-against-dcroxide.md),
  Accepted; [SKEW.md](../SKEW.md)): the reference Go is go1.27.4;
  `tools/oracle/go.mod` carries `godebug default=go1.25`, the GODEBUG
  defaults dcrwallet's daemon builds with (its `debug.go`; SKEW.md item 4).
- **Performance** (project owner): speed is a design input everywhere; it
  never overrides correctness of keys and funds, constant-time arithmetic on
  secrets, byte-compatible RPC results, or being served correctly by nodes
  and peers. ADR-0014, in preparation, will state the policy; this record
  reports measurements against it and does not anticipate it.
- **What ships is pure Rust**; tests and CI may use other languages
  (`deny.toml`, header comment).

TLS everywhere is rustls 0.23 on dcroxide's in-tree provider
`dcroxide-tlscrypto`, built with `builder_with_provider` (dcroxide
ADR-0015).

### What was built

| Piece | Path | What it is |
|---|---|---|
| Variant C | `spikes/grpc-intree/` | hand-written HTTP/2 as a state machine with no I/O (`src/conn.rs`): preface, SETTINGS, HEADERS/CONTINUATION decoded fragment by fragment with x/net's rules, DATA with per-stream admission against a request budget, WINDOW_UPDATE flow control, PING with grpc-go's keepalive enforcement, GOAWAY, RST_STREAM, an in-flight limit per connection; HPACK (`src/hpack.rs`, `src/huffman.rs`); gRPC framing, grpc-go's request checks and status texts, trailers (`src/grpc.rs`); a protobuf codec for `VersionRequest`/`VersionResponse` (`src/proto.rs`); rustls on `dcroxide-tlscrypto`; the I/O layer (`src/server.rs`): connection and handshake caps at accept, an absolute deadline to the client's first SETTINGS, a reader and a writer thread per connection with blocking I/O, only the writer touching the socket, a worker pool for handlers |
| Variant A | `spikes/grpc-tonic/` | tonic 0.14.9 and prost 0.14.6 on hyper 1.12, h2 0.4.20 and tokio 1.54, the runtime owned by the listener alone; the same TLS configuration under tokio-rustls; tonic's transport (and with it axum and tonic's TLS features) left out, each connection served by hyper's HTTP/2 server with tonic-build's service shape written by hand; hyper's options set toward grpc-go's defaults (no advertised stream limit, 65,535-byte windows without adaptive sizing, 16 MiB header list, a ping after 2 h idle with a 20 s timeout, no `date` header; `src/server.rs`); wallet calls through `blocking` (`src/lib.rs`): `spawn_blocking` plus a flag set when tonic drops the call's future |
| The client | `tools/oracle/cmd/grpcclient` | an unmodified grpc-go v1.83.2 client with `decred.org/dcrwallet/v5/rpc/walletrpc`, speaking the oracle's line-JSON protocol: PKI generation; sequential and concurrent `Version` calls with latency percentiles and the negotiated TLS parameters; calls to arbitrary methods with raw bodies; concurrent large requests (`bigcalls`); a large metadata value (`metacall`); calls cancelled by their deadline, reporting whether the connection survived (`cancels`); a client that never reads (`slowread`); an HTTP/1.1 request; raw HTTP/2 frame scripts read back with x/net's framer; bursts of streams |
| The reference | same binary (`server.go`) | a grpc-go server built as `startRPCServers` builds dcrwallet's (`tls.Config` with the key pair, MinVersion TLS 1.2, ClientCAs from the client CA file, `RequireAndVerifyClientCert`, `credentials.NewTLS`) with dcrwallet's `VersionService` implementation, so every case runs against grpc-go and each Rust server |

Certificates are made by the oracle with `crypto/x509` and the parameters of
the code that makes them in dcrwallet, because dcrd's `certgen` v1.2.0 is in
dcrwallet's module graph but not in the oracle's `go.sum` (`pki.go`, header
comment): the server pair follows certgen (`NewEd25519TLSCertPair` for
dcrwallet's default `--tlscurve` Ed25519, `NewTLSCertPair` for the NIST
curves), except that the names are `localhost`, 127.0.0.1 and ::1; the
client CA and client certificate follow `--issueclientcert`. The oracle is
built with the reference Go, go1.27.4, and runs with go1.25's GODEBUG
defaults, as dcrwallet's daemon does (`tools/oracle/go.mod`). Everything
listens on 127.0.0.1 with OS-assigned ports; keys and certificates live in
test temporary directories.

## Options considered

- **(A) tonic/prost on tokio/hyper/h2, confined to the listener.** The
  mature Rust gRPC stack. ADR-0008 permits the runtime at this seam if this
  spike calls for it; every handler is async and reaches the wallet through
  a blocking bridge.
- **(B) Something between.** Two shapes:
  - **(B1) hyper/h2 on tokio at the listener, gRPC layer and codec in
    tree, no tonic or prost.** HTTP/2, the connection I/O and TLS I/O are
    hyper's, h2's and tokio-rustls's; the gRPC layer (framing, grpc-go's
    request checks, statuses and trailers, the router) is the in-tree
    spike's `src/grpc.rs` and the request-header checks of `src/conn.rs`,
    run on hyper's request and body types. Not built as a variant: its
    transport half is variant A's hyper configuration, its gRPC half is
    variant C's code, and the evidence below prices each half.
  - **(B2) in-tree HTTP/2 with prost-generated messages.** The codec
    question only; it does not change the transport comparison.
- **(C) In-tree HTTP/2, HPACK, gRPC framing and protobuf codec** on OS
  threads with blocking I/O, consistent with dcroxide's in-tree HTTP/1.1 and
  websocket and with ADR-0008 without its exception.

## Evidence

### grpc-go client cases

Thirteen cases with the generated grpc-go client (`tests/interop.rs`,
`grpc_go_client_cases`, in both spikes). What the test asserts differs by
row: for the successful calls, the reply, the response headers and the
trailers equal the reference's; for the gRPC error rows (unknown service,
unknown method, malformed message, metadata with a deadline), the code, the
message, the headers and the trailers equal the reference's; for the TLS
and transport failures, only the code (14, Unavailable). "=" below means
"equal on what the test asserts".

| Case | grpc-go reference | C (in-tree) | A (tonic) |
|---|---|---|---|
| Issued (CA-signed) Ed25519 client certificate, TLS 1.3 | OK, `version_string "10.0.0"`, `major 10`; TLS_AES_128_GCM_SHA256, X25519MLKEM768, h2 | = (TLS_AES_256_GCM_SHA384, X25519) | = (same) |
| Same, TLS 1.2 | OK; ECDHE_ECDSA_AES_128_GCM_SHA256, X25519 | = (ECDHE_ECDSA_AES_256_GCM_SHA384) | = (same) |
| Self-signed client certificate (no CA flag) listed in `clients.pem` | OK | = | = |
| Self-signed client certificate with CA flag (certgen's template) in `clients.pem` | OK | refused: `remote error: tls: unknown certificate` | refused, same |
| No client certificate | Unavailable; the text varied: `write: broken pipe` in two of four runs, `tls: certificate required` in two | Unavailable, `tls: certificate required` in all four runs | Unavailable, `tls: certificate required` |
| Client certificate from another CA | Unavailable, `tls: unknown certificate authority` | Unavailable, `tls: error decrypting message` | same as C |
| Expired client certificate | Unavailable, `tls: expired certificate` | Unavailable, same text | same |
| Plaintext (insecure credentials) | Unavailable; `connection reset by peer` in two of four runs, `EOF` in two | Unavailable, `unexpected EOF` | same as C |
| HTTP/1.1 over TLS (ALPN `http/1.1`) | EOF (Go completes the handshake through its http/1.1 fallback, grpc-go's ALPN check closes) | `remote error: tls: no application protocol` | same as C |
| Unknown service | Unimplemented, `unknown service walletrpc.NoSuchService` | = | same code and message; trailers carry `content-type: application/grpc` where grpc-go echoes `application/grpc+proto` (not asserted) |
| Unknown method | Unimplemented, `unknown method NoSuchMethod for service walletrpc.VersionService` | = | as for unknown service |
| Malformed request message | Internal, `grpc: error unmarshalling request: proto: cannot parse invalid wire-format data` | = (Go's text has a space or a U+00A0 after `proto:`, which protobuf-go's `detrand` chooses per binary; C writes a space) | Internal, `failed to decode Protobuf message: buffer underflow` |
| Call with metadata and a deadline | OK, reply bytes `0a0631302e302e30100a` (spec 19 section 5.5) | = | same reply; content-type not echoed |

A TLS failure's text is composed by the client from what it read on the
socket before the close, so it depends on timing on both sides: four runs
of `grpc_go_client_cases` gave two different texts from grpc-go for two
cases, and the review saw C's TLS 1.3 refusal of a P-521 client surface as a
broken pipe as well as `certificate required` (review record, P-521
finding). Only the code is pinned for these rows, and ADR-0011's ET-001
already treats such texts as own text.

### Raw HTTP/2 cases

Sixty frame scripts (`tests/common/mod.rs`, `raw_cases`) sent through the
oracle after the preface and a SETTINGS frame, every frame the server sent
read back with x/net's framer and summarized (the server's own SETTINGS,
SETTINGS ACKs, its PINGs and WINDOW_UPDATEs left out). Run after the fixes
(`raw_frame_cases` in both spikes).

| | C (in-tree) | A (tonic) |
|---|---|---|
| Same as grpc-go | 31, of which 28 asserted in the test (`INTREE_PARITY`, compared without `RST_STREAM(NO_ERROR)`, whose presence after a status depends on goroutine timing in grpc-go) | 5 |
| Differs by timing | 1 on this run (the order two concurrent streams finish in). Two more are timing-dependent and were equal on this run, so they are not asserted: a decreasing stream id, where grpc-go's answer to the earlier stream races its own GOAWAY in the control buffer (grpc `internal/transport/http2_server.go:679-687`; grpc-go answered it on one run and not on another), and a reply racing the client's later over-window DATA | 2: the stream order, and the decreasing stream id (no answer from either server on that run, GOAWAY without grpc-go's debug text from h2) |
| A grpc-go quirk not reproduced | 1: a HEADERS frame with END_STREAM and no message gets no answer from grpc-go (the handler's read blocks until the client cancels); C answers Unknown `EOF` | 1: tonic answers Internal `Missing request message.` |
| GOAWAY (code, debug text) before closing, where grpc-go closes silently | 18: frames over MAX_FRAME_SIZE, DATA on stream 0, five HPACK and Huffman errors, the HPACK size update above 4096, three CONTINUATION violations, the HPACK bomb and the 16 KiB-frame CONTINUATION flood (both closed by x/net's rule, below), four SETTINGS/PING length and range errors, WINDOW_UPDATE of 0 | 18, with empty debug text and PROTOCOL_ERROR where C sends COMPRESSION_ERROR or FRAME_SIZE_ERROR, and ENHANCE_YOUR_CALM `header_list_way_too_large` for the bomb |
| Stricter than grpc-go, which ignores or accepts | 9: DATA and RST_STREAM on idle streams, PUSH_PROMISE from a client, ENABLE_PUSH 2, MAX_FRAME_SIZE 100, connection window overflow, an HPACK size update after a field, a HEADERS frame depending on itself, rapid reset (2,000 opened-and-reset streams) | the same 9, plus 10,000 empty CONTINUATION frames (h2's `too_many_continuations`), DATA past the connection window as a connection error, and a bad preface |
| Differs otherwise | 0 | 25 (below) |

What changed in C since the first draft's table: 10,000 empty CONTINUATION
frames are now accepted, as grpc-go accepts them (they were refused at a
128-frame cap that also refused valid input; now asserted); the HPACK bomb
now closes the connection as grpc-go does, because a non-empty fragment
after the header-list budget is spent closes it in x/net
(`golang.org/x/net@v0.60.0/http2/frame.go:1784-1795`); C adds its GOAWAY.

Variant A's 55 differences divide by where they live:

- **gRPC layer, 19 cases, all changeable without touching h2.** On
  well-formed calls: `grpc-message` omitted from trailers when empty (5
  cases), the response content-type not echoing the request's subtype, and
  spaces percent-encoded in `grpc-message` (2). On malformed calls: tonic's
  own texts (truncated message, compressed flag, payload format, gzip, a
  malformed body, a HEADERS-only request) and its own code in two (OutOfRange
  where grpc-go answers ResourceExhausted `grpc: received message larger than
  max (5242880 vs. 4194304)`; Internal where grpc-go answers Unknown
  `unexpected EOF`); and, served without tonic's transport, leniency where
  grpc-go refuses (`content-type: text/plain`, no `:authority`,
  `:method GET`, a malformed or zero `grpc-timeout`). A tower layer and a
  wrapper around tonic's request and response bodies can reach every one; so
  can (B1), whose gRPC layer is C's, which produces grpc-go's output for all
  19 already.
- **HTTP/2 layer, 36 cases, inside h2 (a fork to change).** A malformed
  `:path` refused by h2 before the service sees it; an uppercase header
  name and an unknown pseudo-header as connection errors where grpc-go
  resets the stream; GOAWAY without grpc-go's debug text for illegal stream
  ids (2); the 18 GOAWAYs and 12 strictnesses above; client PINGs not
  policed (1,000 pings acknowledged, where grpc-go sends GOAWAY
  ENHANCE_YOUR_CALM `too_many_pings` after the fourth; grpc
  `http2_server.go:930`); and a bad preface. Of these 36, only the PING
  policy is reachable by a client that follows the protocol: a client whose
  keepalive pings come faster than grpc-go's enforcement allows would be
  disconnected by grpc-go and not by h2, a leniency. The other 35 need input
  no conforming HTTP/2 client sends.

C's differences are all on invalid input, where its choices (a GOAWAY with
code and debug text, RFC 9113's validity checks, flood limits) are
deliberate and each can be switched to grpc-go's silence or leniency. Every
grpc-go text on the request path the cases reach is reproduced, down to the
GOAWAY debug text for an illegal stream id (x/net's `FrameHeader.String`).

Which client could observe what: dcrctl speaks dcrwallet's JSON-RPC API,
not gRPC (dcrwallet `README.md:139-140` names it the RPC client; its gRPC
client guide, `rpc/documentation/clientusage.md`, does not mention it; not
checked against dcrctl's source). Decrediton's gRPC library was not checked
(dcrwallet's guide describes the Node.js `grpc` package over gRPC Core,
`clientusage.md:348-360`). A client built on any conforming library sees,
from A as configured, the gRPC-layer differences on well-formed calls only
as metadata or wire bytes (a library decodes percent-encoding and treats a
missing `grpc-message` as empty), the malformed-call codes and texts only
if it sends such a call (an over-4 MiB request is the realistic one), and
the PING leniency. From C and from (B1) it sees none of those.

Two observations from the reference: grpc-go's server sends SETTINGS with
MAX_FRAME_SIZE 16384 and nothing else (as dcrwallet configures it, no
MAX_CONCURRENT_STREAMS: grpc `http2_server.go:182-191`); and it answers
`grpc-encoding: gzip` with Unimplemented `grpc: Decompressor is not
installed for grpc-encoding "gzip"`, so no gzip compressor is registered in
a binary linking dcrwallet's `wallet` and `walletrpc` packages (evidence
toward spec 19's UNVERIFIED compression item; the oracle is not dcrwallet's
own binary).

### I/O-level cases

The first draft's tests drove the state machine in memory and the HTTP/2
parser through TLS, never the listener, its threads or its writes. The
review found the in-tree server's I/O layer failing in exactly those
places. These cases now run against each server through the real grpc-go
client or raw sockets (`tests/interop.rs` in C: `concurrency_and_bursts`,
`large_concurrent_requests`, `large_request_headers`,
`cancelled_calls_keep_the_connection`,
`slow_reader_cannot_stall_other_clients`,
`shutdown_does_not_wait_on_a_stuck_peer`,
`unauthenticated_sockets_are_bounded`; in A: `io_level_cases`). The
"C, first version" column is the same cases run, in release, against a copy
of the spike as the first draft described it (not kept in the repository).

| Case | grpc-go reference | C, first version | C, fixed | A (tonic) |
|---|---|---|---|---|
| 20,000 concurrent unary calls on one connection (grpc-go client) | all OK | 8,229 failed: Unavailable, `stream terminated by RST_STREAM with error code: REFUSED_STREAM` | all OK | all OK |
| A 3,000,000-byte metadata value (184 frames from grpc-go's client) | OK | Unavailable, `the connection is draining` | OK | OK |
| 1,500 calls cancelled by their deadline while their handlers run | no slow method in the reference; by reading, a client reset never closes the connection (grpc `http2_server.go:837-849`) | 2 failed Unavailable, and grpc-go had to reconnect (GOAWAY ENHANCE_YOUR_CALM) | 1,500 DeadlineExceeded, same connection | 1,500 DeadlineExceeded, same connection |
| 20 requests of 4,000,000 bytes at once (80 MB on one connection) | OK | all DeadlineExceeded after 30 s: the connection wedged | OK (and 1,000 x 100,000 bytes; in the debug test, past a 4 MiB budget) | OK |
| A client that writes requests and never reads, with five other clients timed meanwhile | the others answered in 1.2–2.1 ms; its connection still open after 3 s | the others failed (`context deadline exceeded` after 3 s) | the others answered; its connection closed at the write deadline (2 s in the test) | the others answered; its connection closed by the server within 3 s (cause not identified) |
| Shutdown while that client is stuck | not tested | blocked on its lock (review, by reading) | returned in 0.06–17 ms | not tested |
| Idle unauthenticated TCP sockets | a goroutine each, no cap (by reading: grpc `server.go:980-991`) | 500 sockets, 499 more threads | 50 sockets, 6 more threads: 8 handshake slots (test setting), 42 refused at accept | 500 sockets, 0 more threads |
| A ClientHello trickled one byte per 300 ms | 120 s absolute deadline (grpc `server.go:987-991`) | still open after 10 s (review: after 5 min) | closed at the deadline (2.0 s with a 2 s test deadline) | closed at 2.0 s |

What the fixes are (all in `spikes/grpc-intree`, each with the regression
case named above, and an in-memory case in `tests/hostile.rs` where the
state machine is involved):

1. **Writes off the workers.** Only a per-connection writer thread writes to
   the socket; workers and the reader append output under the state lock and
   return. A batch the peer has not taken within `write_timeout` (default
   20 s, after grpc-go's TCP_USER_TIMEOUT of 20 s, grpc
   `http2_server.go:244-247`) closes the connection. SO_SNDTIMEO alone was
   not enough: it bounds one system call, and a socket whose send buffer the
   kernel keeps growing takes a few bytes per call, which the first fixed
   version's test exposed.
2. **Admission and the deadline.** A cap on connections (1,024) and on
   connections still in the handshake (128), taken at accept, a socket past
   either closed at once and counted; an absolute deadline from accept to the
   first SETTINGS, each read and write given only the time left (grpc-go's
   `connectionTimeout`); a failed thread spawn counted and the socket closed;
   accept errors back off.
3. **Request memory charged until the response.** A complete request stays
   charged to the connection's budget (64 MiB) until its handler's response
   is delivered, whether or not the stream was reset meanwhile.
4. **Flow control that cannot wedge.** The connection window is always
   returned, as grpc-go returns it; a stream's message is admitted against
   the budget when its 5-byte prefix arrives, a stream that does not fit
   stops getting its own window back and is admitted first come first
   served. Without admission a stream holds at most its 65,535-byte window,
   so request memory per connection is bounded by `max_buffered +
   max_streams x 65,535` (320 MiB at the defaults); grpc-go has no bound
   (one goroutine and up to 4 MiB per stream, unlimited streams).
5. **The stream cap advertised.** SETTINGS_MAX_CONCURRENT_STREAMS 4,096,
   so grpc-go's client queues calls past it; a stream past it is still
   refused for a client that ignores the setting. This is a SETTINGS
   difference from grpc-go, which advertises none.
6. **Header limits as grpc-go's.** Header blocks are decoded fragment by
   fragment against the 16 MiB header-list budget with x/net's rules: a
   fragment more than twice the remaining budget, or any fragment after an
   invalid field, closes the connection; a list over budget in a complete
   block is RST_STREAM FRAME_SIZE_ERROR (grpc `http2_server.go:384-393`).
   The 1 MiB and 128-frame caps, which refused valid input, are gone; a
   block is capped at 16,384 frames (one per KiB of budget) against an
   endless run of empty CONTINUATIONs.
7. **Cancellation is not rapid reset.** Handler work per connection is
   capped (64 calls, counting calls whose stream was reset); complete
   requests past it wait in the connection, never refused. Only a reset of
   a request still arriving is scored, with decay, so ordinary cancellation
   never closes the connection; what a rapid-reset flood can start is the
   in-flight cap.

Not fixed in the spike, because each is too large for it or a decision for
the owner, and listed with an estimate below: server streaming and the
bidirectional stream; keepalive enforcement (server-initiated PING and the
idle policy for authenticated connections, each of which costs two
threads); `grpc-timeout` timers; grpc-go's two-phase graceful shutdown;
window growth (grpc-go's BDP estimator); fairness of the shared worker pool
across connections; and the cost of refusing rather than queueing at the
handshake cap, which lets 128 trickling sockets keep new clients out for up
to the 120 s deadline (pinned in `unauthenticated_sockets_are_bounded`).

### TLS and provider findings

Both variants use the same configuration (`src/tls.rs`): rustls 0.23.46,
`dcroxide-tlscrypto` at the main workspace's pinned revision, key exchange
groups in Go's order (X25519, P-256, P-384, P-521), the RPC verification set
(`RPC_SIG_ALGS`), TLS 1.3 and 1.2, ALPN `h2` with a connection that
negotiated none closed after the handshake as grpc-go closes it, server-side
suite order, no stateful resumption and no tickets, and rustls's
`WebPkiClientVerifier` over the client CA file (dcroxide-node's
`GoClientCertVerifier` is private to the node).

| | Result |
|---|---|
| Server keys | Ed25519 (dcrwallet's default), P-256, P-384, P-521: served in TLS 1.3 and 1.2 to grpc-go (`server_key_types`). RSA-2048: refused when the configuration is built (`RSA private keys are not supported: this provider signs with ECDSA (P-256, P-384, P-521), Ed25519 and ML-DSA keys`, dcroxide ADR-0015 T6); dcrwallet never generates one but loads one a user supplies |
| Client certificates, CA-signed | Ed25519, P-256, P-384, RSA-2048: accepted over TLS 1.3 and 1.2. P-521: refused (`client_key_types`). Go signs every P-521 certificate with ECDSA-SHA512, which the provider does not verify yet (dcroxide ADR-0015 T4), and the CertificateRequest offers no scheme a P-521 key can sign with. Over TLS 1.3 the grpc-go client then sends no certificate and reports `tls: certificate required`; over TLS 1.2 it reports `tls: error decrypting message`; grpc-go's server accepts both |
| Client certificates, self-signed in `clients.pem` | without basic constraints: accepted for Ed25519, P-256, P-384 and RSA-2048 keys (webpki takes the leaf as its own anchor); P-521 refused as above, with the same texts; with CA basic constraints, the shape certgen produces: refused (webpki will not take a CA certificate as an end entity), where Go accepts. Brief section 8 already names this and the fix: dcroxide-node's `GoClientCertVerifier`, which ADR-0006 rule 3 schedules into a dcroxide library crate before Phase 7 |
| Key exchange | X25519 where Go's server picks X25519MLKEM768 for a Go client: the provider has no hybrid groups yet (dcroxide ADR-0015 T3) |
| Cipher suite | AES-256-GCM first (the provider's default order is ring's) where Go prefers AES-128-GCM on hardware with AES-GCM; dcroxide-node computes Go's order per ClientHello (`go_aes_gcm_preferred`, `rpctls.rs`), privately |
| Alerts | a client certificate from another dcrwallet-issued CA draws `decrypt_error` where Go sends `unknown_ca`: every issued CA has the same subject, so webpki finds the trusted CA by name and fails on the signature. An `http/1.1`-only ALPN offer draws `no_application_protocol` during the handshake, where Go completes it and grpc-go closes the connection |

None of these differ between the variants, and they apply to (B1) as well;
all are work for the full server either way.

The spike's own TLS glue had one real defect before the review, found by
the raw cases: a client's first frames that arrive with its Finished
message are decrypted by rustls during the handshake, and the in-tree read
loop waited on the socket before taking them (56 of 100 runs of one case,
before the fix). tokio-rustls has no such window.

### Lines of code (variant C)

Code lines, without comments, blank lines and unit tests, after `rustfmt`:

| Layer | Files | First draft | Now | Unit tests now |
|---|---|---|---|---|
| HTTP/2 framing and connection state machine (includes about 200 lines of grpc-go's request header checks) | `frame.rs`, `conn.rs` | 1,135 | 1,356 | (in `tests/hostile.rs`) |
| HPACK and Huffman | `hpack.rs`, `huffman.rs` | 415 | 480 | 242 |
| gRPC framing, statuses, router | `grpc.rs` | 228 | 228 | 62 |
| protobuf (reader, writer, two messages) | `proto.rs` | 179 | 179 | 37 |
| TLS configuration and threaded I/O | `tls.rs`, `server.rs` | 351 | 650 | — |
| Crate root, binary | `lib.rs`, `bin/` | 89 | 89 | — |
| **Total** | | **2,397** | **2,982** | 341 |

The fixes cost 585 code lines: 299 in the I/O layer, 221 in the state
machine (admission, the in-flight queue, fragment-wise header decoding,
reset scoring), 65 in HPACK. Tests, non-blank lines: `tests/hostile.rs`
734, `tests/interop.rs` 463, `tests/common/mod.rs` (shared with A) 899,
`tests/measure.rs` 126. Variant A is 352 code lines.

### What the full server needs beyond the spike

Estimates are code lines and engineer-weeks for one person who knows the
spike; "done" means built and covered by a regression case in the spike.

| Item | C (in-tree) | (B1) hyper/h2 + in-tree gRPC layer | (A) tonic |
|---|---|---|---|
| Per-connection writes and back-pressure that cannot stall other connections | done (writer thread, batch deadline, outbox limit); polish ~100, 0.5 wk | h2/hyper (shown by the slow-reader case) | same as B1 |
| Bounded accept path, handshake deadline before a thread or large allocation is committed | done, with a thread per handshake inside a cap; a handshake without a thread needs a readiness API std lacks (ADR-0008 option E); ~20 for a shorter TLS deadline if chosen | a task per connection, `tokio::time::timeout` (spike); a connection cap ~30 | same as B1 |
| Memory accounting covering dispatched requests; flow control that cannot deadlock | done for unary requests | h2's windows; a per-connection handler cap and request limit ~50 (A's spike has none: unlimited streams, as grpc-go) | same as B1 |
| Stream-concurrency limit advertised and enforced | done (4,096 advertised; owner question 4) | hyper option, one line, same SETTINGS difference | same |
| Header-list limits matching grpc-go's | done; send-side limit (peer's MAX_HEADER_LIST_SIZE on responses) ~50 | h2's, 16 MiB set; h2's own CONTINUATION and bomb policies differ on invalid input (36 h2 cases) | same |
| Rapid-reset accounting that tolerates ordinary cancellation | done | h2's (20 pending-accept resets, 1,024 local error resets: h2 `src/proto/mod.rs:34-35`); passed the cancellation case | same |
| Server streaming for 11 RPCs and the bidirectional stream: a send path blocking the handler within the 64 KiB per-stream quota, half-close, the client-stream receive path, a thread per stream with a limit, fair scheduling among streams | 1,000–1,500, 3–4 wk | a body stream fed from a blocking producer through a bounded channel, h2's `max_send_buf_size` at the quota; client stream bridged the same way: 300–500, 1–1.5 wk | tonic's streaming plus the same bridge: 200–400, 1 wk |
| Keepalive enforcement: a ping after 2 h idle with a 20 s timeout (grpc `defaults.go:38-39`), the idle policy for authenticated connections; the ping-strike policy | strikes done; server pings and idle 150–250, 1 wk (needs a timer thread) | hyper options (set); the ping-strike policy is not available in h2 (PARITY entry) | same as B1 |
| Deadlines and cancellation: `grpc-timeout` timers resetting the stream, cancellation reaching handlers' `Ctx` | flag done; timers 150–250, 1 wk | timer and drop guard (spike's `blocking`): 100–150, 0.5 wk | same, inside a tower layer |
| Graceful shutdown: grpc-go's two-phase GOAWAY with a PING, draining | 150–200, 0.5 wk | hyper's (one GOAWAY; PARITY entry) | same |
| Window growth (grpc-go's BDP estimator, grpc `http2_server.go:302`) | 150–250, 1 wk | hyper's adaptive window, one option (owner question 6) | same |
| Fairness of handler threads across connections | 150–250, 0.5–1 wk | the blocking pool (512 threads by default) behind a per-connection cap ~30 | same |
| Metadata details (`-bin` values), interceptors (logging, `ServiceReady`'s FailedPrecondition) | ~200, 0.5 wk | ~200, 0.5 wk | tower layers, ~200, 0.5 wk |
| grpc-go parity at the gRPC layer | done for the request path | the spike's layer moved onto hyper's request and body types: 500–700, mostly moved, 1–1.5 wk | a tower layer and body wrappers in front of tonic: 400–600, 1.5 wk, re-checked on every tonic release |
| grpc-go parity at the HTTP/2 layer | done where wanted | not reachable (36 cases, 35 on invalid input; PARITY entries) | same |
| Codec for the remaining ~220 messages (tag order, empty sub-messages, proto3's lenient decoding; spec 19 section 4.3) | in-tree generator for `api.proto`'s subset, 1,000–1,500 plus checked-in output, 2–3 wk (or prost, B2) | same as C | prost; `prost-build` needs `protoc`, so generated code is checked in or `protox` added; prost-derive's field order (`BlockDetails` field 4 declared after 5) unverified |
| Fuzz targets and I/O stress in CI | frame parser, fragment-wise HPACK, state machine, codec; the I/O cases above as a CI job: 400–600 test lines, 1–1.5 wk | codec and gRPC layer; I/O cases kept: 200–300, 0.5 wk | same as B1 |
| **Transport total** (excluding the codec) | **+2,100–3,100 code lines, ending near 5,100–6,100; 8–11 wk** | **+1,100–1,600 lines; 4–5 wk** | **+1,000–1,500 lines; 4–5 wk, plus the codec's protoc question** |

Ongoing cost. C: the project owns every HTTP/2 and connection-I/O defect
class, current and future, in a process that holds keys, and finds and fixes
each one itself. The record of the past few years is the scale: h2 alone
shipped fixes for reset floods (0.3.17, CVE-2023-26964; 0.4.2, "limit error
resets"), the CONTINUATION flood (0.4.4, April 2024) and a dozen reset and
flow-control corner cases since (h2 `CHANGELOG.md`). The spike is evidence of
the rate: nine of the twelve findings one review confirmed were defects in
the first version's server (seven distinct), four of them failures with the
unmodified grpc-go client on well-formed input (20,000 concurrent calls, a
large header, ordinary cancellation, 80 MB in flight), the first three
answered with Unavailable and the last as a wedged connection. The parser sits behind
mutual TLS on a localhost listener by default, so only an authenticated
client reaches it, and C's code is `#![forbid(unsafe_code)]`; tokio, mio,
hyper and h2 contain `unsafe`. (B1) and (A): every hyper, h2 and tokio
release is a dependency change to review and re-run the matrix against, and
h2's policies (its flood limits, error mapping, GOAWAY texts) are chosen
elsewhere.

### Dependency footprint

Counted with `cargo tree -e normal,build` (host target unless stated) and
checked against `deny.toml`'s rules by a script over `cargo metadata`, since
`cargo-deny` is not installed on the machine the spike ran on and was not
run. (B1)'s graph is a manifest with hyper, hyper-util, http,
http-body-util, bytes, tokio, tokio-rustls, rustls and
`dcroxide-tlscrypto` as variant A configures them, resolved against A's
lockfile; it was not built.

| | C | (B1) | (A) |
|---|---|---|---|
| Crates built (normal and build edges, proc macros included) | 73 (75 with every target) | 101 (106) | 118 (123) |
| Proc macros | 1 (`curve25519-dalek-derive`, via the provider) | 1 | 5: also `async-trait`, `pin-project-internal`, `prost-derive`, `tracing-attributes` |
| New against dcroxide's lockfile at the pin | none | 20: tokio, mio, hyper, hyper-util, h2, http-body, http-body-util, httpdate, tokio-rustls, tokio-util, tower-service, tracing, tracing-core, futures-core, -sink, -channel, atomic-waker, pin-project-lite, slab, smallvec | 34: B1's 20 and tonic, tonic-prost, prost, prost-derive, tokio-stream, tower-layer, async-trait, pin-project, pin-project-internal, sync_wrapper, itertools, either, anyhow, tracing-attributes |
| Duplicates inside the graph | none | none | `syn` 2 and 3 (the main workspace already skips `syn:2`) |
| Against the main workspace's and dcroxide's lockfiles | patch-level drift only (`rustls-webpki` 0.103.17 against dcroxide's 0.103.16 and five others), which one lockfile unifies | the same | the same, plus `base64` 0.22.1 (tonic's) against dcroxide's 0.23.1: a duplicate `multiple-versions = "deny"` refuses, needing a `bans.skip` mirrored or justified under ADR-0006 rule 5 |
| `ring`, `aws-lc-*`, `cc` and the other banned crates built | none | none | none (`ring` and `cc` appear in every lockfile as optional dependencies of `rustls-webpki` that nothing enables, as in dcroxide's) |
| Licenses | all within `deny.toml`'s allow list | all within it | all within it (adds MIT-only and Apache-2.0-only crates) |

### Binary size and build time

Release profile of the main workspace (one codegen unit, thin LTO, line
tables, abort on panic). Sizes re-measured after the fixes; build times are
the first draft's (each server built from cold in its own target directory)
and were not re-measured.

| | C | A |
|---|---|---|
| Binary | 11.9 MB (2.35 MB stripped) | 15.1 MB (3.17 MB stripped) |
| Clean release build | 13.3 s wall, 42.6 s CPU | 12.7 s wall, 64.5 s CPU |
| Clean debug build | 6.2 s wall, 22.3 s CPU | 7.0 s wall, 37.5 s CPU |

### Speed

Unary `Version` calls on one connection from the grpc-go client (5,000,
40,000 and 80,000 calls), after a warm-up, release builds, TLS 1.3 with an
Ed25519 key (`tests/measure.rs` in both spikes). Three runs of each spike,
alternating; each run measures grpc-go beside the spike, so grpc-go's
figures are medians of six runs, the spikes' of three. The machine was a
shared 32-thread x86_64 Linux desktop with a load average of 15 to 20 from
other work during the runs, so the spread between runs is wide (the p99 of
a single run moved by up to 10x); the figures compare the servers with each
other, not with any target.

| Concurrency | Server | p50 µs | p99 µs | calls/s | first draft, calls/s |
|---|---|---|---|---|---|
| 1 | grpc-go | 41 | 137 | 19,900 | 21,600 |
| 1 | C | 40 | 104 | 21,600 | 23,100 |
| 1 | A | 45 | 117 | 20,100 | 18,400 |
| 16 | grpc-go | 161 | 1,265 | 76,800 | 93,100 |
| 16 | C | 120 | 378 | 118,800 | 64,600 |
| 16 | A | 208 | 2,023 | 51,100 | 65,600 |
| 256 | grpc-go | 1,828 | 3,016 | 139,300 | 132,500 |
| 256 | C | 1,433 | 2,918 | 170,200 | 84,600 |
| 256 | A | 3,491 | 13,536 | 66,500 | 70,000 |

C's throughput at 16 and 256 roughly doubled from the first draft: the
writer thread takes everything queued and writes it as one batch, which is
the write coalescing the first draft listed as future work. At one call in
flight the three are within 5 µs of each other at the median; A's extra
microseconds are the hop to the blocking pool per call.

Large requests, all at once on one connection (`bigcalls`; median of five
per run, then of the runs):

| | grpc-go | C | A | A with hyper's adaptive window (one run) |
|---|---|---|---|---|
| 20 x 4,000,000 bytes | 54 ms (1.48 GB/s) | 105 ms (0.76 GB/s) | 119 ms (0.67 GB/s) | 79 ms (1.01 GB/s) |
| 1,000 x 100,000 bytes | 81 ms (1.24 GB/s) | 159 ms (0.63 GB/s) | 154 ms (0.65 GB/s) | 112 ms (0.89 GB/s) |

Both Rust servers keep 65,535-byte windows and so reach about half of
grpc-go's upload throughput, whose BDP estimator grows its windows; hyper's
adaptive window, one builder option, closes most of the gap for A and would
for (B1); C needs the BDP item above.

A new connection with a TLS 1.3 handshake and one call, including the
oracle's own round trip, p50: grpc-go 962 µs, C 947 µs, A 807 µs. Process
after load: C 67 to 69 threads (64 workers on 32 hardware threads, the accept
thread, a reader and a writer per connection) and about 9 MB resident; A 84
to 156 threads (32 runtime workers plus the blocking pool) and 21 to 23 MB.

Not measured: streaming throughput and latency in any variant (no
streaming RPC was built); (B1); any machine but this one.

### Robustness

The state machine bounds what a peer can make it hold: one frame of input;
during a header block, the fields kept (the 16 MiB header-list budget, as
grpc-go's) and one cut representation (at most one name or value, bounded by
the same budget); admitted request bytes, charged until the response is
delivered (64 MiB), plus one 65,535-byte window per stream not admitted;
4,096 open streams, advertised; 64 handler calls per connection, counting
reset ones; and resets of incomplete requests scored with decay (1,000, one
point decaying per 10 ms and per delivered response). Output is bounded by
input and by the in-flight cap, and the I/O layer stops reading a
connection while 1 MiB of its output waits.

`tests/hostile.rs` checks each class below in memory with the error it must
end in, drives flow control with a fair round-robin sender that respects
windows (grpc-go's loopy writer's order) and asserts every call finishes,
and runs a mutation loop over valid sessions (bit flips, rewritten lengths,
types, flags and stream ids, truncation, splices, repeats; random chunking;
answers of up to 70 KB) asserting no panic and the memory bound: 4,000,000
sessions over eight seeds ran clean after the fixes. It drives the state
machine only; the I/O layer is covered by the I/O-level cases above, which
are not fuzzed. Through TLS, a panic hook counts panics on every server
thread across the sixty raw cases, and the server must answer a call after
each case: zero panics.

`held_bytes` measures what a connection retains between reads, not the peak
inside one read; the peak while an HPACK bomb is decoded is bounded by the
same 16 MiB header-list budget, as in grpc-go, and was not measured with a
counting allocator.

| Attack class | grpc-go as dcrwallet runs it | C (spike, fixed) | A (h2) | The full server must |
|---|---|---|---|---|
| Rapid reset (CVE-2023-44487) | 2,000 opened-and-reset streams accepted; its mitigation bounds handler goroutines only when MaxConcurrentStreams is set (grpc `server.go:1071-1089`), which dcrwallet does not set | GOAWAY ENHANCE_YOUR_CALM once resets of incomplete requests outrun decay by 1,000; handler work capped at 64 per connection; cancellation of running calls never scored | GOAWAY ENHANCE_YOUR_CALM `too_many_resets` after 20 pending-accept resets; ordinary cancellation passed | keep a handler cap per connection whatever the transport |
| CONTINUATION flood (2024, VU#421644) | closes at the header-list limit; 10,000 empty frames accepted | the same rules as grpc-go (x/net's), plus a 16,384-frame cap per block | GOAWAY `too_many_continuations` at its limit | keep x/net's rules |
| SETTINGS flood | acknowledges all, reader throttled while responses queue (grpc `controlbuf.go:320`) | acknowledges all, 9 bytes out per 9 in, reader stops while 1 MiB of output waits | acknowledges all | keep output bounded |
| PING flood | GOAWAY `too_many_pings` on the third strike | the same policy, byte for byte | acknowledges every ping | keep grpc-go's policy (A and B1 cannot) |
| HPACK bomb | closes (a non-empty fragment after the budget is spent) | the same, with a GOAWAY; a complete block over budget is RST_STREAM FRAME_SIZE_ERROR, as grpc-go's | GOAWAY `header_list_way_too_large` | keep the budget |
| Stream id violations, oversized frames, HPACK and Huffman errors | close (GOAWAY with text for stream ids) | GOAWAY with the RFC's code | GOAWAY, mostly PROTOCOL_ERROR | decide GOAWAY-or-silence per case (C only) |
| Request memory across streams | unbounded by reading (4 MiB per unary stream, streams unlimited) | 64 MiB admitted plus 65,535 per unadmitted stream, charged until the response; cannot wedge | h2's windows; streams unlimited as configured | choose a stream cap and a handler cap (owner question 4) |
| A client that never reads | its connection stays open for at least 3 s; other clients served | its connection closed at the write deadline; other clients served | its connection closed within 3 s; other clients served | a write deadline per connection |
| Unauthenticated sockets, slow handshakes | a goroutine each, no cap, 120 s absolute deadline | a thread each within a cap of 128, refused past it; 120 s absolute deadline | a task each, no cap; the handshake deadline | a cap and a deadline; in C, a decision on the handshake timeout (owner question 5) |
| Idle authenticated connections | keepalive after 2 h idle, 20 s timeout | none yet (two threads each) | keepalive set | keepalive |

Pre-authentication, the reachable surface is the accept path and TLS
(rustls on `dcroxide-tlscrypto`, the same in every option): a thread per
connection in C within its handshake cap, a task in A and (B1). The HTTP/2
parser is reached only by a client holding a certificate `clients.pem`
accepts, and dcrwallet's listeners default to localhost.

### Threads and the rest of the wallet (D6)

In C a handler is a plain function on a worker thread calling the wallet's
blocking API directly; a streaming handler would block on its send within
the per-stream quota and hold a thread for as long as the stream lives.
Cancellation is a flag the handler polls (`CallContext::is_cancelled`, used
by the cancellation case). Per connection: a reader and a writer thread,
plus the shared worker pool. One concurrency model in the process.

In A and (B1) every call crosses from a runtime task to a blocking thread:
`blocking` in the tonic spike runs the wallet call on the runtime's blocking
pool with a flag a drop guard sets when the call's future is dropped, which
is ADR-0008's "guard per handler future"; the cancellation case shows it
freeing 1,500 cancelled calls' threads. Without the guard, a cancelled
call's blocking work runs to completion while the pool (512 threads by
default) fills. A streaming handler bridges a blocking source into a bounded
channel feeding the response body. Two models of waiting meet at every
handler, and the boundary is crossed by all 88 RPCs; behind it, ADR-0008's
rules hold unchanged.

## Comparison

| | C (in-tree) | (B1) hyper/h2 + in-tree gRPC layer | (A) tonic |
|---|---|---|---|
| What a conforming client can observe, against grpc-go | the TLS findings; the advertised stream cap | the TLS findings; the stream cap if advertised; h2's PING leniency | as B1, plus 19 gRPC-layer differences until a parity layer removes them |
| Invalid input | grpc-go's texts, or deliberate GOAWAY and strictness, each switchable | h2's choices (35 cases), fixed | as B1 |
| I/O-level cases above | all pass after 585 lines of fixes; the first version failed five of the seven run and bounded neither of the other two | as A (not run) | all run pass with the transport unmodified (shutdown not run) |
| Remaining transport work | 8–11 weeks, the hardest part (streams under back-pressure) not yet built | 4–5 weeks | 4–5 weeks plus the protoc question |
| Ongoing | owns HTTP/2 and connection I/O security response | dependency releases; h2's policies | as B1, plus tonic's releases against the parity layer |
| Unary throughput at 16 / 256 calls in flight | 118,800 / 170,200 calls/s, above grpc-go | not measured | 51,100 / 66,500 calls/s |
| Unary latency at 1 call | 40 µs p50 | not measured | 45 µs p50 |
| Large uploads | 0.6–0.8 GB/s until a BDP estimator is written | hyper's adaptive window (A measured 0.9–1.0 GB/s) | as B1 |
| Crates (host), new against dcroxide | 73, none | 101, 20 | 118, 34, `base64` duplicate |
| Runtime in the process | none | tokio in the listener (ADR-0008's seam) | same |
| `unsafe` in the transport | none (`forbid`) | in tokio, mio, hyper, h2 | same plus tonic's dependencies |

## Proposal (not adopted: the owner chose option (A), see below)

**(B1): the gRPC listener runs hyper's HTTP/2 server (h2) on a tokio
runtime confined to the listener, as ADR-0008 permits; the gRPC layer and
the protobuf codec are written in tree, starting from the spike's
`src/grpc.rs` and request checks; everything behind the listener stays on
threads and the wallet's blocking API. Not tonic.** For the codec, an
in-tree generator over `api.proto`'s subset with its output checked in, as
before. The in-tree HTTP/2 spike is kept, with its tests, as the record and
the fallback.

The reasons, from the evidence:

1. **The defects were where h2 is mature.** Nine of the twelve confirmed
   findings were defects in the in-tree server (seven distinct), all in its
   I/O, concurrency, flow-control and header-limit code; four broke the
   unmodified grpc-go client on well-formed input. Variant A passed every
   one of those cases with its transport unmodified.
   They are fixed in the spike, but the parts not yet built (long-lived
   streams under back-pressure, keepalive, deadlines, window growth,
   graceful shutdown, fairness) are the same kind of code, and
   byte-compatible RPC results are one of the things speed may not
   override.
2. **Client-visible parity is no reason to own HTTP/2.** With the gRPC layer
   in tree, (B1) would produce grpc-go's statuses, texts, headers and
   trailers for everything a conforming client sends, as C does (by
   construction: the layer is C's; not built); h2's 36 differences
   are 35 cases of invalid input and the PING-strike leniency. tonic, by
   contrast, would add 19 gRPC-layer differences to undo and the `base64`
   duplicate, for little a hand-written service layer does not already do.
3. **Cost.** 4–5 weeks of transport work against 8–11 for C, and no HTTP/2
   security response to carry alone.
4. **The policy allows it.** ADR-0008 admits a runtime at exactly this seam;
   the bridge it requires (a drop guard cancelling `Ctx`, blocking threads
   behind) is demonstrated.
5. **Speed.** With one call in flight the variants are within 5 µs at the
   median (a GUI's load is expected to be a handful of calls; Decrediton's
   was not measured); uploads favour hyper's adaptive
   window until C has a BDP estimator. At 16 and more concurrent unary
   calls C is twice tonic's throughput; (B1) was not measured.

**The strongest argument against.** Speed and footprint. C is the fastest
server measured here, above grpc-go at 16 and 256 calls in flight and more
than twice tonic, with 73 crates, no runtime, one concurrency model and no
`unsafe` in the transport; (B1) adds 20 crates, tokio's scheduler and a
thread hop per call, and its own throughput is unmeasured, so the
recommendation may give up a measured factor of two at high concurrency for
an estimate. It also hands h2's policies to someone else for good: the
PING-strike policy cannot be reproduced, h2's flood limits and error
mapping become permanent PARITY entries, and every hyper and h2 release is
a change to review in a process that holds keys. The answer this record
gives is that the measured factor appears only above a handful of calls in
flight, which a wallet's GUI does not generate (an expectation, not
measured against Decrediton), that (B1)'s own throughput is a week's
measurement away (question 7), and that the cost C pays for its speed is
the kind of defect the review found; the owner may weigh speed and a single
model higher.

### Questions for the owner

1. **Admit tokio at the gRPC listener (ADR-0008's seam) for hyper/h2?**
   Recommended: yes. If no: (C), with the work and the ongoing cost above.
2. **If yes, hyper/h2 with an in-tree gRPC layer (B1) rather than tonic
   (A)?** Recommended: (B1); tonic brings a `base64` duplicate needing a
   `bans.skip` under ADR-0006 rule 5, 14 more crates and 19 differences to
   undo.
3. **Accept h2's behaviour on invalid input and its lack of a PING-strike
   policy as PARITY entries** (and, under ADR-0011, ET-003's transport
   statuses as h2 produces them for those cases)? Recommended: yes; none is
   reachable by a conforming client but the PING leniency.
4. **Advertise SETTINGS_MAX_CONCURRENT_STREAMS and cap handler calls per
   connection**, a SETTINGS difference from grpc-go, which advertises
   nothing as dcrwallet configures it? Recommended: yes, in any option;
   without both, memory and handler work per connection are unbounded, in
   grpc-go too.
5. **Keep grpc-go's 120 s connection timeout for the handshake?**
   Recommended: yes under (B1), where a waiting handshake is a task; under
   (C) the handshake cap makes 128 trickling sockets a 120 s lock-out for
   new clients, and a shorter deadline (10–30 s, a recorded difference)
   would be the choice.
6. **Turn on hyper's adaptive window**, which approximates grpc-go's BDP
   estimator with different WINDOW_UPDATE and PING bytes? Recommended: yes
   (A measured 1.01 against 0.67 GB/s on large uploads).
7. **Before Phase 12, build (B1) as a spike**: the gRPC layer moved onto
   hyper, the full matrix and the measurements re-run? Recommended: yes,
   about a week; if it fails a case class above or its unary throughput at
   one to sixteen calls in flight is materially below C's, this record is
   revisited before Phase 12 starts.

## Decision by the project owner (2026-11-19)

Asked why not tonic, and given the answer below, the owner chose it: "ok go
with tonic and your recommendations for the rest". What was put to the
owner: tonic is the least work, brings streaming and code generation, and
is the widely used stack; against it, it was the slowest server measured at
16 and more calls in flight, differs from grpc-go on 55 of the 60 raw
cases (36 inside h2, which (B1) shares, and 19 in its own gRPC layer),
brings 34 crates new to dcroxide's lockfile and a `base64` duplicate; and
it is not known whether Decrediton can observe any of the 19.

**(A): the gRPC listener is tonic and prost over hyper's HTTP/2 server
(h2) on a tokio runtime confined to the listener, as ADR-0008 permits;
everything behind the listener stays on threads and the wallet's blocking
API.** `spikes/grpc-tonic` is the starting point: tonic's own transport and
its TLS features are not used, TLS is rustls on `dcroxide-tlscrypto`
through tokio-rustls, and a handler reaches the wallet through the
blocking bridge with a guard that cancels its `Ctx` on drop. The in-tree
spike is kept, with its tests, as the record.

The questions:

1. *Admit tokio at the listener.* Yes.
2. *(B1) rather than tonic.* No: tonic.
3. *h2's behaviour on invalid input and its lack of a PING-strike policy
   as PARITY entries.* Yes.
4. *Advertise a stream cap and cap handler calls per connection.* Yes.
5. *grpc-go's 120 s connection timeout for the handshake.* Kept.
6. *hyper's adaptive window.* On.
7. *A (B1) spike before Phase 12.* Not built; the question fell away with
   question 2.

What follows, in place of "If the owner accepts (B1)" under Consequences:

- Phase 12 builds `purse-grpc` on tonic and prost, the runtime confined to
  its transport module; streaming bodies are fed through bounded channels
  within ADR-0008's per-stream quota; a connection cap, a stream cap and a
  handler cap per connection; the I/O-level cases and the raw cases above
  are the differential suite, with the oracle's grpc-go client and
  reference server as the harness.
- tonic's 19 gRPC-layer differences from grpc-go are each either undone (in
  a layer around the tonic service) or recorded in PARITY.md. Which of them
  a conforming client, Decrediton above all, can observe is not known and
  is established first, with Decrediton's own call inventory (see "What
  remains open").
- ADR-0008 is amended as its "ADR-0004 and D11" section prescribes, for
  tonic at this seam only. D11 is decided inside the streaming bridge.
- `deny.toml` gains a `bans.skip` for the `base64` duplicate when tonic
  enters the main workspace (ADR-0006 rule 5); the 34 crates are reviewed
  as a dependency addition.
- How the 222 messages and 88 methods are generated without `protoc` in the
  build (the spike wrote two messages by hand) is settled at the start of
  Phase 12; proposed: generated code checked in, regenerated by a CI job.
- **Speed is the accepted risk.** At one call in flight tonic matched
  grpc-go; at 16 it measured 51.1k calls per second against grpc-go's
  76.8k, on a loaded machine, unary only. ADR-0014's Phase 12 criterion
  (no slower than Go at 1 and 8 clients) is measured as soon as the
  listener exists; if tonic misses it, the hyper options, the bridge and
  the runtime's configuration are tuned first, and this record is
  revisited if that is not enough.
- PARITY.md records h2's differences on invalid input, the PING-strike
  leniency, hyper's one-phase graceful shutdown, the advertised stream cap
  and the adaptive window.

## Consequences

If the owner accepts (B1):

- Phase 12 builds `purse-grpc` as a listener module on hyper's HTTP/2
  server and tokio-rustls, the runtime confined to it; the gRPC layer from
  the spike's `src/grpc.rs` and request checks on hyper's request and body
  types; the codec generator; the bridge with a drop guard per call
  cancelling `Ctx`; streaming bodies fed through bounded channels within
  ADR-0008's per-stream quota; a connection cap, a stream cap and a handler
  cap per connection; the I/O-level cases above and the raw cases as the
  differential suite, with the oracle's grpc-go client and reference server
  (`tools/oracle/cmd/grpcclient`) as the harness.
- ADR-0008 is amended as its "ADR-0004 and D11" section prescribes: its
  option C admitted at this seam only, provided `deny.toml` passes, tokio
  stays in `purse-grpc`'s transport module, wallet calls run on std threads,
  a guard per handler future cancels its `Ctx` on drop, and sends follow the
  quota rule. D11 is decided inside the streaming bridge.
- `deny.toml` gains no skip for (B1) (no duplicate against dcroxide's lock
  at the pin); the 20 crates are reviewed as a dependency addition.
- PARITY.md records h2's differences on invalid input, the PING-strike
  leniency, hyper's one-phase graceful shutdown, the advertised stream cap
  and, if chosen, the adaptive window.

If the owner chooses (C) instead: Phase 12 builds `purse-grpc` from the
fixed spike (the I/O-free state machine, the reader-and-writer listener with
admission and deadlines) and the items of the table above, with fuzz targets
for the frame parser, fragment-wise HPACK, the state machine and every
protobuf decoder in `fuzz/`, and the I/O-level cases as a CI job; ADR-0008
stands without its exception.

Either way:

- dcroxide-node's `GoClientCertVerifier`, Go's per-ClientHello suite order
  and Go's PEM and key reading are extracted into a dcroxide library crate
  first and consumed at the pinned revision (ADR-0006 rule 3, before Phase
  7); dcroxide ADR-0015's T3 (hybrid key exchange) and T4 (P-521 and
  SHA-512 ECDSA client certificates) land before Decrediton acceptance; the
  RSA `rpc.key` refusal is recorded in PARITY.md.
- QUIRKS.md gains grpc-go's unanswered HEADERS-only request, decided under
  ADR-0012 Rule 1 (it repeats and shows on the wire, but only for a request
  no client library sends); PARITY.md gains the `detrand` separator in
  protobuf-go's error texts.

## Corrections to the first draft

- The robustness section said a peer that stops reading "stops the reader";
  it stopped every worker answering that peer, and with them every other
  client's calls. Fixed (writer thread, write deadline).
- "120 s to the first SETTINGS" was a per-read idle timeout that a trickle
  extended indefinitely, and every TCP connection got a thread, uncapped,
  before TLS. Fixed (admission, absolute deadline). "All of this surface
  sits behind mutual TLS" held for the parser, not for the per-connection
  cost.
- "64 MiB per connection, then the connection window stops" did not count
  requests handed to handlers, and the same rule wedged the connection once
  64 MiB of partial messages were in flight. Fixed (admission per stream,
  charge until the response).
- The 4,096-stream cap was not advertised, so grpc-go's client failed calls
  past it with Unavailable. Fixed (advertised).
- The 1 MiB and 128-frame header-block caps were presented as flood
  defences; they refused requests grpc-go accepts. Fixed (x/net's rules).
- Client cancellations of running calls counted as rapid-reset strikes.
  Fixed.
- "Nothing past the limit materialized ... held under 64 KiB" for the HPACK
  bomb described retained memory, not the peak; fields are kept up to the
  16 MiB budget, as in grpc-go, and the connection now closes as grpc-go's
  does.
- P-521 was listed as refused only when CA-signed; self-signed P-521
  certificates are refused too, and the failure text depends on the TLS
  version.
- TLS failure texts were shown as deterministic; they vary between runs on
  both sides, and the tests assert only the code. "=" in the client table
  claimed trailers were compared for rows where only the code is.
- The estimate (4,000–4,500 lines, 4–6 weeks) left out the I/O and
  concurrency work; it now reads 5,100–6,100 lines and 8–11 weeks for C.
- Statements the decisions of 2026-11-18 and 2026-11-19 overtook: (A) "brings an async runtime
  into the process, which dcroxide ADR-0005 decided against" now reads as
  ADR-0008's permitted seam; "ADR-0008 (D6, proposed) assumes ... no runtime
  in the process" now reads as the accepted ADR-0008 with its listener
  exception; "dcroxide ADR-0005's exception for the listener is recorded"
  now reads as ADR-0008's own amendment rule; "Performance does not decide
  it" is replaced by the owner's performance goal and the Speed section;
  "dcrwallet's go.mod declares go 1.26.0, as the oracle module does, so the
  reference runs with dcrwallet's GODEBUG defaults" was not so (dcrwallet's
  daemon builds with go1.25's defaults through `debug.go`), and now holds
  through `godebug default=go1.25` in `tools/oracle/go.mod`; the verifier
  extraction "into a library crate" now names dcroxide as its home under
  ADR-0006.

## What remains open

- **Decrediton was not available.** Its gRPC client library, its channel
  options (keepalive, message sizes), whether it uses `--issueclientcert` or
  a self-signed certificate of its own and with which key type, and its call
  inventory were not exercised; the brief's Phase 0 item to read its
  launcher stands. The PING-strike leniency and the CA-flagged self-signed
  certificate case matter only as far as Decrediton's configuration reaches
  them.
- **(B1) was not built**; its parity and speed are inferred from its two
  halves (owner question 7).
- **Streaming RPCs** were not built in any variant; flow control under
  back-pressure with long-lived streams, half-close and streaming
  throughput are estimated, not measured.
- **Windows and macOS** were not run; Linux x86_64 only.
- Keepalive timers were not run in real time, nor grpc-go's client-side
  keepalive options, nor deadlines expiring mid-call on the server side.
- The tonic server closed the never-reading client's connection within
  3 s; the cause (an h2 or hyper limit) was not identified.
- `cargo-deny` was not run on any graph; its rules were checked by a script
  over `cargo metadata`.
- The measurements come from one shared, loaded machine; build times were
  not re-measured after the fixes.

## Addendum, 2026-11-19 — ADR-0015 (D11)

On 2026-11-19 the project owner accepted ADR-0015, notification delivery
by a byte-counted queue per client. This record says "D11 is decided
inside the streaming bridge" (under "Decision by the project owner" and
under Consequences). Under ADR-0015 D11's mechanism sits ahead of the
bridge instead: the queue per client, the 8 MiB cap per client, the
256 MiB budget for all notification streams and the removal of an
overflowed client live in `purse-wallet`'s notification server, because
the queue is filled and the client removed under the server's mutex. The
bridge stays ADR-0008's plain sender within the 64 KiB per-stream quota,
and it reports when quota comes back, which the budget needs. The handler
of an overflowed `TransactionNotifications`, `AccountNotifications` or
`ConfirmationNotifications` stream ends it with `ResourceExhausted` and a
message of dcr-purse's own until Decrediton's handling is read (ADR-0015
Open point 2). The sync streams `RpcSync` and `SpvSync` send through the
bridge with Go's back-pressure, as before. Nothing else in this record
changes.
