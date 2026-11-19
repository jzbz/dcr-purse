//! HTTP/2 frame layout (RFC 9113 section 4 and 6): the frame header, the
//! type, flag, setting and error code constants, and frame writers.

/// The client connection preface (RFC 9113 section 3.4).
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Frame header length.
pub const HEADER_LEN: usize = 9;

/// Frame types.
pub mod kind {
    /// DATA
    pub const DATA: u8 = 0x0;
    /// HEADERS
    pub const HEADERS: u8 = 0x1;
    /// PRIORITY
    pub const PRIORITY: u8 = 0x2;
    /// RST_STREAM
    pub const RST_STREAM: u8 = 0x3;
    /// SETTINGS
    pub const SETTINGS: u8 = 0x4;
    /// PUSH_PROMISE
    pub const PUSH_PROMISE: u8 = 0x5;
    /// PING
    pub const PING: u8 = 0x6;
    /// GOAWAY
    pub const GOAWAY: u8 = 0x7;
    /// WINDOW_UPDATE
    pub const WINDOW_UPDATE: u8 = 0x8;
    /// CONTINUATION
    pub const CONTINUATION: u8 = 0x9;
}

/// Frame flags.
pub mod flag {
    /// END_STREAM (DATA, HEADERS)
    pub const END_STREAM: u8 = 0x1;
    /// ACK (SETTINGS, PING)
    pub const ACK: u8 = 0x1;
    /// END_HEADERS (HEADERS, CONTINUATION)
    pub const END_HEADERS: u8 = 0x4;
    /// PADDED (DATA, HEADERS)
    pub const PADDED: u8 = 0x8;
    /// PRIORITY (HEADERS)
    pub const PRIORITY: u8 = 0x20;
}

/// Setting identifiers.
pub mod setting {
    /// SETTINGS_HEADER_TABLE_SIZE
    pub const HEADER_TABLE_SIZE: u16 = 0x1;
    /// SETTINGS_ENABLE_PUSH
    pub const ENABLE_PUSH: u16 = 0x2;
    /// SETTINGS_MAX_CONCURRENT_STREAMS
    pub const MAX_CONCURRENT_STREAMS: u16 = 0x3;
    /// SETTINGS_INITIAL_WINDOW_SIZE
    pub const INITIAL_WINDOW_SIZE: u16 = 0x4;
    /// SETTINGS_MAX_FRAME_SIZE
    pub const MAX_FRAME_SIZE: u16 = 0x5;
    /// SETTINGS_MAX_HEADER_LIST_SIZE
    pub const MAX_HEADER_LIST_SIZE: u16 = 0x6;
}

/// Error codes (RFC 9113 section 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode {
    /// NO_ERROR
    NoError = 0x0,
    /// PROTOCOL_ERROR
    Protocol = 0x1,
    /// INTERNAL_ERROR
    Internal = 0x2,
    /// FLOW_CONTROL_ERROR
    FlowControl = 0x3,
    /// SETTINGS_TIMEOUT
    SettingsTimeout = 0x4,
    /// STREAM_CLOSED
    StreamClosed = 0x5,
    /// FRAME_SIZE_ERROR
    FrameSize = 0x6,
    /// REFUSED_STREAM
    RefusedStream = 0x7,
    /// CANCEL
    Cancel = 0x8,
    /// COMPRESSION_ERROR
    Compression = 0x9,
    /// CONNECT_ERROR
    Connect = 0xa,
    /// ENHANCE_YOUR_CALM
    EnhanceYourCalm = 0xb,
    /// INADEQUATE_SECURITY
    InadequateSecurity = 0xc,
    /// HTTP_1_1_REQUIRED
    Http11Required = 0xd,
}

/// A parsed frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Payload length.
    pub length: u32,
    /// Frame type.
    pub kind: u8,
    /// Flags.
    pub flags: u8,
    /// Stream identifier (reserved bit cleared).
    pub stream: u32,
}

impl Header {
    /// Parses the 9-byte frame header.
    pub fn parse(b: &[u8; HEADER_LEN]) -> Header {
        Header {
            length: u32::from_be_bytes([0, b[0], b[1], b[2]]),
            kind: b[3],
            flags: b[4],
            stream: u32::from_be_bytes([b[5], b[6], b[7], b[8]]) & 0x7fff_ffff,
        }
    }
}

/// Appends a frame header and payload to `out`.
pub fn write(out: &mut Vec<u8>, kind: u8, flags: u8, stream: u32, payload: &[u8]) {
    let len = payload.len() as u32;
    out.extend_from_slice(&len.to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&(stream & 0x7fff_ffff).to_be_bytes());
    out.extend_from_slice(payload);
}

/// Appends a SETTINGS frame.
pub fn settings(out: &mut Vec<u8>, values: &[(u16, u32)]) {
    let mut p = Vec::with_capacity(values.len() * 6);
    for &(id, v) in values {
        p.extend_from_slice(&id.to_be_bytes());
        p.extend_from_slice(&v.to_be_bytes());
    }
    write(out, kind::SETTINGS, 0, 0, &p);
}

/// Appends a GOAWAY frame.
pub fn goaway(out: &mut Vec<u8>, last_stream: u32, code: ErrorCode, debug: &[u8]) {
    let mut p = Vec::with_capacity(8 + debug.len());
    p.extend_from_slice(&last_stream.to_be_bytes());
    p.extend_from_slice(&(code as u32).to_be_bytes());
    p.extend_from_slice(debug);
    write(out, kind::GOAWAY, 0, 0, &p);
}

/// Appends a RST_STREAM frame.
pub fn rst_stream(out: &mut Vec<u8>, stream: u32, code: ErrorCode) {
    write(
        out,
        kind::RST_STREAM,
        0,
        stream,
        &(code as u32).to_be_bytes(),
    );
}

/// Appends a WINDOW_UPDATE frame.
pub fn window_update(out: &mut Vec<u8>, stream: u32, increment: u32) {
    write(
        out,
        kind::WINDOW_UPDATE,
        0,
        stream,
        &increment.to_be_bytes(),
    );
}
