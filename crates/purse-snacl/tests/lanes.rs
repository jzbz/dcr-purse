// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Argon2id keys for nine lane counts from 1 to 255, both ends of the
//! stored thread count's range included, pinned to the keys the Go oracle
//! derived for them
//! (`tools/oracle/cmd/snacl`, `argon2-derive`, `kdf.DeriveKey(.., 32)`) and
//! sent to Go again on every run that has the oracle.
//!
//! The argon2 crate fills the lanes of each slice in parallel through
//! rayon (its `parallel` feature), on a pool each derivation makes of one
//! thread per lane, at most one per CPU: 255 lanes are more than that
//! pool has threads on any machine with fewer CPUs, 33 more than it has
//! on CI's runners or a 32-thread desktop, 3 and 33 do not divide evenly
//! between threads, and time 2 runs the data-dependent pass that
//! references other lanes' blocks. The frozen and differential corpora
//! pin 1 to 4 lanes at the least memory and 1 and 8 at dcrwallet's
//! 256 MiB. Fewer threads than lanes, the calling thread alone included,
//! are pinned by the unit tests in `src/kdf.rs`.

use purse_snacl::Argon2idParams;
use purse_testutil::serde_json::json;
use purse_testutil::{hex, oracle_or_skip};

const PASS: &[u8] = b"lanes";

/// Lanes and Go's key for [`params`]`(lanes)` and [`PASS`].
const CASES: [(u8, &str); 9] = [
    (
        1,
        "e39fefe43372dd3af936fd0a6bc2cbe1de6899d12163035cc8858a4c08eb7cdd",
    ),
    (
        2,
        "f01fe690b21ec7083db3d42d6df8ad58bdf5222138f1280888731a9495a02fba",
    ),
    (
        3,
        "e75506aedce2030c7a980cb610231a9e7c763b3f1038b579f3e88410a45bb08d",
    ),
    (
        4,
        "d2f408ed9166f224f0c4b53d21cc4309df2b552b4bfc515b817f6f4ed8503421",
    ),
    (
        8,
        "24d369f95133b211e12d8fde9b0357c2d05be11ccca79de68e80ef0dd3fa4986",
    ),
    (
        16,
        "753b5ced6743b5ce172394d38c3a7d51af91f87f24dbee6b255776445b722f8a",
    ),
    (
        33,
        "046291bc6a1228d145832e7c791c439741679f9d1a8263318bf7985fe49d7f07",
    ),
    (
        64,
        "a9cc0b1448b39d7d25bdfa1f74dc237b7eade6182e9059213ff87acb86eb12ab",
    ),
    (
        255,
        "8d43bdcdb9a2eeef8a7ca5571ea968deaf1ee3c438807324e69257d22ee4f007",
    ),
];

/// Salt 00..0f, time 2, 32 KiB per lane (8 blocks per segment).
fn params(lanes: u8) -> Argon2idParams {
    Argon2idParams {
        salt: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        time: 2,
        memory: u32::from(lanes).checked_mul(32).expect("fits"),
        threads: lanes,
    }
}

#[test]
fn keys_for_every_lane_count_are_gos() {
    for (lanes, want) in CASES {
        let key = params(lanes).derive_key(PASS).expect("derive");
        assert_eq!(hex(key.as_bytes()), want, "{lanes} lanes");
    }
}

/// Every case at once, each from its own thread, so derivations, each on
/// a pool of its own, compete for the CPUs.
#[test]
fn concurrent_derivations_give_the_same_keys() {
    std::thread::scope(|s| {
        let running: Vec<_> = CASES
            .iter()
            .map(|&(lanes, want)| {
                s.spawn(move || {
                    let key = params(lanes).derive_key(PASS).expect("derive");
                    assert_eq!(hex(key.as_bytes()), want, "{lanes} lanes");
                })
            })
            .collect();
        for r in running {
            r.join().expect("a derivation panicked");
        }
    });
}

#[test]
fn pinned_keys_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    for (lanes, want) in CASES {
        let args = json!({
            "params": hex(&params(lanes).marshal_binary()),
            "pass": hex(PASS),
        });
        let outcome = oracle.call("argon2-derive", args);
        assert_eq!(
            outcome["ok"]["key"].as_str(),
            Some(want),
            "{lanes} lanes: {outcome}"
        );
    }
}
