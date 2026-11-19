// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! `purse-bench`: see the library's documentation.

use std::path::PathBuf;
use std::process::ExitCode;

use purse_bench::drive::{DriveOptions, drive};
use purse_bench::harness::{Config, Harness};
use purse_bench::workloads;

const USAGE: &str = "\
usage:
  purse-bench drive --out DIR [--runs N] [--reps N] [--ms F] [--only LIST] [--smoke]
                    [--cpus LIST] [--go-bin PATH] [--go-tags TAGS] [--oracle-dir DIR]
                    [--gate BASELINE]
  purse-bench run [--reps N] [--ms F] [--only LIST] [--smoke]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("drive") => drive_cmd(&args[1..]),
        Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The options common to both commands, and the rest.
struct Parsed {
    cfg: Config,
    rest: Vec<(String, String)>,
}

fn parse(args: &[String], with_values: &[&str]) -> Result<Parsed, String> {
    let mut cfg = Config::default();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--smoke" => cfg.smoke = true,
            "--reps" => {
                cfg.reps = value(a)?
                    .parse()
                    .map_err(|_| "--reps takes a count".to_owned())?;
            }
            "--ms" => {
                cfg.ms = value(a)?
                    .parse()
                    .ok()
                    .filter(|m: &f64| m.is_finite() && *m > 0.0)
                    .ok_or_else(|| "--ms takes a positive number".to_owned())?;
            }
            "--only" => {
                cfg.only = value(a)?
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            other if with_values.contains(&other) => {
                let v = value(other)?;
                rest.push((other.to_owned(), v));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Parsed { cfg, rest })
}

fn run(args: &[String]) -> ExitCode {
    let parsed = match parse(args, &[]) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("purse-bench run: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let stdout = std::io::stdout().lock();
    let mut h = Harness::new(parsed.cfg, std::io::BufWriter::new(stdout));
    h.meta("version", env!("CARGO_PKG_VERSION"));
    h.meta(
        "profile",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    h.meta(
        "target",
        format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
    );
    h.meta(
        "threads",
        std::thread::available_parallelism().map_or(0, usize::from),
    );
    let smoke = h.config().smoke;
    h.meta("mode", if smoke { "smoke" } else { "measure" });
    let (reps, ms) = (h.config().reps, h.config().ms);
    h.meta("reps", reps);
    h.meta("ms", ms);
    workloads::run_all(&mut h);
    drop(h.finish());
    ExitCode::SUCCESS
}

fn drive_cmd(args: &[String]) -> ExitCode {
    let keys = [
        "--out",
        "--runs",
        "--cpus",
        "--go-bin",
        "--go-tags",
        "--oracle-dir",
        "--gate",
    ];
    let opts = parse(args, &keys).and_then(|p| {
        let get = |k: &str| {
            p.rest
                .iter()
                .rev()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
        };
        let out = get("--out").ok_or("--out is required")?;
        let runs = match get("--runs") {
            Some(r) => r.parse().map_err(|_| "--runs takes a count".to_owned())?,
            None => purse_bench::drive::MIN_RUNS,
        };
        let rust_bin = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let oracle_dir = match get("--oracle-dir") {
            Some(d) => PathBuf::from(d),
            None => purse_testutil::oracle_source_dir(),
        };
        Ok(DriveOptions {
            out: PathBuf::from(out),
            runs,
            cfg: p.cfg.clone(),
            cpus: get("--cpus"),
            go_bin: get("--go-bin").map(PathBuf::from),
            go_tags: get("--go-tags"),
            oracle_dir,
            rust_bin,
            gate: get("--gate").map(PathBuf::from),
        })
    });
    let opts = match opts {
        Ok(o) => o,
        Err(e) => {
            eprintln!("purse-bench drive: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match drive(&opts) {
        Ok(outcome) => {
            println!("{}", outcome.text);
            if outcome.gate_passed == Some(false) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("purse-bench drive: {e}");
            ExitCode::FAILURE
        }
    }
}
