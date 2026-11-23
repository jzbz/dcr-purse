// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against the Go oracle `tools/oracle/cmd/snacl` (a
//! verbatim copy of dcrwallet's snacl, dcrwallet's kdf, a verbatim copy of
//! udb's seal/unseal, udb's public API), over inputs generated from
//! `PURSE_TEST_SEED`; the replay of every frozen file in `tests/data`
//! against Go; and their regeneration:
//!
//! ```text
//! PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl \
//!     --test oracle -- --ignored regenerate_frozen_vectors
//! ```
//!
//! Regenerating `snacl_vectors.txt`, `generated_db.txt` and
//! `generated-simnet.db.gz` does not reproduce them byte for byte: Go draws
//! salts, nonces and keys from dcrd's `crypto/rand`, seeded from the
//! operating system, which the oracle cannot steer, and later requests are
//! built from earlier answers. So the `frozen_*_against_go` tests send every
//! frozen entry, and the frozen database, back to Go on each run with the
//! oracle, which proves them Go's without regenerating them.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters on small, bounded values"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;

use common::*;
use purse_snacl::{
    Argon2idParams, CryptoKey, DEFAULT_N, DEFAULT_P, DEFAULT_R, NONCE_SIZE, OVERHEAD, SecretKey,
    XCHACHA20POLY1305_OVERHEAD,
};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{DCRWALLET_PARITY_COMMIT, Oracle, SplitMix64, hex, oracle_or_skip};
use sha2::{Digest, Sha256};

fn ask(oracle: &mut Oracle) -> impl FnMut(&str, &Value) -> Value + '_ {
    |cmd, args| oracle.call(cmd, args.clone())
}

#[test]
fn differential_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-snacl differential");
    let entries = build_corpus(&mut rng, &mut ask(&mut oracle));
    par_map(&entries, check_entry);
    let cmds: BTreeSet<&str> = entries.iter().map(|e| e.cmd.as_str()).collect();
    assert_eq!(cmds.len(), 17, "{cmds:?}");
    eprintln!("{} requests agree with Go", entries.len());
}

/// Set in the child this test starts under `taskset` or a CPU quota, which
/// prints `purse_snacl::num_cpu()` and `available_parallelism` and returns.
const NUM_CPU_CHILD: &str = "PURSE_SNACL_NUM_CPU_CHILD";

/// Set to 1 to run `processor_count_is_gos`'s quota leg, which starts a
/// transient scope in the user's service manager; unset, or set to
/// anything else, the leg is skipped with a message. CI's test jobs set it.
#[cfg(target_os = "linux")]
const CPU_QUOTA: &str = "PURSE_TEST_CPU_QUOTA";

/// `systemd-run`'s arguments for a transient scope of the user's service
/// manager with a CPU quota of one processor (`CPUQuota=100%`, cgroup v2
/// `cpu.max` 100000 100000), which needs no privileges where the user
/// manager runs and has the `cpu` controller delegated.
#[cfg(target_os = "linux")]
const QUOTA: [&str; 7] = [
    "--user",
    "--scope",
    "--quiet",
    "--collect",
    "-p",
    "CPUQuota=100%",
    "--",
];

/// Go's `runtime.NumCPU()` from the oracle binary `bin`, started as the
/// last argument of `launcher` (`taskset -c <cpu>`, or `systemd-run` with
/// [`QUOTA`]): one request on stdin, then end of input.
#[cfg(target_os = "linux")]
fn go_num_cpu_under(bin: &std::path::Path, launcher: &[&str]) -> u64 {
    use std::process::{Command, Stdio};
    let (prog, args) = launcher.split_first().expect("a launcher");
    let mut child = Command::new(prog)
        .args(args)
        .arg(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {prog}: {e}"));
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"{\"cmd\":\"info\",\"args\":null}\n")
        .expect("write the request");
    let out = child.wait_with_output().expect("oracle output");
    let line = String::from_utf8(out.stdout).expect("UTF-8");
    let v: Value = serde_json::from_str(line.lines().next().expect("a response")).expect("JSON");
    v["result"]["num_cpu"].as_u64().expect("num_cpu")
}

/// This test binary re-run as the last argument of `launcher`, as a child
/// that prints its counts: `purse_snacl::num_cpu()` and
/// `std::thread::available_parallelism`.
#[cfg(target_os = "linux")]
fn rust_counts_under(launcher: &[&str]) -> (u64, u64) {
    use std::process::Command;
    let (prog, args) = launcher.split_first().expect("a launcher");
    let out = Command::new(prog)
        .args(args)
        .arg("env")
        .arg(format!("{NUM_CPU_CHILD}=1"))
        .arg(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "processor_count_is_gos",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap_or_else(|e| panic!("re-run under {prog}: {e}"));
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |name: &str| -> u64 {
        text.split_whitespace()
            .find_map(|w| w.strip_prefix(name))
            .unwrap_or_else(|| panic!("child printed no {name}: {text}"))
            .parse()
            .expect("a count")
    };
    (field("num_cpu="), field("parallelism="))
}

/// ADR-0012 Rule 9(b), the processor count new Argon2id parameters store
/// (decided by the project owner on 2026-11-22): `purse_snacl::num_cpu` is
/// Go's `runtime.NumCPU`, the CPUs of the process's affinity mask, here on
/// Linux and Android. Both sides are asked in this test's environment and,
/// on Linux, in two more, each leg skipped with a message where its tool
/// is missing:
///
/// - with both processes started on a single CPU of the mask by `taskset`
///   (Go's oracle, and this test binary re-run as a child that prints its
///   count): both count 1;
/// - only with `PURSE_TEST_CPU_QUOTA=1` (otherwise skipped with a message,
///   so that a normal run starts no unit in the user's service manager):
///   with both started in a transient scope of that manager under a CPU
///   quota of one processor (`systemd-run --user`, no privileges needed):
///   Go ignores the quota and counts the mask, and so must `num_cpu`,
///   while `available_parallelism`, the count before the decision,
///   honours it. This is the leg that tells the decided count from the
///   old one; it compares only where the quota takes effect (the child's
///   `available_parallelism` falls below Go's count), and says so
///   otherwise, as on a one-processor mask or where the user manager has
///   no `cpu` controller.
///
/// On other systems the two counts are printed and not compared (see
/// `num_cpu`'s documentation for what differs there).
#[test]
fn processor_count_is_gos() {
    if std::env::var_os(NUM_CPU_CHILD).is_some() {
        println!(
            "num_cpu={} parallelism={}",
            purse_snacl::num_cpu(),
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
        );
        return;
    }
    let Some(bin) = purse_testutil::oracle_bin_or_skip("snacl") else {
        return;
    };
    let mut oracle = Oracle::spawn_at(&bin);
    let go = oracle.call("info", Value::Null)["num_cpu"]
        .as_u64()
        .expect("num_cpu");
    let rust = u64::try_from(purse_snacl::num_cpu()).expect("fits");
    eprintln!("runtime.NumCPU() {go}, purse_snacl::num_cpu() {rust}");
    if !cfg!(any(target_os = "linux", target_os = "android")) {
        return;
    }
    assert_eq!(rust, go, "the processor count Go stores");
    #[cfg(target_os = "linux")]
    {
        use std::process::{Command, Stdio};
        if Command::new("taskset").arg("--version").output().is_err() {
            eprintln!("taskset not installed: the single-CPU leg is skipped");
        } else {
            let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
            let cpu = status
                .lines()
                .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
                .and_then(|l| l.trim().split([',', '-']).next())
                .expect("Cpus_allowed_list")
                .to_owned();
            let pin = ["taskset", "-c", cpu.as_str()];
            let go_pinned = go_num_cpu_under(&bin, &pin);
            let (rust_pinned, _) = rust_counts_under(&pin);
            eprintln!("on CPU {cpu} alone: runtime.NumCPU() {go_pinned}, num_cpu() {rust_pinned}");
            assert_eq!((go_pinned, rust_pinned), (1, 1), "on CPU {cpu} alone");
        }
        if std::env::var_os(CPU_QUOTA).as_deref() != Some(std::ffi::OsStr::new("1")) {
            eprintln!("{CPU_QUOTA} is not set to 1: the quota leg is skipped");
            return;
        }
        let mut quota = vec!["systemd-run"];
        quota.extend(QUOTA);
        let probe = Command::new(quota[0])
            .args(&quota[1..])
            .arg("true")
            .stdin(Stdio::null())
            .output();
        match probe {
            Ok(out) if out.status.success() => {
                let go_quota = go_num_cpu_under(&bin, &quota);
                let (rust_quota, parallelism) = rust_counts_under(&quota);
                eprintln!(
                    "under a quota of one CPU: runtime.NumCPU() {go_quota}, num_cpu() \
                     {rust_quota}, available_parallelism {parallelism}"
                );
                if parallelism >= go_quota {
                    eprintln!(
                        "the quota did not lower available_parallelism ({parallelism}): the \
                         quota leg cannot tell the counts apart and is skipped"
                    );
                } else {
                    assert_eq!(go_quota, go, "Go ignores the quota");
                    assert_eq!(rust_quota, go_quota, "num_cpu ignores the quota, as Go");
                }
            }
            Ok(out) => eprintln!(
                "systemd-run --user failed ({}): the quota leg is skipped: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => eprintln!("systemd-run not available ({e}): the quota leg is skipped"),
        }
    }
}

#[test]
fn generated_database_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let num_cpu = oracle.call("info", Value::Null)["num_cpu"]
        .as_u64()
        .expect("num_cpu");
    let dir = scratch("generated-live");
    let path = dir.join("wallet.db");
    let mut request = make_db_request("simnet");
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("make-db", request.clone());
    let expect = outcome
        .get("ok")
        .unwrap_or_else(|| panic!("make-db failed: {outcome}"));
    let bytes = std::fs::read(&path).expect("read the database");
    let walk = check_generated_db(bytes, &request, expect);
    // kdf.NewArgon2idParams stored the oracle machine's CPU count.
    let threads = Argon2idParams::threads_for_cpus(usize::try_from(num_cpu).expect("fits"));
    assert_eq!(walk.unique_threads, vec![threads, threads]);
}

/// The reverse direction of the Phase 1 exit's blob clause: keys, boxes and
/// account blobs written by this crate with its own randomness (the
/// operating system's, through the plain constructors a wallet uses) open
/// in Go, and Go derives the same keys from the parameters this crate
/// wrote. The scrypt master keys include the costs dcrwallet itself writes
/// (N = 2^18, r = 8, p = 1, and simnet's N = 2, r = 1, p = 1), as well as
/// small ones and snacl's defaults; the Argon2id keys use small memory
/// costs only. Go must refuse each value once tampered with: every box and
/// account blob with a bit flipped (`secret-decrypt`, `crypto-decrypt`,
/// `unseal`, `account-unseal`); scrypt parameters with a bit flipped in the
/// salt or digest, truncated, and with a wrong passphrase (`derive-key`);
/// Argon2id parameters with a bit flipped in the salt (`account-unseal`)
/// and truncated (`argon2-unmarshal`). Bits of the cost fields are not
/// flipped: a flipped high bit of scrypt's N or Argon2id's memory asks Go
/// for terabytes.
#[test]
fn rust_written_blobs_open_in_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let mut rng = SplitMix64::from_env("purse-snacl rust-written");
    let ok = |o: Value, what: &str| -> Value {
        o.get("ok")
            .unwrap_or_else(|| panic!("{what}: Go refused a Rust-written value: {o}"))
            .clone()
    };
    let refused = |o: &Value, what: &str| {
        assert!(
            o.get("err").is_some(),
            "{what}: Go accepted a tampered value: {o}"
        );
    };
    // A copy of `b` with one bit flipped among its first `within` bytes.
    let flip_in = |rng: &mut SplitMix64, b: &[u8], within: usize| {
        let mut t = b.to_vec();
        let at = rng.below(within.min(t.len()) as u64) as usize;
        t[at] ^= 1 << rng.below(8);
        t
    };
    let flip = |rng: &mut SplitMix64, b: &[u8]| flip_in(rng, b, b.len());
    let mut counts = BTreeMap::<&str, usize>::new();

    // Master keys (scrypt), at small costs, at snacl's defaults and at the
    // costs dcrwallet itself writes (`udb.scryptOptionsForNet` at the pin:
    // N = 2, r = 1, p = 1 on simnet, N = 2^18, r = 8, p = 1 on every other
    // network), and what each seals: a crypto key, as a wallet's master
    // keys do, and other plaintexts.
    let costs = [
        (16i64, 1i64, 1i64),
        (1024, 8, 1),
        (64, 2, 3),
        (DEFAULT_N, DEFAULT_R, DEFAULT_P),
        DCRWALLET_SIMNET_SCRYPT,
        DCRWALLET_SCRYPT,
    ];
    for (n, r, p) in costs {
        let pass_len = rng.below(33) as usize;
        let pass = rng.bytes(pass_len);
        let sk = SecretKey::new(&pass, n, r, p).expect("new secret key");
        let params = sk.marshal();
        let go = ok(
            oracle.call(
                "derive-key",
                json!({"params": hex(&params), "pass": hex(&pass)}),
            ),
            "derive-key",
        );
        assert_eq!(
            go["key"].as_str(),
            Some(hex(sk.key().as_bytes()).as_str()),
            "derived key"
        );
        let mut wrong = pass.clone();
        wrong.push(1);
        refused(
            &oracle.call(
                "derive-key",
                json!({"params": hex(&params), "pass": hex(&wrong)}),
            ),
            "derive-key with a wrong passphrase",
        );
        // The salt and the digest (the first 64 bytes; N, r and p follow).
        let bad = flip_in(&mut rng, &params, 64);
        refused(
            &oracle.call(
                "derive-key",
                json!({"params": hex(&bad), "pass": hex(&pass)}),
            ),
            "derive-key with a flipped bit in the salt or digest",
        );
        refused(
            &oracle.call(
                "derive-key",
                json!({"params": hex(&params[..params.len() - 1]), "pass": hex(&pass)}),
            ),
            "derive-key with truncated parameters",
        );
        *counts.entry("master key parameters").or_default() += 1;

        let crypto = CryptoKey::generate().expect("crypto key");
        let other_len = 1 + rng.below(200) as usize;
        let plaintexts = [crypto.as_bytes().to_vec(), Vec::new(), rng.bytes(other_len)];
        for pt in &plaintexts {
            let sealed = sk.encrypt(pt).expect("secret encrypt");
            let key = hex(sk.key().as_bytes());
            let go = ok(
                oracle.call(
                    "secret-decrypt",
                    json!({"key": key, "sealed": hex(&sealed)}),
                ),
                "secret-decrypt",
            );
            assert_eq!(go["plaintext"].as_str(), Some(hex(pt).as_str()));
            let bad = flip(&mut rng, &sealed);
            refused(
                &oracle.call("secret-decrypt", json!({"key": key, "sealed": hex(&bad)})),
                "secret-decrypt",
            );
            *counts.entry("master key boxes").or_default() += 1;
        }

        // What a crypto key seals: extended keys, scripts, anything.
        for len in [0usize, 32, 111, rng.below(300) as usize] {
            let pt = rng.bytes(len);
            let sealed = crypto.encrypt(&pt).expect("crypto encrypt");
            let key = hex(crypto.as_bytes());
            let go = ok(
                oracle.call(
                    "crypto-decrypt",
                    json!({"key": key, "sealed": hex(&sealed)}),
                ),
                "crypto-decrypt",
            );
            assert_eq!(go["plaintext"].as_str(), Some(hex(&pt).as_str()));
            let bad = flip(&mut rng, &sealed);
            refused(
                &oracle.call("crypto-decrypt", json!({"key": key, "sealed": hex(&bad)})),
                "crypto-decrypt",
            );
            *counts.entry("crypto key boxes").or_default() += 1;
        }
    }

    // Per-account passphrases: parameters from the plain constructor (an
    // operating-system salt and this machine's thread count), read back by
    // Go; then, at small memory costs (dcrwallet's 256 MiB is left out of
    // the test suite), the derived key and the sealed blobs.
    let fresh = Argon2idParams::new().expect("argon2 params");
    let go = ok(
        oracle.call(
            "argon2-unmarshal",
            json!({"data": hex(&fresh.marshal_binary())}),
        ),
        "argon2-unmarshal",
    );
    assert_eq!(go["salt"].as_str(), Some(hex(&fresh.salt).as_str()));
    assert_eq!(go["time"], json!(fresh.time));
    assert_eq!(go["memory"], json!(fresh.memory));
    assert_eq!(go["threads"], json!(fresh.threads));
    let truncated = fresh.marshal_binary();
    refused(
        &oracle.call(
            "argon2-unmarshal",
            json!({"data": hex(&truncated[..truncated.len() - 1])}),
        ),
        "argon2-unmarshal of truncated parameters",
    );
    let own_threads = fresh.threads.max(1);
    for (memory, threads) in [
        (64u32, 1u8),
        (256, 4),
        (8 * u32::from(own_threads), own_threads),
    ] {
        let params = Argon2idParams {
            memory,
            threads,
            ..fresh
        };
        let encoded = hex(&params.marshal_binary());
        let pass_len = rng.below(33) as usize;
        let pass = rng.bytes(pass_len);
        let key = params.derive_key(&pass).expect("argon2 derive");
        let go = ok(
            oracle.call(
                "argon2-derive",
                json!({"params": encoded, "pass": hex(&pass)}),
            ),
            "argon2-derive",
        );
        assert_eq!(
            go["key"].as_str(),
            Some(hex(key.as_bytes()).as_str()),
            "argon2 key"
        );
        for len in [0usize, 111, rng.below(300) as usize] {
            let pt = rng.bytes(len);
            let sealed = key.seal(&pt).expect("seal");
            let go = ok(
                oracle.call(
                    "account-unseal",
                    json!({"params": encoded, "pass": hex(&pass), "sealed": hex(&sealed)}),
                ),
                "account-unseal",
            );
            assert_eq!(go["key"].as_str(), Some(hex(key.as_bytes()).as_str()));
            assert_eq!(go["plaintext"].as_str(), Some(hex(&pt).as_str()));
            let k = hex(key.as_bytes());
            let go = ok(
                oracle.call("unseal", json!({"key": k, "sealed": hex(&sealed)})),
                "unseal",
            );
            assert_eq!(go["plaintext"].as_str(), Some(hex(&pt).as_str()));
            let bad = flip(&mut rng, &sealed);
            refused(
                &oracle.call("unseal", json!({"key": k, "sealed": hex(&bad)})),
                "unseal",
            );
            let bad = flip(&mut rng, &sealed);
            refused(
                &oracle.call(
                    "account-unseal",
                    json!({"params": encoded, "pass": hex(&pass), "sealed": hex(&bad)}),
                ),
                "account-unseal of a tampered blob",
            );
            // The salt is the first 16 bytes; the cost fields follow.
            let bad_params = flip_in(&mut rng, &params.marshal_binary(), 16);
            refused(
                &oracle.call(
                    "account-unseal",
                    json!({"params": hex(&bad_params), "pass": hex(&pass), "sealed": hex(&sealed)}),
                ),
                "account-unseal under parameters with a flipped salt bit",
            );
            *counts.entry("account blobs").or_default() += 1;
        }
    }
    eprintln!("Rust-written values opened by Go: {counts:?}");
    assert_eq!(counts["master key parameters"], costs.len());
    assert_eq!(counts["master key boxes"], 3 * costs.len());
    assert_eq!(counts["crypto key boxes"], 4 * costs.len());
}

/// The scrypt costs the Rust-to-Go test above takes as dcrwallet's are the
/// ones dcrwallet's `udb` actually wrote: the upstream fixture databases
/// (testnet3) carry [`DCRWALLET_SCRYPT`] in both master key parameter sets,
/// the generated simnet database [`DCRWALLET_SIMNET_SCRYPT`]. Needs no
/// oracle.
#[test]
fn dcrwallet_scrypt_costs_are_the_ones_go_wrote() {
    let costs_of = |db: &purse_boltread::Db, what: &str| {
        for name in [&b"mpub"[..], b"mpriv"] {
            let stored = get(db, &[WADDRMGR, b"main"], name)
                .unwrap_or_else(|| panic!("{what}: no main/mpub or main/mpriv"));
            let p = *SecretKey::unmarshal(&stored)
                .expect("master key parameters")
                .parameters();
            let net_costs = (p.n, p.r, p.p);
            if what == GENERATED_DB {
                assert_eq!(net_costs, DCRWALLET_SIMNET_SCRYPT, "{what}");
            } else {
                assert_eq!(net_costs, DCRWALLET_SCRYPT, "{what}");
            }
        }
    };
    for (file, _) in FIXTURES {
        costs_of(&open_db(upstream_fixture(file)), file);
    }
    costs_of(
        &open_db(gunzip(&data_dir().join(GENERATED_DB))),
        GENERATED_DB,
    );
}

#[test]
fn frozen_vectors_against_go() {
    let entries = read_vectors(VECTORS, VECTORS_ENTRIES);
    if !replay_against_go(&entries) {
        return;
    }
    eprintln!("{} frozen entries replayed against Go", entries.len());
}

#[test]
fn frozen_fixture_vectors_against_go() {
    let entries = read_vectors(FIXTURE_VECTORS, FIXTURE_VECTORS_ENTRIES);
    assert!(
        entries
            .iter()
            .all(|e| DETERMINISTIC.contains(&e.cmd.as_str()))
    );
    replay_against_go(&entries);
}

/// The checked-in database, opened by Go through udb's public API with the
/// recorded passphrases: every member of the recorded result is read back
/// from it (open-db), and must equal what make-db reported when it wrote it.
#[test]
fn frozen_generated_database_against_go() {
    let Some(mut oracle) = oracle_or_skip("snacl") else {
        return;
    };
    let line = read_generated_expect();
    let bytes = gunzip(&data_dir().join(GENERATED_DB));
    assert_eq!(
        line["db_sha256"].as_str(),
        Some(hex(&Sha256::digest(&bytes)).as_str()),
        "{GENERATED_DB} is not the database the expectations were recorded from"
    );
    // A copy: opening a database may write to it.
    let path = scratch("generated-frozen-replay").join("wallet.db");
    std::fs::write(&path, &bytes).expect("write the database copy");
    let mut request = line["request"].clone();
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("open-db", request);
    assert_eq!(
        outcome,
        json!({ "ok": line["result"] }),
        "Go reads back what it recorded"
    );
}

/// The commands whose outcome depends on nothing but their arguments.
/// argon2-new-params is among them (its reader is the request's stream),
/// except for the machine's CPU count, which its answer reports.
const DETERMINISTIC: [&str; 11] = [
    "marshal",
    "unmarshal",
    "derive-key",
    "crypto-decrypt",
    "secret-decrypt",
    "unseal",
    "account-unseal",
    "argon2-marshal",
    "argon2-unmarshal",
    "argon2-derive",
    "argon2-new-params",
];

/// Replays frozen entries against Go, on several oracle processes at once;
/// false when there is no oracle (the test skips).
fn replay_against_go(entries: &[Entry]) -> bool {
    if oracle_or_skip("snacl").is_none() {
        return false;
    }
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let chunk = entries.len().div_ceil(workers).max(1);
    std::thread::scope(|s| {
        for part in entries.chunks(chunk) {
            s.spawn(move || {
                let mut oracle = oracle_or_skip("snacl").expect("the oracle was available");
                for e in part {
                    replay_entry(&mut oracle, e);
                }
            });
        }
    });
    true
}

/// Checks one frozen entry against Go: a deterministic request must give
/// the recorded outcome again; a request that drew randomness is checked by
/// its inverse, since Go cannot be made to draw the same bytes again.
fn replay_entry(oracle: &mut Oracle, e: &Entry) {
    let ctx = format!("{} {}", e.cmd, e.args);
    let a = &e.args;
    let ok = e.outcome.get("ok");
    let mut call = |cmd: &str, args: Value| oracle.call(cmd, args);
    match (e.cmd.as_str(), ok) {
        ("argon2-new-params", _) => {
            let live = call(&e.cmd, a.clone());
            assert_eq!(
                without_cpu_count(&live),
                without_cpu_count(&e.outcome),
                "{ctx}"
            );
        }
        (cmd, _) if DETERMINISTIC.contains(&cmd) => {
            assert_eq!(call(cmd, a.clone()), e.outcome, "{ctx}");
        }
        ("new-secret-key", Some(ok)) => {
            let params = &ok["params"];
            let u = call("unmarshal", json!({ "data": params }));
            for k in ["n", "r", "p"] {
                assert_eq!(u["ok"][k], a[k], "{ctx}: {k}");
            }
            assert_eq!(
                call("derive-key", json!({"params": params, "pass": a["pass"]})),
                json!({"ok": {"key": ok["key"], "key_after": ok["key"]}}),
                "{ctx}: the key derived again from the recorded parameters"
            );
        }
        ("generate-crypto-key", Some(ok)) => {
            let key = &ok["key"];
            assert_eq!(key.as_str().map(str::len), Some(64), "{ctx}");
            let pt = hex(b"a crypto key round trip");
            let sealed = call("crypto-encrypt", json!({"key": key, "plaintext": pt}));
            assert_eq!(
                call(
                    "crypto-decrypt",
                    json!({"key": key, "sealed": sealed["ok"]["sealed"]})
                ),
                json!({"ok": {"plaintext": pt}}),
                "{ctx}"
            );
        }
        (cmd @ ("crypto-encrypt" | "secret-encrypt" | "seal"), Some(ok)) => {
            let open = match cmd {
                "crypto-encrypt" => "crypto-decrypt",
                "secret-encrypt" => "secret-decrypt",
                _ => "unseal",
            };
            let sealed = ok["sealed"].as_str().expect("sealed");
            let pt = a["plaintext"].as_str().expect("plaintext");
            let overhead = if cmd == "seal" {
                XCHACHA20POLY1305_OVERHEAD
            } else {
                NONCE_SIZE + OVERHEAD
            };
            assert_eq!(sealed.len(), pt.len() + 2 * overhead, "{ctx}");
            assert_eq!(
                call(open, json!({"key": a["key"], "sealed": sealed})),
                json!({"ok": {"plaintext": pt}}),
                "{ctx}: the recorded box opens to the recorded plaintext"
            );
        }
        ("account-seal", Some(ok)) => {
            let sealed = ok["sealed"].as_str().expect("sealed");
            let pt = a["plaintext"].as_str().expect("plaintext");
            assert_eq!(
                sealed.len(),
                pt.len() + 2 * XCHACHA20POLY1305_OVERHEAD,
                "{ctx}"
            );
            assert_eq!(
                call(
                    "account-unseal",
                    json!({"params": a["params"], "pass": a["pass"], "sealed": sealed})
                ),
                json!({"ok": {"key": ok["key"], "plaintext": pt}}),
                "{ctx}: the recorded blob opens to the recorded plaintext"
            );
        }
        // A request that failed drew nothing visible: the failure is
        // deterministic.
        (
            "new-secret-key"
            | "generate-crypto-key"
            | "crypto-encrypt"
            | "secret-encrypt"
            | "seal"
            | "account-seal",
            None,
        ) => {
            assert_eq!(call(&e.cmd, a.clone()), e.outcome, "{ctx}");
        }
        (cmd, _) => panic!("no Go replay for {cmd}"),
    }
}

/// An argon2-new-params outcome without what depends on the machine: the
/// CPU count it reports, and the thread count (the last byte of the
/// parameters) set from it, which must be the count Go's truncation gives.
fn without_cpu_count(outcome: &Value) -> Value {
    let mut o = outcome.clone();
    let member = if o.get("ok").is_some() { "ok" } else { "err" };
    let body = o
        .get_mut(member)
        .and_then(Value::as_object_mut)
        .expect("ok or err");
    let cpus = body
        .remove("num_cpu")
        .and_then(|v| v.as_u64())
        .expect("num_cpu");
    if let Some(params) = body.get("params").and_then(Value::as_str) {
        let mut bytes = purse_testutil::unhex(params);
        let threads = bytes.pop().expect("25 bytes");
        let cpus = usize::try_from(cpus).expect("fits");
        assert_eq!(threads, Argon2idParams::threads_for_cpus(cpus), "{outcome}");
        body.insert("params".to_owned(), json!(hex(&bytes)));
    }
    o
}

#[test]
#[ignore = "writes tests/data from the Go oracle; set PURSE_SNACL_REGEN=1"]
fn regenerate_frozen_vectors() {
    if !regen_requested() {
        eprintln!("skipping: set PURSE_SNACL_REGEN=1 to regenerate tests/data");
        return;
    }
    let mut oracle = oracle_or_skip("snacl").expect("regeneration needs the oracle");
    // The release alone: a distribution's build appends its own tags.
    let go = oracle
        .go_version()
        .split(['-', ' ', '+'])
        .next()
        .unwrap_or_default()
        .to_owned();

    // The general corpus.
    let mut rng = SplitMix64(FROZEN_RNG_SEED);
    let entries = build_corpus(&mut rng, &mut ask(&mut oracle));
    par_map(&entries, check_entry);
    eprintln!("{NOT_REPRODUCIBLE}");
    let mut header = provenance("General vectors for purse-snacl", &go);
    header.extend(not_reproducible_note());
    header.extend([
        format!(
            "Inputs: tests/common/mod.rs build_corpus with SplitMix64({FROZEN_RNG_SEED:#x}); later requests"
        ),
        "use earlier answers. Each line is {cmd, args, outcome}: the request as sent and Go's".to_owned(),
        "outcome, in the shapes tools/oracle/cmd/snacl/main.go documents. Lines with a".to_owned(),
        "\"divergence\" member pin a recorded difference (see the crate docs).".to_owned(),
        String::new(),
    ]);
    write_vectors(VECTORS, &header, &entries);

    // The upstream fixtures.
    let mut fixture_entries = Vec::new();
    for (file, wrong) in FIXTURES {
        let flow = fixture_flow(file, wrong);
        let go_flow: Vec<Entry> = flow
            .iter()
            .map(|e| Entry {
                outcome: oracle.call(&e.cmd, e.args.clone()),
                ..e.clone()
            })
            .collect();
        check_flow_against(&flow, &go_flow, file);
        fixture_entries.extend(go_flow);
    }
    let mut header = provenance("Upstream fixture vectors for purse-snacl", &go);
    header.extend([
        "Inputs: the master key parameters and sealed keys of dcrwallet's own fixture".to_owned(),
        "databases (crates/purse-boltread/tests/data/upstream, from wallet/udb/testdata),"
            .to_owned(),
        "opened with the passphrases their generators name (\"public\", \"private\"), as"
            .to_owned(),
        "tests/common/mod.rs fixture_flow requests them. The \"fixture\" member names the"
            .to_owned(),
        "database. key_after is Go's diagnostic, not compared for these.".to_owned(),
        "These requests draw nothing at random: regeneration reproduces this file byte for"
            .to_owned(),
        "byte, and frozen_fixture_vectors_against_go replays it against Go on every oracle run."
            .to_owned(),
        String::new(),
    ]);
    write_vectors(FIXTURE_VECTORS, &header, &fixture_entries);

    // The generated database.
    let dir = scratch("generated-regen");
    let path = dir.join("wallet.db");
    let mut request = make_db_request("simnet");
    request["path"] = json!(path.to_str().expect("UTF-8 path"));
    let outcome = oracle.call("make-db", request.clone());
    let expect = outcome.get("ok").expect("make-db").clone();
    let bytes = std::fs::read(&path).expect("read the database");
    check_generated_db(bytes.clone(), &request, &expect);
    let gz = std::fs::File::create(data_dir().join(GENERATED_DB)).expect("create db.gz");
    let mut enc = flate2::write::GzEncoder::new(gz, flate2::Compression::best());
    enc.write_all(&bytes).expect("compress");
    enc.finish().expect("finish gzip");
    if let Some(o) = request.as_object_mut() {
        o.remove("path");
    }
    let line = serde_json::to_string(&json!({
        "request": request,
        "result": expect,
        "db_sha256": hex(&Sha256::digest(&bytes)),
    }))
    .expect("serialize");
    let mut text = String::new();
    let mut header = provenance("The generated database for purse-snacl", &go);
    header.extend(not_reproducible_note());
    for h in header {
        text.push_str(&format!("# {h}\n").replace("# \n", "#\n"));
    }
    for h in [
        format!("{GENERATED_DB} was written by the oracle's make-db through udb's public API"),
        "(Initialize, then a second account given its own passphrase with".to_owned(),
        "SetAccountPassphrase, an imported voting account, an imported private key and an"
            .to_owned(),
        "imported script); the line below holds the request (path omitted), Go's result and"
            .to_owned(),
        "the SHA-256 of the decompressed database. Argon2id threads are the generating".to_owned(),
        "machine's CPU count, as dcrwallet stores them.".to_owned(),
    ] {
        text.push_str(&format!("# {h}\n"));
    }
    text.push_str("# entries: 1\n");
    text.push_str(&format!("# body-sha256: {}\n", body_sha256(&line)));
    text.push_str(&line);
    text.push('\n');
    std::fs::write(data_dir().join(GENERATED_EXPECT), text).expect("write expectations");
    eprintln!("wrote tests/data/{GENERATED_DB} and tests/data/{GENERATED_EXPECT}");
}

/// What the regeneration prints, and the headers of the files it concerns
/// say.
const NOT_REPRODUCIBLE: &str = "note: snacl_vectors.txt, generated_db.txt and generated-simnet.db.gz \
     will not regenerate byte for byte: Go draws salts, nonces and keys from dcrd's crypto/rand, \
     which the oracle cannot steer, and later requests use earlier answers";

/// The header lines of the files whose regeneration is not reproducible.
fn not_reproducible_note() -> Vec<String> {
    [
        "Regeneration does not reproduce this file byte for byte: Go draws salts, nonces and",
        "keys from github.com/decred/dcrd/crypto/rand, a generator seeded from the operating",
        "system that the oracle cannot steer, and later requests are built from earlier",
        "answers. So tests/oracle.rs replays every entry (and the database) against Go on",
        "every run with the oracle: deterministic requests must give the recorded outcome",
        "again, and each random draw is checked by its inverse (the key derived again from",
        "the recorded parameters, the recorded box opened to the recorded plaintext).",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The header lines every frozen file starts with.
fn provenance(title: &str, go: &str) -> Vec<String> {
    vec![
        title.to_owned(),
        String::new(),
        "Every expected value is the output of the Go oracle tools/oracle/cmd/snacl, built with"
            .to_owned(),
        format!("{go}, which links dcrwallet {DCRWALLET_PARITY_COMMIT}'s"),
        "kdf and udb packages, a verbatim copy of its wallet/internal/snacl and a verbatim copy"
            .to_owned(),
        "of udb's seal/unseal. Regenerate with:".to_owned(),
        "  PURSE_SNACL_REGEN=1 PURSE_REQUIRE_ORACLE=1 cargo test -p purse-snacl --test oracle \\"
            .to_owned(),
        "      -- --ignored regenerate_frozen_vectors".to_owned(),
        "which also checks the Rust crate agrees. Do not edit by hand.".to_owned(),
    ]
}
