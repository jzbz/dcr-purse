// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Conformance of the Go exporter (`tools/oracle/cmd/walletexport`) on
//! databases dcrwallet's own code generated (`tools/oracle/cmd/walletgen`):
//! the upstream fixtures upgraded to version 26, and fresh simnet wallets
//! with and without the scripted scenario. Every export must parse
//! strictly, be byte-identical when repeated and after dcrwallet reopens the
//! database, differ between two wallets of one seed only in volatile
//! fields (and, with different injected clocks, in clock fields), and agree
//! with what purse-boltread reads from the same file. Fault-injected copies
//! (walletgen `mutate`) exercise the anomaly sections, the VSP counter, the
//! residue's place outside the eof digests and the private-key verdicts.

// Test code: an overflow here is a test bug and panics loudly.
#![allow(clippy::arithmetic_side_effects)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use common::*;
use purse_boltread::Db;
use purse_export_check::{
    COMPARED_METAS, ENUMS, Export, Profile, REGISTRY, Record, Section, decode_hash, decode_hex,
    decode_str, encode_str, parse, validate,
};
use purse_testutil::hex;
use purse_testutil::serde_json::{Value, json};

/// The upstream fixtures (all testnet3 or testnet2, public passphrase
/// "public").
const FIXTURES: [&str; 8] = [
    "v1.db.gz",
    "v2.db.gz",
    "v3.db.gz",
    "v4.db.gz",
    "v5.db.gz",
    "v7.db.gz",
    "v11.db.gz",
    "v24.db.gz",
];

#[test]
fn registry_matches_the_go_exporter() {
    let Some(mut o) = oracles() else { return };
    let res = o.exp.call("sections", Value::Null);
    assert_eq!(res["magic"], purse_export_check::MAGIC);
    let sections = res["sections"].as_array().expect("sections");
    assert_eq!(res["version"], purse_export_check::FORMAT_VERSION);
    assert_eq!(sections.len(), REGISTRY.len());
    for (got, spec) in sections.iter().zip(REGISTRY) {
        assert_eq!(got["name"], spec.name);
        assert_eq!(got["version"], spec.version);
        assert_eq!(got["compared"], spec.compared, "section {}", spec.name);
        let types: Vec<String> = spec.types.iter().map(|d| format!("=type {d}")).collect();
        assert_eq!(got["types"], json!(types), "section {}", spec.name);
    }
    let enums = res["enums"].as_object().expect("enums");
    assert_eq!(enums.len(), ENUMS.len());
    for (field, values) in ENUMS {
        assert_eq!(enums[*field], json!(values), "{field}");
    }
    assert_eq!(res["compared_metas"], json!(COMPARED_METAS));
}

/// One generated database.
struct Db26 {
    name: String,
    path: PathBuf,
    network: &'static str,
}

fn corpus(o: &mut Oracles, dir: &Path) -> Vec<Db26> {
    let mut dbs = Vec::new();
    for (name, scenario) in [("full", "full"), ("empty", "empty")] {
        let path = dir.join(format!("{name}.db"));
        o.create(&path, scenario, TIME, TIME);
        dbs.push(Db26 {
            name: name.to_owned(),
            path,
            network: "simnet",
        });
    }
    for f in FIXTURES {
        let name = f.trim_end_matches(".db.gz").to_owned();
        let path = dir.join(format!("{name}.db"));
        let res = o.upgrade(f, &path);
        assert_eq!(res["to"], 26, "{f} upgraded to {res}");
        dbs.push(Db26 {
            name,
            path,
            network: "testnet3",
        });
    }
    dbs
}

#[test]
fn generated_databases_export_deterministically_and_match_the_file() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("corpus");
    let dbs = corpus(&mut o, &dir);
    let mut table = BTreeMap::new();
    for db in &dbs {
        let extra = if db.name == "full" {
            json!({"privpass": PRIVPASS, "acctpass": {"1": ACCT1_PASS, VOTING_ACCOUNT: VOTING_PASS}})
        } else {
            json!({})
        };
        let (res1, text1) = o.export(
            &db.path,
            &dir.join(format!("{}.1.pe", db.name)),
            db.network,
            extra.clone(),
        );
        let (res2, text2) = o.export(
            &db.path,
            &dir.join(format!("{}.2.pe", db.name)),
            db.network,
            extra.clone(),
        );
        assert_eq!(text1, text2, "{}: two exports differ", db.name);
        assert_eq!(res1, res2);

        let ex = parse(&text1).unwrap_or_else(|e| panic!("{}: {e}", db.name));
        let streamed = validate(text1.as_bytes(), &[]).expect("validate");
        assert_eq!(streamed.export.sha256, ex.sha256);
        assert_eq!(res1["sha256"], ex.sha256.as_str());
        assert_eq!(res1["msha256"], ex.msha256.as_str());
        assert_eq!(res1["lines"].as_u64(), Some(text1.lines().count() as u64));
        for s in &ex.sections {
            assert_eq!(
                res1["counts"][&s.name],
                s.records.len(),
                "{} {}",
                db.name,
                s.name
            );
        }
        for name in ["bad", "orphans", "unknown"] {
            let s = ex.section(name).expect("section");
            assert!(
                s.records.is_empty(),
                "{}: [{name}] is not empty: {:?}",
                db.name,
                s.lines(false)
            );
        }
        let derived = ex.section("derived").expect("derived");
        assert_eq!(
            derived.records_of("failed").count(),
            0,
            "{}: an API call failed",
            db.name
        );

        // dcrwallet opening the database (as at start-up) changes nothing
        // the export sees.
        o.reopen(&db.path, db.network);
        let (_, text3) = o.export(
            &db.path,
            &dir.join(format!("{}.3.pe", db.name)),
            db.network,
            extra,
        );
        assert_eq!(
            text1, text3,
            "{}: export changed after a dcrwallet reopen",
            db.name
        );

        cross_read(&db.name, &db.path, &ex);
        let populated: Vec<String> = ex
            .sections
            .iter()
            .filter(|s| !s.records.is_empty())
            .map(|s| s.name.clone())
            .collect();
        table.insert(db.name.clone(), populated);
    }

    // Report which database populates which section, and require the
    // scenario to fill every section a database without a chain backend
    // can fill.
    for (name, secs) in &table {
        eprintln!("{name:6} {}", secs.join(" "));
    }
    let full: BTreeSet<&str> = table["full"].iter().map(String::as_str).collect();
    for s in REGISTRY.iter().map(|r| r.name) {
        let expected_empty = ["orphans", "bad", "unknown"].contains(&s);
        assert_eq!(full.contains(s), !expected_empty, "full scenario and [{s}]");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fields whose values differ between two parsed exports of one
/// shape, as `type.field`, with the flags of each.
fn differing_fields(a: &Export, b: &Export) -> BTreeMap<String, (bool, bool)> {
    let mut differing = BTreeMap::new();
    for (sa, sb) in a.sections.iter().zip(&b.sections) {
        assert_eq!(sa.records.len(), sb.records.len(), "[{}]", sa.name);
        for (ra, rb) in sa.records.iter().zip(&sb.records) {
            assert_eq!(ra.ty, rb.ty);
            let t = &sa.types[ra.ty];
            for (i, f) in t.fields.iter().enumerate() {
                if ra.values[i] != rb.values[i] {
                    differing.insert(format!("{}.{}", t.name, f.name), (f.volatile, f.clock));
                }
            }
        }
    }
    differing
}

#[test]
fn masked_fields_are_the_only_differences_between_two_wallets_of_one_seed() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("masking");
    // Same seed, passphrases, scenario, chain and injected clock; udb's own
    // randomness and clock readings differ, which nothing outside udb can
    // steer (salts, nonces, keys, creation dates, address add times).
    o.create(&dir.join("a.db"), "full", TIME, TIME);
    o.create(&dir.join("b.db"), "full", TIME, TIME);
    // The same again with the injected wallet clock a day earlier
    // (transaction received times, the birthday time).
    o.create(&dir.join("c.db"), "full", TIME - 86_400, TIME);
    let (_, ta) = o.export_private(&dir.join("a.db"), &dir.join("a.pe"));
    let (_, tb) = o.export_private(&dir.join("b.db"), &dir.join("b.pe"));
    let (_, tc) = o.export_private(&dir.join("c.db"), &dir.join("c.pe"));
    let (a, b, c) = (
        parse(&ta).expect("a"),
        parse(&tb).expect("b"),
        parse(&tc).expect("c"),
    );
    for ex in [&a, &b, &c] {
        ex.check_sealed_secrets()
            .unwrap_or_else(|e| panic!("masked profile not valid: {e}"));
    }

    // Same injected clock: masked equal, and only '~' fields differ, so the
    // injected clock values compare exactly under masked.
    assert_ne!(a.sha256, b.sha256, "the two wallets should differ");
    assert_eq!(a.msha256, b.msha256, "masked exports differ");
    let ab = differing_fields(&a, &b);
    eprintln!("same clock, differing: {:?}", ab.keys().collect::<Vec<_>>());
    for (f, (volatile, _)) in &ab {
        assert!(*volatile, "{f} differs and is not volatile");
    }
    for must in ["masterkey.salt", "cryptokey.sealed", "account.encpriv"] {
        assert!(ab.contains_key(must), "{must} did not differ");
    }

    // Different injected clocks: masked differs (a wrong received time or
    // birthday is seen), masked-clock agrees, and only '~' and '@' fields
    // differ, every '@' field among them.
    assert_ne!(a.msha256, c.msha256, "masked hides the injected clock");
    assert_eq!(
        a.digest(Profile::MASKED_CLOCK),
        c.digest(Profile::MASKED_CLOCK),
        "masked-clock exports differ"
    );
    let ac = differing_fields(&a, &c);
    eprintln!(
        "other clock, differing: {:?}",
        ac.keys().collect::<Vec<_>>()
    );
    for (f, (volatile, clock)) in &ac {
        assert!(
            *volatile || *clock,
            "{f} differs and is neither volatile nor a clock value"
        );
    }
    for must in ["tx.received", "utx.received", "birth.time"] {
        assert!(ac.contains_key(must), "{must} did not differ");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Digest mode must be a function of full mode (ADR-0001 6.10):
/// recomputing the segments from a full export gives the digest export.
#[test]
fn digest_mode_is_computed_from_full_mode() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("digest");
    let db = dir.join("full.db");
    o.create(&db, "full", TIME, TIME);
    let (_, full) = o.export(&db, &dir.join("full.pe"), "simnet", json!({}));
    let (_, dig) = o.export(
        &db,
        &dir.join("digest.pe"),
        "simnet",
        json!({"chain": "digest"}),
    );
    let (full, dig) = (parse(&full).expect("full"), parse(&dig).expect("digest"));
    assert_eq!(full.meta("chain"), Some("full"));
    assert_eq!(dig.meta("chain"), Some("digest"));
    for (sf, sd) in full.sections.iter().zip(&dig.sections) {
        let want = match sf.name.as_str() {
            "blocks" | "headers" | "cfilters" => to_digest(sf),
            _ => sf.lines(false),
        };
        assert_eq!(want, sd.lines(false), "[{}]", sf.name);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn databases_below_version_26_are_refused() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("refuse");
    let gz = std::fs::File::open(upstream_dir().join("v24.db.gz")).expect("open fixture");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .read_to_end(&mut bytes)
        .expect("gunzip");
    let db = dir.join("v24.db");
    std::fs::write(&db, bytes).expect("write");
    let err = o
        .exp
        .try_call(
            "export",
            json!({"path": path_str(&db), "out": path_str(&dir.join("x.pe")), "network": "testnet3"}),
        )
        .expect_err("a version 24 database must be refused");
    assert!(err.text.contains("database version 24"), "{}", err.text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Regenerates `tests/data/full-simnet.pe` when PURSE_REGEN_EXPORT is set.
#[test]
fn regenerate_frozen_export() {
    if std::env::var_os("PURSE_REGEN_EXPORT").is_none() {
        return;
    }
    let Some(mut o) = oracles() else { return };
    let dir = scratch("regen");
    let db = dir.join("full.db");
    o.create(&db, "full", TIME, TIME);
    let (_, _) = o.export(
        &db,
        &data_dir().join("full-simnet.pe"),
        "simnet",
        json!({
            "privpass": PRIVPASS,
            "acctpass": {"1": ACCT1_PASS, VOTING_ACCOUNT: VOTING_PASS},
            "comments": [
                "Frozen logical export, format purse-export 2 (docs/adr/0001-logical-export-format.md).",
                "Made by tools/oracle/cmd/walletgen `create` (network simnet, seed 000102...1f, public",
                "passphrase \"public\", private passphrase \"private\", scenario \"full\", time 1794990000)",
                "and exported by tools/oracle/cmd/walletexport with the private and account passphrases;",
                "regenerate with PURSE_REGEN_EXPORT=1 cargo test -p purse-export-check --test oracle",
                "regenerate_frozen_export. Volatile values differ on every regeneration.",
            ],
        }),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ fault injection

/// The value of `key` in the bucket at `path`.
fn get(db: &Db, path: &[&[u8]], key: &[u8]) -> Option<Vec<u8>> {
    rows(db, path)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn len_prefixed(b: &[u8]) -> (&[u8], &[u8]) {
    let n = le32(b) as usize;
    (&b[4..4 + n], &b[4 + n..])
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// Every anomaly class the format defines appears, and the export still
/// parses strictly: each orphan class, a block record with trailing bytes
/// (ADR-0001 6.9), a tip that is not 32 bytes (6.8), and VSP host IDs above
/// the counter, one with a host row and one named only by a ticket (6.22).
#[test]
fn fault_injected_anomalies_are_exported() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("faults");
    let db = dir.join("full.db");
    o.create(&db, "full", TIME, TIME);
    let (counter, bk, bv, tip) = {
        let r = Db::open(&db).expect("open");
        let counter = be32(&get(&r, &[b"vsphost"], b"vsphostindex").expect("counter"));
        let (bk, bv) = rows(&r, &[b"wtxmgr", b"b"])
            .into_iter()
            .find(|(_, v)| be32(&v[43..47]) > 0)
            .expect("a block with transactions");
        let tip = get(&r, &[b"wtxmgr"], b"tip").expect("tip");
        (counter, bk, bv, tip)
    };
    let (far, dangling) = (counter + 4, counter + 6);
    let host = b"https://other.example";
    let acct0 = 0u32.to_le_bytes();
    let ghost = 77u32.to_le_bytes();
    let w: &[u8] = b"waddrmgr";
    let t: &[u8] = b"wtxmgr";
    o.mutate(
        &db,
        &[
            (
                "put",
                &[b"vsphost"],
                &far.to_be_bytes(),
                &cat(&[&(host.len() as u32).to_be_bytes(), host]),
            ),
            (
                "put",
                &[b"vsp"],
                &[0x42; 32],
                &cat(&[&[0x43; 32], &1u32.to_be_bytes(), &dangling.to_be_bytes()]),
            ),
            ("put", &[w, b"addracctidx"], &[0xaa; 32], &acct0),
            ("put", &[w, b"addracctidx", &acct0], &[0xbb; 32], &[0]),
            ("bucket", &[w, b"acctvars"], &ghost, &[]),
            ("put", &[w, b"acctvars", &ghost], b"name", b"ghost"),
            (
                "put",
                &[w, b"acctididx"],
                &ghost,
                &cat(&[&5u32.to_le_bytes(), b"ghost"]),
            ),
            (
                "put",
                &[t, b"u"],
                &cat(&[&[0xcc; 32], &0u32.to_be_bytes()]),
                &cat(&[&1u32.to_be_bytes(), &[0xdd; 32]]),
            ),
            (
                "put",
                &[t, b"mu"],
                &cat(&[&[0xee; 32], &1u32.to_be_bytes()]),
                &[0],
            ),
            (
                "put",
                &[t, b"cmu"],
                &cat(&[&[0xfe; 32], &2u32.to_be_bytes()]),
                &[0],
            ),
            ("put", &[t, b"b"], &bk, &cat(&[&bv, &[0x11; 32]])),
            ("put", &[t], b"tip", &cat(&[&tip, &[0]])),
        ],
    );
    let (_, text) = o.export_private(&db, &dir.join("faults.pe"));
    let ex = parse(&text).unwrap_or_else(|e| panic!("{e}"));

    let orphans = ex.section("orphans").expect("orphans");
    let classes: BTreeSet<&str> = orphans
        .records
        .iter()
        .filter_map(|r| orphans.value(r, "index"))
        .collect();
    let all = purse_export_check::enum_values("orphan", "index").expect("orphan.index");
    assert_eq!(classes, all.iter().copied().collect(), "orphan classes");

    let bad = ex.section("bad").expect("bad");
    let has_bad = |path: &str, key: &[u8], reason: &str| {
        bad.records.iter().any(|r| {
            bad.value(r, "path") == Some(encode_str(path.as_bytes()).as_str())
                && bad.value(r, "key") == Some(hex(key).as_str())
                && bad.value(r, "reason") == Some(reason)
        })
    };
    assert!(has_bad("wtxmgr/b", &bk, "trail"), "{:?}", bad.lines(false));
    assert!(has_bad("wtxmgr", b"tip", "len"), "{:?}", bad.lines(false));
    assert!(
        has_bad("vsphost", b"vsphostindex", "counter"),
        "{:?}",
        bad.lines(false)
    );

    // The block record and the tip still read as Go's readers read them.
    let blocks = ex.section("blocks").expect("blocks");
    let n = be32(&bv[43..47]) as usize;
    let txs: Vec<String> = (0..n)
        .map(|i| disp(&bv[47 + 32 * i..79 + 32 * i]))
        .collect();
    let r = find(
        blocks,
        "block",
        &[("height", be32(&bk).to_string().as_str())],
    );
    assert_eq!(blocks.value(r, "txs"), Some(txs.join(",").as_str()));
    let chain = ex.section("chain").expect("chain");
    let r = chain.records_of("tip").next().expect("tip");
    assert_eq!(chain.value(r, "hash"), Some(disp(&tip).as_str()));

    // IDs above the counter, by offset.
    let vsp = ex.section("vsp").expect("vsp");
    let future: BTreeMap<String, String> = vsp
        .records_of("vspfuture")
        .map(|r| {
            (
                vsp.value(r, "offset").expect("offset").to_owned(),
                vsp.value(r, "host").expect("host").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        future,
        BTreeMap::from([
            ("4".to_owned(), encode_str(host)),
            ("6".to_owned(), "-".to_owned())
        ])
    );
    let r = find(vsp, "vspticket", &[("ticket", disp(&[0x42; 32]).as_str())]);
    assert_eq!(vsp.value(r, "future"), Some("6"));
    assert_eq!(vsp.value(r, "host"), Some("\"\""));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two exports that differ only in dead items (ADR-0001 section 9) have
/// equal eof digests: [residue] is outside them (section 4).
#[test]
fn residue_is_outside_the_eof_digests() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("residue");
    let (a, b) = (dir.join("a.db"), dir.join("b.db"));
    o.create(&a, "full", TIME, TIME);
    std::fs::copy(&a, &b).expect("copy");
    o.mutate(
        &b,
        &[
            ("put", &[b"waddrmgr", b"main"], b"seed", &[0x5e; 32]),
            ("put", &[b"wtxmgr"], b"vers", &1u32.to_be_bytes()),
        ],
    );
    let (ra, ta) = o.export_private(&a, &dir.join("a.pe"));
    let (rb, tb) = o.export_private(&b, &dir.join("b.pe"));
    let (ea, eb) = (parse(&ta).expect("a"), parse(&tb).expect("b"));
    let (sa, sb) = (
        ea.section("residue").expect("residue"),
        eb.section("residue").expect("residue"),
    );
    assert_eq!(
        sa.records.len() + 2,
        sb.records.len(),
        "{:?}",
        sb.lines(false)
    );
    assert_ne!(ra["file_sha256"], rb["file_sha256"]);
    assert_eq!(ea.sha256, eb.sha256, "exact eof digest");
    assert_eq!(ea.msha256, eb.msha256, "masked eof digest");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sealed private key that does not belong to its public half gets a
/// `no` verdict even where masking makes the two exports equal: the
/// SLIP0044 coin-type key replaced by the legacy one, and the parked
/// SLIP0044 account 0 key by account 0's (ADR-0001 6.27).
#[test]
fn swapped_sealed_keys_get_a_no_verdict() {
    let Some(mut o) = oracles() else { return };
    let dir = scratch("privswap");
    let (a, b) = (dir.join("a.db"), dir.join("b.db"));
    o.create(&a, "full", TIME, TIME);
    std::fs::copy(&a, &b).expect("copy");
    let (ctpriv, slip_row) = {
        let r = Db::open(&b).expect("open");
        let main: &[&[u8]] = &[b"waddrmgr", b"main"];
        let ctpriv = get(&r, main, b"ctpriv").expect("ctpriv");
        let slip = get(&r, main, b"slip0044acct0").expect("slip0044acct0");
        let acct0 = get(&r, &[b"waddrmgr", b"acct"], &0u32.to_le_bytes()).expect("account 0");
        // v1 row: type 0, LE32 length, <pub><priv><ext><int><name>;
        // version-26 account row: type, LE32 length, <pub><priv>.
        let (pubk, rest) = len_prefixed(&slip[5..]);
        let (_, tail) = len_prefixed(rest);
        let (_, r2) = len_prefixed(&acct0[5..]);
        let (acct_priv, _) = len_prefixed(r2);
        let raw = cat(&[
            &(pubk.len() as u32).to_le_bytes(),
            pubk,
            &(acct_priv.len() as u32).to_le_bytes(),
            acct_priv,
            tail,
        ]);
        (
            ctpriv,
            cat(&[&[0], &(raw.len() as u32).to_le_bytes(), &raw]),
        )
    };
    o.mutate(
        &b,
        &[
            ("put", &[b"waddrmgr", b"main"], b"ctpriv-slip0044", &ctpriv),
            ("put", &[b"waddrmgr", b"main"], b"slip0044acct0", &slip_row),
        ],
    );
    let (_, ta) = o.export_private(&a, &dir.join("a.pe"));
    let (_, tb) = o.export_private(&b, &dir.join("b.pe"));
    let (ea, eb) = (parse(&ta).expect("a"), parse(&tb).expect("b"));
    ea.check_sealed_secrets().expect("the original is verified");
    let pc = eb.section("privcheck").expect("privcheck");
    let r = find(pc, "ctok", &[("which", "slip0044")]);
    assert_eq!(pc.value(r, "ok"), Some("no"));
    let r = find(pc, "ctok", &[("which", "legacy")]);
    assert_eq!(pc.value(r, "ok"), Some("yes"));
    let r = pc
        .records_of("slip0044acct0ok")
        .next()
        .expect("slip0044acct0ok");
    assert_eq!(pc.value(r, "ok"), Some("no"));
    assert!(eb.check_sealed_secrets().is_err());
    // Masking alone hides the swap: [cointypes] is masked-equal.
    let (ca, cb) = (
        ea.section("cointypes").expect("cointypes"),
        eb.section("cointypes").expect("cointypes"),
    );
    assert_ne!(ca.sha256, cb.sha256);
    assert_eq!(ca.msha256, cb.msha256);
    assert_ne!(ea.msha256, eb.msha256, "the verdicts are compared");
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ digest mode

const SEG: u64 = 4096;

fn sha256(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn u(s: &str) -> u64 {
    s.parse().expect("u")
}

/// Converts the full-mode [blocks], [headers] or [cfilters] section to its
/// digest-mode records.
fn to_digest(s: &Section) -> Vec<String> {
    let (seg_type, item_type) = match s.name.as_str() {
        "blocks" => ("blockseg", "block"),
        "headers" => ("hdrseg", "header"),
        _ => ("cfseg", "cfilter"),
    };
    let mut segs: BTreeMap<u64, (u64, Vec<u8>)> = BTreeMap::new();
    let mut kept = Vec::new();
    for r in &s.records {
        let t = &s.types[r.ty];
        let line = purse_export_check::render(t, &r.values, false);
        if t.name != item_type {
            kept.push((t.name.clone(), line));
            continue;
        }
        let v = |f: &str| s.value(r, f).expect("field");
        let height = u(v("height"));
        let mut item = (height as u32).to_be_bytes().to_vec();
        item.extend_from_slice(&decode_hash(v("hash")).expect("hash"));
        match item_type {
            "block" => {
                if v("txs") != "_" || v("inv") != "0" {
                    kept.push((t.name.clone(), line));
                    continue;
                }
                item.extend_from_slice(&u(v("time")).to_be_bytes());
                item.extend_from_slice(&(u(v("votebits")) as u16).to_be_bytes());
            }
            "header" => {
                let raw = decode_hex(v("raw")).expect("raw");
                item.extend_from_slice(&(raw.len() as u32).to_be_bytes());
                item.extend_from_slice(&raw);
            }
            _ => {
                let key = decode_hex(v("key")).expect("key");
                let filter = decode_hex(v("filter")).expect("filter");
                item.extend_from_slice(&key);
                item.extend_from_slice(&(filter.len() as u32).to_be_bytes());
                item.extend_from_slice(&filter);
            }
        }
        let e = segs.entry(height / SEG).or_default();
        e.0 += 1;
        e.1.extend_from_slice(&item);
    }
    let mut out: Vec<String> = segs
        .iter()
        .map(|(seg, (n, bytes))| format!("{seg_type} seg={seg} n={n} sha256={}", sha256(bytes)))
        .collect();
    // The segment type is declared first; the kept records follow in their
    // full-mode order, which digest mode does not change.
    out.extend(kept.into_iter().map(|(_, l)| l));
    out
}

// ------------------------------------------------------------ cross-read

/// Non-bucket entries of the bucket at `path` (empty if it is missing).
fn rows(db: &Db, path: &[&[u8]]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let Some(b) = db.bucket_path(path).expect("bucket_path") else {
        return Vec::new();
    };
    b.iter()
        .map(|item| item.expect("cursor"))
        .filter_map(|item| item.value().map(|v| (item.key().to_vec(), v.to_vec())))
        .collect()
}

/// Nested bucket names of the bucket at `path`.
fn nested(db: &Db, path: &[&[u8]]) -> Vec<Vec<u8>> {
    let Some(b) = db.bucket_path(path).expect("bucket_path") else {
        return Vec::new();
    };
    b.iter()
        .map(|item| item.expect("cursor"))
        .filter(|item| item.is_bucket())
        .map(|item| item.key().to_vec())
        .collect()
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().expect("4 bytes"))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().expect("4 bytes"))
}

/// Display form of a stored hash.
fn disp(stored: &[u8]) -> String {
    let mut h = stored.to_vec();
    h.reverse();
    hex(&h)
}

fn count_where(s: &Section, ty: &str, field: &str, value: &str) -> usize {
    s.records_of(ty)
        .filter(|r| s.value(r, field) == Some(value))
        .count()
}

fn find<'a>(s: &'a Section, ty: &str, keys: &[(&str, &str)]) -> &'a Record {
    s.records_of(ty)
        .find(|r| keys.iter().all(|(f, v)| s.value(r, f) == Some(*v)))
        .unwrap_or_else(|| panic!("[{}] has no {ty} with {keys:?}", s.name))
}

/// Reads the database with purse-boltread, independently of Go, and checks
/// the export against it: entry counts of every bucket that maps to a
/// section, and the records themselves where they are simple.
fn cross_read(name: &str, path: &Path, ex: &Export) {
    let db = Db::open(path).unwrap_or_else(|e| panic!("{name}: open: {e}"));
    let sec = |n: &str| ex.section(n).expect("section");
    let full_chain = ex.meta("chain") == Some("full");

    // Bucket entry counts against record counts.
    let counts: [(&[&[u8]], &str, &str); 17] = [
        (&[b"waddrmgr", b"acct"], "accounts", "account"),
        (&[b"waddrmgr", b"acctnameidx"], "accountnames", "nameindex"),
        (&[b"waddrmgr", b"addr"], "addresses", "address"),
        (&[b"wtxmgr", b"t"], "txs", "tx"),
        (&[b"wtxmgr", b"m"], "txs", "utx"),
        (&[b"wtxmgr", b"up"], "unpublished", "unpublished"),
        (&[b"wtxmgr", b"mc"], "ucredits", "ucredit"),
        (&[b"wtxmgr", b"mi"], "uinputs", "uinput"),
        (&[b"wtxmgr", b"tix"], "tickets", "ticket"),
        (&[b"wtxmgr", b"cmt"], "commitments", "commitment"),
        (&[b"wtxmgr", b"ms"], "multisig", "multisig"),
        (&[b"agendaprefs"], "agendas", "agenda"),
        (&[b"tspendpolicy"], "treasury", "tspendpolicy"),
        (&[b"vsptspendpolicy"], "treasury", "vsptspendpolicy"),
        (&[b"treasurypolicy"], "treasury", "treasurypolicy"),
        (&[b"vsptreasurypolicy"], "treasury", "vsptreasurypolicy"),
        (&[b"vsp"], "vsp", "vspticket"),
    ];
    for (bpath, section, ty) in counts {
        assert_eq!(
            rows(&db, bpath).len(),
            sec(section).records_of(ty).count(),
            "{name}: {} rows against {section}.{ty}",
            bpath
                .iter()
                .map(|p| String::from_utf8_lossy(p))
                .collect::<Vec<_>>()
                .join("/")
        );
    }
    let credits = sec("credits");
    let n_c = rows(&db, &[b"wtxmgr", b"c"]).len();
    let n_ic = rows(&db, &[b"wtxmgr", b"ic"]).len();
    assert_eq!(credits.records.len(), n_c + n_ic, "{name}: credits");
    assert_eq!(
        count_where(credits, "credit", "inv", "1"),
        n_ic,
        "{name}: ic"
    );
    assert_eq!(
        count_where(credits, "credit", "utxo", "1"),
        rows(&db, &[b"wtxmgr", b"u"]).len(),
        "{name}: u"
    );
    let debits = sec("debits");
    let n_id = rows(&db, &[b"wtxmgr", b"id"]).len();
    assert_eq!(
        debits.records.len(),
        rows(&db, &[b"wtxmgr", b"d"]).len() + n_id,
        "{name}: debits"
    );
    assert_eq!(count_where(debits, "debit", "inv", "1"), n_id, "{name}: id");
    assert_eq!(
        count_where(sec("commitments"), "commitment", "unspent", "1"),
        rows(&db, &[b"wtxmgr", b"cmu"]).len(),
        "{name}: cmu"
    );
    assert_eq!(
        count_where(sec("multisig"), "multisig", "utxo", "1"),
        rows(&db, &[b"wtxmgr", b"mu"]).len(),
        "{name}: mu"
    );
    let ticket_prefs: usize = nested(&db, &[b"ticketsagendaprefs"])
        .iter()
        .map(|t| rows(&db, &[b"ticketsagendaprefs".as_slice(), t.as_slice()]).len())
        .sum();
    assert_eq!(
        ticket_prefs,
        sec("agendas").records_of("ticketagenda").count(),
        "{name}: ticket prefs"
    );
    assert_eq!(
        rows(&db, &[b"vsppubkey"]).len(),
        sec("vsp").records_of("vsphost").count(),
        "{name}: vsp hosts"
    );
    if full_chain {
        let blocks = rows(&db, &[b"wtxmgr", b"b"]);
        assert_eq!(
            blocks.len(),
            sec("blocks").records_of("block").count(),
            "{name}: blocks"
        );
        let headers = sec("headers");
        assert_eq!(
            rows(&db, &[b"wtxmgr", b"h"]).len(),
            headers.records_of("header").count() + headers.records_of("sideheader").count(),
            "{name}: headers"
        );
        let cf = sec("cfilters");
        assert_eq!(
            rows(&db, &[b"wtxmgr", b"cf"]).len(),
            cf.records_of("cfilter").count() + cf.records_of("sidecfilter").count(),
            "{name}: cfilters"
        );
        // Block records: height, hash, the invalidated flag, the tx list.
        let bs = sec("blocks");
        for (k, v) in &blocks {
            let n = be32(&v[43..47]) as usize;
            let txs: Vec<String> = (0..n).map(|i| disp(&v[47 + 32 * i..79 + 32 * i])).collect();
            let list = if txs.is_empty() {
                "_".to_owned()
            } else {
                txs.join(",")
            };
            let r = find(
                bs,
                "block",
                &[
                    ("height", be32(k).to_string().as_str()),
                    ("hash", disp(&v[..32]).as_str()),
                ],
            );
            assert_eq!(bs.value(r, "inv"), Some(if v[42] != 0 { "1" } else { "0" }));
            assert_eq!(
                bs.value(r, "txs"),
                Some(list.as_str()),
                "{name}: block {}",
                be32(k)
            );
        }
    }

    // [wallet] and [chain] values.
    let wallet = sec("wallet");
    let w = &wallet.records[0];
    let meta = rows(&db, &[b"waddrmgr", b"meta"]);
    let last = meta
        .iter()
        .find(|(k, _)| k == b"lastaccount")
        .expect("lastaccount");
    assert_eq!(
        wallet.value(w, "lastaccount"),
        Some(le32(&last.1).to_string().as_str())
    );
    let txroot = rows(&db, &[b"wtxmgr"]);
    let tip = txroot.iter().find(|(k, _)| k == b"tip").expect("tip");
    let chain = sec("chain");
    let mut tip32 = [0u8; 32];
    tip32[..tip.1.len().min(32)].copy_from_slice(&tip.1[..tip.1.len().min(32)]);
    assert_eq!(
        chain.value(chain.records_of("tip").next().expect("tip"), "hash"),
        Some(disp(&tip32).as_str())
    );

    // Accounts: number, type, the acctvars name and counters, and the
    // idname index.
    let accounts = sec("accounts");
    for (k, v) in rows(&db, &[b"waddrmgr", b"acct"]) {
        let num = le32(&k).to_string();
        let r = find(accounts, "account", &[("num", num.as_str())]);
        assert_eq!(
            accounts.value(r, "type"),
            Some(v[0].to_string().as_str()),
            "{name}: account {num} type"
        );
        let vars = rows(&db, &[b"waddrmgr".as_slice(), b"acctvars", &k]);
        let var = |key: &[u8]| {
            vars.iter()
                .find(|(kk, _)| kk == key)
                .map(|(_, vv)| vv.clone())
        };
        let vname = var(b"name").unwrap_or_default();
        assert_eq!(
            accounts.value(r, "name"),
            Some(encode_str(&vname).as_str()),
            "{name}: account {num} name"
        );
        for (field, key) in [
            ("extused", "extused"),
            ("intused", "intused"),
            ("extret", "extret"),
            ("intret", "intret"),
        ] {
            let val = var(key.as_bytes()).expect("counter");
            assert_eq!(
                accounts.value(r, field),
                Some(le32(&val).to_string().as_str()),
                "{name}: {num} {field}"
            );
        }
        assert_eq!(
            accounts.value(r, "kdfsalt") != Some("-"),
            var(b"kdf-params").is_some()
        );
        let idx = rows(&db, &[b"waddrmgr", b"acctididx"]);
        let (_, iv) = idx
            .iter()
            .find(|(kk, _)| *kk == k)
            .expect("acctididx entry");
        assert_eq!(
            accounts.value(r, "idname"),
            Some(encode_str(&iv[4..]).as_str())
        );
    }
    let names = sec("accountnames");
    for (k, v) in rows(&db, &[b"waddrmgr", b"acctnameidx"]) {
        let n = le32(&k) as usize;
        let r = find(
            names,
            "nameindex",
            &[("name", encode_str(&k[4..4 + n]).as_str())],
        );
        assert_eq!(names.value(r, "num"), Some(le32(&v).to_string().as_str()));
    }

    // Addresses: type, account, add time, and branch and index of chained
    // rows; and membership in the per-account index.
    let addrs = sec("addresses");
    for (k, v) in rows(&db, &[b"waddrmgr", b"addr"]) {
        let r = find(addrs, "address", &[("key", hex(&k).as_str())]);
        let ty = ["chain", "key", "script"][v[0] as usize];
        assert_eq!(addrs.value(r, "type"), Some(ty));
        let acct = le32(&v[1..5]);
        assert_eq!(addrs.value(r, "account"), Some(acct.to_string().as_str()));
        let added = u64::from_le_bytes(v[5..13].try_into().expect("8 bytes"));
        assert_eq!(addrs.value(r, "added"), Some(added.to_string().as_str()));
        if v[0] == 0 {
            assert_eq!(
                addrs.value(r, "branch"),
                Some(le32(&v[18..22]).to_string().as_str())
            );
            assert_eq!(
                addrs.value(r, "index"),
                Some(le32(&v[22..26]).to_string().as_str())
            );
        }
        let set = rows(
            &db,
            &[b"waddrmgr".as_slice(), b"addracctidx", &acct.to_le_bytes()],
        );
        let inset = set.iter().any(|(kk, _)| *kk == k);
        assert_eq!(addrs.value(r, "inset"), Some(if inset { "1" } else { "0" }));
        if ex.meta("decrypt") == Some("public") {
            // SHA-256 of the exported hash160 is the bucket key.
            let id = decode_hex(addrs.value(r, "id").expect("id")).expect("hex");
            assert_eq!(sha256(&id), hex(&k), "{name}: address id");
        }
    }

    // Transactions: key parts and the serialized transaction.
    let txs = sec("txs");
    for (k, v) in rows(&db, &[b"wtxmgr", b"t"]) {
        let r = find(
            txs,
            "tx",
            &[
                ("hash", disp(&k[..32]).as_str()),
                ("height", be32(&k[32..36]).to_string().as_str()),
                ("block", disp(&k[36..68]).as_str()),
            ],
        );
        assert_eq!(txs.value(r, "raw"), Some(hex(&v[8..]).as_str()));
    }
    for (k, v) in rows(&db, &[b"wtxmgr", b"m"]) {
        let r = find(txs, "utx", &[("hash", disp(&k).as_str())]);
        assert_eq!(txs.value(r, "raw"), Some(hex(&v[8..]).as_str()));
    }
    // Agenda preferences: version, agenda ID and choice.
    let agendas = sec("agendas");
    for (k, v) in rows(&db, &[b"agendaprefs"]) {
        let r = find(
            agendas,
            "agenda",
            &[
                ("version", be32(&k).to_string().as_str()),
                ("agenda", encode_str(&k[4..]).as_str()),
            ],
        );
        assert_eq!(agendas.value(r, "choice").and_then(decode_str), Some(v));
    }
}
