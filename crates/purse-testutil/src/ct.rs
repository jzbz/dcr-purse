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
//! what this workspace's targets need: no known reports and no DIT probe.
//! The workspace does not link `dcroxide-testutil`; a port keeps it out of
//! the lockfile. The port left out dcroxide's per-target trace too, as
//! more than the targets then needed; since 2026-11-24 it is ported
//! (below, "The trace"), after a failure on GitHub that its log could not
//! explain (ADR-0012, addendum of 2026-11-24). Known reports stay out: no
//! target here has a failure measured and judged as one.
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
//! `ct_check row |`, a line for each class, marked `ct_check class |`, and
//! the trace. The harness's own tests run in every `cargo test` and time
//! nothing: they drive it with a simulated clock.
//!
//! # The trace
//!
//! Below its class lines, whatever its verdict, and in a failure's message,
//! each target prints lines that let a failure be read against other runs
//! and against the machine (ported from dcroxide's `8053abe`, with the
//! crop shares and the processor's name added). None of it changes what is
//! timed, the classes' shares, the crops or the verdict:
//!
//! * `ct_check run |`: the samples counted and the warm-up's, the
//!   segments' length, the address of one of the harness's functions and
//!   of the first chunk of inputs, the processors the thread may run on
//!   (`Cpus_allowed_list` of `/proc/self/status`, one when the run is
//!   pinned) and the processor's name (the first `model name` of
//!   `/proc/cpuinfo`; `unknown` where there is none, as off Linux).
//! * `ct_check random |`: the random class's mean nanoseconds an
//!   operation in the uncropped test, in crop 32 ([`FIXED_CROP`]) and in
//!   each of six equal segments of the counted time ([`SEGMENTS`]), with
//!   the samples crop 32 actually holds, of every class together and of the
//!   random class, as counts and percentages of the run's, beside its
//!   nominal share.
//! * `ct_check trace |`, a line a class: its t and difference in the
//!   uncropped test and in crop 32, whatever their |t|, so that runs can be
//!   pooled on the same two tests and not on a maximum, the class's samples
//!   crop 32 holds, and its t in each segment, which tells a difference
//!   that lasts the run from one that comes and goes. The segments are
//!   told apart between chunks of samples, to the nearest chunk, so that
//!   the loop over a chunk's samples counts each sample as it did before
//!   the trace. A chunk holds at most 4,096 operations and
//!   [`CHUNK_BYTES`] of inputs, caps that bind before its 100 ms aim for
//!   every target here: measured at opt-level 3, a chunk lasts about a
//!   millisecond for the `chars` targets (0.9 to 1.4 ms) and up to about
//!   16 ms for the `decode` and `encode` targets, at most about 1% of a
//!   segment at the push budget of 10 s.
//! * `ct_check share |`, a line a class: for the comparison that decides
//!   the class, the test's nominal share of the warm-up (a crop's quantile)
//!   and its threshold, the samples of the class and of the random class in
//!   it, each as a count and a percentage of that class's samples in the
//!   run, and the percentage of every class's samples together. A crop's
//!   thresholds come from the warm-up's samples alone, so a crop can keep
//!   far less of the run than its nominal share; a verdict decided in such
//!   a tail shows it here.
//!
//! # The diagnostic pass
//!
//! [`time_diagnostic`] times, for the same budget as a target and in a
//! test of its own run after the target's, a second pass of the target's
//! classes and some more, the diagnostic classes, and
//! [`Report::print_not_judged`] prints its row (the target's name followed
//! by `, diagnostic pass`), class lines and trace, and never fails. The
//! target's own test, the verdict's pass, is untouched by it: the same
//! classes, each with the same share, the same generator and the same
//! rule. Timing the diagnostic classes in the verdict's pass instead would
//! have cut every class's share, and with it every t at a fixed budget,
//! which measurably weakened what the targets find (ADR-0012, addendum of
//! 2026-11-24). The diagnostic pass itself has those smaller shares, and
//! for some leaks a different mix of inputs changes the leak, so it sees
//! less than the verdict's pass. It times its own later window, with its
//! own warm-up and thresholds, so it shows only a difference that lasts
//! into that window, and may show nothing of a passing event: what it shows
//! is read beside the verdict's pass, and a class that passes it shows
//! nothing.
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
use std::fs;
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

/// The crop whose comparison each class's trace line gives beside the
/// uncropped test's, whatever their |t|: crop 32, which keeps the samples
/// quicker than the warm-up's quantile 0.898, the nearest of dudect's to
/// nine tenths (dcroxide's choice). A class's verdict is decided by one
/// comparison of many, so results pooled over runs are statistics of a
/// maximum; these two are the same tests in every run. A crop's thresholds
/// come from the warm-up alone, so the share of the run's samples crop 32
/// holds can drift from its nominal share, and the trace prints it beside
/// it. The trace's alone: the verdict does not read it.
pub const FIXED_CROP: usize = 32;

/// The equal parts of a run's counted time, from the warm-up's end to the
/// budget's, in each of which the trace also compares every fixed class
/// with the random class, uncropped: a sixth of the run each, about 1.5 s
/// at 10 s and 10 s at 60 s. A difference that lasts the whole run shows in
/// every one; one that comes and goes, in some. The trace's alone.
pub const SEGMENTS: usize = 6;

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

    /// The moments of the times pushed since `earlier`, a state these
    /// moments passed through: Chan, Golub and LeVeque's combination of two
    /// parts, solved for the later part.
    fn less(&self, earlier: &Moments) -> Moments {
        let n = self.n - earlier.n;
        if n == 0 {
            return Moments::default();
        }
        let mean = (self.mean * self.n as f64 - earlier.mean * earlier.n as f64) / n as f64;
        let delta = mean - earlier.mean;
        let m2 = self.m2 - earlier.m2 - delta * delta * earlier.n as f64 * n as f64 / self.n as f64;
        Moments {
            n,
            mean,
            m2: m2.max(0.0),
        }
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

/// The segment ([`SEGMENTS`]) of the time `began` (a chunk boundary), of
/// the equal parts of the time from `from`, the first such boundary after
/// the warm-up, to `until`, where the budget ends: the last for a time at
/// `until` or later, and the first for one before `from`.
fn segment(began: u64, from: u64, until: u64) -> usize {
    let span = until.saturating_sub(from).max(1);
    let at = began.saturating_sub(from);
    (at.saturating_mul(SEGMENTS as u64) / span).min(SEGMENTS as u64 - 1) as usize
}

/// Every class's moments in the uncropped test in each segment of the
/// counted time, `[segment][slot]`, from the marks [`Stats::mark`] left and
/// the moments at the run's end (`end`): segment k's are the samples
/// counted between mark k and mark k + 1, or the end after the last mark,
/// and a segment the run did not reach has none.
fn segment_moments(marks: &[Vec<Moments>], end: &[Moments]) -> Vec<Vec<Moments>> {
    let empty = vec![Moments::default(); end.len()];
    let first = [empty.clone()];
    let marks = if marks.is_empty() { &first[..] } else { marks };
    (0..SEGMENTS)
        .map(|k| match marks.get(k) {
            None => empty.clone(),
            Some(from) => {
                let to = marks.get(k + 1).map_or(end, Vec::as_slice);
                to.iter()
                    .zip(from)
                    .map(|(to, from)| to.less(from))
                    .collect()
            }
        })
        .collect()
}

/// Each fixed class's t against the random class in each segment, from
/// [`segment_moments`], in the segments' order: `None` where a side has
/// fewer than [`MIN_SAMPLES`].
fn by_segment(segments: &[Vec<Moments>]) -> Vec<Vec<Option<f64>>> {
    let slots = segments.first().map_or(0, Vec::len);
    (1..slots)
        .map(|slot| {
            segments
                .iter()
                .map(|segment| {
                    (segment[0].n >= MIN_SAMPLES && segment[slot].n >= MIN_SAMPLES)
                        .then(|| welch_t(&segment[slot], &segment[0]))
                })
                .collect()
        })
        .collect()
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

    /// Notes, between two chunks, that the counted time has reached
    /// segment `segment` ([`segment`]): `marks[k]` becomes every class's
    /// moments in the uncropped test as they stood when segment k began,
    /// to the nearest chunk ([`segment_moments`]). The first mark is empty,
    /// so that samples counted before the first chunk boundary after the
    /// warm-up fall in segment 0. The trace's alone, and outside the loop
    /// over a chunk's samples, whose code it leaves as it was.
    fn mark(&self, marks: &mut Vec<Vec<Moments>>, segment: usize) {
        if marks.is_empty() {
            marks.push(vec![Moments::default(); self.tests[0].len()]);
        }
        while marks.len() <= segment {
            marks.push(self.tests[0].clone());
        }
    }

    /// Every class's count of samples in every test, `[test][slot]`.
    fn counts(&self) -> Vec<Vec<u64>> {
        self.tests
            .iter()
            .map(|test| test.iter().map(|m| m.n).collect())
            .collect()
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
    /// The samples the warm-up took and threw away.
    warm_up: usize,
    /// The clock's time at the first chunk boundary after the warm-up,
    /// where the segments begin (the samples counted before it, at most a
    /// chunk's, fall in the first).
    counted_from: u64,
    /// The clock's time when the budget ends, where the segments end.
    deadline: u64,
    /// The trace's marks of the segments ([`Stats::mark`]).
    marks: Vec<Vec<Moments>>,
    /// The address of the first chunk's inputs, on the heap.
    inputs_at: usize,
}

/// Times `op` for `budget` nanoseconds of `clock` on inputs `build` makes
/// from the sample's class (`None` for the random class) and the
/// generator. The batch is sized from the median of single operations on
/// the random class; then, chunk after chunk, each sample's class is drawn
/// and its batch's inputs built, the whole chunk's before any of it is
/// timed, and each sample is timed as its batch, each input and result
/// through `black_box`, the results dropped after the clock is read.
/// Between chunks, after the warm-up, the trace's marks of the segments
/// ([`Stats::mark`]) are taken; nothing is added to the loop over a chunk's
/// samples, which counts each as it did before the trace.
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
    let mut counted_from = start;
    let mut marks = Vec::new();
    let mut inputs_at = None;
    loop {
        if let Some(stats) = &stats {
            let now = clock.now();
            if marks.is_empty() {
                counted_from = now;
            }
            stats.mark(&mut marks, segment(now, counted_from, deadline));
        }
        let classes: Vec<Option<usize>> = (0..chunk).map(|_| draw_class(rng, fixed)).collect();
        let mut inputs = Vec::with_capacity((chunk * batch) as usize);
        for &class in &classes {
            for _ in 0..batch {
                inputs.push(build(class, rng));
            }
        }
        inputs_at.get_or_insert(inputs.as_ptr().addr());
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
                    warm_up: warm_up.len(),
                    counted_from,
                    deadline,
                    marks,
                    inputs_at: inputs_at.unwrap_or_default(),
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
    /// False for a diagnostic pass ([`time_diagnostic`]), whose lines say
    /// that it is not judged; it changes only what is printed.
    judged: bool,
    trace: Trace,
}

/// What a target's trace lines give besides its verdict (the module's
/// documentation, "The trace").
#[derive(Debug, Default)]
struct Trace {
    /// Each fixed class's t in each segment ([`Stats::by_segment`]).
    by_segment: Vec<Vec<Option<f64>>>,
    /// The random class's moments in the uncropped test, [`FIXED_CROP`]
    /// and each segment ([`Stats::random`]).
    random: Vec<Moments>,
    /// Every class's samples in every test ([`Stats::counts`]).
    counts: Vec<Vec<u64>>,
    /// Each crop's threshold in nanoseconds a sample.
    thresholds: Vec<u64>,
    /// The samples the warm-up took and threw away.
    warm_up: usize,
    /// A segment's length in nanoseconds.
    segment_nanos: u64,
    /// The address of one of the harness's functions, `Stats::push`: the
    /// binary is mapped whole, so with its symbols every function's
    /// address follows from this one.
    code: usize,
    /// The address of the run's first chunk of inputs, on the heap.
    inputs: usize,
    /// The processors the thread may run on ([`allowed_cpus`]).
    cpus: String,
    /// The processor's name ([`cpu_model`]).
    cpu: String,
}

/// The processors this thread may run on, from `status`, the text of
/// `/proc/self/status`: its `Cpus_allowed_list` line's value, or `-` when
/// there is none, as on a system without `/proc`.
fn allowed_cpus(status: Option<&str>) -> String {
    status
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        })
        .map_or_else(|| "-".to_owned(), |list| list.trim().to_owned())
}

/// The processor's name, from `cpuinfo`, the text of `/proc/cpuinfo`: the
/// value of its first `model name` line, or `unknown` when there is none,
/// as on a system without `/proc` or on a processor whose kernel gives no
/// such line.
fn cpu_model(cpuinfo: Option<&str>) -> String {
    cpuinfo
        .and_then(|info| {
            info.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim() == "model name").then(|| value.trim())
            })
        })
        .filter(|name| !name.is_empty())
        .map_or_else(|| "unknown".to_owned(), str::to_owned)
}

/// The name of test `test`: every sample, or a crop by the share of the
/// warm-up's times it keeps.
fn test_name(test: usize) -> String {
    match test.checked_sub(1) {
        None => "all samples".to_owned(),
        Some(i) => format!("crop {i} ({:.1}%)", 100.0 * crop_fraction(i)),
    }
}

/// `part` as a percentage of `whole`, `-` when `whole` is zero.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        "-".to_owned()
    } else {
        format!("{:.1}%", 100.0 * part as f64 / whole as f64)
    }
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

    /// A class's verdict as its line gives it: the verdict's word, or, in
    /// a diagnostic pass, which is not judged, what the rule would say,
    /// in lower case and marked as not judged.
    fn word(&self, verdict: Verdict) -> String {
        if self.judged {
            verdict.word()
        } else {
            format!(
                "not judged (by the rule: {})",
                verdict.word().to_lowercase()
            )
        }
    }

    /// One class's line.
    fn class_line(&self, name: &str, verdict: Verdict, c: Option<&Comparison>) -> String {
        match c {
            None => format!(
                "ct_check class | {} | {name} | no comparison | {}",
                self.target,
                self.word(verdict)
            ),
            Some(c) => {
                let test = test_name(c.test);
                format!(
                    "ct_check class | {} | {name} | t {:.2} | {:.1} ns/op random | {:+.2} ns/op | \
                     {test} | {} fixed | {} random | {}",
                    self.target,
                    c.t,
                    c.random_mean / self.batch as f64,
                    c.difference / self.batch as f64,
                    c.fixed,
                    c.random,
                    self.word(verdict)
                )
            }
        }
    }

    /// The trace (the module's documentation, "The trace"), the same lines
    /// whatever the verdict, each marked at its start, a `-` where there is
    /// nothing to give. The share line gives, for the comparison that
    /// decides each class (what its class line shows), the samples of the
    /// class and of the random class in that test as counts and as
    /// percentages of each one's samples in the run.
    fn trace_lines(&self) -> String {
        let trace = &self.trace;
        let batch = self.batch as f64;
        let mut out = format!(
            "ct_check run | {} | {} samples after {} to warm up | {SEGMENTS} segments of {:.2} s \
             | code {:#x} | inputs {:#x} | cpus {} | cpu {}\n",
            self.target,
            self.samples,
            trace.warm_up,
            trace.segment_nanos as f64 / 1e9,
            trace.code,
            trace.inputs,
            trace.cpus,
            trace.cpu
        );
        let mean = |m: Option<&Moments>| match m {
            Some(m) if m.n > 0 => format!("{:.2}", m.mean / batch),
            _ => "-".to_owned(),
        };
        let segments: Vec<String> = (0..SEGMENTS)
            .map(|k| mean(trace.random.get(2 + k)))
            .collect();
        let count = |test: usize, slot: usize| {
            trace
                .counts
                .get(test)
                .and_then(|test| test.get(slot))
                .copied()
                .unwrap_or(0)
        };
        let slots = trace.counts.first().map_or(0, Vec::len);
        let pooled = |test: usize| (0..slots).map(|s| count(test, s)).sum::<u64>();
        // What crop 32 actually holds, beside its nominal share: its
        // thresholds come from the warm-up alone, so in a run that slowed
        // after the warm-up it can hold a thin tail of the run.
        let held = |slot: Option<usize>| {
            let (part, whole) = match slot {
                Some(slot) => (count(1 + FIXED_CROP, slot), count(0, slot)),
                None => (pooled(1 + FIXED_CROP), pooled(0)),
            };
            format!("holds {part} of {whole} ({})", percent(part, whole))
        };
        let _ = writeln!(
            out,
            "ct_check random | {} | ns/op | all samples {} | {} {}, every class {}, random {} | \
             segments {}",
            self.target,
            mean(trace.random.first()),
            test_name(1 + FIXED_CROP),
            mean(trace.random.get(1)),
            held(None),
            held(Some(0)),
            segments.join(" ")
        );
        for (class, ((name, comparisons), (_, deciding))) in self
            .names
            .iter()
            .zip(&self.by_class)
            .zip(self.classes())
            .enumerate()
        {
            let at = |test: usize| match comparisons.iter().find(|c| c.test == test) {
                Some(c) => format!("t {:+.2} {:+.2} ns/op", c.t, c.difference / batch),
                None => "-".to_owned(),
            };
            let segments: Vec<String> = (0..SEGMENTS)
                .map(|k| {
                    match trace
                        .by_segment
                        .get(class)
                        .and_then(|segments| segments.get(k).copied().flatten())
                    {
                        Some(t) => format!("{t:+.2}"),
                        None => "-".to_owned(),
                    }
                })
                .collect();
            let _ = writeln!(
                out,
                "ct_check trace | {} | {name} | all samples {} | {} {}, {} | t by segment {}",
                self.target,
                at(0),
                test_name(1 + FIXED_CROP),
                at(1 + FIXED_CROP),
                held(Some(1 + class)),
                segments.join(" ")
            );
            let Some(c) = deciding else {
                let _ = writeln!(
                    out,
                    "ct_check share | {} | {name} | no comparison",
                    self.target
                );
                continue;
            };
            let slot = 1 + class;
            let threshold = match c.test.checked_sub(1) {
                None => "every sample".to_owned(),
                Some(i) => match trace.thresholds.get(i) {
                    Some(&ns) => format!("under {ns} ns a sample"),
                    None => "-".to_owned(),
                },
            };
            let all_in = pooled(c.test);
            let all = pooled(0);
            let _ = writeln!(
                out,
                "ct_check share | {} | {name} | deciding {}, {threshold} | {name} {} of {} ({}) | \
                 random {} of {} ({}) | every class {} of {} ({})",
                self.target,
                test_name(c.test),
                count(c.test, slot),
                count(0, slot),
                percent(count(c.test, slot), count(0, slot)),
                count(c.test, 0),
                count(0, 0),
                percent(count(c.test, 0), count(0, 0)),
                all_in,
                all,
                percent(all_in, all)
            );
        }
        out
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
        out + &self.trace_lines()
    }

    /// Prints the row, the classes' lines and the trace, and fails when a
    /// fixed class has a comparison of |t| above [`T_FAIL`] on a difference
    /// of [`MIN_FAIL_NANOS`] an operation or more, or when a class had no
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

    /// A diagnostic pass's output ([`time_diagnostic`]): prints the row,
    /// whose verdict reads `not judged (diagnostic pass)`, the classes'
    /// lines, each giving what the rule would say and marked as not judged,
    /// and the trace. It never fails, whatever the classes' times. Returns
    /// what it printed.
    pub fn print_not_judged(&self) -> String {
        let printed = self.printed("not judged (diagnostic pass)");
        print!("{printed}");
        printed
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

/// Times `op` as the diagnostic pass of the target `target`, for the
/// budget [`BUDGET_VAR`] gives, on `classes` followed by `diagnostic`,
/// every class with an equal share, as [`time_target`] times a target: its
/// generator seeded through [`SplitMix64::from_env`] with the target's
/// name followed by `, diagnostic pass`, the name it is reported as. A
/// test of its own calls it, after the target's, and prints the report
/// with [`Report::print_not_judged`]: never judged (the module's
/// documentation, "The diagnostic pass").
///
/// # Panics
///
/// When [`BUDGET_VAR`] is set to anything but a whole number of seconds
/// above zero.
pub fn time_diagnostic<I, R>(
    target: &str,
    classes: &[(&'static str, Vec<u8>)],
    diagnostic: &[(&'static str, Vec<u8>)],
    draw: impl FnMut(&mut SplitMix64) -> Vec<u8>,
    build: impl FnMut(&[u8]) -> I,
    op: impl FnMut(&mut I) -> R,
) -> Report {
    let value = env::var(BUDGET_VAR).ok();
    let seconds = budget_seconds(value.as_deref()).unwrap_or_else(|e| panic!("{e}"));
    let name = diagnostic_name(target);
    let all = with_diagnostic(classes, diagnostic);
    let measured = measure(
        &WallClock(Instant::now()),
        &mut SplitMix64::from_env(&name),
        seconds * 1_000_000_000,
        all.len(),
        by_class(&all, draw, build),
        op,
    );
    unjudged(report(&name, &all, &measured))
}

/// The name a diagnostic pass of `target` is reported as.
fn diagnostic_name(target: &str) -> String {
    format!("{target}, diagnostic pass")
}

/// The classes of a diagnostic pass: `classes`, then `diagnostic`.
fn with_diagnostic(
    classes: &[(&'static str, Vec<u8>)],
    diagnostic: &[(&'static str, Vec<u8>)],
) -> Vec<(&'static str, Vec<u8>)> {
    classes.iter().chain(diagnostic).cloned().collect()
}

/// `report`, as a diagnostic pass's: its lines say it is not judged.
fn unjudged(report: Report) -> Report {
    Report {
        judged: false,
        ..report
    }
}

fn report(target: &str, classes: &[(&'static str, Vec<u8>)], measured: &Measured) -> Report {
    let push: fn(&mut Stats, usize, u64) = Stats::push;
    let stats = &measured.stats;
    let segments = segment_moments(&measured.marks, &stats.tests[0]);
    Report {
        target: target.to_owned(),
        names: classes.iter().map(|(name, _)| *name).collect(),
        by_class: stats.by_class(),
        batch: measured.batch,
        samples: measured.samples,
        seconds: measured.nanos as f64 / 1e9,
        judged: true,
        trace: Trace {
            by_segment: by_segment(&segments),
            random: [&stats.tests[0], &stats.tests[1 + FIXED_CROP]]
                .into_iter()
                .chain(&segments)
                .map(|test| test[0])
                .collect(),
            counts: stats.counts(),
            thresholds: stats.thresholds.clone(),
            warm_up: measured.warm_up,
            segment_nanos: measured.deadline.saturating_sub(measured.counted_from)
                / SEGMENTS as u64,
            code: push as usize,
            inputs: measured.inputs_at,
            cpus: allowed_cpus(fs::read_to_string("/proc/self/status").ok().as_deref()),
            cpu: cpu_model(fs::read_to_string("/proc/cpuinfo").ok().as_deref()),
        },
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

    /// A diagnostic pass ([`time_diagnostic`]) on the simulated clock, on
    /// `classes` and `diagnostic` with the generator `SplitMix64(2)`. Each
    /// operation costs 600 ns plus up to 20 from a generator that knows
    /// nothing of the class, plus `extra[b]` for an input of byte `b` (the
    /// random class's is 0).
    fn simulate_diagnostic(
        classes: &[(&'static str, Vec<u8>)],
        diagnostic: &[(&'static str, Vec<u8>)],
        extra: [u64; 4],
        budget: u64,
    ) -> Report {
        let clock = Simulated(Cell::new(0));
        let mut jitter = SplitMix64(7);
        let all = with_diagnostic(classes, diagnostic);
        let measured = measure(
            &clock,
            &mut SplitMix64(2),
            budget,
            all.len(),
            by_class(&all, |_| vec![0], |b| b[0]),
            |input: &mut u8| {
                let cost = 600 + jitter.below(21) + extra[usize::from(*input)];
                clock.0.set(clock.0.get() + cost);
            },
        );
        unjudged(report(&diagnostic_name("simulated"), &all, &measured))
    }

    fn lines<'a>(printed: &'a str, marker: &str) -> Vec<&'a str> {
        printed
            .lines()
            .filter(|line| line.starts_with(marker))
            .collect()
    }

    #[test]
    fn segments_split_the_counted_time() {
        assert_eq!(segment(0, 100, 700), 0, "before the warm-up's end");
        assert_eq!(segment(100, 100, 700), 0);
        assert_eq!(segment(199, 100, 700), 0);
        assert_eq!(segment(200, 100, 700), 1);
        assert_eq!(segment(699, 100, 700), 5);
        assert_eq!(segment(700, 100, 700), 5, "at the budget's end");
        assert_eq!(segment(9_000, 100, 700), 5, "after it");
        assert_eq!(segment(5, 7, 7), 0, "an empty span");
    }

    #[test]
    fn the_processor_and_its_allowed_set_are_read() {
        let cpuinfo = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD EPYC \
                       9V74 80-Core Processor\n\nprocessor\t: 1\nmodel name\t: other\n";
        assert_eq!(cpu_model(Some(cpuinfo)), "AMD EPYC 9V74 80-Core Processor");
        // An arm64 kernel's /proc/cpuinfo has no model name line.
        assert_eq!(
            cpu_model(Some("processor\t: 0\nCPU implementer\t: 0x41\n")),
            "unknown"
        );
        assert_eq!(cpu_model(Some("model name\t:\n")), "unknown");
        assert_eq!(cpu_model(None), "unknown");
        let status = "Name:\tct\nCpus_allowed:\t80000000\nCpus_allowed_list:\t31\n";
        assert_eq!(allowed_cpus(Some(status)), "31");
        assert_eq!(allowed_cpus(Some("Name:\tct\n")), "-");
        assert_eq!(allowed_cpus(None), "-");
    }

    #[test]
    fn the_trace_gives_every_class_its_tests_segments_and_share() {
        let r = simulate(600, 20, 2, 200_000_000);
        let printed = r.printed("?");
        let run = lines(&printed, "ct_check run | simulated | ");
        assert_eq!(run.len(), 1, "{printed}");
        assert!(run[0].contains(" | 6 segments of "), "{printed}");
        assert!(run[0].contains(" | cpus "), "{printed}");
        assert!(run[0].contains(" | cpu "), "{printed}");
        let random = lines(
            &printed,
            "ct_check random | simulated | ns/op | all samples ",
        );
        assert_eq!(random.len(), 1, "{printed}");
        let segments = random[0].split(" | segments ").nth(1).expect("segments");
        assert_eq!(segments.split(' ').count(), SEGMENTS, "{printed}");
        assert!(random[0].contains(" | crop 32 (89.8%) "), "{printed}");
        assert!(random[0].contains(", every class holds "), "{printed}");

        let traces = lines(&printed, "ct_check trace | simulated | ");
        let shares = lines(&printed, "ct_check share | simulated | ");
        assert_eq!(traces.len(), 2, "one a class: {printed}");
        assert_eq!(shares.len(), 2, "one a class: {printed}");
        // The leaking class leans in every segment; t is given in each.
        let by_segment = traces[0].split(" | t by segment ").nth(1).expect("t");
        let ts: Vec<f64> = by_segment
            .split(' ')
            .map(|t| t.parse().expect("a t"))
            .collect();
        assert_eq!(ts.len(), SEGMENTS);
        assert!(ts.iter().all(|&t| t > T_FAIL), "{printed}");
        assert!(traces[0].contains(" | all samples t +"), "{printed}");

        // The share line's counts are the deciding comparison's, out of each
        // class's samples in the run.
        for (class, share) in shares.iter().enumerate() {
            let (_, c) = r.classes()[class];
            let c = c.expect("a comparison");
            let slot = 1 + class;
            let counts = &r.trace.counts;
            assert!(
                share.contains(&format!("deciding {}", test_name(c.test))),
                "{share}"
            );
            assert!(
                share.contains(&format!(" {} of {} (", c.fixed, counts[0][slot])),
                "{share}"
            );
            assert!(
                share.contains(&format!(" | random {} of {} (", c.random, counts[0][0])),
                "{share}"
            );
            let total: u64 = counts[0].iter().sum();
            assert_eq!(total, r.samples);
            assert!(share.contains(&format!(" of {total} (")), "{share}");
        }
    }

    #[test]
    fn a_verdict_decided_in_a_thin_crop_shows_its_share() {
        // Each operation costs 600 ns during the warm-up and 640 after it,
        // as if the processor slowed: the crops' thresholds, from the
        // warm-up alone, keep far less of the run than their nominal
        // shares.
        let clock = Simulated(Cell::new(0));
        let mut jitter = SplitMix64(7);
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("fixed", vec![1])];
        let measured = measure(
            &clock,
            &mut SplitMix64(1),
            200_000_000,
            1,
            by_class(&classes, |_| vec![0], |b| b[0]),
            |input: &mut u8| {
                let slow = if clock.0.get() > 5_000_000 { 40 } else { 0 };
                let cost = 600 + slow + jitter.below(41) + 2 * u64::from(*input);
                clock.0.set(clock.0.get() + cost);
            },
        );
        let r = report("simulated", &classes, &measured);
        let counts = &r.trace.counts;
        let all: u64 = counts[0].iter().sum();
        let in_crop_50: u64 = counts[1 + 50].iter().sum();
        assert!(crop_fraction(50) > 0.97);
        assert!(
            (in_crop_50 as f64) < 0.05 * all as f64,
            "{in_crop_50} of {all}"
        );
        let printed = r.printed("?");
        // Crop 32, nominally 89.8% of the warm-up, holds a thin tail of the
        // run, and its lines say so beside the nominal share.
        let in_crop_32: u64 = counts[1 + FIXED_CROP].iter().sum();
        assert!((in_crop_32 as f64) < 0.1 * all as f64, "{in_crop_32}");
        let random = lines(&printed, "ct_check random | simulated | ");
        assert!(
            random[0].contains(&format!(
                "crop 32 (89.8%) {:.2}, every class holds {in_crop_32} of {all} ({}), random \
                 holds {} of {} (",
                r.trace.random[1].mean / r.batch as f64,
                percent(in_crop_32, all),
                counts[1 + FIXED_CROP][0],
                counts[0][0]
            )),
            "{printed}"
        );
        let trace = lines(&printed, "ct_check trace | simulated | fixed | ");
        assert!(
            trace[0].contains(&format!(
                ", holds {} of {} ({}) | t by segment ",
                counts[1 + FIXED_CROP][1],
                counts[0][1],
                percent(counts[1 + FIXED_CROP][1], counts[0][1])
            )),
            "{printed}"
        );
        let share = lines(&printed, "ct_check share | simulated | fixed | deciding ");
        assert_eq!(share.len(), 1, "{printed}");
        let (_, c) = r.classes()[0];
        let c = c.expect("a comparison");
        let pooled = counts[c.test].iter().sum::<u64>();
        assert!(
            share[0].ends_with(&format!(
                "every class {pooled} of {all} ({})",
                percent(pooled, all)
            )),
            "{printed}"
        );
    }

    #[test]
    fn a_diagnostic_pass_is_printed_and_never_judged() {
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("fixed", vec![1]), (PLACEBO, vec![])];
        let diagnostic: Vec<(&'static str, Vec<u8>)> =
            vec![("fixed draw", vec![2]), ("fixed again", vec![1])];

        // A leak of 5 ns an operation on byte 2, which only the diagnostic
        // class "fixed draw" has, and one of 2 ns on byte 1, which the
        // judged class "fixed" and its copy have: the lines show both, by
        // the rule, and printing them does not fail.
        let d = simulate_diagnostic(&classes, &diagnostic, [0, 2, 5, 0], 200_000_000);
        assert_eq!(d.names, ["fixed", PLACEBO, "fixed draw", "fixed again"]);
        let verdicts: Vec<Verdict> = d.classes().iter().map(|(v, _)| *v).collect();
        assert_eq!(verdicts[0], Verdict::Leak);
        assert!(verdicts[1] <= Verdict::Reported, "{}", d.printed("?"));
        assert_eq!(verdicts[2], Verdict::Leak);
        assert_eq!(verdicts[3], Verdict::Leak);
        let printed = d.print_not_judged();
        let row = lines(&printed, "ct_check row | simulated, diagnostic pass | ");
        assert_eq!(row.len(), 1, "{printed}");
        assert!(
            row[0].ends_with("| not judged (diagnostic pass)"),
            "{printed}"
        );
        let draw = lines(
            &printed,
            "ct_check class | simulated, diagnostic pass | fixed draw |",
        );
        assert!(
            draw[0].ends_with("| not judged (by the rule: leak)"),
            "{printed}"
        );
        assert!(!printed.contains("LEAK"), "{printed}");
        assert_eq!(
            lines(&printed, "ct_check trace | simulated, diagnostic pass | ").len(),
            4
        );
        // Each of the five classes has a share of 1/5 here.
        let total: u64 = d.trace.counts[0].iter().sum();
        for &n in &d.trace.counts[0] {
            let share = n as f64 / total as f64;
            assert!((share - 0.2).abs() < 0.01, "{share}");
        }
    }

    #[test]
    fn segments_are_the_differences_of_the_marks() {
        let mut all = Moments::default();
        let mut early = Moments::default();
        let mut late = Moments::default();
        let mut jitter = SplitMix64(3);
        for i in 0..10_000u64 {
            let x = 2_000.0 + jitter.below(97) as f64 + if i >= 3_000 { 40.0 } else { 0.0 };
            all.push(x);
            if i < 3_000 {
                early.push(x);
            } else {
                late.push(x);
            }
            if i == 2_999 {
                let part = all.less(&early);
                assert_eq!((part.n, part.mean, part.m2), (0, 0.0, 0.0));
            }
        }
        let part = all.less(&early);
        assert_eq!(part.n, late.n);
        assert!((part.mean - late.mean).abs() < 1e-9, "{part:?} {late:?}");
        assert!((part.m2 / late.m2 - 1.0).abs() < 1e-9, "{part:?} {late:?}");

        // A run whose fixed class is 3 ns an operation slower in its fourth
        // sixth of the counted time alone: the marks, taken between chunks,
        // put the difference in that segment and nowhere else.
        let clock = Simulated(Cell::new(0));
        let mut jitter = SplitMix64(7);
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("fixed", vec![1])];
        let budget = 200_000_000;
        let measured = measure(
            &clock,
            &mut SplitMix64(1),
            budget,
            1,
            by_class(&classes, |_| vec![0], |b| b[0]),
            |input: &mut u8| {
                let now = clock.0.get();
                let fourth = now > 105_000_000 && now < 130_000_000;
                let cost = 600 + jitter.below(21) + 3 * u64::from(*input) * u64::from(fourth);
                clock.0.set(now + cost);
            },
        );
        let r = report("simulated", &classes, &measured);
        let ts: Vec<f64> = r.trace.by_segment[0]
            .iter()
            .map(|t| t.expect("a t"))
            .collect();
        assert!(ts[3] > T_FAIL, "{ts:?}");
        for (k, t) in ts.iter().enumerate() {
            if k != 3 {
                assert!(t.abs() < T_FAIL, "{ts:?}");
            }
        }
        // The segments hold every counted sample, each once.
        let segments = segment_moments(&measured.marks, &measured.stats.tests[0]);
        let n: u64 = segments.iter().flatten().map(|m| m.n).sum();
        assert_eq!(n, r.samples);
        for segment in &segments {
            let share = segment.iter().map(|m| m.n).sum::<u64>() as f64 / n as f64;
            assert!((share - 1.0 / 6.0).abs() < 0.02, "{share}");
        }
    }

    #[test]
    #[should_panic(expected = "only the placebo")]
    fn an_empty_class_must_be_the_placebo() {
        let classes: Vec<(&'static str, Vec<u8>)> = vec![("empty", vec![])];
        let mut build = by_class(&classes, |_| vec![0], |b| b[0]);
        build(Some(0), &mut SplitMix64(0));
    }
}
