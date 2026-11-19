// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Reading the two sides' output, cross-checking it, and the session table:
//! per workload and side, the median and range of the runs' medians, the
//! ratio of Rust's time to Go's, and the verdict against ADR-0014's Phase 1
//! row; and the CI gate's comparison with a baseline.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::harness::median;

/// Which implementation a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    /// dcr-purse.
    Rust,
    /// The Go reference.
    Go,
}

impl Side {
    /// The name the output lines carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Rust => "rust",
            Side::Go => "go",
        }
    }

    fn parse(s: &str) -> Option<Side> {
        match s {
            "rust" => Some(Side::Rust),
            "go" => Some(Side::Go),
            _ => None,
        }
    }
}

/// What one process printed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOutput {
    /// `META` lines, in order.
    pub meta: Vec<(String, String)>,
    /// `CHECK` lines by name.
    pub checks: BTreeMap<String, String>,
    /// The median time per operation, in nanoseconds, of each workload, in
    /// the order printed.
    pub results: Vec<(String, f64)>,
    /// Workloads skipped, with the reason.
    pub skips: BTreeMap<String, String>,
}

/// Parses one process's output. Lines starting with `#` are the driver's
/// own notes and are ignored; any other line that is not one of the four
/// kinds, or that names the other side, is an error.
pub fn parse(text: &str, side: Side) -> Result<RunOutput, String> {
    let mut out = RunOutput::default();
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("line {}: {line:?}", n + 1);
        if f.len() < 4 || Side::parse(f[1]) != Some(side) {
            return Err(bad());
        }
        let (name, value) = (f[2].to_owned(), f[3].to_owned());
        match f[0] {
            "META" => out.meta.push((name, value)),
            "CHECK" => {
                if out.checks.insert(name, value).is_some() {
                    return Err(format!("{}: CHECK repeated", bad()));
                }
            }
            "SKIP" => {
                out.skips.insert(name, value);
            }
            "RESULT" if f.len() == 9 => {
                let med: f64 = value.parse().map_err(|_| bad())?;
                if !med.is_finite() || med < 0.0 {
                    return Err(bad());
                }
                out.results.push((name, med));
            }
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// A criterion of ADR-0014's exit table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Criterion {
    /// No slower than Go: Rust's time at most Go's.
    NoSlower,
    /// A secret path: Rust's time at most this multiple of Go's.
    AtMost(f64),
    /// At least this many times faster than Go.
    Faster(f64),
}

impl Criterion {
    /// Whether a ratio (Rust's time over Go's) meets the criterion.
    pub fn met(self, ratio: f64) -> bool {
        match self {
            Criterion::NoSlower => ratio <= 1.0,
            Criterion::AtMost(m) => ratio <= m,
            Criterion::Faster(n) => ratio > 0.0 && 1.0 / ratio >= n,
        }
    }

    fn describe(self) -> String {
        match self {
            Criterion::NoSlower => "≤ 1.00".to_owned(),
            Criterion::AtMost(m) => format!("≤ {m:.2}"),
            Criterion::Faster(n) => format!("≥ {n}x faster"),
        }
    }
}

/// The Phase 1 row of ADR-0014's exit table: the clause a workload is held
/// to, if any.
pub fn phase1_criterion(name: &str) -> Option<(&'static str, Criterion)> {
    Some(match name {
        "hd.pub_child" => ("public derivation", Criterion::NoSlower),
        "hd.address" => ("address", Criterion::NoSlower),
        "hd.script" => ("script", Criterion::NoSlower),
        "crypto.hmac_sha512" => ("HMAC", Criterion::NoSlower),
        "crypto.hash160" => ("hash160", Criterion::NoSlower),
        "crypto.base58check_encode" | "crypto.base58check_decode" => {
            ("base58", Criterion::NoSlower)
        }
        "hd.master" => ("secret path: master", Criterion::AtMost(1.15)),
        "hd.hardened_child" => ("secret path: hardened child", Criterion::AtMost(1.15)),
        "hd.priv_child" => ("secret path: private child", Criterion::AtMost(1.15)),
        "hd.addr20k" => ("20,000 addresses", Criterion::Faster(1.5)),
        _ => return None,
    })
}

/// One side's figures for one workload over the session's runs.
#[derive(Clone, Debug, PartialEq)]
pub struct SideSummary {
    /// Each run's median, in run order.
    pub runs: Vec<f64>,
    /// The median of `runs`.
    pub median: f64,
    /// The least of `runs`.
    pub min: f64,
    /// The greatest of `runs`.
    pub max: f64,
}

impl SideSummary {
    fn of(runs: Vec<f64>) -> Option<SideSummary> {
        let mut s = runs.clone();
        s.sort_by(f64::total_cmp);
        Some(SideSummary {
            median: median(&s),
            min: *s.first()?,
            max: *s.last()?,
            runs,
        })
    }
}

/// One row of the session table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The workload.
    pub name: String,
    /// Go's figures, or why there are none.
    pub go: Result<SideSummary, String>,
    /// Rust's figures, or why there are none.
    pub rust: Result<SideSummary, String>,
}

impl Row {
    /// Rust's median time over Go's, when both sides ran.
    pub fn ratio(&self) -> Option<f64> {
        match (&self.go, &self.rust) {
            (Ok(g), Ok(r)) if g.median > 0.0 => Some(r.median / g.median),
            _ => None,
        }
    }

    /// The criterion and whether it was met, for a held workload.
    pub fn verdict(&self) -> Option<(&'static str, Criterion, Option<bool>)> {
        let (clause, c) = phase1_criterion(&self.name)?;
        Some((clause, c, self.ratio().map(|r| c.met(r))))
    }
}

/// The session: every run's output, checked and summarised.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The rows, in the order the workloads ran.
    pub rows: Vec<Row>,
    /// The checks both sides agreed on.
    pub checks: usize,
}

/// Cross-checks the runs and summarises them. Every `CHECK` must be
/// printed with the same value by every run of both sides; a mismatch, or a
/// check one run lacks, is an error, since the sides would then not have
/// run the same workloads.
pub fn summarize(runs: &[(Side, RunOutput)]) -> Result<Session, String> {
    let Some((_, first)) = runs.first() else {
        return Err("no runs".to_owned());
    };
    for (i, (side, run)) in runs.iter().enumerate() {
        if run.checks != first.checks {
            let mut diff = Vec::new();
            for (k, v) in &first.checks {
                match run.checks.get(k) {
                    Some(w) if w == v => {}
                    Some(w) => diff.push(format!("{k}: {v} (run 1) against {w}")),
                    None => diff.push(format!("{k}: missing")),
                }
            }
            for k in run.checks.keys() {
                if !first.checks.contains_key(k) {
                    diff.push(format!("{k}: only in this run"));
                }
            }
            return Err(format!(
                "run {} ({}) does not agree with run 1: {}",
                i + 1,
                side.as_str(),
                diff.join("; ")
            ));
        }
    }
    let mut order: Vec<String> = Vec::new();
    let mut by: BTreeMap<(String, Side), Vec<f64>> = BTreeMap::new();
    let mut skipped: BTreeMap<(String, Side), String> = BTreeMap::new();
    for (side, run) in runs {
        for (name, med) in &run.results {
            if !order.contains(name) {
                order.push(name.clone());
            }
            by.entry((name.clone(), *side)).or_default().push(*med);
        }
        for (name, why) in &run.skips {
            if !order.contains(name) {
                order.push(name.clone());
            }
            skipped.insert((name.clone(), *side), why.clone());
        }
    }
    let side = |name: &str, s: Side| -> Result<SideSummary, String> {
        let key = (name.to_owned(), s);
        if let Some(why) = skipped.get(&key) {
            return Err(format!("skipped: {why}"));
        }
        by.get(&key)
            .cloned()
            .and_then(SideSummary::of)
            .ok_or_else(|| "not run".to_owned())
    };
    let rows = order
        .iter()
        .map(|name| Row {
            name: name.clone(),
            go: side(name, Side::Go),
            rust: side(name, Side::Rust),
        })
        .collect();
    Ok(Session {
        rows,
        checks: first.checks.len(),
    })
}

/// A time per operation in its natural unit.
pub fn fmt_ns(ns: f64) -> String {
    if ns >= 1e9 {
        format!("{:.3} s", ns / 1e9)
    } else if ns >= 1e6 {
        format!("{:.2} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.2} µs", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

fn fmt_side(s: &Result<SideSummary, String>) -> String {
    match s {
        Ok(s) => format!("{} ({}–{})", fmt_ns(s.median), fmt_ns(s.min), fmt_ns(s.max)),
        Err(why) => why.clone(),
    }
}

impl Session {
    /// The table as Markdown: per workload, Go's and Rust's median time per
    /// operation with the range of the runs' medians, Rust's time over Go's
    /// and, where ADR-0014's Phase 1 row holds the workload, its criterion
    /// and verdict.
    ///
    /// In smoke mode the times mean nothing, and no verdict is given.
    pub fn table(&self, smoke: bool) -> String {
        let mut t = String::new();
        t.push_str(
            "| workload | Go median (range) | Rust median (range) | Rust/Go | Phase 1 criterion | verdict |\n",
        );
        t.push_str("|---|---|---|---|---|---|\n");
        for row in &self.rows {
            let ratio = row
                .ratio()
                .map_or_else(|| "—".to_owned(), |r| format!("{r:.2}"));
            let (crit, verdict) = match row.verdict() {
                None => ("—".to_owned(), "—".to_owned()),
                Some((clause, c, _)) if smoke => {
                    (format!("{clause} {}", c.describe()), "smoke".to_owned())
                }
                Some((clause, c, met)) => (
                    format!("{clause} {}", c.describe()),
                    match met {
                        Some(true) => "met".to_owned(),
                        Some(false) => "**missed**".to_owned(),
                        None => "not measured".to_owned(),
                    },
                ),
            };
            let _ = writeln!(
                t,
                "| {} | {} | {} | {ratio} | {crit} | {verdict} |",
                row.name,
                fmt_side(&row.go),
                fmt_side(&row.rust)
            );
        }
        t
    }

    /// The workloads held by a Phase 1 criterion that missed it, or could
    /// not be measured.
    pub fn misses(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|r| matches!(r.verdict(), Some((_, _, met)) if met != Some(true)))
            .map(|r| r.name.as_str())
            .collect()
    }
}

/// The CI gate's baseline: workload names and ratios (Rust's time over
/// Go's), one per line, tab-separated, `#` comments.
pub fn parse_baseline(text: &str) -> Result<Vec<(String, f64)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut f = line.split_whitespace();
        let (Some(name), Some(ratio), None) = (f.next(), f.next(), f.next()) else {
            return Err(format!("baseline line {}: {line:?}", n + 1));
        };
        let ratio: f64 = ratio
            .parse()
            .map_err(|_| format!("baseline line {}: {line:?}", n + 1))?;
        if !(ratio.is_finite() && ratio > 0.0) {
            return Err(format!("baseline line {}: {line:?}", n + 1));
        }
        out.push((name.to_owned(), ratio));
    }
    Ok(out)
}

/// The factor of ADR-0014's CI gate: a ratio more than twice its baseline
/// is a regression.
pub const GATE_FACTOR: f64 = 2.0;

/// Compares a session with the baseline: one line per baselined workload,
/// and whether every one is within [`GATE_FACTOR`] of its baseline. A
/// baselined workload the session did not measure on both sides fails.
pub fn gate(session: &Session, baseline: &[(String, f64)]) -> (String, bool) {
    let mut report = String::new();
    let mut pass = true;
    for (name, base) in baseline {
        let limit = base * GATE_FACTOR;
        let ratio = session
            .rows
            .iter()
            .find(|r| &r.name == name)
            .and_then(Row::ratio);
        let line = match ratio {
            Some(r) if r <= limit => {
                format!("ok   {name}: {r:.3} (baseline {base:.3}, limit {limit:.3})")
            }
            Some(r) => {
                pass = false;
                format!(
                    "FAIL {name}: {r:.3} exceeds {limit:.3} ({GATE_FACTOR}x the baseline {base:.3})"
                )
            }
            None => {
                pass = false;
                format!("FAIL {name}: not measured on both sides")
            }
        };
        report.push_str(&line);
        report.push('\n');
    }
    (report, pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(side: Side, text: &str) -> (Side, RunOutput) {
        (side, parse(text, side).expect("parse"))
    }

    const GO1: &str = "META\tgo\tversion\tgo1\nCHECK\tgo\thd.x\tabc\n\
        RESULT\tgo\thd.pub_child\t100.0\t90.0\t110.0\t10\t3\t90.0,100.0,110.0\n\
        RESULT\tgo\thd.master\t100.0\t90.0\t110.0\t10\t3\t90.0,100.0,110.0\n\
        SKIP\tgo\tmix.roots_20\tno flint\n";
    const RS1: &str = "# a note\nCHECK\trust\thd.x\tabc\n\
        RESULT\trust\thd.pub_child\t50.0\t40.0\t60.0\t10\t3\t40.0,50.0,60.0\n\
        RESULT\trust\thd.master\t130.0\t120.0\t140.0\t10\t3\t120.0,130.0,140.0\n\
        RESULT\trust\tmix.roots_20\t7.0\t7.0\t7.0\t1\t1\t7.0\n";

    #[test]
    fn parse_rejects_the_other_side_and_junk() {
        assert!(parse(GO1, Side::Rust).is_err());
        assert!(parse("RESULT\tgo\tx\tnan\t1\t1\t1\t1\t1\n", Side::Go).is_err());
        assert!(parse("hello\n", Side::Go).is_err());
        assert!(parse("CHECK\tgo\ta\t1\nCHECK\tgo\ta\t1\n", Side::Go).is_err());
    }

    #[test]
    fn summary_and_verdicts() {
        let s = summarize(&[
            run(Side::Rust, RS1),
            run(Side::Go, GO1),
            run(Side::Go, &GO1.replace("\t100.0\t90", "\t120.0\t90")),
            run(Side::Rust, RS1),
        ])
        .expect("summary");
        assert_eq!(s.checks, 1);
        let pubc = &s.rows[0];
        assert_eq!(pubc.name, "hd.pub_child");
        let go = pubc.go.as_ref().expect("go");
        assert_eq!((go.median, go.min, go.max), (110.0, 100.0, 120.0));
        assert_eq!(pubc.ratio(), Some(50.0 / 110.0));
        assert_eq!(pubc.verdict().map(|v| v.2), Some(Some(true)));
        let master = &s.rows[1];
        assert_eq!(master.verdict().map(|v| v.2), Some(Some(false)));
        assert_eq!(s.misses(), vec!["hd.master"]);
        let mix = &s.rows[2];
        assert!(mix.go.is_err() && mix.ratio().is_none());
        let table = s.table(false);
        assert!(table.contains("| hd.master |"), "{table}");
        assert!(table.contains("**missed**"), "{table}");
        assert!(!s.table(true).contains("missed"));
    }

    #[test]
    fn a_disagreeing_check_is_refused() {
        let bad = GO1.replace("abc", "abd");
        let err = summarize(&[run(Side::Rust, RS1), run(Side::Go, &bad)]).unwrap_err();
        assert!(err.contains("hd.x"), "{err}");
        let missing = GO1.replace("CHECK\tgo\thd.x\tabc\n", "");
        assert!(summarize(&[run(Side::Rust, RS1), run(Side::Go, &missing)]).is_err());
    }

    #[test]
    fn criteria() {
        assert!(Criterion::NoSlower.met(1.0) && !Criterion::NoSlower.met(1.01));
        assert!(Criterion::AtMost(1.15).met(1.15) && !Criterion::AtMost(1.15).met(1.2));
        assert!(Criterion::Faster(1.5).met(0.6) && !Criterion::Faster(1.5).met(0.7));
        assert!(phase1_criterion("snacl.derive").is_none());
    }

    #[test]
    fn the_gate() {
        let s = summarize(&[run(Side::Rust, RS1), run(Side::Go, GO1)]).expect("summary");
        let base = parse_baseline("# c\nhd.pub_child\t0.3\nhd.master 1.0 # x\n").expect("baseline");
        let (report, pass) = gate(&s, &base);
        assert!(pass, "{report}");
        let (report, pass) = gate(&s, &[("hd.pub_child".to_owned(), 0.2)]);
        assert!(!pass && report.contains("FAIL hd.pub_child"), "{report}");
        let (_, pass) = gate(&s, &[("mix.roots_20".to_owned(), 1.0)]);
        assert!(!pass);
        assert!(parse_baseline("a 0\n").is_err());
        assert!(parse_baseline("a 1 2\n").is_err());
    }
}
