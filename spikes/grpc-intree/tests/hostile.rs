//! The in-tree HTTP/2 state machine against hostile input, in memory: a
//! mutation loop over valid sessions (no panic, bounded memory, bounded
//! streams), and one check per known attack class with the error it must
//! end in.
//!
//! `PURSE_TEST_SEED` replays a run; `HOSTILE_ITERATIONS` lengthens it.

mod common;

use std::time::{Duration, Instant};

use common::h2;
use grpc_intree_spike::conn::{Connection, Event, Limits};
use grpc_intree_spike::frame::{self, ErrorCode};
use grpc_intree_spike::grpc::{Status, code};
use grpc_intree_spike::version_router;
use purse_testutil::SplitMix64;

fn small_limits() -> Limits {
    Limits {
        max_header_list_size: 64 << 10,
        max_header_frames: 64,
        max_streams: 64,
        max_buffered: 1 << 20,
        max_inflight: 8,
        max_reset_score: 100,
        ..Limits::default()
    }
}

/// Answers `events`, and the calls each answer lets through, with
/// `result`.
fn answer_all(
    c: &mut Connection,
    events: Vec<Event>,
    result: impl Fn() -> Result<Vec<u8>, Status>,
) -> usize {
    let mut queue: std::collections::VecDeque<Event> = events.into();
    let mut n = 0;
    while let Some(ev) = queue.pop_front() {
        let Event::Call { stream, .. } = ev;
        n += 1;
        queue.extend(c.respond(stream, result()));
    }
    n
}

/// Feeds `input` after the preface and a SETTINGS frame, answering every
/// call; returns the connection and every byte it wrote.
fn session(limits: Limits, input: &[u8]) -> (Connection, Vec<u8>) {
    let router = version_router();
    let mut c = Connection::new(limits);
    let mut all = frame::PREFACE.to_vec();
    all.extend(h2::settings(&[]));
    all.extend_from_slice(input);
    let mut out = c.take_output();
    let now = Instant::now();
    let events = c.recv(&all, &router, now);
    answer_all(&mut c, events, || {
        Ok(grpc_intree_spike::wallet_version().encode())
    });
    out.extend(c.take_output());
    (c, out)
}

/// Frames the server wrote: (type, flags, stream, payload).
fn frames_full(out: &[u8]) -> Vec<(u8, u8, u32, Vec<u8>)> {
    let mut v = Vec::new();
    let mut p = 0;
    while p + 9 <= out.len() {
        let len = u32::from_be_bytes([0, out[p], out[p + 1], out[p + 2]]) as usize;
        let stream = u32::from_be_bytes([out[p + 5], out[p + 6], out[p + 7], out[p + 8]]);
        v.push((
            out[p + 3],
            out[p + 4],
            stream,
            out[p + 9..p + 9 + len].to_vec(),
        ));
        p += 9 + len;
    }
    v
}

/// Frames the server wrote: (type, stream, first payload word).
fn frames(out: &[u8]) -> Vec<(u8, u32, u32)> {
    let mut v = Vec::new();
    let mut p = 0;
    while p + 9 <= out.len() {
        let len = u32::from_be_bytes([0, out[p], out[p + 1], out[p + 2]]) as usize;
        let ty = out[p + 3];
        let stream = u32::from_be_bytes([out[p + 5], out[p + 6], out[p + 7], out[p + 8]]);
        let pl = &out[p + 9..p + 9 + len];
        let word = match ty {
            3 => u32::from_be_bytes([pl[0], pl[1], pl[2], pl[3]]),
            7 => u32::from_be_bytes([pl[4], pl[5], pl[6], pl[7]]),
            _ => 0,
        };
        v.push((ty, stream, word));
        p += 9 + len;
    }
    v
}

fn closed_with(c: &Connection, code: ErrorCode) {
    assert!(c.is_closed(), "connection should be closed");
    assert_eq!(
        c.close_reason.as_ref().map(|r| r.0),
        Some(code),
        "{:?}",
        c.close_reason
    );
}

#[test]
fn oversized_frame_is_a_frame_size_error() {
    let mut b = h2::header_only(0, 0, 1, 16_385);
    b.extend(vec![0; 16_385]);
    let (c, _) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::FrameSize);
}

#[test]
fn continuation_flood_is_bounded() {
    // Large CONTINUATIONs of indexed fields: kept until the 16 MiB budget is
    // spent, then the next non-empty fragment closes the connection, as
    // x/net's readMetaFrame does (PROTOCOL_ERROR).
    let mut b = h2::frame(1, 0, 1, &h2::block(&h2::request_fields(h2::VERSION)));
    // 0x90: the static entry accept-encoding: gzip, deflate.
    for _ in 0..1000 {
        b.extend(h2::frame(9, 0, 1, &[0x90; 16_000]));
    }
    let (c, _) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::Protocol);
    assert_eq!(c.close_reason.as_ref().unwrap().1, "header list too large");
    // Empty CONTINUATIONs: x/net accepts any number; the frame cap (one per
    // KiB of budget, 16,384) ends the run.
    let mut b = h2::frame(1, 0, 1, &h2::block(&h2::request_fields(h2::VERSION)));
    for _ in 0..10_000 {
        b.extend(h2::frame(9, 0, 1, &[]));
    }
    let (c, _) = session(Limits::default(), &b);
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    for _ in 0..10_000 {
        b.extend(h2::frame(9, 0, 1, &[]));
    }
    let (c, _) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::EnhanceYourCalm);
}

#[test]
fn large_valid_headers_are_accepted_as_grpc_go_accepts_them() {
    // A 3,000,000-byte metadata value in 16 KiB fragments (what grpc-go's
    // client sends), well inside the 16 MiB budget: the call is answered.
    let mut blk = h2::block(&h2::request_fields(h2::VERSION));
    blk.push(0);
    h2::push_str(&mut blk, b"x-big");
    h2::push_str(&mut blk, &vec![b'a'; 3_000_000]);
    let chunks: Vec<&[u8]> = blk.chunks(16_384).collect();
    let mut b = Vec::new();
    for (i, ch) in chunks.iter().enumerate() {
        let kind = if i == 0 { 1 } else { 9 };
        let flags = if i + 1 == chunks.len() { 0x4 } else { 0 };
        b.extend(h2::frame(kind, flags, 1, ch));
    }
    b.extend(h2::message(1, &[], true));
    let (c, out) = session(Limits::default(), &b);
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    assert!(
        frames_full(&out)
            .iter()
            .any(|f| f.0 == 1 && f.1 & 1 != 0 && f.2 == 1),
        "the call is answered"
    );
    // A small block split into 300 fragments (past the old 128-frame cap).
    let blk = h2::block(&h2::request_fields(h2::VERSION));
    let mut b = h2::frame(1, 0, 3, &blk[..1]);
    for i in 1..300 {
        let piece = if i < blk.len() {
            &blk[i..i + 1]
        } else {
            &[][..]
        };
        b.extend(h2::frame(9, 0, 3, piece));
    }
    b.extend(h2::frame(9, 0x4, 3, &blk[300.min(blk.len())..]));
    b.extend(h2::message(3, &[], true));
    let (c, out) = session(Limits::default(), &b);
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    assert!(
        frames_full(&out)
            .iter()
            .any(|f| f.0 == 1 && f.1 & 1 != 0 && f.2 == 3)
    );
}

#[test]
fn hpack_bomb_is_bounded_by_the_header_list_budget() {
    // 800,000 references to a 4 KiB entry (3.2 GB if every one were
    // built), in 51 fragments. Fields are kept only up to the 16 MiB
    // header-list budget, grpc-go's own bound; the next non-empty fragment
    // closes the connection, as x/net's readMetaFrame closes it.
    // held_bytes measures what the connection retains between reads, not
    // the peak inside one read, which is bounded by the same budget.
    let mut blk = h2::block(&h2::request_fields(h2::VERSION));
    blk.push(0x40);
    h2::push_str(&mut blk, b"bomb");
    h2::push_str(&mut blk, &[b'x'; 4000]);
    blk.extend(std::iter::repeat_n(0xbe, 16_000 - blk.len()));
    let mut b = h2::frame(1, 0, 1, &blk);
    for _ in 0..50 {
        b.extend(h2::frame(9, 0, 1, &[0xbe; 16_000]));
    }
    b.extend(h2::frame(9, 0x4, 1, &[0xbe]));
    let (c, _) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::Protocol);
    assert!(c.held_bytes() < 64 << 10);
    // The same references in one fragment that ends the block: the list is
    // over budget but the block is complete, so grpc-go's answer, a stream
    // error, and the connection stays.
    let (c, out) = session(Limits::default(), &h2::frame(1, 0x4, 1, &blk));
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    assert!(frames(&out).contains(&(3, 1, ErrorCode::FrameSize as u32)));
    assert!(c.held_bytes() < 64 << 10);
}

#[test]
fn settings_flood_costs_output_proportional_to_input() {
    let b: Vec<u8> = std::iter::repeat_n(h2::settings(&[]), 100_000)
        .flatten()
        .collect();
    let (c, out) = session(Limits::default(), &b);
    assert!(!c.is_closed());
    // One 9-byte ACK per SETTINGS (plus the server's own and the first ACK).
    assert!(out.len() <= 9 * 100_002 + 15);
}

#[test]
fn ping_flood_closes_on_the_third_strike() {
    let b: Vec<u8> = std::iter::repeat_n(h2::frame(6, 0, 0, &[1; 8]), 1000)
        .flatten()
        .collect();
    let (c, out) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::EnhanceYourCalm);
    let acks = frames(&out).iter().filter(|f| f.0 == 6).count();
    assert_eq!(acks, 4, "grpc-go acknowledges four pings before its GOAWAY");
}

#[test]
fn ping_policy_resets_after_the_server_writes() {
    let router = version_router();
    let mut c = Connection::new(Limits::default());
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[]));
    let t0 = Instant::now();
    c.recv(&b, &router, t0);
    for i in 0..10u32 {
        // A call (the server writes HEADERS and DATA), then a ping.
        let id = 2 * i + 1;
        let events = c.recv(&h2::version_call(id), &router, t0);
        answer_all(&mut c, events, || Ok(vec![]));
        c.recv(&h2::frame(6, 0, 0, &[1; 8]), &router, t0);
    }
    assert!(!c.is_closed());
    // Two hours apart with no stream open: no strike.
    let mut t = t0;
    for _ in 0..5 {
        t += Duration::from_secs(2 * 3600 + 1);
        c.recv(&h2::frame(6, 0, 0, &[1; 8]), &router, t);
    }
    assert!(!c.is_closed());
}

#[test]
fn rapid_reset_is_cut_off() {
    let mut b = Vec::new();
    for i in 0..1100u32 {
        let id = 2 * i + 1;
        b.extend(h2::headers(id, &h2::request_fields(h2::VERSION), false));
        b.extend(h2::frame(3, 0, id, &8u32.to_be_bytes()));
    }
    let (c, _) = session(Limits::default(), &b);
    closed_with(&c, ErrorCode::EnhanceYourCalm);
}

#[test]
fn completed_calls_balance_resets() {
    // A client that cancels some calls and completes others stays.
    let router = version_router();
    let mut c = Connection::new(Limits::default());
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[]));
    c.recv(&b, &router, Instant::now());
    for i in 0..3000u32 {
        let id = 2 * i + 1;
        if i % 2 == 0 {
            let mut x = h2::headers(id, &h2::request_fields(h2::VERSION), false);
            x.extend(h2::frame(3, 0, id, &8u32.to_be_bytes()));
            c.recv(&x, &router, Instant::now());
        } else {
            let events = c.recv(&h2::version_call(id), &router, Instant::now());
            answer_all(&mut c, events, || Ok(vec![]));
        }
        c.take_output();
    }
    assert!(!c.is_closed(), "{:?}", c.close_reason);
}

#[test]
fn stream_id_violations_are_protocol_errors() {
    for b in [
        h2::version_call(2),
        [h2::version_call(5), h2::version_call(3)].concat(),
    ] {
        let (c, _) = session(Limits::default(), &b);
        closed_with(&c, ErrorCode::Protocol);
    }
}

#[test]
fn hpack_and_huffman_errors_are_compression_errors() {
    for blk in [
        &[0x80][..],
        &[0xff, 0x7f],
        &[0x3f, 0xe2, 0x1f],
        &[0x00, 0x01, b'a', 0x84, 0xff, 0xff, 0xff, 0xff],
        &[0x00, 0x01, b'a', 0x81, 0b0001_1110],
        &[0x00, 0x01, b'a', 0x82, 0b0001_1111, 0xff],
        &[0x00, 0x7f], // truncated name length
    ] {
        let (c, _) = session(Limits::default(), &h2::frame(1, 0x5, 1, blk));
        closed_with(&c, ErrorCode::Compression);
    }
}

#[test]
fn reset_of_a_dispatched_call_is_a_cancellation_not_a_strike() {
    // 1,500 complete calls whose handlers have not answered, then all
    // cancelled (RST_STREAM CANCEL), as a client whose deadlines expire
    // while handlers are slow does. grpc-go never closes for this.
    let router = version_router();
    let mut c = Connection::new(Limits::default());
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[]));
    let now = Instant::now();
    c.recv(&b, &router, now);
    let mut held = Vec::new();
    for i in 0..1500u32 {
        held.extend(c.recv(&h2::version_call(2 * i + 1), &router, now));
    }
    assert_eq!(held.len(), Limits::default().max_inflight);
    assert_eq!(c.queued_calls(), 1500 - held.len());
    for i in 0..1500u32 {
        c.recv(
            &h2::frame(3, 0, 2 * i + 1, &8u32.to_be_bytes()),
            &router,
            now,
        );
    }
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    assert_eq!(c.queued_calls(), 0);
    // The running handlers still hold their slots until they answer.
    assert_eq!(c.inflight_calls(), held.len());
    let n = answer_all(&mut c, held, || Ok(vec![]));
    assert_eq!(n, Limits::default().max_inflight);
    assert_eq!(c.inflight_calls(), 0);
    assert!(!c.is_closed());
}

#[test]
fn reset_flood_with_stuck_handlers_is_bounded_by_the_inflight_limit() {
    // Complete requests reset at once, handlers that never return: the
    // handler work started stays at max_inflight, everything past it waits
    // in the connection and is dropped on reset, and memory stays bounded.
    let limits = small_limits();
    let router = version_router();
    let mut c = Connection::new(limits.clone());
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[]));
    let now = Instant::now();
    c.recv(&b, &router, now);
    let mut started = 0;
    for i in 0..20_000u32 {
        let id = 2 * i + 1;
        let mut x = h2::version_call(id);
        x.extend(h2::frame(3, 0, id, &8u32.to_be_bytes()));
        started += c.recv(&x, &router, now).len();
        c.take_output();
    }
    assert!(!c.is_closed(), "{:?}", c.close_reason);
    assert_eq!(started, limits.max_inflight);
    assert_eq!(c.inflight_calls(), limits.max_inflight);
    assert!(c.open_streams() <= limits.max_streams);
    assert!(c.held_bytes() < 64 << 10, "{}", c.held_bytes());
}

#[test]
fn settings_advertise_the_stream_cap() {
    let c_out = Connection::new(Limits::default()).take_output();
    let f = &frames_full(&c_out)[0];
    assert_eq!(f.0, 4);
    let mut settings = Vec::new();
    for ch in f.3.chunks(6) {
        settings.push((
            u16::from_be_bytes([ch[0], ch[1]]),
            u32::from_be_bytes([ch[2], ch[3], ch[4], ch[5]]),
        ));
    }
    assert_eq!(settings, vec![(5, 16_384), (3, 4096)]);
}

/// A client that respects flow control: it opens `n` streams each sending
/// one message of `size` bytes, writes one frame of up to 16 KiB per stream
/// in turn (grpc-go's loopy writer's round robin), and reads the server's
/// WINDOW_UPDATEs. Handlers answer `delay` rounds after their call
/// arrives. Returns the calls answered OK and the most the connection held.
fn fair_sender(limits: Limits, n: u32, size: usize, delay: usize) -> (usize, usize, Connection) {
    let router = version_router();
    let mut c = Connection::new(limits);
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[]));
    let now = Instant::now();
    c.recv(&b, &router, now);
    c.take_output();
    // A VersionRequest carrying one unknown length-delimited field (15),
    // which the decoder skips: `size` bytes in all.
    let body = |size: usize| {
        let mut m = vec![0x7a];
        let mut len = size - 1;
        let mut v = Vec::new();
        loop {
            let mut byte = (len & 0x7f) as u8;
            len >>= 7;
            if len > 0 {
                byte |= 0x80;
            }
            v.push(byte);
            if len == 0 {
                break;
            }
        }
        let payload = size - 1 - v.len();
        m.extend(v);
        m.resize(1 + (m.len() - 1) + payload, 0);
        let mut framed = vec![0];
        framed.extend((m.len() as u32).to_be_bytes());
        framed.extend(m);
        framed
    };
    let mut conn_window: i64 = 65_535;
    let mut streams: Vec<(u32, Vec<u8>, usize, i64)> = Vec::new();
    for i in 0..n {
        let id = 2 * i + 1;
        let input = h2::headers(id, &h2::request_fields(h2::VERSION), false);
        let ev = c.recv(&input, &router, now);
        assert!(ev.is_empty());
        streams.push((id, body(size), 0, 65_535));
    }
    let mut answered = 0usize;
    let mut max_held = 0usize;
    let mut pending: Vec<(usize, Event)> = Vec::new();
    let mut round = 0usize;
    loop {
        round += 1;
        let mut sent = false;
        let mut input = Vec::new();
        for s in streams.iter_mut() {
            let left = s.1.len() - s.2;
            if left == 0 {
                continue;
            }
            let k = left
                .min(16_384)
                .min(conn_window.max(0) as usize)
                .min(s.3.max(0) as usize);
            if k == 0 {
                continue;
            }
            let end = s.2 + k == s.1.len();
            input.extend(h2::frame(0, u8::from(end), s.0, &s.1[s.2..s.2 + k]));
            s.2 += k;
            s.3 -= k as i64;
            conn_window -= k as i64;
            sent = true;
        }
        let mut events = c.recv(&input, &router, now);
        let mut due = Vec::new();
        // Answer the calls whose delay has passed.
        let mut keep = Vec::new();
        for (r, ev) in std::mem::take(&mut pending) {
            if r + delay <= round {
                due.push(ev);
            } else {
                keep.push((r, ev));
            }
        }
        pending = keep;
        for ev in due {
            let Event::Call { stream, .. } = ev;
            events.extend(c.respond(stream, Ok(vec![])));
        }
        let had_events = !events.is_empty();
        for ev in events {
            pending.push((round, ev));
        }
        max_held = max_held.max(c.held_bytes());
        let mut credit = false;
        for f in frames_full(&c.take_output()) {
            match f.0 {
                8 => {
                    credit = true;
                    let inc = i64::from(u32::from_be_bytes([f.3[0], f.3[1], f.3[2], f.3[3]]));
                    if f.2 == 0 {
                        conn_window += inc;
                    } else if let Some(s) = streams.iter_mut().find(|s| s.0 == f.2) {
                        s.3 += inc;
                    }
                }
                1 if f.1 & 1 != 0 => answered += 1,
                3 => panic!("stream {} reset", f.2),
                7 => panic!("GOAWAY {:?}", c.close_reason),
                _ => {}
            }
        }
        if answered == n as usize {
            return (answered, max_held, c);
        }
        assert!(
            sent || had_events || credit || !pending.is_empty(),
            "wedged after {round} rounds: {answered} of {n} answered, held {}",
            c.held_bytes()
        );
        assert!(round < 1_000_000);
    }
}

#[test]
fn flow_control_cannot_wedge_and_request_memory_stays_bounded() {
    // 200 streams of 60 KB against a 1 MiB budget (12 MB in all), sent
    // round robin: before admission existed this wedged once 1 MiB of
    // partial messages was held. Every call must finish, with the
    // connection holding at most the budget plus one window per stream.
    let limits = Limits {
        max_buffered: 1 << 20,
        ..Limits::default()
    };
    let (answered, max_held, _) = fair_sender(limits.clone(), 200, 60_000, 0);
    assert_eq!(answered, 200);
    let bound = limits.max_buffered + 200 * 65_535;
    assert!(max_held <= bound, "held {max_held} > {bound}");
    // Large messages and slow handlers: 20 streams of 4,000,000 bytes on a
    // 16 MiB budget, each handler answering ten rounds late, so admitted
    // bytes are held by handlers, not by the connection's buffers.
    let limits = Limits {
        max_buffered: 16 << 20,
        ..Limits::default()
    };
    let (answered, max_held, _) = fair_sender(limits.clone(), 20, 4_000_000, 10);
    assert_eq!(answered, 20);
    let bound = limits.max_buffered + 20 * 65_535;
    assert!(max_held <= bound, "held {max_held} > {bound}");
}

#[test]
fn dispatched_requests_stay_charged_until_answered() {
    // Handlers that never answer: complete 1 MiB requests are charged to
    // the budget until their response, so after 4 MiB (the budget) further
    // streams are not admitted and get no window back.
    let limits = Limits {
        max_buffered: 4 << 20,
        max_inflight: 64,
        ..Limits::default()
    };
    let router = version_router();
    let mut c = Connection::new(limits.clone());
    let mut b = frame::PREFACE.to_vec();
    b.extend(h2::settings(&[(4, 0x7fff_ffff)]));
    let now = Instant::now();
    c.recv(&b, &router, now);
    let mut calls = 0;
    // The client pretends every window is open, as an attacker would; the
    // server answers over-window data with a stream error.
    for i in 0..40u32 {
        let id = 2 * i + 1;
        let mut x = h2::headers(id, &h2::request_fields(h2::VERSION), false);
        let mut msg = vec![0u8, 0, 0x10, 0, 0];
        msg.extend(vec![0u8; 16_379]);
        x.extend(h2::frame(0, 0, id, &msg));
        for _ in 0..63 {
            x.extend(h2::frame(0, 0, id, &[0u8; 16_384]));
        }
        x.extend(h2::frame(0, 1, id, &[0u8; 5]));
        calls += c.recv(&x, &router, now).len();
        c.take_output();
        assert!(c.held_bytes() <= limits.max_buffered + 40 * 65_535 + 16_393);
    }
    assert!(calls <= 4, "{calls} calls dispatched on a 4 MiB budget");
}

/// A pool of frames to build sessions from: valid ones and edge cases.
fn pool() -> Vec<Vec<u8>> {
    let v = h2::VERSION;
    let mut p = vec![
        h2::settings(&[]),
        h2::settings(&[(4, 1 << 20), (5, 1 << 20), (1, 0)]),
        h2::frame(4, 1, 0, &[]),
        h2::frame(6, 0, 0, &[7; 8]),
        h2::frame(6, 1, 0, &[7; 8]),
        h2::frame(8, 0, 0, &1000u32.to_be_bytes()),
        h2::frame(8, 0, 1, &1000u32.to_be_bytes()),
        h2::frame(2, 0, 1, &[0, 0, 0, 0, 16]),
        h2::frame(3, 0, 1, &8u32.to_be_bytes()),
        h2::frame(7, 0, 0, &[0; 8]),
        h2::frame(0xee, 0xff, 3, &[1, 2, 3]),
        h2::version_call(1),
        h2::version_call(3),
        h2::headers(5, &h2::request_fields(v), true),
        h2::message(5, &[0x08, 0x01], true),
        h2::frame(0, 0x8, 3, &[2, 0, 0, 0, 0, 0, 9, 9]),
    ];
    // A request split over HEADERS and CONTINUATION.
    let blk = h2::block(&h2::request_fields(v));
    let (a, b) = blk.split_at(blk.len() / 2);
    p.push([h2::frame(1, 0, 7, a), h2::frame(9, 0x4, 7, b)].concat());
    // Dynamic table use and a size update.
    let mut d = vec![0x20, 0x3f, 0xe1, 0x1f, 0x40];
    h2::push_str(&mut d, b"x-key");
    h2::push_str(&mut d, b"x-value");
    d.extend(h2::block(&h2::request_fields(v)));
    p.push(h2::frame(1, 0x5, 9, &d));
    p
}

fn mutate(rng: &mut SplitMix64, f: &mut Vec<u8>) {
    if f.is_empty() {
        return;
    }
    match rng.below(8) {
        0 => {
            let i = rng.below(f.len() as u64) as usize;
            f[i] ^= 1 << rng.below(8);
        }
        1 => {
            // Rewrite the length field.
            if f.len() >= 3 {
                let len = rng.below(20_000) as u32;
                f[..3].copy_from_slice(&len.to_be_bytes()[1..]);
            }
        }
        2 => {
            if f.len() >= 5 {
                f[3] = rng.below(12) as u8;
            }
        }
        3 => {
            if f.len() >= 5 {
                f[4] = rng.below(256) as u8;
            }
        }
        4 => {
            if f.len() >= 9 {
                let s = rng.below(12) as u32;
                f[5..9].copy_from_slice(&s.to_be_bytes());
            }
        }
        5 => {
            let n = rng.below(f.len() as u64 + 1) as usize;
            f.truncate(n);
        }
        6 => {
            let n = rng.below(64) as usize;
            let extra = rng.bytes(n);
            let i = rng.below(f.len() as u64 + 1) as usize;
            f.splice(i..i, extra);
        }
        _ => {
            let i = 9.min(f.len());
            let j = i + rng.below((f.len() - i) as u64 + 1) as usize;
            for b in &mut f[i..j] {
                *b = rng.next_u64() as u8;
            }
        }
    }
}

#[test]
fn mutation_loop_never_panics_and_stays_bounded() {
    let iterations: usize = std::env::var("HOSTILE_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let mut rng = SplitMix64::from_env("hostile mutation loop");
    let pool = pool();
    let router = version_router();
    let limits = small_limits();
    // One frame in the input buffer; a header block's kept fields and one
    // cut representation; the admitted request bytes plus one window per
    // stream; a dynamic table; and the output of one chunk.
    let bound = 16_393
        + limits.max_header_list_size
        + 2 * (limits.max_header_list_size + 8)
        + limits.max_buffered
        + limits.max_streams * 65_535
        + 16_384
        + 4096
        + (64 << 10);
    let mut closed = 0usize;
    let mut calls = 0usize;
    let started = Instant::now();
    for _ in 0..iterations {
        let mut c = Connection::new(limits.clone());
        let mut input = frame::PREFACE.to_vec();
        if rng.below(10) != 0 {
            input.extend(h2::settings(&[]));
        }
        for _ in 0..rng.below(40) + 1 {
            let mut f = pool[rng.below(pool.len() as u64) as usize].clone();
            // Most frames stay valid, so sessions get deep enough to reach
            // flow control, responses and the later checks.
            if rng.below(5) == 0 {
                for _ in 0..rng.below(2) + 1 {
                    mutate(&mut rng, &mut f);
                }
            }
            let repeat = if rng.below(20) == 0 {
                rng.below(200) + 1
            } else {
                1
            };
            for _ in 0..repeat {
                input.extend_from_slice(&f);
            }
        }
        let mut pos = 0;
        let mut t = Instant::now();
        while pos < input.len() && !c.is_closed() {
            let n = (rng.below(4096) as usize + 1).min(input.len() - pos);
            let mut queue: std::collections::VecDeque<Event> =
                c.recv(&input[pos..pos + n], &router, t).into();
            while let Some(ev) = queue.pop_front() {
                calls += 1;
                let Event::Call { stream, .. } = ev;
                let result = if rng.below(4) == 0 {
                    Err(Status::new(code::INTERNAL, "boom\n%é"))
                } else {
                    Ok(vec![0x55; rng.below(70_000) as usize])
                };
                queue.extend(c.respond(stream, result));
            }
            pos += n;
            t += Duration::from_millis(rng.below(1000));
            assert!(c.held_bytes() <= bound, "held {} > {bound}", c.held_bytes());
            assert!(c.open_streams() <= limits.max_streams);
            c.take_output();
        }
        if c.is_closed() {
            closed += 1;
        }
    }
    eprintln!(
        "{iterations} sessions in {:?}: {closed} closed by the server, {calls} calls answered",
        started.elapsed()
    );
}
