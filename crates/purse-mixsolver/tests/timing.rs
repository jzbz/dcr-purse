// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Time: ADR-0010's speed gate, a benchmark table, and a bounded-time
//! check that runs in every `cargo test`.
//!
//! - `speed_gate` (ignored; release only) is the gate of ADR-0010,
//!   Decision 5: 21 fixed seeded instances at `mtot` = 512, timed one at a
//!   time on one thread, fail if the median exceeds 1 s. The gate is
//!   defined on the slowest CI runner (`ubuntu-24.04-arm`). Build first,
//!   unpinned, then pin only the measurement to one core (under `taskset`
//!   cargo would build with one job, every rustc confined to that core):
//!
//!   ```text
//!   cargo test --release -p purse-mixsolver --test timing --no-run
//!   taskset -c 0 cargo test --release -p purse-mixsolver --test timing -- --ignored --exact speed_gate --nocapture
//!   ```
//!
//! - `bench_table` (ignored) prints the median, p99 and worst over random
//!   inputs at degrees 2 to 1024 (`PURSE_MIXSOLVER_BENCH_SCALE` multiplies
//!   the repetitions, `PURSE_MIXSOLVER_BENCH_DEGREES=512,1024` picks the
//!   degrees):
//!
//!   ```text
//!   cargo test --release -p purse-mixsolver --test timing -- --ignored --exact bench_table --nocapture
//!   ```
//!
//! - `bounded_time` runs always and asserts only a bound far above any
//!   build's time, so it cannot flake: it catches a hang or an accidental
//!   change of complexity class, not a slowdown. The gate is `speed_gate`'s
//!   job. It solves three instances at degree 512 in an optimised build (or
//!   under `PURSE_MIXSOLVER_HEAVY`) and at degree 128 in a debug build,
//!   where 512 would cost seconds per solve.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test code: statistics over small vectors"
)]

mod common;

use std::time::{Duration, Instant};

use common::{Lead, distinct_roots, env_count, heavy, polynomial, sorted};
use purse_mixsolver::roots;
use purse_testutil::SplitMix64;

/// The seed of the gate's 21 instances (fixed: the gate times the same
/// polynomials on every run).
const GATE_SEED: u64 = 0x6761_7465_0000_0512;

/// Solves and checks one instance, returning the time the solve took.
fn timed(rng: &mut SplitMix64, degree: usize) -> Duration {
    let r = distinct_roots(rng, degree, &[]);
    let coeffs = polynomial(rng, &r, Lead::Client);
    let start = Instant::now();
    let got = roots(&coeffs);
    let took = start.elapsed();
    assert_eq!(got, Ok(sorted(&r)), "degree {degree}");
    took
}

fn quantile(sorted: &[Duration], q: f64) -> Duration {
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore = "release-profile gate: cargo test --release -- --ignored --exact speed_gate"]
fn speed_gate() {
    if cfg!(debug_assertions) {
        panic!("the speed gate is defined on a release build: run it with --release");
    }
    let mut rng = SplitMix64(GATE_SEED);
    let mut times: Vec<Duration> = (0..21).map(|_| timed(&mut rng, 512)).collect();
    times.sort_unstable();
    let median = times[10];
    eprintln!(
        "speed gate, mtot = 512, 21 instances: median {:.1} ms, min {:.1} ms, max {:.1} ms",
        ms(median),
        ms(times[0]),
        ms(times[20])
    );
    assert!(
        median <= Duration::from_secs(1),
        "ADR-0010 speed gate failed: median {median:?} > 1 s at mtot = 512"
    );
}

#[test]
#[ignore = "benchmark: cargo test --release -- --ignored --exact bench_table --nocapture"]
fn bench_table() {
    let scale = env_count("PURSE_MIXSOLVER_BENCH_SCALE", 1);
    let mut rng = SplitMix64::from_env("bench_table");
    let plan: [(usize, usize); 7] = [
        (2, 5000),
        (8, 2000),
        (64, 400),
        (128, 200),
        (256, 101),
        (512, 101),
        (1024, 31),
    ];
    eprintln!(
        "{:>6} {:>6} {:>12} {:>12} {:>12} {:>12}",
        "degree", "reps", "median ms", "p99 ms", "worst ms", "best ms"
    );
    let only: Option<Vec<usize>> = std::env::var("PURSE_MIXSOLVER_BENCH_DEGREES")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|d| d.trim().parse().expect("degree"))
                .collect()
        });
    for (degree, reps) in plan {
        if only.as_ref().is_some_and(|o| !o.contains(&degree)) {
            continue;
        }
        let reps = reps * scale;
        let mut times: Vec<Duration> = (0..reps).map(|_| timed(&mut rng, degree)).collect();
        times.sort_unstable();
        eprintln!(
            "{degree:>6} {reps:>6} {:>12.3} {:>12.3} {:>12.3} {:>12.3}",
            ms(quantile(&times, 0.5)),
            ms(quantile(&times, 0.99)),
            ms(times[times.len() - 1]),
            ms(times[0]),
        );
    }
}

#[test]
fn bounded_time() {
    let degree = if heavy() { 512 } else { 128 };
    let mut rng = SplitMix64::from_env("degree_512_finishes");
    let start = Instant::now();
    for _ in 0..3 {
        let took = timed(&mut rng, degree);
        eprintln!("degree {degree}: {took:?}");
    }
    let total = start.elapsed();
    assert!(
        total < Duration::from_secs(BOUND_SECS),
        "three degree-{degree} solves took {total:?}"
    );
}

/// The bound for three solves, in either profile. An optimised build on an
/// x86-64 desktop takes about 0.2 s per degree-512 solve, an unoptimised one
/// about 0.1 s per degree-128 solve (and 3.5 s at 512); 120 s is two orders
/// of magnitude above either, leaving room for a slow shared runner.
const BOUND_SECS: u64 = 120;
