// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The timing loop and the line output of one side, the same as the Go
//! side's (`tools/oracle/cmd/bench/main.go`, `bench`, `benchOnce`,
//! `report`).

use std::fmt::Display;
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

/// How one side runs its workloads.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Timed repetitions per workload.
    pub reps: usize,
    /// Target milliseconds per repetition of a calibrated workload.
    pub ms: f64,
    /// Substrings of the workload names to run; empty runs all.
    pub only: Vec<String>,
    /// One repetition of one operation each, at the reduced sizes: a check
    /// that the workloads run and agree, not a measurement.
    pub smoke: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            reps: 5,
            ms: 200.0,
            only: Vec::new(),
            smoke: false,
        }
    }
}

/// Runs workloads and writes their lines to `out`.
pub struct Harness<W: Write> {
    cfg: Config,
    out: W,
    sink: u64,
}

impl<W: Write> Harness<W> {
    /// A harness writing to `out`. Smoke mode forces one repetition.
    pub fn new(mut cfg: Config, out: W) -> Self {
        if cfg.smoke || cfg.reps == 0 {
            cfg.reps = 1;
        }
        Harness { cfg, out, sink: 0 }
    }

    /// The configuration in force.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Whether the workload `name` runs.
    pub fn selected(&self, name: &str) -> bool {
        self.cfg.only.is_empty() || self.cfg.only.iter().any(|s| name.contains(s.as_str()))
    }

    /// Whether any of the workloads `names` runs: a group whose workloads
    /// are all left out skips its setup and its checks.
    pub fn any_selected(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.selected(n))
    }

    /// Whether a selected workload shares the group of `name` (the part
    /// before its first dot), so a run limited with `only` still prints the
    /// checks of its group.
    fn group_selected(&self, name: &str) -> bool {
        let group = name.split('.').next().unwrap_or(name);
        self.cfg
            .only
            .iter()
            .any(|s| s == group || s.strip_prefix(group).is_some_and(|r| r.starts_with('.')))
    }

    fn line(&mut self, kind: &str, name: &str, value: impl Display) {
        writeln!(self.out, "{kind}\trust\t{name}\t{value}").expect("write the harness output");
    }

    /// A `META` line.
    pub fn meta(&mut self, key: &str, value: impl Display) {
        self.line("META", key, value);
    }

    /// A `CHECK` line: an output the Go side must give too.
    pub fn check(&mut self, name: &str, value: impl Display) {
        if self.selected(name) || self.group_selected(name) {
            self.line("CHECK", name, value);
        }
    }

    /// A `SKIP` line.
    pub fn skip(&mut self, name: &str, why: &str) {
        if self.selected(name) {
            self.line("SKIP", name, why);
        }
    }

    /// Times `f(n)`, which runs `n` operations and returns a value kept
    /// alive: doubles `n` until one call takes 25 ms (warm-up and
    /// calibration), then times `reps` calls of the size that takes about
    /// `ms`.
    pub fn bench(&mut self, name: &str, mut f: impl FnMut(u64) -> u64) {
        if !self.selected(name) {
            return;
        }
        let mut iters = 1u64;
        if self.cfg.smoke {
            self.sink ^= black_box(f(1));
        } else {
            let mut n = 1u64;
            let mut t;
            loop {
                let start = Instant::now();
                self.sink ^= black_box(f(n));
                t = start.elapsed().as_nanos() as f64;
                if t >= 25e6 || n >= 1 << 40 {
                    break;
                }
                n *= 2;
            }
            iters = ((self.cfg.ms * 1e6) / (t / n as f64)).max(1.0) as u64;
        }
        let mut v = Vec::with_capacity(self.cfg.reps);
        for _ in 0..self.cfg.reps {
            let start = Instant::now();
            self.sink ^= black_box(f(iters));
            v.push(start.elapsed().as_nanos() as f64 / iters as f64);
        }
        self.report(name, &v, iters);
    }

    /// Times whole operations too slow to calibrate: one warm-up call, then
    /// `reps` timed calls of one operation each.
    pub fn bench_once(&mut self, name: &str, mut f: impl FnMut() -> u64) {
        if !self.selected(name) {
            return;
        }
        self.sink ^= black_box(f());
        let mut v = Vec::with_capacity(self.cfg.reps);
        for _ in 0..self.cfg.reps {
            let start = Instant::now();
            self.sink ^= black_box(f());
            v.push(start.elapsed().as_nanos() as f64);
        }
        self.report(name, &v, 1);
    }

    fn report(&mut self, name: &str, v: &[f64], iters: u64) {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        let all: Vec<String> = v.iter().map(|x| format!("{x:.1}")).collect();
        writeln!(
            self.out,
            "RESULT\trust\t{name}\t{:.1}\t{:.1}\t{:.1}\t{iters}\t{}\t{}",
            median(&s),
            s.first().copied().unwrap_or(0.0),
            s.last().copied().unwrap_or(0.0),
            v.len(),
            all.join(",")
        )
        .expect("write the harness output");
        self.out.flush().expect("flush the harness output");
    }

    /// Ends the run: writes the sink's low bit, so no workload's result can
    /// be optimised away, and returns the writer.
    pub fn finish(mut self) -> W {
        let bit = self.sink & 1;
        self.meta("sink", bit);
        self.out
    }
}

/// The median of an ascending slice (the mean of the middle two for an
/// even length; 0 for an empty one).
pub fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => sorted[n / 2],
        _ => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
    }
}
