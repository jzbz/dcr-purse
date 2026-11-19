//! Decision spike for D5, variant A: tonic and prost on tokio, hyper and h2,
//! with the async runtime confined to the gRPC listener.
//!
//! It answers `walletrpc.VersionService/Version` over HTTP/2 and mutual
//! TLS to an unmodified grpc-go client. TLS is rustls 0.23 on
//! `dcroxide-tlscrypto` through tokio-rustls, configured exactly as the
//! in-tree variant's; tonic's own transport (and with it axum and tonic's
//! TLS features) is not used: each accepted connection is served by hyper's
//! HTTP/2 server with the tonic service below, which is what tonic-build
//! would generate for the service, written out by hand so the build needs
//! no `protoc`.
//!
//! Handlers reach the wallet, whose API is blocking, through
//! `tokio::task::spawn_blocking`: the pattern the full server would use for
//! every RPC (decision D6).

#![forbid(unsafe_code)]

pub mod server;
pub mod tls;

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};

use tonic::codegen::{BoxFuture, StdError, http};

/// `walletrpc.VersionRequest`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct VersionRequest {}

/// `walletrpc.VersionResponse`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct VersionResponse {
    /// Field 1.
    #[prost(string, tag = "1")]
    pub version_string: String,
    /// Field 2.
    #[prost(uint32, tag = "2")]
    pub major: u32,
    /// Field 3.
    #[prost(uint32, tag = "3")]
    pub minor: u32,
    /// Field 4.
    #[prost(uint32, tag = "4")]
    pub patch: u32,
    /// Field 5.
    #[prost(string, tag = "5")]
    pub prerelease: String,
    /// Field 6.
    #[prost(string, tag = "6")]
    pub build_metadata: String,
}

/// The wallet side of `Version`: a blocking call, as every wallet API the
/// full server calls would be.
pub fn wallet_version() -> VersionResponse {
    VersionResponse {
        version_string: "10.0.0".into(),
        major: 10,
        minor: 0,
        patch: 0,
        ..Default::default()
    }
}

/// The service: tonic-build's generated server shape for one unary method,
/// with grpc-go's Unimplemented messages for unknown paths in place of
/// tonic's bare Unimplemented.
#[derive(Clone, Default)]
pub struct VersionServiceServer {
    _inner: Arc<()>,
    slow: bool,
}

impl VersionServiceServer {
    /// The service with a test method, `VersionService/Slow`, standing for
    /// slow wallet work that checks for cancellation (see [`blocking`]).
    pub fn with_slow_method() -> Self {
        VersionServiceServer {
            slow: true,
            ..Default::default()
        }
    }
}

/// Sets the flag when dropped: tonic drops a call's future when the client
/// cancels it, and this is how that reaches blocking work.
struct CancelOnDrop(Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The bridge every handler would use: runs `f`, a blocking wallet call, on
/// the runtime's blocking pool with a flag that is set if the call is
/// cancelled (the future dropped). Without the flag a cancelled call keeps
/// its blocking thread until the wallet call returns on its own.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce(&std::sync::atomic::AtomicBool) -> T + Send + 'static,
) -> Result<T, tonic::Status> {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _guard = CancelOnDrop(Arc::clone(&flag));
    tokio::task::spawn_blocking(move || f(&flag))
        .await
        .map_err(|e| tonic::Status::internal(e.to_string()))
}

struct SlowSvc;

impl tonic::server::UnaryService<VersionRequest> for SlowSvc {
    type Response = VersionResponse;
    type Future = BoxFuture<tonic::Response<VersionResponse>, tonic::Status>;

    fn call(&mut self, _request: tonic::Request<VersionRequest>) -> Self::Future {
        Box::pin(async move {
            blocking(|cancelled| {
                let start = std::time::Instant::now();
                while !cancelled.load(std::sync::atomic::Ordering::Relaxed)
                    && start.elapsed() < std::time::Duration::from_secs(5)
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            })
            .await?;
            Err(tonic::Status::deadline_exceeded("slow call ended"))
        })
    }
}

const SERVICE: &str = "walletrpc.VersionService";

struct VersionSvc;

impl tonic::server::UnaryService<VersionRequest> for VersionSvc {
    type Response = VersionResponse;
    type Future = BoxFuture<tonic::Response<VersionResponse>, tonic::Status>;

    fn call(&mut self, _request: tonic::Request<VersionRequest>) -> Self::Future {
        Box::pin(async move {
            // The bridge from the runtime to the blocking wallet API.
            let reply = blocking(|_| wallet_version()).await?;
            Ok(tonic::Response::new(reply))
        })
    }
}

/// grpc-go's statuses for a path that names no method (server.go:1824-1875).
fn unroutable(path: &str) -> tonic::Status {
    let Some(sm) = path.strip_prefix('/') else {
        return tonic::Status::unimplemented(format!("malformed method name: {path:?}"));
    };
    let Some(pos) = sm.rfind('/') else {
        return tonic::Status::unimplemented(format!("malformed method name: {path:?}"));
    };
    let (service, method) = (&sm[..pos], &sm[pos + 1..]);
    if service == SERVICE {
        tonic::Status::unimplemented(format!("unknown method {method} for service {service}"))
    } else {
        tonic::Status::unimplemented(format!("unknown service {service}"))
    }
}

impl<B> tonic::codegen::Service<http::Request<B>> for VersionServiceServer
where
    B: tonic::codegen::Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        match req.uri().path() {
            "/walletrpc.VersionService/Version" => Box::pin(async move {
                let codec = tonic_prost::ProstCodec::default();
                let mut grpc = tonic::server::Grpc::new(codec)
                    .apply_max_message_size_config(Some(4 << 20), None);
                Ok(grpc.unary(VersionSvc, req).await)
            }),
            "/walletrpc.VersionService/Slow" if self.slow => Box::pin(async move {
                let codec = tonic_prost::ProstCodec::default();
                let mut grpc = tonic::server::Grpc::new(codec)
                    .apply_max_message_size_config(Some(4 << 20), None);
                Ok(grpc.unary(SlowSvc, req).await)
            }),
            path => {
                let status = unroutable(path);
                Box::pin(async move { Ok(status.into_http()) })
            }
        }
    }
}
