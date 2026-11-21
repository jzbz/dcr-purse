// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The benchmark harness of ADR-0014 (section 4, "Enforcement"): dcr-purse's
//! workloads and their Go counterparts (`tools/oracle/cmd/bench`, which
//! links the packages dcrwallet links at the parity pin) on the same
//! inputs, run alternately in one session, with the results recorded in
//! `docs/bench-ledger.md` under its rules. Not shipped.
//!
//! # Running it
//!
//! ```text
//! cargo build --release -p purse-bench
//! target/release/purse-bench drive --out <new directory> --runs 3 \
//!     --cpus 12-15 --go-tags flint
//! ```
//!
//! `drive` builds the Go side from `tools/oracle` (with `--go-tags flint`
//! the mixing solver's reference, cgo over FLINT, is included; without it
//! those workloads are reported as skipped on the Go side), then runs
//! `--runs` processes per side, alternating (Rust then Go in odd runs, Go
//! then Rust in even ones), each pinned with `taskset -c` when `--cpus` is
//! given. Every process's output is kept in the directory, with the
//! session's notes and `table.md`, the table printed at the end. `--smoke`
//! runs one repetition of one operation each at reduced sizes, to check
//! that the workloads run and agree; it is what the test suite runs, and
//! its times mean nothing. `--gate <baseline>` compares each ratio with a
//! baseline and fails above twice it (ADR-0014's CI gate). `--only` takes
//! comma-separated substrings of workload names. `run` is one side's
//! process, which `drive` starts.
//!
//! # Workloads
//!
//! Inputs are fixed: the seed is the bytes 0 to 31 on mainnet, the account
//! `m/44'/42'/0'`, and every other input comes from SplitMix64 with a seed
//! per group. Calibrated workloads repeat an operation for about `--ms`
//! milliseconds per repetition; the others (marked once) time single
//! operations.
//!
//! | name | operation | Phase 1 criterion |
//! |---|---|---|
//! | `hd.master` | master key from a 32-byte seed, with its public key | secret path, ≤ 1.15× Go |
//! | `hd.hardened_child` | hardened child of the account key, with its public key | secret path, ≤ 1.15× Go |
//! | `hd.priv_child` | normal child of the private branch key, with its public key | secret path, ≤ 1.15× Go |
//! | `hd.pub_child` | normal child of the public branch key | no slower |
//! | `hd.address` | public child, then its P2PKH address string | no slower |
//! | `hd.script` | public child, then its P2PKH script | no slower |
//! | `hd.addr20k` (once) | the first 20,000 addresses of a branch, one thread | ≥ 1.5× faster |
//! | `hd.xpub_string` | an extended public key as a string, cycling through the first 16,384 keys of the public branch | — |
//! | `hd.parse_xpub` | parse of those keys' strings, in turn | — |
//! | `crypto.hmac_sha512` | HMAC-SHA512, 32-byte key, 37-byte message (one derivation step) | no slower |
//! | `crypto.hash160` | RIPEMD160(BLAKE256) of a 33-byte key | no slower |
//! | `crypto.base58check_encode` | base58check of a 20-byte hash, 2-byte version (a mainnet address), cycling through 16,384 drawn hashes | no slower |
//! | `crypto.base58check_decode` | the reverse, cycling through those 16,384 addresses | no slower |
//! | `crypto.base58check_decode_one` | the reverse of one address, the same string every time (what `crypto.base58check_decode` timed until 2026-11-21) | — |
//! | `snacl.derive` (once) | a master key's scrypt derivation at snacl's defaults (N = 16384, r = 8, p = 1) and digest check | — |
//! | `snacl.seal_32`, `snacl.seal_111` | secretbox seal under a crypto key, fresh nonce from the default source | — |
//! | `snacl.open_32`, `snacl.open_111` | secretbox open | — |
//! | `kdf.argon2id_t1`, `kdf.argon2id_t4` (once) | an account passphrase's Argon2id at dcrwallet's cost (time 1, 256 MiB), 1 and 4 lanes | — |
//! | `dcrutil.amount_string` | `Amount`'s string form, amounts of every magnitude | — |
//! | `mix.roots_20`, `mix.roots_64`, `mix.roots_512` (once) | the mixing root solver at three degrees | — |
//!
//! The base58 workloads (`crypto.base58check_*` and the two `hd.*xpub*`)
//! take a different string, or the input of a different string, on every
//! call, cycling through a fixed set the two sides build alike and
//! cross-check (`*_set_*` checks: the strings' digest and the digest of
//! what each side decodes them to): with one string repeated, the
//! per-character branches of a conversion are learnt by the branch
//! predictor and replayed, which favours a decoder whose cost lies in
//! such branches (dcroxide's ledger, the base58 entry of 2026-11-20).
//!
//! Smoke mode reduces 20,000 addresses to 200, the extended keys the
//! `hd.*xpub*` workloads cycle through to 256 (the 16,384 addresses of the
//! base58check workloads are not reduced), scrypt's N to 1024, Argon2id's
//! memory to 1 MiB and the solver's degrees to 4, 8 and 32.
//!
//! # Output
//!
//! Each side prints tab-separated lines: `META side key value`,
//! `CHECK side name value` (an output both sides must give; the driver
//! refuses a session where any differs), `RESULT side name median min max
//! ops-per-rep reps all-reps` (nanoseconds per operation), and
//! `SKIP side name reason`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "a measurement tool, not shipped: counters, statistics and input generation"
)]

pub mod drive;
pub mod harness;
pub mod report;
pub mod workloads;
