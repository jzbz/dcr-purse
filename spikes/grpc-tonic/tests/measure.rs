//! Latency and throughput of unary Version calls at 1, 16 and 256
//! concurrent calls on one connection, for this server and the reference
//! grpc-go server, driven by the same grpc-go client. Ignored by default;
//! run in release:
//!
//! `PURSE_REQUIRE_ORACLE=1 cargo test --release --test measure -- --ignored --nocapture`

#[path = "../../grpc-intree/tests/common/mod.rs"]
mod common;

use std::net::TcpListener;
use std::sync::Arc;

use common::Harness;
use serde_json::json;

const LABEL: &str = "tonic";

fn start(h: &Harness) -> (Box<dyn std::any::Any>, String) {
    use grpc_tonic_spike::{server, tls};
    let config = tls::server_config(
        &h.read("rpc.cert"),
        &h.read("rpc.key"),
        &h.read("clients.pem"),
    )
    .expect("TLS configuration");
    let srv = server::serve(
        TcpListener::bind("127.0.0.1:0").expect("bind"),
        Arc::new(config),
        server::Options::default(),
    )
    .expect("serve");
    let addr = srv.local_addr().to_string();
    (Box::new(srv), addr)
}

/// The process's thread count and resident set, from /proc (Linux only).
fn proc_status() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .map(|l| l[k.len()..].trim().to_owned())
            .unwrap_or_default()
    };
    format!("threads {}, VmRSS {}", field("Threads:"), field("VmRSS:"))
}

#[test]
#[ignore = "measurement; run in release with --ignored"]
fn measure() {
    let Some(mut h) = Harness::new("measure", "Ed25519") else {
        return;
    };
    eprintln!("before the server: {}", proc_status());
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    eprintln!("| server | concurrency | calls | p50 µs | p90 µs | p99 µs | max µs | calls/s |");
    eprintln!("|---|---|---|---|---|---|---|---|");
    for (label, a) in [("grpc-go", reference.as_str()), (LABEL, addr.as_str())] {
        // Warm up the connection pool, page cache and allocators.
        h.run(
            "version",
            a,
            "client",
            json!({"n": 200, "total": 2000, "concurrency": 16}),
        );
        for (conc, total) in [(1, 5000), (16, 40_000), (256, 80_000)] {
            let r = h.run(
                "version",
                a,
                "client",
                json!({"n": 1, "total": total, "concurrency": conc, "timeout_ms": 60000}),
            );
            let c = &r["concurrent"];
            assert_eq!(c["errors"], 0, "{label} {conc}");
            eprintln!(
                "| {label} | {conc} | {} | {} | {} | {} | {} | {:.0} |",
                c["calls"],
                c["p50_us"],
                c["p90_us"],
                c["p99_us"],
                c["max_us"],
                c["calls_per_s"].as_f64().unwrap_or(0.0)
            );
        }
        // A new connection per call: handshake plus one call.
        if label == LABEL {
            eprintln!("after load: {}", proc_status());
        }
    }
    // Large requests on one connection: 80 MB in 20 requests of 4,000,000
    // bytes (past the in-tree server's 64 MiB budget), and 100 MB in 1,000
    // of 100,000 bytes, all at once.
    for (label, a) in [("grpc-go", reference.as_str()), (LABEL, addr.as_str())] {
        for (total, size) in [(20u64, 4_000_000u64), (1000, 100_000)] {
            let mut ms = Vec::new();
            for _ in 0..5 {
                let r = h.run(
                    "bigcalls",
                    a,
                    "client",
                    json!({"total": total, "concurrency": total, "size": size, "timeout_ms": 60000}),
                );
                assert_eq!(r["codes"]["OK"], total, "{label} {total} x {size}: {r}");
                ms.push(r["elapsed_ms"].as_f64().unwrap_or(0.0));
            }
            ms.sort_by(f64::total_cmp);
            eprintln!(
                "{label}: {total} x {size} B at once: median {:.0} ms ({:.0} MB/s)",
                ms[2],
                (total * size) as f64 / 1e6 / (ms[2] / 1e3)
            );
        }
    }
    // Connection set-up latency: dial, TLS 1.3 handshake, first call.
    for (label, a) in [("grpc-go", reference.as_str()), (LABEL, addr.as_str())] {
        let mut v: Vec<f64> = (0..300)
            .map(|_| {
                let t = std::time::Instant::now();
                let r = h.run("version", a, "client", json!({"n": 0}));
                assert_eq!(r["reply"]["version_string"], "10.0.0");
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        v.sort_by(f64::total_cmp);
        eprintln!(
            "{label}: new connection + first call (oracle round trip included): p50 {:.0} µs, p99 {:.0} µs",
            v[v.len() / 2],
            v[v.len() * 99 / 100]
        );
    }
}
