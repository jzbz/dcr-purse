// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Test support: the Rust half of the operation-script differential
//! (ADR-0002 section 6) and helpers shared by the test files.
//!
//! Each test file that needs it says `mod support;`; not every file uses
//! every item.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the support module"
)]

pub mod compare;
pub mod generator;
pub mod runner;
pub mod txgen;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh, empty directory under cargo's per-test scratch directory.
pub fn scratch(label: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("purse-udb")
        .join(format!("{label}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The directory of this crate's test data.
pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}
