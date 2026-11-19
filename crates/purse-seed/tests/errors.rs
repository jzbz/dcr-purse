// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The shape of the error chains, as dcrwallet's `errors.E` calls build
//! them (`pgpwordlist/pgpwordlist.go:47-53`, `walletseed/seed.go:24-30,
//! 81-99`), matching as dcrwallet's consumers match them, the redaction of
//! quoted words, and agreement with purse-hdkeychain on the seed lengths
//! and `ErrInvalidSeedLen`'s text.
//!
//! The rendered text and outermost kind of every error are compared with Go
//! by `oracle.rs` and `frozen.rs`; the chain shapes here follow the Go
//! source, which the oracle does not report.

use purse_errors::{AnyError, DAEMON_SEPARATOR, Error, Kind, MULTILINE_SEPARATOR, Op, e};
use purse_seed::pgpwordlist::{OP_DECODE_MNEMONICS, decode_mnemonics};
use purse_seed::walletseed::{
    MAX_SEED_BYTES, MIN_SEED_BYTES, OP_DECODE_USER_INPUT, OP_GENERATE_RANDOM_SEED,
    RECOMMENDED_SEED_LEN, RandomSourceError, decode_user_input, encode_mnemonic,
    generate_random_seed, generate_random_seed_with, go_hex_decode,
};
use purse_seed::{ERR_INVALID_SEED_LEN, HexError, INVALID_SEED_LEN_TEXT, SeedErrorExt, WordError};

/// The number of `Error` levels in the chain.
fn depth(e: &Error) -> usize {
    match &e.err {
        Some(AnyError::Error(inner)) => depth(inner).saturating_add(1),
        _ => 1,
    }
}

fn decode_user_input_err(input: &str) -> Error {
    match decode_user_input(input) {
        Ok(_) => panic!("{input:?} decoded"),
        Err(e) => e,
    }
}

#[test]
fn seed_lengths_and_sentinel_text_equal_hdkeychain() {
    assert_eq!(
        MIN_SEED_BYTES,
        usize::from(purse_hdkeychain::MIN_SEED_BYTES)
    );
    assert_eq!(
        MAX_SEED_BYTES,
        usize::from(purse_hdkeychain::MAX_SEED_BYTES)
    );
    assert_eq!(
        RECOMMENDED_SEED_LEN,
        usize::from(purse_hdkeychain::RECOMMENDED_SEED_LEN)
    );
    assert_eq!(
        INVALID_SEED_LEN_TEXT,
        purse_hdkeychain::Error::InvalidSeedLen.to_string()
    );
    assert_eq!(
        INVALID_SEED_LEN_TEXT,
        purse_hdkeychain::SeedError::<std::convert::Infallible>::InvalidSeedLen.to_string()
    );
    assert_eq!(
        ERR_INVALID_SEED_LEN.render(DAEMON_SEPARATOR),
        INVALID_SEED_LEN_TEXT
    );
    assert!(matches!(&*ERR_INVALID_SEED_LEN, AnyError::Text(_)));
}

/// `DecodeMnemonics`: `E(op, Encoding, Errorf(...))`, one level, the word
/// error as the leaf.
#[test]
fn decode_mnemonics_chain() {
    let e = decode_mnemonics(&["aardvark", " NotAWord\t"]).expect_err("unknown word");
    assert_eq!(e.op, OP_DECODE_MNEMONICS);
    assert_eq!(e.op.as_str(), "pgpwordlist.DecodeMnemonics");
    assert_eq!(e.kind, Kind::Encoding);
    assert_eq!(depth(&e), 1);
    let w = e.find_foreign::<WordError>().expect("word leaf");
    assert!(matches!(w, WordError::UnknownWord { .. }));
    assert_eq!(w.word(), "NotAWord");
    assert!(e.leaf().is_some_and(|l| matches!(l, AnyError::Foreign(_))));
    assert!(!e.is_invalid_seed_len());

    let e = decode_mnemonics(&["aardvark", "aardvark"]).expect_err("wrong parity");
    let w = e.find_foreign::<WordError>().expect("word leaf");
    assert!(matches!(w, WordError::WrongPosition { position: 1, .. }));
    assert_eq!(w.word(), "aardvark");
    assert_eq!(
        e.render(DAEMON_SEPARATOR),
        "pgpwordlist.DecodeMnemonics: invalid encoding: word aardvark is not valid at \
         position 1, check for missing words"
    );
}

/// `DecodeUserInput` with a bad word: `E(op, Encoding, err)` around the
/// `DecodeMnemonics` error, which stays a nested level.
#[test]
fn decode_user_input_word_chain() {
    let e = decode_user_input_err("aardvark adroitness bogus");
    assert_eq!(e.op, OP_DECODE_USER_INPUT);
    assert_eq!(e.kind, Kind::Encoding);
    assert_eq!(depth(&e), 2);
    let Some(AnyError::Error(inner)) = &e.err else {
        panic!("no nested level");
    };
    assert_eq!(inner.op, OP_DECODE_MNEMONICS);
    assert_eq!(inner.kind, Kind::Encoding);
    assert_eq!(
        e.find_foreign::<WordError>().map(WordError::word),
        Some("bogus")
    );
    // Matching as Go's errors.Is with templates and kinds.
    assert!(e.is_kind(Kind::Encoding));
    assert_eq!(e.as_kind(), Some(Kind::Encoding));
    assert!(e.matches(&e!(OP_DECODE_USER_INPUT, Kind::Encoding)));
    assert!(e.matches(&e!(OP_DECODE_MNEMONICS)));
    assert!(e.matches(&Error::from_parts(
        Op::NONE,
        Kind::Other,
        Some(AnyError::from(e!(OP_DECODE_MNEMONICS, Kind::Encoding)))
    )));
    assert!(!e.matches(&e!(OP_GENERATE_RANDOM_SEED)));
    assert!(!e.is_invalid_seed_len());
}

/// `DecodeUserInput` with hex input: the `encoding/hex` error as the leaf.
#[test]
fn decode_user_input_hex_chains() {
    for (input, want) in [
        ("abc", HexError::OddLength),
        ("abg", HexError::InvalidByte(b'g')),
        ("0x00", HexError::InvalidByte(b'x')),
    ] {
        let e = decode_user_input_err(input);
        assert_eq!(
            (e.op.clone(), e.kind, depth(&e)),
            (OP_DECODE_USER_INPUT, Kind::Encoding, 1)
        );
        assert_eq!(e.find_foreign::<HexError>(), Some(&want), "{input}");
        assert_eq!(go_hex_decode(input.as_bytes()).err(), Some(want));
        assert!(!e.is_invalid_seed_len());
    }
}

/// `DecodeUserInput`'s checksum failure: a string argument, so a text leaf.
#[test]
fn decode_user_input_checksum_chain() {
    let seed = [7u8; 16];
    let mnemonic = encode_mnemonic(&seed);
    let mut words: Vec<&str> = mnemonic.as_str().split(' ').collect();
    // Another even-position word for the last data byte.
    words[14] = if words[14] == "aardvark" {
        "absurd"
    } else {
        "aardvark"
    };
    let e = decode_user_input_err(&words.join(" "));
    assert_eq!(
        (e.op.clone(), e.kind, depth(&e)),
        (OP_DECODE_USER_INPUT, Kind::Encoding, 1)
    );
    let Some(AnyError::Text(t)) = &e.err else {
        panic!("leaf is not a text error: {e:?}");
    };
    assert_eq!(t.as_str(), "checksum mismatch");
    // Go's errors.Is with a template whose leaf is a text compares texts.
    assert!(e.matches(&e!(OP_DECODE_USER_INPUT, "checksum mismatch")));
}

/// Every wrong-length path wraps the one shared sentinel, under kind
/// Invalid (`GenerateRandomSeed`) or Encoding (`DecodeUserInput`).
#[test]
fn invalid_seed_len_is_the_shared_sentinel() {
    let short_mnemonic = encode_mnemonic(&[1u8; 15]);
    let long_hex = "00".repeat(65);
    for input in ["", "   ", "00", long_hex.as_str(), short_mnemonic.as_str()] {
        let e = decode_user_input_err(input);
        assert_eq!(
            (e.op.clone(), e.kind, depth(&e)),
            (OP_DECODE_USER_INPUT, Kind::Encoding, 1)
        );
        assert!(e.is_invalid_seed_len(), "{input:?}");
        assert!(e.is(&ERR_INVALID_SEED_LEN));
        assert!(
            e.err
                .as_ref()
                .is_some_and(|l| l.identical(&ERR_INVALID_SEED_LEN))
        );
    }
    let mut never = |_: &mut [u8]| -> Result<(), RandomSourceError> {
        panic!("the source must not be read for a bad size")
    };
    for size in [0, 15, 65, 255, usize::MAX] {
        let e = generate_random_seed_with(size, &mut never).expect_err("bad size");
        assert_eq!(
            (e.op.clone(), e.kind, depth(&e)),
            (OP_GENERATE_RANDOM_SEED, Kind::Invalid, 1)
        );
        assert!(e.is_invalid_seed_len());
        assert!(
            generate_random_seed(size)
                .expect_err("bad size")
                .is_invalid_seed_len()
        );
    }
}

/// dcrwallet's gRPC `errorCode` (`internal/rpc/rpcserver/server.go:87-136`)
/// takes any error and asks `errors.Is(err, hdkeychain.ErrInvalidSeedLen)`
/// after the kind switch; the question works on the chain however it was
/// wrapped since, and only for the sentinel itself.
#[test]
fn seed_service_style_matching() {
    let e = decode_user_input_err("");
    let any = AnyError::from(e.clone());
    assert!(any.is_invalid_seed_len());
    let rewrapped = AnyError::from(e!(Op::new("rpcserver.Something"), e));
    assert!(rewrapped.is_invalid_seed_len());
    let joined =
        purse_errors::join([Some(purse_errors::new("x")), Some(rewrapped)]).expect("members");
    assert!(joined.is_invalid_seed_len());
    let lookalike = AnyError::from(e!(
        OP_DECODE_USER_INPUT,
        Kind::Encoding,
        INVALID_SEED_LEN_TEXT
    ));
    assert_eq!(
        lookalike.render(DAEMON_SEPARATOR),
        any.render(DAEMON_SEPARATOR)
    );
    assert!(!lookalike.is_invalid_seed_len());
}

/// The links a chain reporter prints: the error, then each `source()`.
fn source_links(e: &Error) -> Vec<String> {
    let mut links = vec![e.to_string()];
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        links.push(s.to_string());
        src = s.source();
    }
    links
}

/// `source()` follows Go's `Unwrap`: the nested level, then the word leaf.
/// Each link's `Display` quotes the word, so a reporter printing the chain
/// prints it once per link, as "What is wiped" documents; this pins the
/// counts given there.
#[test]
fn source_chain_quotes_the_word_once_per_link() {
    let e = decode_user_input_err("aardvark adroitness bogus");
    let links = source_links(&e);
    assert_eq!(links.len(), 3, "{links:?}");
    assert!(links.iter().all(|l| l.contains("bogus")), "{links:?}");
    let mut src = std::error::Error::source(&e);
    let nested = src.and_then(|s| s.downcast_ref::<Error>()).expect("level");
    assert_eq!(nested.op, OP_DECODE_MNEMONICS);
    src = src.and_then(std::error::Error::source);
    assert!(src.and_then(|s| s.downcast_ref::<WordError>()).is_some());

    let e = decode_mnemonics(&["bogus"]).expect_err("unknown word");
    let links = source_links(&e);
    assert_eq!(links.len(), 2, "{links:?}");
    assert!(links.iter().all(|l| l.contains("bogus")), "{links:?}");

    // A wrong length: the outer level, then the sentinel.
    assert_eq!(source_links(&decode_user_input_err("")).len(), 2);
}

/// `is` with an `Error` template and `matches` are the same question
/// (Go's `errors.Is` with an `*Error` target), text comparison of a leaf
/// included, as "What is wiped" documents them together.
#[test]
fn is_with_an_error_template_agrees_with_matches() {
    let word = decode_user_input_err("aardvark adroitness bogus");
    let checksum = {
        let seed = [7u8; 16];
        let mnemonic = encode_mnemonic(&seed);
        let mut words: Vec<&str> = mnemonic.as_str().split(' ').collect();
        words[14] = if words[14] == "aardvark" {
            "absurd"
        } else {
            "aardvark"
        };
        decode_user_input_err(&words.join(" "))
    };
    let leaf_text = word.leaf().expect("leaf").to_string();
    let templates = [
        e!(OP_DECODE_USER_INPUT, Kind::Encoding),
        e!(OP_DECODE_USER_INPUT, "checksum mismatch"),
        e!(OP_DECODE_USER_INPUT, "something else"),
        e!(OP_DECODE_MNEMONICS, leaf_text.as_str()),
        e!(OP_DECODE_USER_INPUT, word.err.clone()),
        e!(OP_GENERATE_RANDOM_SEED),
    ];
    let mut hits = 0usize;
    for candidate in [&word, &checksum] {
        let any = AnyError::from(candidate.clone());
        for t in &templates {
            let target = AnyError::from(t.clone());
            let m = candidate.matches(t);
            assert_eq!(candidate.is(&target), m, "{t:?}");
            assert_eq!(any.matches(t), m, "{t:?}");
            assert_eq!(any.is(&target), m, "{t:?}");
            hits = hits.saturating_add(usize::from(m));
        }
    }
    // Templates 1, 4 and 5 match the word error; 1 and 2 the checksum one.
    assert_eq!(hits, 5);
}

/// No `Debug` output of any error that quotes a word contains it; the
/// wiped rendering is sized exactly and equals the plain one.
#[test]
fn quoted_words_stay_out_of_debug() {
    let secret = "Tissue";
    for e in [
        decode_user_input_err(&format!("aardvark adroitness {secret}x")),
        // The same word at an even and an odd position: one is wrong.
        decode_user_input_err(&format!("{secret} {secret}")),
        decode_mnemonics(&[secret, secret]).expect_err("wrong parity"),
    ] {
        let any = AnyError::from(e.clone());
        for debug in [format!("{e:?}"), format!("{e:#?}"), format!("{any:?}")] {
            assert!(!debug.contains(secret), "{debug}");
            assert!(debug.contains("<redacted>"), "{debug}");
        }
        for sep in [DAEMON_SEPARATOR, MULTILINE_SEPARATOR] {
            let text = e.render(sep);
            assert!(text.contains(secret));
            let wiped = e.render_zeroizing(sep);
            assert_eq!(wiped.as_str(), text);
            assert_eq!(wiped.capacity(), wiped.len());
            assert_eq!(any.render_zeroizing(sep).as_str(), text);
        }
    }
}
