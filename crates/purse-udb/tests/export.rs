// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The Rust producer of the logical export against the format
//! (`purse-export-check`, ADR-0001): its registry is the parser's, what it
//! writes parses strictly, with and without the passphrases, and it
//! refuses a store holding rows this slice does not export.

mod support;

use purse_errors::Kind;
use purse_export_check::{REGISTRY as CHECK_REGISTRY, parse};
use purse_snacl::{Limits, OsRandom};
use purse_store::{Hash32, Store, VspTickets};
use purse_udb::Manager;
use purse_udb::addrmgr::ChainParams;
use purse_udb::export::{COVERED, ExportOptions, REGISTRY, export};
use support::scratch;

#[test]
fn registry_is_the_parsers() {
    assert_eq!(REGISTRY.len(), CHECK_REGISTRY.len());
    for (a, b) in REGISTRY.iter().zip(CHECK_REGISTRY) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.version, b.version, "[{}]", a.name);
        assert_eq!(a.types, b.types, "[{}]", a.name);
    }
    for name in COVERED {
        assert!(REGISTRY.iter().any(|s| s.name == name), "{name}");
    }
}

fn wallet() -> (Store, ChainParams) {
    let chain = ChainParams::by_name("simnet").expect("simnet");
    let dir = scratch("export");
    let store = Store::create(&dir.join(purse_store::STORE_FILE_NAME)).expect("create store");
    store
        .update(|tx| {
            Manager::create(
                tx,
                &chain,
                &[7; 32],
                b"public",
                b"private",
                &mut OsRandom,
                &Limits::DEFAULT,
            )
            .expect("create");
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
    (store, chain)
}

#[test]
fn exports_parse_with_and_without_passphrases() {
    let (store, chain) = wallet();
    let tx = store.read().expect("read");
    for (pub_pass, priv_pass, decrypt, privcheck) in [
        (None, None, "none", "0"),
        (Some(&b"public"[..]), None, "public", "0"),
        (Some(&b"public"[..]), Some(&b"private"[..]), "public", "1"),
    ] {
        let opts = ExportOptions {
            pub_pass,
            priv_pass,
            acct_pass: &[],
            exporter: "crates/purse-udb/tests/export.rs",
            limits: Limits::DEFAULT,
            digest: false,
        };
        let text = export(&tx, &chain, &opts).expect("export");
        let ex = parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(ex.meta("decrypt"), Some(decrypt));
        assert_eq!(ex.meta("privcheck"), Some(privcheck));
        assert_eq!(ex.meta("origin"), Some("purse-udb/1"));
        if privcheck == "1" {
            ex.check_sealed_secrets().expect("every verdict yes");
        }
        // Without the public passphrase no plaintext field is filled.
        let accounts = ex.section("accounts").expect("accounts");
        let xpub = accounts
            .records
            .iter()
            .find_map(|r| accounts.value(r, "xpub"));
        assert_eq!(xpub == Some("-"), decrypt == "none");
    }
    // A wrong private passphrase is a "no" verdict, not an error.
    let opts = ExportOptions {
        pub_pass: Some(b"public"),
        priv_pass: Some(b"not it"),
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let ex = parse(&export(&tx, &chain, &opts).expect("export")).expect("parse");
    let pc = ex.section("privcheck").expect("privcheck");
    assert_eq!(
        pc.records.iter().find_map(|r| pc.value(r, "ok")),
        Some("no")
    );
    assert!(ex.check_sealed_secrets().is_err());
    // A wrong public passphrase fails the export.
    let opts = ExportOptions {
        pub_pass: Some(b"not it"),
        ..opts
    };
    let err = export(&tx, &chain, &opts).expect_err("wrong public passphrase");
    assert!(err.is_kind(Kind::Passphrase));
}

/// `ExportOptions`' `Debug` names which passphrases are given and never
/// prints their bytes.
#[test]
fn export_options_debug_hides_the_passphrases() {
    let opts = ExportOptions {
        pub_pass: Some(b"public-secret"),
        priv_pass: Some(b"hunter2"),
        acct_pass: &[(0x8000_0000, b"voting-secret")],
        exporter: "x",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let s = format!("{opts:?}");
    assert!(s.contains("pub_pass: <given>"), "{s}");
    assert!(s.contains("priv_pass: <given>"), "{s}");
    assert!(s.contains("acct_pass: <1 given>"), "{s}");
    assert!(s.contains("exporter: \"x\""), "{s}");
    for secret in [&b"public-secret"[..], b"hunter2", b"voting-secret"] {
        assert!(!s.contains(std::str::from_utf8(secret).expect("ascii")));
        let bytes = format!("{secret:?}");
        assert!(!s.contains(bytes.trim_matches(['[', ']'])), "{s}");
    }
    let none = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        ..opts
    };
    let s = format!("{none:?}");
    assert!(
        s.contains("pub_pass: <none>") && s.contains("priv_pass: <none>"),
        "{s}"
    );
}

#[test]
fn a_store_with_rows_not_exported_is_refused() {
    let (store, chain) = wallet();
    store
        .update(|tx| {
            tx.put::<VspTickets>(
                &Hash32([1; 32]),
                &purse_store::VspTicketRow {
                    fee_hash: Hash32([2; 32]),
                    fee_status: 0,
                    host_id: 1,
                },
            )
        })
        .expect("commit");
    let tx = store.read().expect("read");
    let opts = ExportOptions {
        pub_pass: Some(b"public"),
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let err = export(&tx, &chain, &opts).expect_err("refused");
    assert_eq!(
        err.to_string(),
        "invalid operation: logical export: table vsp_tickets is not exported by this version of purse-udb"
    );
}

/// The commitment and multisig tables are exported: a commitment with its
/// unspent entry folded in (`unminedspent` absent without one), and a
/// multisig record with its spender only while spent and `utxo` while
/// unspent, `txhash` repeating the key's hash as Go's value does.
#[test]
fn commitments_and_multisig_records_are_exported() {
    let opts = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let (store, chain) = wallet();
    let op = |b: u8, index: u32| purse_store::Outpoint {
        hash: Hash32([b; 32]),
        index,
    };
    store
        .update(|tx| {
            for (o, amount, account, unspent) in [
                (op(1, 1), 1100, 0, Some(false)),
                (op(1, 3), 900, 2, Some(true)),
                (op(2, 1), 500, 1, None),
            ] {
                tx.put::<purse_store::Commitments>(
                    &o,
                    &purse_store::CommitmentRow { amount, account },
                )?;
                if let Some(u) = unspent {
                    tx.put::<purse_store::CommitmentsUnspent>(
                        &o,
                        &purse_store::CommitmentUnspentRow { unmined_spent: u },
                    )?;
                }
            }
            let row = purse_store::MultisigOutRow {
                script_hash: purse_store::Hash160([5; 20]),
                m: 1,
                n: 2,
                stake_tree: true,
                block: purse_store::MultisigBlock::new(Hash32([6; 32]), 7),
                amount: 4000,
                spender: None,
            };
            tx.put_multisig_out(&op(3, 0), &row)?;
            tx.put_multisig_out(
                &op(3, 1),
                &purse_store::MultisigOutRow {
                    stake_tree: false,
                    block: None,
                    spender: Some(purse_store::MultisigSpender {
                        hash: Hash32([8; 32]),
                        input: 2,
                    }),
                    ..row
                },
            )
        })
        .expect("commit");
    let tx = store.read().expect("read");
    let text = export(&tx, &chain, &opts).expect("export");
    let h = |b: u8| format!("{b:02x}").repeat(32);
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("commitment ") || l.starts_with("multisig "))
        .collect();
    assert_eq!(
        lines,
        [
            format!(
                "commitment tx={} index=1 amount=1100 account=0 unspent=1 unminedspent=0",
                h(1)
            ),
            format!(
                "commitment tx={} index=3 amount=900 account=2 unspent=1 unminedspent=1",
                h(1)
            ),
            format!(
                "commitment tx={} index=1 amount=500 account=1 unspent=0 unminedspent=-",
                h(2)
            ),
            format!(
                "multisig tx={} index=0 scripthash={} m=1 n=2 spent=0 stake=1 block={} height=7 \
                 amount=4000 sptx=- spindex=- txhash={} utxo=1",
                h(3),
                "05".repeat(20),
                h(6),
                h(3)
            ),
            format!(
                "multisig tx={} index=1 scripthash={} m=1 n=2 spent=1 stake=0 block={} height=0 \
                 amount=4000 sptx={} spindex=2 txhash={} utxo=0",
                h(3),
                "05".repeat(20),
                h(0),
                h(8),
                h(3)
            ),
        ]
    );
    purse_export_check::parse(&text).expect("the export parses");
}

#[test]
fn a_store_with_address_path_entries_is_refused() {
    // addr_by_path is not written by the address manager; a store holding
    // an entry has state the export cannot show.
    let (store, chain) = wallet();
    store
        .update(|tx| {
            tx.put::<purse_store::AddrByPath>(
                &purse_store::AddrPathKey {
                    account: 0,
                    branch: 0,
                    index: 0,
                },
                &purse_store::Hash160([2; 20]),
            )
        })
        .expect("commit");
    let tx = store.read().expect("read");
    let opts = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let err = export(&tx, &chain, &opts).expect_err("refused");
    assert_eq!(
        err.to_string(),
        "invalid operation: logical export: table addr_by_path is not exported by this version of purse-udb"
    );
}
