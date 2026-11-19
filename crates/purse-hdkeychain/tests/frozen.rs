// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Frozen vectors in `tests/data`, generated from the Go oracle (dcrd
//! hdkeychain v3.1.3) by `regenerate_frozen_vectors`, so the comparison
//! with Go is pinned even where no Go toolchain is available. No expected
//! value in these files was written by hand.
//!
//! To regenerate (needs Go; a release build keeps the search fast):
//!
//! ```text
//! PURSE_HD_REGEN=1 cargo test --release -p purse-hdkeychain --test frozen \
//!     -- --ignored regenerate_frozen_vectors
//! ```

#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: counters and index arithmetic on small, bounded values"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use common::*;
use purse_hdkeychain::{ExtendedKey, HdVersions};
use purse_testutil::{SplitMix64, oracle_or_skip, unhex};

const LEADING_ZERO_LINES: &str = "leading_zero_lines.txt";
const LEADING_ZERO_DIGESTS: &str = "leading_zero_digests.txt";
const PARSE_CORPUS_LINES: &str = "parse_corpus_lines.txt";
const PARSE_CORPUS_DIGESTS: &str = "parse_corpus_digests.txt";
const RANDOM_DIGESTS: &str = "random_seed_digests.txt";

/// The entries each file must hold (`# entries:` is checked against these
/// and against the body).
const LEADING_ZERO_LINES_ENTRIES: usize = 16;
const LEADING_ZERO_DIGESTS_ENTRIES: usize = 255;
const PARSE_CORPUS_ENTRIES: usize = 399;
const RANDOM_ENTRIES: usize = 128;
const RANDOM_PER_NET: usize = 32;

/// The PRNG seeds the regenerator uses, so a regeneration is reproducible.
const SEARCH_BASE: u64 = 0x6864_6b65_7963_6861;
const CORPUS_SEED: u64 = 0x636f_7270_7573_0001;
const RANDOM_SEED: u64 = 0x7261_6e64_6f6d_0001;

/// Reads a vector file and checks its integrity headers and pinned size.
fn read(name: &str, entries: usize) -> String {
    let text = std::fs::read_to_string(data_dir().join(name))
        .unwrap_or_else(|e| panic!("read tests/data/{name}: {e}"));
    assert_eq!(
        verify_integrity(name, &text),
        entries,
        "{name}: pinned entry count"
    );
    text
}

fn check_lines_file(name: &str, count: usize) -> usize {
    let entries = read_lines_file(&read(name, count));
    assert!(!entries.is_empty(), "{name} is empty");
    let results = par_map(&entries, |e| run_job(&e.job, &e.net));
    for (e, r) in entries.iter().zip(&results) {
        assert!(
            *r == e.lines,
            "{name}: {}",
            first_difference(&e.lines, r, &e.job)
        );
    }
    entries.len()
}

fn check_digest_file(name: &str, count: usize) -> usize {
    let entries = read_digest_file(&read(name, count));
    assert!(!entries.is_empty(), "{name} is empty");
    let results = par_map(&entries, |e| digest(&run_job(&e.job, &e.net)));
    for (e, r) in entries.iter().zip(&results) {
        assert_eq!(
            r, &e.digest,
            "{name}: root {:?} ({}) differs from Go's frozen digest",
            e.job.root, e.note
        );
    }
    entries.len()
}

#[test]
fn leading_zero_lines() {
    let n = check_lines_file(LEADING_ZERO_LINES, LEADING_ZERO_LINES_ENTRIES);
    eprintln!("{LEADING_ZERO_LINES}: {n} jobs agree");
}

#[test]
fn leading_zero_digests() {
    let entries = read_digest_file(&read(LEADING_ZERO_DIGESTS, LEADING_ZERO_DIGESTS_ENTRIES));
    let n = check_digest_file(LEADING_ZERO_DIGESTS, LEADING_ZERO_DIGESTS_ENTRIES);
    // The file covers every hardened depth, with two and three zero bytes.
    let mut classes: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for e in &entries {
        if let Root::Seed(s) = &e.job.root {
            let (zeros, _) = classify(s);
            for (d, z) in zeros.iter().enumerate() {
                if *z > 0 {
                    *classes.entry((d, *z)).or_default() += 1;
                }
            }
        }
    }
    eprintln!("{LEADING_ZERO_DIGESTS}: {n} seeds agree; (depth, zero bytes) -> seeds: {classes:?}");
    assert!(n >= 200);
    for d in 0..4 {
        assert!(
            classes.get(&(d, 1)).copied().unwrap_or(0) >= 25,
            "depth {d}"
        );
        assert!(
            classes.get(&(d, 2)).copied().unwrap_or(0) >= 1,
            "depth {d}, two zeros"
        );
    }
    assert!(classes.keys().any(|(_, z)| *z >= 3), "no three-zero case");
}

#[test]
fn parse_corpus_lines() {
    let n = check_lines_file(PARSE_CORPUS_LINES, PARSE_CORPUS_ENTRIES);
    eprintln!("{PARSE_CORPUS_LINES}: {n} strings agree");
}

#[test]
fn parse_corpus_digests() {
    let n = check_digest_file(PARSE_CORPUS_DIGESTS, PARSE_CORPUS_ENTRIES);
    eprintln!("{PARSE_CORPUS_DIGESTS}: {n} jobs agree");
}

#[test]
fn random_seed_digests() {
    let n = check_digest_file(RANDOM_DIGESTS, RANDOM_ENTRIES);
    eprintln!("{RANDOM_DIGESTS}: {n} seeds agree");
}

/// The jobs a deterministic corpus file holds, rebuilt from its fixed PRNG
/// seed: (network, root, paths) in file order.
fn job_list<'a>(
    entries: impl Iterator<Item = (&'a HdVersions, &'a Job)>,
) -> Vec<(HdVersions, Root, Vec<String>)> {
    entries
        .map(|(n, j)| (*n, j.root.clone(), j.paths.clone()))
        .collect()
}

fn corpus_jobs(paths: Vec<String>) -> Vec<(HdVersions, Root, Vec<String>)> {
    let corpus = parse_corpus(&mut SplitMix64(CORPUS_SEED));
    let mut out = Vec::new();
    for (_, n) in NETS {
        for (cn, s) in &corpus {
            if *cn == n {
                out.push((n, Root::Key(s.clone()), paths.clone()));
            }
        }
    }
    out
}

fn random_job_list() -> Vec<(HdVersions, Root, Vec<String>)> {
    let mut rng = SplitMix64(RANDOM_SEED);
    let mut out = Vec::new();
    for (_, n) in NETS {
        for j in random_jobs(&mut rng, RANDOM_PER_NET) {
            out.push((n, j.root, j.paths));
        }
    }
    out
}

/// The files built from a fixed PRNG seed still hold exactly the jobs that
/// seed builds now, so a change to the generators cannot leave them stale
/// unnoticed (no Go needed).
#[test]
fn deterministic_files_are_not_stale() {
    let lines = read_lines_file(&read(PARSE_CORPUS_LINES, PARSE_CORPUS_ENTRIES));
    assert!(
        job_list(lines.iter().map(|e| (&e.net, &e.job))) == corpus_jobs(vec![String::new()]),
        "{PARSE_CORPUS_LINES} is stale: regenerate"
    );
    let digests = read_digest_file(&read(PARSE_CORPUS_DIGESTS, PARSE_CORPUS_ENTRIES));
    assert!(
        job_list(digests.iter().map(|e| (&e.net, &e.job))) == corpus_jobs(corpus_paths()),
        "{PARSE_CORPUS_DIGESTS} is stale: regenerate"
    );
    let random = read_digest_file(&read(RANDOM_DIGESTS, RANDOM_ENTRIES));
    assert!(
        job_list(random.iter().map(|e| (&e.net, &e.job))) == random_job_list(),
        "{RANDOM_DIGESTS} is stale: regenerate"
    );
}

// ---------------------------------------------------------------------------
// Regeneration.
// ---------------------------------------------------------------------------

fn header(what: &str, how: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# {what}");
    let _ = writeln!(s, "#");
    for line in how.lines() {
        let _ = writeln!(s, "# {line}");
    }
    let _ = writeln!(s, "#");
    let _ = writeln!(
        s,
        "# Every expected value is the output of the Go oracle tools/oracle/cmd/hdkeychain\n\
         # (github.com/decred/dcrd/hdkeychain/v3 v3.1.3, as linked by dcrwallet {}),\n\
         # written by `PURSE_HD_REGEN=1 cargo test --release -p purse-hdkeychain --test frozen\n\
         # -- --ignored regenerate_frozen_vectors`, which also checked the Rust crate equal.\n\
         # Format: crates/purse-hdkeychain/tests/common/mod.rs.",
        &purse_testutil::DCRWALLET_PARITY_COMMIT[..12]
    );
    s
}

fn note_for(seed: &[u8]) -> String {
    let (zeros, stripped) = classify(seed);
    format!(
        "zeros master={} purpose={} coin={} account={}; stored stripped purpose={} coin={} account={}",
        zeros[0],
        zeros[1],
        zeros[2],
        zeros[3],
        u8::from(stripped[1]),
        u8::from(stripped[2]),
        u8::from(stripped[3])
    )
}

fn go_digest_entries(
    oracle: &mut purse_testutil::Oracle,
    net: &HdVersions,
    jobs: Vec<Job>,
    notes: Vec<String>,
) -> Vec<DigestEntry> {
    let mut go = Vec::new();
    for chunk in jobs.chunks(256) {
        go.extend(go_digests(oracle, net, chunk));
    }
    let rust = par_map(&jobs, |j| digest(&run_job(j, net)));
    assert_eq!(go, rust, "Rust disagrees with Go; not writing vectors");
    jobs.into_iter()
        .zip(go)
        .zip(notes)
        .map(|((job, digest), note)| DigestEntry {
            net: *net,
            job,
            digest,
            note,
        })
        .collect()
}

fn go_lines_entries(
    oracle: &mut purse_testutil::Oracle,
    net: &HdVersions,
    jobs: Vec<Job>,
) -> Vec<LinesEntry> {
    let go = go_lines(oracle, net, &jobs);
    let rust = par_map(&jobs, |j| run_job(j, net));
    for ((g, r), j) in go.iter().zip(&rust).zip(&jobs) {
        assert!(
            g == r,
            "Rust disagrees with Go: {}",
            first_difference(g, r, j)
        );
    }
    jobs.into_iter()
        .zip(go)
        .map(|(job, lines)| LinesEntry {
            net: *net,
            job,
            lines,
        })
        .collect()
}

/// Writes a vector file: the descriptive header, the integrity headers,
/// then the body.
fn write(name: &str, header: &str, body: &str) {
    let text = format!("{header}{}{body}", integrity_header(body));
    let path = data_dir().join(name);
    std::fs::write(&path, &text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!(
        "wrote tests/data/{name} ({} entries, {} bytes)",
        count_entries(body),
        text.len()
    );
}

#[test]
#[ignore = "writes tests/data from the Go oracle; set PURSE_HD_REGEN=1"]
fn regenerate_frozen_vectors() {
    if std::env::var_os("PURSE_HD_REGEN").is_none() {
        eprintln!("skipping: set PURSE_HD_REGEN=1 to regenerate tests/data");
        return;
    }
    let Some(mut oracle) = oracle_or_skip("hdkeychain") else {
        return;
    };
    let net = HdVersions::MAINNET;
    let per_worker: u64 = std::env::var("PURSE_HD_REGEN_PER_WORKER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let workers = 32u64;

    // Leading-zero seeds: screen, classify with the crate, select.
    let found = search(SEARCH_BASE, workers, per_worker);
    let mut quota: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut selected: Vec<Vec<u8>> = Vec::new();
    let classified = par_map(&found, |(s, _)| classify(s));
    for ((seed, _), (zeros, _)) in found.iter().zip(&classified) {
        let max = *zeros.iter().max().expect("4 depths");
        if max == 0 {
            continue;
        }
        let depth = zeros.iter().position(|&z| z == max).expect("present");
        let cap = match max {
            1 => 50,
            2 => 10,
            _ => 6,
        };
        let n = quota.entry((depth, max.min(3))).or_default();
        if *n < cap {
            *n += 1;
            selected.push(seed.clone());
        }
    }
    eprintln!(
        "screened {} seeds, {} candidates, selected {}: {quota:?}",
        workers * per_worker,
        found.len(),
        selected.len()
    );

    // dcr-rs's seeds, and dcrd's vector 4 seed.
    let named: Vec<(&str, Vec<u8>)> = vec![
        ("dcrd BIP32 vector 4", unhex(VEC4_SEED)),
        (
            "dcr-rs ZERO_LEAD_SEED_B",
            unhex("2fb78f0f7770720fa05952587c055866852c1e2f2cd7814e92e06d91b72af083"),
        ),
        (
            "dcr-rs ZERO_LEAD_MASTER_SEED",
            unhex("de8ed51864182c52cc0205cd0ec55fa236a48c2bce8e9994c69ab44ae31ebb5e"),
        ),
        (
            "dcr-rs mixed-variant seed",
            unhex("667c017d055e9d57ab3374a9d49b061e4f5de2c2a767896b67d59b5d6eeeca33"),
        ),
    ];

    let mut jobs = Vec::new();
    let mut notes = Vec::new();
    for (name, seed) in &named {
        jobs.push(Job::seed(seed, leading_zero_paths()));
        notes.push(format!("{name}; {}", note_for(seed)));
    }
    for seed in &selected {
        jobs.push(Job::seed(seed, leading_zero_paths()));
        notes.push(note_for(seed));
    }
    let entries = go_digest_entries(&mut oracle, &net, jobs, notes);
    let head = header(
        "Leading-zero seeds: one SHA-256 digest of Go's lines per seed over the paths listed.",
        &format!(
            "Seeds: dcrd's BIP32 vector 4 seed and dcr-rs's leading-zero seeds, then seeds whose\n\
             private keys at master, purpose, coin type or account depth start with 1, 2 or 3\n\
             zero bytes, found by screening {} 32-byte seeds from SplitMix64 streams based at\n\
             {SEARCH_BASE:#x} (tests/common/mod.rs `search`), at most 50/10/6 per depth and\n\
             zero count. Each note is the crate's own classification.",
            workers * per_worker
        ),
    );
    let mut body = String::new();
    write_digest_entries(&mut body, &entries);
    write(LEADING_ZERO_DIGESTS, &head, &body);

    // Full lines for a few: one seed per (depth, zero count), and the named
    // seeds, over a smaller path set.
    let (p, c42, c20) = (h(44), h(42), h(20));
    let line_paths = vec![
        format!("{p}/{c42}/{}/0/0", h(0)),
        format!("{p}s/{c42}s/{}s/0s/0s", h(0)),
        format!("{p}/{c42}r/{}/0/0", h(1)),
        format!("{p}/{c42}s/{}/0", h(0)),
        format!("{p}/{c20}/{}n/0/0", h(0)),
    ];
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut line_jobs: Vec<Job> = named
        .iter()
        .map(|(_, s)| Job::seed(s, line_paths.clone()))
        .collect();
    line_jobs[0].paths.push(format!("{}/{}", h(0), h(1)));
    line_jobs[0].paths.push(format!("{}s/{}s", h(0), h(1)));
    for seed in &selected {
        let (zeros, _) = classify(seed);
        let max = *zeros.iter().max().expect("4 depths");
        let depth = zeros.iter().position(|&z| z == max).expect("present");
        if seen.insert((depth, max.min(3))) {
            line_jobs.push(Job::seed(seed, line_paths.clone()));
        }
    }
    let entries = go_lines_entries(&mut oracle, &net, line_jobs);
    let head = header(
        "Leading-zero seeds with Go's full lines.",
        "The named seeds of leading_zero_digests.txt and the first selected seed of each\n\
         (depth, zero bytes) class, along five wallet paths.",
    );
    let mut body = String::new();
    write_lines_entries(&mut body, &entries);
    write(LEADING_ZERO_LINES, &head, &body);

    // The parse corpus: Go's parse result per string, then digests of the
    // derivations from each.
    let corpus = parse_corpus(&mut SplitMix64(CORPUS_SEED));
    let mut lines_entries = Vec::new();
    let mut digest_entries = Vec::new();
    for (_, n) in NETS {
        let keys: Vec<&String> = corpus
            .iter()
            .filter(|(cn, _)| *cn == n)
            .map(|(_, s)| s)
            .collect();
        if keys.is_empty() {
            continue;
        }
        let jobs: Vec<Job> = keys
            .iter()
            .map(|s| Job::key(s, vec![String::new()]))
            .collect();
        lines_entries.extend(go_lines_entries(&mut oracle, &n, jobs));
        let jobs: Vec<Job> = keys.iter().map(|s| Job::key(s, corpus_paths())).collect();
        let notes = vec![String::new(); jobs.len()];
        digest_entries.extend(go_digest_entries(&mut oracle, &n, jobs, notes));
    }
    let how = format!(
        "tests/common/mod.rs `parse_corpus` with SplitMix64({CORPUS_SEED:#x}): valid keys of every\n\
         network, wrong networks, character mutations, length and character edge cases, and\n\
         well-checksummed payloads with edge-valued versions, keys, depths and child numbers."
    );
    let head = header(
        "Parse corpus: Go's NewKeyFromString result per string.",
        &how,
    );
    let mut body = String::new();
    write_lines_entries(&mut body, &lines_entries);
    write(PARSE_CORPUS_LINES, &head, &body);
    let head = header(
        "Parse corpus: digests of Go's derivations from each parsed string.",
        &format!("{how}\nPaths: tests/common/mod.rs `corpus_paths`."),
    );
    let mut body = String::new();
    write_digest_entries(&mut body, &digest_entries);
    write(PARSE_CORPUS_DIGESTS, &head, &body);

    // Random seeds along random wallet paths, 32 per network.
    let mut rng = SplitMix64(RANDOM_SEED);
    let mut entries = Vec::new();
    for (_, n) in NETS {
        let jobs = random_jobs(&mut rng, RANDOM_PER_NET);
        let notes = vec![String::new(); jobs.len()];
        entries.extend(go_digest_entries(&mut oracle, &n, jobs, notes));
    }
    let head = header(
        "Random seeds along random wallet paths: digests of Go's lines.",
        &format!(
            "tests/common/mod.rs `random_jobs` with SplitMix64({RANDOM_SEED:#x}), 32 seeds per\n\
             network (mainnet, testnet3, simnet, regnet in that order)."
        ),
    );
    let mut body = String::new();
    write_digest_entries(&mut body, &entries);
    write(RANDOM_DIGESTS, &head, &body);

    // Keep the named seeds' expectations honest: the strings in
    // dcr_rs_vectors.rs must be what Go derives.
    let check = ExtendedKey::new_master(&named[1].1, &net)
        .and_then(|m| m.child(h(44)))
        .and_then(|k| k.child(h(42)))
        .and_then(|k| k.child(h(0)))
        .expect("path");
    let go = go_lines(
        &mut oracle,
        &net,
        &[Job::seed(
            &named[1].1,
            vec![format!("{}/{}/{}n", h(44), h(42), h(0))],
        )],
    );
    assert_eq!(go[0][0].last().expect("line"), &key_line(&check.neuter()));
}

#[test]
#[should_panic(expected = "body hash")]
fn a_changed_body_fails_the_integrity_check() {
    let text = read(RANDOM_DIGESTS, RANDOM_ENTRIES);
    let changed = text.replacen(
        "\nD 02fda4e8:02fda926 seed ",
        "\nD 02fda4e8:02fda926 seed 00",
        1,
    );
    verify_integrity("changed", &changed);
}

#[test]
#[should_panic(expected = "no \"# body-sha256:\" header")]
fn a_missing_header_fails_the_integrity_check() {
    let text = read(RANDOM_DIGESTS, RANDOM_ENTRIES);
    let stripped: String = text
        .lines()
        .filter(|l| !l.starts_with("# body-sha256:"))
        .map(|l| format!("{l}\n"))
        .collect();
    verify_integrity("stripped", &stripped);
}
