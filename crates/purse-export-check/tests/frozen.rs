// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Checks that need no Go toolchain: the frozen export under `tests/data`
//! parses and validates (in memory and streamed), the ADR specifies every
//! section of the registry in registry order with its declarations and
//! enumerated values, the eof digests leave `[residue]` out, and a file
//! that breaks the grammar in any of a range of ways is rejected.

#![allow(clippy::arithmetic_side_effects)]

mod common;

use common::data_dir;
use purse_export_check::{
    COMPARED_METAS, ENUMS, MAGIC, Profile, REGISTRY, RecordType, parse, render, validate,
};
use sha2::{Digest, Sha256};

fn frozen() -> String {
    std::fs::read_to_string(data_dir().join("full-simnet.pe")).expect("read frozen export")
}

#[test]
fn frozen_export_parses_and_validates() {
    let ex = parse(&frozen()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        ex.comments.iter().any(|c| c.contains("walletgen")),
        "header says how it was made"
    );
    assert_eq!(ex.meta("network"), Some("simnet"));
    assert_eq!(ex.meta("decrypt"), Some("public"));
    assert_eq!(ex.meta("privcheck"), Some("1"));
    assert_eq!(ex.sections.len(), REGISTRY.len());
    for name in ["orphans", "bad", "unknown"] {
        assert!(ex.section(name).expect(name).records.is_empty(), "[{name}]");
    }
    for s in &ex.sections {
        if !["orphans", "bad", "unknown"].contains(&s.name.as_str()) {
            assert!(
                !s.records.is_empty(),
                "[{}] is empty in the full scenario",
                s.name
            );
        }
    }
    let pc = ex.section("privcheck").expect("privcheck");
    for r in &pc.records {
        assert_eq!(
            pc.value(r, "ok"),
            Some("yes"),
            "{:?}",
            render(&pc.types[r.ty], &r.values, false)
        );
    }
    // Every sealed private field has its verdict, so the masked profile
    // is valid for this export.
    ex.check_sealed_secrets()
        .unwrap_or_else(|e| panic!("masked profile not valid: {e}"));
    assert_eq!(pc.records_of("ctok").count(), 2, "legacy and SLIP0044");
    assert_eq!(pc.records_of("slip0044acct0ok").count(), 1);

    // The stored digests are the exact and masked profiles; the streaming
    // validator agrees with the in-memory parser.
    assert_eq!(ex.digest(Profile::EXACT), ex.sha256);
    assert_eq!(ex.digest(Profile::MASKED), ex.msha256);
    let profiles = [
        Profile::MASKED_CLOCK,
        Profile::EXACT.observable(),
        Profile::MASKED.observable(),
    ];
    let sum = validate(frozen().as_bytes(), &profiles).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sum.export.sha256, ex.sha256);
    assert_eq!(sum.export.msha256, ex.msha256);
    assert!(sum.export.sections.iter().all(|s| s.records.is_empty()));
    for (p, d) in &sum.digests {
        assert_eq!(*d, ex.digest(*p), "{p:?}");
    }
    assert_ne!(ex.digest(Profile::MASKED_CLOCK), ex.msha256);
}

fn adr() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/adr/0001-logical-export-format.md"),
    )
    .expect("read ADR-0001")
}

#[test]
fn every_registry_section_is_specified_in_the_adr() {
    let adr = adr();
    assert!(
        adr.contains(&format!("`{MAGIC}`")),
        "ADR-0001 names {MAGIC}"
    );
    for s in REGISTRY {
        let name = s.name;
        assert!(
            adr.contains(&format!("`[{name}]` (version {})", s.version)),
            "ADR-0001 does not give [{name}] at version {}",
            s.version
        );
        for d in s.types {
            assert!(
                adr.contains(d),
                "ADR-0001 does not give the declaration {d:?}"
            );
        }
    }
    // The section headings of section 6 name the sections in registry
    // order ([headers] and [cfilters] share one heading).
    let mut order = Vec::new();
    for line in adr.lines().filter(|l| l.starts_with("#### 6.")) {
        let mut rest = line;
        while let Some(i) = rest.find("`[") {
            let tail = &rest[i + 2..];
            let j = tail.find("]`").expect("closing ]`");
            order.push(tail[..j].to_owned());
            rest = &tail[j..];
        }
    }
    let registry: Vec<&str> = REGISTRY.iter().map(|s| s.name).collect();
    assert_eq!(order, registry, "ADR-0001 section 6 headings");
    // The table of enumerated values gives every e field's set exactly.
    for (field, values) in ENUMS {
        let row = format!(
            "| `{field}` | {} |",
            values
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(adr.contains(&row), "ADR-0001 lacks the row {row}");
    }
}

/// Recomputes every `=end` and the `=eof` line (over the compared metas
/// and the compared sections), so that a mutation reaches the check it
/// targets instead of failing on a digest.
fn redigest(text: &str) -> String {
    let mut out = String::new();
    let mut types: Vec<RecordType> = Vec::new();
    let (mut raw, mut masked, mut n) = (Sha256::new(), Sha256::new(), 0usize);
    let (mut eof_raw, mut eof_masked, mut sections) = (Sha256::new(), Sha256::new(), 0usize);
    let mut metas_done = false;
    let hexd = |h: Sha256| {
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let metas: Vec<&str> = text.lines().filter(|l| l.starts_with('@')).collect();
    for line in text.lines() {
        if let Some(decl) = line.strip_prefix("=type ") {
            types.push(RecordType::parse(decl).expect("decl"));
        } else if line.starts_with("=section ") {
            if !metas_done {
                for want in COMPARED_METAS {
                    if let Some(m) = metas.iter().find(|m| {
                        m.strip_prefix('@')
                            .and_then(|m| m.split_once('='))
                            .map(|p| p.0)
                            == Some(want)
                    }) {
                        eof_raw.update(format!("{m}\n"));
                        eof_masked.update(format!("{m}\n"));
                    }
                }
                metas_done = true;
            }
            types.clear();
        } else if let Some(rest) = line.strip_prefix("=end ") {
            let name = rest.split(' ').next().expect("name");
            let (r, m) = (
                hexd(std::mem::take(&mut raw)),
                hexd(std::mem::take(&mut masked)),
            );
            if REGISTRY.iter().any(|s| s.name == name && s.compared) {
                eof_raw.update(format!("{name} {r}\n"));
                eof_masked.update(format!("{name} {m}\n"));
            }
            out.push_str(&format!("=end {name} records={n} sha256={r} msha256={m}\n"));
            n = 0;
            sections += 1;
            continue;
        } else if line.starts_with("=eof") {
            let (r, m) = (
                hexd(std::mem::take(&mut eof_raw)),
                hexd(std::mem::take(&mut eof_masked)),
            );
            out.push_str(&format!(
                "=eof sections={sections} sha256={r} msha256={m}\n"
            ));
            continue;
        } else if !line.starts_with(['#', '@', '=']) && line != MAGIC {
            let mut tokens = line.split(' ');
            let tname = tokens.next().expect("type");
            if let Some(t) = types.iter().find(|t| t.name == tname) {
                let values: Vec<String> = tokens
                    .map(|tok| {
                        tok.split_once('=')
                            .map_or(String::new(), |(_, v)| v.to_owned())
                    })
                    .collect();
                if values.len() == t.fields.len() {
                    masked.update(render(t, &values, true));
                    masked.update("\n");
                }
            }
            raw.update(line);
            raw.update("\n");
            n += 1;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn redigest_is_the_identity_on_a_valid_file() {
    assert_eq!(redigest(&frozen()), frozen());
}

/// The eof digests leave [residue] out (ADR-0001 section 4): an export
/// that differs only there, as purse-udb's export, which has no dcrwallet
/// dead items, differs from Go's, has the same eof line.
#[test]
fn exports_that_differ_only_in_residue_have_equal_eof_digests() {
    let text = frozen();
    let base = parse(&text).expect("frozen");
    assert!(!base.section("residue").expect("residue").records.is_empty());
    let changed: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("residue "))
        .collect();
    let mut changed = changed.join("\n");
    changed.push('\n');
    let changed = redigest(&changed);
    let ex = parse(&changed).expect("export without residue");
    assert!(ex.section("residue").expect("residue").records.is_empty());
    assert_ne!(
        ex.section("residue").expect("residue").sha256,
        base.section("residue").expect("residue").sha256
    );
    assert_eq!(ex.sha256, base.sha256, "exact eof digest");
    assert_eq!(ex.msha256, base.msha256, "masked eof digest");
    for p in [Profile::MASKED_CLOCK, Profile::EXACT.observable()] {
        assert_eq!(ex.digest(p), base.digest(p), "{p:?}");
    }
    // A compared section is still covered: the same removal in [orphans]'
    // neighbour [unpublished] changes both digests.
    let other: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("unpublished "))
        .collect();
    let mut other = other.join("\n");
    other.push('\n');
    let other = parse(&redigest(&other)).expect("export without unpublished");
    assert_ne!(other.sha256, base.sha256);
    assert_ne!(other.msha256, base.msha256);
}

/// Applies `f` to the lines of the frozen export and expects the result
/// (with digests recomputed when `fix` is set) to be rejected with a
/// message containing `want`.
fn rejects(what: &str, fix: bool, want: &str, f: impl FnOnce(&mut Vec<String>)) {
    let mut lines: Vec<String> = frozen().lines().map(str::to_owned).collect();
    f(&mut lines);
    let mut text = lines.join("\n");
    text.push('\n');
    if fix {
        text = redigest(&text);
    }
    match parse(&text) {
        Ok(_) => panic!("{what}: accepted"),
        Err(e) => assert!(
            e.message.contains(want),
            "{what}: rejected with {e}, expected {want:?}"
        ),
    }
}

fn index_of(lines: &[String], prefix: &str) -> usize {
    lines
        .iter()
        .position(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line {prefix}"))
}

#[test]
fn malformed_exports_are_rejected() {
    rejects("changed value", false, "expected \"=end", |l| {
        let i = index_of(l, "account num=0 ");
        l[i] = l[i].replace("type=1", "type=2");
    });
    rejects("swapped records", true, "out of order", |l| {
        let i = index_of(l, "address ");
        l.swap(i, i + 1);
    });
    rejects("duplicated record", true, "duplicate key", |l| {
        let i = index_of(l, "address ");
        let dup = l[i].clone();
        l.insert(i, dup);
    });
    rejects("two accounts with one number", true, "duplicate key", |l| {
        let i = index_of(l, "account num=0 ");
        let dup = l[i].replace("type=1", "type=2");
        l.insert(i + 1, dup);
    });
    rejects(
        "two unmined transactions with one hash",
        true,
        "duplicate key",
        |l| {
            let i = index_of(l, "utx ");
            let dup = l[i].replace("received=", "received=1");
            l.insert(i + 1, dup);
        },
    );
    rejects(
        "enumerated value outside its set",
        true,
        "not one of",
        |l| {
            let i = index_of(l, "address ");
            let (head, tail) = l[i].split_once(" type=").expect("type");
            let tail = tail.split_once(' ').expect("rest").1;
            l[i] = format!("{head} type=bogus {tail}");
        },
    );
    rejects("verdict outside its set", true, "not one of", |l| {
        let i = index_of(l, "mkeyok ");
        l[i] = "mkeyok ok=maybe".to_owned();
    });
    rejects("derived records with @derived=0", true, "@derived", |l| {
        let i = index_of(l, "@derived=");
        l[i] = "@derived=0".to_owned();
        let i = index_of(l, "@privcheck=");
        l[i] = "@privcheck=0".to_owned();
        let i = index_of(l, "=section privcheck ");
        let j = index_of(l, "=end privcheck ");
        let keep = l[i..].iter().take_while(|x| x.starts_with('=')).count();
        l.drain(i + keep..j);
    });
    rejects("plaintext with @decrypt=none", true, "@decrypt", |l| {
        for (m, v) in [
            ("@decrypt=", "none"),
            ("@derived=", "0"),
            ("@privcheck=", "0"),
        ] {
            let i = index_of(l, m);
            l[i] = format!("{m}{v}");
        }
        for sec in ["derived", "privcheck"] {
            let i = index_of(l, &format!("=section {sec} "));
            let j = index_of(l, &format!("=end {sec} "));
            let keep = l[i..].iter().take_while(|x| x.starts_with('=')).count();
            l.drain(i + keep..j);
        }
    });
    rejects(
        "listed header with @chain=digest",
        true,
        "@chain=digest",
        |l| {
            let i = index_of(l, "@chain=");
            l[i] = "@chain=digest".to_owned();
        },
    );
    rejects(
        "private check without the master key",
        true,
        "mkeyok",
        |l| {
            let i = index_of(l, "mkeyok ");
            l.remove(i);
        },
    );
    rejects("second singleton", true, "more than one", |l| {
        let i = index_of(l, "tip ");
        let dup = l[i].clone();
        l.insert(i, dup);
    });
    rejects("type order", true, "out of order", |l| {
        let i = index_of(l, "tip ");
        let j = index_of(l, "birth ");
        l.swap(i, j);
    });
    rejects("upper-case hex", true, "not a canonical value", |l| {
        let i = index_of(l, "address ");
        let (head, tail) = l[i].split_once(" id=").expect("id");
        let (prefix, key) = head.split_once("key=").expect("key");
        l[i] = format!("{prefix}key={} id={tail}", key.to_uppercase());
    });
    rejects("absent required field", true, "may not be absent", |l| {
        let i = index_of(l, "account num=0 ");
        l[i] = l[i].replace("type=1", "type=-");
    });
    rejects("non-canonical number", true, "not a canonical value", |l| {
        let i = index_of(l, "account num=0 ");
        l[i] = l[i].replace("type=1", "type=01");
    });
    rejects("needless escape", true, "not a canonical value", |l| {
        let i = index_of(l, "account num=0 ");
        l[i] = l[i].replace("name=\"default\"", "name=\"\\x64efault\"");
    });
    rejects("field order", true, "expected field", |l| {
        let i = index_of(l, "account num=0 ");
        l[i] = l[i].replacen("num=0 type=1", "type=1 num=0", 1);
    });
    rejects("unknown record type", true, "no record type", |l| {
        let i = index_of(l, "tip ");
        l.insert(i, "tipp hash=00".to_owned());
    });
    rejects("missing section", true, "expected \"=section", |l| {
        let i = index_of(l, "=section unpublished ");
        let j = index_of(l, "=end unpublished ");
        l.drain(i..=j);
    });
    rejects("changed schema", true, "expected \"=type", |l| {
        let i = index_of(l, "=type tip ");
        l[i] = "=type tip 0 hash:x".to_owned();
    });
    rejects("trailing space", false, "trailing space", |l| {
        let i = index_of(l, "tip ");
        l[i].push(' ');
    });
    rejects("carriage return", false, "outside printable ASCII", |l| {
        let i = index_of(l, "tip ");
        l[i].push('\r');
    });
    rejects("meta order", false, "expected meta", |l| {
        let i = index_of(l, "@chain=");
        l.swap(i, i + 1);
    });
    rejects("bad meta value", false, "bad value", |l| {
        let i = index_of(l, "@network=");
        l[i] = "@network=testnet".to_owned();
    });
    rejects("content after eof", false, "after =eof", |l| {
        l.push("# trailing".to_owned());
    });
}
