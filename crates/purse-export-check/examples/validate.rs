// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Validates logical exports line by line, without holding them in memory,
//! and prints each one's eof digests under every profile:
//!
//! ```sh
//! cargo run --release -p purse-export-check --example validate -- FILE...
//! ```

use std::io::BufReader;
use std::process::ExitCode;

use purse_export_check::{Profile, validate};

fn main() -> ExitCode {
    let profiles = [
        ("maskedclock", Profile::MASKED_CLOCK),
        ("observable-exact", Profile::EXACT.observable()),
        ("observable-masked", Profile::MASKED.observable()),
    ];
    let wanted: Vec<Profile> = profiles.iter().map(|(_, p)| *p).collect();
    let mut status = ExitCode::SUCCESS;
    for path in std::env::args().skip(1) {
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{path}: {e}");
                status = ExitCode::FAILURE;
                continue;
            }
        };
        match validate(BufReader::with_capacity(1 << 20, file), &wanted) {
            Ok(sum) => {
                println!("{path}: exact {}", sum.export.sha256);
                println!("{path}: masked {}", sum.export.msha256);
                for ((name, _), (_, d)) in profiles.iter().zip(&sum.digests) {
                    println!("{path}: {name} {d}");
                }
            }
            Err(e) => {
                eprintln!("{path}: {e}");
                status = ExitCode::FAILURE;
            }
        }
    }
    status
}
