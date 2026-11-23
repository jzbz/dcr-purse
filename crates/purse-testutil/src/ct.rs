// Copyright (c) 2026 The dcr-purse developers
// Copyright (c) 2026 The dcroxide developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! dudect's fixed-against-random timing test (Reparaz, Balasch and
//! Verbauwhede, "Dude, is my code constant time?", 2017) for code that
//! handles secrets.
//!
//! The method, the constants and the verdict are those of the shared
//! harness of the sister project dcroxide (`dcroxide-testutil`'s `ct`
//! module, which its "Constant time" workflow runs; ported at the revision
//! `48f5b5e` and unchanged at the pinned revision `1587ca1`), reduced to
//! what this workspace's targets need: no
//! known reports, no per-segment trace and no DIT probe. The workspace
//! does not link `dcroxide-testutil`; a port keeps it out of the lockfile.
//!
//! An operation is timed with [`Instant`] on inputs of fixed classes and of
//! a random class, interleaved, and Welch's t-test asks whether a fixed
//! class's mean time differs from the random class's:
//!
//! * **Fixed classes at extremes.** A fixed input drawn at random has about
//!   the random class's average of whatever a leak would key on, so a
//!   target names several fixed classes, each a corner case of its secret,
//!   and each is compared with the random class on its own. A placebo
//!   ([`PLACEBO`]), whose inputs are drawn as the random class's are, can
//!   lean only on the harness or the machine.
//! * **Every class an equal share.** Each sample is of the random class or
//!   of one of the K fixed classes, each with probability 1/(K + 1), so
//!   that rare slow outliers (a neighbour's burst on the processor) fall on
//!   every class alike; dcroxide's harness documents the false leaks that
//!   a half-random split gave on unchanged code.
//! * **Inputs built ahead, every class's the same way.** A chunk of
//!   samples' inputs is built before any of them is timed, each from a
//!   buffer the random class's draw fills and over which a fixed class's
//!   bytes are written, so what building allocates does not depend on the
//!   class. A chunk's inputs fit in a core's cache ([`CHUNK_BYTES`]). An
//!   operation quicker than about two microseconds runs in a batch of the
//!   sample's class, each on its own input, so that the clock's cost and
//!   resolution are a small part of a sample.
//! * **dudect's crops.** The first samples warm up and set the crops'
//!   thresholds and are thrown away: crop i keeps the samples quicker than
//!   the warm-up's quantile 1 - 0.5^(10(i + 1)/100), for i from 0 to 99.
//!   Each fixed class is compared with the random class in the uncropped
//!   test and in every crop with [`MIN_SAMPLES`] samples on each side.
//! * **The verdict.** A class fails the target when a comparison's |t| is
//!   above [`T_FAIL`], 10, and the largest difference between the class's
//!   mean time and the random class's among those comparisons is
//!   [`MIN_FAIL_NANOS`], 1 ns an operation, or more: at the millions of
//!   samples a quick operation's run takes, |t| crosses 10 on differences
//!   far below an instruction's cost, while a secret-dependent instruction
//!   path costs a nanosecond or more. Above 10 on smaller differences a
//!   class is reported as under the floor, and above [`T_REPORT`], 4.5, it
//!   is reported; both pass. A positive control must fail by the same rule
//!   ([`Report::assert_leak_found`]), or the run fails: a run that does not
//!   see a large planted leak sees none.
//!
//! Each target's budget is [`BUDGET_VAR`] seconds of wall time
//! ([`DEFAULT_SECONDS`] when it is unset). A target prints one row, marked
//! `ct_check row |`, and a line for each class, marked `ct_check class |`.
//! The harness's own tests run in every `cargo test` and time nothing: they
//! drive it with a simulated clock.
//!
//! It is statistical. It sees a difference large enough to show on the
//! machine that runs it, fails only on one of a nanosecond an operation or
//! more, and a pass is not a proof. It times wall-clock time in one
//! process, so it sees neither what a cache shared with another process
//! would reveal of the addresses read nor anything below its floor.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test-harness arithmetic over counts, lengths and nanoseconds, each far below its \
              type's limit"
)]

use std::env;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

use crate::SplitMix64;

/// |t| above which a fixed class fails its target, on a difference of
/// [`MIN_FAIL_NANOS`] an operation or more: dudect's threshold for a clear
/// leak.
pub const T_FAIL: f64 = 10.0;

/// |t| above which a fixed class is reported, though it passes: dudect's
/// threshold for a possible leak.
pub const T_REPORT: f64 = 4.5;

/// The difference between a fixed class's mean time and the random
/// class's, in nanoseconds an operation, at or above which a comparison of
/// |t| above [`T_FAIL`] fails its class: a secret-dependent instruction
/// path costs a few cycles at least, about a nanosecond at 3 to 5 GHz.
pub const MIN_FAIL_NANOS: f64 = 1.0;

/// The fewest samples each side of a comparison needs for its t to count.
pub const MIN_SAMPLES: u64 = 100;

/// The variable that sets each target's budget, in whole seconds.
pub const BUDGET_VAR: &str = "PURSE_CT_SECONDS";

/// Each target's budget when [`BUDGET_VAR`] is unset.
pub const DEFAULT_SECONDS: u64 = 10;

/// The most bytes of inputs built ahead at once, as the inputs' values hold
/// them (not the heap they point to): well within a core's second-level
/// cache, so that every class finds its inputs in cache alike.
pub const CHUNK_BYTES: u64 = 256 * 1024;

/// The label of a placebo class, which has no bytes of its own: its inputs
/// are drawn as the random class's are.
pub const PLACEBO: &str = "placebo";

/// dudect's crops, in addition to the uncropped test.
const CROPS: usize = 100;

/// The samples that warm up and set the crops, unless a tenth of the budget
/// passes first.
const WARM_UP_SAMPLES: usize = 1_000;

/// About what a sample takes: a quicker operation runs in a batch.
const MIN_SAMPLE_NANOS: u64 = 2_000;

/// The largest batch.
const MAX_BATCH: u64 = 1_000;

/// The operations timed one by one, on the random class, whose median sizes
/// the batch.
const CALIBRATION_OPS: usize = 33;

/// The most inputs built ahead at once.
const CHUNK_OPS: u64 = 4_096;

/// The timed nanoseconds a chunk aims at.
const CHUNK_NANOS: u64 = 100_000_000;

/// One class's times in one test: count, mean and sum of squared deviations
/// from the mean, kept by Welford's update.
#[derive(Clone, Copy, Debug, Default)]
struct Moments {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Moments {
    fn push(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (x - self.mean);
    }

    /// The sample variance, of two times or more.
    fn variance(&self) -> f64 {
        self.m2 / (self.n - 1) as f64
    }
}

/// Welch's t of `a` against `b`, positive when `a` is slower; infinite when
/// the means differ and neither varies, zero when nothing differs.
fn welch_t(a: &Moments, b: &Moments) -> f64 {
    let difference = a.mean - b.mean;
    let error = (a.variance() / a.n as f64 + b.variance() / b.n as f64).sqrt();
    if error > 0.0 {
        difference / error
    } else if difference == 0.0 {
        0.0
    } else {
        f64::INFINITY.copysign(difference)
    }
}

/// The share of the warm-up's times under crop `i`'s threshold, dudect's
/// 1 - 0.5^(10(i + 1)/100).
fn crop_fraction(i: usize) -> f64 {
    1.0 - 0.5f64.powf(10.0 * (i + 1) as f64 / CROPS as f64)
}

/// A fixed class's comparison with the random class in one test.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Comparison {
    /// Welch's t, positive when the fixed class is slower.
    t: f64,
    /// 0 for every sample, 1 + i for crop i.
    test: usize,
    /// The samples on each side.
    fixed: u64,
    random: u64,
    /// The fixed class's mean time a sample, less the random class's, in
    /// nanoseconds.
    difference: f64,
    /// The random class's mean time a sample, in nanoseconds.
    random_mean: f64,
}

/// Every class's moments in the uncropped test and in each crop.
struct Stats {
    /// A sample counts in crop i when it took less than `thresholds[i]`.
    thresholds: Vec<u64>,
    /// `tests[0]` is every sample, `tests[1 + i]` crop i; slot 0 is the
    /// random class and slot 1 + j fixed class j.
    tests: Vec<Vec<Moments>>,
}

impl Stats {
    fn new(warm_up: &mut [u64], fixed: usize) -> Stats {
        warm_up.sort_unstable();
        let thresholds = (0..CROPS)
            .map(|i| {
                let at = (warm_up.len() as f64 * crop_fraction(i)) as usize;
                warm_up.get(at).copied().unwrap_or(u64::MAX)
            })
            .collect();
        Stats {
            thresholds,
            tests: vec![vec![Moments::default(); 1 + fixed]; 1 + CROPS],
        }
    }

    fn push(&mut self, slot: usize, nanos: u64) {
        let x = nanos as f64;
        self.tests[0][slot].push(x);
        for (crop, &threshold) in self.tests[1..].iter_mut().zip(&self.thresholds) {
            if nanos < threshold {
                crop[slot].push(x);
            }
        }
    }

    /// Each fixed class's comparisons, one in each test with
    /// [`MIN_SAMPLES`] on each side.
    fn by_class(&self) -> Vec<Vec<Comparison>> {
        (1..self.tests[0].len())
            .map(|slot| {
                self.tests
                    .iter()
                    .enumerate()
                    .filter(|(_, test)| test[0].n >= MIN_SAMPLES && test[slot].n >= MIN_SAMPLES)
                    .map(|(index, test)| Comparison {
                        t: welch_t(&test[slot], &test[0]),
                        test: index,
                        fixed: test[slot].n,
                        random: test[0].n,
                        difference: test[slot].mean - test[0].mean,
                        random_mean: test[0].mean,
                    })
                    .collect()
            })
            .collect()
    }
}

/// A monotonic clock in nanoseconds: the wall clock for the targets, a
/// simulated one for the harness's own tests.
trait Clock {
    fn now(&self) -> u64;
}

/// The wall clock, from an [`Instant`] read when it is made.
struct WallClock(Instant);

impl Clock for WallClock {
    fn now(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// A sample's class: `None` for the random class or one of `fixed` fixed
/// classes, each with probability 1/(`fixed` + 1).
fn draw_class(rng: &mut SplitMix64, fixed: usize) -> Option<usize> {
    (rng.below(fixed as u64 + 1) as usize).checked_sub(1)
}

/// What [`measure`] found.
struct Measured {
    stats: Stats,
    batch: u64,
    samples: u64,
    nanos: u64,
}

/// Times `op` for `budget` nanoseconds of `clock` on inputs `build` makes
/// from the sample's class (`None` for the random class) and the
/// generator. The batch is sized from the median of single operations on
/// the random class; then, chunk after chunk, each sample's class is drawn
/// and its batch's inputs built, the whole chunk's before any of it is
/// timed, and each sample is timed as its batch, each input and result
/// through `black_box`, the results dropped after the clock is read.
fn measure<I, R>(
    clock: &impl Clock,
    rng: &mut SplitMix64,
    budget: u64,
    fixed: usize,
    mut build: impl FnMut(Option<usize>, &mut SplitMix64) -> I,
    mut op: impl FnMut(&mut I) -> R,
) -> Measured {
    let start = clock.now();
    let deadline = start.saturating_add(budget);
    let warm_up_end = start.saturating_add(budget / 10);

    let mut inputs: Vec<I> = (0..CALIBRATION_OPS).map(|_| build(None, rng)).collect();
    let mut results = Vec::with_capacity(1);
    let mut single = Vec::with_capacity(CALIBRATION_OPS);
    for input in &mut inputs {
        let began = clock.now();
        results.push(black_box(op(black_box(input))));
        single.push(clock.now().saturating_sub(began));
        results.clear();
    }
    drop(inputs);
    single.sort_unstable();
    let op_nanos = single[single.len() / 2].max(1);
    let batch = MIN_SAMPLE_NANOS.div_ceil(op_nanos).clamp(1, MAX_BATCH);
    let per_chunk = CHUNK_OPS.min(CHUNK_BYTES / (size_of::<I>() as u64).max(1));
    let chunk = (CHUNK_NANOS / (op_nanos * batch)).clamp(1, (per_chunk / batch).max(1));
    results.reserve(batch as usize);

    let mut warm_up = Vec::new();
    let mut stats: Option<Stats> = None;
    let mut samples = 0;
    loop {
        let classes: Vec<Option<usize>> = (0..chunk).map(|_| draw_class(rng, fixed)).collect();
        let mut inputs = Vec::with_capacity((chunk * batch) as usize);
        for &class in &classes {
            for _ in 0..batch {
                inputs.push(build(class, rng));
            }
        }
        for (class, batch_inputs) in classes.into_iter().zip(inputs.chunks_mut(batch as usize)) {
            let began = clock.now();
            for input in batch_inputs {
                results.push(black_box(op(black_box(input))));
            }
            let ended = clock.now();
            results.clear();
            let nanos = ended.saturating_sub(began);
            if let Some(stats) = stats.as_mut() {
                stats.push(class.map_or(0, |j| 1 + j), nanos);
                samples += 1;
            } else {
                warm_up.push(nanos);
                if warm_up.len() >= WARM_UP_SAMPLES || ended >= warm_up_end {
                    stats = Some(Stats::new(&mut warm_up, fixed));
                }
            }
            if ended >= deadline {
                return Measured {
                    stats: stats.unwrap_or_else(|| Stats::new(&mut warm_up, fixed)),
                    batch,
                    samples,
                    nanos: ended.saturating_sub(start),
                };
            }
        }
    }
}

/// A class's verdict, or a target's (the worst of its classes').
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    /// Every |t| at most [`T_REPORT`].
    Pass,
    /// A |t| above [`T_REPORT`], none above [`T_FAIL`].
    Reported,
    /// |t| above [`T_FAIL`] only on differences under [`MIN_FAIL_NANOS`].
    UnderFloor,
    /// |t| above [`T_FAIL`] on a difference of [`MIN_FAIL_NANOS`] or more.
    Leak,
    /// A class had no comparison with [`MIN_SAMPLES`] on each side.
    TooFew,
}

impl Verdict {
    fn word(self) -> String {
        match self {
            Verdict::Pass => "pass".to_owned(),
            Verdict::Reported => "reported".to_owned(),
            Verdict::UnderFloor => format!("reported (under the {MIN_FAIL_NANOS} ns floor)"),
            Verdict::Leak => "LEAK".to_owned(),
            Verdict::TooFew => "TOO FEW SAMPLES".to_owned(),
        }
    }
}

/// The verdict on a comparison of `t` on a difference of `per_op`
/// nanoseconds an operation; a t or difference that is not a number fails.
fn verdict(t: f64, per_op: f64) -> Verdict {
    let (t, per_op) = (t.abs(), per_op.abs());
    if t.is_nan() || per_op.is_nan() || (t > T_FAIL && per_op >= MIN_FAIL_NANOS) {
        Verdict::Leak
    } else if t > T_FAIL {
        Verdict::UnderFloor
    } else if t > T_REPORT {
        Verdict::Reported
    } else {
        Verdict::Pass
    }
}

/// A target's result, from [`time_target`]: judge it with
/// [`Report::assert_constant_time`], or a positive control with
/// [`Report::assert_leak_found`].
pub struct Report {
    target: String,
    names: Vec<&'static str>,
    by_class: Vec<Vec<Comparison>>,
    batch: u64,
    samples: u64,
    seconds: f64,
}

impl Report {
    /// The comparison that decides a class: of those with |t| above
    /// [`T_FAIL`], the one of the largest difference; otherwise the one of
    /// the largest |t|. `None` when the class has none.
    fn deciding(comparisons: &[Comparison]) -> Option<&Comparison> {
        let over = comparisons
            .iter()
            .filter(|c| c.t.abs() > T_FAIL)
            .max_by(|a, b| a.difference.abs().total_cmp(&b.difference.abs()));
        over.or_else(|| {
            comparisons
                .iter()
                .max_by(|a, b| a.t.abs().total_cmp(&b.t.abs()))
        })
    }

    /// Each class's verdict and deciding comparison.
    fn classes(&self) -> Vec<(Verdict, Option<&Comparison>)> {
        self.by_class
            .iter()
            .map(|comparisons| match Report::deciding(comparisons) {
                None => (Verdict::TooFew, None),
                Some(c) => (verdict(c.t, c.difference / self.batch as f64), Some(c)),
            })
            .collect()
    }

    fn verdict(&self) -> Verdict {
        self.classes()
            .iter()
            .map(|(v, _)| *v)
            .max()
            .unwrap_or(Verdict::TooFew)
    }

    /// One class's line.
    fn class_line(&self, name: &str, verdict: Verdict, c: Option<&Comparison>) -> String {
        match c {
            None => format!(
                "ct_check class | {} | {name} | no comparison | {}",
                self.target,
                verdict.word()
            ),
            Some(c) => {
                let test = match c.test.checked_sub(1) {
                    None => "all samples".to_owned(),
                    Some(i) => format!("crop {i} ({:.1}%)", 100.0 * crop_fraction(i)),
                };
                format!(
                    "ct_check class | {} | {name} | t {:.2} | {:.1} ns/op random | {:+.2} ns/op | \
                     {test} | {} fixed | {} random | {}",
                    self.target,
                    c.t,
                    c.random_mean / self.batch as f64,
                    c.difference / self.batch as f64,
                    c.fixed,
                    c.random,
                    verdict.word()
                )
            }
        }
    }

    /// What the target prints: its row and each class's line.
    fn printed(&self, verdict: &str) -> String {
        let classes = self.classes();
        let worst = classes
            .iter()
            .zip(&self.names)
            .max_by(|((a, ca), _), ((b, cb), _)| {
                a.cmp(b).then_with(|| {
                    let t = |c: &Option<&Comparison>| c.map_or(0.0, |c| c.t.abs());
                    t(ca).total_cmp(&t(cb))
                })
            });
        let mut out = format!(
            "ct_check row | {} | {} samples in {:.1} s | batch {} | ",
            self.target, self.samples, self.seconds, self.batch
        );
        if let Some(((_, Some(c)), name)) = worst {
            let _ = write!(
                out,
                "largest: {name}, t {:.2}, {:+.2} ns/op | ",
                c.t,
                c.difference / self.batch as f64
            );
        }
        out += verdict;
        out.push('\n');
        for ((v, c), name) in classes.iter().zip(&self.names) {
            out += &self.class_line(name, *v, *c);
            out.push('\n');
        }
        out
    }

    /// Prints the row and the classes' lines, and fails when a fixed class
    /// has a comparison of |t| above [`T_FAIL`] on a difference of
    /// [`MIN_FAIL_NANOS`] an operation or more, or when a class had no
    /// comparison with [`MIN_SAMPLES`] on each side. Returns what it
    /// printed.
    ///
    /// # Panics
    ///
    /// On a leak or on too few samples, with the lines in the message.
    pub fn assert_constant_time(&self) -> String {
        let verdict = self.verdict();
        let printed = self.printed(&verdict.word());
        print!("{printed}");
        match verdict {
            Verdict::Pass | Verdict::Reported | Verdict::UnderFloor => printed,
            Verdict::Leak => panic!(
                "{}: a fixed class's time differs from the random class's, |t| above {T_FAIL} \
                 on {MIN_FAIL_NANOS} ns an operation or more\n{printed}",
                self.target
            ),
            Verdict::TooFew => panic!(
                "{}: a class had no comparison with {MIN_SAMPLES} samples on each side; raise \
                 {BUDGET_VAR}\n{printed}",
                self.target
            ),
        }
    }

    /// The positive control's check: prints the row and the classes' lines,
    /// and fails unless the control fails by the targets' rule, since a run
    /// that does not see the control's leak, far larger than any a target
    /// looks for, sees none. Returns what it printed.
    ///
    /// # Panics
    ///
    /// When the control's leak is not found.
    pub fn assert_leak_found(&self) -> String {
        let found = self.verdict() == Verdict::Leak;
        let printed = self.printed(if found {
            "control found"
        } else {
            "CONTROL NOT FOUND"
        });
        print!("{printed}");
        assert!(
            found,
            "{}: the control's leak was not found: this run's clock or measurement does not see \
             a leak\n{printed}",
            self.target
        );
        printed
    }
}

/// The budget [`BUDGET_VAR`]'s value gives, in seconds.
fn budget_seconds(value: Option<&str>) -> Result<u64, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_SECONDS);
    };
    match value.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(seconds),
        _ => Err(format!(
            "{BUDGET_VAR}={value:?} is not a whole number of seconds above zero"
        )),
    }
}

/// The inputs of a target whose fixed classes are `classes`, every class's
/// the same way: `draw` fills a buffer, a fixed class's bytes (of the same
/// length) are written over it, and `build` makes the input from it. The
/// placebo, the one class of no bytes, keeps its draw.
fn by_class<I>(
    classes: &[(&'static str, Vec<u8>)],
    mut draw: impl FnMut(&mut SplitMix64) -> Vec<u8>,
    mut build: impl FnMut(&[u8]) -> I,
) -> impl FnMut(Option<usize>, &mut SplitMix64) -> I {
    move |class, rng| {
        let mut bytes = draw(rng);
        if let Some(fixed) = class {
            let (label, value) = &classes[fixed];
            assert_eq!(
                value.is_empty(),
                *label == PLACEBO,
                "{label}: only the placebo is a class of no bytes"
            );
            if !value.is_empty() {
                bytes.copy_from_slice(value);
            }
        }
        build(&bytes)
    }
}

/// Times `op` as the target `target` for the budget [`BUDGET_VAR`] gives,
/// on inputs `build` makes from byte strings: each fixed class's own (of
/// the length `draw` gives) and, for the random class and the placebo, one
/// that `draw` makes. The generator is seeded through
/// [`SplitMix64::from_env`] with the target's name.
///
/// # Panics
///
/// When [`BUDGET_VAR`] is set to anything but a whole number of seconds
/// above zero.
pub fn time_target<I, R>(
    target: &str,
    classes: &[(&'static str, Vec<u8>)],
    draw: impl FnMut(&mut SplitMix64) -> Vec<u8>,
    build: impl FnMut(&[u8]) -> I,
    op: impl FnMut(&mut I) -> R,
) -> Report {
    let value = env::var(BUDGET_VAR).ok();
    let seconds = budget_seconds(value.as_deref()).unwrap_or_else(|e| panic!("{e}"));
    let mut rng = SplitMix64::from_env(target);
    let measured = measure(
        &WallClock(Instant::now()),
        &mut rng,
        seconds * 1_000_000_000,
        classes.len(),
        by_class(classes, draw, build),
        op,
    );
    report(target, classes, &measured)
}

fn report(target: &str, classes: &[(&'static str, Vec<u8>)], measured: &Measured) -> Report {
    Report {
        target: target.to_owned(),
        names: classes.iter().map(|(name, _)| *name).collect(),
        by_class: measured.stats.by_class(),
        batch: measured.batch,
        samples: measured.samples,
        seconds: measured.nanos as f64 / 1e9,
    }
}

#[cfg(test)]
mod tests {
    //! The harness on a simulated clock: each operation advances it by a
    //! cost the test sets, and nothing is timed.

    use std::cell::Cell;

    use super::*;

    struct Simulated(Cell<u64>);

    impl Clock for Simulated {
        fn now(&self) -> u64 {
            self.0.get()
        }
    }

    /// Runs a simulated target of one fixed class (input byte 1) and a
    /// placebo against the random class (input byte 0) for `budget`
    /// simulated nanoseconds: each operation costs `base` plus up to
    /// `noise` from a generator that knows nothing of the class, plus
    /// `extra` per operation for the fixed class.
    fn simulate(base: u64, noise: u64, extra: u64, budget: u64) -> Report {
        let clock = Simulated(Cell::new(0));
        let mut jitter = SplitMix64(7);
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("fixed", vec![1]), (PLACEBO, vec![])];
        let measured = measure(
            &clock,
            &mut SplitMix64(1),
            budget,
            classes.len(),
            by_class(&classes, |_| vec![0], |b| b[0]),
            |input: &mut u8| {
                let cost = base + jitter.below(noise + 1) + extra * u64::from(*input);
                clock.0.set(clock.0.get() + cost);
            },
        );
        report("simulated", &classes, &measured)
    }

    #[test]
    fn no_difference_passes() {
        let r = simulate(600, 20, 0, 200_000_000);
        assert_eq!(r.batch, 4, "2 µs of 600 ns operations");
        assert!(r.samples > 50_000, "{}", r.samples);
        assert!(r.verdict() <= Verdict::Reported, "{}", r.printed("?"));
        r.assert_constant_time();
    }

    #[test]
    fn a_leak_of_two_nanoseconds_an_operation_is_found() {
        let r = simulate(600, 20, 2, 200_000_000);
        assert_eq!(r.verdict(), Verdict::Leak, "{}", r.printed("?"));
        let printed = r.assert_leak_found();
        assert!(printed.contains("ct_check row | simulated |"), "{printed}");
        // The placebo, drawn as the random class is, does not lean.
        let placebo = &r.classes()[1];
        assert!(placebo.0 <= Verdict::Reported, "{}", r.printed("?"));
        assert!(std::panic::catch_unwind(|| r.assert_constant_time()).is_err());
    }

    #[test]
    fn a_clear_difference_under_the_floor_passes() {
        // One nanosecond a sample of four operations: a quarter of the floor,
        // and |t| far above 10 at this noise.
        let clock = Simulated(Cell::new(0));
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("fixed", vec![1])];
        let mut n = 0u64;
        let measured = measure(
            &clock,
            &mut SplitMix64(2),
            200_000_000,
            1,
            by_class(&classes, |_| vec![0], |b| b[0]),
            |input: &mut u8| {
                n += 1;
                // The fixed class's extra nanosecond on one operation in four.
                let extra = u64::from(*input) * u64::from(n.is_multiple_of(4));
                clock.0.set(clock.0.get() + 600 + (n % 3) + extra);
            },
        );
        let r = report("simulated", &classes, &measured);
        assert_eq!(r.verdict(), Verdict::UnderFloor, "{}", r.printed("?"));
        r.assert_constant_time();
        assert!(std::panic::catch_unwind(|| r.assert_leak_found()).is_err());
    }

    #[test]
    fn a_class_never_sampled_is_too_few() {
        // A budget that ends inside the warm-up leaves no comparison.
        let r = simulate(600, 20, 0, 300_000);
        assert_eq!(r.verdict(), Verdict::TooFew);
        assert!(std::panic::catch_unwind(|| r.assert_constant_time()).is_err());
    }

    #[test]
    fn verdict_rule() {
        assert_eq!(verdict(3.0, 50.0), Verdict::Pass);
        assert_eq!(verdict(-5.0, 0.1), Verdict::Reported);
        assert_eq!(verdict(11.0, 0.9), Verdict::UnderFloor);
        assert_eq!(verdict(-11.0, -1.0), Verdict::Leak);
        assert_eq!(verdict(f64::NAN, 0.0), Verdict::Leak);
        assert_eq!(budget_seconds(None), Ok(DEFAULT_SECONDS));
        assert_eq!(budget_seconds(Some("3")), Ok(3));
        assert!(budget_seconds(Some("0")).is_err());
        assert!(budget_seconds(Some("1.5")).is_err());
    }

    #[test]
    fn welch_t_and_crops() {
        let mut a = Moments::default();
        let mut b = Moments::default();
        for x in [1.0, 2.0, 3.0] {
            a.push(x);
            b.push(x + 1.0);
        }
        assert!((a.mean - 2.0).abs() < 1e-12 && (a.variance() - 1.0).abs() < 1e-12);
        // Means 2 and 3, each variance 1 over 3 samples: t = -1 / sqrt(2/3).
        assert!((welch_t(&a, &b) + 1.0 / (2.0f64 / 3.0).sqrt()).abs() < 1e-12);
        assert_eq!(welch_t(&a, &a), 0.0);
        assert!((crop_fraction(0) - (1.0 - 0.5f64.powf(0.1))).abs() < 1e-12);
        assert!(crop_fraction(99) > 0.999 && crop_fraction(99) < 1.0);
    }

    #[test]
    #[should_panic(expected = "only the placebo")]
    fn an_empty_class_must_be_the_placebo() {
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("empty", vec![])];
        let mut build = by_class(&classes, |_| vec![0], |b| b[0]);
        build(Some(0), &mut SplitMix64(0));
    }
}
