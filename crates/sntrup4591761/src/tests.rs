// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Tests that need the crate's internals: the upstream Sage test, ported
//! (`sntrup4591761_test.go`), and the arithmetic facts the decoders' and
//! multiplications' documentation relies on.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: arithmetic and indexing on small test values"
)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use purse_testutil::{SplitMix64, unhex};

use crate::arith::P;
use crate::kem::{create_cipher, derive_key};
use crate::{
    CIPHERTEXT_SIZE, Ciphertext, PRIVATE_KEY_SIZE, PUBLIC_KEY_SIZE, PrivateKey, PublicKey,
    decapsulate, r3, rq, zx,
};

/// The upstream data file, copied verbatim (see `tests/data/README.md`).
const SAGE_PATH: &str = "tests/data/sage128.gz";
/// SHA-256 of the copy, to catch an edited or truncated file.
const SAGE_SHA256: &str = "61b61c86851f585e0b71fb90304619d0ee45a127d3e3f97be172b405fcdc180a";

fn small(bytes: &[u8]) -> [i8; P] {
    let mut f = [0i8; P];
    zx::decode(bytes.try_into().expect("191 bytes"), &mut f);
    f
}

/// `TestSage`: 128 instances `f:g:r:pk:sk:c:k` from the Sage model of NTRU
/// Prime; key derivation, encapsulation and decapsulation must reproduce
/// each. Go's test checks exactly these three things per line
/// (`testKeyDerivation`, `testEncapsulation`, `testDecapsulation`).
#[test]
fn sage() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SAGE_PATH);
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {SAGE_PATH}: {e}"));
    assert_eq!(
        purse_testutil::hex(&sha256(&raw)),
        SAGE_SHA256,
        "{SAGE_PATH} is not the upstream file"
    );
    let reader = BufReader::new(flate2::read::GzDecoder::new(&raw[..]));
    let mut lines = 0;
    for line in reader.lines() {
        let line = line.expect("gunzip");
        let parts: Vec<Vec<u8>> = line.split(':').map(unhex).collect();
        assert_eq!(parts.len(), 7, "line {}: bad number of parts", lines + 1);
        let f = small(&parts[0]);
        let g = small(&parts[1]);
        let r = small(&parts[2]);

        // testKeyDerivation: Go ignores Reciprocal's result here.
        let mut gr = [0i8; P];
        let _ = r3::reciprocal(&mut gr, &g);
        let (pk, sk) = derive_key(&f, &g, &gr);
        assert_eq!(
            &pk.as_bytes()[..],
            &parts[3][..],
            "line {}: public key",
            lines + 1
        );
        assert_eq!(
            &sk.as_bytes()[..],
            &parts[4][..],
            "line {}: private key",
            lines + 1
        );

        // testEncapsulation.
        let pk = PublicKey::try_from(&parts[3][..]).expect("public key size");
        let (ct, k) = create_cipher(&r, &pk);
        assert_eq!(
            &ct.as_bytes()[..],
            &parts[5][..],
            "line {}: ciphertext",
            lines + 1
        );
        assert_eq!(
            &k.as_bytes()[..],
            &parts[6][..],
            "line {}: shared key",
            lines + 1
        );

        // testDecapsulation.
        let sk = PrivateKey::try_from(&parts[4][..]).expect("private key size");
        let ct = Ciphertext::try_from(&parts[5][..]).expect("ciphertext size");
        let (k, ok) = decapsulate(&ct, &sk);
        assert!(bool::from(ok), "line {}: decapsulation error", lines + 1);
        assert_eq!(
            &k.as_bytes()[..],
            &parts[6][..],
            "line {}: shared key",
            lines + 1
        );
        lines += 1;
    }
    assert_eq!(lines, 128);
    assert_eq!(PUBLIC_KEY_SIZE + 382, PRIVATE_KEY_SIZE);
    assert_eq!(CIPHERTEXT_SIZE, 1047);
}

/// A small SHA-256 for the data file check, through the `sha2` the crate
/// already depends on.
fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

fn random_small(rng: &mut SplitMix64) -> [i8; P] {
    let mut f = [0i8; P];
    for fi in &mut f {
        *fi = rng.below(3) as i8 - 1;
    }
    f
}

/// The canonical residue of `a` mod q.
fn modq(a: i64) -> i16 {
    let r = a.rem_euclid(4591);
    (if r > 2295 { r - 4591 } else { r }) as i16
}

/// Schoolbook product in Z[x]/(x^761 − x − 1) mod q, on i64, as a model.
fn model_mult_q(f: &[i16; P], g: &[i8; P]) -> [i16; P] {
    let mut fg = vec![0i64; 2 * P - 1];
    for i in 0..P {
        for j in 0..P {
            fg[i + j] += i64::from(f[i]) * i64::from(g[j]);
        }
    }
    for i in (P..2 * P - 1).rev() {
        fg[i - P] += fg[i];
        fg[i - P + 1] += fg[i];
    }
    let mut h = [0i16; P];
    for i in 0..P {
        h[i] = modq(fg[i]);
    }
    h
}

/// `rq::mult` is the true product of `f` mod q and `g`, canonical, for any
/// `i16` coefficients of `f` (as the decoders can return on malformed
/// input) and small `g` (including the coefficient 2 that `zx::decode`
/// gives for the two-bit value 3).
#[test]
fn rq_mult_is_exact_for_any_i16_input() {
    let mut rng = SplitMix64::from_env("rq_mult_is_exact_for_any_i16_input");
    for round in 0..6 {
        let mut f = [0i16; P];
        for fi in &mut f {
            *fi = match round % 3 {
                0 => modq(rng.next_u64() as i64),
                1 => rng.next_u64() as i16,
                _ => [i16::MIN, i16::MAX, -1, 0][rng.below(4) as usize],
            };
        }
        let mut g = [0i8; P];
        for gi in &mut g {
            *gi = rng.below(4) as i8 - 1;
        }
        let mut h = [0i16; P];
        rq::mult(&mut h, &f, &g);
        assert_eq!(h, model_mult_q(&f, &g), "round {round}");
    }
}

/// `r3::mult` is the true product mod 3.
#[test]
fn r3_mult_is_exact() {
    let mut rng = SplitMix64::from_env("r3_mult_is_exact");
    for _ in 0..4 {
        let f = random_small(&mut rng);
        let mut g = [0i8; P];
        for gi in &mut g {
            *gi = rng.below(4) as i8 - 1;
        }
        let mut h = [0i8; P];
        r3::mult(&mut h, &f, &g);
        // The exact integer product, reduced mod 3.
        let mut fg = vec![0i64; 2 * P - 1];
        for i in 0..P {
            for j in 0..P {
                fg[i + j] += i64::from(f[i]) * i64::from(g[j]);
            }
        }
        for i in (P..2 * P - 1).rev() {
            fg[i - P] += fg[i];
            fg[i - P + 1] += fg[i];
        }
        for i in 0..P {
            let r = fg[i].rem_euclid(3);
            let want = if r == 2 { -1 } else { r as i8 };
            assert_eq!(h[i], want, "coefficient {i}");
        }
    }
}

/// The reciprocals invert: s·(1/s) = 1 in R3, and (3f)·(1/(3f)) = 1 in Rq.
#[test]
fn reciprocals_invert() {
    let mut rng = SplitMix64::from_env("reciprocals_invert");
    let mut one3 = [0i8; P];
    one3[0] = 1;
    let mut found = 0;
    for _ in 0..8 {
        let s = random_small(&mut rng);
        let mut sr = [0i8; P];
        if r3::reciprocal(&mut sr, &s) == 0 {
            let mut prod = [0i8; P];
            r3::mult(&mut prod, &sr, &s);
            assert_eq!(prod, one3);
            found += 1;
        }
        let mut f3r = [0i16; P];
        assert_eq!(rq::reciprocal3(&mut f3r, &s), 0);
        let mut prod = [0i16; P];
        rq::mult(&mut prod, &f3r, &s);
        // prod = (1/(3s))·s = 1/3 mod q.
        for (i, &c) in prod.iter().enumerate() {
            let want = if i == 0 {
                modq(i64::from(crate::arith::recipq(3)))
            } else {
                0
            };
            assert_eq!(c, want, "coefficient {i}");
        }
    }
    assert!(found > 0, "no invertible element in 8 draws");
    // 0 is not invertible.
    let mut sr = [0i8; P];
    assert_eq!(r3::reciprocal(&mut sr, &[0i8; P]), -1);
}

/// The encodings round-trip on canonical input.
#[test]
fn rq_encodings_round_trip() {
    let mut rng = SplitMix64::from_env("rq_encodings_round_trip");
    for _ in 0..50 {
        let mut f = [0i16; P];
        for fi in &mut f {
            *fi = modq(rng.next_u64() as i64);
        }
        let mut c = [0u8; rq::ENCODED_BYTES];
        rq::encode(&f, &mut c);
        let mut back = [0i16; P];
        rq::decode(&c, &mut back);
        assert_eq!(back, f);

        rq::round3(&mut f);
        assert!(
            f.iter()
                .all(|&x| (x + 2295) % 3 == 0 && (-2295..=2295).contains(&x))
        );
        let mut c = [0u8; rq::ROUNDED_BYTES];
        rq::encode_rounded(&f, &mut c);
        rq::decode_rounded(&c, &mut back);
        assert_eq!(back, f);
    }
}

/// The ciphertext tail (the last three bytes, coefficients 759 and 760)
/// is not injective: of the 2^24 values, 3,308,491 decode to a pair of
/// valid rounded coefficients, where the encoder produces only 1531² =
/// 2,343,961 of them; the other 964,530 decode to the same pair as some
/// canonical tail (QK-0740). Exhaustive, over the crate's own decoder.
#[test]
fn rounded_tail_aliases_exhaustive() {
    let valid = |x: i16| (-2295..=2295).contains(&x) && (x + 2295) % 3 == 0;
    let (mut n_valid, mut n_alias) = (0u32, 0u32);
    for v in 0u32..(1 << 24) {
        let b = v.to_le_bytes();
        let [a, c] = rq::decode_rounded_tail([b[0], b[1], b[2]]);
        if valid(a) && valid(c) {
            n_valid += 1;
            let mut f = [0i16; P];
            f[759] = a;
            f[760] = c;
            let mut e = [0u8; rq::ROUNDED_BYTES];
            rq::encode_rounded(&f, &mut e);
            if e[1012..] != b[..3] {
                n_alias += 1;
            }
        }
    }
    assert_eq!((n_valid, n_alias), (3_308_491, 964_530));
}

/// The four-byte groups are injective on valid output: of the 2^32 values,
/// exactly the 1531³ = 3,588,604,291 the encoder produces decode to three
/// valid rounded coefficients, each to its own. Exhaustive; about 40 s in a
/// release build on an x86_64 Linux desktop:
///
/// ```text
/// cargo test --release -p sntrup4591761 --lib -- --ignored --exact tests::rounded_groups_have_no_aliases
/// ```
#[test]
#[ignore = "2^32 decodes: run in a release build"]
fn rounded_groups_have_no_aliases() {
    let valid = |x: i16| (-2295..=2295).contains(&x) && (x + 2295) % 3 == 0;
    let digit = |x: i16| ((x + 2295) / 3) as u64;
    let (mut n_valid, mut n_alias) = (0u64, 0u64);
    for v in 0u32..=u32::MAX {
        let [a, b, c] = rq::decode_rounded_group(v.to_le_bytes());
        if valid(a) && valid(b) && valid(c) {
            n_valid += 1;
            // EncodeRounded: f0 + 1536·f1 + 1536²·f2, digits below 1531.
            if digit(a) + 1536 * digit(b) + 1536 * 1536 * digit(c) != u64::from(v) {
                n_alias += 1;
            }
        }
    }
    assert_eq!((n_valid, n_alias), (3_588_604_291, 0));
}

// ---- Component differentials against Go's exported zx, rq and r3 ----

use purse_testutil::serde_json::{Value, json};

fn ints(v: &Value) -> Vec<i64> {
    v.as_array()
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .iter()
        .map(|x| x.as_i64().expect("integer"))
        .collect()
}

fn widen<T: Copy + Into<i64>>(v: &[T]) -> Vec<i64> {
    v.iter().map(|&x| x.into()).collect()
}

/// Random bytes biased to the decoders' edges: random, 0x00, 0xff, or
/// random with the top bits of every byte set.
fn edge_bytes(rng: &mut SplitMix64, n: usize, kind: u64) -> Vec<u8> {
    match kind % 4 {
        0 => rng.bytes(n),
        1 => vec![0; n],
        2 => vec![0xff; n],
        _ => rng.bytes(n).into_iter().map(|b| b | 0xc0).collect(),
    }
}

/// Every internal function the KEM is built from equals Go's exported
/// counterpart, on the domains the KEM feeds it and beyond: the random
/// polynomials (with the bytes consumed and the error on a short stream),
/// the three decoders on arbitrary bytes, both multiplications on
/// arbitrary coefficients of their Go types, and both reciprocals.
#[test]
fn components_agree_with_go() {
    let Some(mut oracle) = purse_testutil::oracle_or_skip("sntrup") else {
        return;
    };
    let mut rng = SplitMix64::from_env("components_agree_with_go");
    let n = if cfg!(debug_assertions) { 6 } else { 40 };

    // zx.RandomSmall, zx.RandomTSmall.
    for i in 0..4 * n {
        let len = if i % 5 == 4 {
            rng.below(3044) as usize
        } else {
            3044 + (i % 3)
        };
        let stream = rng.bytes(len);
        for (cmd, tsmall) in [("zx.random-small", false), ("zx.random-tsmall", true)] {
            let go = oracle.try_call(cmd, json!({ "rand": purse_testutil::hex(&stream) }));
            let mut pos = 0usize;
            let mut src = |buf: &mut [u8]| -> Result<(), ()> {
                if stream.len() - pos >= buf.len() {
                    buf.copy_from_slice(&stream[pos..pos + buf.len()]);
                    pos += buf.len();
                    Ok(())
                } else {
                    pos = stream.len();
                    Err(())
                }
            };
            let mut f = [0i8; P];
            let r = if tsmall {
                zx::random_tsmall(&mut f, &mut src)
            } else {
                zx::random_small(&mut f, &mut src)
            };
            match (r, go) {
                (Ok(()), Ok(v)) => {
                    assert_eq!(widen(&f), ints(&v["f"]), "{cmd}");
                    assert_eq!(v["consumed"], 3044, "{cmd}");
                }
                (Err(()), Err(e)) => {
                    assert_eq!(e.kind.as_deref(), Some("rand"), "{cmd}");
                    assert!(e.text.ends_with(&format!("\t{len}")), "{cmd}: {}", e.text);
                }
                (r, go) => panic!("{cmd} on {len} bytes: Rust {r:?}, Go {go:?}"),
            }
        }
    }

    // The decoders.
    for i in 0..4 * n as u64 {
        let b = edge_bytes(&mut rng, 191, i);
        let go = oracle.call("zx.decode", json!({ "bytes": purse_testutil::hex(&b) }));
        let mut f = [0i8; P];
        zx::decode(b.as_slice().try_into().expect("191"), &mut f);
        assert_eq!(widen(&f), ints(&go), "zx.decode");

        let b = edge_bytes(&mut rng, rq::ENCODED_BYTES, i);
        let go = oracle.call("rq.decode", json!({ "bytes": purse_testutil::hex(&b) }));
        let mut f = [0i16; P];
        rq::decode(b.as_slice().try_into().expect("1218"), &mut f);
        assert_eq!(widen(&f), ints(&go), "rq.decode");

        let b = edge_bytes(&mut rng, rq::ROUNDED_BYTES, i);
        let go = oracle.call(
            "rq.decode-rounded",
            json!({ "bytes": purse_testutil::hex(&b) }),
        );
        rq::decode_rounded(b.as_slice().try_into().expect("1015"), &mut f);
        assert_eq!(widen(&f), ints(&go), "rq.decode-rounded");
    }

    // The multiplications: f of any i16 (any i8 for r3), g small or any i8.
    for i in 0..n {
        let mut f = [0i16; P];
        for x in &mut f {
            *x = if i % 2 == 0 {
                modq(rng.next_u64() as i64)
            } else {
                rng.next_u64() as i16
            };
        }
        let mut g = [0i8; P];
        for x in &mut g {
            *x = if i % 3 == 2 {
                rng.next_u64() as i8
            } else {
                rng.below(4) as i8 - 1
            };
        }
        let go = oracle.call("rq.mult", json!({ "f": widen(&f), "g": widen(&g) }));
        let mut h = [0i16; P];
        rq::mult(&mut h, &f, &g);
        assert_eq!(widen(&h), ints(&go), "rq.mult, round {i}");
        reference::rq_mult(&mut h, &f, &g);
        assert_eq!(widen(&h), ints(&go), "rq.mult model, round {i}");

        let mut f3 = [0i8; P];
        for x in &mut f3 {
            *x = if i % 3 == 2 {
                rng.next_u64() as i8
            } else {
                rng.below(3) as i8 - 1
            };
        }
        let go = oracle.call("r3.mult", json!({ "f": widen(&f3), "g": widen(&g) }));
        let mut h3 = [0i8; P];
        r3::mult(&mut h3, &f3, &g);
        assert_eq!(widen(&h3), ints(&go), "r3.mult, round {i}");
        reference::r3_mult(&mut h3, &f3, &g);
        assert_eq!(widen(&h3), ints(&go), "r3.mult model, round {i}");
    }

    // The reciprocals: on small elements (the KEM's domain), on arbitrary
    // i8 coefficients, and on elements with no inverse in R3 (zero, the
    // factors of the modulus and their multiples). The test model must
    // give Go's output on every input, including what Go's loop leaves in
    // r when there is no inverse; the optimised functions must give Go's
    // return value on every input and Go's r whenever there is an inverse
    // (the KEM never reads r otherwise).
    let mut inputs = Vec::new();
    for i in 0..n {
        inputs.push(if i % 4 == 3 {
            wide_i8(&mut rng, 2)
        } else {
            random_small(&mut rng)
        });
    }
    inputs.extend(r3_non_invertible(&mut rng, 6));
    for (i, s) in inputs.iter().enumerate() {
        let go = oracle.call("r3.reciprocal", json!({ "s": widen(s) }));
        let go_ret = go["ret"].as_i64().expect("ret");
        let mut r = [0i8; P];
        let ret = reference::r3_reciprocal(&mut r, s);
        assert_eq!(widen(&r), ints(&go["r"]), "r3.reciprocal model, input {i}");
        assert_eq!(i64::from(ret), go_ret, "r3.reciprocal model, input {i}");
        let ret = r3::reciprocal(&mut r, s);
        assert_eq!(i64::from(ret), go_ret, "r3.reciprocal, input {i}");
        if go_ret == 0 {
            assert_eq!(widen(&r), ints(&go["r"]), "r3.reciprocal, input {i}");
        }

        let go = oracle.call("rq.reciprocal3", json!({ "s": widen(s) }));
        let go_ret = go["ret"].as_i64().expect("ret");
        let mut r = [0i16; P];
        let ret = reference::rq_reciprocal3(&mut r, s);
        assert_eq!(widen(&r), ints(&go["r"]), "rq.reciprocal3 model, input {i}");
        assert_eq!(i64::from(ret), go_ret, "rq.reciprocal3 model, input {i}");
        let ret = rq::reciprocal3(&mut r, s);
        assert_eq!(i64::from(ret), go_ret, "rq.reciprocal3, input {i}");
        if go_ret == 0 {
            assert_eq!(widen(&r), ints(&go["r"]), "rq.reciprocal3, input {i}");
        }
    }
    // Every non-invertible input was seen as such by Go.
    assert!(
        inputs[n..]
            .iter()
            .all(|s| { oracle.call("r3.reciprocal", json!({ "s": widen(s) }))["ret"] == -1 })
    );
}

// ---- The optimised arithmetic against the test model ----

use crate::reference;

/// Coefficients of `f` for the model comparisons: canonical, any `i16`, the
/// extremes only, or a constant extreme (the largest sums).
fn wide_i16(rng: &mut SplitMix64, kind: usize) -> [i16; P] {
    let mut f = [0i16; P];
    for x in &mut f {
        *x = match kind % 5 {
            0 => modq(rng.next_u64() as i64),
            1 => rng.next_u64() as i16,
            2 => [i16::MIN, i16::MAX, -2295, 2295, -1, 0, 1][rng.below(7) as usize],
            3 => i16::MIN,
            _ => 2295,
        };
    }
    f
}

/// Coefficients of `g`: small, small with Go's decoded 2, any `i8`, or a
/// constant extreme.
fn wide_i8(rng: &mut SplitMix64, kind: usize) -> [i8; P] {
    let mut g = [0i8; P];
    for x in &mut g {
        *x = match kind % 5 {
            0 => rng.below(3) as i8 - 1,
            1 => rng.below(4) as i8 - 1,
            2 => rng.next_u64() as i8,
            3 => i8::MIN,
            _ => [i8::MIN, i8::MAX, -1, 0, 1][rng.below(5) as usize],
        };
    }
    g
}

/// `rq::mult` and `r3::mult` equal Go's statement-for-statement order
/// (`reference`) on every kind of input the Go types allow, including the
/// operands whose sums are largest.
#[test]
fn mults_match_the_model() {
    let mut rng = SplitMix64::from_env("mults_match_the_model");
    let rounds = if cfg!(debug_assertions) { 25 } else { 400 };
    for round in 0..rounds {
        let f = wide_i16(&mut rng, round);
        let g = wide_i8(&mut rng, round / 5);
        let (mut got, mut want) = ([0i16; P], [0i16; P]);
        rq::mult(&mut got, &f, &g);
        reference::rq_mult(&mut want, &f, &g);
        assert_eq!(got, want, "rq::mult, round {round}");

        let f3 = wide_i8(&mut rng, round);
        let (mut got, mut want) = ([0i8; P], [0i8; P]);
        r3::mult(&mut got, &f3, &g);
        reference::r3_mult(&mut want, &f3, &g);
        assert_eq!(got, want, "r3::mult, round {round}");
    }
}

/// The irreducible factors of x^761 − x − 1 over GF(3), of degrees 19, 60
/// and 682, coefficient 0 first (`+` is 1, `-` is −1), from a distinct-degree
/// factorisation (SymPy's `gf_ddf_zassenhaus`) whose product was checked to
/// be the modulus; `r3_factors_multiply_to_the_modulus` checks it again.
/// A multiple of any of them is not invertible in R3: these are the inputs
/// on which key generation rejects g.
pub(crate) const R3_FACTORS: [&str; 3] = [
    "--0---+-+-0-++++-0++",
    "+-++00+-0-0-0+++++00-0++00+0+00--++-0++00++-00++++-00+-0--+0+",
    "++00-+++0+-0-+000+++--+0+0++0-+0-0-+00-+0+------0++++---0++0+000+-0--+0-+--0-0-+-0+00+-++000++000-000+++---00-0+--+-++0--+-00+-+-+-0++-+000-+00++0000-0-+0++-0++00+--+00+0+-0+++-0+0+--+--+-+0-+++0-++0+00-0++0000+--0--+-000-00+---+--0+0-++--000--0-+--++-0-00--00+0-0--0000+-0++0--+0+----+0+000+-0--0--+0+++0++000+0+-00-0+-+0+---++-++-+--+0000+0-0+0-00-00++-0++-0-0-+---+--00+-+0+-00+0--00-++--+0+0--00++++0-++---0++++0000++0+++--0++00---+00-0-+00+0--00-+0-+0++-++-++00-+-000--000+00-0+-++-0++++----00-++000-0-00---++-+0-0++++0+0++-++--0+-0++00++0--0---+---+++0+++0--+++++00++-----000++-++0++-+0+0-++++0+0++-00-000-++0000-0++-0-0--+0++0+-0-++--0--++++-++++0-----+---++++----00+0-+0+-0-+",
];

/// A factor as an element of R3.
fn r3_factor(s: &str) -> [i8; P] {
    let mut f = [0i8; P];
    for (fi, ch) in f.iter_mut().zip(s.chars()) {
        *fi = match ch {
            '+' => 1,
            '-' => -1,
            _ => 0,
        };
    }
    f
}

#[test]
fn r3_factors_multiply_to_the_modulus() {
    let [a, b, c] = R3_FACTORS.map(r3_factor);
    assert_eq!(
        R3_FACTORS.map(|s| s.len() - 1),
        [19, 60, 682],
        "factor degrees"
    );
    // a·b has degree 79 and c degree 682: their product in R3 is the
    // modulus reduced, zero.
    let mut ab = [0i8; P];
    r3::mult(&mut ab, &a, &b);
    let mut abc = [0i8; P];
    r3::mult(&mut abc, &ab, &c);
    assert_eq!(abc, [0i8; P]);
    // No two of them make zero alone.
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c)] {
        let mut xy = [0i8; P];
        r3::mult(&mut xy, x, y);
        assert_ne!(xy, [0i8; P]);
    }
}

/// Elements of R3 that are not invertible: zero, each factor, and
/// multiples of a factor by random small elements.
fn r3_non_invertible(rng: &mut SplitMix64, n: usize) -> Vec<[i8; P]> {
    let factors = R3_FACTORS.map(r3_factor);
    let mut out = vec![[0i8; P]];
    out.extend(factors);
    for i in 0..n {
        let t = random_small(rng);
        let mut m = [0i8; P];
        r3::mult(&mut m, &factors[i % 3], &t);
        out.push(m);
    }
    out
}

/// `r3::reciprocal` and `rq::reciprocal3` report invertibility as Go's
/// loop (`reference`) does on every input, and give its result whenever
/// there is an inverse: random small elements, t-small ones, arbitrary
/// `i8` coefficients (Go reads them mod 3, and `rq` wraps 3·s in `int8`),
/// monomials, and, for R3, zero, the factors of the modulus and their
/// multiples, which have none.
#[test]
fn reciprocals_match_the_model() {
    let mut rng = SplitMix64::from_env("reciprocals_match_the_model");
    let n = if cfg!(debug_assertions) { 6 } else { 60 };
    let mut inputs: Vec<[i8; P]> = Vec::new();
    for i in 0..n {
        inputs.push(random_small(&mut rng));
        let mut t = [0i8; P];
        zx::random_tsmall(&mut t, &mut |b: &mut [u8]| -> Result<(), ()> {
            rng.fill(b);
            Ok(())
        })
        .expect("endless");
        inputs.push(t);
        inputs.push(wide_i8(&mut rng, 2));
        let mut mono = [0i8; P];
        mono[(i * 97) % P] = if i % 2 == 0 { 1 } else { -1 };
        inputs.push(mono);
    }
    let bad = r3_non_invertible(&mut rng, n);
    inputs.extend(bad.iter().copied());
    for (i, s) in inputs.iter().enumerate() {
        let (mut got, mut want) = ([0i8; P], [0i8; P]);
        let ret = r3::reciprocal(&mut got, s);
        let want_ret = reference::r3_reciprocal(&mut want, s);
        assert_eq!(ret, want_ret, "r3::reciprocal return, input {i}");
        if want_ret == 0 {
            assert_eq!(got, want, "r3::reciprocal, input {i}");
        }

        let (mut got, mut want) = ([0i16; P], [0i16; P]);
        let ret = rq::reciprocal3(&mut got, s);
        let want_ret = reference::rq_reciprocal3(&mut want, s);
        assert_eq!(ret, want_ret, "rq::reciprocal3 return, input {i}");
        if want_ret == 0 {
            assert_eq!(got, want, "rq::reciprocal3, input {i}");
        }
    }
    for s in &bad {
        assert_eq!(r3::reciprocal(&mut [0i8; P], s), -1);
    }
    assert_eq!(rq::reciprocal3(&mut [0i16; P], &[0i8; P]), -1);
}

/// The Montgomery product of `rq::reciprocal3` stays within its bound and
/// is a·x·2^−16 mod q, for every canonical a and every x within the loose
/// bound (48 million pairs).
#[test]
fn mont_is_exact_within_the_loose_bound() {
    let rinv = i64::from(crate::arith::recipq(1262));
    let step = if cfg!(debug_assertions) { 7 } else { 1 };
    for a in (-2295i16..=2295).step_by(step) {
        for x in -rq::LOOSE..=rq::LOOSE {
            let m = rq::mont(a, a.wrapping_mul(15631), x);
            assert!(m.abs() <= 2479, "mont({a}, {x}) = {m}");
            assert_eq!(
                modq(i64::from(m)),
                modq(i64::from(a) * i64::from(x) * rinv),
                "mont({a}, {x})"
            );
        }
    }
}

/// Times each optimised function against the test model it replaced.
///
/// ```text
/// cargo test --release -p sntrup4591761 --lib -- --ignored --exact tests::bench_components --nocapture
/// ```
#[test]
#[ignore = "benchmark: run in a release build with --nocapture"]
fn bench_components() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    fn median(mut f: impl FnMut(), reps: usize) -> Duration {
        for _ in 0..3 {
            f();
        }
        let mut t: Vec<Duration> = (0..reps)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed()
            })
            .collect();
        t.sort_unstable();
        t[reps / 2]
    }
    fn row(name: &str, model: Duration, fast: Duration) {
        eprintln!(
            "{name:<16} {:>12.1} {:>12.1} {:>9.1}x",
            model.as_secs_f64() * 1e6,
            fast.as_secs_f64() * 1e6,
            model.as_secs_f64() / fast.as_secs_f64()
        );
    }

    let mut rng = SplitMix64(0x0062_656e_6368);
    let reps = 101;
    let f = wide_i16(&mut rng, 0);
    let g = random_small(&mut rng);
    let s = random_small(&mut rng);
    eprintln!(
        "{:<16} {:>12} {:>12} {:>10}",
        "function", "model µs", "optimised µs", "speed-up"
    );

    let mut h = [0i16; P];
    let m = median(
        || reference::rq_mult(black_box(&mut h), black_box(&f), black_box(&g)),
        reps,
    );
    let o = median(
        || rq::mult(black_box(&mut h), black_box(&f), black_box(&g)),
        reps,
    );
    row("rq::mult", m, o);

    let mut h3 = [0i8; P];
    let m = median(
        || reference::r3_mult(black_box(&mut h3), black_box(&s), black_box(&g)),
        reps,
    );
    let o = median(
        || r3::mult(black_box(&mut h3), black_box(&s), black_box(&g)),
        reps,
    );
    row("r3::mult", m, o);

    let m = median(
        || {
            black_box(reference::r3_reciprocal(black_box(&mut h3), black_box(&s)));
        },
        reps,
    );
    let o = median(
        || {
            black_box(r3::reciprocal(black_box(&mut h3), black_box(&s)));
        },
        reps,
    );
    row("r3::reciprocal", m, o);

    let m = median(
        || {
            black_box(reference::rq_reciprocal3(black_box(&mut h), black_box(&s)));
        },
        reps,
    );
    let o = median(
        || {
            black_box(rq::reciprocal3(black_box(&mut h), black_box(&s)));
        },
        reps,
    );
    row("rq::reciprocal3", m, o);
}
