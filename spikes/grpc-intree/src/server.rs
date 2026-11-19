//! The I/O layer: a TCP listener with connection admission, per connection
//! a reader thread and a writer thread with blocking I/O, and a fixed pool of
//! worker threads that run handlers, which may block on the wallet.
//!
//! The rule the layout serves: nothing that one peer controls can hold a
//! thread that other peers need.
//!
//! - **Admission.** The accept thread counts connections and connections
//!   still in their handshake, and closes a new socket at once when either
//!   cap is reached (counted in [`Stats::refused`]) rather than queueing it,
//!   so a flood of unauthenticated sockets costs at most `max_handshakes`
//!   threads. Each admitted connection gets one thread for its TLS
//!   handshake; the writer thread is started only once the handshake has
//!   authenticated the client.
//! - **Deadline.** `handshake_timeout` is an absolute deadline from accept to
//!   the client's first SETTINGS, as grpc-go's `connectionTimeout` is
//!   (`rawConn.SetDeadline` before the handshake, cleared after the HTTP/2
//!   preface): every read and write until then is given only the time left,
//!   so a peer trickling bytes cannot extend it.
//! - **Writes.** Only the connection's writer thread writes to its socket.
//!   The reader and the workers append the state machine's output to the
//!   connection's outbox under the state lock (so frames keep the order the
//!   state machine produced them in, which HPACK requires) and return; the
//!   writer takes everything queued, encrypts it under the TLS lock and
//!   writes it with no lock held, which also coalesces responses into fewer
//!   records and system calls. A batch that the peer has not taken within
//!   `write_timeout` closes the connection (grpc-go sets TCP_USER_TIMEOUT to
//!   its 20 s keepalive timeout, which bounds unacknowledged data rather than
//!   a peer that acknowledges and does not read).
//! - **Back-pressure.** The reader stops reading while more than
//!   `max_unsent` bytes wait to be written, which stalls only that
//!   connection (grpc-go throttles its reader the same way when control
//!   frames queue up); the state machine bounds request bytes and in-flight
//!   calls per connection, so the shared job queue holds at most
//!   `max_connections * max_inflight` jobs.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::conn::{Connection, Event, Limits};
use crate::grpc::Router;

/// Server options.
#[derive(Debug, Clone)]
pub struct Options {
    /// HTTP/2 and gRPC limits per connection.
    pub limits: Limits,
    /// Worker threads running handlers.
    pub workers: usize,
    /// Absolute deadline from accept to the client's first SETTINGS: the TLS
    /// handshake and the HTTP/2 preface (grpc-go's connection timeout, 120 s).
    pub handshake_timeout: Duration,
    /// A batch of output the peer has not taken within this long closes the
    /// connection (grpc-go sets TCP_USER_TIMEOUT to 20 s).
    pub write_timeout: Duration,
    /// Connections open at once; a socket past it is closed at accept.
    pub max_connections: usize,
    /// Connections still in their handshake at once; a socket past it is
    /// closed at accept.
    pub max_handshakes: usize,
    /// Output waiting to be written past which the reader stops reading.
    pub max_unsent: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            limits: Limits::default(),
            workers: thread::available_parallelism().map_or(4, |n| n.get() * 2),
            handshake_timeout: Duration::from_secs(120),
            write_timeout: Duration::from_secs(20),
            max_connections: 1024,
            max_handshakes: 128,
            max_unsent: 1 << 20,
        }
    }
}

/// Counters for tests and logs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Sockets closed at accept because a cap was reached.
    pub refused: u64,
    /// Accept calls that failed (the loop backs off after each).
    pub accept_errors: u64,
    /// Threads that could not be started (the connection is closed).
    pub spawn_failures: u64,
    /// Connections closed because the deadline to the first SETTINGS passed.
    pub handshake_timeouts: u64,
    /// Connections closed because a write made no progress in time.
    pub write_timeouts: u64,
    /// Connections open now.
    pub connections: usize,
    /// Connections in their handshake now.
    pub handshaking: usize,
}

#[derive(Default)]
struct Counters {
    refused: AtomicU64,
    accept_errors: AtomicU64,
    spawn_failures: AtomicU64,
    handshake_timeouts: AtomicU64,
    write_timeouts: AtomicU64,
    connections: AtomicUsize,
    handshaking: AtomicUsize,
}

/// One counted connection or handshake, released when dropped.
struct OwnedSlot {
    counters: Arc<Counters>,
    handshake: bool,
}

impl Drop for OwnedSlot {
    fn drop(&mut self) {
        let c = if self.handshake {
            &self.counters.handshaking
        } else {
            &self.counters.connections
        };
        c.fetch_sub(1, Ordering::SeqCst);
    }
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Outbox {
    /// Plaintext frames in the order the state machine produced them.
    plain: Vec<u8>,
    /// Bytes the writer has taken and not finished writing.
    unsent: usize,
    /// rustls has records of its own to send (an alert).
    tls_wants: bool,
    /// Send what is queued, then close_notify, then close.
    closing: bool,
    /// The socket is finished; output is dropped.
    dead: bool,
}

struct Shared {
    state: Mutex<Connection>,
    tls: Mutex<rustls::ServerConnection>,
    outbox: Mutex<Outbox>,
    /// Wakes the writer (output queued) and the reader (output drained).
    cv: Condvar,
    socket: TcpStream,
    write_timeout: Duration,
    counters: Arc<Counters>,
    _slot: OwnedSlot,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// Moves the state machine's pending output to the outbox. Called with
    /// the state lock held; never blocks on the socket.
    fn queue(&self, st: &mut Connection) {
        let out = st.take_output();
        let closed = st.is_closed();
        let mut ob = lock(&self.outbox);
        if ob.dead || ob.closing {
            return;
        }
        if !out.is_empty() {
            ob.plain.extend_from_slice(&out);
        }
        if closed {
            ob.closing = true;
        }
        self.cv.notify_all();
    }

    /// Asks the writer to send what is queued and close.
    fn begin_close(&self) {
        let mut ob = lock(&self.outbox);
        ob.closing = true;
        self.cv.notify_all();
    }

    /// Tells the writer rustls has records to send.
    fn tls_wants_write(&self) {
        let mut ob = lock(&self.outbox);
        ob.tls_wants = true;
        self.cv.notify_all();
    }

    /// Blocks the reader while too much output waits; returns false once
    /// the connection is dead.
    fn wait_drained(&self, max: usize) -> bool {
        let mut ob = lock(&self.outbox);
        while !ob.dead && ob.plain.len() + ob.unsent > max {
            ob = self.cv.wait(ob).unwrap_or_else(|e| e.into_inner());
        }
        !ob.dead
    }

    fn mark_dead(&self) {
        let mut ob = lock(&self.outbox);
        ob.dead = true;
        ob.plain = Vec::new();
        self.cv.notify_all();
        drop(ob);
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    /// Writes `buf` within `write_timeout` in all. SO_SNDTIMEO alone bounds
    /// only one system call, and a socket whose send buffer the kernel keeps
    /// growing takes a few bytes per call, so the deadline covers the whole
    /// batch: a peer must take a batch (at most `max_unsent` plus the
    /// responses in flight) within the timeout.
    fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        let deadline = Instant::now() + self.write_timeout;
        let mut off = 0;
        let mut shortened = false;
        let result = loop {
            if off == buf.len() {
                break Ok(());
            }
            if off > 0 {
                match remaining(deadline) {
                    Ok(left) => {
                        if let Err(e) = self.socket.set_write_timeout(Some(left)) {
                            break Err(e);
                        }
                        shortened = true;
                    }
                    Err(e) => break Err(e),
                }
            }
            match (&self.socket).write(&buf[off..]) {
                Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => off += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => break Err(e),
            }
        };
        if shortened && result.is_ok() {
            self.socket.set_write_timeout(Some(self.write_timeout))?;
        }
        result
    }

    /// The writer thread's loop.
    fn write_loop(&self) {
        loop {
            let (batch, closing) = {
                let mut ob = lock(&self.outbox);
                while !ob.dead && ob.plain.is_empty() && !ob.tls_wants && !ob.closing {
                    ob = self.cv.wait(ob).unwrap_or_else(|e| e.into_inner());
                }
                if ob.dead {
                    return;
                }
                let batch = std::mem::take(&mut ob.plain);
                ob.tls_wants = false;
                ob.unsent = batch.len();
                (batch, ob.closing)
            };
            let cipher = {
                let mut tls = lock(&self.tls);
                let mut cipher = Vec::with_capacity(batch.len() + batch.len() / 64 + 64);
                let mut rest = &batch[..];
                let mut ok = true;
                loop {
                    if !rest.is_empty() {
                        match tls.writer().write(rest) {
                            Ok(0) if !tls.wants_write() => {
                                ok = false;
                                break;
                            }
                            Ok(n) => rest = &rest[n..],
                            Err(_) => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if closing && rest.is_empty() {
                        tls.send_close_notify();
                    }
                    while tls.wants_write() {
                        if tls.write_tls(&mut cipher).is_err() {
                            ok = false;
                            break;
                        }
                    }
                    if rest.is_empty() || !ok {
                        break;
                    }
                }
                if !ok {
                    drop(tls);
                    self.mark_dead();
                    return;
                }
                cipher
            };
            if let Err(e) = self.write_all(&cipher) {
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) {
                    self.counters.write_timeouts.fetch_add(1, Ordering::Relaxed);
                }
                self.mark_dead();
                return;
            }
            let mut ob = lock(&self.outbox);
            ob.unsent = 0;
            self.cv.notify_all();
            if closing {
                drop(ob);
                self.mark_dead();
                return;
            }
        }
    }
}

/// A running server.
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<Weak<Shared>>>>,
    counters: Arc<Counters>,
}

impl Server {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The counters.
    pub fn stats(&self) -> Stats {
        let c = &self.counters;
        Stats {
            refused: c.refused.load(Ordering::Relaxed),
            accept_errors: c.accept_errors.load(Ordering::Relaxed),
            spawn_failures: c.spawn_failures.load(Ordering::Relaxed),
            handshake_timeouts: c.handshake_timeouts.load(Ordering::Relaxed),
            write_timeouts: c.write_timeouts.load(Ordering::Relaxed),
            connections: c.connections.load(Ordering::SeqCst),
            handshaking: c.handshaking.load(Ordering::SeqCst),
        }
    }

    /// Stops accepting and sends GOAWAY NO_ERROR on every connection, which
    /// closes once its open calls finish. Never waits on a peer: each
    /// connection's own writer sends the GOAWAY.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        let conns: Vec<Arc<Shared>> = lock(&self.conns).iter().filter_map(Weak::upgrade).collect();
        for c in conns {
            let mut st = lock(&c.state);
            st.shutdown();
            c.queue(&mut st);
        }
    }
}

/// Serves `router` on `listener` with the TLS configuration `tls`.
pub fn serve(
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    router: Router,
    opts: Options,
) -> io::Result<Server> {
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let conns: Arc<Mutex<Vec<Weak<Shared>>>> = Arc::default();
    let counters = Arc::new(Counters::default());
    let (tx, rx) = mpsc::channel::<Job>();
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..opts.workers.max(1) {
        let rx = Arc::clone(&rx);
        thread::Builder::new()
            .name("grpc-worker".into())
            .spawn(move || {
                loop {
                    let job = lock(&rx).recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                }
            })?;
    }
    let router = Arc::new(router);
    {
        let stop = Arc::clone(&stop);
        let conns = Arc::clone(&conns);
        let counters = Arc::clone(&counters);
        thread::Builder::new()
            .name("grpc-accept".into())
            .spawn(move || {
                for tcp in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let tcp = match tcp {
                        Ok(tcp) => tcp,
                        Err(_) => {
                            // EMFILE and the like: back off instead of
                            // spinning on the same error.
                            counters.accept_errors.fetch_add(1, Ordering::Relaxed);
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    };
                    let accepted = Instant::now();
                    if counters.connections.load(Ordering::SeqCst) >= opts.max_connections
                        || counters.handshaking.load(Ordering::SeqCst) >= opts.max_handshakes
                    {
                        counters.refused.fetch_add(1, Ordering::Relaxed);
                        let _ = tcp.shutdown(Shutdown::Both);
                        continue;
                    }
                    counters.connections.fetch_add(1, Ordering::SeqCst);
                    counters.handshaking.fetch_add(1, Ordering::SeqCst);
                    let conn_slot = OwnedSlot {
                        counters: Arc::clone(&counters),
                        handshake: false,
                    };
                    let hs_slot = OwnedSlot {
                        counters: Arc::clone(&counters),
                        handshake: true,
                    };
                    let tls = Arc::clone(&tls);
                    let router = Arc::clone(&router);
                    let tx = tx.clone();
                    let opts = opts.clone();
                    let conns = Arc::clone(&conns);
                    let spawned =
                        thread::Builder::new()
                            .name("grpc-conn".into())
                            .spawn(move || {
                                let _ = serve_conn(
                                    tcp, accepted, tls, router, tx, opts, conns, conn_slot, hs_slot,
                                );
                            });
                    if spawned.is_err() {
                        // The closure, with the socket and both slots, was
                        // dropped: the connection is closed and uncounted.
                        counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })?;
    }
    Ok(Server {
        addr,
        stop,
        conns,
        counters,
    })
}

/// The time left before `deadline`, or a timeout error.
fn remaining(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "handshake deadline passed",
        ))
    } else {
        Ok(left)
    }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// The TLS handshake, every read and write bounded by what is left of the
/// deadline. A refused handshake sends its alert before failing.
fn handshake(
    tls: &mut rustls::ServerConnection,
    tcp: &TcpStream,
    deadline: Instant,
) -> io::Result<()> {
    let mut sock = tcp;
    let flush = |tls: &mut rustls::ServerConnection, mut sock: &TcpStream| -> io::Result<()> {
        while tls.wants_write() {
            tcp.set_write_timeout(Some(remaining(deadline)?))?;
            tls.write_tls(&mut sock)?;
        }
        Ok(())
    };
    while tls.is_handshaking() {
        flush(tls, sock)?;
        if !tls.is_handshaking() {
            break;
        }
        tcp.set_read_timeout(Some(remaining(deadline)?))?;
        if tls.read_tls(&mut sock)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if let Err(e) = tls.process_new_packets() {
            let _ = flush(tls, sock);
            return Err(io::Error::other(e));
        }
    }
    flush(tls, sock)
}

#[allow(clippy::too_many_arguments)]
fn serve_conn(
    tcp: TcpStream,
    accepted: Instant,
    config: Arc<rustls::ServerConfig>,
    router: Arc<Router>,
    jobs: mpsc::Sender<Job>,
    opts: Options,
    conns: Arc<Mutex<Vec<Weak<Shared>>>>,
    conn_slot: OwnedSlot,
    hs_slot: OwnedSlot,
) -> io::Result<()> {
    let deadline = accepted + opts.handshake_timeout;
    let counters = Arc::clone(&conn_slot.counters);
    let timed_out = |e: io::Error| {
        if is_timeout(&e) {
            counters.handshake_timeouts.fetch_add(1, Ordering::Relaxed);
        }
        e
    };
    tcp.set_nodelay(true)?;
    let mut tls = rustls::ServerConnection::new(config).map_err(io::Error::other)?;
    handshake(&mut tls, &tcp, deadline).map_err(timed_out)?;
    if tls.alpn_protocol() != Some(b"h2") {
        // grpc-go: "credentials: cannot check peer: missing selected ALPN
        // property", and the connection is closed.
        tls.send_close_notify();
        let mut sock = &tcp;
        while tls.wants_write() {
            tcp.set_write_timeout(Some(remaining(deadline)?))?;
            tls.write_tls(&mut sock)?;
        }
        return Ok(());
    }
    tcp.set_write_timeout(Some(opts.write_timeout))?;
    // The writer encrypts everything it takes in one go; what bounds the
    // plaintext is the outbox's own limit.
    tls.set_buffer_limit(None);
    let shared = Arc::new(Shared {
        state: Mutex::new(Connection::new(opts.limits.clone())),
        tls: Mutex::new(tls),
        outbox: Mutex::new(Outbox::default()),
        cv: Condvar::new(),
        socket: tcp.try_clone()?,
        write_timeout: opts.write_timeout,
        counters: Arc::clone(&counters),
        _slot: conn_slot,
    });
    {
        let mut c = lock(&conns);
        c.retain(|w| w.strong_count() > 0);
        c.push(Arc::downgrade(&shared));
    }
    {
        let writer = Arc::clone(&shared);
        if thread::Builder::new()
            .name("grpc-write".into())
            .spawn(move || writer.write_loop())
            .is_err()
        {
            counters.spawn_failures.fetch_add(1, Ordering::Relaxed);
            shared.mark_dead();
            return Ok(());
        }
    }
    {
        let mut st = lock(&shared.state);
        shared.queue(&mut st);
    }
    let result = read_loop(&tcp, &shared, &router, &jobs, &opts, deadline, hs_slot);
    {
        let mut st = lock(&shared.state);
        shared.queue(&mut st);
        st.abort();
    }
    shared.begin_close();
    result.map_err(timed_out)
}

/// Hands a call to the worker pool. The job delivers the response and hands
/// on the calls the freed in-flight slot lets through.
fn submit(shared: &Arc<Shared>, jobs: &mpsc::Sender<Job>, ev: Event) {
    let Event::Call {
        stream,
        handler,
        ctx,
        message,
    } = ev;
    let shared = Arc::clone(shared);
    let next = jobs.clone();
    let job: Job = Box::new(move || {
        let result = handler(&ctx, &message);
        drop(message);
        let more = {
            let mut st = lock(&shared.state);
            let more = st.respond(stream, result);
            shared.queue(&mut st);
            more
        };
        for ev in more {
            submit(&shared, &next, ev);
        }
    });
    let _ = jobs.send(job);
}

fn read_loop(
    tcp: &TcpStream,
    shared: &Arc<Shared>,
    router: &Router,
    jobs: &mpsc::Sender<Job>,
    opts: &Options,
    deadline: Instant,
    hs_slot: OwnedSlot,
) -> io::Result<()> {
    let mut hs_slot = Some(hs_slot);
    let mut reader = tcp;
    let mut buf = vec![0u8; 32 * 1024];
    let mut plain = vec![0u8; 32 * 1024];
    // The first pass reads nothing from the socket: the handshake may have
    // read the client's first frames along with its Finished message, and
    // rustls holds them decrypted until they are taken.
    let mut first = true;
    loop {
        let n = if std::mem::take(&mut first) {
            0
        } else {
            if hs_slot.is_some() {
                tcp.set_read_timeout(Some(remaining(deadline)?))?;
            }
            match reader.read(&mut buf)? {
                0 => return Ok(()),
                n => n,
            }
        };
        let mut pt = Vec::new();
        let mut peer_closed = false;
        let mut wants_write = false;
        {
            let mut tls = lock(&shared.tls);
            let mut slice = &buf[..n];
            // Decrypt what was read, taking the plaintext after each batch
            // of records (rustls bounds what it buffers). On the first pass
            // the slice is empty and only what the handshake left is taken.
            loop {
                if !slice.is_empty() {
                    tls.read_tls(&mut slice)?;
                    if let Err(e) = tls.process_new_packets() {
                        drop(tls);
                        shared.tls_wants_write();
                        return Err(io::Error::other(e));
                    }
                }
                loop {
                    match tls.reader().read(&mut plain) {
                        Ok(0) => {
                            peer_closed = true;
                            break;
                        }
                        Ok(k) => pt.extend_from_slice(&plain[..k]),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    }
                }
                if slice.is_empty() || peer_closed {
                    break;
                }
            }
            if tls.wants_write() {
                wants_write = true;
            }
        }
        if wants_write {
            shared.tls_wants_write();
        }
        let events = {
            let mut st = lock(&shared.state);
            let events = st.recv(&pt, router, Instant::now());
            shared.queue(&mut st);
            if hs_slot.is_some() && st.is_established() {
                // grpc-go clears its connection deadline here; an idle
                // limit for authenticated connections is keepalive's job.
                tcp.set_read_timeout(None)?;
                hs_slot = None;
            }
            if st.is_closed() {
                return Ok(());
            }
            events
        };
        for ev in events {
            submit(shared, jobs, ev);
        }
        if peer_closed {
            return Ok(());
        }
        if !shared.wait_drained(opts.max_unsent) {
            return Ok(());
        }
    }
}
