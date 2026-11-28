// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The Rust producer of the logical export against the format
//! (`purse-export-check`, ADR-0001): its registry is the parser's, what it
//! writes parses strictly, with and without the passphrases, it exports
//! the auxiliary records in the format's order, and it refuses a store
//! holding rows this slice does not export or a `vsp_fee_status` index
//! that disagrees with its tickets.

mod support;

use purse_errors::Kind;
use purse_export_check::{REGISTRY as CHECK_REGISTRY, parse};
use purse_snacl::{Limits, OsRandom};
use purse_store::{Hash32, Store, VspTickets};
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
    purse_udb::initialize(
        &store,
        &chain,
        &[7; 32],
        b"public",
        b"private",
        &mut OsRandom,
        &Limits::DEFAULT,
    )
    .expect("initialize");
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

/// The `vsp_fee_status` index has no counterpart in Go's database and is
/// folded away; a store where it does not list exactly the tickets of
/// `vsp_tickets` under their statuses is refused rather than exported as
/// though it did.
#[test]
fn a_store_whose_vsp_index_disagrees_is_refused() {
    let opts = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    let row = purse_store::VspTicketRow {
        fee_hash: Hash32([2; 32]),
        fee_status: 0,
        host_id: 0,
    };
    for (index_entry, text) in [
        (
            None,
            "invalid operation: logical export: a VSP ticket missing from vsp_fee_status is not exported by this version of purse-udb",
        ),
        (
            Some(purse_store::VspFeeStatusKey {
                status: 1,
                ticket: Hash32([1; 32]),
            }),
            "invalid operation: logical export: a VSP ticket missing from vsp_fee_status is not exported by this version of purse-udb",
        ),
    ] {
        let (store, chain) = wallet();
        store
            .update(|tx| {
                tx.put::<VspTickets>(&Hash32([1; 32]), &row)?;
                if let Some(k) = &index_entry {
                    tx.put::<purse_store::VspFeeStatus>(k, &purse_store::Empty)?;
                }
                Ok::<_, purse_store::Error>(())
            })
            .expect("commit");
        let tx = store.read().expect("read");
        let err = export(&tx, &chain, &opts).expect_err("refused");
        assert_eq!(err.to_string(), text);
    }
    // An index entry with no ticket.
    let (store, chain) = wallet();
    store
        .update(|tx| {
            tx.put::<purse_store::VspFeeStatus>(
                &purse_store::VspFeeStatusKey {
                    status: 3,
                    ticket: Hash32([1; 32]),
                },
                &purse_store::Empty,
            )
        })
        .expect("commit");
    let tx = store.read().expect("read");
    assert_eq!(
        export(&tx, &chain, &opts).expect_err("refused").to_string(),
        "invalid operation: logical export: a vsp_fee_status entry with no VSP ticket of its status is not exported by this version of purse-udb"
    );
}

/// The auxiliary records are exported in ADR-0001's order (hashes by their
/// display rendering, which here differs from the store's internal byte
/// order), hosts and keys resolved as `GetVSPTicket` resolves them, and
/// host rows no public-key row names (never written by the operations) as
/// `vsphostid` orphans, one per host with the number of such rows as its
/// value, as Go's exporter counts them.
#[test]
fn auxiliary_records_are_exported() {
    use purse_udb::vsp::VspTicket;
    use purse_udb::{agendas, treasury, vsp};
    let opts = ExportOptions {
        pub_pass: None,
        priv_pass: None,
        acct_pass: &[],
        exporter: "crates/purse-udb/tests/export.rs",
        limits: Limits::DEFAULT,
        digest: false,
    };
    // Internal order a < b; display order (reversed) b < a.
    let mut a = [0u8; 32];
    a[0] = 1;
    a[31] = 0xf0;
    let mut b = [0u8; 32];
    b[0] = 2;
    b[31] = 0x0f;
    let (a, b) = (Hash32(a), Hash32(b));
    let (store, chain) = wallet();
    store
        .update(|tx| {
            let t = |host: &str, st| VspTicket {
                fee_hash: b,
                fee_tx_status: st,
                vsp_host_id: 0,
                host: host.as_bytes().to_vec(),
                pub_key: vec![0xaa],
            };
            vsp::set_vsp_ticket(tx, &a, &t("https://b", 1)).expect("a");
            vsp::set_vsp_ticket(tx, &b, &t("https://a", 3)).expect("b");
            treasury::set_tspend_policy(tx, &a, 1).expect("tspend a");
            treasury::set_tspend_policy(tx, &b, 2).expect("tspend b");
            treasury::set_vsp_tspend_policy(tx, &a, &b, 1).expect("vsp tspend");
            treasury::set_treasury_key_policy(tx, &[3, 1], 2).expect("pi");
            treasury::set_vsp_treasury_key_policy(tx, &b, b"", 255).expect("vsp pi");
            agendas::set_default_agenda_preference(tx, 9, b"x\"", b"").expect("agenda");
            agendas::set_ticket_agenda_preference(tx, &a, 9, b"x", b"yes").expect("a");
            agendas::set_ticket_agenda_preference(tx, &b, 9, b"x", b"no").expect("b");
            // Host rows that no public-key row names: two of a host that
            // has a named row too, one of a host that has none.
            for (id, host) in [(7, "https://a"), (8, "https://a"), (9, "https://0")] {
                tx.put::<purse_store::VspHosts>(
                    &purse_store::VspHostId(id),
                    &purse_store::VspHost(host.as_bytes().to_vec()),
                )?;
            }
            Ok::<_, purse_store::Error>(())
        })
        .expect("commit");
    let tx = store.read().expect("read");
    let text = export(&tx, &chain, &opts).expect("export");
    let lines =
        |prefix: &str| -> Vec<&str> { text.lines().filter(|l| l.starts_with(prefix)).collect() };
    let (ad, bd) = (hex_display(&a), hex_display(&b));
    assert_eq!(
        lines("ticketagenda "),
        vec![
            format!("ticketagenda ticket={bd} version=9 agenda=\"x\" choice=\"no\""),
            format!("ticketagenda ticket={ad} version=9 agenda=\"x\" choice=\"yes\""),
        ]
    );
    assert_eq!(
        lines("agenda "),
        vec!["agenda version=9 agenda=\"x\\x22\" choice=\"\""]
    );
    assert_eq!(
        lines("tspendpolicy "),
        vec![
            format!("tspendpolicy tspend={bd} vote=2"),
            format!("tspendpolicy tspend={ad} vote=1"),
        ]
    );
    assert_eq!(
        lines("vsptspendpolicy "),
        vec![format!("vsptspendpolicy ticket={ad} tspend={bd} vote=1")]
    );
    assert_eq!(
        lines("treasurypolicy "),
        vec!["treasurypolicy key=0301 vote=2"]
    );
    assert_eq!(
        lines("vsptreasurypolicy "),
        vec![format!("vsptreasurypolicy ticket={bd} key=_ vote=255")]
    );
    assert_eq!(
        lines("vsphost "),
        vec![
            "vsphost host=\"https://a\" pubkey=aa idhost=\"https://a\" future=-",
            "vsphost host=\"https://b\" pubkey=aa idhost=\"https://b\" future=-",
        ]
    );
    assert_eq!(
        lines("vspticket "),
        vec![
            format!(
                "vspticket ticket={bd} feetx={bd} status=3 host=\"https://a\" pubkey=aa future=-"
            ),
            format!(
                "vspticket ticket={ad} feetx={bd} status=1 host=\"https://b\" pubkey=aa future=-"
            ),
        ]
    );
    assert!(lines("vspfuture ").is_empty());
    assert_eq!(
        lines("orphan "),
        vec![
            "orphan index=vsphostid ref=\"https://0\" val=\"1\"",
            "orphan index=vsphostid ref=\"https://a\" val=\"2\"",
        ]
    );
    purse_export_check::parse(&text).expect("the export parses");
}

/// A hash in display order.
fn hex_display(h: &Hash32) -> String {
    h.0.iter().rev().map(|b| format!("{b:02x}")).collect()
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
