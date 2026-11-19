//! Decision spike for D5, variant C: a minimal gRPC server written in tree.
//!
//! It answers `walletrpc.VersionService/Version` over HTTP/2 and mutual
//! TLS to an unmodified grpc-go client, using:
//!
//! - [`frame`] and [`conn`]: HTTP/2 framing and the connection state machine
//!   (preface, SETTINGS, HEADERS/CONTINUATION, DATA, WINDOW_UPDATE flow
//!   control, PING with grpc-go's keepalive enforcement, GOAWAY,
//!   RST_STREAM), with no I/O;
//! - [`hpack`] and [`huffman`]: HPACK with the static and dynamic tables and
//!   Huffman decoding;
//! - [`grpc`]: gRPC message framing, request checks, statuses and trailers;
//! - [`proto`]: the protobuf codec for the two messages;
//! - [`tls`] and [`server`]: rustls on `dcroxide-tlscrypto`, OS threads and
//!   blocking I/O, as dcroxide's own HTTP/1.1 and websocket servers are:
//!   admission at accept, a reader and a writer thread per connection, and
//!   a worker pool for handlers.
//!
//! This is a spike: it exists to produce evidence for the decision, and is
//! not the wallet's gRPC server.

#![forbid(unsafe_code)]

pub mod conn;
pub mod frame;
pub mod grpc;
pub mod hpack;
pub mod huffman;
pub mod proto;
pub mod server;
pub mod tls;

use grpc::{CallContext, Router, Status, code};
use proto::{VersionRequest, VersionResponse};

/// The gRPC API version (dcrwallet `internal/rpc/rpcserver/server.go:63-68`).
pub const SEMVER: (&str, u32, u32, u32) = ("10.0.0", 10, 0, 0);

/// The wallet side of `Version`: a blocking call, as every wallet API the
/// full server calls would be.
pub fn wallet_version() -> VersionResponse {
    VersionResponse {
        version_string: SEMVER.0.to_owned(),
        major: SEMVER.1,
        minor: SEMVER.2,
        patch: SEMVER.3,
        ..Default::default()
    }
}

/// `VersionService.Version`: decode, call the wallet, encode.
pub fn version(_ctx: &CallContext, req: &[u8]) -> Result<Vec<u8>, Status> {
    VersionRequest::decode(req).map_err(|e| {
        Status::new(
            code::INTERNAL,
            format!("grpc: error unmarshalling request: {e}"),
        )
    })?;
    Ok(wallet_version().encode())
}

/// The router with VersionService registered.
pub fn version_router() -> Router {
    Router::default().unary("walletrpc.VersionService", "Version", version)
}
