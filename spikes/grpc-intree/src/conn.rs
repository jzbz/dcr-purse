//! The server side of one HTTP/2 connection carrying gRPC, as a state
//! machine with no I/O: bytes in through [`Connection::recv`], bytes out
//! through [`Connection::take_output`], completed unary requests out as
//! [`Event`]s, responses in through [`Connection::respond`].
//!
//! Policies follow grpc-go v1.83.2 as dcrwallet configures it (spec 19
//! section 4.1): the initial windows stay at 65,535, request messages are
//! limited to 4 MiB, header blocks are decoded fragment by fragment against
//! grpc-go's 16 MiB header-list budget with x/net's rules for what closes
//! the connection, and client PINGs are policed with grpc-go's default
//! enforcement (a strike for a ping within 5 minutes of the previous one
//! while a stream is open, or within 2 hours while none is, strikes reset
//! whenever the server has written HEADERS or DATA, GOAWAY ENHANCE_YOUR_CALM
//! "too_many_pings" on the third). The server's SETTINGS carry
//! MAX_FRAME_SIZE 16384, as grpc-go's do, and, unlike grpc-go's,
//! MAX_CONCURRENT_STREAMS: the stream cap below is advertised so that a
//! well-behaved client queues calls past it instead of having them refused.
//!
//! Beyond grpc-go, and because owning HTTP/2 means owning its attack
//! surface, the state machine bounds what a peer can make it hold:
//!
//! - the input buffer (one frame) and, during a header block, the fields
//!   kept (the 16 MiB header-list budget) and one cut representation (one
//!   name or value, at most that budget);
//! - request bytes: a stream's message is admitted against a per-connection
//!   budget when its 5-byte prefix arrives, and the budget is charged until
//!   the handler's response is delivered, not merely until the message is
//!   handed to a handler. A stream that does not fit stops having its window
//!   returned (only that stream) and is admitted first-come first-served as
//!   the budget frees, so admitted streams always finish and flow control
//!   cannot wedge; the connection window is always returned, as grpc-go
//!   returns it. Without admission a stream holds at most its 65,535-byte
//!   window, so the bound is `max_buffered + max_streams * 65,535`;
//! - handler work: at most `max_inflight` calls per connection run at once,
//!   counting calls whose stream the client has since reset; complete
//!   requests past that wait in the connection (back-pressure, never an
//!   error);
//! - streams (advertised, and refused past the cap for a client that ignores
//!   it);
//! - resets of streams whose request was still incomplete (rapid reset),
//!   scored with decay; a reset of a call already handed to a handler is
//!   ordinary cancellation and never scored.
//!
//! Output is bounded by input and by the in-flight bound: every frame in
//! produces a bounded number of bytes out, response data waits for the
//! peer's windows, and the I/O layer stops reading while output it cannot
//! write is pending.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::frame::{self, ErrorCode, Header, flag, kind, setting};
use crate::grpc::{self, Body, CallContext, Handler, Router, Status, code};
use crate::hpack::{self, Field};

/// Connection limits.
#[derive(Debug, Clone)]
pub struct Limits {
    /// SETTINGS_MAX_FRAME_SIZE this side advertises and enforces.
    pub max_frame_size: u32,
    /// Decoded header list budget (grpc-go's default, 16 MiB), which also
    /// limits one name or value, as x/net's string limit does.
    pub max_header_list_size: usize,
    /// Frames in one header block (HEADERS plus CONTINUATIONs). x/net has
    /// no count; the default, one frame per KiB of the header-list budget,
    /// is sixteen times what a client splitting at 16 KiB needs for a full
    /// budget, and stops an endless run of empty CONTINUATIONs.
    pub max_header_frames: usize,
    /// Open streams, advertised as SETTINGS_MAX_CONCURRENT_STREAMS; a stream
    /// past it is refused (REFUSED_STREAM).
    pub max_streams: usize,
    /// Request bytes admitted per connection: messages being received with
    /// an admission, and complete messages whose response has not been
    /// delivered.
    pub max_buffered: usize,
    /// Handler calls per connection that may run at once, counting calls
    /// whose stream has been reset; complete requests past it wait.
    pub max_inflight: usize,
    /// Score of resets of incomplete requests before the connection is
    /// closed with ENHANCE_YOUR_CALM; each such reset adds one, each
    /// delivered response and each `reset_decay` of time take one away.
    pub max_reset_score: u32,
    /// Time over which one point of reset score decays.
    pub reset_decay: Duration,
    /// Keepalive enforcement: minimum interval between client pings while
    /// a stream is open.
    pub ping_min_time: Duration,
    /// Minimum interval between client pings while no stream is open.
    pub ping_idle_time: Duration,
    /// Strikes tolerated; one more closes the connection.
    pub max_ping_strikes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_frame_size: 16_384,
            max_header_list_size: 16 << 20,
            max_header_frames: (16 << 20) / 1024,
            max_streams: 4096,
            max_buffered: 64 << 20,
            max_inflight: 64,
            max_reset_score: 1000,
            reset_decay: Duration::from_millis(10),
            ping_min_time: Duration::from_secs(5 * 60),
            ping_idle_time: Duration::from_secs(2 * 3600),
            max_ping_strikes: 2,
        }
    }
}

/// The default flow-control window (RFC 9113 section 6.9.2).
const DEFAULT_WINDOW: i64 = 65_535;
const MAX_WINDOW: i64 = 0x7fff_ffff;
/// grpc-go returns window once a quarter of it has been consumed.
const WINDOW_UPDATE_THRESHOLD: u32 = (DEFAULT_WINDOW / 4) as u32;

/// A completed request for a handler.
pub enum Event {
    /// A unary call.
    Call {
        /// The stream to respond on.
        stream: u32,
        /// The handler the path resolved to.
        handler: Handler,
        /// The call context.
        ctx: CallContext,
        /// The request message, without its prefix.
        message: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preface,
    FirstSettings,
    Open,
    Closed,
}

struct PeerSettings {
    initial_window: i64,
    max_frame_size: u32,
}

/// A header block being received: what x/net's `readMetaFrame` keeps.
struct PendingBlock {
    stream: u32,
    first: Header,
    end_stream: bool,
    bad_priority: bool,
    fields: Vec<Field>,
    /// Bytes of name and value in `fields`.
    field_bytes: usize,
    /// x/net's `headersRemainSize` and `trailersRemainSize`.
    remain: usize,
    trailers_remain: usize,
    truncated: bool,
    invalid: bool,
    saw_regular: bool,
    frames: usize,
}

/// Whether a stream's request message has been admitted against the
/// connection's byte budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// The 5-byte prefix has not arrived (or declared too large a message).
    Unknown,
    /// Admitted for this many bytes; the stream's window is returned.
    Admitted(usize),
    /// Waiting for budget; the stream's window is withheld.
    Waiting(usize),
    /// The message is complete; its bytes are counted as held.
    Done,
}

struct Stream {
    recv_window: i64,
    recv_unacked: u32,
    send_window: i64,
    remote_closed: bool,
    handler: Option<Handler>,
    ctx: Option<CallContext>,
    subtype: String,
    body: Vec<u8>,
    admission: Admission,
    /// The request is complete (or answered without one): no more body is
    /// read, and a reset is not a rapid-reset strike.
    dispatched: bool,
    /// A complete request waiting for an in-flight slot.
    ready: Option<Vec<u8>>,
    responded: bool,
    pending: Vec<u8>,
    pending_offset: usize,
    trailers: Option<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
}

/// One server connection.
pub struct Connection {
    limits: Limits,
    phase: Phase,
    inbuf: Vec<u8>,
    decoder: hpack::Decoder,
    peer: PeerSettings,
    send_window: i64,
    recv_window: i64,
    recv_unacked: u32,
    streams: BTreeMap<u32, Stream>,
    max_stream_id: u32,
    block: Option<PendingBlock>,
    out: Vec<u8>,
    events: Vec<Event>,
    /// Bytes in partial request bodies.
    buffered: usize,
    /// Bytes admitted to streams still receiving their message.
    admitted: usize,
    /// Bytes of complete requests not yet answered (waiting or running).
    held: usize,
    /// Calls handed to handlers and not answered, with their message size;
    /// a reset does not remove one, the handler's response does.
    inflight: BTreeMap<u32, usize>,
    /// Complete requests waiting for an in-flight slot, in arrival order.
    ready: VecDeque<u32>,
    /// Streams waiting for admission, in arrival order.
    waiting: VecDeque<u32>,
    reset_score: u32,
    last_reset: Option<Instant>,
    last_ping: Option<Instant>,
    ping_strikes: u32,
    reset_ping_strikes: bool,
    draining: bool,
    deferred: Vec<(u32, Status)>,
    /// Why the connection closed, for logs and tests.
    pub close_reason: Option<(ErrorCode, String)>,
}

impl Connection {
    /// A new connection; its SETTINGS frame is already queued.
    pub fn new(limits: Limits) -> Self {
        let mut out = Vec::new();
        frame::settings(
            &mut out,
            &[
                (setting::MAX_FRAME_SIZE, limits.max_frame_size),
                (
                    setting::MAX_CONCURRENT_STREAMS,
                    u32::try_from(limits.max_streams).unwrap_or(u32::MAX),
                ),
            ],
        );
        let mut decoder = hpack::Decoder::default();
        decoder.set_max_string(limits.max_header_list_size);
        Connection {
            limits,
            phase: Phase::Preface,
            inbuf: Vec::new(),
            decoder,
            peer: PeerSettings {
                initial_window: DEFAULT_WINDOW,
                max_frame_size: 16_384,
            },
            send_window: DEFAULT_WINDOW,
            recv_window: DEFAULT_WINDOW,
            recv_unacked: 0,
            streams: BTreeMap::new(),
            max_stream_id: 0,
            block: None,
            out,
            events: Vec::new(),
            buffered: 0,
            admitted: 0,
            held: 0,
            inflight: BTreeMap::new(),
            ready: VecDeque::new(),
            waiting: VecDeque::new(),
            reset_score: 0,
            last_reset: None,
            last_ping: None,
            ping_strikes: 0,
            reset_ping_strikes: false,
            draining: false,
            deferred: Vec::new(),
            close_reason: None,
        }
    }

    /// Whether the connection has closed (after a GOAWAY, or a drain that
    /// finished): the I/O layer flushes the output and closes the socket.
    pub fn is_closed(&self) -> bool {
        self.phase == Phase::Closed
    }

    /// Whether the preface and first SETTINGS have arrived.
    pub fn is_established(&self) -> bool {
        matches!(self.phase, Phase::Open)
    }

    /// Takes the bytes to write.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Bytes held for the peer: the input buffer, the header block being
    /// received, partial request bodies, complete requests not yet answered
    /// (including those a handler is running with), unsent response data
    /// and output. Bounded by [`Limits`].
    pub fn held_bytes(&self) -> usize {
        self.inbuf.len()
            + self.block.as_ref().map_or(0, |b| b.field_bytes)
            + self.decoder.saved_len()
            + self
                .streams
                .values()
                .map(|s| s.body.len() + s.pending.len() - s.pending_offset)
                .sum::<usize>()
            + self.held
            + self.out.len()
            + self.decoder.table_size()
    }

    /// Open streams.
    pub fn open_streams(&self) -> usize {
        self.streams.len()
    }

    /// Calls handed to handlers and not yet answered.
    pub fn inflight_calls(&self) -> usize {
        self.inflight.len()
    }

    /// Complete requests waiting for an in-flight slot.
    pub fn queued_calls(&self) -> usize {
        self.ready
            .iter()
            .filter(|id| self.streams.get(id).is_some_and(|s| s.ready.is_some()))
            .count()
    }

    /// Feeds bytes read from the peer and returns the requests completed.
    pub fn recv(&mut self, data: &[u8], router: &Router, now: Instant) -> Vec<Event> {
        if self.phase == Phase::Closed {
            return Vec::new();
        }
        self.inbuf.extend_from_slice(data);
        let mut pos = 0;
        while self.phase != Phase::Closed {
            let rest = &self.inbuf[pos..];
            if self.phase == Phase::Preface {
                // grpc-go reads the whole preface, then compares it, and
                // closes on a bogus greeting without a GOAWAY.
                let n = frame::PREFACE.len();
                if rest.len() < n {
                    break;
                }
                if rest[..n] != *frame::PREFACE {
                    self.close(ErrorCode::Protocol, "bogus greeting", false);
                    break;
                }
                pos += n;
                self.phase = Phase::FirstSettings;
                continue;
            }
            if rest.len() < frame::HEADER_LEN {
                break;
            }
            let mut h = [0u8; frame::HEADER_LEN];
            h.copy_from_slice(&rest[..frame::HEADER_LEN]);
            let header = Header::parse(&h);
            if header.length > self.limits.max_frame_size {
                self.conn_error(ErrorCode::FrameSize, "frame too large");
                break;
            }
            let total = frame::HEADER_LEN + header.length as usize;
            if rest.len() < total {
                break;
            }
            let payload = rest[frame::HEADER_LEN..total].to_vec();
            pos += total;
            if self.phase == Phase::FirstSettings {
                if header.kind != kind::SETTINGS || header.flags & flag::ACK != 0 {
                    self.close(ErrorCode::Protocol, "first frame is not SETTINGS", false);
                    break;
                }
                self.phase = Phase::Open;
            }
            self.frame(header, &payload, router, now);
        }
        self.inbuf.drain(..pos.min(self.inbuf.len()));
        if self.phase == Phase::Closed {
            self.inbuf.clear();
        }
        // Statuses grpc-go writes from the stream's own goroutine (an
        // unroutable method, an unsupported encoding) go out once the bytes
        // that arrived with the HEADERS have been read, so that, as with
        // grpc-go, a request whose END_STREAM came in the same read gets no
        // RST_STREAM after its trailers.
        for (id, st) in std::mem::take(&mut self.deferred) {
            self.respond_status(id, &st);
        }
        self.pump();
        self.replenish();
        std::mem::take(&mut self.events)
    }

    /// Delivers a handler's result, and returns the requests that the freed
    /// in-flight slot lets through. A stream the client has reset since is
    /// not answered, but its slot and bytes are released here.
    pub fn respond(&mut self, stream: u32, result: Result<Vec<u8>, Status>) -> Vec<Event> {
        if self.phase == Phase::Closed {
            return Vec::new();
        }
        if let Some(len) = self.inflight.remove(&stream) {
            self.held -= len;
            self.reset_score = self.reset_score.saturating_sub(1);
        }
        self.deliver(stream, result);
        self.finish_drain();
        self.pump();
        self.replenish();
        std::mem::take(&mut self.events)
    }

    fn deliver(&mut self, stream: u32, result: Result<Vec<u8>, Status>) {
        match result {
            Ok(msg) => {
                let Some(s) = self.streams.get_mut(&stream) else {
                    return;
                };
                if s.responded {
                    return;
                }
                s.responded = true;
                let ct = grpc::content_type(&s.subtype);
                let mut block = Vec::new();
                hpack::encode(&[(":status", "200"), ("content-type", &ct)], &mut block);
                s.pending = grpc::frame_message(&msg);
                s.pending_offset = 0;
                let mut trailers = Vec::new();
                hpack::encode(&[("grpc-status", "0"), ("grpc-message", "")], &mut trailers);
                s.trailers = Some(trailers);
                self.write_headers(stream, &block, false);
                self.flush_stream(stream);
            }
            Err(st) => self.respond_status(stream, &st),
        }
    }

    /// Hands waiting requests to handlers while in-flight slots are free,
    /// then admits waiting streams while the byte budget allows, in arrival
    /// order (a stream at the head that does not fit holds the rest back, so
    /// the budget goes to streams in the order they asked).
    fn pump(&mut self) {
        if self.phase == Phase::Closed {
            return;
        }
        while self.inflight.len() < self.limits.max_inflight {
            let Some(id) = self.ready.pop_front() else {
                break;
            };
            let Some(s) = self.streams.get_mut(&id) else {
                continue;
            };
            let Some(message) = s.ready.take() else {
                continue;
            };
            let (Some(handler), Some(ctx)) = (s.handler.clone(), s.ctx.clone()) else {
                continue;
            };
            self.inflight.insert(id, message.len());
            self.events.push(Event::Call {
                stream: id,
                handler,
                ctx,
                message,
            });
        }
        while let Some(&id) = self.waiting.front() {
            let need = match self.streams.get(&id).map(|s| s.admission) {
                Some(Admission::Waiting(n)) => n,
                _ => {
                    self.waiting.pop_front();
                    continue;
                }
            };
            if !self.fits(need) {
                break;
            }
            self.waiting.pop_front();
            self.admitted += need;
            if let Some(s) = self.streams.get_mut(&id) {
                s.admission = Admission::Admitted(need);
                if s.recv_unacked > 0 && !s.remote_closed {
                    let inc = s.recv_unacked;
                    s.recv_unacked = 0;
                    s.recv_window += i64::from(inc);
                    frame::window_update(&mut self.out, id, inc);
                }
            }
        }
    }

    /// Whether a message of `need` bytes can be admitted now. One message
    /// is always admitted when nothing else is, so a budget smaller than a
    /// message cannot wedge the connection.
    fn fits(&self, need: usize) -> bool {
        let used = self.admitted + self.held;
        used == 0 || used + need <= self.limits.max_buffered
    }

    /// Starts a graceful shutdown: GOAWAY NO_ERROR, no new streams, close
    /// once the open ones finish.
    pub fn shutdown(&mut self) {
        if self.phase == Phase::Closed || self.draining {
            return;
        }
        self.draining = true;
        frame::goaway(&mut self.out, self.max_stream_id, ErrorCode::NoError, b"");
        self.finish_drain();
    }

    /// Marks every open call cancelled (the socket died).
    pub fn abort(&mut self) {
        for s in self.streams.values() {
            s.cancelled.store(true, Ordering::Relaxed);
        }
        self.streams.clear();
        self.phase = Phase::Closed;
    }

    fn finish_drain(&mut self) {
        if self.draining && self.streams.is_empty() && self.phase != Phase::Closed {
            self.phase = Phase::Closed;
            self.close_reason = Some((ErrorCode::NoError, "drained".into()));
        }
    }

    fn close(&mut self, code: ErrorCode, why: &str, goaway: bool) {
        if goaway {
            frame::goaway(&mut self.out, self.max_stream_id, code, why.as_bytes());
        }
        for s in self.streams.values() {
            s.cancelled.store(true, Ordering::Relaxed);
        }
        self.streams.clear();
        self.block = None;
        self.buffered = 0;
        self.admitted = 0;
        self.ready.clear();
        self.waiting.clear();
        self.phase = Phase::Closed;
        self.close_reason = Some((code, why.to_owned()));
    }

    fn conn_error(&mut self, code: ErrorCode, why: &str) {
        self.close(code, why, true);
    }

    fn stream_error(&mut self, stream: u32, code: ErrorCode) {
        frame::rst_stream(&mut self.out, stream, code);
        self.remove_stream(stream);
    }

    /// Forgets a stream. A call already handed to a handler keeps its
    /// in-flight slot and bytes until the handler's response arrives.
    fn remove_stream(&mut self, stream: u32) {
        if let Some(s) = self.streams.remove(&stream) {
            s.cancelled.store(true, Ordering::Relaxed);
            self.buffered -= s.body.len();
            if let Admission::Admitted(n) = s.admission {
                self.admitted -= n;
            }
            if let Some(m) = &s.ready {
                self.held -= m.len();
            }
        }
    }

    fn frame(&mut self, h: Header, p: &[u8], router: &Router, now: Instant) {
        if let Some(block) = &self.block
            && (h.kind != kind::CONTINUATION || h.stream != block.stream)
        {
            self.conn_error(ErrorCode::Protocol, "expected CONTINUATION");
            return;
        }
        match h.kind {
            kind::DATA => self.on_data(h, p),
            kind::HEADERS => self.on_headers(h, p, router, now),
            kind::CONTINUATION => self.on_continuation(h, p, router, now),
            kind::PRIORITY => {
                if h.stream == 0 {
                    self.conn_error(ErrorCode::Protocol, "PRIORITY on stream 0");
                } else if p.len() != 5 {
                    self.stream_error(h.stream, ErrorCode::FrameSize);
                } else if u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0x7fff_ffff == h.stream {
                    self.stream_error(h.stream, ErrorCode::Protocol);
                }
            }
            kind::RST_STREAM => self.on_rst(h, p, now),
            kind::SETTINGS => self.on_settings(h, p),
            kind::PUSH_PROMISE => self.conn_error(ErrorCode::Protocol, "PUSH_PROMISE from client"),
            kind::PING => self.on_ping(h, p, now),
            kind::GOAWAY => {
                if h.stream != 0 {
                    self.conn_error(ErrorCode::Protocol, "GOAWAY on a stream");
                } else if p.len() < 8 {
                    self.conn_error(ErrorCode::FrameSize, "short GOAWAY");
                }
                // A client GOAWAY is otherwise ignored, as grpc-go does.
            }
            kind::WINDOW_UPDATE => self.on_window_update(h, p),
            _ => {} // Unknown types are ignored (RFC 9113 section 4.1).
        }
    }

    /// Strips padding (and the PRIORITY fields of HEADERS), or fails the
    /// connection.
    fn unpad<'a>(&mut self, h: Header, p: &'a [u8], priority: bool) -> Option<(&'a [u8], u32)> {
        let mut body = p;
        let mut pad = 0usize;
        if h.flags & flag::PADDED != 0 {
            let Some((&n, rest)) = body.split_first() else {
                self.conn_error(ErrorCode::FrameSize, "padded frame too short");
                return None;
            };
            pad = usize::from(n);
            body = rest;
        }
        let mut dep = 0;
        if priority && h.flags & flag::PRIORITY != 0 {
            if body.len() < 5 {
                self.conn_error(ErrorCode::FrameSize, "HEADERS too short for PRIORITY");
                return None;
            }
            dep = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) & 0x7fff_ffff;
            body = &body[5..];
        }
        if pad > body.len() {
            // Padding as long as the payload or longer is a PROTOCOL_ERROR
            // (RFC 9113 sections 6.1 and 6.2).
            self.conn_error(ErrorCode::Protocol, "padding exceeds payload");
            return None;
        }
        Some((&body[..body.len() - pad], dep))
    }

    fn on_data(&mut self, h: Header, p: &[u8]) {
        if h.stream == 0 {
            self.conn_error(ErrorCode::Protocol, "DATA on stream 0");
            return;
        }
        if h.stream > self.max_stream_id {
            self.conn_error(ErrorCode::Protocol, "DATA on an idle stream");
            return;
        }
        let len = p.len() as i64;
        if len > self.recv_window {
            self.conn_error(ErrorCode::FlowControl, "connection window exceeded");
            return;
        }
        self.recv_window -= len;
        self.recv_unacked += p.len() as u32;
        let Some((data, _)) = self.unpad(h, p, false) else {
            return;
        };
        let end = h.flags & flag::END_STREAM != 0;
        let Some(s) = self.streams.get_mut(&h.stream) else {
            // A closed stream: ignored, as grpc-go ignores it.
            return;
        };
        if s.remote_closed {
            self.stream_error(h.stream, ErrorCode::StreamClosed);
            return;
        }
        if len > s.recv_window {
            self.stream_error(h.stream, ErrorCode::FlowControl);
            return;
        }
        s.recv_window -= len;
        s.recv_unacked += p.len() as u32;
        if end {
            s.remote_closed = true;
        }
        if !s.dispatched && !s.responded {
            s.body.extend_from_slice(data);
            self.buffered += data.len();
            self.admit(h.stream);
            self.check_body(h.stream);
        } else if end && s.responded && s.pending_offset >= s.pending.len() && s.trailers.is_none()
        {
            self.remove_stream(h.stream);
        }
        // The stream window is returned only while an admitted request
        // message is being read: grpc-go returns it as the handler reads,
        // and a unary handler reads one message, so further data runs the
        // stream out of window (RST_STREAM FLOW_CONTROL_ERROR). A stream
        // waiting for admission gets its window when it is admitted.
        if let Some(s) = self.streams.get_mut(&h.stream)
            && !s.remote_closed
            && !s.dispatched
            && !s.responded
            && matches!(s.admission, Admission::Admitted(_))
            && s.recv_unacked >= WINDOW_UPDATE_THRESHOLD
        {
            let inc = s.recv_unacked;
            s.recv_unacked = 0;
            s.recv_window += i64::from(inc);
            frame::window_update(&mut self.out, h.stream, inc);
        }
    }

    /// Admits a stream's message against the byte budget once its prefix
    /// has arrived, or queues it.
    fn admit(&mut self, stream: u32) {
        let Some(s) = self.streams.get(&stream) else {
            return;
        };
        if s.admission != Admission::Unknown || s.body.len() < 5 {
            return;
        }
        let need = u32::from_be_bytes([s.body[1], s.body[2], s.body[3], s.body[4]]) as usize;
        if need > grpc::MAX_RECV_MESSAGE {
            // check_body answers ResourceExhausted.
            return;
        }
        let admission = if self.waiting.is_empty() && self.fits(need) {
            self.admitted += need;
            Admission::Admitted(need)
        } else {
            self.waiting.push_back(stream);
            Admission::Waiting(need)
        };
        if let Some(s) = self.streams.get_mut(&stream) {
            s.admission = admission;
        }
    }

    /// Returns connection window once a quarter is consumed, always, as
    /// grpc-go does: what bounds memory is the per-stream admission.
    fn replenish(&mut self) {
        if self.phase == Phase::Closed {
            return;
        }
        if self.recv_unacked >= WINDOW_UPDATE_THRESHOLD {
            let inc = self.recv_unacked;
            self.recv_unacked = 0;
            self.recv_window += i64::from(inc);
            frame::window_update(&mut self.out, 0, inc);
        }
    }

    fn check_body(&mut self, stream: u32) {
        let Some(s) = self.streams.get_mut(&stream) else {
            return;
        };
        match grpc::parse_body(&s.body, s.remote_closed) {
            Body::Incomplete => {}
            Body::Message(message) => {
                s.dispatched = true;
                self.buffered -= s.body.len();
                s.body = Vec::new();
                if let Admission::Admitted(n) = s.admission {
                    self.admitted -= n;
                }
                s.admission = Admission::Done;
                if s.handler.is_none() || s.ctx.is_none() {
                    return;
                }
                self.held += message.len();
                s.ready = Some(message);
                self.ready.push_back(stream);
                self.pump();
            }
            Body::Fail(st) => {
                self.buffered -= s.body.len();
                s.body = Vec::new();
                if let Admission::Admitted(n) = s.admission {
                    self.admitted -= n;
                }
                s.admission = Admission::Done;
                self.respond_status(stream, &st);
            }
        }
    }

    fn on_headers(&mut self, h: Header, p: &[u8], router: &Router, now: Instant) {
        if h.stream == 0 {
            self.conn_error(ErrorCode::Protocol, "HEADERS on stream 0");
            return;
        }
        let Some((fragment, dep)) = self.unpad(h, p, true) else {
            return;
        };
        let budget = self.limits.max_header_list_size;
        self.block = Some(PendingBlock {
            stream: h.stream,
            first: h,
            end_stream: h.flags & flag::END_STREAM != 0,
            bad_priority: h.flags & flag::PRIORITY != 0 && dep == h.stream,
            fields: Vec::new(),
            field_bytes: 0,
            remain: budget,
            trailers_remain: budget,
            truncated: false,
            invalid: false,
            saw_regular: false,
            frames: 0,
        });
        self.fragment(fragment, h.flags & flag::END_HEADERS != 0, router, now);
    }

    fn on_continuation(&mut self, h: Header, p: &[u8], router: &Router, now: Instant) {
        if self.block.is_none() {
            self.conn_error(ErrorCode::Protocol, "CONTINUATION without HEADERS");
            return;
        }
        self.fragment(p, h.flags & flag::END_HEADERS != 0, router, now);
    }

    /// One fragment of a header block, decoded as it arrives (x/net's
    /// `readMetaFrame`): a fragment more than twice the header-list budget
    /// left (so any non-empty one once the budget is spent) or any fragment
    /// after an invalid field closes the connection; otherwise fields are
    /// kept until the budget runs out and decoded, but not kept, after.
    fn fragment(&mut self, frag: &[u8], end_headers: bool, router: &Router, now: Instant) {
        let Some(mut block) = self.block.take() else {
            return;
        };
        if frag.len() as u64 > 2 * (block.remain as u64 + block.trailers_remain as u64) {
            self.conn_error(ErrorCode::Protocol, "header list too large");
            return;
        }
        if block.invalid {
            self.conn_error(ErrorCode::Protocol, "invalid header field");
            return;
        }
        block.frames += 1;
        if block.frames > self.limits.max_header_frames {
            self.conn_error(ErrorCode::EnhanceYourCalm, "too many CONTINUATION frames");
            return;
        }
        let result = self.decoder.write(frag, &mut |name, value| {
            if block.invalid || block.truncated {
                return;
            }
            if value.iter().any(|&c| (c < b' ' && c != b'\t') || c == 0x7f) {
                block.invalid = true;
                return;
            }
            if name.first() == Some(&b':') {
                if block.saw_regular {
                    block.invalid = true;
                    return;
                }
            } else {
                block.saw_regular = true;
                if !valid_wire_name(name) {
                    block.invalid = true;
                    return;
                }
            }
            let (remain, size) = if name == b"trailer" {
                // x/net counts each listed trailer name as a field.
                let n = value.iter().filter(|&&c| c == b',').count() + 1;
                (&mut block.trailers_remain, value.len() + 1 - n + n * 32)
            } else {
                (&mut block.remain, name.len() + value.len() + 32)
            };
            if size > *remain {
                block.truncated = true;
                block.remain = 0;
                block.trailers_remain = 0;
                return;
            }
            *remain -= size;
            block.field_bytes += name.len() + value.len();
            block.fields.push(Field {
                name: name.to_vec(),
                value: value.to_vec(),
            });
        });
        if result.is_err() {
            self.conn_error(ErrorCode::Compression, "header block does not decode");
            return;
        }
        self.block = Some(block);
        if end_headers {
            self.end_block(router, now);
        }
    }

    fn end_block(&mut self, router: &Router, now: Instant) {
        let Some(block) = self.block.take() else {
            return;
        };
        if self.decoder.finish().is_err() {
            self.conn_error(ErrorCode::Compression, "header block does not decode");
            return;
        }
        let fields = &block.fields;
        let id = block.stream;

        // Trailers on an open stream.
        if self.streams.contains_key(&id) {
            if !block.end_stream {
                self.stream_error(id, ErrorCode::Protocol);
                return;
            }
            if let Some(s) = self.streams.get_mut(&id) {
                if s.remote_closed {
                    self.stream_error(id, ErrorCode::StreamClosed);
                    return;
                }
                s.remote_closed = true;
                if !s.dispatched && !s.responded {
                    self.check_body(id);
                }
            }
            return;
        }
        if id % 2 != 1 || id <= self.max_stream_id {
            // grpc-go: "received an illegal stream id", GOAWAY
            // PROTOCOL_ERROR with the text as debug data.
            let text = format!(
                "received an illegal stream id: {id}. headers frame: {}",
                go_frame_header(&block.first)
            );
            self.conn_error(ErrorCode::Protocol, &text);
            return;
        }
        self.max_stream_id = id;
        if self.draining {
            frame::rst_stream(&mut self.out, id, ErrorCode::RefusedStream);
            return;
        }
        // x/net reports an invalid field or pseudo-header set as a stream
        // error before grpc-go sees the truncation.
        if block.bad_priority || block.invalid || !valid_request_fields(fields) {
            frame::rst_stream(&mut self.out, id, ErrorCode::Protocol);
            return;
        }
        if block.truncated {
            frame::rst_stream(&mut self.out, id, ErrorCode::FrameSize);
            return;
        }
        self.open_stream(id, block.end_stream, fields, router, now);
    }

    fn early_abort(&mut self, id: u32, subtype: &str, http: &str, st: &Status, ended: bool) {
        let ct = grpc::content_type(subtype);
        let code = st.code.to_string();
        let msg = grpc::encode_grpc_message(&st.message);
        let mut block = Vec::new();
        hpack::encode(
            &[
                (":status", http),
                ("content-type", &ct),
                ("grpc-status", &code),
                ("grpc-message", &msg),
            ],
            &mut block,
        );
        self.write_headers(id, &block, true);
        if !ended {
            frame::rst_stream(&mut self.out, id, ErrorCode::NoError);
        }
    }

    fn open_stream(
        &mut self,
        id: u32,
        ended: bool,
        fields: &[Field],
        router: &Router,
        now: Instant,
    ) {
        // grpc-go's operateHeaders, in its order.
        let mut subtype: Option<String> = None;
        let mut bad_content_type = String::new();
        let mut method = String::new();
        let mut path = String::new();
        let mut recv_compress = String::new();
        let mut timeout: Option<Result<Duration, String>> = None;
        let mut protocol_error = false;
        let mut authorities = 0;
        let mut hosts = 0;
        let mut metadata = Vec::new();
        for f in fields {
            let name = String::from_utf8_lossy(&f.name);
            let value = String::from_utf8_lossy(&f.value);
            match name.as_ref() {
                "content-type" => match grpc::content_subtype(&value) {
                    Some(st) => subtype = Some(st.to_owned()),
                    None => bad_content_type = value.into_owned(),
                },
                "grpc-encoding" => recv_compress = value.into_owned(),
                ":method" => method = value.into_owned(),
                ":path" => path = value.into_owned(),
                ":authority" => authorities += 1,
                "host" => hosts += 1,
                "grpc-timeout" => timeout = Some(grpc::decode_timeout(&value)),
                "connection" => protocol_error = true,
                n if n.starts_with(':')
                    || matches!(
                        n,
                        "user-agent" | "grpc-message-type" | "grpc-message" | "grpc-status" | "te"
                    ) => {}
                _ => metadata.push((name.into_owned(), value.into_owned())),
            }
        }
        let sub = subtype.clone().unwrap_or_default();
        if authorities > 1 || hosts > 1 {
            let st = Status::new(
                code::INTERNAL,
                format!(
                    "num values of :authority: {authorities}, num values of host: {hosts}, both must only have 1 value as per HTTP/2 spec"
                ),
            );
            self.early_abort(id, &sub, "400", &st, ended);
            return;
        }
        if protocol_error {
            frame::rst_stream(&mut self.out, id, ErrorCode::Protocol);
            return;
        }
        if subtype.is_none() {
            let st = Status::new(
                code::INVALID_ARGUMENT,
                format!(
                    "invalid gRPC request content-type {}",
                    grpc::go_quote(&bad_content_type)
                ),
            );
            self.early_abort(id, &sub, "415", &st, ended);
            return;
        }
        if let Some(Err(e)) = &timeout {
            let st = Status::new(code::INTERNAL, format!("malformed grpc-timeout: {e}"));
            self.early_abort(id, &sub, "400", &st, ended);
            return;
        }
        if authorities == 0 && hosts == 0 {
            let st = Status::new(code::INTERNAL, "no host or :authority header present");
            self.early_abort(id, &sub, "400", &st, ended);
            return;
        }
        if self.streams.len() >= self.limits.max_streams {
            frame::rst_stream(&mut self.out, id, ErrorCode::RefusedStream);
            return;
        }
        if method != "POST" {
            let st = Status::new(
                code::INTERNAL,
                format!(
                    "Received a HEADERS frame with :method {} which should be POST",
                    grpc::go_quote(&method)
                ),
            );
            self.early_abort(id, &sub, "405", &st, ended);
            return;
        }
        let deadline = match timeout {
            Some(Ok(d)) => {
                if d.is_zero() {
                    let st = Status::new(code::DEADLINE_EXCEEDED, "context deadline exceeded");
                    self.early_abort(id, &sub, "200", &st, ended);
                    return;
                }
                Some(now + d)
            }
            _ => None,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let routed = router.route(&path);
        let ctx = CallContext {
            method: path,
            deadline,
            metadata,
            cancelled: Arc::clone(&cancelled),
        };
        self.streams.insert(
            id,
            Stream {
                recv_window: DEFAULT_WINDOW,
                recv_unacked: 0,
                send_window: self.peer.initial_window,
                remote_closed: ended,
                handler: routed.as_ref().ok().cloned(),
                ctx: Some(ctx),
                subtype: sub,
                body: Vec::new(),
                admission: Admission::Unknown,
                dispatched: false,
                ready: None,
                responded: false,
                pending: Vec::new(),
                pending_offset: 0,
                trailers: None,
                cancelled,
            },
        );
        if let Err(st) = routed {
            self.defer_status(id, st);
            return;
        }
        if !recv_compress.is_empty() && recv_compress != "identity" {
            let st = Status::new(
                code::UNIMPLEMENTED,
                format!(
                    "grpc: Decompressor is not installed for grpc-encoding {}",
                    grpc::go_quote(&recv_compress)
                ),
            );
            self.defer_status(id, st);
            return;
        }
        if ended {
            self.check_body(id);
        }
    }

    /// Queues a status for the end of this read; the request body is not
    /// read meanwhile.
    fn defer_status(&mut self, id: u32, st: Status) {
        if let Some(s) = self.streams.get_mut(&id) {
            s.dispatched = true;
        }
        self.deferred.push((id, st));
    }

    /// A trailers-only response.
    fn respond_status(&mut self, stream: u32, st: &Status) {
        let Some(s) = self.streams.get_mut(&stream) else {
            return;
        };
        if s.responded {
            return;
        }
        s.responded = true;
        let ct = grpc::content_type(&s.subtype);
        let code = st.code.to_string();
        let msg = grpc::encode_grpc_message(&st.message);
        let mut block = Vec::new();
        hpack::encode(
            &[
                (":status", "200"),
                ("content-type", &ct),
                ("grpc-status", &code),
                ("grpc-message", &msg),
            ],
            &mut block,
        );
        self.write_headers(stream, &block, true);
        self.finish_stream(stream);
    }

    fn finish_stream(&mut self, stream: u32) {
        let remote_closed = self.streams.get(&stream).is_some_and(|s| s.remote_closed);
        if !remote_closed {
            frame::rst_stream(&mut self.out, stream, ErrorCode::NoError);
        }
        self.remove_stream(stream);
    }

    fn write_headers(&mut self, stream: u32, block: &[u8], end_stream: bool) {
        self.reset_ping_strikes = true;
        let max = self.peer.max_frame_size as usize;
        let mut chunks = block.chunks(max.max(1)).peekable();
        let first = chunks.next().unwrap_or(&[]);
        let mut flags = if end_stream { flag::END_STREAM } else { 0 };
        if chunks.peek().is_none() {
            flags |= flag::END_HEADERS;
        }
        frame::write(&mut self.out, kind::HEADERS, flags, stream, first);
        while let Some(c) = chunks.next() {
            let f = if chunks.peek().is_none() {
                flag::END_HEADERS
            } else {
                0
            };
            frame::write(&mut self.out, kind::CONTINUATION, f, stream, c);
        }
    }

    fn flush_stream(&mut self, stream: u32) {
        let max = self.peer.max_frame_size as usize;
        loop {
            let Some(s) = self.streams.get_mut(&stream) else {
                return;
            };
            let left = s.pending.len() - s.pending_offset;
            if left == 0 {
                if let Some(t) = s.trailers.take() {
                    self.write_headers(stream, &t, true);
                    self.finish_stream(stream);
                }
                return;
            }
            let window = self.send_window.min(s.send_window);
            if window <= 0 {
                return;
            }
            let n = left.min(window as usize).min(max);
            let start = s.pending_offset;
            s.pending_offset += n;
            s.send_window -= n as i64;
            self.send_window -= n as i64;
            let chunk = s.pending[start..start + n].to_vec();
            if s.pending_offset == s.pending.len() {
                s.pending = Vec::new();
                s.pending_offset = 0;
            }
            self.reset_ping_strikes = true;
            frame::write(&mut self.out, kind::DATA, 0, stream, &chunk);
        }
    }

    fn flush_all(&mut self) {
        let ids: Vec<u32> = self
            .streams
            .iter()
            .filter(|(_, s)| {
                s.responded && (s.pending.len() > s.pending_offset || s.trailers.is_some())
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.flush_stream(id);
        }
        self.finish_drain();
    }

    fn on_rst(&mut self, h: Header, p: &[u8], now: Instant) {
        if p.len() != 4 {
            self.conn_error(ErrorCode::FrameSize, "RST_STREAM length");
            return;
        }
        if h.stream == 0 {
            self.conn_error(ErrorCode::Protocol, "RST_STREAM on stream 0");
            return;
        }
        if h.stream > self.max_stream_id {
            self.conn_error(ErrorCode::Protocol, "RST_STREAM on an idle stream");
            return;
        }
        let Some(s) = self.streams.get(&h.stream) else {
            return;
        };
        // A reset of a request the server has all of (waiting for, or
        // running in, a handler) is a cancellation: its cost is bounded by
        // the in-flight limit, and grpc-go never counts it. A reset of a
        // request still arriving cost the server only its headers and is
        // what a rapid-reset flood is made of.
        let strike = !s.dispatched;
        self.remove_stream(h.stream);
        if strike {
            if let Some(last) = self.last_reset
                && !self.limits.reset_decay.is_zero()
            {
                let periods = now.saturating_duration_since(last).as_nanos()
                    / self.limits.reset_decay.as_nanos();
                let periods = u32::try_from(periods).unwrap_or(u32::MAX);
                self.reset_score = self.reset_score.saturating_sub(periods);
                if periods > 0 {
                    self.last_reset = Some(now);
                }
            } else {
                self.last_reset = Some(now);
            }
            self.reset_score += 1;
            if self.reset_score > self.limits.max_reset_score {
                self.conn_error(ErrorCode::EnhanceYourCalm, "too many stream resets");
                return;
            }
        }
        self.finish_drain();
    }

    fn on_settings(&mut self, h: Header, p: &[u8]) {
        if h.stream != 0 {
            self.conn_error(ErrorCode::Protocol, "SETTINGS on a stream");
            return;
        }
        if h.flags & flag::ACK != 0 {
            if !p.is_empty() {
                self.conn_error(ErrorCode::FrameSize, "SETTINGS ACK with payload");
            }
            return;
        }
        if !p.len().is_multiple_of(6) {
            self.conn_error(ErrorCode::FrameSize, "SETTINGS length");
            return;
        }
        for c in p.as_chunks::<6>().0 {
            let id = u16::from_be_bytes([c[0], c[1]]);
            let v = u32::from_be_bytes([c[2], c[3], c[4], c[5]]);
            match id {
                setting::ENABLE_PUSH if v > 1 => {
                    self.conn_error(ErrorCode::Protocol, "ENABLE_PUSH out of range");
                    return;
                }
                setting::INITIAL_WINDOW_SIZE => {
                    let v = i64::from(v);
                    if v > MAX_WINDOW {
                        self.conn_error(ErrorCode::FlowControl, "INITIAL_WINDOW_SIZE too large");
                        return;
                    }
                    let delta = v - self.peer.initial_window;
                    self.peer.initial_window = v;
                    for s in self.streams.values_mut() {
                        s.send_window += delta;
                        if s.send_window > MAX_WINDOW {
                            self.conn_error(ErrorCode::FlowControl, "stream window overflow");
                            return;
                        }
                    }
                }
                setting::MAX_FRAME_SIZE => {
                    if !(16_384..=16_777_215).contains(&v) {
                        self.conn_error(ErrorCode::Protocol, "MAX_FRAME_SIZE out of range");
                        return;
                    }
                    self.peer.max_frame_size = v;
                }
                // HEADER_TABLE_SIZE needs nothing (the encoder keeps no
                // table); MAX_CONCURRENT_STREAMS limits pushes, which this
                // server never makes; MAX_HEADER_LIST_SIZE is not enforced on
                // the small responses sent here; unknown ids are ignored.
                _ => {}
            }
        }
        frame::write(&mut self.out, kind::SETTINGS, flag::ACK, 0, &[]);
        self.flush_all();
    }

    fn on_ping(&mut self, h: Header, p: &[u8], now: Instant) {
        if p.len() != 8 {
            self.conn_error(ErrorCode::FrameSize, "PING length");
            return;
        }
        if h.stream != 0 {
            self.conn_error(ErrorCode::Protocol, "PING on a stream");
            return;
        }
        if h.flags & flag::ACK != 0 {
            return;
        }
        frame::write(&mut self.out, kind::PING, flag::ACK, 0, p);
        // grpc-go's handlePing.
        let last = self.last_ping.replace(now);
        if std::mem::take(&mut self.reset_ping_strikes) {
            self.ping_strikes = 0;
            return;
        }
        let min = if self.streams.is_empty() {
            self.limits.ping_idle_time
        } else {
            self.limits.ping_min_time
        };
        if last.is_some_and(|last| last + min > now) {
            self.ping_strikes += 1;
        }
        if self.ping_strikes > self.limits.max_ping_strikes {
            self.conn_error(ErrorCode::EnhanceYourCalm, "too_many_pings");
        }
    }

    fn on_window_update(&mut self, h: Header, p: &[u8]) {
        if p.len() != 4 {
            self.conn_error(ErrorCode::FrameSize, "WINDOW_UPDATE length");
            return;
        }
        let inc = i64::from(u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0x7fff_ffff);
        if h.stream == 0 {
            if inc == 0 {
                self.conn_error(ErrorCode::Protocol, "WINDOW_UPDATE of 0");
                return;
            }
            self.send_window += inc;
            if self.send_window > MAX_WINDOW {
                self.conn_error(ErrorCode::FlowControl, "connection window overflow");
                return;
            }
            self.flush_all();
            return;
        }
        if h.stream > self.max_stream_id {
            self.conn_error(ErrorCode::Protocol, "WINDOW_UPDATE on an idle stream");
            return;
        }
        let Some(s) = self.streams.get_mut(&h.stream) else {
            return;
        };
        if inc == 0 {
            self.stream_error(h.stream, ErrorCode::Protocol);
            return;
        }
        s.send_window += inc;
        if s.send_window > MAX_WINDOW {
            self.stream_error(h.stream, ErrorCode::FlowControl);
            return;
        }
        self.flush_stream(h.stream);
        self.finish_drain();
    }
}

/// x/net's `FrameHeader.String`, which grpc-go's GOAWAY debug text embeds
/// for an illegal stream id: `[FrameHeader HEADERS flags=END_STREAM|
/// END_HEADERS stream=3 len=136]`.
fn go_frame_header(h: &Header) -> String {
    let names: &[(u8, &str)] = &[
        (flag::END_STREAM, "END_STREAM"),
        (flag::END_HEADERS, "END_HEADERS"),
        (flag::PADDED, "PADDED"),
        (flag::PRIORITY, "PRIORITY"),
    ];
    let mut s = String::from("[FrameHeader HEADERS");
    if h.flags != 0 {
        s.push_str(" flags=");
        let mut first = true;
        for i in 0..8 {
            let bit = 1u8 << i;
            if h.flags & bit == 0 {
                continue;
            }
            if !first {
                s.push('|');
            }
            first = false;
            match names.iter().find(|(b, _)| *b == bit) {
                Some((_, n)) => s.push_str(n),
                None => s.push_str(&format!("0x{bit:x}")),
            }
        }
    }
    if h.stream != 0 {
        s.push_str(&format!(" stream={}", h.stream));
    }
    s.push_str(&format!(" len={}]", h.length));
    s
}

/// HTTP/2's request header rules as x/net's framer applies them for
/// grpc-go: lowercase token names, pseudo-headers first, only the request
/// pseudo-headers (x/net's four plus `:protocol`) and each at most once,
/// values without control bytes other than tab (httpguts'
/// `ValidHeaderFieldValue`).
fn valid_request_fields(fields: &[Field]) -> bool {
    let mut seen_regular = false;
    let mut seen = [false; 5];
    for f in fields {
        if f.value
            .iter()
            .any(|&c| (c < b' ' && c != b'\t') || c == 0x7f)
        {
            return false;
        }
        if let Some(name) = f.name.strip_prefix(b":") {
            if seen_regular {
                return false;
            }
            let i = match name {
                b"method" => 0,
                b"scheme" => 1,
                b"path" => 2,
                b"authority" => 3,
                b"protocol" => 4,
                _ => return false,
            };
            if std::mem::replace(&mut seen[i], true) {
                return false;
            }
        } else {
            seen_regular = true;
            if !valid_wire_name(&f.name) {
                return false;
            }
        }
    }
    true
}

/// x/net's `validWireHeaderFieldName`: a non-empty token without uppercase.
fn valid_wire_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.iter().all(|&b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$%&'*+-.^_`|~".contains(&b)
        })
}
