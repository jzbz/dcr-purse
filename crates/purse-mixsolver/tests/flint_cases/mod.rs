// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! What the FLINT differential (`tests/oracle.rs`) and the replay of its
//! frozen vectors (`tests/frozen.rs`) share: the corpus, Go's outcome as
//! the oracle reports it, and the rule that compares it with this crate's.
//!
//! # Go's outcome and the client's action
//!
//! The oracle runs `decred.org/cspp/v2@v2.4.0/solver.Roots(coeffs,
//! mixing.F)` and applies `Client.roots`'s rule to the answer
//! (`mixclient/client.go:2162-2187`, transcribed in
//! `tools/oracle/cmd/mixsolver/flint.go` `clientRule`). The answer FLINT
//! gives is one value per monic irreducible factor, −(constant coefficient)
//! mod F, as a canonical `big.Int`, **in FLINT's factor order**, which is
//! not ascending (`flint_order`); the client sorts by `big.Int.Cmp` and
//! publishes each root's minimal big-endian bytes (`fp_roots`, with the
//! root 0 as no bytes at all) in one factored-polynomial message per local
//! peer. Any error, or any count other than the degree, sets
//! `triggeredBlame` on every local peer instead; only
//! `ErrSolverProcessExited` (class "exited") takes the fallback.
//!
//! # The comparison rule
//!
//! Each side is reduced to the client's action ([`Action`]): publish these
//! bytes, triggered blame on every local peer, or the fallback. For this
//! crate, `Ok(roots)` publishes the minimal bytes of the roots **in the
//! order returned** (so the comparison also proves the crate returns them
//! ascending, where Go's client sorts), and every error is triggered blame
//! (ADR-0010, Decision 1). The two actions must be equal, with exactly two
//! exceptions, the documented divergences: a non-canonical coefficient
//! and a degree above [`MAX_DEGREE`], where FLINT reduces or factors what
//! this crate refuses. Neither is reachable from the client. Beyond the
//! action, each case states what Go must answer ([`Expect`]) and the crate
//! must give its specific [`Error`], so that the classes are pinned on both
//! sides.

#![allow(dead_code, reason = "the two test crates use different parts")]

use std::collections::BTreeMap;

use purse_mixsolver::{Error, MAX_DEGREE, roots};
use purse_testutil::SplitMix64;
use purse_testutil::serde_json::{self, Value, json};

use crate::common::model::{self, P};
use crate::common::{
    Lead, distinct_roots, hex128, parse_hex128, polynomial, random_fe, random_nonzero,
};

/// The environment variable that turns every FLINT skip into a failure: set
/// on the one CI leg that installs FLINT.
pub const REQUIRE_FLINT: &str = "PURSE_REQUIRE_FLINT";

/// The frozen vectors, relative to the crate.
pub const FROZEN_PATH: &str = "tests/data/flint_vectors.jsonl";

/// The fixed seed of the frozen vectors' inputs.
pub const FROZEN_SEED: u64 = 0x666c_696e_7476_0002;

/// Whether FLINT is required.
pub fn flint_required() -> bool {
    std::env::var_os(REQUIRE_FLINT).is_some()
}

/// What the crate's validation makes of a category's inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Canonical, at least two coefficients, nonzero leading coefficient,
    /// degree at most [`MAX_DEGREE`]: every input the client can build.
    WellFormed,
    /// Fewer than two coefficients.
    TooFew,
    /// A zero leading coefficient, the zero polynomial included.
    ZeroLead,
    /// A coefficient at least F.
    NonCanonical,
    /// More than [`MAX_DEGREE`] + 1 coefficients.
    AboveMax,
}

/// What Go must answer for a case (pinned against FLINT by these tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Accepted: `degree` distinct roots.
    Accept,
    /// The repeated-root error.
    Repeated,
    /// No error and this many values, one per factor: fewer than the degree.
    Count(u64),
    /// The error "too few coefficients".
    TooFew,
    /// Rejected, by whatever class.
    Reject,
    /// Anything: random input, where only agreement is required.
    Any,
}

/// The categories, in corpus order, with their kind. `parse_frozen_line`
/// accepts exactly these names.
pub const CATEGORIES: [(&str, Kind); 21] = [
    ("probe", Kind::WellFormed),
    ("degree-1", Kind::WellFormed),
    ("split", Kind::WellFormed),
    ("non-monic", Kind::WellFormed),
    ("monic", Kind::WellFormed),
    ("edge-roots", Kind::WellFormed),
    ("max-degree", Kind::WellFormed),
    ("repeated", Kind::WellFormed),
    ("repeated-zero", Kind::WellFormed),
    ("irreducible-quadratic", Kind::WellFormed),
    ("irreducible-cubic", Kind::WellFormed),
    ("mixture", Kind::WellFormed),
    ("random", Kind::WellFormed),
    ("too-few", Kind::TooFew),
    ("zero-lead", Kind::ZeroLead),
    ("zero-lead-constant", Kind::ZeroLead),
    ("zero-polynomial", Kind::ZeroLead),
    ("non-canonical", Kind::NonCanonical),
    ("non-canonical-rejected", Kind::NonCanonical),
    ("above-max", Kind::AboveMax),
    ("above-max-rejected", Kind::AboveMax),
];

fn kind_of(category: &str) -> Kind {
    CATEGORIES
        .iter()
        .find(|(name, _)| *name == category)
        .map(|&(_, kind)| kind)
        .unwrap_or_else(|| panic!("unknown category {category:?}"))
}

/// One input, its category and what Go must answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub category: &'static str,
    pub coeffs: Vec<u128>,
    pub expect: Expect,
}

/// Go's outcome for one input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoOutcome {
    /// "ok", "count", "repeated", "error" or "exited".
    pub class: String,
    /// The roots, sorted as the client sorts them, when accepted.
    pub roots: Vec<u128>,
    /// What the client's factored-polynomial message carries when accepted:
    /// the hex of each sorted root's `big.Int.Bytes()`.
    pub fp_roots: Vec<String>,
    /// The values in FLINT's factor order, when Roots returned no error.
    pub flint_order: Vec<u128>,
    /// len(values) when Roots returned no error.
    pub count: u64,
    /// The error text, when Roots returned an error.
    pub error: String,
}

fn hex_array(v: &Value, key: &str) -> Vec<u128> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key}: not an array in {v}"))
        .iter()
        .map(|s| parse_hex128(s.as_str().expect("hex string")))
        .collect()
}

impl GoOutcome {
    /// Parses the oracle's "roots" answer (or a frozen entry's "go").
    pub fn from_json(v: &Value) -> GoOutcome {
        GoOutcome {
            class: v["class"].as_str().expect("class").to_owned(),
            roots: hex_array(v, "roots"),
            fp_roots: v["fp_roots"]
                .as_array()
                .expect("fp_roots")
                .iter()
                .map(|s| s.as_str().expect("hex").to_owned())
                .collect(),
            flint_order: hex_array(v, "flint_order"),
            count: v["count"].as_u64().expect("count"),
            error: v["error"].as_str().expect("error").to_owned(),
        }
    }

    /// The outcome as a frozen entry stores it.
    pub fn to_json(&self) -> Value {
        let hexes = |v: &[u128]| v.iter().map(|&x| hex128(x)).collect::<Vec<_>>();
        json!({
            "class": self.class,
            "roots": hexes(&self.roots),
            "fp_roots": self.fp_roots,
            "flint_order": hexes(&self.flint_order),
            "count": self.count,
            "error": self.error,
        })
    }

    pub fn accepted(&self) -> bool {
        self.class == "ok"
    }
}

/// What the mixing client does with a solver's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Every local peer's factored-polynomial message carries these root
    /// bytes, in this order (`client.go:2181-2195`).
    Publish(Vec<String>),
    /// `triggeredBlame` on every local peer (`client.go:2173-2179`).
    TriggeredBlame,
    /// `ErrSolverProcessExited`: stop solving, wait for others' FPs
    /// (`client.go:2163-2172`).
    Fallback,
}

/// The hex of `v`'s big-endian bytes without leading zeros: `big.Int.Bytes`
/// (no bytes at all for 0).
pub fn minimal_bytes_hex(v: u128) -> String {
    let bytes = v.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    purse_testutil::hex(&bytes[first..])
}

/// The client's action on Go's outcome.
pub fn go_action(go: &GoOutcome) -> Action {
    match go.class.as_str() {
        "ok" => Action::Publish(go.fp_roots.clone()),
        "count" | "repeated" | "error" => Action::TriggeredBlame,
        "exited" => Action::Fallback,
        other => panic!("unknown class {other:?}"),
    }
}

/// The client's action on this crate's answer (ADR-0010, Decision 1: every
/// error takes the triggered-blame path).
pub fn rust_action(rust: &Result<Vec<u128>, Error>) -> Action {
    match rust {
        Ok(r) => Action::Publish(r.iter().map(|&x| minimal_bytes_hex(x)).collect()),
        Err(_) => Action::TriggeredBlame,
    }
}

/// How a case came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Both accept, with the same published bytes.
    SameAccept,
    /// Both reject: the same triggered blame.
    SameReject,
    /// A documented divergence: Go accepts what the crate refuses.
    Divergence,
}

/// The oracle's request for `coeffs`.
pub fn coeffs_json(coeffs: &[u128]) -> Value {
    json!({ "coeffs": coeffs.iter().map(|&c| hex128(c)).collect::<Vec<_>>() })
}

/// A random quadratic non-residue.
fn non_residue(rng: &mut SplitMix64) -> u128 {
    loop {
        let a = random_nonzero(rng);
        if !model::is_square(a) {
            return a;
        }
    }
}

/// A random non-cube (F ≡ 1 mod 3).
fn non_cube(rng: &mut SplitMix64) -> u128 {
    loop {
        let a = random_nonzero(rng);
        if model::pow_mod(a, (P - 1) / 3) != 1 {
            return a;
        }
    }
}

/// x² − a for a random non-residue a: irreducible.
fn irreducible_quadratic(rng: &mut SplitMix64) -> Vec<u128> {
    vec![model::neg_mod(non_residue(rng)), 0, 1]
}

/// x³ − b for a random non-cube b: irreducible.
fn irreducible_cubic(rng: &mut SplitMix64) -> Vec<u128> {
    vec![model::neg_mod(non_cube(rng)), 0, 0, 1]
}

/// What a corpus contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Multiplies the random instances.
    pub scale: usize,
    /// Adds degrees 200 to 1024 (slow without optimisation).
    pub heavy: bool,
}

/// The frozen vectors' plan: every category but `max-degree`, light enough
/// to replay in a debug build on every platform.
pub const FROZEN_PLAN: Plan = Plan {
    scale: 1,
    heavy: false,
};

/// The corpus. Every category of [`CATEGORIES`] appears, `max-degree` only
/// under `plan.heavy`.
pub fn cases(rng: &mut SplitMix64, plan: Plan) -> Vec<Case> {
    let Plan { scale, heavy } = plan;
    let mut out = Vec::new();
    let mut push = |category: &'static str, coeffs: Vec<u128>, expect: Expect| {
        out.push(Case {
            category,
            coeffs,
            expect,
        });
    };
    let leads = [Lead::Client, Lead::One, Lead::Random];

    // dcrwallet's start-up probe (testsolver.go:25-29).
    push("probe", vec![1, 2], Expect::Accept);
    for c in [[0, 1], [0, P - 1], [P - 1, 1], [1, P - 1]] {
        push("degree-1", c.to_vec(), Expect::Accept);
    }
    for _ in 0..8 * scale {
        push(
            "degree-1",
            vec![random_fe(rng), random_nonzero(rng)],
            Expect::Accept,
        );
    }
    let mut split_plan = vec![
        (2usize, 6usize),
        (3, 6),
        (4, 6),
        (5, 6),
        (6, 6),
        (7, 6),
        (8, 6),
        (12, 2),
        (16, 2),
        (31, 2),
        (32, 2),
        (33, 2),
        (64, 2),
        (100, 1),
        (128, 1),
    ];
    if heavy {
        split_plan.extend([(200, 1), (256, 1), (512, 1)]);
    }
    for (n, reps) in split_plan {
        for _ in 0..reps * scale {
            let r = distinct_roots(rng, n, &[]);
            push("split", polynomial(rng, &r, Lead::Client), Expect::Accept);
        }
    }
    for n in [2usize, 3, 7, 30] {
        for _ in 0..scale {
            let r = distinct_roots(rng, n, &[]);
            push(
                "non-monic",
                polynomial(rng, &r, Lead::Random),
                Expect::Accept,
            );
            let r = distinct_roots(rng, n, &[]);
            push("monic", polynomial(rng, &r, Lead::One), Expect::Accept);
        }
    }
    // Roots at the field's edges and of every byte length, the root 0 (no
    // bytes in the client's message) and 0x01 and 0x0100 among them.
    let edge_sets: [&[u128]; 8] = [
        &[0],
        &[P - 1],
        &[0, P - 1],
        &[0, 1],
        &[P - 2, P - 1],
        &[0, 1, 2, 3, 4, 5, 6, 7],
        &[P - 1, P - 2, P - 3, P - 4, 0, 1, 2],
        &[
            0xff,
            0x100,
            1 << 64,
            (1 << 64) - 1,
            1 << 120,
            1 << 126,
            (1 << 126) - 1,
        ],
    ];
    for (i, set) in edge_sets.iter().enumerate() {
        for extra in [0usize, 3] {
            let r = distinct_roots(rng, set.len() + extra, set);
            push(
                "edge-roots",
                polynomial(rng, &r, leads[i % leads.len()]),
                Expect::Accept,
            );
        }
    }
    if heavy {
        // The client's largest degree, mixing.MaxMtot.
        let r = distinct_roots(rng, MAX_DEGREE, &[0, P - 1]);
        push(
            "max-degree",
            polynomial(rng, &r, Lead::Client),
            Expect::Accept,
        );
        let mut c: Vec<u128> = (0..MAX_DEGREE).map(|_| random_fe(rng)).collect();
        c.push(P - 1);
        push("max-degree", c, Expect::Any);
    }
    for n in [2usize, 3, 5, 12] {
        for _ in 0..scale {
            // One double root.
            let mut r = distinct_roots(rng, n - 1, &[]);
            r.push(r[0]);
            push(
                "repeated",
                polynomial(rng, &r, Lead::Client),
                Expect::Repeated,
            );
        }
    }
    for _ in 0..scale {
        // A triple root, and every root doubled.
        let r = distinct_roots(rng, 2, &[]);
        push(
            "repeated",
            polynomial(rng, &[r[0], r[0], r[0], r[1]], Lead::Client),
            Expect::Repeated,
        );
        let r = distinct_roots(rng, 3, &[]);
        let doubled: Vec<u128> = r.iter().flat_map(|&x| [x, x]).collect();
        push(
            "repeated",
            polynomial(rng, &doubled, Lead::Random),
            Expect::Repeated,
        );
    }
    // x^k · (distinct linear factors), k ≥ 2: the constant and the linear
    // coefficients are zero; the root 0 is repeated.
    for (k, extra) in [(2usize, 0usize), (2, 3), (3, 1), (5, 4)] {
        let mut r = vec![0u128; k];
        r.extend(
            distinct_roots(rng, extra, &[0])
                .into_iter()
                .filter(|&x| x != 0),
        );
        push(
            "repeated-zero",
            polynomial(rng, &r, Lead::Client),
            Expect::Repeated,
        );
    }
    for extra in [0usize, 1, 3, 10] {
        for _ in 0..scale {
            let lin = model::expand(&distinct_roots(rng, extra, &[]));
            let quad = irreducible_quadratic(rng);
            let c = model::scale(&model::poly_mul(&quad, &lin), P - 1);
            push("irreducible-quadratic", c, Expect::Count(extra as u64 + 1));
            let lin = model::expand(&distinct_roots(rng, extra, &[]));
            let cubic = irreducible_cubic(rng);
            let c = model::scale(&model::poly_mul(&cubic, &lin), P - 1);
            push("irreducible-cubic", c, Expect::Count(extra as u64 + 1));
        }
    }
    for _ in 0..scale {
        let q = irreducible_quadratic(rng);
        let q2 = loop {
            let q2 = irreducible_quadratic(rng);
            if q2 != q {
                break q2;
            }
        };
        let cu = irreducible_cubic(rng);
        let r = distinct_roots(rng, 3, &[]);
        let lin = |roots: &[u128]| model::expand(roots);
        let mul = |a: &[u128], b: &[u128]| model::poly_mul(a, b);
        let client = |c: Vec<u128>| model::scale(&c, P - 1);
        // Distinct irreducible factors of degrees 2 and 3, with 0 or 2
        // linear factors; two distinct quadratics and a linear factor.
        push("mixture", client(mul(&q, &cu)), Expect::Count(2));
        push(
            "mixture",
            client(mul(&mul(&q, &cu), &lin(&r[..2]))),
            Expect::Count(4),
        );
        push(
            "mixture",
            client(mul(&mul(&q, &q2), &lin(&r[..1]))),
            Expect::Count(3),
        );
        // A repeated irreducible quadratic; a repeated root beside an
        // irreducible quadratic; a triple root beside an irreducible cubic.
        push(
            "mixture",
            client(mul(&mul(&q, &q), &lin(&r[..1]))),
            Expect::Repeated,
        );
        push(
            "mixture",
            client(mul(&q, &lin(&[r[0], r[0], r[1]]))),
            Expect::Repeated,
        );
        push(
            "mixture",
            client(mul(&cu, &lin(&[r[2], r[2], r[2]]))),
            Expect::Repeated,
        );
    }
    for n in [2usize, 3, 4, 5, 8, 16, 32] {
        for i in 0..2 * scale {
            let mut c: Vec<u128> = (0..n).map(|_| random_fe(rng)).collect();
            c.push(if i % 2 == 0 {
                P - 1
            } else {
                random_nonzero(rng)
            });
            push("random", c, Expect::Any);
        }
    }

    // Malformed: unreachable from the client.
    for c in [vec![], vec![5], vec![0], vec![P - 1], vec![u128::MAX]] {
        push("too-few", c, Expect::TooFew);
    }
    for n in [1usize, 2, 5] {
        let r = distinct_roots(rng, n, &[]);
        let mut c = polynomial(rng, &r, Lead::Client);
        c.push(0);
        // FLINT factors the shorter polynomial: n values, not n + 1.
        push("zero-lead", c.clone(), Expect::Count(n as u64));
        c.push(0);
        push("zero-lead", c, Expect::Count(n as u64));
    }
    for c in [vec![5, 0], vec![5, 0, 0], vec![P - 1, 0], vec![1, 0, 0, 0]] {
        push("zero-lead-constant", c, Expect::Count(0));
    }
    for len in [2usize, 3, 9] {
        push("zero-polynomial", vec![0; len], Expect::Count(0));
    }
    // A coefficient raised by F, or another value at least F that FLINT
    // reduces (2^127 ≡ 1, 2^128 − 1 ≡ 1, F ≡ 0): congruent to a polynomial
    // that splits.
    for n in [1usize, 2, 6, 20] {
        for _ in 0..scale {
            let r = distinct_roots(rng, n, &[]);
            let mut c = polynomial(rng, &r, Lead::Client);
            let i = rng.below(c.len() as u64) as usize;
            c[i] += P;
            push("non-canonical", c, Expect::Accept);
        }
    }
    let r = distinct_roots(rng, 3, &[]);
    let mut c = polynomial(rng, &r, Lead::Client);
    *c.last_mut().expect("lead") += P;
    push("non-canonical", c, Expect::Accept);
    for c in [[u128::MAX, 1], [1 << 127, 1], [P, 1], [1, 1 << 127]] {
        push("non-canonical", c.to_vec(), Expect::Accept);
    }
    // Values at least F in polynomials that are refused once reduced: a
    // leading coefficient F (≡ 0), and an irreducible factor.
    let r = distinct_roots(rng, 4, &[]);
    let mut c = polynomial(rng, &r, Lead::Client);
    *c.last_mut().expect("lead") = P;
    push("non-canonical-rejected", c, Expect::Reject);
    push("non-canonical-rejected", vec![1, P], Expect::Count(0));
    let lin = model::expand(&distinct_roots(rng, 2, &[]));
    let mut c = model::scale(&model::poly_mul(&irreducible_quadratic(rng), &lin), P - 1);
    c[1] += P;
    push("non-canonical-rejected", c, Expect::Count(3));
    // One degree above the client's maximum: a polynomial that splits (its
    // roots 0..=1024, small so that the frozen entry stays small), and a
    // random one.
    let r: Vec<u128> = (0..=MAX_DEGREE as u128).collect();
    push(
        "above-max",
        polynomial(rng, &r, Lead::Client),
        Expect::Accept,
    );
    let mut c: Vec<u128> = (0..=MAX_DEGREE).map(|_| random_fe(rng)).collect();
    c.push(P - 1);
    push("above-max-rejected", c, Expect::Reject);
    out
}

/// The crate's specific refusal for a case of `kind`.
fn expected_error(kind: Kind, coeffs: &[u128]) -> Error {
    match kind {
        Kind::WellFormed => Error::NotSplit,
        Kind::TooFew => Error::TooFewCoefficients { len: coeffs.len() },
        Kind::ZeroLead => Error::ZeroLeadingCoefficient,
        Kind::NonCanonical => Error::NonCanonical {
            index: coeffs
                .iter()
                .position(|&c| c >= P)
                .expect("a non-canonical case has a coefficient at least F"),
        },
        Kind::AboveMax => Error::DegreeTooLarge {
            degree: coeffs.len() - 1,
        },
    }
}

/// Checks Go's outcome against the case's expectation and its own
/// consistency (the oracle's transcription of the client's rule).
fn check_go(case: &Case, go: &GoOutcome, what: &str) {
    let degree = case.coeffs.len().saturating_sub(1) as u64;
    match go.class.as_str() {
        "ok" => {
            assert_eq!(go.count, degree, "{what}: count");
            assert!(
                go.roots.windows(2).all(|w| w[0] < w[1]),
                "{what}: roots not strictly ascending"
            );
            assert!(
                go.roots.iter().all(|&r| r < P),
                "{what}: non-canonical root"
            );
            let fp: Vec<String> = go.roots.iter().map(|&r| minimal_bytes_hex(r)).collect();
            assert_eq!(go.fp_roots, fp, "{what}: fp_roots are not the roots' bytes");
            let mut order = go.flint_order.clone();
            order.sort_unstable();
            assert_eq!(
                order, go.roots,
                "{what}: FLINT's order is not a permutation"
            );
        }
        "count" => {
            assert_ne!(go.count, degree, "{what}: count");
            assert_eq!(go.count, go.flint_order.len() as u64, "{what}: count");
            assert!(go.roots.is_empty() && go.fp_roots.is_empty(), "{what}");
        }
        "repeated" => assert_eq!(go.error, "repeated roots", "{what}"),
        "error" => assert!(!go.error.is_empty(), "{what}"),
        "exited" => panic!("{what}: the solver process exited: {go:?}"),
        other => panic!("{what}: unknown class {other:?}"),
    }
    let ok = match case.expect {
        Expect::Accept => go.class == "ok",
        Expect::Repeated => go.class == "repeated",
        Expect::Count(n) => go.class == "count" && go.count == n,
        Expect::TooFew => go.class == "error" && go.error == "too few coefficients",
        Expect::Reject => go.class != "ok",
        Expect::Any => true,
    };
    assert!(ok, "{what}: expected {:?} of Go, got {go:?}", case.expect);
}

/// Compares this crate's answer for `case` with Go's outcome by the rule in
/// the module documentation; panics on anything the rule forbids. Returns
/// the verdict and the crate's answer.
pub fn check(case: &Case, go: &GoOutcome) -> (Verdict, Result<Vec<u128>, Error>) {
    let c = &case.coeffs;
    let kind = kind_of(case.category);
    let what = format!("{} (degree {})", case.category, c.len().saturating_sub(1));
    check_go(case, go, &what);
    let rust = roots(c);
    if let Err(e) = &rust {
        assert_eq!(*e, expected_error(kind, c), "{what}: Go {go:?}");
    }
    let (go_act, rust_act) = (go_action(go), rust_action(&rust));
    if go_act == rust_act {
        let verdict = if rust.is_ok() {
            Verdict::SameAccept
        } else {
            Verdict::SameReject
        };
        return (verdict, rust);
    }
    // The only allowed difference: FLINT reduces a non-canonical input, or
    // factors one above the maximum degree, and accepts it; this crate
    // refuses (crate documentation, "Divergences").
    assert!(
        matches!(kind, Kind::NonCanonical | Kind::AboveMax) && go.accepted() && rust.is_err(),
        "{what}: the client's action differs: Go {go_act:?}, crate {rust_act:?} ({rust:?})"
    );
    (Verdict::Divergence, rust)
}

/// Per-category counts of a run.
#[derive(Debug, Default)]
pub struct Tally {
    rows: Vec<(&'static str, BTreeMap<String, usize>)>,
}

impl Tally {
    /// Records one case.
    pub fn add(
        &mut self,
        case: &Case,
        go: &GoOutcome,
        (verdict, rust): &(Verdict, Result<Vec<u128>, Error>),
    ) {
        let rust = match rust {
            Ok(_) => "Ok".to_owned(),
            Err(e) => format!("{e:?}")
                .split([' ', '{'])
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
        let row = match self.rows.iter_mut().find(|(c, _)| *c == case.category) {
            Some((_, row)) => row,
            None => {
                self.rows.push((case.category, BTreeMap::new()));
                &mut self.rows.last_mut().expect("just pushed").1
            }
        };
        for key in [
            "cases".to_owned(),
            format!("go:{}", go.class),
            format!("rust:{rust}"),
            format!("{verdict:?}"),
        ] {
            *row.entry(key).or_default() += 1;
        }
    }

    /// The counts, one line per category.
    pub fn lines(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(category, row)| {
                let cells: Vec<String> = row.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{category}: {}", cells.join(" "))
            })
            .collect()
    }

    /// The number of cases recorded with `verdict`.
    pub fn count(&self, verdict: Verdict) -> usize {
        let key = format!("{verdict:?}");
        self.rows
            .iter()
            .map(|(_, row)| row.get(&key).copied().unwrap_or(0))
            .sum()
    }
}

/// A frozen-vector line for `case` and Go's outcome.
pub fn frozen_line(case: &Case, go: &GoOutcome) -> String {
    let v = json!({
        "category": case.category,
        "coeffs": case.coeffs.iter().map(|&c| hex128(c)).collect::<Vec<_>>(),
        "go": go.to_json(),
    });
    serde_json::to_string(&v).expect("serialize")
}

/// Parses a frozen-vector line: the category, the coefficients and Go's
/// outcome.
pub fn parse_frozen_line(line: &str) -> (&'static str, Vec<u128>, GoOutcome) {
    let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("bad line {line:?}: {e}"));
    let name = v["category"].as_str().expect("category");
    let category = CATEGORIES
        .iter()
        .map(|&(c, _)| c)
        .find(|&c| c == name)
        .unwrap_or_else(|| panic!("unknown category {name:?}"));
    (
        category,
        hex_array(&v, "coeffs"),
        GoOutcome::from_json(&v["go"]),
    )
}
