//! The HPACK Huffman code (RFC 7541, Appendix B).
//!
//! The code is canonical: codes of one length are consecutive and assigned in
//! symbol order, shorter lengths first. So the table is stored as the code
//! length of each of the 257 symbols (256 octets and EOS) and the codes are
//! derived from it. The decoder is the canonical one, a bit at a time, with
//! the three checks RFC 7541 section 5.2 requires: EOS must not appear, the
//! padding must be shorter than 8 bits and must be the most significant bits
//! of EOS (all ones).

use std::sync::OnceLock;

/// Code length in bits of each symbol, 0..=255 then EOS (256).
const LENGTHS: [u8; 257] = [
    13, 23, 28, 28, 28, 28, 28, 28, 28, 24, 30, 28, 28, 30, 28, 28, //
    28, 28, 28, 28, 28, 28, 30, 28, 28, 28, 28, 28, 28, 28, 28, 28, //
    6, 10, 10, 12, 13, 6, 8, 11, 10, 10, 8, 11, 8, 6, 6, 6, //
    5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 7, 8, 15, 6, 12, 10, //
    13, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, //
    7, 7, 7, 7, 7, 7, 7, 7, 8, 7, 8, 13, 19, 13, 14, 6, //
    15, 5, 6, 5, 6, 5, 6, 6, 6, 5, 7, 7, 6, 6, 6, 5, //
    6, 7, 6, 5, 5, 6, 7, 7, 7, 7, 7, 15, 11, 14, 13, 28, //
    20, 22, 20, 20, 22, 22, 22, 23, 22, 23, 23, 23, 23, 23, 24, 23, //
    24, 24, 22, 23, 24, 23, 23, 23, 23, 21, 22, 23, 22, 23, 23, 24, //
    22, 21, 20, 22, 22, 23, 23, 21, 23, 22, 22, 24, 21, 22, 23, 23, //
    21, 21, 22, 21, 23, 22, 23, 23, 20, 22, 22, 22, 23, 22, 22, 23, //
    26, 26, 20, 19, 22, 23, 22, 25, 26, 26, 26, 27, 27, 26, 24, 25, //
    19, 21, 26, 27, 27, 26, 27, 24, 21, 21, 26, 26, 28, 27, 27, 27, //
    20, 24, 20, 21, 22, 21, 21, 23, 22, 22, 25, 25, 24, 24, 26, 23, //
    26, 27, 26, 26, 27, 27, 27, 27, 27, 28, 27, 27, 27, 27, 27, 26, //
    30,
];

const MAX_LEN: usize = 30;
const EOS: u16 = 256;

struct Tables {
    /// Number of codes of each length.
    count: [u32; MAX_LEN + 1],
    /// First code of each length.
    first: [u32; MAX_LEN + 1],
    /// Index into `symbols` of the first symbol of each length.
    offset: [u32; MAX_LEN + 1],
    /// Symbols sorted by (length, symbol).
    symbols: [u16; 257],
    /// Code of each symbol, for the encoder.
    codes: [u32; 257],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = Tables {
            count: [0; MAX_LEN + 1],
            first: [0; MAX_LEN + 1],
            offset: [0; MAX_LEN + 1],
            symbols: [0; 257],
            codes: [0; 257],
        };
        for &len in &LENGTHS {
            t.count[usize::from(len)] += 1;
        }
        // The canonical rule: first[1] = 0 and
        // first[l] = (first[l-1] + count[l-1]) << 1.
        let mut code = 0u32;
        let mut index = 0u32;
        for len in 1..=MAX_LEN {
            if len > 1 {
                code = (code + t.count[len - 1]) << 1;
            }
            t.first[len] = code;
            t.offset[len] = index;
            index += t.count[len];
        }
        let mut next = t.first;
        let mut pos = t.offset;
        for len in 1..=MAX_LEN {
            for sym in 0..257u16 {
                if usize::from(LENGTHS[usize::from(sym)]) == len {
                    t.symbols[pos[len] as usize] = sym;
                    pos[len] += 1;
                    t.codes[usize::from(sym)] = next[len];
                    next[len] += 1;
                }
            }
        }
        t
    })
}

/// A Huffman decoding failure (an HPACK COMPRESSION_ERROR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuffmanError {
    /// The EOS symbol appeared in the string.
    Eos,
    /// More than 7 bits of padding.
    PaddingTooLong,
    /// Padding that is not the most significant bits of EOS.
    PaddingNotOnes,
}

/// Decodes a Huffman-coded string, appending to `out`. `limit` bounds the
/// decoded length: decoding stops with `Ok(false)` once `out` would exceed
/// it, so a caller enforcing a header-list limit never allocates past it.
pub fn decode(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<bool, HuffmanError> {
    let t = tables();
    let mut code = 0u32;
    let mut len = 0usize;
    for &byte in input {
        for shift in (0..8).rev() {
            code = (code << 1) | u32::from((byte >> shift) & 1);
            len += 1;
            let index = code.wrapping_sub(t.first[len]);
            if index < t.count[len] {
                let sym = t.symbols[(t.offset[len] + index) as usize];
                if sym == EOS {
                    return Err(HuffmanError::Eos);
                }
                if out.len() >= limit {
                    return Ok(false);
                }
                out.push(sym as u8);
                code = 0;
                len = 0;
            } else if len == MAX_LEN {
                // Every 30-bit code is assigned, so this cannot happen with a
                // complete code; kept as a guard.
                return Err(HuffmanError::Eos);
            }
        }
    }
    if len > 7 {
        return Err(HuffmanError::PaddingTooLong);
    }
    if code != (1u32 << len) - 1 {
        return Err(HuffmanError::PaddingNotOnes);
    }
    Ok(true)
}

/// Huffman-encodes `input` (used by tests and by clients; the server sends
/// raw literals).
pub fn encode(input: &[u8]) -> Vec<u8> {
    let t = tables();
    let mut out = Vec::new();
    let mut acc = 0u64;
    let mut bits = 0u32;
    for &b in input {
        let len = u32::from(LENGTHS[usize::from(b)]);
        acc = (acc << len) | u64::from(t.codes[usize::from(b)]);
        bits += len;
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1u64 << bits) - 1;
    }
    if bits > 0 {
        let pad = 8 - bits;
        out.push(((acc << pad) as u8) | ((1u8 << pad) - 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7541_c4_examples() {
        let cases: &[(&str, &str)] = &[
            ("f1e3c2e5f23a6ba0ab90f4ff", "www.example.com"),
            ("a8eb10649cbf", "no-cache"),
            ("25a849e95ba97d7f", "custom-key"),
            ("25a849e95bb8e8b4bf", "custom-value"),
            ("6402", "302"),
            ("aec3771a4b", "private"),
            (
                "d07abe941054d444a8200595040b8166e082a62d1bff",
                "Mon, 21 Oct 2013 20:13:21 GMT",
            ),
            (
                "9d29ad171863c78f0b97c8e9ae82ae43d3",
                "https://www.example.com",
            ),
        ];
        for (hex, text) in cases {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let mut out = Vec::new();
            assert_eq!(decode(&bytes, &mut out, usize::MAX), Ok(true));
            assert_eq!(out, text.as_bytes());
            assert_eq!(encode(text.as_bytes()), bytes);
        }
    }

    #[test]
    fn padding_rules() {
        // 'a' is 00011 (5 bits); padding 111 is valid.
        assert_eq!(decode(&[0b0001_1111], &mut Vec::new(), 10), Ok(true));
        // Padding with a zero bit.
        assert_eq!(
            decode(&[0b0001_1110], &mut Vec::new(), 10),
            Err(HuffmanError::PaddingNotOnes)
        );
        // A full byte of ones is 8 bits of padding.
        assert_eq!(
            decode(&[0b0001_1111, 0xff], &mut Vec::new(), 10),
            Err(HuffmanError::PaddingTooLong)
        );
        // EOS itself (30 ones) inside the string.
        assert_eq!(
            decode(&[0xff, 0xff, 0xff, 0xff], &mut Vec::new(), 10),
            Err(HuffmanError::Eos)
        );
    }

    #[test]
    fn round_trip_all_octets() {
        let all: Vec<u8> = (0..=255u8).collect();
        let mut out = Vec::new();
        assert_eq!(decode(&encode(&all), &mut out, usize::MAX), Ok(true));
        assert_eq!(out, all);
    }
}
