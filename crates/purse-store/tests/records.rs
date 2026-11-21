// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! The record operations of `purse_store::records`: VSP host ids given out
//! as dcrwallet gives them, the two indexes kept through every change,
//! refusals that write nothing, the consistency check catching each broken
//! invariant, and the address-manager records that need no table of their
//! own (imported keys and scripts, voting accounts).

#![allow(clippy::arithmetic_side_effects, reason = "test code: small counters")]

mod support;

use purse_errors::Kind;
use purse_store::{
    AccountKey, AccountRow, Accounts, AddrAcctShaKey, AddrByAcctSha, AddrByHash, AddrRow,
    AgendaChoice, AgendaKey, AgendaPrefs, Codec, Empty, Error, Hash32, Hash160, MAX_KEY_SIZE,
    MultisigBlock, MultisigOutRow, MultisigSpender, MultisigUnspent, MultisigUnspentKey,
    OpenOptions, Outpoint, PiKey, RECORD_TABLES, Store, TSpendPolicies, TableId, TicketAgendaKey,
    TicketAgendaPrefs, TreasuryKeyPolicies, TreasuryVote, UNKNOWN_VSP_HOST_TEXT,
    VSP_HOST_IDS_EXHAUSTED_TEXT, VspFeeStatus, VspFeeStatusKey, VspHost, VspHostId, VspHosts,
    VspPubKeyRow, VspPubKeys, VspTicketRow, VspTickets, VspTreasuryKey, VspTreasuryKeyPolicies,
    WriteTx,
};
use purse_testutil::SplitMix64;
use support::digest;
use support::records::records_view;

fn store() -> Store {
    OpenOptions::new()
        .create_on_backend(redb::backends::InMemoryBackend::new(), |_| Ok(()))
        .expect("create")
}

fn h(b: u8) -> Hash32 {
    Hash32([b; 32])
}

fn op(b: u8, index: u32) -> Outpoint {
    Outpoint { hash: h(b), index }
}

fn checked(s: &Store) {
    let tx = s.read().expect("read");
    tx.check_records().expect("store's check");
    records_view(&tx).expect("independent check");
}

/// Hosts get ids 1, 2, 3 in order of first use; a known host keeps its id
/// and the public key of its first use (dcrwallet's `SetVSPTicket`,
/// `wallet/udb/vsp.go:47-82`; QK-0122).
#[test]
fn vsp_hosts_get_ids_in_order_of_first_use() {
    let s = store();
    s.update(|tx| {
        assert_eq!(tx.add_vsp_host(b"https://a.example", &[1; 33])?, 1);
        assert_eq!(tx.add_vsp_host(b"https://b.example", &[2; 33])?, 2);
        assert_eq!(tx.add_vsp_host(b"https://a.example", &[9; 33])?, 1);
        assert_eq!(tx.add_vsp_host(b"https://c.example", b"")?, 3);
        assert_eq!(tx.vsp_host_id(b"https://b.example")?, Some(2));
        assert_eq!(tx.vsp_host_id(b"https://d.example")?, None);
        assert_eq!(tx.vsp_host_id(b"")?, None);
        Ok::<_, Error>(())
    })
    .unwrap();
    let tx = s.read().unwrap();
    assert_eq!(
        tx.get::<VspPubKeys>(&VspHost(b"https://a.example".to_vec()))
            .unwrap(),
        Some(VspPubKeyRow {
            host_id: 1,
            pubkey: vec![1; 33]
        })
    );
    assert_eq!(
        tx.get::<VspHosts>(&VspHostId(3)).unwrap(),
        Some(VspHost(b"https://c.example".to_vec()))
    );
    assert_eq!(tx.len(TableId::VspHosts).unwrap(), 3);
    drop(tx);
    checked(&s);
}

/// The refusals of `add_vsp_host` write nothing and leave the transaction
/// usable, with bbolt's texts for a key it refuses.
#[test]
fn vsp_host_refusals_write_nothing() {
    let s = store();
    let before = digest(&s.read().unwrap());
    s.update(|tx| {
        let e = tx.add_vsp_host(b"", &[1]).unwrap_err();
        assert_eq!(
            (e.kind(), e.to_string().as_str()),
            (Kind::Invalid, "invalid operation: key required")
        );
        let long = vec![b'h'; MAX_KEY_SIZE + 1];
        let e = tx.add_vsp_host(&long, &[1]).unwrap_err();
        assert_eq!(e.to_string(), "invalid operation: key too large");
        // The longest host bbolt takes as a key is accepted.
        assert_eq!(tx.add_vsp_host(&long[..MAX_KEY_SIZE], &[1])?, 1);
        tx.delete_raw(TableId::VspHosts, &1u32.to_be_bytes())?;
        tx.delete_raw(TableId::VspPubKeys, &long[..MAX_KEY_SIZE])?;
        Ok::<_, Error>(())
    })
    .unwrap();
    assert_eq!(digest(&s.read().unwrap()), before);
    // Every id taken: refused, nothing written.
    s.update(|tx| {
        tx.put::<VspHosts>(&VspHostId(u32::MAX), &VspHost(b"last".to_vec()))?;
        tx.put::<VspPubKeys>(
            &VspHost(b"last".to_vec()),
            &VspPubKeyRow {
                host_id: u32::MAX,
                pubkey: Vec::new(),
            },
        )?;
        let e = tx.add_vsp_host(b"one more", &[]).unwrap_err();
        assert_eq!(e.kind(), Kind::Invalid);
        assert!(e.to_string().ends_with(VSP_HOST_IDS_EXHAUSTED_TEXT), "{e}");
        assert_eq!(tx.len(TableId::VspHosts)?, 1);
        // A known host is still found.
        assert_eq!(tx.add_vsp_host(b"last", &[])?, u32::MAX);
        Ok::<_, Error>(())
    })
    .unwrap();
    checked(&s);
}

/// The fee-status index follows every write and delete of a ticket.
#[test]
fn vsp_fee_status_index_follows_the_tickets() {
    let s = store();
    s.update(|tx| {
        let host = tx.add_vsp_host(b"https://v.example", &[5; 33])?;
        for (t, st) in [(3u8, 0u32), (1, 0), (2, 1), (4, 3)] {
            tx.put_vsp_ticket(
                &h(t),
                &VspTicketRow {
                    fee_hash: Hash32::default(),
                    fee_status: st,
                    host_id: host,
                },
            )?;
        }
        assert_eq!(tx.vsp_tickets_with_fee_status(0)?, [h(1), h(3)]);
        // Paid: moves from 0 to 1.
        tx.put_vsp_ticket(
            &h(3),
            &VspTicketRow {
                fee_hash: h(0x33),
                fee_status: 1,
                host_id: host,
            },
        )?;
        assert_eq!(tx.vsp_tickets_with_fee_status(0)?, [h(1)]);
        assert_eq!(tx.vsp_tickets_with_fee_status(1)?, [h(2), h(3)]);
        assert!(tx.delete_vsp_ticket(&h(2))?);
        assert!(!tx.delete_vsp_ticket(&h(2))?);
        assert_eq!(tx.vsp_tickets_with_fee_status(1)?, [h(3)]);
        assert_eq!(tx.vsp_tickets_with_fee_status(3)?, [h(4)]);
        assert_eq!(tx.vsp_tickets_with_fee_status(2)?, []);
        // Host id 0 (no host recorded) is accepted; an unknown id is not,
        // and the refusal writes nothing.
        tx.put_vsp_ticket(&h(5), &VspTicketRow::default())?;
        let e = tx
            .put_vsp_ticket(
                &h(6),
                &VspTicketRow {
                    host_id: 7,
                    ..VspTicketRow::default()
                },
            )
            .unwrap_err();
        assert_eq!(e.kind(), Kind::Invalid);
        assert!(e.to_string().ends_with(UNKNOWN_VSP_HOST_TEXT), "{e}");
        assert!(!tx.contains::<VspTickets>(&h(6))?);
        Ok::<_, Error>(())
    })
    .unwrap();
    checked(&s);
    let tx = s.read().unwrap();
    assert_eq!(tx.len(TableId::VspFeeStatus).unwrap(), 4);
    assert_eq!(tx.vsp_tickets_with_fee_status(0).unwrap(), [h(1), h(5)]);
}

fn ms(script: u8, block: bool, spent: bool) -> MultisigOutRow {
    MultisigOutRow {
        script_hash: Hash160([script; 20]),
        m: 2,
        n: 3,
        stake_tree: false,
        block: block.then(|| MultisigBlock::new(h(0xbb), 9)).flatten(),
        amount: 5_000,
        spender: spent.then_some(MultisigSpender {
            hash: h(0xcc),
            input: 0,
        }),
    }
}

/// A multisig output's index entry follows it through dcrwallet's
/// transitions (added unmined, mined, spent, unspent by a removed spender,
/// unmined by a rollback), and the listing is in outpoint order within its
/// script.
#[test]
fn multisig_unspent_index_follows_the_outputs() {
    let s = store();
    s.update(|tx| {
        // Outpoints given out of order; the listing comes back sorted by
        // raw hash, then big-endian index (index 256 after 1).
        for (o, script) in [
            (op(9, 0), 1u8),
            (op(2, 256), 1),
            (op(2, 1), 1),
            (op(5, 0), 2),
        ] {
            tx.put_multisig_out(&o, &ms(script, false, false))?;
        }
        let one = Hash160([1; 20]);
        assert_eq!(
            tx.unspent_multisig_for(&one)?,
            [op(2, 1), op(2, 256), op(9, 0)]
        );
        assert_eq!(tx.unspent_multisig_for(&Hash160([2; 20]))?, [op(5, 0)]);
        // Mined: still unspent.
        tx.put_multisig_out(&op(2, 1), &ms(1, true, false))?;
        assert_eq!(tx.unspent_multisig_for(&one)?.len(), 3);
        // Spent.
        tx.put_multisig_out(&op(2, 1), &ms(1, true, true))?;
        assert_eq!(tx.unspent_multisig_for(&one)?, [op(2, 256), op(9, 0)]);
        // Spent again (no entry to remove).
        tx.put_multisig_out(&op(2, 1), &ms(1, true, true))?;
        // Unspent again, then unmined.
        tx.put_multisig_out(&op(2, 1), &ms(1, true, false))?;
        tx.put_multisig_out(&op(2, 1), &ms(1, false, false))?;
        assert_eq!(
            tx.unspent_multisig_for(&one)?,
            [op(2, 1), op(2, 256), op(9, 0)]
        );
        // A row rewritten under another script moves its entry.
        tx.put_multisig_out(&op(9, 0), &ms(2, false, false))?;
        assert_eq!(
            tx.unspent_multisig_for(&Hash160([2; 20]))?,
            [op(5, 0), op(9, 0)]
        );
        assert_eq!(
            tx.delete_multisig_out(&op(5, 0))?,
            Some(ms(2, false, false))
        );
        assert_eq!(tx.delete_multisig_out(&op(5, 0))?, None);
        assert_eq!(tx.unspent_multisig_for(&Hash160([2; 20]))?, [op(9, 0)]);
        Ok::<_, Error>(())
    })
    .unwrap();
    checked(&s);
}

/// A multisig output's block cannot carry the all-zero hash, which is
/// dcrwallet's "unmined": such a row would have been stored by
/// `put_multisig_out` and then read back as a different value (height 0)
/// or as a corrupt row (any other height), failing `get`, `check_records`
/// and the verification of every backup of the store. Every row that can be
/// built is read back as written.
#[test]
fn a_multisig_block_never_has_the_unmined_hash() {
    for height in [0, 7, u32::MAX] {
        assert_eq!(MultisigBlock::new(Hash32::default(), height), None);
    }
    let s = store();
    let rows = [
        (op(1, 0), MultisigBlock::new(Hash32::default(), 7)),
        (op(1, 1), MultisigBlock::new(Hash32::default(), 0)),
        (op(1, 2), MultisigBlock::new(h(0xbb), 0)),
        (op(1, 3), MultisigBlock::new(h(0xbb), u32::MAX)),
    ];
    s.update(|tx| {
        for (o, block) in rows {
            tx.put_multisig_out(
                &o,
                &MultisigOutRow {
                    block,
                    ..ms(1, false, false)
                },
            )?;
        }
        Ok::<_, Error>(())
    })
    .unwrap();
    let tx = s.read().unwrap();
    for (o, block) in rows {
        let got = tx.get::<purse_store::MultisigOuts>(&o).unwrap().unwrap();
        assert_eq!(got.block, block, "{o:?}");
    }
    drop(tx);
    checked(&s);
}

/// Every broken invariant is reported by both checks.
#[test]
fn the_checks_catch_each_broken_invariant() {
    type Breaker = fn(&mut WriteTx<'_>) -> purse_store::Result<()>;
    let base = |tx: &mut WriteTx<'_>| -> purse_store::Result<()> {
        let host = tx.add_vsp_host(b"https://v.example", &[5; 33])?;
        tx.put_vsp_ticket(
            &h(1),
            &VspTicketRow {
                fee_hash: h(2),
                fee_status: 1,
                host_id: host,
            },
        )?;
        tx.put_multisig_out(&op(3, 0), &ms(1, true, false))?;
        tx.put_multisig_out(&op(4, 0), &ms(1, true, true))
    };
    let breakers: &[(&str, Breaker)] = &[
        ("unspent output without entry", |tx| {
            tx.delete::<MultisigUnspent>(&MultisigUnspentKey {
                script_hash: Hash160([1; 20]),
                outpoint: op(3, 0),
            })
            .map(drop)
        }),
        ("entry of a spent output", |tx| {
            tx.put::<MultisigUnspent>(
                &MultisigUnspentKey {
                    script_hash: Hash160([1; 20]),
                    outpoint: op(4, 0),
                },
                &Empty,
            )
        }),
        ("entry under another script", |tx| {
            tx.put::<MultisigUnspent>(
                &MultisigUnspentKey {
                    script_hash: Hash160([7; 20]),
                    outpoint: op(3, 0),
                },
                &Empty,
            )
        }),
        ("spent through the generic put", |tx| {
            tx.put::<purse_store::MultisigOuts>(&op(3, 0), &ms(1, true, true))
        }),
        ("ticket without status entry", |tx| {
            tx.delete::<VspFeeStatus>(&VspFeeStatusKey {
                status: 1,
                ticket: h(1),
            })
            .map(drop)
        }),
        ("status entry under another status", |tx| {
            tx.put::<VspFeeStatus>(
                &VspFeeStatusKey {
                    status: 2,
                    ticket: h(1),
                },
                &Empty,
            )
        }),
        ("ticket with an unknown host", |tx| {
            tx.put::<VspTickets>(
                &h(9),
                &VspTicketRow {
                    host_id: 4,
                    ..VspTicketRow::default()
                },
            )?;
            tx.put::<VspFeeStatus>(
                &VspFeeStatusKey {
                    status: 0,
                    ticket: h(9),
                },
                &Empty,
            )
        }),
        ("host without public key row", |tx| {
            tx.delete::<VspPubKeys>(&VspHost(b"https://v.example".to_vec()))
                .map(drop)
        }),
        ("public key row naming another id", |tx| {
            tx.put::<VspPubKeys>(
                &VspHost(b"https://v.example".to_vec()),
                &VspPubKeyRow {
                    host_id: 2,
                    pubkey: Vec::new(),
                },
            )
        }),
        ("host id 0", |tx| {
            tx.put::<VspHosts>(&VspHostId(0), &VspHost(b"zero".to_vec()))?;
            tx.put::<VspPubKeys>(
                &VspHost(b"zero".to_vec()),
                &VspPubKeyRow {
                    host_id: 0,
                    pubkey: Vec::new(),
                },
            )
        }),
    ];
    for (what, brk) in breakers {
        let s = store();
        s.update(base).unwrap();
        checked(&s);
        s.update(brk).unwrap();
        let tx = s.read().unwrap();
        let e = tx
            .check_records()
            .expect_err(&format!("check_records misses: {what}"));
        assert_eq!(e.kind(), Kind::Io, "{what}: {e}");
        assert!(
            records_view(&tx).is_err(),
            "the independent check misses: {what}"
        );
    }
}

/// The policy and agenda tables take dcrwallet's keys through the generic
/// writes: an empty pi key is refused as bbolt refuses it, deleting it is
/// not an error (QK-0127), and a VSP ticket's treasury key may be empty.
#[test]
fn policies_and_preferences_use_the_generic_writes() {
    let s = store();
    s.update(|tx| {
        let e = tx
            .put::<TreasuryKeyPolicies>(&PiKey(Vec::new()), &TreasuryVote::YES)
            .unwrap_err();
        assert_eq!(e.to_string(), "invalid operation: key required");
        assert!(!tx.delete::<TreasuryKeyPolicies>(&PiKey(Vec::new()))?);
        tx.put::<VspTreasuryKeyPolicies>(
            &VspTreasuryKey {
                ticket: h(1),
                pikey: Vec::new(),
            },
            &TreasuryVote::NO,
        )?;
        tx.put::<TSpendPolicies>(&h(2), &TreasuryVote::new(7).expect("non-zero"))?;
        tx.put::<AgendaPrefs>(
            &AgendaKey {
                version: 9,
                agenda: Vec::new(),
            },
            &AgendaChoice(Vec::new()),
        )?;
        for v in [10u32, 11] {
            tx.put::<TicketAgendaPrefs>(
                &TicketAgendaKey {
                    ticket: h(3),
                    version: v,
                    agenda: b"a".to_vec(),
                },
                &AgendaChoice(b"yes".to_vec()),
            )?;
        }
        Ok::<_, Error>(())
    })
    .unwrap();
    let tx = s.read().unwrap();
    assert_eq!(
        tx.get::<TSpendPolicies>(&h(2))
            .unwrap()
            .map(TreasuryVote::code),
        Some(7)
    );
    // One ticket's preferences are one prefix.
    assert_eq!(tx.prefix::<TicketAgendaPrefs>(&h(3).0).unwrap().count(), 2);
    // A zero byte written raw is a corrupt row, not a policy.
    drop(tx);
    s.update(|tx| tx.put_raw(TableId::TSpendPolicies, &h(4).0, &[0]))
        .unwrap();
    let e = s.read().unwrap().get::<TSpendPolicies>(&h(4)).unwrap_err();
    assert!(e.corrupt_row().is_some(), "{e}");
}

/// Imported keys and scripts are `addr_by_hash` rows listed by
/// `addr_by_acct_sha` under the imported account, in the order of the
/// SHA-256 of their HASH160 (dcrwallet's `addracctidx/<account>/`
/// order, which `getaddressesbyaccount "imported"` returns); a voting
/// account is an `accounts` row of its own type.
#[test]
fn imported_addresses_and_voting_accounts_need_no_table_of_their_own() {
    const IMPORTED: u32 = 0x7fff_ffff;
    let s = store();
    let mut r = SplitMix64::from_env("purse-store imported listing");
    let hashes: Vec<Hash160> = (0..50)
        .map(|_| {
            let mut x = [0u8; 20];
            r.fill(&mut x);
            Hash160(x)
        })
        .collect();
    s.update(|tx| {
        for (i, a) in hashes.iter().enumerate() {
            tx.put::<AddrByHash>(
                a,
                &AddrRow::Imported {
                    account: IMPORTED,
                    data: vec![i as u8; 40],
                },
            )?;
            tx.put::<AddrByAcctSha>(&AddrAcctShaKey::for_address(IMPORTED, a), a)?;
        }
        // A derived address of account 1 between them in byte order.
        let d = Hash160([0x80; 20]);
        tx.put::<AddrByHash>(
            &d,
            &AddrRow::Derived {
                account: 1,
                branch: 0,
                index: 0,
            },
        )?;
        tx.put::<AddrByAcctSha>(&AddrAcctShaKey::for_address(1, &d), &d)?;
        tx.put::<Accounts>(
            &AccountKey(2),
            &AccountRow {
                account_type: 2,
                name: b"voting".to_vec(),
                keys: vec![0xa5; 120],
                ..AccountRow::default()
            },
        )
    })
    .unwrap();
    let tx = s.read().unwrap();
    let listed: Vec<Hash160> = tx
        .prefix::<AddrByAcctSha>(&AddrAcctShaKey::account_prefix(IMPORTED))
        .unwrap()
        .map(|e| e.unwrap().1)
        .collect();
    let mut want = hashes.clone();
    want.sort_by_key(|a| AddrAcctShaKey::for_address(IMPORTED, a).encode());
    assert_eq!(listed, want);
    for a in &hashes {
        assert_eq!(
            tx.get::<AddrByHash>(a).unwrap().unwrap().account(),
            IMPORTED
        );
    }
    assert_eq!(
        tx.get::<Accounts>(&AccountKey(2))
            .unwrap()
            .unwrap()
            .account_type,
        2
    );
}

/// A new store has every record table, empty, and passes the checks.
#[test]
fn a_new_store_has_the_record_tables_empty() {
    let s = store();
    let tx = s.read().unwrap();
    for t in RECORD_TABLES {
        assert!(tx.is_empty(t).unwrap(), "{}", t.name());
        assert!(TableId::ALL.contains(&t));
    }
    tx.check_records().unwrap();
    assert_eq!(records_view(&tx).unwrap(), Default::default());
}
