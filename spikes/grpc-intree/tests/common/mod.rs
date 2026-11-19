//! The interop harness both spikes share (grpc-tonic includes this file by
//! path): the grpc-go oracle `tools/oracle/cmd/grpcclient`, the reference
//! grpc-go server it runs, PKI in a test temp directory, raw HTTP/2 frame
//! builders, and a summary of what a server sent that two servers can be
//! compared on.
//!
//! Nothing here depends on either spike's code.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use purse_testutil::{Oracle, hex, oracle_or_skip};
use serde_json::{Value, json};

/// A fresh directory under cargo's per-target test temp directory.
pub fn temp_dir(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "{label}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The oracle, the PKI it generated and the reference server it runs.
pub struct Harness {
    /// The oracle process.
    pub oracle: Oracle,
    /// File name to path, from the oracle's `pki` command.
    pub pki: Value,
    /// The reference grpc-go server's address.
    pub reference: String,
}

impl Harness {
    /// Spawns the oracle, generates PKI with the given server key type and
    /// starts the reference server; `None` when the oracle is unavailable
    /// and not required.
    pub fn new(label: &str, server_key: &str) -> Option<Harness> {
        Self::with_keys(label, server_key, "Ed25519")
    }

    /// As [`Harness::new`], with the client CA and client certificates on
    /// keys of type `client_key`.
    pub fn with_keys(label: &str, server_key: &str, client_key: &str) -> Option<Harness> {
        let mut oracle = oracle_or_skip("grpcclient")?;
        let dir = temp_dir(label);
        let pki = oracle.call(
            "pki",
            json!({"dir": dir, "server_key": server_key, "client_key": client_key}),
        );
        let reference = if server_key == "RSA-2048" || server_key.is_empty() {
            String::new()
        } else {
            oracle.call(
                "server.start",
                json!({"cert": pki["rpc.cert"], "key": pki["rpc.key"], "clientca": pki["clients.pem"]}),
            )["addr"]
                .as_str()
                .expect("addr")
                .to_owned()
        };
        Some(Harness {
            oracle,
            pki,
            reference,
        })
    }

    /// A PKI file's path.
    pub fn file(&self, name: &str) -> String {
        self.pki[name].as_str().expect("pki file").to_owned()
    }

    /// The PEM bytes of a PKI file.
    pub fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.file(name)).expect("read pki file")
    }

    /// Dial arguments for `addr` with the client identity `client` (a file
    /// stem such as "client"; empty for none).
    pub fn base(&self, addr: &str, client: &str) -> Value {
        let mut v = json!({"addr": addr, "rootca": self.file("rpc.cert"), "timeout_ms": 5000});
        if !client.is_empty() {
            v["cert"] = json!(self.file(&format!("{client}.pem")));
            v["key"] = json!(self.file(&format!("{client}.key")));
        }
        v
    }

    /// Runs `cmd` with the base arguments merged with `extra`.
    pub fn run(&mut self, cmd: &str, addr: &str, client: &str, extra: Value) -> Value {
        let mut args = self.base(addr, client);
        if let (Some(a), Some(e)) = (args.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                a.insert(k.clone(), v.clone());
            }
        }
        self.oracle.call(cmd, args)
    }

    /// Sends raw frames after the preface and reports what came back.
    pub fn raw(&mut self, addr: &str, steps: &[(Vec<u8>, usize)], read_ms: u64) -> Value {
        let steps: Vec<Value> = steps
            .iter()
            .map(|(b, n)| json!({"hex": hex(b), "repeat": n}))
            .collect();
        self.run(
            "raw",
            addr,
            "client",
            json!({"steps": steps, "read_ms": read_ms}),
        )
    }
}

/// Raw HTTP/2 frame builders.
pub mod h2 {
    /// A frame.
    pub fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + payload.len());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
        out.push(kind);
        out.push(flags);
        out.extend_from_slice(&stream.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// A frame header claiming `len` bytes, without the payload.
    pub fn header_only(kind: u8, flags: u8, stream: u32, len: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&len.to_be_bytes()[1..]);
        out.push(kind);
        out.push(flags);
        out.extend_from_slice(&stream.to_be_bytes());
        out
    }

    /// An empty SETTINGS frame.
    pub fn settings(values: &[(u16, u32)]) -> Vec<u8> {
        let mut p = Vec::new();
        for (id, v) in values {
            p.extend_from_slice(&id.to_be_bytes());
            p.extend_from_slice(&v.to_be_bytes());
        }
        frame(4, 0, 0, &p)
    }

    /// A HPACK block of literals without indexing, new names, no Huffman.
    pub fn block(fields: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (n, v) in fields {
            out.push(0);
            push_str(&mut out, n.as_bytes());
            push_str(&mut out, v.as_bytes());
        }
        out
    }

    /// An HPACK string literal (no Huffman).
    pub fn push_str(out: &mut Vec<u8>, s: &[u8]) {
        push_int(out, 0, 7, s.len());
        out.extend_from_slice(s);
    }

    /// An HPACK integer.
    pub fn push_int(out: &mut Vec<u8>, first: u8, n: u32, mut v: usize) {
        let mask = (1usize << n) - 1;
        if v < mask {
            out.push(first | v as u8);
            return;
        }
        out.push(first | mask as u8);
        v -= mask;
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    /// The request headers of a Version call, optionally altered.
    pub fn request_fields(path: &'static str) -> Vec<(&'static str, &'static str)> {
        vec![
            (":method", "POST"),
            (":scheme", "https"),
            (":path", path),
            (":authority", "127.0.0.1"),
            ("content-type", "application/grpc"),
            ("te", "trailers"),
        ]
    }

    /// The Version path.
    pub const VERSION: &str = "/walletrpc.VersionService/Version";

    /// HEADERS with END_HEADERS (and END_STREAM when `end`).
    pub fn headers(stream: u32, fields: &[(&str, &str)], end: bool) -> Vec<u8> {
        frame(1, 0x4 | u8::from(end), stream, &block(fields))
    }

    /// DATA carrying one gRPC message.
    pub fn message(stream: u32, msg: &[u8], end: bool) -> Vec<u8> {
        let mut p = vec![0];
        p.extend_from_slice(&(msg.len() as u32).to_be_bytes());
        p.extend_from_slice(msg);
        frame(0, u8::from(end), stream, &p)
    }

    /// A complete Version request on `stream`.
    pub fn version_call(stream: u32) -> Vec<u8> {
        let mut v = headers(stream, &request_fields(VERSION), false);
        v.extend(message(stream, &[], true));
        v
    }
}

/// A compact account of what a server sent on a raw connection, to compare
/// two servers by: the server's own SETTINGS, SETTINGS ACKs, its PINGs
/// (grpc-go's BDP estimator sends one) and WINDOW_UPDATEs are left out;
/// PING ACKs are counted; HEADERS show their fields; GOAWAY its code, last
/// stream and debug text; then how the connection ended.
pub fn summarize(r: &Value) -> String {
    if let Some(e) = r.get("dial_error") {
        return format!("dial error: {e}");
    }
    let mut s = String::new();
    let mut acks = 0;
    for f in r["frames"].as_array().into_iter().flatten() {
        let ty = f["type"].as_str().unwrap_or("?");
        let stream = f["stream"].as_u64().unwrap_or(0);
        let ack = f["ack"].as_bool().unwrap_or(false);
        match ty {
            "SETTINGS" | "WINDOW_UPDATE" => continue,
            "PING" if ack => {
                acks += 1;
                continue;
            }
            "PING" => continue,
            _ => {}
        }
        if acks > 0 {
            let _ = write!(s, "PINGACKx{acks} ");
            acks = 0;
        }
        match ty {
            "HEADERS" => {
                let fields: Vec<String> = f["headers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|h| {
                        format!(
                            "{}={}",
                            h[0].as_str().unwrap_or(""),
                            h[1].as_str().unwrap_or("")
                        )
                    })
                    .collect();
                let end = if f["end_stream"].as_bool() == Some(true) {
                    " ES"
                } else {
                    ""
                };
                let _ = write!(s, "H{stream}[{}]{end} ", fields.join(","));
            }
            "DATA" => {
                let end = if f["end_stream"].as_bool() == Some(true) {
                    " ES"
                } else {
                    ""
                };
                let _ = write!(s, "D{stream}({}){end} ", f["length"]);
            }
            "RST_STREAM" => {
                let _ = write!(
                    s,
                    "RST{stream}({}) ",
                    f["error_code"].as_str().unwrap_or("")
                );
            }
            "GOAWAY" => {
                let _ = write!(
                    s,
                    "GOAWAY({},last={},{:?}) ",
                    f["error_code"].as_str().unwrap_or(""),
                    f["last_stream_id"].as_u64().unwrap_or(0),
                    f["debug"].as_str().unwrap_or("")
                );
            }
            other => {
                let _ = write!(s, "{other}{stream} ");
            }
        }
    }
    if acks > 0 {
        let _ = write!(s, "PINGACKx{acks} ");
    }
    let end = r["end"].as_str().unwrap_or("?");
    // A read error after the server closed (a reset) and EOF are the same
    // outcome for comparison.
    let end = if end == "eof" || end.contains("connection reset") || end.contains("broken pipe") {
        "closed"
    } else if end == "timeout" {
        "open"
    } else {
        end
    };
    let _ = write!(s, "| {end}");
    s
}

/// A raw-frame case: the frames after the preface and the client SETTINGS.
pub struct Case {
    /// Name.
    pub name: &'static str,
    /// Frames, each repeated.
    pub steps: Vec<(Vec<u8>, usize)>,
    /// How long to read.
    pub read_ms: u64,
}

fn case(name: &'static str, steps: Vec<(Vec<u8>, usize)>) -> Case {
    let mut all = vec![(h2::settings(&[]), 1)];
    all.extend(steps);
    Case {
        name,
        steps: all,
        read_ms: 1500,
    }
}

fn one(v: Vec<u8>) -> Vec<(Vec<u8>, usize)> {
    vec![(v, 1)]
}

fn fields_with(
    path: &'static str,
    change: impl FnOnce(&mut Vec<(&'static str, &'static str)>),
) -> Vec<(&'static str, &'static str)> {
    let mut f = h2::request_fields(path);
    change(&mut f);
    f
}

fn call_with(fields: &[(&str, &str)]) -> Vec<(Vec<u8>, usize)> {
    let mut v = h2::headers(1, fields, false);
    v.extend(h2::message(1, &[], true));
    one(v)
}

/// The raw cases: well-formed requests compared header by header, malformed
/// requests at the gRPC layer, and hostile HTTP/2 input.
pub fn raw_cases() -> Vec<Case> {
    use h2::*;
    let v = VERSION;
    let mut cases = vec![
        case("valid Version call", one(version_call(1))),
        case(
            "two calls, second on stream 3",
            one([version_call(1), version_call(3)].concat()),
        ),
        case(
            "HEADERS with END_STREAM, no message",
            one(headers(1, &request_fields(v), true)),
        ),
        case("truncated message", {
            let mut b = headers(1, &request_fields(v), false);
            b.extend(frame(0, 1, 1, &[0, 0, 0, 0, 10, 1, 2, 3]));
            one(b)
        }),
        case("message over 4 MiB declared", {
            let mut b = headers(1, &request_fields(v), false);
            b.extend(frame(0, 0, 1, &[0, 0, 0x50, 0, 0]));
            one(b)
        }),
        case("compressed flag, no grpc-encoding", {
            let mut b = headers(1, &request_fields(v), false);
            b.extend(frame(0, 1, 1, &[1, 0, 0, 0, 0]));
            one(b)
        }),
        case("payload format 2", {
            let mut b = headers(1, &request_fields(v), false);
            b.extend(frame(0, 1, 1, &[2, 0, 0, 0, 0]));
            one(b)
        }),
        case(
            "grpc-encoding gzip",
            call_with(&fields_with(v, |f| f.push(("grpc-encoding", "gzip")))),
        ),
        case(
            "malformed protobuf body",
            one([
                headers(1, &request_fields(v), false),
                message(1, &[0x0a, 0x05], true),
            ]
            .concat()),
        ),
        case(
            "content-type text/plain",
            call_with(&fields_with(v, |f| f[4] = ("content-type", "text/plain"))),
        ),
        case(
            "content-type application/grpc+proto",
            call_with(&fields_with(v, |f| {
                f[4] = ("content-type", "application/grpc+proto")
            })),
        ),
        case(
            "no :authority",
            call_with(&fields_with(v, |f| {
                f.remove(3);
            })),
        ),
        case(
            "two :authority",
            call_with(&fields_with(v, |f| f.insert(4, (":authority", "x")))),
        ),
        case(
            ":method GET",
            call_with(&fields_with(v, |f| f[0] = (":method", "GET"))),
        ),
        case(
            "connection header",
            call_with(&fields_with(v, |f| f.push(("connection", "keep-alive")))),
        ),
        case(
            "grpc-timeout malformed",
            call_with(&fields_with(v, |f| f.push(("grpc-timeout", "1x")))),
        ),
        case(
            "grpc-timeout zero",
            call_with(&fields_with(v, |f| f.push(("grpc-timeout", "0S")))),
        ),
        case("malformed path", call_with(&fields_with("nope", |_| {}))),
        case(
            "unknown service",
            call_with(&fields_with("/walletrpc.NoSuchService/Version", |_| {})),
        ),
        case(
            "unknown method",
            call_with(&fields_with("/walletrpc.VersionService/Nope", |_| {})),
        ),
        case(
            "uppercase header name",
            call_with(&fields_with(v, |f| f.push(("X-Upper", "1")))),
        ),
        case(
            "pseudo-header after regular",
            call_with(&fields_with(v, |f| f.push((":scheme", "https")))),
        ),
        case(
            "unknown pseudo-header",
            call_with(&fields_with(v, |f| f.insert(0, (":foo", "x")))),
        ),
        case("even stream id", one(version_call(2))),
        case(
            "decreasing stream id",
            one([version_call(5), version_call(3)].concat()),
        ),
        case("frame over MAX_FRAME_SIZE", {
            let mut b = header_only(0, 0, 1, 16_385);
            b.extend(vec![0u8; 16_385]);
            one(b)
        }),
        case("DATA on stream 0", one(frame(0, 0, 0, &[0; 5]))),
        case("DATA on an idle stream", one(frame(0, 0, 7, &[0; 5]))),
        case("HPACK index 0", one(frame(1, 0x5, 1, &[0x80]))),
        case(
            "HPACK index past the table",
            one(frame(1, 0x5, 1, &[0xff, 0x7f])),
        ),
        case(
            "HPACK size update above 4096",
            one(frame(1, 0x5, 1, &[0x3f, 0xe2, 0x1f])),
        ),
        case(
            "HPACK size update after a field",
            one(frame(1, 0x5, 1, &[0x82, 0x20])),
        ),
        case("Huffman EOS in a string", {
            // Literal without indexing, new name "a", Huffman value = EOS.
            one(frame(
                1,
                0x5,
                1,
                &[0x00, 0x01, b'a', 0x84, 0xff, 0xff, 0xff, 0xff],
            ))
        }),
        case("Huffman padding not all ones", {
            one(frame(1, 0x5, 1, &[0x00, 0x01, b'a', 0x81, 0b0001_1110]))
        }),
        case("Huffman padding of 8 bits", {
            one(frame(
                1,
                0x5,
                1,
                &[0x00, 0x01, b'a', 0x82, 0b0001_1111, 0xff],
            ))
        }),
        case("HPACK bomb: 16 KiB of references to a 4 KiB entry", {
            let mut b = vec![0x40];
            push_str(&mut b, b"bomb");
            push_str(&mut b, &vec![b'x'; 4000]);
            let mut blk = block(&request_fields(v));
            blk.extend(b);
            blk.extend(std::iter::repeat_n(0xbe, 16_000 - blk.len()));
            let mut out = frame(1, 0, 1, &blk);
            for _ in 0..4 {
                out.extend(frame(9, 0, 1, &[0xbe; 16_000]));
            }
            // END_HEADERS on a last, short CONTINUATION, then the message.
            out.extend(frame(9, 0x4, 1, &[0xbe]));
            out.extend(message(1, &[], true));
            one(out)
        }),
        case("CONTINUATION flood (16 KiB frames, 4 MiB)", {
            let mut b = frame(1, 0, 1, &block(&request_fields(v)));
            b.extend(std::iter::repeat_n(frame(9, 0, 1, &vec![0x82; 16_000]), 256).flatten());
            one(b)
        }),
        case("CONTINUATION flood (10000 empty frames)", {
            let mut s = one(frame(1, 0, 1, &block(&request_fields(v))));
            s.push((frame(9, 0, 1, &[]), 10_000));
            s
        }),
        case("CONTINUATION on another stream", {
            let mut b = frame(1, 0, 1, &block(&request_fields(v)));
            b.extend(frame(9, 0x4, 3, &[]));
            one(b)
        }),
        case("CONTINUATION without HEADERS", one(frame(9, 0x4, 1, &[]))),
        case("SETTINGS flood (10000)", vec![(settings(&[]), 10_000)]),
        case("SETTINGS length 5", one(frame(4, 0, 0, &[0; 5]))),
        case("SETTINGS ENABLE_PUSH 2", one(settings(&[(2, 2)]))),
        case(
            "SETTINGS INITIAL_WINDOW_SIZE 2^31",
            one(settings(&[(4, 1 << 31)])),
        ),
        case("SETTINGS MAX_FRAME_SIZE 100", one(settings(&[(5, 100)]))),
        case("SETTINGS ACK with payload", one(frame(4, 1, 0, &[0; 6]))),
        case("PING flood (1000)", vec![(frame(6, 0, 0, &[1; 8]), 1000)]),
        case("PING length 7", one(frame(6, 0, 0, &[1; 7]))),
        case("PING on a stream", one(frame(6, 0, 1, &[1; 8]))),
        case(
            "PING after a call (permitted)",
            one([version_call(1), frame(6, 0, 0, &[2; 8])].concat()),
        ),
        case(
            "WINDOW_UPDATE 0 on the connection",
            one(frame(8, 0, 0, &[0; 4])),
        ),
        case(
            "WINDOW_UPDATE overflow on the connection",
            one(frame(8, 0, 0, &0x7fff_ffffu32.to_be_bytes())),
        ),
        case(
            "RST_STREAM on an idle stream",
            one(frame(3, 0, 5, &[0, 0, 0, 8])),
        ),
        case(
            "PUSH_PROMISE from the client",
            one(frame(5, 0x4, 1, &[0, 0, 0, 2])),
        ),
        case("rapid reset (2000 streams)", {
            let mut b = Vec::new();
            for i in 0..2000u32 {
                let id = 2 * i + 1;
                b.extend(headers(id, &request_fields(v), false));
                b.extend(frame(3, 0, id, &8u32.to_be_bytes()));
            }
            one(b)
        }),
        case("DATA past the connection window", {
            let mut b = headers(1, &request_fields(v), false);
            for _ in 0..5 {
                b.extend(frame(0, 0, 1, &vec![0u8; 16_000]));
            }
            one(b)
        }),
        case("DATA that is all padding (valid)", {
            let mut b = headers(1, &request_fields(v), false);
            b.extend(frame(0, 0x8, 1, &[5, 0, 0, 0, 0, 0]));
            one(b)
        }),
        case("HEADERS depending on itself", {
            let mut p = vec![0, 0, 0, 1, 16];
            p.extend(block(&request_fields(v)));
            one(frame(1, 0x4 | 0x20, 1, &p))
        }),
        case(
            "unknown frame type, then a call",
            one([frame(0xfa, 0, 0, &[1, 2, 3]), version_call(1)].concat()),
        ),
    ];
    cases.push(Case {
        name: "bad preface",
        steps: vec![],
        read_ms: 1500,
    });
    cases
}

/// The raw cases on which the in-tree server is held to grpc-go's output
/// (compared after [`without_clean_resets`]). The others differ by design
/// (a GOAWAY before closing, RFC 9113's stricter validity checks, the
/// spike's flood limits) or by timing (stream order, a reply racing the
/// client's later DATA, a reply racing grpc-go's GOAWAY for a decreasing
/// stream id); the ADR lists each.
pub const INTREE_PARITY: &[&str] = &[
    "valid Version call",
    "truncated message",
    "message over 4 MiB declared",
    "compressed flag, no grpc-encoding",
    "payload format 2",
    "grpc-encoding gzip",
    "content-type text/plain",
    "content-type application/grpc+proto",
    "no :authority",
    "two :authority",
    ":method GET",
    "connection header",
    "grpc-timeout malformed",
    "grpc-timeout zero",
    "malformed path",
    "unknown service",
    "unknown method",
    "uppercase header name",
    "pseudo-header after regular",
    "unknown pseudo-header",
    "even stream id",
    "CONTINUATION flood (10000 empty frames)",
    "SETTINGS flood (10000)",
    "PING flood (1000)",
    "PING after a call (permitted)",
    "DATA that is all padding (valid)",
    "unknown frame type, then a call",
    "bad preface",
];

/// A summary without `RST_STREAM(NO_ERROR)`, which grpc-go sends or not
/// after a status depending on whether its reader saw the client's
/// END_STREAM before the stream's goroutine wrote the status.
pub fn without_clean_resets(summary: &str) -> String {
    summary
        .split(' ')
        .filter(|t| !(t.starts_with("RST") && t.ends_with("(NO_ERROR)")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The bad-preface case is sent without the oracle's preface.
pub fn run_case(h: &mut Harness, addr: &str, c: &Case) -> Value {
    if c.name == "bad preface" {
        return h.run(
            "raw",
            addr,
            "client",
            json!({"preface": false, "steps": [{"hex": hex(b"GET / HTTP/1.1\r\n\r\n")}], "read_ms": c.read_ms}),
        );
    }
    h.raw(addr, &c.steps, c.read_ms)
}

/// Whether a server still answers a Version call.
pub fn alive(h: &mut Harness, addr: &str) -> bool {
    let r = h.run("version", addr, "client", json!({}));
    r["reply"]["version_string"] == "10.0.0"
}

/// The status of a grpc-go call result: (code, message).
pub fn status(r: &Value) -> (u64, String) {
    match r.get("status") {
        Some(s) => (
            s["code"].as_u64().unwrap_or(99),
            s["message"].as_str().unwrap_or("").to_owned(),
        ),
        None => (0, String::new()),
    }
}

/// Strips addresses and ports from a message so runs compare.
pub fn normalize(msg: &str) -> String {
    let mut out = String::new();
    for word in msg.split(' ') {
        if word.contains("127.0.0.1:") {
            out.push_str("<addr>");
        } else {
            out.push_str(word);
        }
        out.push(' ');
    }
    out.trim_end().to_owned()
}

/// The grpc-go-client cases: (name, client identity, extra args, command).
pub fn client_cases() -> Vec<(&'static str, &'static str, Value, &'static str)> {
    vec![
        (
            "issued client certificate, TLS 1.3",
            "client",
            json!({}),
            "version",
        ),
        (
            "issued client certificate, TLS 1.2",
            "client",
            json!({"tls_max": "1.2"}),
            "version",
        ),
        (
            "self-signed client certificate in clients.pem",
            "selfsigned",
            json!({}),
            "version",
        ),
        (
            "self-signed CA-flagged client certificate in clients.pem",
            "selfsigned-ca",
            json!({}),
            "version",
        ),
        ("no client certificate", "", json!({}), "version"),
        (
            "client certificate from another CA",
            "wrongca-client",
            json!({}),
            "version",
        ),
        (
            "expired client certificate",
            "expired-client",
            json!({}),
            "version",
        ),
        (
            "plaintext (insecure credentials)",
            "",
            json!({"insecure": true}),
            "version",
        ),
        ("HTTP/1.1 over TLS", "client", json!({}), "http1"),
        (
            "unknown service",
            "client",
            json!({"method": "/walletrpc.NoSuchService/Version"}),
            "call",
        ),
        (
            "unknown method",
            "client",
            json!({"method": "/walletrpc.VersionService/NoSuchMethod"}),
            "call",
        ),
        (
            "malformed request message",
            "client",
            json!({"method": "/walletrpc.VersionService/Version", "body_hex": "0a05"}),
            "call",
        ),
        (
            "request with metadata and a deadline",
            "client",
            json!({"method": "/walletrpc.VersionService/Version", "metadata": {"x-test": "1"}, "timeout_ms": 3000}),
            "call",
        ),
    ]
}

/// One line describing a grpc-go client result.
pub fn describe_client(cmd: &str, r: &Value) -> String {
    match cmd {
        "http1" => match r.get("error") {
            Some(e) => format!("error: {}", normalize(e.as_str().unwrap_or(""))),
            None => format!("HTTP {} {}", r["status"], r["proto"]),
        },
        _ => {
            let (code, msg) = status(r);
            let mut s = if code == 0 {
                match r.get("reply") {
                    Some(reply) => format!("OK {reply}"),
                    None => format!("OK reply_hex={}", r["reply_hex"]),
                }
            } else {
                format!("code {code}: {}", normalize(&msg))
            };
            if let Some(t) = r.get("tls").filter(|t| !t.is_null()) {
                let _ = write!(
                    s,
                    " [{} {} {} alpn={}]",
                    t["version"].as_str().unwrap_or(""),
                    t["cipher_suite"].as_str().unwrap_or(""),
                    t["curve"].as_str().unwrap_or(""),
                    t["negotiated_protocol"].as_str().unwrap_or("")
                );
            }
            if let Some(h) = r
                .get("headers")
                .filter(|h| h.as_object().is_some_and(|o| !o.is_empty()))
            {
                let _ = write!(s, " headers={h}");
            }
            if let Some(t) = r
                .get("trailers")
                .filter(|h| h.as_object().is_some_and(|o| !o.is_empty()))
            {
                let _ = write!(s, " trailers={t}");
            }
            s
        }
    }
}

/// I/O-level cases: what one peer can do to the others through the
/// listener, its threads and its writes, rather than through the HTTP/2
/// parser. Each takes the server's address; the callers add what only
/// their server can report.
pub mod io {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use super::Harness;

    /// The process's thread count (Linux only; 0 elsewhere).
    pub fn threads() -> usize {
        std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix("Threads:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Starts an authenticated client that writes `calls` requests and
    /// never reads, then makes `victims` Version calls on fresh connections
    /// with a `victim_ms` deadline each. Returns the slow reader's id, how
    /// many requests it got in, and each victim's outcome (`Ok(µs)` or the
    /// status message).
    pub fn slow_reader(
        h: &mut Harness,
        addr: &str,
        calls: usize,
        victims: usize,
        victim_ms: u64,
    ) -> (u64, u64, Vec<Result<u128, String>>) {
        let r = h.run(
            "slowread",
            addr,
            "client",
            json!({"calls": calls, "rcvbuf": 4096}),
        );
        let id = r["id"].as_u64().expect("slow reader id");
        let written = r["written"].as_u64().unwrap_or(0);
        let mut out = Vec::new();
        for _ in 0..victims {
            let t = Instant::now();
            let v = h.run(
                "version",
                addr,
                "client",
                json!({"n": 0, "timeout_ms": victim_ms}),
            );
            out.push(if v["reply"]["version_string"] == "10.0.0" {
                Ok(t.elapsed().as_micros())
            } else {
                Err(super::status(&v).1)
            });
        }
        (id, written, out)
    }

    /// Ends a slow reader: (requests written, how its connection ended).
    pub fn slow_reader_stop(h: &mut Harness, id: u64) -> (u64, String) {
        let r = h.oracle.call("slowread.stop", json!({"id": id}));
        (
            r["written"].as_u64().unwrap_or(0),
            r["end"].as_str().unwrap_or("?").to_owned(),
        )
    }

    /// Opens `n` TCP connections that never send a byte, and returns them
    /// with the change in this process's thread count once they are
    /// accepted.
    pub fn idle_sockets(addr: &str, n: usize) -> (Vec<TcpStream>, isize) {
        let before = threads() as isize;
        let socks: Vec<TcpStream> = (0..n)
            .filter_map(|_| TcpStream::connect(addr).ok())
            .collect();
        std::thread::sleep(Duration::from_millis(500));
        let after = threads() as isize;
        (socks, after - before)
    }

    /// Sends a TLS ClientHello's first bytes one at a time every `every`
    /// for up to `total`, and returns when the server closed the connection
    /// (counted from the first byte), or `None` if it never did.
    pub fn trickle(addr: &str, every: Duration, total: Duration) -> Option<Duration> {
        let mut s = TcpStream::connect(addr).ok()?;
        s.set_read_timeout(Some(every)).ok()?;
        // A TLS record header and the start of a ClientHello.
        let hello = [
            0x16u8, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01, 0xfc, 0x03, 0x03,
        ];
        let start = Instant::now();
        let mut i = 0;
        let mut buf = [0u8; 64];
        while start.elapsed() < total {
            if s.write_all(&[hello[i % hello.len()]]).is_err() {
                return Some(start.elapsed());
            }
            i += 1;
            match s.read(&mut buf) {
                Ok(0) => return Some(start.elapsed()),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return Some(start.elapsed()),
            }
        }
        None
    }

    /// The status codes counted by a `bigcalls` or `cancels` result.
    pub fn codes(r: &Value) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = r["codes"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, n)| (k.clone(), n.as_u64().unwrap_or(0)))
            .collect();
        v.sort();
        v
    }
}
