//! The in-tree server against an unmodified grpc-go v1.83.2 client, case by
//! case beside the reference grpc-go server built as dcrwallet builds its
//! own. Run with `--nocapture` to see the comparison tables.

mod common;

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::time::{Duration, Instant};

use common::{Harness, alive, client_cases, describe_client, io, raw_cases, run_case, summarize};
use grpc_intree_spike::grpc::{CallContext, Router, Status, code};
use grpc_intree_spike::{server, tls, version_router};
use serde_json::json;

fn start(h: &Harness) -> (server::Server, String) {
    start_with(h, server::Options::default(), version_router())
}

fn start_with(h: &Harness, opts: server::Options, router: Router) -> (server::Server, String) {
    let config = tls::server_config(
        &h.read("rpc.cert"),
        &h.read("rpc.key"),
        &h.read("clients.pem"),
    )
    .expect("TLS configuration");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let srv = server::serve(listener, Arc::new(config), router, opts).expect("serve");
    let addr = srv.local_addr().to_string();
    (srv, addr)
}

/// A handler standing for slow wallet work that checks for cancellation,
/// as the full server's handlers would: it returns when the client cancels
/// or after five seconds.
fn slow(ctx: &CallContext, _req: &[u8]) -> Result<Vec<u8>, Status> {
    let start = Instant::now();
    while !ctx.is_cancelled() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(Status::new(code::DEADLINE_EXCEEDED, "slow call ended"))
}

/// Every grpc-go client case, positive and negative, on both servers.
#[test]
fn grpc_go_client_cases() {
    let Some(mut h) = Harness::new("intree-client", "Ed25519") else {
        return;
    };
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    eprintln!("| case | grpc-go reference | in-tree |\n|---|---|---|");
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
                // end entity. A finding, pinned here.
                assert!(r.get("reply").is_some(), "{name}: reference");
                assert_eq!(common::status(&c).0, 14, "{name}: in-tree");
            }
            "no client certificate"
            | "client certificate from another CA"
            | "expired client certificate"
            | "plaintext (insecure credentials)" => {
                assert_eq!(common::status(&r).0, 14, "{name}: reference");
                assert_eq!(common::status(&c).0, 14, "{name}: in-tree");
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
                    // Go's protobuf text carries a separator its detrand
                    // package randomizes per binary (space or U+00A0).
                    let norm = |s: &str| s.replace('\u{a0}', " ");
                    assert_eq!(norm(&rm), norm(&cm), "{name}");
                } else {
                    assert_eq!(rm, cm, "{name}");
                }
                assert_eq!(r["trailers"], c["trailers"], "{name}");
                assert_eq!(r["headers"], c["headers"], "{name}");
            }
        }
    }
    assert!(alive(&mut h, &addr));
}

/// Raw HTTP/2 cases, well-formed and hostile, on both servers: what each
/// sent back, and that the in-tree server never stops serving.
#[test]
fn raw_frame_cases() {
    let Some(mut h) = Harness::new("intree-raw", "Ed25519") else {
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
    eprintln!("| case | grpc-go reference | in-tree |\n|---|---|---|");
    for c in raw_cases() {
        let r = summarize(&run_case(&mut h, &reference, &c));
        let i = summarize(&run_case(&mut h, &addr, &c));
        let mark = if r == i { "" } else { " **differs**" };
        eprintln!("| {}{mark} | `{r}` | `{i}` |", c.name);
        if r != i {
            differ.push(c.name);
        }
        if common::INTREE_PARITY.contains(&c.name) {
            assert_eq!(
                common::without_clean_resets(&r),
                common::without_clean_resets(&i),
                "{}",
                c.name
            );
        }
        assert!(alive(&mut h, &addr), "in-tree server dead after {}", c.name);
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
        let Some(mut h) = Harness::new("intree-keys", key) else {
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
                    version_router(),
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
    let Some(mut h) = Harness::new("intree-conc", "Ed25519") else {
        return;
    };
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    for (label, a) in [
        ("reference", reference.as_str()),
        ("in-tree", addr.as_str()),
    ] {
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
        // Past the stream cap (4,096, advertised): grpc-go's client queues
        // the calls past it instead of having them refused.
        let r = h.run(
            "version",
            a,
            "client",
            json!({"n": 0, "total": 20_000, "concurrency": 20_000, "timeout_ms": 60_000}),
        );
        let c = &r["concurrent"];
        eprintln!(
            "{label} 20,000 concurrent calls: {} OK, {} errors {}",
            c["calls"], c["errors"], r["concurrent_status"]
        );
        assert_eq!(c["errors"], 0, "{label} 20,000 concurrent");
    }
}

/// Large requests on one connection, through grpc-go's client and its flow
/// control, past the per-connection request budget: here a 4 MiB budget
/// (the default is 64 MiB; `tests/measure.rs` runs 20 x 4,000,000 and
/// 1,000 x 100,000 bytes against it in release), with twenty requests of
/// 1,000,000 bytes and two hundred of 100,000 at once. Before admission
/// existed, partial messages filling the budget wedged the connection.
#[test]
fn large_concurrent_requests() {
    let Some(mut h) = Harness::new("intree-large", "Ed25519") else {
        return;
    };
    let mut opts = server::Options::default();
    opts.limits.max_buffered = 4 << 20;
    let (_srv, addr) = start_with(&h, opts, version_router());
    let reference = h.reference.clone();
    for (label, a) in [
        ("reference", reference.as_str()),
        ("in-tree", addr.as_str()),
    ] {
        for (total, size) in [(20, 1_000_000), (200, 100_000)] {
            let r = h.run(
                "bigcalls",
                a,
                "client",
                json!({"total": total, "concurrency": total, "size": size, "timeout_ms": 60_000}),
            );
            eprintln!(
                "{label} {total} x {size} B: {:?} in {} ms",
                io::codes(&r),
                r["elapsed_ms"]
            );
            assert_eq!(
                io::codes(&r),
                vec![("OK".to_owned(), total)],
                "{label} {total} x {size}"
            );
        }
    }
}

/// A 3,000,000-byte metadata value, which grpc-go's client sends as a
/// header block of 184 frames: grpc-go accepts it (its budget is 16 MiB);
/// before this was fixed the in-tree server closed the connection at 1 MiB
/// or 128 frames.
#[test]
fn large_request_headers() {
    let Some(mut h) = Harness::new("intree-meta", "Ed25519") else {
        return;
    };
    let (_srv, addr) = start(&h);
    let reference = h.reference.clone();
    for (label, a) in [
        ("reference", reference.as_str()),
        ("in-tree", addr.as_str()),
    ] {
        let r = h.run(
            "metacall",
            a,
            "client",
            json!({"meta_key": "x-big", "size": 3_000_000, "timeout_ms": 30_000}),
        );
        eprintln!("{label} 3,000,000-byte metadata value: {}", r["status"]);
        assert_eq!(r["status"]["code"], 0, "{label}");
    }
}

/// 1,500 calls cancelled by their deadline while their handlers are still
/// working: ordinary cancellation, which must not close the connection.
/// Before this was fixed each counted as a rapid-reset strike and the
/// 1,001st closed the connection with ENHANCE_YOUR_CALM.
#[test]
fn cancelled_calls_keep_the_connection() {
    let Some(mut h) = Harness::new("intree-cancel", "Ed25519") else {
        return;
    };
    let router = version_router().unary("walletrpc.VersionService", "Slow", slow);
    let (_srv, addr) = start_with(&h, server::Options::default(), router);
    let r = h.run(
        "cancels",
        &addr,
        "client",
        json!({"calls": 1500, "method": "/walletrpc.VersionService/Slow", "call_ms": 300, "timeout_ms": 10_000}),
    );
    eprintln!("in-tree: 1,500 cancelled calls: {r}");
    assert_eq!(io::codes(&r), vec![("DeadlineExceeded".to_owned(), 1500)]);
    assert_eq!(r["same_connection"], true, "{r}");
}

/// A client that sends requests and never reads must not stall anyone
/// else, and its connection must be closed once writes stop progressing.
/// Before this was fixed the workers blocked writing to it while holding
/// its lock, and every other client's calls timed out.
#[test]
fn slow_reader_cannot_stall_other_clients() {
    let Some(mut h) = Harness::new("intree-slowread", "Ed25519") else {
        return;
    };
    let opts = server::Options {
        write_timeout: Duration::from_secs(2),
        ..server::Options::default()
    };
    let (srv, addr) = start_with(&h, opts, version_router());
    // The reference beside it, for the record (not asserted).
    let reference = h.reference.clone();
    let (rid, rwritten, rvictims) = io::slow_reader(&mut h, &reference, 400_000, 5, 3000);
    std::thread::sleep(Duration::from_secs(3));
    let (_, rend) = io::slow_reader_stop(&mut h, rid);
    eprintln!(
        "reference: slow reader got {rwritten} requests in; victims: {rvictims:?}; its connection after 3 s: {rend}"
    );
    let (id, written, victims) = io::slow_reader(&mut h, &addr, 400_000, 5, 3000);
    eprintln!("in-tree: slow reader got {written} requests in; victims: {victims:?}");
    assert!(victims.iter().all(Result::is_ok), "{victims:?}");
    // Shutdown must not wait on the stuck connection.
    std::thread::sleep(Duration::from_secs(3));
    let (_, end) = io::slow_reader_stop(&mut h, id);
    let stats = srv.stats();
    eprintln!("in-tree: slow reader's connection: {end}; {stats:?}");
    assert_eq!(stats.write_timeouts, 1, "{stats:?}");
    assert!(end.starts_with("closed"), "{end}");
    let t = Instant::now();
    srv.shutdown();
    assert!(t.elapsed() < Duration::from_secs(1));
}

/// Shutdown while a peer is stuck: it returns at once.
#[test]
fn shutdown_does_not_wait_on_a_stuck_peer() {
    let Some(mut h) = Harness::new("intree-shutdown", "Ed25519") else {
        return;
    };
    let (srv, addr) = start(&h);
    let (id, _, _) = io::slow_reader(&mut h, &addr, 400_000, 0, 1000);
    let t = Instant::now();
    srv.shutdown();
    let took = t.elapsed();
    io::slow_reader_stop(&mut h, id);
    eprintln!("in-tree: shutdown with a stuck peer took {took:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
}

/// Unauthenticated sockets: admission caps the threads they can take, and
/// the handshake deadline is absolute, so trickling bytes does not extend
/// it. Before this was fixed every socket got a thread with no limit and a
/// trickle kept one open indefinitely.
#[test]
fn unauthenticated_sockets_are_bounded() {
    let Some(mut h) = Harness::new("intree-sockets", "Ed25519") else {
        return;
    };
    let opts = server::Options {
        max_handshakes: 8,
        max_connections: 32,
        handshake_timeout: Duration::from_secs(2),
        ..server::Options::default()
    };
    let (srv, addr) = start_with(&h, opts, version_router());
    // An authenticated client connected before the flood.
    assert!(alive(&mut h, &addr));
    let (socks, delta) = io::idle_sockets(&addr, 50);
    let stats = srv.stats();
    // The process-wide thread count also moves with tests running in
    // parallel, so it is printed; the server's own counters are asserted
    // (one thread per admitted connection).
    eprintln!("in-tree: 50 idle sockets: {delta} more threads in the process; {stats:?}");
    assert_eq!(stats.handshaking, 8);
    assert_eq!(stats.connections, 8);
    assert_eq!(stats.refused, 42);
    // While the handshake slots are held, a new client is refused too: the
    // cost of a cap that refuses rather than queues.
    let r = h.run(
        "version",
        &addr,
        "client",
        json!({"n": 0, "timeout_ms": 1000}),
    );
    eprintln!(
        "in-tree: new client while the slots are full: {}",
        common::status(&r).1
    );
    assert_eq!(common::status(&r).0, 14);
    // The deadline frees them.
    std::thread::sleep(Duration::from_millis(2500));
    let stats = srv.stats();
    assert_eq!(stats.handshaking, 0, "{stats:?}");
    assert_eq!(stats.handshake_timeouts, 8, "{stats:?}");
    assert!(alive(&mut h, &addr));
    drop(socks);
    // A trickled handshake is closed at the deadline, not kept open.
    let closed = io::trickle(&addr, Duration::from_millis(300), Duration::from_secs(10));
    eprintln!("in-tree: trickled handshake closed after {closed:?}");
    let closed = closed.expect("closed");
    assert!(closed < Duration::from_secs(3), "{closed:?}");
}

/// Client certificate key types (the client CA, and the self-signed
/// certificate in clients.pem, on the same type): what the provider
/// verifies, beside grpc-go, over TLS 1.3 and 1.2.
#[test]
fn client_key_types() {
    for key in ["Ed25519", "P-256", "P-384", "P-521", "RSA-2048"] {
        let Some(mut h) = Harness::with_keys("intree-ckeys", "Ed25519", key) else {
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
                "client key {key} {who} TLS {tls_max}: reference {} | in-tree {}",
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
            assert_eq!(
                c["reply"]["version_string"], "10.0.0",
                "{key} {who}: in-tree"
            );
        }
    }
}
