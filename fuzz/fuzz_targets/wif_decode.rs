// SPDX-License-Identifier: ISC
//! WIF fuzz target: `Wif::decode` on the four networks, and the string
//! round trip of whatever decodes.
//!
//! WIF strings come from users and RPC clients (`importprivkey`), and their
//! base58 decoder is the crate's own wiped, constant-time-lookup codec.
//! Nothing may panic. A string that decodes with a known signature type
//! prints as the string it came from (decoded to 39 bytes behind nonzero
//! network bytes, it has no leading `1` and so is base58's one spelling of
//! its value).
//!
//! The first byte picks the mode. Raw text exercises the length, base58,
//! checksum and network layers, which mutation cannot get past; the other
//! mode builds a WIF with `Wif::new` from the input's bytes as the key, so
//! every input reaches the encoder and a decode of its output: a 32-byte
//! key comes back as it went in, and a key of any other length gives a
//! string the decoder refuses as malformed, as Go's does (spec 24, Q-27).

#![no_main]

use libfuzzer_sys::fuzz_target;
use purse_dcrutil::wif::{SignatureType, Wif, WifError};

/// The four networks' `PrivateKeyID`: mainnet, testnet3, simnet, regnet.
const NETS: [[u8; 2]; 4] = [[0x22, 0xde], [0x23, 0x0e], [0x23, 0x07], [0x22, 0xfe]];

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        let net = NETS[usize::from(mode >> 1) % NETS.len()];
        let scheme = SignatureType(i64::from((mode >> 3) % 3));
        let Ok(wif) = Wif::new(rest, net, scheme) else {
            return;
        };
        let text = wif.string();
        match Wif::decode(&text, net) {
            Ok(back) => {
                assert_eq!(rest.len(), 32, "a key of another length decoded");
                assert_eq!(back.priv_key(), rest);
                assert_eq!(back.pub_key(), wif.pub_key());
                assert_eq!(back.dsa(), scheme);
                assert_eq!(back.net_id(), net);
                assert_eq!(*back.string(), *text);
            }
            Err(e) => {
                assert_ne!(rest.len(), 32, "a 32-byte key did not round-trip: {e}");
                assert_eq!(e, WifError::MalformedPrivateKey);
            }
        }
        return;
    }
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    for net in NETS {
        if let Ok(wif) = Wif::decode(text, net) {
            assert_eq!(wif.net_id(), net);
            // An unknown signature type byte decodes to an empty key, which
            // prints as another string (QUIRKS QK-0515).
            if !wif.priv_key().is_empty() {
                assert_eq!(*wif.string(), *text);
            }
        }
    }
});
