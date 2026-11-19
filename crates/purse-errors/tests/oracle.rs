// Copyright (c) 2018-2019 The Decred developers
// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Differential tests against dcrwallet's `errors` package
//! (`tools/oracle/cmd/errors`), and the frozen vectors that oracle wrote to
//! `tests/data/errors-vectors.jsonl`.
//!
//! A program is a list of nodes, each building one error value and
//! possibly referring to earlier ones (the node forms are documented in the
//! oracle's source). Go and Rust each build the program and report, for
//! every value: the rendered text, the shape of the chain after `E`'s
//! promote and collapse rules (with which values are identical), `Is`
//! against every kind and every other value, `As` for a kind and for an
//! `*Error`, `Unwrap` and the number of stack traces.
//!
//! The separator is process-wide, so every test here that sets it holds
//! `SEP_LOCK`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use purse_errors::{
    AnyError, Arg, Builder, DAEMON_SEPARATOR, Error, Kind, MULTILINE_SEPARATOR, Op, join, new,
    set_separator,
};
use purse_testutil::serde_json::{self, Value, json};
use purse_testutil::{Oracle, SplitMix64, oracle_or_skip};

static SEP_LOCK: Mutex<()> = Mutex::new(());

const SEPARATORS: [&str; 2] = [DAEMON_SEPARATOR, MULTILINE_SEPARATOR];

/// Kind codes queried through the oracle's `kinds` command, including
/// values outside the defined range.
const KIND_CODES: [i64; 27] = [
    -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 100,
    255,
];

/// The seed of the frozen corpus (fixed, independent of `PURSE_TEST_SEED`).
const FROZEN_SEED: u64 = 0x6572_726f_7273;
const FROZEN_RANDOM_PROGRAMS: usize = 120;

fn vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/errors-vectors.jsonl")
}

// ---------------------------------------------------------------------------
// Program generation
// ---------------------------------------------------------------------------

fn index(rng: &mut SplitMix64, len: usize) -> usize {
    let n = u64::try_from(len).expect("len fits u64");
    usize::try_from(rng.below(n)).expect("index fits usize")
}

fn pick<'a, T>(rng: &mut SplitMix64, xs: &'a [T]) -> &'a T {
    &xs[index(rng, xs.len())]
}

fn gen_op(rng: &mut SplitMix64) -> &'static str {
    // Ops equal to kind strings and to leaf texts, and ops containing a
    // separator, make the rendering's repeat suppression and the text
    // comparison of template leaves meet look-alike strings.
    pick(
        rng,
        &[
            "",
            "a",
            "a",
            "b",
            "wallet.Foo",
            "udb.Bar(1)",
            "invalid operation",
            "I/O error",
            "abc",
            "base error",
            "a:: b",
            "x:\n\ty",
        ],
    )
}

fn gen_kind(rng: &mut SplitMix64) -> u64 {
    if rng.below(2) == 0 {
        *pick(rng, &[0, 2, 4, 6, 10])
    } else {
        rng.below(21)
    }
}

fn gen_text(rng: &mut SplitMix64) -> &'static str {
    pick(
        rng,
        &[
            "abc",
            "def",
            "base error",
            "invalid operation",
            "a: I/O error",
            "",
            "x",
            "line1\nline2",
            "unclassified error",
            "b",
            ":: ",
            "a:: b",
            ":\n\t",
            "b:\n\tc",
        ],
    )
}

/// A random program; `nil[i]` records which nodes evaluate to nil.
fn gen_program(rng: &mut SplitMix64) -> Vec<Value> {
    let n = usize::try_from(rng.below(8))
        .expect("small")
        .saturating_add(2);
    let mut nodes = Vec::with_capacity(n);
    let mut nil: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        let r = rng.below(100);
        let non_nil: Vec<usize> = (0..i).filter(|&j| !nil[j]).collect();
        let (node, is_nil) = if i == 0 || r < 15 {
            (json!({"t": "new", "text": gen_text(rng)}), false)
        } else if r < 60 {
            let nargs = rng.below(5).saturating_add(1);
            let args: Vec<Value> = (0..nargs)
                .map(|_| match rng.below(100) {
                    0..25 => json!({"op": gen_op(rng)}),
                    25..50 => json!({"kind": gen_kind(rng)}),
                    50..65 => json!({"str": gen_text(rng)}),
                    _ => json!({"err": index(rng, i)}),
                })
                .collect();
            (
                json!({"t": "e", "args": args, "stack": rng.below(20) == 0}),
                false,
            )
        } else if r < 70 {
            let err = if rng.below(10) < 6 {
                json!(index(rng, i))
            } else {
                Value::Null
            };
            (
                json!({"t": "lit", "op": gen_op(rng), "kind": gen_kind(rng), "err": err}),
                false,
            )
        } else if r < 78 {
            (json!({"t": "kind", "kind": gen_kind(rng)}), false)
        } else if r < 89 && !non_nil.is_empty() {
            let count = rng.below(3).saturating_add(1);
            let errs: Vec<usize> = (0..count).map(|_| *pick(rng, &non_nil)).collect();
            let prefix = pick(rng, &["ctx: ", "", "wrapped ", ":: ", ":\n\t", "a:: "]);
            (json!({"t": "wrap", "prefix": prefix, "errs": errs}), false)
        } else {
            let count = rng.below(4);
            let errs: Vec<usize> = (0..count).map(|_| index(rng, i)).collect();
            let is_nil = errs.iter().all(|&j| nil[j]);
            (json!({"t": "join", "errs": errs}), is_nil)
        };
        nodes.push(node);
        nil.push(is_nil);
    }
    nodes
}

fn op(s: &str) -> Value {
    json!({"op": s})
}
fn kind(k: Kind) -> Value {
    json!({"kind": k.code()})
}
fn st(s: &str) -> Value {
    json!({"str": s})
}
fn er(i: usize) -> Value {
    json!({"err": i})
}
fn e(args: &[Value]) -> Value {
    json!({"t": "e", "args": args, "stack": false})
}
fn e_stack(args: &[Value]) -> Value {
    json!({"t": "e", "args": args, "stack": true})
}
fn lit(op: &str, kind: Kind, err: Option<usize>) -> Value {
    json!({"t": "lit", "op": op, "kind": kind.code(), "err": err})
}
fn text(s: &str) -> Value {
    json!({"t": "new", "text": s})
}
fn bare(k: Kind) -> Value {
    json!({"t": "kind", "kind": k.code()})
}
fn wrap(prefix: &str, errs: &[usize]) -> Value {
    json!({"t": "wrap", "prefix": prefix, "errs": errs})
}
fn joined(errs: &[usize]) -> Value {
    json!({"t": "join", "errs": errs})
}

/// Hand-picked programs: the worked examples of spec 22 section 5.4, the
/// upstream tests, and the quirks of spec 22 section 10. Their expected
/// results come from the oracle like everything else.
fn fixed_programs() -> Vec<Vec<Value>> {
    use Kind::*;
    vec![
        vec![e(&[op("a"), kind(Passphrase)])],
        vec![text("x"), e(&[kind(Io), er(0)])],
        vec![e(&[kind(Io), st("disk")]), e(&[op("a"), er(0)])],
        vec![e(&[op("b"), kind(Io), st("disk")]), e(&[op("a"), er(0)])],
        vec![
            e(&[op("b"), kind(Io), st("disk")]),
            e(&[op("a"), kind(Invalid), er(0)]),
        ],
        vec![e(&[op("a"), kind(Io), st("disk")]), e(&[op("a"), er(0)])],
        vec![e(&[kind(Invalid), st("abc")]), e(&[er(0), st("def")])],
        vec![e(&[st("text")])],
        vec![e(&[op("")])],
        // errors_test.go TestCollapse.
        vec![
            e(&[op("abc")]),
            e(&[er(0), kind(Passphrase)]),
            e(&[op("abc"), kind(Passphrase)]),
        ],
        // errors_test.go TestIs.
        vec![
            text("base error"),
            e(&[er(0), op("operation"), kind(Permission)]),
            e(&[st("different error")]),
            e(&[op("operation")]),
            e(&[op("different operation")]),
            e(&[kind(Permission)]),
            e(&[kind(Invalid)]),
        ],
        // Q5 / QK-0628: match on a template leaf against a level without
        // one panics in Go.
        vec![
            e(&[op("x"), kind(Invalid)]),
            e(&[st("text")]),
            lit("", Other, Some(1)),
            lit("x", Other, Some(1)),
        ],
        // Q8: struct-literal chains with repeated and empty levels.
        vec![
            text("leaf"),
            lit("a", Io, Some(0)),
            lit("a", Io, Some(1)),
            lit("", Other, Some(2)),
            lit("b", Other, Some(3)),
            lit("a", Io, Some(4)),
            lit("", Other, None),
            lit("", Other, Some(6)),
        ],
        // Q2: with two *Error arguments the first one's kind wins.
        vec![
            e(&[kind(Invalid), st("a")]),
            e(&[kind(Io), st("b")]),
            e(&[er(0), er(1)]),
            e(&[er(1), er(0)]),
            e(&[er(0), er(1), st("c")]),
            e(&[er(0), st("c"), er(0)]),
        ],
        // An explicit Other is the same as no kind.
        vec![
            e(&[kind(Io), st("x")]),
            e(&[op("a"), kind(Other), er(0)]),
            e(&[er(0), kind(Other)]),
            e(&[kind(Invalid), er(0)]),
        ],
        // Q3: a nil error argument is ignored.
        vec![
            joined(&[]),
            e(&[op("a"), er(0)]),
            lit("a", Other, Some(0)),
            e(&[op("a"), st("x"), er(0)]),
        ],
        // Kinds used as error values (Q6, Q7).
        vec![
            bare(Other),
            bare(Invalid),
            lit("", Other, Some(0)),
            lit("a", Other, Some(1)),
            e(&[er(1)]),
            e(&[op("a"), er(0)]),
            e(&[op("a"), kind(NotExist)]),
            lit("", Io, Some(1)),
        ],
        // Errorf %w wrappers.
        vec![
            e(&[op("b"), kind(NotExist), st("missing")]),
            wrap("ctx: ", &[0]),
            e(&[op("a"), er(1)]),
            wrap("", &[1, 0]),
            e(&[kind(Locked), er(3)]),
            text("plain"),
            wrap("w ", &[5]),
        ],
        // Join.
        vec![
            e(&[op("a"), kind(Io)]),
            e(&[op("b"), kind(Invalid), st("x")]),
            joined(&[0, 1]),
            e(&[op("c"), er(2)]),
            joined(&[2]),
            joined(&[1, 0]),
        ],
        // Leaf texts equal to kind strings and to renderings.
        vec![
            text("invalid operation"),
            e(&[kind(Invalid)]),
            lit("", Invalid, Some(0)),
            e(&[kind(Invalid), st("invalid operation")]),
            lit("", Other, Some(1)),
            lit("", Other, Some(0)),
        ],
        // Empty texts render as "unclassified error" only when nothing else
        // is written.
        vec![
            e(&[st("")]),
            text(""),
            e(&[op("a"), er(1)]),
            lit("", Other, Some(1)),
            lit("", Other, Some(0)),
        ],
        // Stack traces.
        vec![
            e_stack(&[op("a"), kind(Io)]),
            e(&[er(0), op("a")]),
            e_stack(&[op("b"), er(0)]),
            e_stack(&[er(2), op("b")]),
            e(&[er(1)]),
            wrap("", &[0]),
        ],
        // Nested templates.
        vec![
            lit("b", Other, None),
            e(&[op("b"), kind(Io), st("disk")]),
            e(&[op("a"), kind(Invalid), er(1)]),
            lit("a", Other, Some(0)),
            lit("a", Invalid, Some(1)),
            e(&[op("a"), kind(Invalid), er(0)]),
            lit("", Other, Some(0)),
        ],
        // Template leaves compared by text where the text holds the
        // separator in force when it was built: node 3's leaf is node 2,
        // whose text is node 1 rendered.
        vec![
            lit("b", Other, None),
            lit("a", Other, Some(0)),
            wrap("", &[1]),
            lit("", Other, Some(2)),
            lit("", Other, Some(1)),
        ],
    ]
}

/// The frozen corpus: every fixed program under both separators, then
/// [`FROZEN_RANDOM_PROGRAMS`] random programs from [`FROZEN_SEED`] under
/// alternating separators.
fn frozen_corpus() -> Vec<(&'static str, Vec<Value>)> {
    let mut corpus = Vec::new();
    for nodes in fixed_programs() {
        for sep in SEPARATORS {
            corpus.push((sep, nodes.clone()));
        }
    }
    let mut rng = SplitMix64(FROZEN_SEED);
    for n in 0..FROZEN_RANDOM_PROGRAMS {
        let sep = SEPARATORS[n.checked_rem(SEPARATORS.len()).expect("nonzero")];
        corpus.push((sep, gen_program(&mut rng)));
    }
    corpus
}

// ---------------------------------------------------------------------------
// Building and evaluating a program in Rust
// ---------------------------------------------------------------------------

fn idx(v: &Value) -> usize {
    usize::try_from(v.as_u64().expect("index")).expect("index fits usize")
}

fn kind_of(v: &Value) -> Kind {
    Kind::from_code(v.as_i64().expect("kind code")).expect("defined kind")
}

fn to_arg(a: &Value, vals: &[Option<AnyError>]) -> Arg {
    if let Some(s) = a.get("op") {
        Arg::from(Op::from(s.as_str().expect("op").to_owned()))
    } else if let Some(k) = a.get("kind") {
        Arg::from(kind_of(k))
    } else if let Some(s) = a.get("str") {
        Arg::from(s.as_str().expect("str"))
    } else {
        Arg::from(vals[idx(&a["err"])].clone())
    }
}

fn build(nodes: &[Value]) -> Vec<Option<AnyError>> {
    let mut vals: Vec<Option<AnyError>> = Vec::with_capacity(nodes.len());
    for n in nodes {
        let v = match n["t"].as_str().expect("node type") {
            "e" => {
                let mut args = n["args"].as_array().expect("args").iter();
                let first = to_arg(args.next().expect("at least one arg"), &vals);
                let b = args.fold(Builder::new(first), |b, a| b.arg(to_arg(a, &vals)));
                let err = if n["stack"].as_bool() == Some(true) {
                    b.build_with_stack()
                } else {
                    b.build()
                };
                Some(AnyError::from(err))
            }
            "lit" => {
                let err = match &n["err"] {
                    Value::Null => None,
                    i => vals[idx(i)].clone(),
                };
                let op = Op::from(n["op"].as_str().expect("op").to_owned());
                Some(AnyError::from(Error::from_parts(
                    op,
                    kind_of(&n["kind"]),
                    err,
                )))
            }
            "new" => Some(new(n["text"].as_str().expect("text"))),
            "kind" => Some(AnyError::Kind(kind_of(&n["kind"]))),
            "wrap" => {
                let errs: Vec<AnyError> = n["errs"]
                    .as_array()
                    .expect("errs")
                    .iter()
                    .map(|i| vals[idx(i)].clone().expect("wrap of a non-nil error"))
                    .collect();
                let mut text = n["prefix"].as_str().expect("prefix").to_owned();
                for (j, err) in errs.iter().enumerate() {
                    if j != 0 {
                        text.push_str("; ");
                    }
                    write!(text, "{err}").expect("write to String");
                }
                Some(AnyError::wrap(text, errs))
            }
            "join" => join(
                n["errs"]
                    .as_array()
                    .expect("errs")
                    .iter()
                    .map(|i| vals[idx(i)].clone()),
            ),
            other => panic!("unknown node type {other}"),
        };
        vals.push(v);
    }
    vals
}

fn same_any(vals: &[Option<AnyError>], v: &AnyError) -> i64 {
    vals.iter()
        .position(|w| w.as_ref().is_some_and(|w| w.identical(v)))
        .map_or(-1, |i| i64::try_from(i).expect("small"))
}

fn same_error(vals: &[Option<AnyError>], e: &Error) -> i64 {
    vals.iter()
        .position(|w| matches!(w, Some(AnyError::Error(x)) if std::ptr::eq(&**x, e)))
        .map_or(-1, |i| i64::try_from(i).expect("small"))
}

fn shape_error(vals: &[Option<AnyError>], e: &Error) -> Value {
    json!({
        "t": "error",
        "op": e.op.as_str(),
        "kind": e.kind.code(),
        "stack": e.stack().is_some(),
        "err": e.err.as_ref().map_or(Value::Null, |x| shape(vals, x)),
        "same": same_error(vals, e),
    })
}

fn shape(vals: &[Option<AnyError>], v: &AnyError) -> Value {
    let same = same_any(vals, v);
    match v {
        AnyError::Error(e) => shape_error(vals, e),
        AnyError::Kind(k) => json!({"t": "kind", "kind": k.code(), "same": same}),
        AnyError::Text(t) => json!({"t": "text", "text": t.as_str(), "same": same}),
        AnyError::Wrapped(w) => json!({
            "t": "wrap",
            "text": w.text(),
            "errs": w.wrapped().iter().map(|x| shape(vals, x)).collect::<Vec<_>>(),
            "same": same,
        }),
        AnyError::Joined(j) => json!({
            "t": "join",
            "errs": j.errors().iter().map(|x| shape(vals, x)).collect::<Vec<_>>(),
            "same": same,
        }),
        AnyError::Foreign(_) => panic!("no foreign errors in oracle programs"),
    }
}

fn eval(vals: &[Option<AnyError>]) -> Value {
    let nodes: Vec<Value> = vals
        .iter()
        .map(|v| {
            let Some(v) = v else {
                return json!({"nil": true});
            };
            let unwrap = match v {
                AnyError::Error(e) => e.wrapped().map_or(Value::Null, |w| shape(vals, &w)),
                _ => Value::Null,
            };
            json!({
                "text": v.to_string(),
                "shape": shape(vals, v),
                "is_kind": Kind::ALL.iter().map(|&k| v.is_kind(k)).collect::<Vec<_>>(),
                "as_kind": v.as_kind().map(Kind::code),
                "as_error": v.as_error().map(|e| shape_error(vals, &e)),
                "unwrap": unwrap,
                "stacks": v.stacks().len(),
                "is": vals
                    .iter()
                    .map(|t| match t {
                        Some(t) if v.is(t) => "t",
                        _ => "f",
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "nodes": nodes })
}

/// Builds `nodes` in Rust with `sep` installed and compares the result with
/// Go's. Returns the number of `Is` queries where Go panicked (QK-0628).
fn check(label: &str, sep: &'static str, nodes: &[Value], go: &Value) -> usize {
    set_separator(sep);
    let vals = build(nodes);
    let rust = eval(&vals);
    let program = serde_json::to_string(nodes).expect("serialize");
    let go_nodes = go["nodes"].as_array().expect("go nodes");
    let rust_nodes = rust["nodes"].as_array().expect("rust nodes");
    assert_eq!(go_nodes.len(), rust_nodes.len(), "{label}: node count");
    let mut panics = 0usize;
    for (i, (g, r)) in go_nodes.iter().zip(rust_nodes).enumerate() {
        let (mut g, mut r) = (g.clone(), r.clone());
        let g_is = g.as_object_mut().and_then(|m| m.remove("is"));
        let r_is = r.as_object_mut().and_then(|m| m.remove("is"));
        assert_eq!(
            g, r,
            "{label}: node {i} differs (sep {sep:?})\nprogram: {program}"
        );
        if let (Some(g_is), Some(r_is)) = (g_is, r_is) {
            for (j, (gv, rv)) in g_is
                .as_array()
                .expect("is")
                .iter()
                .zip(r_is.as_array().expect("is"))
                .enumerate()
            {
                // Where Go panics (QK-0628) it gives no answer to compare
                // with: the panicking level does not match in Rust, and the
                // walk goes on (pinned by the unit tests).
                if gv == "p" {
                    panics = panics.saturating_add(1);
                    continue;
                }
                assert_eq!(
                    rv.as_str(),
                    gv.as_str(),
                    "{label}: Is(node {i}, node {j}) differs (Go {gv}, sep {sep:?})\nprogram: {program}"
                );
            }
        }
    }
    panics
}

fn go_eval(oracle: &mut Oracle, sep: &str, nodes: &[Value]) -> Value {
    oracle.call("eval", json!({"sep": sep, "nodes": nodes}))
}

fn check_kinds(go: &Value) {
    let texts = go.as_array().expect("kind texts");
    assert_eq!(texts.len(), KIND_CODES.len());
    for (code, text) in KIND_CODES.iter().zip(texts) {
        assert_eq!(
            Some(Kind::describe_code(*code)),
            text.as_str(),
            "kind {code}"
        );
        if let Some(k) = Kind::from_code(*code) {
            assert_eq!(Some(AnyError::Kind(k).to_string().as_str()), text.as_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn differential_against_go() {
    let Some(mut oracle) = oracle_or_skip("errors") else {
        return;
    };
    let _guard = SEP_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

    check_kinds(&oracle.call("kinds", json!({"codes": KIND_CODES})));

    let mut programs = fixed_programs();
    let fixed = programs.len();
    let mut rng = SplitMix64::from_env("purse-errors");
    let random: usize = std::env::var("PURSE_ERRORS_PROGRAMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    programs.extend((0..random).map(|_| gen_program(&mut rng)));

    let mut panics = 0usize;
    let mut checks = 0usize;
    for (n, nodes) in programs.iter().enumerate() {
        for sep in SEPARATORS {
            let go = go_eval(&mut oracle, sep, nodes);
            panics = panics.saturating_add(check(&format!("program {n}"), sep, nodes, &go));
            checks = checks.saturating_add(1);
        }
    }
    set_separator(MULTILINE_SEPARATOR);
    eprintln!(
        "compared {checks} program evaluations ({fixed} fixed, {random} random programs, \
         2 separators); Go panicked in {panics} Is queries (not compared)"
    );
    assert!(panics > 0, "the QK-0628 case was not exercised");
}

/// Replays the vectors the Go oracle wrote; runs without Go. The file must
/// name the parity commit once and hold exactly the corpus
/// [`frozen_corpus`] rebuilds, so a truncated or stale file fails.
#[test]
fn frozen_vectors() {
    let data = std::fs::read_to_string(vectors_path()).expect("read frozen vectors");
    let _guard = SEP_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

    let commits: Vec<&str> = data
        .lines()
        .filter_map(|l| l.strip_prefix("# dcrwallet commit: "))
        .collect();
    assert_eq!(
        commits,
        [purse_testutil::DCRWALLET_PARITY_COMMIT],
        "the vectors must name the parity commit exactly once"
    );

    let corpus = frozen_corpus();
    assert_eq!(
        corpus.len(),
        fixed_programs()
            .len()
            .saturating_mul(SEPARATORS.len())
            .saturating_add(FROZEN_RANDOM_PROGRAMS)
    );
    let (mut programs, mut kinds, mut panics) = (0usize, 0usize, 0usize);
    for (lineno, line) in data.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let label = format!("vector line {}", lineno.saturating_add(1));
        let rec: Value = serde_json::from_str(line).expect("vector line");
        if let Some(codes) = rec.get("kinds") {
            let want: Vec<i64> = KIND_CODES.to_vec();
            assert_eq!(codes, &json!(want), "kind codes changed");
            check_kinds(&rec["go"]);
            kinds = kinds.saturating_add(1);
            continue;
        }
        let Some((sep, nodes)) = corpus.get(programs) else {
            panic!("{label}: more programs than the corpus holds");
        };
        assert_eq!(
            rec["sep"].as_str(),
            Some(*sep),
            "{label}: separator differs from the corpus"
        );
        assert_eq!(
            rec["nodes"].as_array(),
            Some(nodes),
            "{label}: program differs from the corpus (regenerate the vectors)"
        );
        panics = panics.saturating_add(check(&label, sep, nodes, &rec["go"]));
        programs = programs.saturating_add(1);
    }
    set_separator(MULTILINE_SEPARATOR);
    eprintln!("replayed {programs} frozen programs and {kinds} kind tables ({panics} Go panics)");
    assert_eq!(kinds, 1);
    assert_eq!(programs, corpus.len(), "the vectors are truncated");
    assert!(panics > 0, "the QK-0628 case is missing from the vectors");
}

/// Rewrites `tests/data/errors-vectors.jsonl` from the Go oracle:
/// `cargo test -p purse-errors --test oracle -- --ignored regenerate`.
#[test]
#[ignore = "writes tests/data from the Go oracle; run by hand to regenerate"]
fn regenerate_frozen_vectors() {
    let mut oracle = oracle_or_skip("errors").expect("the Go oracle is required to regenerate");
    let _guard = SEP_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut out = String::new();
    out.push_str(
        "# Frozen vectors for purse-errors, generated by the Go oracle\n\
         # tools/oracle/cmd/errors (dcrwallet's errors package at the parity pin)\n\
         # with: cargo test -p purse-errors --test oracle -- --ignored regenerate\n\
         # Do not edit by hand. Each line after this header is one JSON object:\n\
         # {\"sep\", \"nodes\", \"go\"} gives a program (node forms in the oracle's\n\
         # source) and Go's evaluation of it; {\"kinds\", \"go\"} gives Go's\n\
         # Kind(code).Error() for each code. The fixed programs come first,\n\
         # each under both separators,\n",
    );
    writeln!(
        out,
        "# then {FROZEN_RANDOM_PROGRAMS} random ones from SplitMix64 seed {FROZEN_SEED:#x}, \
         alternating separators."
    )
    .expect("write to String");
    writeln!(
        out,
        "# dcrwallet commit: {}",
        purse_testutil::DCRWALLET_PARITY_COMMIT
    )
    .expect("write to String");

    let go = oracle.call("kinds", json!({"codes": KIND_CODES}));
    check_kinds(&go);
    writeln!(out, "{}", json!({"kinds": KIND_CODES, "go": go})).expect("write to String");

    for (n, (sep, nodes)) in frozen_corpus().iter().enumerate() {
        let go = go_eval(&mut oracle, sep, nodes);
        check(&format!("program {n}"), sep, nodes, &go);
        writeln!(out, "{}", json!({"sep": sep, "nodes": nodes, "go": go}))
            .expect("write to String");
    }
    set_separator(MULTILINE_SEPARATOR);
    std::fs::write(vectors_path(), out).expect("write frozen vectors");
}
