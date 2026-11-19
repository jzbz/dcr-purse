# grpc-intree: D5 spike, variant C

A minimal gRPC server written in tree, the evidence for decision D5
([ADR-0004](../../docs/adr/0004-grpc-transport.md)): it answers
`walletrpc.VersionService/Version` (gRPC API 10.0.0) to an unmodified grpc-go
v1.83.2 client over mutual TLS.

| Module | Layer |
|---|---|
| `src/frame.rs`, `src/conn.rs` | HTTP/2: preface, SETTINGS, HEADERS/CONTINUATION decoded fragment by fragment with x/net's limits, DATA with per-stream admission against a request budget, WINDOW_UPDATE flow control, PING with grpc-go's keepalive enforcement, GOAWAY, RST_STREAM; a per-connection in-flight limit; a state machine with no I/O, bounded against hostile input |
| `src/hpack.rs`, `src/huffman.rs` | HPACK: static and dynamic tables, Huffman decoding, a decoder fed one fragment at a time, a stateless encoder |
| `src/grpc.rs` | gRPC: length-prefixed messages, grpc-go's request checks and status texts, trailers, the router |
| `src/proto.rs` | protobuf: the wire reader and writer and the two messages |
| `src/tls.rs`, `src/server.rs` | rustls 0.23 on `dcroxide-tlscrypto` (`builder_with_provider`); connection and handshake caps at accept, an absolute deadline to the first SETTINGS, a reader and a writer thread per connection with blocking I/O (only the writer touches the socket, with a write deadline), a worker pool for handlers |

It is a standalone workspace (its own `Cargo.toml` with an empty
`[workspace]` table and its own `Cargo.lock`); the main workspace excludes
`spikes/`. Nothing here is the wallet's gRPC server.

## Running

The tests drive the grpc-go oracle `tools/oracle/cmd/grpcclient` through
`purse-testutil`, which builds it with the Go toolchain on the path (Go 1.26
or later; the reference Go, 1.27.4, under `PURSE_REQUIRE_ORACLE`). The oracle
generates the certificates into the test's temporary directory and runs a
reference grpc-go server configured as dcrwallet configures its own; every
case runs against both servers.

```sh
cargo test                                   # skips the oracle cases without Go
PURSE_REQUIRE_ORACLE=1 cargo test            # as CI would: the oracle must run
PURSE_REQUIRE_ORACLE=1 cargo test --test interop -- --nocapture --test-threads 1
                                             # prints the comparison tables
PURSE_REQUIRE_ORACLE=1 cargo test --release --test measure -- --ignored --nocapture
                                             # latency and throughput at 1, 16, 256
PURSE_TEST_SEED=7 HOSTILE_ITERATIONS=500000 cargo test --release --test hostile
                                             # a longer mutation loop
```

`tests/interop.rs` holds the grpc-go client cases (positive and negative),
the raw HTTP/2 cases (well-formed, malformed and hostile frames sent through
the oracle, with parity asserted on the cases where grpc-go and this server
must agree), the server and client key types, bursts of concurrent streams
and 20,000 concurrent calls, and the I/O-level cases: large concurrent
requests past the request budget, a 3,000,000-byte metadata value, 1,500
calls cancelled while their handlers run, a client that never reads (with
other clients timed meanwhile), shutdown with a stuck peer, idle
unauthenticated sockets and a trickled handshake. `tests/hostile.rs` checks
each HTTP/2 attack class in memory, flow control against a fair round-robin
sender, the in-flight and request bounds, and runs a mutation loop over
valid sessions; it drives the state machine only, not `server.rs`, which
the I/O-level interop cases cover. `tests/common/mod.rs` is the harness;
the tonic spike includes the same file by path.

To run the server by hand, with certificates made by the oracle's `pki`
command (or any `rpc.cert`/`rpc.key` and `clients.pem` written outside the
repository):

```sh
cargo run --release --bin grpc-intree-server -- \
    --cert DIR/rpc.cert --key DIR/rpc.key --clientca DIR/clients.pem
```

It listens on 127.0.0.1 with an OS-assigned port unless `--listen` says
otherwise, and prints the address.
