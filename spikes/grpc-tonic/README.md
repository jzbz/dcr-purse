# grpc-tonic: D5 spike, variant A

The same server as [grpc-intree](../grpc-intree/README.md), built on tonic
0.14 and prost 0.14 over hyper 1, h2 0.4 and tokio 1, the evidence for the
other side of decision D5
([ADR-0004](../../docs/adr/0004-grpc-transport.md)). It answers
`walletrpc.VersionService/Version` (gRPC API 10.0.0) to an unmodified grpc-go
v1.83.2 client over mutual TLS.

- The tokio runtime is confined to the listener: `server::serve` builds a
  multi-threaded runtime of its own and nothing else in the process uses it.
- TLS is the in-tree variant's configuration (`src/tls.rs`, rustls 0.23 on
  `dcroxide-tlscrypto` through `builder_with_provider`) under tokio-rustls,
  every crate without default features: no ring, no aws-lc.
- tonic's own transport is not used, so axum and tonic's TLS features stay
  out; each connection is served by hyper's HTTP/2 server with the tonic
  service, whose generated shape is written by hand in `src/lib.rs` so the
  build needs no `protoc`. hyper's HTTP/2 options are set toward grpc-go's
  defaults (`src/server.rs`).
- The handler reaches the wallet's blocking API through `blocking`
  (`src/lib.rs`): `tokio::task::spawn_blocking` plus a flag set when tonic
  drops the call's future, which is how a client's cancellation reaches
  blocking work. `server::Options::slow_method` adds a test method,
  `VersionService/Slow`, that waits on that flag, for the cancellation case.

It is a standalone workspace (its own `Cargo.toml` with an empty
`[workspace]` table and its own `Cargo.lock`); the main workspace excludes
`spikes/`.

## Running

The tests use the in-tree spike's harness and cases
(`../grpc-intree/tests/common/mod.rs`, included by path), so both variants
meet exactly the same cases, and need the grpc-go oracle as there.

```sh
cargo test
PURSE_REQUIRE_ORACLE=1 cargo test
PURSE_REQUIRE_ORACLE=1 cargo test --test interop -- --nocapture --test-threads 1
PURSE_REQUIRE_ORACLE=1 cargo test --release --test measure -- --ignored --nocapture
cargo run --release --bin grpc-tonic-server -- \
    --cert DIR/rpc.cert --key DIR/rpc.key --clientca DIR/clients.pem
```

The raw-frame test prints how this server's answers differ from grpc-go's;
unlike the in-tree test it asserts only that the server keeps serving, since
h2's differences cannot be changed from outside it (see the ADR).
`io_level_cases` runs the in-tree spike's I/O-level cases against this
server.
