// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Argon2id derivations in a process the operating system refuses every
//! new thread: this binary runs itself again under `prlimit --nproc=1`
//! (util-linux), so the kernel answers each thread it asks for with
//! `EAGAIN`. Every key must still be Go's, derived on the calling thread
//! alone. Rayon's global pool cannot start there either: a derivation that
//! reached for it would panic (rayon-core 1.13.0 `src/registry.rs:162-172`),
//! which the release profile turns into an abort.
//!
//! Linux only. The limit does not bind root, so as root the test says so
//! and checks nothing.

#![cfg(target_os = "linux")]

use std::process::Command;

use purse_snacl::Argon2idParams;
use purse_testutil::hex;

/// Set in the environment of the run under the limit.
const CHILD: &str = "PURSE_SNACL_REFUSED_THREADS_CHILD";

/// Salt 00..0f, time 2, 32 KiB per lane, as in `tests/lanes.rs`.
fn params(lanes: u8) -> Argon2idParams {
    Argon2idParams {
        salt: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        time: 2,
        memory: u32::from(lanes).checked_mul(32).expect("fits"),
        threads: lanes,
    }
}

/// Lanes and Go's key, from `tests/lanes.rs`.
const CASES: [(u8, &str); 3] = [
    (
        1,
        "e39fefe43372dd3af936fd0a6bc2cbe1de6899d12163035cc8858a4c08eb7cdd",
    ),
    (
        4,
        "d2f408ed9166f224f0c4b53d21cc4309df2b552b4bfc515b817f6f4ed8503421",
    ),
    (
        255,
        "8d43bdcdb9a2eeef8a7ca5571ea968deaf1ee3c438807324e69257d22ee4f007",
    ),
];

/// Whether this process runs as root (effective user ID 0), which
/// `RLIMIT_NPROC` does not bind.
fn is_root() -> bool {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let uid = status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .expect("a Uid line");
    uid.split_whitespace().nth(1) == Some("0")
}

#[test]
fn derives_when_every_thread_is_refused() {
    if std::env::var_os(CHILD).is_some() {
        // libtest itself runs this test on its main thread when it cannot
        // start one (library/test/src/lib.rs, `ErrorKind::WouldBlock`).
        assert!(
            std::thread::Builder::new().spawn(|| ()).is_err(),
            "the thread limit is not in effect"
        );
        for (lanes, want) in CASES {
            let key = params(lanes).derive_key(b"lanes").expect("derive");
            assert_eq!(hex(key.as_bytes()), want, "{lanes} lanes");
        }
        return;
    }
    if is_root() {
        eprintln!("running as root, which the thread limit does not bind: nothing checked");
        return;
    }
    let exe = std::env::current_exe().expect("current_exe");
    let out = Command::new("prlimit")
        .arg("--nproc=1")
        .arg("--")
        .arg(exe)
        .args([
            "--exact",
            "derives_when_every_thread_is_refused",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .expect("run prlimit (util-linux)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "under the thread limit: {}\n{stdout}\n{stderr}",
        out.status
    );
}
