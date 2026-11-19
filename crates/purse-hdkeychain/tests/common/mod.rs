// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Shared by the differential and frozen-vector tests: the canonical key
//! line, the path language and the vector file format, all mirroring
//! `tools/oracle/cmd/hdkeychain`.
//!
//! A key is reported as one line:
//!
//! ```text
//! K|<private 0/1>|<depth>|<child num>|<parent fp>|<string>|<neuter string>|<pubkey hex>|<privkey hex, or ! and the error>|<padded privkey hex, or ! and the error>
//! E|<error text>
//! ```
//!
//! A path is `/`-separated steps; a step is an optional decimal index
//! (derived with `child`, or `child_bip32_std` with flag `s`) followed by
//! flags applied in order: `r` re-parses the key from its string, `n`
//! neuters it, `z` zeroes it. A job is a root (a seed or a key string) and
//! its paths; its answer is one line for the root and one per step, per
//! path, each path stopping at its first error.

#![allow(dead_code, reason = "each test binary uses a different part")]
#![allow(
    clippy::arithmetic_side_effects,
    reason = "test code: index arithmetic on small, bounded values"
)]

pub mod wallet;

use std::fmt::Write as _;
use std::path::PathBuf;

use purse_hdkeychain::{Error, ExtendedKey, HdVersions};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{Oracle, hex, unhex};
use sha2::{Digest, Sha256};

/// Hardened index `i'`.
pub const fn h(i: u32) -> u32 {
    i | 0x8000_0000
}

/// The four Decred networks with their names.
pub const NETS: [(&str, HdVersions); 4] = [
    ("mainnet", HdVersions::MAINNET),
    ("testnet3", HdVersions::TESTNET3),
    ("simnet", HdVersions::SIMNET),
    ("regnet", HdVersions::REGNET),
];

/// The canonical line of a key.
pub fn key_line(k: &ExtendedKey) -> String {
    let priv_field = match k.serialized_priv_key() {
        Ok(b) => hex(b),
        Err(e) => format!("!{e}"),
    };
    let padded_field = match k.priv_key_bytes() {
        Ok(b) => hex(&*b),
        Err(e) => format!("!{e}"),
    };
    format!(
        "K|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        u8::from(k.is_private()),
        k.depth(),
        k.child_num(),
        k.parent_fingerprint(),
        *k.string(),
        *k.neuter().string(),
        hex(k.serialized_pub_key()),
        priv_field,
        padded_field
    )
}

/// The canonical line of an error.
pub fn err_line(e: &Error) -> String {
    format!("E|{e}")
}

/// One step of a path.
#[derive(Clone, Debug)]
pub struct Step {
    pub index: Option<u32>,
    pub flags: String,
}

/// Parses a path in the oracle's language.
pub fn parse_path(path: &str) -> Vec<Step> {
    if path.is_empty() {
        return Vec::new();
    }
    path.split('/')
        .map(|tok| {
            let digits = tok.trim_end_matches(['s', 'r', 'n', 'z']);
            let flags = tok[digits.len()..].to_owned();
            let index = (!digits.is_empty()).then(|| digits.parse().expect("u32 step"));
            Step { index, flags }
        })
        .collect()
}

/// The root of a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    Seed(Vec<u8>),
    Key(String),
}

impl Root {
    pub fn make(&self, net: &HdVersions) -> Result<ExtendedKey, Error> {
        match self {
            Root::Seed(seed) => ExtendedKey::new_master(seed, net),
            Root::Key(key) => ExtendedKey::new_key_from_string(key, net),
        }
    }
}

/// A root and its paths.
#[derive(Clone, Debug)]
pub struct Job {
    pub root: Root,
    pub paths: Vec<String>,
}

impl Job {
    pub fn seed(seed: &[u8], paths: Vec<String>) -> Self {
        Job {
            root: Root::Seed(seed.to_vec()),
            paths,
        }
    }

    pub fn key(key: &str, paths: Vec<String>) -> Self {
        Job {
            root: Root::Key(key.to_owned()),
            paths,
        }
    }

    fn to_json(&self) -> Value {
        match &self.root {
            Root::Seed(s) => json!({ "seed": hex(s), "paths": self.paths }),
            Root::Key(k) => json!({ "key": k, "paths": self.paths }),
        }
    }
}

/// The lines of one path from a root.
pub fn run_path(root: &Root, path: &str, net: &HdVersions) -> Vec<String> {
    let mut k = match root.make(net) {
        Ok(k) => k,
        Err(e) => return vec![err_line(&e)],
    };
    let mut lines = vec![key_line(&k)];
    for step in parse_path(path) {
        if let Some(i) = step.index {
            let r = if step.flags.contains('s') {
                k.child_bip32_std(i)
            } else {
                k.child(i)
            };
            match r {
                Ok(c) => k = c,
                Err(e) => {
                    lines.push(err_line(&e));
                    return lines;
                }
            }
        }
        for f in step.flags.chars() {
            match f {
                'r' => match ExtendedKey::new_key_from_string(&k.string(), net) {
                    Ok(p) => k = p,
                    Err(e) => {
                        lines.push(err_line(&e));
                        return lines;
                    }
                },
                'n' => k = k.neuter(),
                'z' => k.zero(),
                _ => {}
            }
        }
        lines.push(key_line(&k));
    }
    lines
}

/// The lines of every path of a job.
pub fn run_job(job: &Job, net: &HdVersions) -> Vec<Vec<String>> {
    job.paths
        .iter()
        .map(|p| run_path(&job.root, p, net))
        .collect()
}

/// The digest of a job's lines, as the oracle computes it.
pub fn digest(paths: &[Vec<String>]) -> String {
    let mut h = Sha256::new();
    for lines in paths {
        for l in lines {
            h.update(l.as_bytes());
            h.update(b"\n");
        }
        h.update(b"#\n");
    }
    hex(&h.finalize())
}

/// Runs `f` over `items` on all cores, keeping the order.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker panicked"))
            .collect()
    })
}

pub fn net_json(net: &HdVersions) -> Value {
    json!({ "priv": hex(&net.private), "pub": hex(&net.public) })
}

/// Go's lines for each job.
pub fn go_lines(oracle: &mut Oracle, net: &HdVersions, jobs: &[Job]) -> Vec<Vec<Vec<String>>> {
    let args = json!({
        "net": net_json(net),
        "digest": false,
        "jobs": jobs.iter().map(Job::to_json).collect::<Vec<_>>(),
    });
    serde_json::from_value(oracle.call("derive", args)).expect("lines")
}

/// Go's digest for each job.
pub fn go_digests(oracle: &mut Oracle, net: &HdVersions, jobs: &[Job]) -> Vec<String> {
    let args = json!({
        "net": net_json(net),
        "digest": true,
        "jobs": jobs.iter().map(Job::to_json).collect::<Vec<_>>(),
    });
    serde_json::from_value(oracle.call("derive", args)).expect("digests")
}

/// Describes the first difference between two jobs' lines.
pub fn first_difference(go: &[Vec<String>], rust: &[Vec<String>], job: &Job) -> String {
    for (pi, (g, r)) in go.iter().zip(rust).enumerate() {
        for li in 0..g.len().max(r.len()) {
            let gl = g.get(li).map_or("<none>", String::as_str);
            let rl = r.get(li).map_or("<none>", String::as_str);
            if gl != rl {
                return format!(
                    "root {:?} path {:?} line {li}:\n  go:   {gl}\n  rust: {rl}",
                    job.root, job.paths[pi]
                );
            }
        }
    }
    if go.len() != rust.len() {
        return format!("path counts differ: go {} rust {}", go.len(), rust.len());
    }
    "no difference".to_owned()
}

/// What a comparison actually matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compared {
    /// Jobs whose answers were compared and equal.
    pub jobs: usize,
    /// Lines in those answers.
    pub lines: usize,
    /// Of those, error (`E|`) lines.
    pub error_lines: usize,
}

impl Compared {
    fn add_job(&mut self, paths: &[Vec<String>]) {
        self.jobs += 1;
        for lines in paths {
            self.lines += lines.len();
            self.error_lines += lines.iter().filter(|l| l.starts_with("E|")).count();
        }
    }

    pub fn merge(&mut self, other: Compared) {
        self.jobs += other.jobs;
        self.lines += other.lines;
        self.error_lines += other.error_lines;
    }

    /// Asserts that every one of `expected` jobs was compared.
    pub fn assert_all(&self, expected: usize) {
        assert!(expected > 0, "no jobs generated");
        assert_eq!(self.jobs, expected, "jobs compared vs generated");
    }
}

/// Compares jobs with Go line by line; panics with the first difference.
pub fn compare_lines(oracle: &mut Oracle, net: &HdVersions, jobs: &[Job]) -> Compared {
    let go = go_lines(oracle, net, jobs);
    assert_eq!(
        go.len(),
        jobs.len(),
        "Go answered a different number of jobs"
    );
    let rust = par_map(jobs, |j| run_job(j, net));
    let mut compared = Compared::default();
    for ((g, r), job) in go.iter().zip(&rust).zip(jobs) {
        assert_eq!(
            g.len(),
            job.paths.len(),
            "Go answered a different number of paths"
        );
        assert!(g == r, "mismatch: {}", first_difference(g, r, job));
        compared.add_job(r);
    }
    compared
}

/// Compares jobs with Go by digest, in batches, re-asking Go for full
/// lines to report a mismatch. Go and Rust work on a batch concurrently.
pub fn compare_digests(
    oracle: &mut Oracle,
    net: &HdVersions,
    jobs: &[Job],
    batch: usize,
) -> Compared {
    let mut compared = Compared::default();
    for chunk in jobs.chunks(batch.max(1)) {
        let (go, rust) = std::thread::scope(|s| {
            let go = s.spawn(|| go_digests(oracle, net, chunk));
            let rust = par_map(chunk, |j| {
                let lines = run_job(j, net);
                (digest(&lines), lines)
            });
            (go.join().expect("oracle thread"), rust)
        });
        assert_eq!(
            go.len(),
            chunk.len(),
            "Go answered a different number of jobs"
        );
        for (i, (g, (r, lines))) in go.iter().zip(&rust).enumerate() {
            if g != r {
                let job = &chunk[i];
                let gl = go_lines(oracle, net, std::slice::from_ref(job));
                panic!("digest mismatch: {}", first_difference(&gl[0], lines, job));
            }
            compared.add_job(lines);
        }
    }
    compared
}

// ---------------------------------------------------------------------------
// Vector files.
//
// A lines file holds jobs with Go's full lines:
//
//     J <priv hex>:<pub hex> seed <hex>        or  ... keyhex <hex of the UTF-8 key>
//     P <path, or ~ for the empty path>
//     K|... / E|...                            one per line of that path
//
// A digest file holds one job per line:
//
//     D <priv hex>:<pub hex> seed|keyhex <hex> <digest> <paths joined by ','> [# note]
//
// where the paths field `-` stands for the set of the last `S <paths>` line.
//
// Lines starting with '#' are comments.
// ---------------------------------------------------------------------------

/// The `tests/data` directory.
pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// The SHA-256 of a vector file's body: every line that is not a comment
/// (nor empty), each followed by a newline, in order.
pub fn body_sha256(text: &str) -> String {
    let mut h = Sha256::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        h.update(line.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// The number of entries in a vector file body (`J` and `D` lines).
pub fn count_entries(text: &str) -> usize {
    text.lines()
        .filter(|l| l.starts_with("J ") || l.starts_with("D "))
        .count()
}

/// The `# entries:` and `# body-sha256:` header lines for a body.
pub fn integrity_header(body: &str) -> String {
    format!(
        "# entries: {}\n# body-sha256: {}\n",
        count_entries(body),
        body_sha256(body)
    )
}

/// Checks a vector file's integrity headers against its body and returns
/// its entry count. A file missing either header fails.
pub fn verify_integrity(name: &str, text: &str) -> usize {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{name}: no {key:?} header"))
            .to_owned()
    };
    let entries: usize = field("# entries:").parse().expect("entry count");
    assert_eq!(count_entries(text), entries, "{name}: entry count");
    assert_eq!(
        body_sha256(text),
        field("# body-sha256:"),
        "{name}: body hash"
    );
    entries
}

fn net_field(net: &HdVersions) -> String {
    format!("{}:{}", hex(&net.private), hex(&net.public))
}

fn parse_net(s: &str) -> HdVersions {
    let (p, q) = s.split_once(':').expect("net field");
    let p: [u8; 4] = unhex(p).try_into().expect("4 bytes");
    let q: [u8; 4] = unhex(q).try_into().expect("4 bytes");
    HdVersions {
        private: p,
        public: q,
    }
}

fn root_fields(root: &Root) -> String {
    match root {
        Root::Seed(s) => format!("seed {}", hex(s)),
        Root::Key(k) => format!("keyhex {}", hex(k.as_bytes())),
    }
}

fn parse_root(kind: &str, value: &str) -> Root {
    match kind {
        "seed" => Root::Seed(unhex(value)),
        "keyhex" => Root::Key(String::from_utf8(unhex(value)).expect("utf-8 key")),
        _ => panic!("bad root kind {kind}"),
    }
}

fn path_field(p: &str) -> &str {
    if p.is_empty() { "~" } else { p }
}

fn unpath_field(p: &str) -> String {
    if p == "~" {
        String::new()
    } else {
        p.to_owned()
    }
}

/// A job with its expected lines.
pub struct LinesEntry {
    pub net: HdVersions,
    pub job: Job,
    pub lines: Vec<Vec<String>>,
}

pub fn write_lines_entries(out: &mut String, entries: &[LinesEntry]) {
    for e in entries {
        let _ = writeln!(out, "J {} {}", net_field(&e.net), root_fields(&e.job.root));
        for (p, lines) in e.job.paths.iter().zip(&e.lines) {
            let _ = writeln!(out, "P {}", path_field(p));
            for l in lines {
                assert!(!l.contains('\n'), "line with a newline: {l:?}");
                let _ = writeln!(out, "{l}");
            }
        }
    }
}

pub fn read_lines_file(text: &str) -> Vec<LinesEntry> {
    let mut entries: Vec<LinesEntry> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("J ") {
            let f: Vec<&str> = rest.split(' ').collect();
            assert_eq!(f.len(), 3, "bad J line {line:?}");
            entries.push(LinesEntry {
                net: parse_net(f[0]),
                job: Job {
                    root: parse_root(f[1], f[2]),
                    paths: Vec::new(),
                },
                lines: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("P ") {
            let e = entries.last_mut().expect("P before J");
            e.job.paths.push(unpath_field(rest));
            e.lines.push(Vec::new());
        } else if line.starts_with("K|") || line.starts_with("E|") {
            let e = entries.last_mut().expect("line before J");
            e.lines
                .last_mut()
                .expect("line before P")
                .push(line.to_owned());
        } else {
            panic!("bad vector line {line:?}");
        }
    }
    entries
}

/// A job with its expected digest.
pub struct DigestEntry {
    pub net: HdVersions,
    pub job: Job,
    pub digest: String,
    pub note: String,
}

pub fn write_digest_entries(out: &mut String, entries: &[DigestEntry]) {
    let mut current: Option<&Vec<String>> = None;
    for e in entries {
        let joined = |paths: &Vec<String>| {
            paths
                .iter()
                .map(|p| path_field(p))
                .collect::<Vec<_>>()
                .join(",")
        };
        // A path set shared by consecutive entries is written once.
        let shared = entries
            .iter()
            .filter(|o| o.job.paths == e.job.paths)
            .take(2)
            .count()
            > 1;
        let paths_field = if shared {
            if current != Some(&e.job.paths) {
                let _ = writeln!(out, "S {}", joined(&e.job.paths));
                current = Some(&e.job.paths);
            }
            "-".to_owned()
        } else {
            joined(&e.job.paths)
        };
        let _ = write!(
            out,
            "D {} {} {} {}",
            net_field(&e.net),
            root_fields(&e.job.root),
            e.digest,
            paths_field
        );
        if !e.note.is_empty() {
            let _ = write!(out, " # {}", e.note);
        }
        out.push('\n');
    }
}

pub fn read_digest_file(text: &str) -> Vec<DigestEntry> {
    let mut entries = Vec::new();
    let mut shared: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("S ") {
            shared = rest.split(',').map(unpath_field).collect();
            continue;
        }
        let rest = line.strip_prefix("D ").expect("D line");
        let (fields, note) = match rest.split_once(" # ") {
            Some((f, n)) => (f, n.to_owned()),
            None => (rest, String::new()),
        };
        let f: Vec<&str> = fields.split(' ').collect();
        assert_eq!(f.len(), 5, "bad D line {line:?}");
        let paths = if f[4] == "-" {
            shared.clone()
        } else {
            f[4].split(',').map(unpath_field).collect()
        };
        entries.push(DigestEntry {
            net: parse_net(f[0]),
            job: Job {
                root: parse_root(f[1], f[2]),
                paths,
            },
            digest: f[3].to_owned(),
            note,
        });
    }
    entries
}

// ---------------------------------------------------------------------------
// Leading-zero search.
//
// A fast filter for seeds whose private keys on the hardened part of the
// wallet path start with zero bytes: hardened private derivation needs only
// HMAC-SHA512 and a scalar addition, no point multiplication, so millions of
// seeds can be screened. It reimplements the Decred layout to know where the
// zeros fall; it only selects candidates. What is recorded and compared is
// computed by the crate, and every candidate is checked against Go.
// ---------------------------------------------------------------------------

mod filter {
    use dcroxide_dcrec::secp256k1;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha512;

    fn hmac512(key: &[u8], data: &[u8]) -> [u8; 64] {
        let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key).expect("any key length");
        mac.update(data);
        mac.finalize().into_bytes().into()
    }

    /// (full 32-byte key, chain code) of the master, or None if unusable.
    pub fn master(seed: &[u8]) -> Option<([u8; 32], [u8; 32])> {
        let lr = hmac512(b"Bitcoin seed", seed);
        let (k, c) = lr.split_at(32);
        let k: [u8; 32] = k.try_into().ok()?;
        secp256k1::check_scalar(&k).ok()?;
        Some((k, c.try_into().ok()?))
    }

    /// Hardened child of a parent stored as `stored` (left-aligned in the
    /// preimage): (full 32-byte child key, chain code).
    pub fn hardened(stored: &[u8], cc: &[u8; 32], i: u32) -> Option<([u8; 32], [u8; 32])> {
        let mut data = [0u8; 37];
        data[1..1 + stored.len()].copy_from_slice(stored);
        data[33..].copy_from_slice(&i.to_be_bytes());
        let ilr = hmac512(cc, &data);
        let (il, ir) = ilr.split_at(32);
        let il: [u8; 32] = il.try_into().ok()?;
        let sum = *secp256k1::add_private_tweak(&il, stored).ok()?.serialize();
        Some((sum, ir.try_into().ok()?))
    }
}

pub fn leading_zeros(b: &[u8]) -> usize {
    b.iter().take_while(|&&x| x == 0).count()
}

fn strip(b: &[u8; 32]) -> &[u8] {
    &b[leading_zeros(b)..]
}

/// The most leading zero bytes found at each hardened wallet depth
/// (master, purpose, coin type, account) along m/44'/{42',20'}/{0',1'}
/// under both the Decred (stripped) and the standard layout, by the fast
/// filter.
pub fn screen(seed: &[u8]) -> [usize; 4] {
    let mut best = [0usize; 4];
    let Some((mk, mc)) = filter::master(seed) else {
        return best;
    };
    best[0] = leading_zeros(&mk);
    let Some((pk, pc)) = filter::hardened(&mk, &mc, h(44)) else {
        return best;
    };
    best[1] = leading_zeros(&pk);
    for coin in [42, 20] {
        for (pstored, strict) in [(strip(&pk), false), (&pk[..], true)] {
            let Some((ck, cc)) = filter::hardened(pstored, &pc, h(coin)) else {
                continue;
            };
            best[2] = best[2].max(leading_zeros(&ck));
            let cstored = if strict { &ck[..] } else { strip(&ck) };
            for acct in [0, 1] {
                if let Some((ak, _)) = filter::hardened(cstored, &cc, h(acct)) {
                    best[3] = best[3].max(leading_zeros(&ak));
                }
            }
        }
    }
    best
}

/// The paths compared for every leading-zero seed: both coin types, both
/// variants, accounts 0 and 1, the parse split at every hardened depth, a
/// mixed-variant path, and public derivation below the account.
pub fn leading_zero_paths() -> Vec<String> {
    let mut paths = Vec::new();
    for coin in [42u32, 20] {
        let p = h(44);
        let c = h(coin);
        for acct in [0u32, 1] {
            let a = h(acct);
            paths.push(format!("{p}/{c}/{a}/0/0"));
            paths.push(format!("{p}s/{c}s/{a}s/0s/0s"));
            paths.push(format!("{p}r/{c}/{a}/1/7"));
            paths.push(format!("{p}/{c}r/{a}/0/3"));
        }
        paths.push(format!("{p}/{c}/{}r/0/1", h(0)));
        paths.push(format!("{p}/{c}s/{}/0/0", h(0)));
        paths.push(format!("{p}s/{c}/{}s/1/0", h(0)));
        paths.push(format!("{p}/{c}/{}n/0/2", h(0)));
        paths.push(format!("{p}/{c}/{}/{}/{}", h(0), h(0), h(5)));
    }
    paths
}

/// What the crate itself reports along the Decred wallet paths: for each
/// hardened depth (master, purpose, coin, account) the most leading zero
/// bytes of the 32-byte key under either variant, and whether a stripped
/// (shorter than 32) key was stored there by `child`.
pub fn classify(seed: &[u8]) -> ([usize; 4], [bool; 4]) {
    let net = HdVersions::MAINNET;
    let mut zeros = [0usize; 4];
    let mut stripped = [false; 4];
    let Ok(m) = ExtendedKey::new_master(seed, &net) else {
        return (zeros, stripped);
    };
    let full_zeros = |k: &ExtendedKey| {
        let b = k.serialized_priv_key().expect("private");
        32 - b.len() + leading_zeros(b)
    };
    zeros[0] = full_zeros(&m);
    stripped[0] = m.serialized_priv_key().expect("private").len() < 32;
    for strict in [false, true] {
        let step = |k: &ExtendedKey, i: u32| {
            if strict {
                k.child_bip32_std(i)
            } else {
                k.child(i)
            }
        };
        let Ok(p) = step(&m, h(44)) else { continue };
        zeros[1] = zeros[1].max(full_zeros(&p));
        stripped[1] |= p.serialized_priv_key().expect("private").len() < 32;
        for coin in [42, 20] {
            let Ok(c) = step(&p, h(coin)) else { continue };
            zeros[2] = zeros[2].max(full_zeros(&c));
            stripped[2] |= c.serialized_priv_key().expect("private").len() < 32;
            for acct in [0, 1] {
                let Ok(a) = step(&c, h(acct)) else { continue };
                zeros[3] = zeros[3].max(full_zeros(&a));
                stripped[3] |= a.serialized_priv_key().expect("private").len() < 32;
            }
        }
    }
    (zeros, stripped)
}

/// Runs the fast filter over `workers` deterministic streams of `per_worker`
/// 32-byte seeds each and returns every seed with a leading zero byte
/// somewhere on the hardened wallet path, with the filter's counts.
pub fn search(base: u64, workers: u64, per_worker: u64) -> Vec<(Vec<u8>, [usize; 4])> {
    let ws: Vec<u64> = (0..workers).collect();
    let found = par_map(&ws, |&w| {
        let mut rng = purse_testutil::SplitMix64(base ^ w.wrapping_mul(0xd6e8_feb8_6659_fd93));
        let mut out = Vec::new();
        let mut seed = [0u8; 32];
        for _ in 0..per_worker {
            rng.fill(&mut seed);
            let z = screen(&seed);
            if z.iter().any(|&n| n > 0) {
                out.push((seed.to_vec(), z));
            }
        }
        out
    });
    found.into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// Random wallet paths.
// ---------------------------------------------------------------------------

/// A random seed: usually 32 bytes, one time in eight any allowed length.
pub fn random_seed(rng: &mut purse_testutil::SplitMix64) -> Vec<u8> {
    let len = if rng.below(8) == 0 {
        16 + rng.below(49) as usize
    } else {
        32
    };
    rng.bytes(len)
}

/// Paths for one random seed: m/44'/{42',20'}/acct'/branch/index with
/// `child`, with `child_bip32_std`, re-parsed at the coin type key (how
/// dcrwallet derives accounts after the first), neutered at the account
/// (public derivation below it), and with a random mix of variants and
/// re-parses.
pub fn random_wallet_paths(rng: &mut purse_testutil::SplitMix64) -> Vec<String> {
    let mut paths = Vec::with_capacity(10);
    for coin in [42u32, 20] {
        let acct = if rng.below(8) == 0 {
            rng.below(1 << 31) as u32
        } else {
            rng.below(4) as u32
        };
        let branch = rng.below(2) as u32;
        let index = if rng.below(8) == 0 {
            rng.below(1 << 31) as u32
        } else {
            rng.below(100) as u32
        };
        let (p, c, a) = (h(44), h(coin), h(acct));
        paths.push(format!("{p}/{c}/{a}/{branch}/{index}"));
        paths.push(format!("{p}s/{c}s/{a}s/{branch}s/{index}s"));
        paths.push(format!("{p}/{c}r/{a}/{branch}/{index}"));
        paths.push(format!("{p}/{c}/{a}n/{branch}/{index}"));
        let mixed: Vec<String> = [p, c, a, branch, index]
            .iter()
            .map(|i| {
                let flags = ["", "s", "r", "sr"][rng.below(4) as usize];
                format!("{i}{flags}")
            })
            .collect();
        paths.push(mixed.join("/"));
    }
    paths
}

/// `count` random-seed jobs.
pub fn random_jobs(rng: &mut purse_testutil::SplitMix64, count: usize) -> Vec<Job> {
    (0..count)
        .map(|_| {
            let seed = random_seed(rng);
            Job::seed(&seed, random_wallet_paths(rng))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Parse corpus.
// ---------------------------------------------------------------------------

/// The base58 string of a 78-byte payload with a correct checksum.
pub fn encode_payload(payload: &[u8; 78]) -> String {
    let mut b = payload.to_vec();
    b.extend_from_slice(&dcroxide_crypto::blake256::sum256d(payload)[..4]);
    dcroxide_base58::encode(&b)
}

/// The 78-byte payload of a well-formed extended key string.
pub fn decode_payload(s: &str) -> [u8; 78] {
    dcroxide_base58::decode(s)[..78]
        .try_into()
        .expect("82-byte key")
}

const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// The secp256k1 group order, big-endian.
pub const N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];

/// The secp256k1 field prime, big-endian.
pub const P: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xff, 0xff, 0xfc, 0x2f,
];

fn add_small(mut v: [u8; 32], d: i8) -> [u8; 32] {
    // Big-endian add or subtract of a small value, wrapping (test data).
    let mut carry = i16::from(d);
    for b in v.iter_mut().rev() {
        let x = i16::from(*b) + carry;
        *b = x.rem_euclid(256) as u8;
        carry = x.div_euclid(256);
        if carry == 0 {
            break;
        }
    }
    v
}

/// A seed of dcrd's BIP32 test vector 4, whose m/0' private key starts
/// with a zero byte (a stripped key under `child`).
pub const VEC4_SEED: &str = "3ddd5602285899a946114506157c7997e5444528f3003f6134712147db19b678";

/// A deterministic corpus of extended key strings, each with the network
/// it is parsed with: valid keys of every network, keys of one network
/// parsed with another, character mutations, length boundaries, invalid
/// characters, and well-checksummed payloads with every field pushed to
/// its edges.
pub fn parse_corpus(rng: &mut purse_testutil::SplitMix64) -> Vec<(HdVersions, String)> {
    let mut out: Vec<(HdVersions, String)> = Vec::new();
    let main = HdVersions::MAINNET;

    // Valid keys, on every network.
    let mut valid: Vec<(HdVersions, String)> = Vec::new();
    for (_, net) in NETS {
        for _ in 0..2 {
            let seed = rng.bytes(32);
            let m = ExtendedKey::new_master(&seed, &net).expect("master");
            let c = m.child(h(44)).and_then(|k| k.child(7)).expect("child");
            for k in [&m, &c] {
                valid.push((net, k.string().to_string()));
                valid.push((net, k.neuter().string().to_string()));
            }
        }
    }
    // A stripped key (m/0' of vector 4) and its public key.
    let v4 = ExtendedKey::new_master(&unhex(VEC4_SEED), &main)
        .and_then(|m| m.child(h(0)))
        .expect("vector 4");
    valid.push((main, v4.string().to_string()));
    valid.push((main, v4.neuter().string().to_string()));
    out.extend(valid.iter().cloned());

    // Parsed with the wrong network.
    for (i, (net, s)) in valid.iter().enumerate().take(16) {
        let other = NETS[(i + 1) % 4].1;
        if other != *net {
            out.push((other, s.clone()));
        }
    }

    // Character mutations.
    for (net, s) in valid.iter().take(12) {
        let bytes = s.as_bytes();
        for _ in 0..4 {
            let mut m = bytes.to_vec();
            let i = rng.below(m.len() as u64) as usize;
            m[i] = ALPHABET[rng.below(58) as usize];
            out.push((*net, String::from_utf8(m).expect("ascii")));
        }
        for bad in ["0", "O", "I", "l", " ", "+", "\u{e9}"] {
            let i = rng.below(bytes.len() as u64) as usize;
            out.push((*net, format!("{}{bad}{}", &s[..i], &s[i + 1..])));
        }
        let i = rng.below(bytes.len() as u64) as usize;
        out.push((*net, format!("{}{}", &s[..i], &s[i + 1..])));
        let i = rng.below(bytes.len() as u64) as usize;
        let c = ALPHABET[rng.below(58) as usize] as char;
        out.push((*net, format!("{}{c}{}", &s[..i], &s[i..])));
        out.push((*net, format!("{s}1")));
        out.push((*net, format!("1{s}")));
        out.push((*net, s[..s.len() - 1].to_owned()));
        out.push((*net, format!("{s} ")));
    }

    // Lengths and characters.
    for n in [0usize, 1, 4, 81, 82, 83, 110, 111, 112, 113, 114, 120, 200] {
        out.push((main, "1".repeat(n)));
        out.push((main, "z".repeat(n)));
    }
    for n in [56usize, 57] {
        out.push((main, "\u{e9}".repeat(n)));
    }
    out.push((main, "\u{20ac}".repeat(38)));
    out.push((main, "dpub1234".to_owned()));
    out.push((main, "zeroed extended key".to_owned()));

    // Well-checksummed payloads.
    let priv_base = decode_payload(&valid[0].1);
    let pub_base = decode_payload(&valid[1].1);
    let versions: Vec<[u8; 4]> = vec![
        main.private,
        main.public,
        HdVersions::TESTNET3.private,
        HdVersions::TESTNET3.public,
        HdVersions::SIMNET.private,
        HdVersions::REGNET.public,
        [0; 4],
        [0xff; 4],
        [0x02, 0xfd, 0xa4, 0xe9],
    ];
    for v in &versions {
        for base in [priv_base, pub_base] {
            let mut p = base;
            p[..4].copy_from_slice(v);
            out.push((main, encode_payload(&p)));
        }
    }

    let mut priv_values: Vec<[u8; 32]> = vec![
        [0; 32],
        add_small([0; 32], 1),
        add_small(N, -1),
        N,
        add_small(N, 1),
        [0xff; 32],
        P,
    ];
    let mut lz = [0u8; 32];
    rng.fill(&mut lz[2..]);
    priv_values.push(lz);
    let mut lz1 = [0u8; 32];
    rng.fill(&mut lz1[1..]);
    priv_values.push(lz1);
    for v in &priv_values {
        for ver in [main.private, main.public] {
            let mut p = priv_base;
            p[..4].copy_from_slice(&ver);
            p[45] = 0;
            p[46..].copy_from_slice(v);
            out.push((main, encode_payload(&p)));
        }
    }

    let valid_x: [u8; 32] = pub_base[46..].try_into().expect("x");
    let mut xs: Vec<[u8; 32]> = vec![
        valid_x,
        [0; 32],
        add_small([0; 32], 1),
        add_small(P, -1),
        P,
        add_small(P, 1),
        [0xff; 32],
    ];
    for _ in 0..6 {
        let mut x = [0u8; 32];
        rng.fill(&mut x);
        xs.push(x);
    }
    for x in &xs {
        for prefix in [0x02u8, 0x03] {
            let mut p = pub_base;
            p[45] = prefix;
            p[46..].copy_from_slice(x);
            out.push((main, encode_payload(&p)));
        }
    }
    for prefix in [0x01u8, 0x04, 0x05, 0x06, 0x07, 0x0a, 0x80, 0xff] {
        for ver in [main.public, main.private] {
            let mut p = pub_base;
            p[..4].copy_from_slice(&ver);
            p[45] = prefix;
            out.push((main, encode_payload(&p)));
        }
    }

    for base in [priv_base, pub_base] {
        for depth in [0u8, 1, 254, 255] {
            for child in [0u32, 1, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff] {
                let mut p = base;
                p[4] = depth;
                p[9..13].copy_from_slice(&child.to_be_bytes());
                if child == 1 {
                    p[5..9].copy_from_slice(&[0xff; 4]);
                }
                out.push((main, encode_payload(&p)));
            }
        }
    }
    out
}

/// Paths derived from every parsed corpus key.
pub fn corpus_paths() -> Vec<String> {
    vec![
        String::new(),
        "0/1".to_owned(),
        h(0).to_string(),
        format!("{}s/0s", h(1)),
        "n/5".to_owned(),
        "z".to_owned(),
        "z/0".to_owned(),
        format!("z/{}", h(0)),
        "zr".to_owned(),
        "0r/1".to_owned(),
    ]
}
