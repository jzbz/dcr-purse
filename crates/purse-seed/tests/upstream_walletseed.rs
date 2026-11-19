// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2016 The Decred developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Port of dcrwallet's `walletseed/seed_test.go` with its vectors.

use purse_seed::walletseed::{decode_user_input, encode_mnemonic, encode_mnemonic_slice};
use purse_testutil::hex;

struct Vector {
    mnemonics: &'static str,
    data: &'static [u8],
}

const MNEMONIC_TESTS: &[Vector] = &[
    Vector {
        mnemonics: "topmost Istanbul Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell bottomless fracture",
        data: &[
            0xE5, 0x82, 0x94, 0xF2, 0xE9, 0xA2, 0x27, 0x48, 0x6E, 0x8B, 0x06, 0x1B, 0x31, 0xCC,
            0x52, 0x8F, 0xD7, 0xFA, 0x3F, 0x19,
        ],
    },
    Vector {
        mnemonics: "stairway souvenir flytrap recipe adrift upcoming artist positive spearhead Pandora spaniel stupendous tonic concurrent transit Wichita lockup visitor flagpole escapade merit",
        data: &[
            0xD1, 0xD4, 0x64, 0xC0, 0x04, 0xF0, 0x0F, 0xB5, 0xC9, 0xA4, 0xC8, 0xD8, 0xE4, 0x33,
            0xE7, 0xFB, 0x7F, 0xF5, 0x62, 0x56,
        ],
    },
    Vector {
        mnemonics: "tissue disbelief stairway component atlas megaton bedlamp certify tumor monument necklace fascinate tunnel fascinate dreadful armistice upshot Apollo exceed aftermath billiard sardonic vapor microscope brackish suspicious woodlark torpedo hamlet sensation assume recipe",
        data: &[
            0xE3, 0x4C, 0xD1, 0x32, 0x12, 0x8C, 0x19, 0x29, 0xEC, 0x96, 0x86, 0x5C, 0xED, 0x5C,
            0x4D, 0x0B, 0xF4, 0x0A, 0x5D, 0x02, 0x1F, 0xCE, 0xF5, 0x8D, 0x27, 0xDB, 0xFE, 0xE3,
            0x71, 0xD2, 0x10,
        ],
    },
    Vector {
        mnemonics: "aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark adroitness aardvark insurgent",
        data: &[0x00; 31],
    },
];

/// `TestEncodeMnemonicSlice`.
#[test]
fn encode_mnemonic_slice_vectors() {
    for (i, test) in MNEMONIC_TESTS.iter().enumerate() {
        let words = encode_mnemonic_slice(test.data);
        assert_eq!(words.join().as_str(), test.mnemonics, "test {i}");
    }
}

/// `TestEncodeMnemonic`.
#[test]
fn encode_mnemonic_vectors() {
    for (i, test) in MNEMONIC_TESTS.iter().enumerate() {
        assert_eq!(
            encode_mnemonic(test.data).as_str(),
            test.mnemonics,
            "test {i}"
        );
    }
}

/// `TestDecodeMnemonic`.
#[test]
fn decode_mnemonic_vectors() {
    for (i, test) in MNEMONIC_TESTS.iter().enumerate() {
        let data =
            decode_user_input(test.mnemonics).unwrap_or_else(|e| panic!("test {i}: error: {e}"));
        assert_eq!(data.as_bytes(), test.data, "test {i}");
    }
}

/// `TestDecodeHex`.
#[test]
fn decode_hex_vectors() {
    for (i, test) in MNEMONIC_TESTS.iter().enumerate() {
        let data =
            decode_user_input(&hex(test.data)).unwrap_or_else(|e| panic!("test {i}: error: {e}"));
        assert_eq!(data.as_bytes(), test.data, "test {i}");
    }
}
