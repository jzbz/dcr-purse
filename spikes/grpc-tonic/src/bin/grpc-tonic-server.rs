//! Runs the tonic spike server.
//!
//! `grpc-tonic-server --cert rpc.cert --key rpc.key --clientca clients.pem
//! [--listen 127.0.0.1:0]` prints the bound address and serves until killed.

use std::net::TcpListener;
use std::sync::Arc;

use grpc_tonic_spike::{server, tls};

fn main() {
    let mut cert = None;
    let mut key = None;
    let mut ca = None;
    let mut listen = "127.0.0.1:0".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next();
        match (a.as_str(), v) {
            ("--cert", Some(v)) => cert = Some(v),
            ("--key", Some(v)) => key = Some(v),
            ("--clientca", Some(v)) => ca = Some(v),
            ("--listen", Some(v)) => listen = v,
            _ => {
                eprintln!("usage: grpc-tonic-server --cert F --key F --clientca F [--listen A]");
                std::process::exit(2);
            }
        }
    }
    let read = |p: Option<String>, what: &str| {
        let p = p.unwrap_or_else(|| {
            eprintln!("missing --{what}");
            std::process::exit(2)
        });
        std::fs::read(&p).unwrap_or_else(|e| {
            eprintln!("{p}: {e}");
            std::process::exit(1)
        })
    };
    let (cert, key, ca) = (read(cert, "cert"), read(key, "key"), read(ca, "clientca"));
    let config = tls::server_config(&cert, &key, &ca).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("{listen}: {e}");
        std::process::exit(1)
    });
    let srv =
        server::serve(listener, Arc::new(config), server::Options::default()).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1)
        });
    println!("listening on {}", srv.local_addr());
    loop {
        std::thread::park();
    }
}
