//! HPACK (RFC 7541): the decoder with its static and dynamic tables and
//! Huffman decoding, and a stateless encoder.
//!
//! The decoder works as x/net's `hpack.Decoder` does for grpc-go: a header
//! block is fed one HEADERS or CONTINUATION fragment at a time
//! ([`Decoder::write`]), every complete field is handed to the caller's
//! callback as borrowed bytes, and a representation cut by a fragment
//! boundary is kept until the next fragment arrives. The caller decides
//! which fields to keep, so an indexed reference to a large table entry (the
//! "HPACK bomb") costs nothing unless the caller copies it, and the caller's
//! header-list budget is what bounds the copies. The dynamic table is bounded
//! by the size this endpoint advertised (4096, the default it never
//! changes); a single name or value is bounded by [`Decoder::set_max_string`]
//! (x/net's `SetMaxStringLength`, which grpc-go sets to its 16 MiB header
//! list limit), which also bounds what a cut representation can make the
//! decoder hold between fragments.
//!
//! The encoder never inserts into the dynamic table and never Huffman-codes,
//! so it keeps no state the peer's SETTINGS_HEADER_TABLE_SIZE could affect.

use std::collections::VecDeque;

use crate::huffman::{self, HuffmanError};

/// The static table (RFC 7541, Appendix A); index 1 is the first entry.
pub const STATIC_TABLE: [(&str, &str); 61] = [
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

/// The per-entry overhead RFC 7541 section 4.1 adds to name and value.
const ENTRY_OVERHEAD: usize = 32;

/// The dynamic table size this endpoint allows (SETTINGS_HEADER_TABLE_SIZE,
/// left at its default).
pub const DEFAULT_TABLE_SIZE: usize = 4096;

/// One decoded header field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The name, as sent.
    pub name: Vec<u8>,
    /// The value, as sent.
    pub value: Vec<u8>,
}

/// A decoding failure: always a connection error of type COMPRESSION_ERROR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpackError {
    /// Index 0, or an index past the end of both tables.
    BadIndex,
    /// An integer longer than this decoder accepts.
    IntegerOverflow,
    /// The block ended inside a representation.
    Truncated,
    /// A Huffman-coded string that does not decode.
    Huffman(HuffmanError),
    /// A dynamic table size update above the advertised maximum.
    SizeUpdateTooLarge,
    /// A dynamic table size update after the first field of a block.
    SizeUpdateNotAtStart,
    /// A name or value longer than the decoder's string limit.
    StringTooLong,
}

/// What decoding a header block produced besides the fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// The header list size (RFC 7540 section 6.5.2) of every field in the
    /// block, including those not materialized.
    pub list_size: usize,
    /// Whether fields were dropped because the list exceeded the limit.
    pub truncated: bool,
}

/// The decoder: one per connection, fed every header block in order.
#[derive(Debug)]
pub struct Decoder {
    dynamic: VecDeque<Field>,
    size: usize,
    max_size: usize,
    protocol_max: usize,
    max_string: usize,
    /// The start of a representation a fragment boundary cut.
    saved: Vec<u8>,
    /// Whether the current block has had a field (size updates must come
    /// first).
    seen_field: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(DEFAULT_TABLE_SIZE)
    }
}

impl Decoder {
    /// A decoder allowing a dynamic table of at most `protocol_max` octets
    /// and strings of any length.
    pub fn new(protocol_max: usize) -> Self {
        Decoder {
            dynamic: VecDeque::new(),
            size: 0,
            max_size: protocol_max,
            protocol_max,
            max_string: usize::MAX,
            saved: Vec::new(),
            seen_field: false,
        }
    }

    /// Limits the length of one name or value, after Huffman decoding.
    pub fn set_max_string(&mut self, n: usize) {
        self.max_string = n;
    }

    /// The dynamic table's current size in octets (for tests and the
    /// robustness checks).
    pub fn table_size(&self) -> usize {
        self.size
    }

    /// Bytes held between fragments (an incomplete representation).
    pub fn saved_len(&self) -> usize {
        self.saved.len()
    }

    fn get(&self, index: usize) -> Result<(&[u8], &[u8]), HpackError> {
        match index {
            0 => Err(HpackError::BadIndex),
            1..=61 => {
                let (n, v) = STATIC_TABLE[index - 1];
                Ok((n.as_bytes(), v.as_bytes()))
            }
            _ => self
                .dynamic
                .get(index - 62)
                .map(|f| (f.name.as_slice(), f.value.as_slice()))
                .ok_or(HpackError::BadIndex),
        }
    }

    fn evict_to(&mut self, max: usize) {
        while self.size > max {
            match self.dynamic.pop_back() {
                Some(f) => self.size -= f.name.len() + f.value.len() + ENTRY_OVERHEAD,
                None => break,
            }
        }
    }

    fn insert(&mut self, field: Field) {
        let entry = field.name.len() + field.value.len() + ENTRY_OVERHEAD;
        if entry > self.max_size {
            self.dynamic.clear();
            self.size = 0;
            return;
        }
        self.evict_to(self.max_size - entry);
        self.size += entry;
        self.dynamic.push_front(field);
    }

    /// Decodes one fragment of a header block, calling `emit` with the name
    /// and value of every field it completes. A representation the fragment
    /// ends inside is kept for the next call.
    pub fn write(
        &mut self,
        fragment: &[u8],
        emit: &mut dyn FnMut(&[u8], &[u8]),
    ) -> Result<(), HpackError> {
        if self.saved.is_empty() {
            let used = self.decode_complete(fragment, emit)?;
            self.saved.extend_from_slice(&fragment[used..]);
        } else {
            let mut buf = std::mem::take(&mut self.saved);
            buf.extend_from_slice(fragment);
            let used = self.decode_complete(&buf, emit)?;
            buf.drain(..used);
            self.saved = buf;
        }
        // x/net's last resort against a representation that never ends: a
        // cut string can be no longer than the limit plus its length prefix.
        if self.max_string != usize::MAX
            && self.saved.len() > 2 * (self.max_string.saturating_add(8))
        {
            return Err(HpackError::StringTooLong);
        }
        Ok(())
    }

    /// Ends a header block: an unfinished representation is an error.
    pub fn finish(&mut self) -> Result<(), HpackError> {
        self.seen_field = false;
        if self.saved.is_empty() {
            Ok(())
        } else {
            self.saved = Vec::new();
            Err(HpackError::Truncated)
        }
    }

    /// Decodes one complete header block, appending at most `limit` octets
    /// of header list (by RFC 7540's measure) to `out`; past the limit,
    /// fields are decoded for the dynamic table but not kept.
    pub fn decode(
        &mut self,
        block: &[u8],
        limit: usize,
        out: &mut Vec<Field>,
    ) -> Result<Decoded, HpackError> {
        let mut list_size = 0usize;
        let mut truncated = false;
        self.write(block, &mut |name, value| {
            list_size = list_size.saturating_add(name.len() + value.len() + ENTRY_OVERHEAD);
            if list_size <= limit {
                out.push(Field {
                    name: name.to_vec(),
                    value: value.to_vec(),
                });
            } else {
                truncated = true;
            }
        })?;
        self.finish()?;
        Ok(Decoded {
            list_size,
            truncated,
        })
    }

    /// Decodes every complete representation in `buf`, returning how many
    /// bytes they took. A representation is applied (to the table, to
    /// `emit`) only once all of it has been read, so stopping inside one
    /// leaves no trace of it.
    fn decode_complete(
        &mut self,
        buf: &[u8],
        emit: &mut dyn FnMut(&[u8], &[u8]),
    ) -> Result<usize, HpackError> {
        let mut pos = 0;
        while pos < buf.len() {
            let start = pos;
            match self.one(buf, &mut pos, emit) {
                Ok(()) => {}
                Err(HpackError::Truncated) => return Ok(start),
                Err(e) => return Err(e),
            }
        }
        Ok(pos)
    }

    fn one(
        &mut self,
        buf: &[u8],
        pos: &mut usize,
        emit: &mut dyn FnMut(&[u8], &[u8]),
    ) -> Result<(), HpackError> {
        let b = buf[*pos];
        if b & 0x80 != 0 {
            // Indexed header field.
            let index = decode_int(buf, pos, 7)?;
            let (name, value) = self.get(index)?;
            emit(name, value);
            self.seen_field = true;
        } else if b & 0xe0 == 0x20 {
            // Dynamic table size update.
            if self.seen_field {
                return Err(HpackError::SizeUpdateNotAtStart);
            }
            let new = decode_int(buf, pos, 5)?;
            if new > self.protocol_max {
                return Err(HpackError::SizeUpdateTooLarge);
            }
            self.max_size = new;
            self.evict_to(new);
        } else {
            // Literal: with incremental indexing (01), without indexing
            // (0000) or never indexed (0001).
            let (prefix, index_it) = if b & 0xc0 == 0x40 {
                (6, true)
            } else {
                (4, false)
            };
            let index = decode_int(buf, pos, prefix)?;
            let name = if index == 0 {
                decode_string(buf, pos, self.max_string)?
            } else {
                self.get(index)?.0.to_vec()
            };
            let value = decode_string(buf, pos, self.max_string)?;
            emit(&name, &value);
            self.seen_field = true;
            if index_it {
                self.insert(Field { name, value });
            }
        }
        Ok(())
    }
}

/// Decodes an HPACK integer with an `n`-bit prefix (RFC 7541 section 5.1).
/// Values above 2^28 are refused; nothing legitimate is that large.
fn decode_int(buf: &[u8], pos: &mut usize, n: u32) -> Result<usize, HpackError> {
    let mask = (1u32 << n) - 1;
    let first = *buf.get(*pos).ok_or(HpackError::Truncated)?;
    *pos += 1;
    let mut value = u32::from(first) & mask;
    if value < mask {
        return Ok(value as usize);
    }
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos).ok_or(HpackError::Truncated)?;
        *pos += 1;
        if shift > 21 {
            return Err(HpackError::IntegerOverflow);
        }
        value = value
            .checked_add(u32::from(b & 0x7f) << shift)
            .ok_or(HpackError::IntegerOverflow)?;
        if value > 1 << 28 {
            return Err(HpackError::IntegerOverflow);
        }
        shift += 7;
        if b & 0x80 == 0 {
            return Ok(value as usize);
        }
    }
}

fn decode_string(buf: &[u8], pos: &mut usize, max: usize) -> Result<Vec<u8>, HpackError> {
    let huffman = *buf.get(*pos).ok_or(HpackError::Truncated)? & 0x80 != 0;
    let len = decode_int(buf, pos, 7)?;
    if !huffman && len > max {
        return Err(HpackError::StringTooLong);
    }
    let end = pos.checked_add(len).ok_or(HpackError::Truncated)?;
    let raw = buf.get(*pos..end).ok_or(HpackError::Truncated)?;
    *pos = end;
    if huffman {
        let mut out = Vec::with_capacity((len * 8 / 5 + 1).min(max));
        if !huffman::decode(raw, &mut out, max).map_err(HpackError::Huffman)? {
            return Err(HpackError::StringTooLong);
        }
        Ok(out)
    } else {
        Ok(raw.to_vec())
    }
}

fn encode_int(out: &mut Vec<u8>, first_bits: u8, n: u32, mut value: usize) {
    let mask = (1usize << n) - 1;
    if value < mask {
        out.push(first_bits | value as u8);
        return;
    }
    out.push(first_bits | mask as u8);
    value -= mask;
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn encode_string(out: &mut Vec<u8>, s: &[u8]) {
    encode_int(out, 0, 7, s.len());
    out.extend_from_slice(s);
}

/// Encodes header fields with no dynamic table: a field equal to a static
/// entry is indexed, a field whose name is in the static table is a literal
/// without indexing on that name, and any other is a literal without
/// indexing with its name.
pub fn encode(fields: &[(&str, &str)], out: &mut Vec<u8>) {
    for &(name, value) in fields {
        if let Some(i) = STATIC_TABLE
            .iter()
            .position(|&(n, v)| n == name && v == value)
        {
            encode_int(out, 0x80, 7, i + 1);
        } else if let Some(i) = STATIC_TABLE.iter().position(|&(n, _)| n == name) {
            encode_int(out, 0x00, 4, i + 1);
            encode_string(out, value.as_bytes());
        } else {
            out.push(0x00);
            encode_string(out, name.as_bytes());
            encode_string(out, value.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn fields(v: &[Field]) -> Vec<(String, String)> {
        v.iter()
            .map(|f| {
                (
                    String::from_utf8(f.name.clone()).unwrap(),
                    String::from_utf8(f.value.clone()).unwrap(),
                )
            })
            .collect()
    }

    /// RFC 7541 C.4: requests with Huffman coding, one decoder.
    #[test]
    fn rfc7541_c4() {
        let mut d = Decoder::default();
        let blocks = [
            "8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff",
            "8286 84be 5886 a8eb 1064 9cbf",
            "8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf",
        ];
        let want: [&[(&str, &str)]; 3] = [
            &[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
            ],
            &[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
                ("cache-control", "no-cache"),
            ],
            &[
                (":method", "GET"),
                (":scheme", "https"),
                (":path", "/index.html"),
                (":authority", "www.example.com"),
                ("custom-key", "custom-value"),
            ],
        ];
        for (block, want) in blocks.iter().zip(want) {
            let mut out = Vec::new();
            d.decode(&unhex(block), usize::MAX, &mut out).unwrap();
            let want: Vec<(String, String)> = want
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
            assert_eq!(fields(&out), want);
        }
        assert_eq!(d.table_size(), 164);
    }

    /// RFC 7541 C.3.1: the same first request without Huffman coding.
    #[test]
    fn rfc7541_c3_1() {
        let mut d = Decoder::default();
        let mut out = Vec::new();
        d.decode(
            &unhex("8286 8441 0f77 7777 2e65 7861 6d70 6c65 2e63 6f6d"),
            usize::MAX,
            &mut out,
        )
        .unwrap();
        assert_eq!(fields(&out)[3].1, "www.example.com");
        assert_eq!(d.table_size(), 57);
    }

    #[test]
    fn errors() {
        let mut d = Decoder::default();
        let mut out = Vec::new();
        assert_eq!(d.decode(&[0x80], 100, &mut out), Err(HpackError::BadIndex));
        assert_eq!(d.decode(&[0xbe], 100, &mut out), Err(HpackError::BadIndex));
        assert_eq!(
            d.decode(&[0xff, 0xff, 0xff, 0xff, 0xff, 0x7f], 100, &mut out),
            Err(HpackError::IntegerOverflow)
        );
        assert_eq!(
            d.decode(&[0x40, 0x05, b'a'], 100, &mut out),
            Err(HpackError::Truncated)
        );
        // Size update above 4096.
        assert_eq!(
            d.decode(&[0x3f, 0xe2, 0x1f], 100, &mut out),
            Err(HpackError::SizeUpdateTooLarge)
        );
        // Size update after a field.
        assert_eq!(
            d.decode(&[0x82, 0x20], 100, &mut out),
            Err(HpackError::SizeUpdateNotAtStart)
        );
    }

    #[test]
    fn list_limit_truncates_without_losing_table_state() {
        let mut d = Decoder::default();
        let mut out = Vec::new();
        // Insert a 100-byte value, then reference it 1000 times.
        let mut block = vec![0x40, 0x01, b'x', 0x64];
        block.extend(std::iter::repeat_n(b'v', 100));
        block.extend(std::iter::repeat_n(0xbe, 1000));
        let r = d.decode(&block, 1000, &mut out).unwrap();
        assert!(r.truncated);
        assert!(out.len() < 10);
        assert_eq!(r.list_size, 1001 * (1 + 100 + 32));
        assert_eq!(d.table_size(), 133);
    }

    /// Fed one byte at a time, or cut at any point, a block decodes to the
    /// same fields and leaves the same table as fed whole.
    #[test]
    fn fragments_decode_as_the_whole_block() {
        let blocks = [
            unhex("8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff"),
            unhex("8286 84be 5886 a8eb 1064 9cbf"),
            unhex("8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf"),
        ];
        let whole: Vec<Vec<(Vec<u8>, Vec<u8>)>> = {
            let mut d = Decoder::default();
            blocks
                .iter()
                .map(|b| {
                    let mut v = Vec::new();
                    d.write(b, &mut |n, val| v.push((n.to_vec(), val.to_vec())))
                        .unwrap();
                    d.finish().unwrap();
                    v
                })
                .collect()
        };
        for cut in 1..8 {
            let mut d = Decoder::default();
            for (b, want) in blocks.iter().zip(&whole) {
                let mut v = Vec::new();
                for piece in b.chunks(cut) {
                    d.write(piece, &mut |n, val| v.push((n.to_vec(), val.to_vec())))
                        .unwrap();
                }
                d.finish().unwrap();
                assert_eq!(&v, want, "cut {cut}");
            }
            assert_eq!(d.table_size(), 164);
        }
    }

    #[test]
    fn unfinished_block_and_long_strings_are_errors() {
        let mut d = Decoder::default();
        d.write(&[0x40, 0x05, b'a'], &mut |_, _| {}).unwrap();
        assert_eq!(d.saved_len(), 3);
        assert_eq!(d.finish(), Err(HpackError::Truncated));
        let mut d = Decoder::default();
        d.set_max_string(4);
        assert_eq!(
            d.write(&[0x00, 0x05, b'a'], &mut |_, _| {}),
            Err(HpackError::StringTooLong)
        );
        // A Huffman string over the limit once decoded: "www.example.com".
        let mut d = Decoder::default();
        d.set_max_string(4);
        assert_eq!(
            d.write(
                &unhex("0001 618c f1e3 c2e5 f23a 6ba0 ab90 f4ff"),
                &mut |_, _| {}
            ),
            Err(HpackError::StringTooLong)
        );
    }

    #[test]
    fn encoder_round_trips() {
        let f = [
            (":status", "200"),
            ("content-type", "application/grpc"),
            ("grpc-status", "0"),
            ("grpc-message", ""),
        ];
        let mut block = Vec::new();
        encode(&f, &mut block);
        let mut out = Vec::new();
        Decoder::default()
            .decode(&block, usize::MAX, &mut out)
            .unwrap();
        let got = fields(&out);
        let want: Vec<(String, String)> = f
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(got, want);
    }
}
