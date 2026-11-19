//! The listener: a tokio multi-threaded runtime owned by the server and
//! used for nothing else, a TLS handshake per connection through
//! tokio-rustls, and hyper's HTTP/2 server with the tonic service.
//!
//! hyper's HTTP/2 options are set toward grpc-go's defaults as dcrwallet
//! runs them: no advertised concurrent-stream limit (hyper advertises 200
//! by default), 65,535-byte windows without adaptive sizing (grpc-go's BDP
//! estimator is the one thing not approximated), a 16 MiB header list, a
//! keepalive ping after 2 hours idle with a 20-second timeout, and no
//! `date` header. h2 has no counterpart of grpc-go's ping-strike policy.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;

use crate::VersionServiceServer;

/// Server options.
#[derive(Debug, Clone)]
pub struct Options {
    /// Runtime worker threads.
    pub workers: usize,
    /// Deadline for the TLS handshake (grpc-go's connection timeout).
    pub handshake_timeout: Duration,
    /// Serve the test method `VersionService/Slow` as well.
    pub slow_method: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            workers: std::thread::available_parallelism().map_or(4, |n| n.get()),
            handshake_timeout: Duration::from_secs(120),
            slow_method: false,
        }
    }
}

/// A running server.
pub struct Server {
    addr: SocketAddr,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Server {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

fn http2() -> hyper::server::conn::http2::Builder<TokioExecutor> {
    let mut b = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
    b.timer(TokioTimer::new())
        .max_concurrent_streams(None)
        .initial_stream_window_size(65_535)
        .initial_connection_window_size(65_535)
        .adaptive_window(false)
        .max_frame_size(16_384)
        .max_header_list_size(16 << 20)
        .keep_alive_interval(Duration::from_secs(2 * 3600))
        .keep_alive_timeout(Duration::from_secs(20))
        .auto_date_header(false);
    b
}

/// Serves VersionService on `listener` with the TLS configuration `tls`.
pub fn serve(
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    opts: Options,
) -> io::Result<Server> {
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(opts.workers.max(1))
        .thread_name("grpc-tokio")
        .enable_all()
        .build()?;
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    let listener = {
        let _guard = runtime.enter();
        tokio::net::TcpListener::from_std(listener)?
    };
    let builder = Arc::new(http2());
    runtime.spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                continue;
            };
            let acceptor = acceptor.clone();
            let builder = Arc::clone(&builder);
            let timeout = opts.handshake_timeout;
            let slow = opts.slow_method;
            tokio::spawn(async move {
                let _ = tcp.set_nodelay(true);
                let Ok(Ok(tls)) = tokio::time::timeout(timeout, acceptor.accept(tcp)).await else {
                    return;
                };
                if tls.get_ref().1.alpn_protocol() != Some(b"h2") {
                    // grpc-go closes a connection that negotiated no ALPN.
                    return;
                }
                let svc = if slow {
                    VersionServiceServer::with_slow_method()
                } else {
                    VersionServiceServer::default()
                };
                let svc = TowerToHyperService::new(svc);
                let _ = builder.serve_connection(TokioIo::new(tls), svc).await;
            });
        }
    });
    Ok(Server {
        addr,
        runtime: Some(runtime),
    })
}
