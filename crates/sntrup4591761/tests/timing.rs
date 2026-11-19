// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Time: the three KEM operations against Go, and a bounded-time check that
//! runs in every `cargo test`.
//!
//! - `speed_against_go` (ignored; release only) times key generation,
//!   encapsulation and decapsulation in Rust and in Go
//!   (`tools/oracle/cmd/sntrup`, command `bench`) on the same random bytes,
//!   in alternating rounds, checks both sides produced the same key pair,
//!   ciphertext and shared key, prints the medians and the ratios, and
//!   fails if any Rust median exceeds 1.15 times Go's: ADR-0014's bound on
//!   secret paths, which every operation here is (f, 1/g, r and the shared
//!   key are secret). Build first, unpinned, then pin only the measurement
//!   to one core:
//!
//!   ```text
//!   cargo test --release -p sntrup4591761 --test timing --no-run
//!   taskset -c 2 cargo test --release -p sntrup4591761 --test timing -- --ignored --exact speed_against_go --nocapture
//!   ```
//!
//!   `PURSE_SNTRUP_BENCH_REPS` sets the repetitions per round (default 41;
//!   five rounds per side).
//!
//! - `secret_independence` (ignored; release only) is a dudect-style
//!   check (Reparaz, Balasch and Verbauwhede, "Dude, is my code constant
//!   time?", 2017): each operation is timed on two classes of input,
//!   interleaved in random order, and Welch's t-test compares the classes
//!   after the slowest samples are cropped. Key generation: one fixed
//!   stream against fresh random streams. Encapsulation: one fixed r
//!   against fresh ones, to the same key. Decapsulation: one honest
//!   ciphertext against fresh honest ones (varying the r it decrypts), and
//!   honest ciphertexts against random byte strings (which fail every
//!   check), both with one private key; and one key pair with its honest
//!   ciphertext against 64 other key pairs, each with an honest ciphertext
//!   of its own (varying f and 1/g). In the decapsulation pairs each class
//!   reads its inputs in turn from a pool of the same size, the fixed
//!   class from copies of its one input, so that both touch the same
//!   memory footprint: with a single fixed ciphertext against a pool of
//!   256, the fixed class ran measurably faster (t = −4.7 and −5.2 in two
//!   runs; −10.3 and −8.7 on a more loaded machine) and the pooled fixed
//!   class did not (|t| ≤ 3.1 in three runs), which points at the cache
//!   rather than the arithmetic. |t| above 4.5 is dudect's threshold for a
//!   leak; the test fails there. It measures wall time on whatever machine
//!   runs it, so a loaded machine adds noise, not bias (the classes are
//!   interleaved):
//!
//!   ```text
//!   taskset -c 2 cargo test --release -p sntrup4591761 --test timing -- --ignored --exact secret_independence --nocapture
//!   ```
//!
//!   `PURSE_SNTRUP_DUDECT_SAMPLES` sets the samples per class (default
//!   20000; key generation, at about 1.4 ms each, takes a tenth of that).
//!
//! - `bounded_time` runs always and asserts only a bound far above any
//!   build's time, so it cannot flake: it catches a hang or an accidental
//!   change of complexity class, not a slowdown.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: statistics over small vectors"
)]

mod common;

use std::time::{Duration, Instant};

use common::{POLY_BYTES, env_count};
use purse_testutil::serde_json::{Value, json};
use purse_testutil::{SplitMix64, hex, oracle_or_skip};
use sntrup4591761::{
    Ciphertext, PrivateKey, PublicKey, SharedKey, decapsulate, encapsulate, generate_key,
};

/// The seed of the benchmark's random bytes (fixed: every run times the
/// same key pair and ciphertext).
const BENCH_SEED: u64 = 0x6265_6e63_6800_0761;

/// ADR-0014: secret paths take at most 1.15 times Go's time.
const SECRET_PATH_BOUND: f64 = 1.15;

/// The bytes one round reads: g (never rejected for this seed), f, r.
fn bench_stream() -> Vec<u8> {
    SplitMix64(BENCH_SEED).bytes(3 * POLY_BYTES)
}

/// The outputs of one round, for comparison with Go's.
struct Outputs {
    pk: PublicKey,
    ct: Ciphertext,
    key: SharedKey,
    ok: bool,
}

/// One round in Rust: the three operations, each timed.
fn rust_round(stream: &[u8]) -> ([Duration; 3], Outputs) {
    let mut pos = 0usize;
    let mut src = |buf: &mut [u8]| -> Result<(), ()> {
        let end = pos + buf.len();
        buf.copy_from_slice(stream.get(pos..end).ok_or(())?);
        pos = end;
        Ok(())
    };
    let t0 = Instant::now();
    let (pk, sk) = generate_key(&mut src).expect("the stream holds g and f");
    let t1 = Instant::now();
    let (ct, _) = encapsulate(&mut src, &pk).expect("the stream holds r");
    let t2 = Instant::now();
    let (key, ok) = decapsulate(&ct, &sk);
    let t3 = Instant::now();
    assert_eq!(pos, stream.len(), "g was rejected: pick another seed");
    (
        [t1 - t0, t2 - t1, t3 - t2],
        Outputs {
            pk,
            ct,
            key,
            ok: bool::from(ok),
        },
    )
}

fn quantile(sorted: &[Duration], q: f64) -> Duration {
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn durations(v: &Value) -> Vec<Duration> {
    v.as_array()
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .iter()
        .map(|x| Duration::from_nanos(x.as_u64().expect("nanoseconds")))
        .collect()
}

const OPS: [&str; 3] = ["keygen", "encapsulate", "decapsulate"];

#[test]
#[ignore = "benchmark against Go: cargo test --release -- --ignored --exact speed_against_go --nocapture"]
fn speed_against_go() {
    if cfg!(debug_assertions) {
        panic!("the comparison is defined on a release build: run it with --release");
    }
    let Some(mut oracle) = oracle_or_skip("sntrup") else {
        return;
    };
    let reps = env_count("PURSE_SNTRUP_BENCH_REPS", 41);
    let stream = bench_stream();
    let mut rust: [Vec<Duration>; 3] = Default::default();
    let mut go: [Vec<Duration>; 3] = Default::default();
    let mut go_version = String::new();
    for _round in 0..5 {
        let v = oracle.call("bench", json!({ "rand": hex(&stream), "reps": reps }));
        for (k, op) in OPS.iter().enumerate() {
            go[k].extend(durations(&v[op]));
        }
        go_version = v["go"].as_str().expect("go").to_owned();

        // Three untimed rounds first, as the oracle does.
        let mut last = None;
        for i in 0..reps + 3 {
            let (t, out) = rust_round(&stream);
            if i >= 3 {
                for k in 0..3 {
                    rust[k].push(t[k]);
                }
            }
            last = Some(out);
        }
        let out = last.expect("at least one round");
        assert!(out.ok, "an honest ciphertext was rejected");
        assert_eq!(
            hex(out.pk.as_bytes()),
            v["pk"],
            "public key differs from Go's"
        );
        assert_eq!(
            hex(out.ct.as_bytes()),
            v["ct"],
            "ciphertext differs from Go's"
        );
        assert_eq!(
            hex(out.key.as_bytes()),
            v["key"],
            "shared key differs from Go's"
        );
        assert_eq!(v["ok"], 1);
    }

    eprintln!(
        "sntrup4591761, {} rounds per side, {go_version}\n{:<12} {:>12} {:>12} {:>12} {:>12} {:>9}",
        rust[0].len(),
        "operation",
        "Rust med µs",
        "Rust p90 µs",
        "Go med µs",
        "Go p90 µs",
        "Rust/Go"
    );
    let mut over = Vec::new();
    for (k, op) in OPS.iter().enumerate() {
        rust[k].sort_unstable();
        go[k].sort_unstable();
        let (rm, gm) = (quantile(&rust[k], 0.5), quantile(&go[k], 0.5));
        let ratio = rm.as_secs_f64() / gm.as_secs_f64();
        eprintln!(
            "{op:<12} {:>12.1} {:>12.1} {:>12.1} {:>12.1} {:>9.3}",
            us(rm),
            us(quantile(&rust[k], 0.9)),
            us(gm),
            us(quantile(&go[k], 0.9)),
            ratio
        );
        if ratio > SECRET_PATH_BOUND {
            over.push(format!("{op}: {ratio:.3}"));
        }
    }
    assert!(
        over.is_empty(),
        "ADR-0014 bound of {SECRET_PATH_BOUND}x Go's time exceeded: {over:?}"
    );
}

/// Welch's t statistic of two samples.
fn welch_t(a: &[f64], b: &[f64]) -> f64 {
    let stats = |x: &[f64]| {
        let n = x.len() as f64;
        let mean = x.iter().sum::<f64>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0);
        (n, mean, var)
    };
    let (na, ma, va) = stats(a);
    let (nb, mb, vb) = stats(b);
    (ma - mb) / (va / na + vb / nb).sqrt()
}

/// Times `run(class)` for `n` samples of each of two classes in random
/// order, drops the samples above the 90th percentile of all of them (as
/// dudect crops), and returns Welch's t of class 0 against class 1.
fn t_of_classes(rng: &mut SplitMix64, n: usize, mut run: impl FnMut(usize) -> Duration) -> f64 {
    let mut order: Vec<usize> = (0..2 * n).map(|i| i % 2).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let samples: Vec<(usize, f64)> = order
        .iter()
        .map(|&c| (c, run(c).as_secs_f64() * 1e9))
        .collect();
    let mut all: Vec<f64> = samples.iter().map(|s| s.1).collect();
    all.sort_by(f64::total_cmp);
    let cut = all[all.len() * 9 / 10];
    let class = |c: usize| -> Vec<f64> {
        samples
            .iter()
            .filter(|s| s.0 == c && s.1 <= cut)
            .map(|s| s.1)
            .collect()
    };
    welch_t(&class(0), &class(1))
}

#[test]
#[ignore = "dudect-style timing check: cargo test --release -- --ignored --exact secret_independence --nocapture"]
fn secret_independence() {
    if cfg!(debug_assertions) {
        panic!("the timing check is defined on a release build: run it with --release");
    }
    let n = env_count("PURSE_SNTRUP_DUDECT_SAMPLES", 20_000);
    let mut rng = SplitMix64::from_env("secret_independence");
    let mut results = Vec::new();

    // Key generation: a fixed stream against fresh ones.
    let fixed = rng.bytes(3 * POLY_BYTES);
    let mut fresh = SplitMix64(rng.next_u64());
    let t = t_of_classes(&mut rng, n / 10, |c| {
        let stream = if c == 0 {
            fixed.clone()
        } else {
            fresh.bytes(3 * POLY_BYTES)
        };
        let mut pos = 0usize;
        let mut src = |buf: &mut [u8]| -> Result<(), ()> {
            buf.copy_from_slice(&stream[pos..pos + buf.len()]);
            pos += buf.len();
            Ok(())
        };
        let start = Instant::now();
        let out = generate_key(&mut src);
        let took = start.elapsed();
        std::hint::black_box(out).expect("enough bytes");
        took
    });
    results.push(("generate_key: fixed vs random stream", t));

    // Encapsulation: a fixed r against fresh ones, to one key.
    let (pk, sk) = {
        let mut s = SplitMix64(rng.next_u64());
        generate_key(&mut |b: &mut [u8]| -> Result<(), ()> {
            s.fill(b);
            Ok(())
        })
        .expect("endless")
    };
    let fixed_r = rng.bytes(POLY_BYTES);
    let t = t_of_classes(&mut rng, n, |c| {
        let stream = if c == 0 {
            fixed_r.clone()
        } else {
            fresh.bytes(POLY_BYTES)
        };
        let mut src = |buf: &mut [u8]| -> Result<(), ()> {
            buf.copy_from_slice(&stream);
            Ok(())
        };
        let start = Instant::now();
        let out = encapsulate(&mut src, &pk);
        let took = start.elapsed();
        std::hint::black_box(out).expect("enough bytes");
        took
    });
    results.push(("encapsulate: fixed vs random r", t));

    // Decapsulation: a fixed honest ciphertext against fresh honest ones,
    // and honest ciphertexts against random bytes. Each class reads its
    // inputs from a pool of the same size, the fixed class from copies of
    // one input, so both classes touch the same amount of memory: one hot
    // ciphertext against 256 cold ones was itself a difference (see the
    // module documentation).
    let honest = |s: &mut SplitMix64| {
        encapsulate(
            &mut |b: &mut [u8]| -> Result<(), ()> {
                s.fill(b);
                Ok(())
            },
            &pk,
        )
        .expect("endless")
        .0
    };
    let fixed_ct = honest(&mut fresh);
    let fixed_pool: Vec<Ciphertext> = vec![fixed_ct; 256];
    let pool: Vec<Ciphertext> = (0..256).map(|_| honest(&mut fresh)).collect();
    let mut k = 0usize;
    let t = t_of_classes(&mut rng, n, |c| {
        k += 1;
        let ct = if c == 0 {
            &fixed_pool[k % fixed_pool.len()]
        } else {
            &pool[k % pool.len()]
        };
        let start = Instant::now();
        let out = decapsulate(ct, &sk);
        let took = start.elapsed();
        std::hint::black_box(out);
        took
    });
    results.push(("decapsulate: fixed vs random honest ciphertext", t));
    let junk: Vec<Ciphertext> = (0..256)
        .map(|_| {
            let mut b = [0u8; sntrup4591761::CIPHERTEXT_SIZE];
            fresh.fill(&mut b);
            Ciphertext::from_bytes(b)
        })
        .collect();
    let t = t_of_classes(&mut rng, n, |c| {
        k += 1;
        let ct = if c == 0 {
            &pool[k % pool.len()]
        } else {
            &junk[k % junk.len()]
        };
        let start = Instant::now();
        let out = decapsulate(ct, &sk);
        let took = start.elapsed();
        assert_eq!(bool::from(out.1), c == 0);
        took
    });
    results.push(("decapsulate: honest vs invalid ciphertext", t));

    // Decapsulation under a varying private key: one fixed key pair and
    // honest ciphertext against fresh key pairs, each with an honest
    // ciphertext of its own. The pairs above keep one key, so they say
    // nothing about time that depends on f or 1/g.
    let keyed = |s: &mut SplitMix64| {
        let (pk, sk) = generate_key(&mut |b: &mut [u8]| -> Result<(), ()> {
            s.fill(b);
            Ok(())
        })
        .expect("endless");
        let ct = encapsulate(
            &mut |b: &mut [u8]| -> Result<(), ()> {
                s.fill(b);
                Ok(())
            },
            &pk,
        )
        .expect("endless")
        .0;
        (sk, ct)
    };
    let fixed_pair = keyed(&mut fresh);
    let fixed_pairs: Vec<(PrivateKey, Ciphertext)> = vec![fixed_pair; 64];
    let key_pool: Vec<(PrivateKey, Ciphertext)> = (0..64).map(|_| keyed(&mut fresh)).collect();
    let t = t_of_classes(&mut rng, n, |c| {
        k += 1;
        let (sk, ct) = if c == 0 {
            &fixed_pairs[k % fixed_pairs.len()]
        } else {
            &key_pool[k % key_pool.len()]
        };
        let start = Instant::now();
        let out = decapsulate(ct, sk);
        let took = start.elapsed();
        assert!(bool::from(out.1));
        took
    });
    results.push(("decapsulate: fixed vs random private key", t));

    let mut over = Vec::new();
    for (what, t) in &results {
        eprintln!("{what:<48} t = {t:>7.2}");
        if t.abs() > DUDECT_THRESHOLD {
            over.push(format!("{what}: t = {t:.2}"));
        }
    }
    assert!(over.is_empty(), "timing differs between classes: {over:?}");
}

/// dudect's threshold: |t| above 4.5 means the classes' timings differ.
const DUDECT_THRESHOLD: f64 = 4.5;

#[test]
fn bounded_time() {
    let stream = bench_stream();
    let start = Instant::now();
    for _ in 0..3 {
        let (t, out) = rust_round(&stream);
        assert!(out.ok);
        eprintln!(
            "keygen {:?}, encapsulate {:?}, decapsulate {:?}",
            t[0], t[1], t[2]
        );
    }
    let total = start.elapsed();
    assert!(
        total < Duration::from_secs(BOUND_SECS),
        "three rounds took {total:?}"
    );
}

/// The bound for three rounds, in either profile. A round took about 70 ms
/// in an unoptimised build and 1.6 ms in an optimised one on an x86_64
/// Linux desktop (the direct port of Go's arithmetic, about 17 ms
/// optimised); 60 s is far above any of them, leaving room for a slow
/// shared runner.
const BOUND_SECS: u64 = 60;
