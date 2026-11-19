//! The protobuf (proto3) wire codec for the two VersionService messages, on
//! a small reader and writer the remaining messages would share.
//!
//! Decoding is lenient in proto3's ways (spec 19 section 4.3 item 10):
//! unknown fields are skipped, groups included; a wrong wire type, a
//! truncated varint or length, or a length past the buffer fails the whole
//! message. Encoding emits fields in field-number order and omits scalars
//! equal to their zero value.

/// A wire-format error. The text follows Go's protobuf runtime as far as it
/// can: Go's own text contains a deliberately unstable separator (its
/// `detrand` package picks a space or a no-break space per binary), so no
/// implementation can match it byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireError;

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("proto: cannot parse invalid wire-format data")
    }
}

/// Wire types.
const VARINT: u8 = 0;
const FIXED64: u8 = 1;
const LEN: u8 = 2;
const SGROUP: u8 = 3;
const EGROUP: u8 = 4;
const FIXED32: u8 = 5;

/// Nesting depth past which groups are refused (Go's default recursion
/// limit is 10000; nothing in api.proto uses groups at all).
const MAX_DEPTH: usize = 100;

/// A cursor over a message.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `buf`.
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    /// Whether the message is exhausted.
    pub fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Reads a varint (at most 10 bytes, the tenth at most 1).
    pub fn varint(&mut self) -> Result<u64, WireError> {
        let mut v = 0u64;
        for i in 0..10 {
            let b = *self.buf.get(self.pos).ok_or(WireError)?;
            self.pos += 1;
            if i == 9 && b > 1 {
                return Err(WireError);
            }
            v |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(WireError)
    }

    /// Reads a tag: (field number, wire type).
    pub fn tag(&mut self) -> Result<(u32, u8), WireError> {
        let t = self.varint()?;
        let num = t >> 3;
        if num == 0 || num > 0x1fff_ffff {
            return Err(WireError);
        }
        Ok((num as u32, (t & 7) as u8))
    }

    /// Reads a length-delimited payload.
    pub fn bytes(&mut self) -> Result<&'a [u8], WireError> {
        let len = usize::try_from(self.varint()?).map_err(|_| WireError)?;
        let end = self.pos.checked_add(len).ok_or(WireError)?;
        let s = self.buf.get(self.pos..end).ok_or(WireError)?;
        self.pos = end;
        Ok(s)
    }

    fn fixed(&mut self, n: usize) -> Result<(), WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError)?;
        if end > self.buf.len() {
            return Err(WireError);
        }
        self.pos = end;
        Ok(())
    }

    /// Skips the value of a field with the given number and wire type.
    pub fn skip(&mut self, num: u32, wire: u8) -> Result<(), WireError> {
        self.skip_depth(num, wire, 0)
    }

    fn skip_depth(&mut self, num: u32, wire: u8, depth: usize) -> Result<(), WireError> {
        match wire {
            VARINT => self.varint().map(drop),
            FIXED64 => self.fixed(8),
            LEN => self.bytes().map(drop),
            FIXED32 => self.fixed(4),
            SGROUP => {
                if depth >= MAX_DEPTH {
                    return Err(WireError);
                }
                loop {
                    let (n, w) = self.tag()?;
                    if w == EGROUP {
                        return if n == num { Ok(()) } else { Err(WireError) };
                    }
                    self.skip_depth(n, w, depth + 1)?;
                }
            }
            _ => Err(WireError),
        }
    }
}

/// A message writer.
#[derive(Default)]
pub struct Writer {
    /// The encoded bytes.
    pub buf: Vec<u8>,
}

impl Writer {
    fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.buf.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }

    fn tag(&mut self, num: u32, wire: u8) {
        self.varint((u64::from(num) << 3) | u64::from(wire));
    }

    /// A `string` field, omitted when empty.
    pub fn string(&mut self, num: u32, s: &str) {
        if !s.is_empty() {
            self.tag(num, LEN);
            self.varint(s.len() as u64);
            self.buf.extend_from_slice(s.as_bytes());
        }
    }

    /// A `uint32` field, omitted when zero.
    pub fn uint32(&mut self, num: u32, v: u32) {
        if v != 0 {
            self.tag(num, VARINT);
            self.varint(u64::from(v));
        }
    }
}

/// `walletrpc.VersionRequest`: no fields.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VersionRequest;

impl VersionRequest {
    /// Decodes, skipping every field.
    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(buf);
        while !r.done() {
            let (num, wire) = r.tag()?;
            r.skip(num, wire)?;
        }
        Ok(VersionRequest)
    }
}

/// `walletrpc.VersionResponse`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VersionResponse {
    /// Field 1.
    pub version_string: String,
    /// Field 2.
    pub major: u32,
    /// Field 3.
    pub minor: u32,
    /// Field 4.
    pub patch: u32,
    /// Field 5.
    pub prerelease: String,
    /// Field 6.
    pub build_metadata: String,
}

impl VersionResponse {
    /// Encodes in field-number order.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.string(1, &self.version_string);
        w.uint32(2, self.major);
        w.uint32(3, self.minor);
        w.uint32(4, self.patch);
        w.string(5, &self.prerelease);
        w.string(6, &self.build_metadata);
        w.buf
    }

    /// Decodes (for tests): proto3 merge rules, last scalar wins.
    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut m = VersionResponse::default();
        let mut r = Reader::new(buf);
        while !r.done() {
            let (num, wire) = r.tag()?;
            match (num, wire) {
                (1 | 5 | 6, LEN) => {
                    let s = std::str::from_utf8(r.bytes()?).map_err(|_| WireError)?;
                    let s = s.to_owned();
                    match num {
                        1 => m.version_string = s,
                        5 => m.prerelease = s,
                        _ => m.build_metadata = s,
                    }
                }
                (2..=4, VARINT) => {
                    let v = r.varint()? as u32;
                    match num {
                        2 => m.major = v,
                        3 => m.minor = v,
                        _ => m.patch = v,
                    }
                }
                (1..=6, _) => return Err(WireError),
                _ => r.skip(num, wire)?,
            }
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_response_bytes_match_spec() {
        // Spec 19 section 5.5: 0a 06 "10.0.0" 10 0a.
        let r = VersionResponse {
            version_string: "10.0.0".into(),
            major: 10,
            ..Default::default()
        };
        assert_eq!(r.encode(), b"\x0a\x0610.0.0\x10\x0a");
        assert_eq!(VersionResponse::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn request_leniency_and_errors() {
        assert!(VersionRequest::decode(&[]).is_ok());
        // Unknown fields of every wire type, and a group.
        assert!(
            VersionRequest::decode(&[
                0x08, 0x96, 0x01, 0x11, 1, 2, 3, 4, 5, 6, 7, 8, 0x1a, 0x01, 0xff, 0x25, 1, 2, 3, 4,
                0x2b, 0x08, 0x01, 0x2c
            ])
            .is_ok()
        );
        for bad in [
            &[0x0a, 0x05][..], // length past the end
            &[0x08],           // truncated varint
            &[0x00, 0x00],     // field number 0
            &[0x0f],           // wire type 7
            &[0x0c],           // end group with no start
            &[0x0b, 0x14],     // mismatched group end
            &[
                0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02,
            ], // 11-bit tenth byte
        ] {
            assert_eq!(VersionRequest::decode(bad), Err(WireError), "{bad:02x?}");
        }
    }
}
