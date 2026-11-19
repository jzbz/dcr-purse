// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2015-2016 The decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `pgpwordlist/pgpwordlist_test.go` with its vectors.

use purse_seed::pgpwordlist::{byte_to_mnemonic, decode_mnemonics};

struct Vector {
    mnemonics: &'static str,
    data: &'static [u8],
}

const TESTS: &[Vector] = &[
    Vector {
        mnemonics: "topmost Istanbul Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell bottomless",
        data: &[
            0xE5, 0x82, 0x94, 0xF2, 0xE9, 0xA2, 0x27, 0x48, 0x6E, 0x8B, 0x06, 0x1B, 0x31, 0xCC,
            0x52, 0x8F, 0xD7, 0xFA, 0x3F, 0x19,
        ],
    },
    Vector {
        mnemonics: "stairway souvenir flytrap recipe adrift upcoming artist positive spearhead Pandora spaniel stupendous tonic concurrent transit Wichita lockup visitor flagpole escapade",
        data: &[
            0xD1, 0xD4, 0x64, 0xC0, 0x04, 0xF0, 0x0F, 0xB5, 0xC9, 0xA4, 0xC8, 0xD8, 0xE4, 0x33,
            0xE7, 0xFB, 0x7F, 0xF5, 0x62, 0x56,
        ],
    },
];

/// `TestDecode`.
#[test]
fn decode() {
    for test in TESTS {
        let words: Vec<&str> = test.mnemonics.split(' ').collect();
        let data = decode_mnemonics(&words).expect("decode");
        assert_eq!(
            data.as_bytes(),
            test.data,
            "decoded data differs from expected"
        );
    }
}

/// `TestEncodeMnemonics`.
#[test]
fn encode_mnemonics() {
    for test in TESTS {
        let words: Vec<&str> = test.mnemonics.split(' ').collect();
        for (i, &b) in test.data.iter().enumerate() {
            assert_eq!(
                byte_to_mnemonic(b, i),
                words[i],
                "returned mnemonic differs from expected"
            );
        }
    }
}
