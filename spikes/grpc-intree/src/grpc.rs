//! The gRPC layer over HTTP/2: status codes and their trailer encoding,
//! request header checks in grpc-go v1.83.2's order, the length-prefixed
//! message framing, the method router and the call context handlers get.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// gRPC status codes used here.
pub mod code {
    /// OK
    pub const OK: u32 = 0;
    /// UNKNOWN
    pub const UNKNOWN: u32 = 2;
    /// INVALID_ARGUMENT
    pub const INVALID_ARGUMENT: u32 = 3;
    /// DEADLINE_EXCEEDED
    pub const DEADLINE_EXCEEDED: u32 = 4;
    /// RESOURCE_EXHAUSTED
    pub const RESOURCE_EXHAUSTED: u32 = 8;
    /// UNIMPLEMENTED
    pub const UNIMPLEMENTED: u32 = 12;
    /// INTERNAL
    pub const INTERNAL: u32 = 13;
}

/// The largest request message accepted (grpc-go's default, 4 MiB).
pub const MAX_RECV_MESSAGE: usize = 4 * 1024 * 1024;

/// A status: code and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The code.
    pub code: u32,
    /// The message, unencoded.
    pub message: String,
}

impl Status {
    /// A status with a code and message.
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Status {
            code,
            message: message.into(),
        }
    }
}

/// Percent-encodes a status message for `grpc-message` exactly as grpc-go's
/// `encodeGrpcMessage` does: bytes from space to tilde but `%` pass, every
/// other byte of a valid UTF-8 sequence is `%XX`, and an invalid byte is the
/// encoding of U+FFFD, `%EF%BF%BD`.
pub fn encode_grpc_message(msg: &str) -> String {
    let ok = |b: u8| (b' '..=b'~').contains(&b) && b != b'%';
    if msg.bytes().all(ok) {
        return msg.to_owned();
    }
    let mut out = String::with_capacity(msg.len() * 3);
    for ch in msg.chars() {
        let mut buf = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buf).as_bytes();
        if bytes.len() == 1 && ok(bytes[0]) {
            out.push(ch);
        } else {
            for b in bytes {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Formats `s` as Go's `%q` does for the strings this server quotes (valid
/// UTF-8 header values and paths): printable characters as they are, `"`
/// and `\` escaped, the usual C escapes, other control characters as `\xNN`
/// or `\uNNNN`.
pub fn go_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// grpc-go's `decodeTimeout`, with its error texts.
pub fn decode_timeout(s: &str) -> Result<Duration, String> {
    let size = s.len();
    if size < 2 {
        return Err(format!(
            "transport: timeout string is too short: {}",
            go_quote(s)
        ));
    }
    if size > 9 {
        return Err(format!(
            "transport: timeout string is too long: {}",
            go_quote(s)
        ));
    }
    let unit = match s.as_bytes()[size - 1] {
        b'H' => Duration::from_secs(3600),
        b'M' => Duration::from_secs(60),
        b'S' => Duration::from_secs(1),
        b'm' => Duration::from_millis(1),
        b'u' => Duration::from_micros(1),
        b'n' => Duration::from_nanos(1),
        _ => {
            return Err(format!(
                "transport: timeout unit is not recognized: {}",
                go_quote(s)
            ));
        }
    };
    let digits = &s[..size - 1];
    // strconv.ParseUint(digits, 10, 64): at most eight digits fit here, so
    // only the syntax can fail.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "strconv.ParseUint: parsing {}: invalid syntax",
            go_quote(digits)
        ));
    }
    let n: u32 = digits.parse().map_err(|_| "overflow".to_owned())?;
    Ok(unit * n)
}

/// Whether `content-type` names gRPC, and its subtype (grpcutil's
/// `ContentSubtype`).
pub fn content_subtype(ct: &str) -> Option<&str> {
    const BASE: &str = "application/grpc";
    if ct == BASE {
        return Some("");
    }
    let rest = ct.strip_prefix(BASE)?;
    match rest.as_bytes().first() {
        Some(b'+') | Some(b';') => Some(&rest[1..]),
        _ => None,
    }
}

/// The response content type for a request subtype.
pub fn content_type(subtype: &str) -> String {
    if subtype.is_empty() {
        "application/grpc".to_owned()
    } else {
        format!("application/grpc+{subtype}")
    }
}

/// What a handler is given besides the request bytes.
#[derive(Debug, Clone)]
pub struct CallContext {
    /// The full method, `/package.Service/Method`.
    pub method: String,
    /// The deadline from `grpc-timeout`, if any.
    pub deadline: Option<Instant>,
    /// Request metadata (non-reserved headers), in arrival order.
    pub metadata: Vec<(String, String)>,
    /// Set when the client resets the stream or the connection closes: a
    /// handler doing long blocking work polls it.
    pub cancelled: Arc<AtomicBool>,
}

impl CallContext {
    /// Whether the call has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// A unary handler: request bytes in, response bytes or a status out. It
/// runs on a worker thread and may block.
pub type Handler = Arc<dyn Fn(&CallContext, &[u8]) -> Result<Vec<u8>, Status> + Send + Sync>;

/// Methods by service.
#[derive(Default, Clone)]
pub struct Router {
    services: HashMap<String, HashMap<String, Handler>>,
}

impl Router {
    /// Registers a unary method.
    pub fn unary(
        mut self,
        service: &str,
        method: &str,
        h: impl Fn(&CallContext, &[u8]) -> Result<Vec<u8>, Status> + Send + Sync + 'static,
    ) -> Self {
        self.services
            .entry(service.to_owned())
            .or_default()
            .insert(method.to_owned(), Arc::new(h));
        self
    }

    /// Resolves a `:path` as grpc-go's `handleStream` does, with its
    /// Unimplemented statuses for a malformed name, an unknown service and
    /// an unknown method.
    pub fn route(&self, path: &str) -> Result<Handler, Status> {
        let malformed = || {
            Status::new(
                code::UNIMPLEMENTED,
                format!("malformed method name: {}", go_quote(path)),
            )
        };
        let sm = path.strip_prefix('/').ok_or_else(malformed)?;
        let pos = sm.rfind('/').ok_or_else(malformed)?;
        let (service, method) = (&sm[..pos], &sm[pos + 1..]);
        match self.services.get(service) {
            None => Err(Status::new(
                code::UNIMPLEMENTED,
                format!("unknown service {service}"),
            )),
            Some(methods) => methods.get(method).cloned().ok_or_else(|| {
                Status::new(
                    code::UNIMPLEMENTED,
                    format!("unknown method {method} for service {service}"),
                )
            }),
        }
    }
}

/// Frames one message: flag 0, big-endian length, payload.
pub fn frame_message(msg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + msg.len());
    out.push(0);
    out.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    out.extend_from_slice(msg);
    out
}

/// The outcome of looking at a request body received so far.
#[derive(Debug, PartialEq, Eq)]
pub enum Body {
    /// More bytes are needed.
    Incomplete,
    /// One complete message (the payload, without the prefix).
    Message(Vec<u8>),
    /// The request fails with this status.
    Fail(Status),
}

/// Examines the bytes received on a unary stream, as grpc-go's `recvMsg`
/// and `checkRecvPayload` do with no compressor registered. `ended` says
/// whether the client has half-closed.
pub fn parse_body(buf: &[u8], ended: bool) -> Body {
    if buf.len() < 5 {
        if !ended {
            return Body::Incomplete;
        }
        return Body::Fail(if buf.is_empty() {
            // io.EOF through status.Convert.
            Status::new(code::UNKNOWN, "EOF")
        } else {
            Status::new(code::UNKNOWN, "unexpected EOF")
        });
    }
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if len > MAX_RECV_MESSAGE {
        return Body::Fail(Status::new(
            code::RESOURCE_EXHAUSTED,
            format!("grpc: received message larger than max ({len} vs. {MAX_RECV_MESSAGE})"),
        ));
    }
    if buf.len() < 5 + len {
        if ended {
            return Body::Fail(Status::new(code::UNKNOWN, "unexpected EOF"));
        }
        return Body::Incomplete;
    }
    match buf[0] {
        0 => Body::Message(buf[5..5 + len].to_vec()),
        1 => Body::Fail(Status::new(
            code::INTERNAL,
            "grpc: compressed flag set with identity or empty encoding",
        )),
        pf => Body::Fail(Status::new(
            code::INTERNAL,
            format!("grpc: received unexpected payload format {pf}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_message_encoding() {
        assert_eq!(encode_grpc_message("plain text"), "plain text");
        assert_eq!(encode_grpc_message("100%"), "100%25");
        assert_eq!(encode_grpc_message("a\nb"), "a%0Ab");
        assert_eq!(encode_grpc_message("é"), "%C3%A9");
    }

    #[test]
    fn timeouts() {
        assert_eq!(decode_timeout("100m"), Ok(Duration::from_millis(100)));
        assert_eq!(decode_timeout("1H"), Ok(Duration::from_secs(3600)));
        assert!(decode_timeout("1").unwrap_err().contains("too short"));
        assert!(
            decode_timeout("123456789S")
                .unwrap_err()
                .contains("too long")
        );
        assert!(
            decode_timeout("10x")
                .unwrap_err()
                .contains("not recognized")
        );
        assert_eq!(
            decode_timeout("a1S").unwrap_err(),
            "strconv.ParseUint: parsing \"a1\": invalid syntax"
        );
    }

    #[test]
    fn routes() {
        let r = Router::default().unary("walletrpc.VersionService", "Version", |_, _| Ok(vec![]));
        assert!(r.route("/walletrpc.VersionService/Version").is_ok());
        assert_eq!(
            r.route("/walletrpc.VersionService/X")
                .err()
                .unwrap()
                .message,
            "unknown method X for service walletrpc.VersionService"
        );
        assert_eq!(
            r.route("/nope/X").err().unwrap().message,
            "unknown service nope"
        );
        assert_eq!(
            r.route("nope").err().unwrap().message,
            "malformed method name: \"nope\""
        );
    }

    #[test]
    fn bodies() {
        assert_eq!(parse_body(&[], false), Body::Incomplete);
        assert_eq!(
            parse_body(&[], true),
            Body::Fail(Status::new(code::UNKNOWN, "EOF"))
        );
        assert_eq!(parse_body(&[0, 0, 0, 0, 0], false), Body::Message(vec![]));
        assert!(
            matches!(parse_body(&[0, 0x01, 0, 0, 0], false), Body::Fail(s) if s.code == code::RESOURCE_EXHAUSTED)
        );
    }
}
