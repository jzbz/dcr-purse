// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! What an Argon2id derivation leaves behind in the process: no thread and
//! no rayon global pool. One test, alone in its binary, so no other test's
//! threads come and go while it counts.

use purse_snacl::Argon2idParams;
use purse_testutil::hex;

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

/// The names of this process's threads (`/proc/self/task/*/comm`).
#[cfg(target_os = "linux")]
fn thread_names() -> Vec<String> {
    let mut names = Vec::new();
    for task in std::fs::read_dir("/proc/self/task").expect("read /proc/self/task") {
        let comm = task.expect("task").path().join("comm");
        // A thread that ended between the listing and the read has no
        // file left to read.
        if let Ok(name) = std::fs::read_to_string(comm) {
            names.push(name.trim_end().to_owned());
        }
    }
    names
}

/// The thread names once no lane thread is listed and there are `count`
/// threads, or as they are after 10 s. A joined thread has finished, but
/// the kernel can list its task a moment longer, until it is reaped, which
/// on a loaded machine can be milliseconds. A thread that stays (rayon's
/// global pool, a lane thread still running) is still there at the end.
#[cfg(target_os = "linux")]
fn settled(count: usize) -> Vec<String> {
    let start = std::time::Instant::now();
    loop {
        let names = thread_names();
        let done = names.len() == count && !names.iter().any(|n| n == "purse-argon2");
        if done || start.elapsed() >= std::time::Duration::from_secs(10) {
            return names;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn derivations_leave_no_thread_and_no_global_pool() {
    #[cfg(target_os = "linux")]
    let before = thread_names();
    for (lanes, want) in CASES {
        let key = params(lanes).derive_key(b"lanes").expect("derive");
        assert_eq!(hex(key.as_bytes()), want, "{lanes} lanes");
    }
    // Joined, not left to the scope, which waits only until each thread's
    // closure has returned, not for its thread-local destructors.
    std::thread::scope(|s| {
        let running: Vec<_> = CASES
            .iter()
            .map(|&(lanes, want)| {
                s.spawn(move || {
                    let key = params(lanes).derive_key(b"lanes").expect("derive");
                    assert_eq!(hex(key.as_bytes()), want, "{lanes} lanes");
                })
            })
            .collect();
        for r in running {
            r.join().expect("a derivation panicked");
        }
    });
    #[cfg(target_os = "linux")]
    {
        let after = settled(before.len());
        assert!(
            !after.iter().any(|n| n == "purse-argon2"),
            "a lane thread is still running: {after:?}"
        );
        assert_eq!(before.len(), after.len(), "{before:?} then {after:?}");
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .expect("rayon's global pool was already started");
}
