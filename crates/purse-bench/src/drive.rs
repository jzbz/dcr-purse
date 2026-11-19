// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The driver: builds the Go side, runs the two sides alternately in one
//! session, keeps every process's raw output, and writes the table.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::harness::Config;
use crate::report::{self, RunOutput, Session, Side};

/// The ledger's least number of runs per side (ADR-0014, section 4).
pub const MIN_RUNS: usize = 3;

/// What a session does.
#[derive(Clone, Debug)]
pub struct DriveOptions {
    /// The directory the raw output, the session notes and the table are
    /// written to. Created if missing; it must be empty.
    pub out: PathBuf,
    /// Processes per side, alternating: Rust then Go in odd runs, Go then
    /// Rust in even ones.
    pub runs: usize,
    /// What each process runs.
    pub cfg: Config,
    /// A CPU list for `taskset -c`, which pins every measured process.
    pub cpus: Option<String>,
    /// A prebuilt Go side, instead of building `tools/oracle/cmd/bench`.
    pub go_bin: Option<PathBuf>,
    /// Build tags for the Go side (`flint` adds the mixing solver).
    pub go_tags: Option<String>,
    /// The `tools/oracle` directory to build from.
    pub oracle_dir: PathBuf,
    /// The Rust side: this program.
    pub rust_bin: PathBuf,
    /// A baseline of ratios to gate on (ADR-0014's CI gate).
    pub gate: Option<PathBuf>,
}

/// What a session produced.
#[derive(Debug)]
pub struct Outcome {
    /// The session, checked and summarised.
    pub session: Session,
    /// The table and the notes, as written to `table.md`.
    pub text: String,
    /// The gate's verdict, when a baseline was given.
    pub gate_passed: Option<bool>,
}

/// Runs a session.
///
/// # Errors
///
/// A refused configuration (fewer than [`MIN_RUNS`] runs outside smoke
/// mode, a measurement from an unoptimised build, an output directory that
/// is not empty), a build or process that fails, or output the two sides do
/// not agree on.
pub fn drive(opts: &DriveOptions) -> Result<Outcome, String> {
    if !opts.cfg.smoke && opts.runs < MIN_RUNS {
        return Err(format!(
            "the ledger's rules need at least {MIN_RUNS} runs per side; --smoke runs fewer"
        ));
    }
    if opts.runs == 0 {
        return Err("--runs must be at least 1".to_owned());
    }
    if !opts.cfg.smoke && cfg!(debug_assertions) {
        return Err(
            "this is an unoptimised build: measure with `cargo run --release -p purse-bench`"
                .to_owned(),
        );
    }
    std::fs::create_dir_all(&opts.out)
        .map_err(|e| format!("create {}: {e}", opts.out.display()))?;
    if std::fs::read_dir(&opts.out)
        .map_err(|e| format!("read {}: {e}", opts.out.display()))?
        .next()
        .is_some()
    {
        return Err(format!(
            "{} is not empty: each session writes to a directory of its own",
            opts.out.display()
        ));
    }

    let workspace = opts.oracle_dir.join("../..");
    let go_bin = match &opts.go_bin {
        Some(p) => p.clone(),
        None => build_go(opts)?,
    };

    let mut notes = String::new();
    let _ = writeln!(notes, "started (UTC): {}", utc_now());
    let _ = writeln!(
        notes,
        "mode: {}; runs per side: {}; reps: {}; ms per rep: {}; only: {}",
        if opts.cfg.smoke { "smoke" } else { "measure" },
        opts.runs,
        if opts.cfg.smoke { 1 } else { opts.cfg.reps },
        opts.cfg.ms,
        if opts.cfg.only.is_empty() {
            "all".to_owned()
        } else {
            opts.cfg.only.join(",")
        }
    );
    let _ = writeln!(
        notes,
        "pinned: {}",
        opts.cpus
            .as_deref()
            .map_or_else(|| "no".to_owned(), |c| format!("taskset -c {c}"))
    );
    let _ = writeln!(notes, "cpu: {}", cpu_model());
    let _ = writeln!(
        notes,
        "logical cpus: {}",
        std::thread::available_parallelism().map_or(0, usize::from)
    );
    let _ = writeln!(
        notes,
        "os: {} {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    );
    let _ = writeln!(
        notes,
        "rustc: {}",
        command_line("rustc", &["-V"], &workspace)
    );
    let _ = writeln!(
        notes,
        "go: {}",
        command_line("go", &["version"], &workspace)
    );
    let _ = writeln!(
        notes,
        "go side: {}",
        if opts.go_bin.is_some() {
            "prebuilt".to_owned()
        } else {
            format!(
                "built from tools/oracle/cmd/bench, tags: {}",
                opts.go_tags.as_deref().unwrap_or("none")
            )
        }
    );
    let head = command_line("git", &["rev-parse", "HEAD"], &workspace);
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&workspace)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);
    let _ = writeln!(
        notes,
        "repository: {head} with {dirty} uncommitted or untracked paths"
    );
    let _ = writeln!(notes, "load average at start: {}", loadavg());

    let mut runs: Vec<(Side, RunOutput)> = Vec::new();
    for run in 1..=opts.runs {
        let order = if run % 2 == 1 {
            [Side::Rust, Side::Go]
        } else {
            [Side::Go, Side::Rust]
        };
        for side in order {
            let output = run_one(opts, &go_bin, side, run)?;
            runs.push((side, output));
        }
    }
    let _ = writeln!(notes, "load average at end: {}", loadavg());
    let _ = writeln!(notes, "finished (UTC): {}", utc_now());

    let session = report::summarize(&runs)?;
    let mut text = String::new();
    text.push_str("# purse-bench session\n\n");
    for line in notes.lines() {
        let _ = writeln!(text, "- {line}");
    }
    let _ = writeln!(
        text,
        "- checks: {} outputs equal in every run of both sides\n",
        session.checks
    );
    text.push_str(
        "Times are per operation: the median over runs of each run's median \
         repetition, with the range of the runs' medians. Rust/Go is the ratio \
         of the medians (below 1: Rust is faster).\n\n",
    );
    text.push_str(&session.table(opts.cfg.smoke));
    let misses = session.misses();
    if !opts.cfg.smoke && !misses.is_empty() {
        let _ = writeln!(text, "\nPhase 1 criteria missed: {}", misses.join(", "));
    }

    let mut gate_passed = None;
    if let Some(path) = &opts.gate {
        let baseline = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))
            .and_then(|t| report::parse_baseline(&t))?;
        let (gate_text, pass) = report::gate(&session, &baseline);
        let _ = writeln!(
            text,
            "\nCI gate ({}x the baseline): {}\n\n```text\n{gate_text}```",
            report::GATE_FACTOR,
            if pass { "passed" } else { "FAILED" }
        );
        gate_passed = Some(pass);
    }
    let table_path = opts.out.join("table.md");
    std::fs::write(&table_path, &text)
        .map_err(|e| format!("write {}: {e}", table_path.display()))?;
    Ok(Outcome {
        session,
        text,
        gate_passed,
    })
}

fn build_go(opts: &DriveOptions) -> Result<PathBuf, String> {
    // Absolute: `go build` runs in the oracle directory, so a relative `-o`
    // would land under `tools/oracle` while the runs looked for it under
    // the current directory.
    let bin = std::path::absolute(
        opts.out
            .join(format!("purse-bench-go{}", std::env::consts::EXE_SUFFIX)),
    )
    .map_err(|e| format!("resolve {}: {e}", opts.out.display()))?;
    let mut cmd = Command::new("go");
    cmd.arg("build").arg("-trimpath");
    if let Some(tags) = &opts.go_tags {
        cmd.arg("-tags").arg(tags);
    }
    cmd.arg("-o")
        .arg(&bin)
        .arg("./cmd/bench")
        .current_dir(&opts.oracle_dir);
    eprintln!("purse-bench: building the Go side");
    let out = cmd.output().map_err(|e| format!("run go build: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "go build ./cmd/bench failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(bin)
}

fn run_one(
    opts: &DriveOptions,
    go_bin: &Path,
    side: Side,
    run: usize,
) -> Result<RunOutput, String> {
    let (bin, args): (&Path, Vec<String>) = match side {
        Side::Rust => {
            let mut a = vec![
                "run".to_owned(),
                "--reps".to_owned(),
                opts.cfg.reps.to_string(),
                "--ms".to_owned(),
                opts.cfg.ms.to_string(),
            ];
            if !opts.cfg.only.is_empty() {
                a.extend(["--only".to_owned(), opts.cfg.only.join(",")]);
            }
            if opts.cfg.smoke {
                a.push("--smoke".to_owned());
            }
            (&opts.rust_bin, a)
        }
        Side::Go => {
            let mut a = vec![
                "-reps".to_owned(),
                opts.cfg.reps.to_string(),
                "-ms".to_owned(),
                opts.cfg.ms.to_string(),
            ];
            if !opts.cfg.only.is_empty() {
                a.extend(["-only".to_owned(), opts.cfg.only.join(",")]);
            }
            if opts.cfg.smoke {
                a.push("-smoke".to_owned());
            }
            (go_bin, a)
        }
    };
    let mut cmd = match &opts.cpus {
        Some(cpus) => {
            let mut c = Command::new("taskset");
            c.arg("-c").arg(cpus).arg(bin);
            c
        }
        None => Command::new(bin),
    };
    cmd.args(&args);
    eprintln!("purse-bench: run {run} of {}: {}", opts.runs, side.as_str());
    let mut text = format!(
        "# side {} run {run} of {}\n# start (UTC) {} load {}\n",
        side.as_str(),
        opts.runs,
        utc_now(),
        loadavg()
    );
    let start = Instant::now();
    let out = cmd
        .output()
        .map_err(|e| format!("run the {} side: {e}", side.as_str()))?;
    let wall = start.elapsed().as_secs_f64();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    let _ = writeln!(
        text,
        "# end (UTC) {} load {} wall {wall:.1} s, {}",
        utc_now(),
        loadavg(),
        out.status
    );
    let stem = format!("run-{run}-{}", side.as_str());
    let path = opts.out.join(format!("{stem}.txt"));
    std::fs::write(&path, &text).map_err(|e| format!("write {}: {e}", path.display()))?;
    if !out.stderr.is_empty() {
        let err_path = opts.out.join(format!("{stem}.err"));
        std::fs::write(&err_path, &out.stderr)
            .map_err(|e| format!("write {}: {e}", err_path.display()))?;
    }
    if !out.status.success() {
        return Err(format!(
            "the {} side failed in run {run} ({}):\n{}",
            side.as_str(),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    report::parse(&text, side).map_err(|e| format!("{}: {e}", path.display()))
}

/// The first line a command prints, or a note that it could not run.
fn command_line(program: &str, args: &[&str], dir: &Path) -> String {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("({program} unavailable)"))
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|m| m.trim().to_owned())
        })
        .unwrap_or_else(|| "not recorded".to_owned())
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "n/a".to_owned())
}

/// The current time as `YYYY-MM-DD HH:MM:SS`, UTC.
pub fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc(secs)
}

/// A Unix time as `YYYY-MM-DD HH:MM:SS`, UTC (the civil-from-days
/// conversion of the proleptic Gregorian calendar).
pub fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::utc;

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc(1_795_000_000), "2026-11-18 11:06:40");
    }
}
