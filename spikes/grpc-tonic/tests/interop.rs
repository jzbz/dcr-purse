//! The tonic server against an unmodified grpc-go v1.83.2 client, case by
//! case beside the reference grpc-go server built as dcrwallet builds its
//! own. Run with `--nocapture` to see the comparison tables.
//!
//! The harness and the cases are the in-tree spike's, included by path, so
//! both variants meet exactly the same cases.

#[path = "../../grpc-intree/tests/common/mod.rs"]
mod common;

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::time::Duration;

use common::{Harness, alive, client_cases, describe_client, io, raw_cases, run_case, summarize};
use grpc_tonic_spike::{server, tls};
use serde_json::json;

fn start(h: &Harness) -> (server::Server, String) {
    start_with(h, server::Options::default())
}

fn start_with(h: &Harness, opts: server::Options) -> (server::Server, String) {
    let config = tls::server_config(
        &h.read("rpc.cert"),
        &h.read("rpc.key"),
        &h.read("clients.pem"),
    )
    .expect("TLS configuration");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let srv = server::serve(listener, Arc::new(config), opts).expect("serve");
    let addr = srv.local_addr().to_string();
    (srv, addr)
}

/// The in-tree spike's I/O-level cases, on this server: large requests
/// past any budget, a large header block, calls past 4,096 at once,
/// cancellation of slow calls, a client that never reads, idle sockets and
/// a trickled handshake.
#[test]
fn io_level_cases() {
    let Some(mut h) = Harness::new("tonic-io", "Ed25519") else {
        return;
    };
    let opts = server::Options {
        handshake_timeout: Duration::from_secs(2),
        slow_method: true,
        ..server::Options::default()
    };
    let (_srv, addr) = start_with(&h, opts);
    for (total, size) in [(20, 1_000_000), (200, 100_000)] {
        let r = h.run(
            "bigcalls",
            &addr,
            "client",
            json!({"total": total, "concurrency": total, "size": size, "timeout_ms": 60_000}),
        );
        eprintln!(
            "tonic {total} x {size} B: {:?} in {} ms",
            io::codes(&r),
            r["elapsed_ms"]
        );
        assert_eq!(io::codes(&r), vec![("OK".to_owned(), total)]);
    }
    let r = h.run(
        "metacall",
        &addr,
        "client",
        json!({"meta_key": "x-big", "size": 3_000_000, "timeout_ms": 30_000}),
    );
    eprintln!("tonic 3,000,000-byte metadata value: {}", r["status"]);
    assert_eq!(r["status"]["code"], 0);
    let r = h.run(
        "version",
        &addr,
        "client",
        json!({"n": 0, "total": 20_000, "concurrency": 20_000, "timeout_ms": 60_000}),
    );
    eprintln!(
        "tonic 20,000 concurrent calls: {} OK, {} errors",
        r["concurrent"]["calls"], r["concurrent"]["errors"]
    );
    assert_eq!(r["concurrent"]["errors"], 0);
    let r = h.run(
        "cancels",
        &addr,
        "client",
        json!({"calls": 1500, "method": "/walletrpc.VersionService/Slow", "call_ms": 300, "timeout_ms": 10_000}),
    );
    eprintln!("tonic: 1,500 cancelled calls: {r}");
    assert_eq!(io::codes(&r), vec![("DeadlineExceeded".to_owned(), 1500)]);
    assert_eq!(r["same_connection"], true, "{r}");
    let (id, written, victims) = io::slow_reader(&mut h, &addr, 400_000, 5, 3000);
    eprintln!("tonic: slow reader got {written} requests in; victims: {victims:?}");
    assert!(victims.iter().all(Result::is_ok), "{victims:?}");
    std::thread::sleep(Duration::from_secs(3));
    let (_, end) = io::slow_reader_stop(&mut h, id);
    eprintln!("tonic: slow reader's connection after 3 s: {end}");
    let (socks, delta) = io::idle_sockets(&addr, 500);
    eprintln!("tonic: 500 idle sockets: {delta} more threads");
    assert!(alive(&mut h, &addr));
    drop(socks);
    let closed = io::trickle(&addr, Duration::from_millis(300), Duration::from_secs(10));
    eprintln!("tonic: trickled handshake closed after {closed:?}");
    assert!(closed.expect("closed") < Duration::from_secs(3));
}

/// Every grpc-go client case, positive and negative, on both servers.
#[test]
fn grpc_go_client_cases() {
    let Some(mut h) = Harness::new("tonic-client", "Ed25519") else {
        return;
    };
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    eprintln!("| case | grpc-go reference | tonic |\n|---|---|---|");
    for (name, client, extra, cmd) in client_cases() {
        let r = h.run(cmd, &reference, client, extra.clone());
        let c = h.run(cmd, &addr, client, extra);
        let (rd, cd) = (describe_client(cmd, &r), describe_client(cmd, &c));
        eprintln!("| {name} | {rd} | {cd} |");
        match name {
            n if n.starts_with("issued") || n.starts_with("self-signed client") => {
                assert_eq!(
                    c["reply"],
                    json!({"version_string": "10.0.0", "major": 10}),
                    "{name}"
                );
                assert_eq!(c["reply"], r["reply"], "{name}");
                assert_eq!(c["headers"], r["headers"], "{name}");
                assert_eq!(c["trailers"], r["trailers"], "{name}");
                assert_eq!(c["tls"]["negotiated_protocol"], "h2");
            }
            n if n.starts_with("self-signed CA-flagged") => {
                // Go accepts a CA-flagged self-signed client certificate
                // listed in the pool; webpki refuses a CA certificate as an
                // end entity. A finding, pinned here (the same verifier as in-tree).
                assert!(r.get("reply").is_some(), "{name}: reference");
                assert_eq!(common::status(&c).0, 14, "{name}: tonic");
            }
            "no client certificate"
            | "client certificate from another CA"
            | "expired client certificate"
            | "plaintext (insecure credentials)" => {
                assert_eq!(common::status(&r).0, 14, "{name}: reference");
                assert_eq!(common::status(&c).0, 14, "{name}: tonic");
            }
            "HTTP/1.1 over TLS" => {
                assert!(
                    r.get("error").is_some() && c.get("error").is_some(),
                    "{name}"
                );
            }
            _ => {
                // unknown service, unknown method, malformed message, and a
                // call with metadata: same code, message and trailers.
                let (rc, rm) = common::status(&r);
                let (cc, cm) = common::status(&c);
                assert_eq!(rc, cc, "{name}");
                if name == "malformed request message" {
                    // tonic's codec reports prost's decode error in its own
                    // words: same code, different text. Recorded.
                    eprintln!("  {name}: reference {rm:?}, tonic {cm:?}");
                } else {
                    assert_eq!(rm, cm, "{name}");
                }
                // tonic answers with content-type application/grpc whatever
                // subtype the request named, where grpc-go echoes it
                // (application/grpc+proto here): recorded, not asserted.
                if r["trailers"] != c["trailers"] || r["headers"] != c["headers"] {
                    eprintln!(
                        "  {name}: headers/trailers differ: reference {} {}, tonic {} {}",
                        r["headers"], r["trailers"], c["headers"], c["trailers"]
                    );
                }
            }
        }
    }
    assert!(alive(&mut h, &addr));
}

/// Raw HTTP/2 cases, well-formed and hostile, on both servers: what each
/// sent back, and that the tonic server never stops serving.
#[test]
fn raw_frame_cases() {
    let Some(mut h) = Harness::new("tonic-raw", "Ed25519") else {
        return;
    };
    // A panic on any server thread is counted: the server must never panic.
    static PANICS: AtomicUsize = AtomicUsize::new(0);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Server threads only: a failing assertion in a test running in
        // parallel is not a server panic.
        let server = std::thread::current()
            .name()
            .is_some_and(|n| n.starts_with("grpc"));
        if server {
            PANICS.fetch_add(1, Ordering::SeqCst);
        }
        default_hook(info);
    }));
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    let mut differ = Vec::new();
    eprintln!("| case | grpc-go reference | tonic |\n|---|---|---|");
    for c in raw_cases() {
        let r = summarize(&run_case(&mut h, &reference, &c));
        let i = summarize(&run_case(&mut h, &addr, &c));
        let mark = if r == i { "" } else { " **differs**" };
        eprintln!("| {}{mark} | `{r}` | `{i}` |", c.name);
        if r != i {
            differ.push(c.name);
        }
        assert!(alive(&mut h, &addr), "tonic server dead after {}", c.name);
    }
    assert_eq!(PANICS.load(Ordering::SeqCst), 0, "a server thread panicked");
    eprintln!("cases that differ: {}", differ.len());
    for d in &differ {
        eprintln!("  {d}");
    }
}

/// Server key types: what the provider serves and what it refuses.
#[test]
fn server_key_types() {
    for key in ["Ed25519", "P-256", "P-384", "P-521", "RSA-2048"] {
        let Some(mut h) = Harness::new("tonic-keys", key) else {
            return;
        };
        let config = tls::server_config(
            &h.read("rpc.cert"),
            &h.read("rpc.key"),
            &h.read("clients.pem"),
        );
        match (key, config) {
            ("RSA-2048", Err(e)) => {
                eprintln!("{key}: refused: {e}");
                assert!(e.contains("RSA private keys are not supported"));
            }
            (_, Ok(config)) => {
                let srv = server::serve(
                    TcpListener::bind("127.0.0.1:0").unwrap(),
                    Arc::new(config),
                    server::Options::default(),
                )
                .unwrap();
                let addr = srv.local_addr().to_string();
                for tls_max in ["1.3", "1.2"] {
                    let r = h.run("version", &addr, "client", json!({"tls_max": tls_max}));
                    eprintln!("{key} TLS {tls_max}: {}", describe_client("version", &r));
                    assert_eq!(r["reply"]["version_string"], "10.0.0", "{key} {tls_max}");
                }
            }
            (_, Err(e)) => panic!("{key}: {e}"),
        }
    }
}

/// Sequential and concurrent calls on one connection, and many streams
/// opened at once, against both servers (debug build: the numbers here are
/// a smoke test; tests/measure.rs takes the measurements).
#[test]
fn concurrency_and_bursts() {
    let Some(mut h) = Harness::new("tonic-conc", "Ed25519") else {
        return;
    };
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    for (label, a) in [("reference", reference.as_str()), ("tonic", addr.as_str())] {
        for conc in [1, 16, 256] {
            let r = h.run(
                "version",
                a,
                "client",
                json!({"n": 10, "total": 1000, "concurrency": conc}),
            );
            let c = &r["concurrent"];
            eprintln!(
                "{label} concurrency {conc}: {} calls, {} errors, p50 {} us, p99 {} us, {:.0} calls/s",
                c["calls"],
                c["errors"],
                c["p50_us"],
                c["p99_us"],
                c["calls_per_s"].as_f64().unwrap_or(0.0)
            );
            assert_eq!(c["errors"], 0, "{label} {conc}");
            assert_eq!(c["calls"], 1000, "{label} {conc}");
        }
        for streams in [1000, 5000] {
            let r = h.run(
                "burst",
                a,
                "client",
                json!({"streams": streams, "timeout_ms": 30000}),
            );
            eprintln!("{label} burst {streams}: {r}");
            assert_eq!(r["completed"], streams, "{label} burst {streams}");
        }
    }
}

/// Client certificate key types (the client CA, and the self-signed
/// certificate in clients.pem, on the same type): what the provider
/// verifies, beside grpc-go, over TLS 1.3 and 1.2.
#[test]
fn client_key_types() {
    for key in ["Ed25519", "P-256", "P-384", "P-521", "RSA-2048"] {
        let Some(mut h) = Harness::with_keys("tonic-ckeys", "Ed25519", key) else {
            return;
        };
        let (_srv, addr) = start(&h);
        let reference = h.reference.clone();
        for (who, tls_max) in [
            ("client", "1.3"),
            ("client", "1.2"),
            ("selfsigned", "1.3"),
            ("selfsigned", "1.2"),
        ] {
            let args = json!({"tls_max": tls_max});
            let r = h.run("version", &reference, who, args.clone());
            let c = h.run("version", &addr, who, args);
            eprintln!(
                "client key {key} {who} TLS {tls_max}: reference {} | tonic {}",
                describe_client("version", &r),
                describe_client("version", &c)
            );
            assert_eq!(
                r["reply"]["version_string"], "10.0.0",
                "{key} {who}: reference"
            );
            // Go signs every P-521 certificate, CA-issued or self-signed,
            // with ECDSA-SHA512, which the provider verifies since dcroxide
            // 6272357 (its ADR-0015 T4); until then P-521 clients were
            // refused with code 14.
            assert_eq!(c["reply"]["version_string"], "10.0.0", "{key} {who}: tonic");
        }
    }
}
