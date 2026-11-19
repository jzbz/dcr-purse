// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the oracle and frozen-vector tests: the Rust side's outcomes in
//! the oracle's JSON shapes, and the input corpora.

#![allow(dead_code)]

use purse_errors::{DAEMON_SEPARATOR, Kind};
use purse_seed::pgpwordlist::decode_mnemonics;
use purse_seed::walletseed::{
    decode_user_input, encode_mnemonic, encode_mnemonic_slice, generate_random_seed,
    generate_random_seed_with,
};
use purse_seed::{Error, SeedErrorExt, WORD_LIST};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{OracleError, SplitMix64, hex};

/// The frozen vector file, relative to the crate.
pub const FROZEN_PATH: &str = "tests/data/seed_vectors.json";

/// The fixed PRNG seed the frozen corpus is generated from.
pub const FROZEN_RNG_SEED: u64 = 0x5eed_d0c5_2026_0001;

/// The oracle's name for a kind: the Go constant's name for the kinds these
/// packages use, `Kind(<n>)` for any other (`kindName` in
/// tools/oracle/cmd/seed/main.go).
fn kind_name(k: Kind) -> String {
    match k {
        Kind::Other => "Other".to_owned(),
        Kind::Invalid => "Invalid".to_owned(),
        Kind::Encoding => "Encoding".to_owned(),
        other => format!("Kind({})", other.code()),
    }
}

/// The oracle's kind of an error: that of the first `*Error` in the chain
/// (Go's `errors.As(err, &e)`), which is the error itself.
fn error_kind(e: &Error) -> String {
    let first = e.as_error().expect("an Error is its own first Error");
    kind_name(first.kind)
}

/// The text of an error under `separator`, or the daemon's when `None` (the
/// oracle's default). The process-wide separator is never consulted, so
/// tests running in parallel cannot disturb each other. Every rendering
/// path must give the same bytes: `render`, `render_to` (which `Display`
/// uses) and the wiped rendering.
fn error_text(e: &Error, separator: Option<&str>) -> String {
    let sep = separator.unwrap_or(DAEMON_SEPARATOR);
    let text = e.render(sep);
    let mut streamed = String::new();
    e.render_to(&mut streamed, sep)
        .expect("a String takes everything");
    assert_eq!(streamed, text);
    assert_eq!(e.render_zeroizing(sep).as_str(), text);
    text
}

/// An error outcome: `{"error": text, "kind": name}`.
fn error_value(e: &Error, separator: Option<&str>) -> Value {
    json!({ "error": error_text(e, separator), "kind": error_kind(e) })
}

/// The oracle's answer as an outcome value: its result, or the error shape.
pub fn oracle_outcome(r: Result<Value, OracleError>) -> Value {
    match r {
        Ok(v) => v,
        Err(e) => json!({ "error": e.text, "kind": e.kind }),
    }
}

/// `encode` on the Rust side.
pub fn rust_encode(seed: &[u8]) -> Value {
    let words: Vec<&str> = encode_mnemonic_slice(seed).iter().collect();
    json!({ "mnemonic": encode_mnemonic(seed).as_str(), "words": words })
}

/// `decode-mnemonics` on the Rust side.
pub fn rust_decode_mnemonics(words: &[String], separator: Option<&str>) -> Value {
    match decode_mnemonics(words) {
        Ok(b) => json!({ "decoded": hex(b.as_bytes()) }),
        Err(e) => error_value(&e, separator),
    }
}

/// `decode-user-input` on the Rust side.
pub fn rust_decode_user_input(input: &str, separator: Option<&str>) -> Value {
    match decode_user_input(input) {
        Ok(b) => json!({ "seed": hex(b.as_bytes()) }),
        Err(e) => error_value(&e, separator),
    }
}

/// `generate-random-seed` on the Rust side.
pub fn rust_generate(size: u64) -> Value {
    let size = usize::try_from(size).unwrap_or(usize::MAX);
    match generate_random_seed(size) {
        Ok(b) => json!({ "len": b.len() }),
        Err(e) => error_value(&e, None),
    }
}

/// `generate-random-seed-injected` on the Rust side: `generate_random_seed_with`
/// on a source serving `start, start+1, ...` (mod 256), with the number of
/// bytes it served.
pub fn rust_generate_injected(size: u64, start: u8) -> Value {
    let size = usize::try_from(size).unwrap_or(usize::MAX);
    let next = std::cell::Cell::new(start);
    let served = std::cell::Cell::new(0usize);
    let mut source = |buf: &mut [u8]| {
        for b in buf.iter_mut() {
            *b = next.get();
            next.set(next.get().wrapping_add(1));
        }
        served.set(served.get().saturating_add(buf.len()));
        Ok(())
    };
    match generate_random_seed_with(size, &mut source) {
        Ok(b) => json!({ "seed": hex(b.as_bytes()), "served": served.get() }),
        Err(e) => json!({
            "error": error_text(&e, None),
            "kind": error_kind(&e),
            "served": served.get(),
        }),
    }
}

/// `(size, start)` cases for `generate-random-seed-injected`: every legal
/// size with a random start, every start for the recommended size, and
/// invalid sizes.
pub fn injected_cases(rng: &mut SplitMix64) -> Vec<(u64, u8)> {
    let mut v: Vec<(u64, u8)> = (16u64..=64)
        .map(|size| (size, rng.next_u64().to_le_bytes()[0]))
        .collect();
    v.extend((0u8..=255).map(|start| (32, start)));
    for size in [0, 1, 15, 65, 254, 255, 256, u64::MAX] {
        v.push((size, rng.next_u64().to_le_bytes()[0]));
    }
    v
}

/// The request args for a decode command, with the optional separator.
pub fn decode_args(key: &str, value: Value, separator: Option<&str>) -> Value {
    let mut args = json!({ key: value });
    if let Some(sep) = separator {
        args["separator"] = json!(sep);
    }
    args
}

/// White space Go's `TrimSpace` removes, and characters it does not.
const SPACES: &[&str] = &[
    " ", "  ", "\t", "\n", "\r\n", "\u{0B}", "\u{0C}", "\u{85}", "\u{A0}", "\u{1680}", "\u{2003}",
    "\u{2028}", "\u{3000}",
];
const NOT_SPACES: &[&str] = &["\u{200B}", "\u{FEFF}", "\u{180E}", "\u{0}"];

fn pick<'a>(rng: &mut SplitMix64, items: &[&'a str]) -> &'a str {
    let i = usize::try_from(rng.below(items.len() as u64)).unwrap_or(0);
    items[i]
}

fn rand_len(rng: &mut SplitMix64, lo: usize, hi: usize) -> usize {
    let span = hi.saturating_sub(lo).saturating_add(1) as u64;
    lo.saturating_add(usize::try_from(rng.below(span)).unwrap_or(0))
}

fn mnemonic_words(seed: &[u8]) -> Vec<String> {
    encode_mnemonic_slice(seed)
        .iter()
        .map(str::to_owned)
        .collect()
}

/// The list index of `w` (ASCII case-insensitive), if it is a word.
fn word_list_index(w: &str) -> Option<usize> {
    WORD_LIST.iter().position(|x| x.eq_ignore_ascii_case(w))
}

/// Mixed-case version of `s`, one coin flip per ASCII letter.
fn mix_case(rng: &mut SplitMix64, s: &str) -> String {
    s.chars()
        .map(|c| {
            if rng.below(2) == 0 {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

/// Applies one random mutation to a mnemonic's words.
fn mutate_words(rng: &mut SplitMix64, words: &mut Vec<String>) -> &'static str {
    let n = words.len();
    let at = |rng: &mut SplitMix64| usize::try_from(rng.below(n as u64)).unwrap_or(0);
    match rng.below(12) {
        0 => {
            // Wrong checksum: another word of the same parity.
            let last = n.saturating_sub(1);
            if let Some(idx) = word_list_index(&words[last]) {
                let step = rng.below(255).wrapping_add(1).wrapping_mul(2);
                let other = (idx as u64).wrapping_add(step) & 511;
                words[last] = WORD_LIST[other as usize].to_owned();
            }
            "wrong checksum"
        }
        1 => {
            let i = at(rng);
            words[i].push('s');
            "unknown word (suffix)"
        }
        2 => {
            let i = at(rng);
            words[i].pop();
            "unknown word (truncated)"
        }
        3 => {
            let i = at(rng);
            if let Some(idx) = word_list_index(&words[i]) {
                words[i] = WORD_LIST[idx ^ 1].to_owned();
            }
            "wrong parity"
        }
        4 => {
            let i = at(rng);
            words.remove(i);
            "missing word"
        }
        5 => {
            let i = at(rng).min(n.saturating_sub(2));
            words.swap(i, i.saturating_add(1));
            "swapped words"
        }
        6 => {
            for w in words.iter_mut() {
                *w = mix_case(rng, w);
            }
            "mixed case"
        }
        7 => {
            for w in words.iter_mut() {
                *w = w.to_uppercase();
            }
            "upper case"
        }
        8 => {
            // Non-ASCII letters: U+0130 for every i or Kelvin sign for
            // every k (both still match), or one accented a (no match).
            match rng.below(3) {
                0 => {
                    for w in words.iter_mut() {
                        *w = w.replace(['i', 'I'], "\u{130}");
                    }
                }
                1 => {
                    for w in words.iter_mut() {
                        *w = w.replace(['k', 'K'], "\u{212A}");
                    }
                }
                _ => {
                    let i = at(rng);
                    words[i] = words[i].replacen(['a', 'A'], "\u{E1}", 1);
                }
            }
            "non-ASCII letters"
        }
        9 => {
            let i = at(rng);
            let idx = (rng.below(512)) as usize;
            words.insert(i, WORD_LIST[idx].to_owned());
            "extra word"
        }
        10 => {
            let i = at(rng);
            words[i] = format!("{}{}", pick(rng, NOT_SPACES), words[i]);
            "non-space prefix"
        }
        _ => {
            let i = at(rng);
            words[i] = format!(
                "{}{}",
                words[i],
                pick(rng, &["1", "-", "'", "\u{301}", "😀"])
            );
            "junk suffix"
        }
    }
}

/// Joins words with random separators: mostly single spaces, sometimes runs
/// of spaces or white space between spaces (both tolerated), and in a
/// quarter of the calls one gap of bare white space (which Go does not split
/// on, so the two words become one unknown word unless it is a space).
fn join_randomly(rng: &mut SplitMix64, words: &[String]) -> String {
    let gaps = words.len().saturating_sub(1).max(1) as u64;
    let bare_gap = (rng.below(4) == 0).then(|| rng.below(gaps).saturating_add(1) as usize);
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if bare_gap == Some(i) {
            out.push_str(pick(rng, SPACES));
        } else if i != 0 {
            match rng.below(8) {
                0 => out.push_str("   "),
                1 => out.push_str("  "),
                2 => {
                    out.push(' ');
                    out.push_str(pick(rng, SPACES));
                    out.push(' ');
                }
                _ => out.push(' '),
            }
        }
        out.push_str(w);
    }
    out
}

/// Random hex of `len` characters, mixed case.
fn rand_hex(rng: &mut SplitMix64, len: usize) -> String {
    let digits = b"0123456789abcdefABCDEF";
    (0..len)
        .map(|_| char::from(digits[rng.below(digits.len() as u64) as usize]))
        .collect()
}

/// The fixed part of the `decode-user-input` corpus.
fn fixed_user_inputs() -> Vec<String> {
    let mut v: Vec<String> = [
        "",
        " ",
        "  ",
        "\t",
        "\n",
        "\r\n",
        "\u{A0}",
        "\u{3000}",
        "\u{85}",
        "\u{200B}",
        "0",
        "00",
        "0x",
        "g",
        "zz",
        "é",
        "😀",
        "\u{0}",
        "\u{7F}",
        "'",
        "\u{AD}",
        "aardvark",
        "AARDVARK",
        "aardvark adroitness",
        "aardvark  adroitness",
        "AARDVARK ADROITNESS",
        "aardvark aardvark",
        "adroitness aardvark",
        "notaword aardvark",
        "aardvark notaword",
        "aardvark \t adroitness",
        // Failing words with padding: the message quotes the trimmed word.
        "aardvark \tnotaword",
        "aardvark \u{A0}aardvark",
        "aardvark\tadroitness",
        "aardvark\nadroitness",
        "\u{212A}iwi aardvark",
        "İstanbul",
        "topmost İstanbul Pluto vagabond treadmill Pacific brackish dictator goldfish Medusa \
         afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell \
         bottomless fracture",
        "topmost i\u{307}stanbul Pluto vagabond treadmill Pacific brackish dictator goldfish \
         Medusa afflict bravado chatter revolver Dupont midsummer stopwatch whimsical cowbell \
         bottomless fracture",
        "ab cd",
        "a b",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    // Every Latin-1-range byte that can lead an invalid hex digit, as the
    // first character of an otherwise valid 32-byte hex seed.
    let tail = "00".repeat(31) + "0";
    for c in (0x01u32..=0xFF).filter_map(char::from_u32) {
        if c.is_ascii_hexdigit() || c == ' ' {
            continue;
        }
        v.push(format!("{c}{tail}"));
    }
    v
}

/// The `decode-user-input` corpus: fixed cases, every boundary length as
/// hex and mnemonic, and random mutations.
pub fn user_input_corpus(rng: &mut SplitMix64, random_cases: usize) -> Vec<String> {
    let mut v = fixed_user_inputs();
    for len in [0usize, 1, 2, 8, 15, 16, 17, 31, 32, 33, 63, 64, 65, 66, 100] {
        let seed = rng.bytes(len);
        let words = mnemonic_words(&seed);
        v.push(words.join(" "));
        v.push(hex(&seed));
        v.push(hex(&seed).to_uppercase());
        v.push(format!("0x{}", hex(&seed)));
        v.push(format!("{}0", hex(&seed)));
        v.push(format!("{}g", hex(&seed)));
        v.push(format!("\t {} \n", hex(&seed)));
    }
    for _ in 0..random_cases {
        let len = rand_len(rng, 14, 66);
        let seed = rng.bytes(len);
        match rng.below(5) {
            0 => {
                // Hex: random length, maybe a bad byte, prefix or padding.
                let hex_len = rand_len(rng, 0, 140);
                let mut h = rand_hex(rng, hex_len);
                if rng.below(2) == 0 && !h.is_empty() {
                    let at = rng.below(h.len() as u64) as usize;
                    let bad = pick(
                        rng,
                        &["g", "G", "x", "-", "é", "\t", "\u{0}", "'", "\u{A0}"],
                    );
                    h.insert_str(at, bad);
                }
                if rng.below(4) == 0 {
                    h = format!("0x{h}");
                }
                if rng.below(4) == 0 {
                    h = format!("{}{h}{}", pick(rng, SPACES), pick(rng, SPACES));
                }
                v.push(h);
            }
            1 => {
                // A valid mnemonic with odd spacing and padding.
                let words = mnemonic_words(&seed);
                let joined = join_randomly(rng, &words);
                v.push(format!(
                    "{}{joined}{}",
                    pick(rng, SPACES),
                    pick(rng, SPACES)
                ));
            }
            _ => {
                let mut words = mnemonic_words(&seed);
                mutate_words(rng, &mut words);
                if rng.below(3) == 0 {
                    // Two mutations at once.
                    mutate_words(rng, &mut words);
                }
                if words.len() < 2 {
                    words.push("aardvark".to_owned());
                }
                v.push(join_randomly(rng, &words));
            }
        }
    }
    v
}

/// The `decode-mnemonics` corpus: word lists, including empty and
/// white-space-only words.
pub fn mnemonics_corpus(rng: &mut SplitMix64, random_cases: usize) -> Vec<Vec<String>> {
    let fixed: &[&[&str]] = &[
        &[],
        &[""],
        &["", " "],
        &["\t"],
        &["\u{3000}", "\u{85}"],
        &["aardvark"],
        &["adroitness"],
        &["Aardvark", "", "ADROITNESS"],
        &["aardvark adroitness"],
        &[" aardvark\t", "\nadroitness "],
        &["\u{85}aardvark\u{A0}", "adroitness"],
        &["aardvark", "\u{200B}"],
        &["aardvark", "adroitness", "aardvark", "aardvark"],
        &["İstanbul"],
        &["topmost", "İstanbul"],
        &["\u{212A}iwi"],
        &["kiwi", "insurgent", "\u{212A}laxon"],
        &["a\u{0301}ardvark"],
        // Failing words with padding: the message quotes the trimmed word.
        &[" adroitness"],
        &["aardvark", "\u{A0}adroitnes "],
        &["\tnotaword\n"],
    ];
    let mut v: Vec<Vec<String>> = fixed
        .iter()
        .map(|ws| ws.iter().map(|w| (*w).to_owned()).collect())
        .collect();
    for _ in 0..random_cases {
        let len = rand_len(rng, 0, 40);
        let seed = rng.bytes(len);
        let mut words = mnemonic_words(&seed);
        match rng.below(4) {
            0 => {}
            1 => words = pad_words(rng, words),
            2 => {
                // Mutate first, then pad the edges of every word, so failing
                // words are quoted after trimming.
                if words.len() >= 2 {
                    mutate_words(rng, &mut words);
                }
                words = pad_words(rng, words);
            }
            _ => {
                if words.len() >= 2 {
                    mutate_words(rng, &mut words);
                }
            }
        }
        v.push(words);
    }
    v
}

/// Pads both edges of every word with white space and inserts empty or
/// white-space-only words.
fn pad_words(rng: &mut SplitMix64, words: Vec<String>) -> Vec<String> {
    let mut padded = Vec::new();
    for w in words {
        if rng.below(4) == 0 {
            padded.push(pick(rng, &["", " ", "\t", "\u{3000}"]).to_owned());
        }
        padded.push(format!("{}{w}{}", pick(rng, SPACES), pick(rng, SPACES)));
    }
    padded
}

/// Sizes for `generate-random-seed`.
pub fn generate_sizes() -> Vec<u64> {
    let mut v: Vec<u64> = (0..=300).collect();
    v.extend([1 << 16, u64::from(u32::MAX), u64::MAX - 1, u64::MAX]);
    v
}
